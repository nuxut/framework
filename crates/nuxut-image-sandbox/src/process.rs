//! Ebeveyn tarafında tek bir işçi süreci (Linux). Borular ebeveyn ucunda bloklamasızdır; her
//! okuma/yazma tek bir duvar saati son tarihine karşı `poll` ile bekler (yardımcı thread yok).
//! İptal bayrağı varsa bekleme [`CANCEL_SLICE`] dilimleriyle yapılır ve her dilimde denetlenir.
//! İşçi ele geçirilmiş sayılır: yanıt başlığı isteğin üst sınırına göre denetlenmeden piksel
//! tamponu ayrılmaz; fazla ya da eksik bayt protokol ihlalidir.

use std::fs::File;
use std::os::fd::AsFd;
use std::os::unix::fs::FileExt;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use nuxut_image::{CancelFlag, Image, ImageError, SandboxFailure};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;

use crate::WorkerProfile;
use crate::protocol::{self, HANDSHAKE_HEADER_LEN, Handshake, HandshakeReject, RESPONSE_HEADER_LEN, Request, Response};

/// İptal yoklama dilimi: iptalden sonra işçi en geç bu kadar sonra öldürülür.
pub(crate) const CANCEL_SLICE: Duration = Duration::from_millis(50);
/// Dosya akışının parça boyutu (ebeveyn dosyayı belleğe almaz).
pub(crate) const STREAM_CHUNK: usize = 256 << 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IoFail {
    Timeout,
    /// Karşı uç kapandı (işçi öldü ya da stdout'u kapattı).
    Eof,
    Cancelled,
    Other,
}

fn timespec(d: Duration) -> Timespec {
    Timespec { tv_sec: d.as_secs().min(i64::MAX as u64) as i64, tv_nsec: i64::from(d.subsec_nanos()) }
}

fn wait_ready(fd: impl AsFd, events: PollFlags, deadline: Instant, cancel: Option<&CancelFlag>) -> Result<(), IoFail> {
    loop {
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(IoFail::Cancelled);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(IoFail::Timeout);
        }
        let mut wait = deadline - now;
        if cancel.is_some() {
            wait = wait.min(CANCEL_SLICE);
        }
        let mut fds = [PollFd::new(&fd, events)];
        match poll(&mut fds, Some(&timespec(wait))) {
            Ok(0) => continue,
            Ok(_) => {
                let re = fds[0].revents();
                if re.intersects(events) {
                    return Ok(());
                }
                return Err(IoFail::Eof);
            }
            Err(Errno::INTR) => continue,
            Err(_) => return Err(IoFail::Other),
        }
    }
}

fn read_exact(fd: impl AsFd, buf: &mut [u8], deadline: Instant, cancel: Option<&CancelFlag>) -> Result<(), IoFail> {
    let mut off = 0;
    while off < buf.len() {
        match rustix::io::read(&fd, &mut buf[off..]) {
            Ok(0) => return Err(IoFail::Eof),
            Ok(n) => off += n,
            Err(Errno::AGAIN) => wait_ready(&fd, PollFlags::IN, deadline, cancel)?,
            Err(Errno::INTR) => {}
            Err(_) => return Err(IoFail::Other),
        }
    }
    Ok(())
}

fn write_all(fd: impl AsFd, mut buf: &[u8], deadline: Instant, cancel: Option<&CancelFlag>) -> Result<(), IoFail> {
    while !buf.is_empty() {
        match rustix::io::write(&fd, buf) {
            Ok(n) => buf = &buf[n..],
            Err(Errno::AGAIN) => wait_ready(&fd, PollFlags::OUT, deadline, cancel)?,
            Err(Errno::INTR) => {}
            Err(Errno::PIPE) => return Err(IoFail::Eof),
            Err(_) => return Err(IoFail::Other),
        }
    }
    Ok(())
}

fn set_nonblocking(fd: impl AsFd) -> std::io::Result<()> {
    let fl = rustix::fs::fcntl_getfl(&fd)?;
    rustix::fs::fcntl_setfl(&fd, fl | rustix::fs::OFlags::NONBLOCK)?;
    Ok(())
}

/// İsteğin girdisi.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Payload<'a> {
    Bytes(&'a [u8]),
    /// Açık dosyanın ilk `len` baytı, [`STREAM_CHUNK`] parçalarla `read_at` → boru.
    File { file: &'a File, len: u64 },
}

impl Payload<'_> {
    pub fn len(&self) -> u64 {
        match self {
            Self::Bytes(b) => b.len() as u64,
            Self::File { len, .. } => *len,
        }
    }
}

/// İsteğin işçiyi düşüren sonucu (işçi artık kullanılamaz, öldürülmeli).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestFail {
    Sandbox(SandboxFailure),
    /// Çağıran iptal etti (başarısızlık sayılmaz).
    Cancelled,
    /// Dosya akış sırasında okunamadı ya da kısaldı (girdinin değil dosya sisteminin sorunu).
    Input,
}

/// Başarılı yanıt: tek kare, animasyon bayrağı ve (tel v4) animasyon bilgisi.
pub(crate) type Answer = nuxut_image::DecodedFile;

/// Tek kullanımlık işçi (bulgu 3): girdi görmemiş taze süreç; bir isteğe hizmet eder, sonra
/// öldürülür. Önceden ısıtılmış olabilir (el sıkışma henüz okunmamış).
#[derive(Debug)]
pub(crate) struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    /// Okunup kabul edilmiş el sıkışma; `None` = henüz doğrulanmadı.
    verified: Option<Handshake>,
    /// Başlatıldığı (ya da havuza konduğu) an; boşta kapatma buna göre.
    pub last_used: Instant,
}

/// Başlatma başarısız: işçi kullanılamıyor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpawnError {
    /// İkili çalıştırılamadı.
    Exec,
    /// Kısıtlamalar kurulamadı (çıkış kodu 70).
    Lockdown,
    /// El sıkışma tutmadı (imza, sürüm, zaman aşımı, erken çıkış).
    Handshake,
    /// El sıkışma geldi ama kilit/profil beyanı kabul edilmedi.
    Rejected(HandshakeReject),
}

impl Worker {
    /// Süreci `profile` ile başlatır, el sıkışmayı BEKLEMEZ (önceden ısıtma: kilitlenme arka
    /// planda sürer). Kullanmadan önce [`verify`](Self::verify) zorunludur.
    pub fn start(path: &Path, profile: WorkerProfile, args: &[String]) -> Result<Self, SpawnError> {
        let mut child = Command::new(path)
            .arg(profile.arg())
            .args(args)
            .env_clear()
            .current_dir("/")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| SpawnError::Exec)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            reap(&mut child);
            return Err(SpawnError::Exec);
        };
        let mut w = Self { child, stdin, stdout, verified: None, last_used: Instant::now() };
        if set_nonblocking(&w.stdin).is_err() || set_nonblocking(&w.stdout).is_err() {
            w.kill();
            return Err(SpawnError::Exec);
        }
        Ok(w)
    }

    /// İşçinin PID'i (yalnızca testler/tanı; loglanmaz).
    #[allow(dead_code)]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// El sıkışmayı okur ve kabul kuralını uygular (bulgu 2a, profil); zaten doğrulanmışsa
    /// hiçbir şey yapmaz. Hata: işçi öldürülür.
    pub fn verify(&mut self, deadline: Instant, profile: WorkerProfile, parent_has_hooks: bool, allow_unconfined: bool) -> Result<Handshake, SpawnError> {
        if let Some(hs) = self.verified {
            return Ok(hs);
        }
        match self.read_handshake(deadline) {
            Ok(hs) => match hs.accept(profile, parent_has_hooks, allow_unconfined) {
                Ok(()) => {
                    self.verified = Some(hs);
                    Ok(hs)
                }
                Err(r) => {
                    self.kill();
                    Err(SpawnError::Rejected(r))
                }
            },
            Err(()) => {
                let status = self.kill();
                if status.and_then(|s| s.code()) == Some(crate::EXIT_LOCKDOWN) {
                    Err(SpawnError::Lockdown)
                } else {
                    Err(SpawnError::Handshake)
                }
            }
        }
    }

    fn read_handshake(&mut self, deadline: Instant) -> Result<Handshake, ()> {
        let mut head = [0u8; HANDSHAKE_HEADER_LEN];
        read_exact(&self.stdout, &mut head, deadline, None).map_err(|_| ())?;
        let (hs, n) = Handshake::decode_header(&head).ok_or(())?;
        let mut v = [0u8; protocol::MAX_VERSION_LEN];
        read_exact(&self.stdout, &mut v[..n], deadline, None).map_err(|_| ())?;
        if &v[..n] != protocol::CRATE_VERSION.as_bytes() {
            return Err(());
        }
        Ok(hs)
    }

    fn send_payload(&mut self, payload: Payload<'_>, deadline: Instant, cancel: Option<&CancelFlag>) -> Result<(), RequestFail> {
        let io = |e: IoFail| match e {
            IoFail::Timeout => RequestFail::Sandbox(SandboxFailure::Timeout),
            IoFail::Cancelled => RequestFail::Cancelled,
            IoFail::Eof | IoFail::Other => RequestFail::Sandbox(SandboxFailure::Killed),
        };
        match payload {
            Payload::Bytes(b) => write_all(&self.stdin, b, deadline, cancel).map_err(io),
            Payload::File { file, len } => {
                let mut buf = vec![0u8; STREAM_CHUNK.min(usize::try_from(len).unwrap_or(STREAM_CHUNK)).max(1)];
                let mut off = 0u64;
                while off < len {
                    let n = buf.len().min(usize::try_from(len - off).unwrap_or(usize::MAX));
                    file.read_exact_at(&mut buf[..n], off).map_err(|_| RequestFail::Input)?;
                    write_all(&self.stdin, &buf[..n], deadline, cancel).map_err(io)?;
                    off += n as u64;
                }
                Ok(())
            }
        }
    }

    /// Tek istek. Dış `Err`: işçi artık kullanılamaz (öldürülmeli); iç `Err`: işçinin bildirdiği
    /// çözme hatası (işçi sağlam). `input_limit` yalnızca `FileTooLarge` iletisinin sınırıdır.
    pub fn request(
        &mut self,
        req: &Request,
        payload: Payload<'_>,
        deadline: Instant,
        cancel: Option<&CancelFlag>,
        input_limit: u64,
    ) -> Result<Result<Answer, ImageError>, RequestFail> {
        let io = |e: IoFail| match e {
            IoFail::Timeout => RequestFail::Sandbox(SandboxFailure::Timeout),
            IoFail::Cancelled => RequestFail::Cancelled,
            IoFail::Eof | IoFail::Other => RequestFail::Sandbox(SandboxFailure::Killed),
        };
        write_all(&self.stdin, &req.encode(), deadline, cancel).map_err(io)?;
        self.send_payload(payload, deadline, cancel)?;
        let mut head = [0u8; RESPONSE_HEADER_LEN];
        read_exact(&self.stdout, &mut head, deadline, cancel).map_err(io)?;
        let resp = Response::decode(&head);
        let len = resp.validate(req).map_err(|_| RequestFail::Sandbox(SandboxFailure::Protocol))?;
        let Some(len) = len else {
            let code = protocol::ErrorCode::from_code(resp.error).ok_or(RequestFail::Sandbox(SandboxFailure::Protocol))?;
            return Ok(Err(code.into_error(resp.width, resp.height, payload.len(), input_limit)));
        };
        let mut pixels = vec![0u8; len];
        match read_exact(&self.stdout, &mut pixels, deadline, cancel) {
            Ok(()) => {}
            // Kısa yanıt ve çıkış: sözleşme ihlali.
            Err(IoFail::Eof) => return Err(RequestFail::Sandbox(SandboxFailure::Protocol)),
            Err(e) => return Err(io(e)),
        }
        // Fazla bayt (ya da yanıttan hemen sonra kapanma) ihlaldir.
        let mut probe = [PollFd::new(&self.stdout, PollFlags::IN)];
        if poll(&mut probe, Some(&timespec(Duration::ZERO))).is_ok_and(|n| n > 0) {
            return Err(RequestFail::Sandbox(SandboxFailure::Protocol));
        }
        Image::from_premultiplied(resp.width, resp.height, pixels)
            .map(|image| Ok(Answer { image, animated_source: resp.animated_source(), animation: resp.animation() }))
            .map_err(|_| RequestFail::Sandbox(SandboxFailure::Protocol))
    }

    /// İşçiyi öldürür ve bekler (zombi kalmaz); çıkış durumunu döner.
    pub fn kill(&mut self) -> Option<ExitStatus> {
        reap(&mut self.child)
    }

    /// Bir hatadan sonra: işçiyi öldürür, beklenmedik çıkışı `Killed`'e, temiz çıkışı (0)
    /// `Protocol`'e eşler; zaman aşımı/protokol olduğu gibi kalır.
    pub fn kill_classify(&mut self, failure: SandboxFailure) -> (SandboxFailure, Option<ExitStatus>) {
        // Ölmüşse gerçek nedeni (sinyal/kod) görmek için önce kısa bir bekleyiş.
        let mut status = None;
        if failure == SandboxFailure::Killed {
            let until = Instant::now() + Duration::from_millis(200);
            while Instant::now() < until {
                if let Ok(Some(s)) = self.child.try_wait() {
                    status = Some(s);
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let status = status.or_else(|| self.kill());
        let failure = match (failure, status) {
            (SandboxFailure::Killed, Some(s)) if s.code() == Some(0) => SandboxFailure::Protocol,
            (f, _) => f,
        };
        (failure, status)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        reap(&mut self.child);
    }
}

fn reap(child: &mut Child) -> Option<ExitStatus> {
    if let Ok(Some(s)) = child.try_wait() {
        return Some(s);
    }
    let _ = child.kill();
    child.wait().ok()
}

/// Çıkış durumunun log için kısa biçimi (içerik yok).
pub(crate) fn describe(status: Option<ExitStatus>) -> String {
    match status {
        Some(s) => match (s.code(), s.signal()) {
            (Some(c), _) => format!("kod {c}"),
            (_, Some(sig)) => format!("sinyal {sig}"),
            _ => "bilinmiyor".into(),
        },
        None => "bilinmiyor".into(),
    }
}
