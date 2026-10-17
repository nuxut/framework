//! Tek örnek + komut IPC.
//!
//! `$XDG_RUNTIME_DIR/nuxut/<app>.sock` üzerinde bağlantı başına tek istek: istemci argümanları
//! `\t` ile ayırıp tek satır (`\n` ile biten, ≤ 4 KiB) yazar, sunucu yanıtı yazıp kapatır.
//! Hangi örneğin birincil olduğuna `<app>.lock` üzerindeki `flock` karar verir.
//!
//! Komutlar asla shell'e verilmez (değişmez 8): uygulama yalnızca argüman vektörü alır.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{DirBuilder, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};

/// İstek satırının en büyük boyu (`\n` hariç).
pub const MAX_LINE: usize = 4096;
/// Yanıtın en büyük boyu; fazlası kesilir.
pub const MAX_REPLY: usize = 64 * 1024;
/// Eşzamanlı bağlantı (okunmakta olan + yanıt bekleyen) sınırı.
pub const MAX_CONNECTIONS: usize = 16;
/// Varsayılan sunucu yanıt penceresi ([`Listener::with_reply_timeout`]) ve istek başına
/// istemci zaman aşımı ([`Client::with_timeout`]). Sunucu yanıtı bağlantı kabulünden pencere
/// içinde yazmalıdır ([`Reply::send`]); istemci zaman aşımı sunucunun penceresini uzatmaz,
/// pencereyi uzatan uygulamanın istemcileri de en az o kadar beklemelidir.
pub const TIMEOUT: Duration = Duration::from_secs(2);
/// İstemci zaman aşımının alt ve üst sınırı ([`Client::with_timeout`] bu aralığa kırpar).
pub const MIN_CLIENT_TIMEOUT: Duration = Duration::from_millis(10);
pub const MAX_CLIENT_TIMEOUT: Duration = Duration::from_secs(30);
/// Sunucu yanıt penceresinin alt ve üst sınırı ([`Listener::with_reply_timeout`]); istemciyle
/// aynı aralık, böylece her pencere için yeterli bir istemci zaman aşımı seçilebilir.
pub const MIN_REPLY_TIMEOUT: Duration = MIN_CLIENT_TIMEOUT;
pub const MAX_REPLY_TIMEOUT: Duration = MAX_CLIENT_TIMEOUT;
/// İkincil örnek, birincil henüz bind etmemişse bu süre boyunca yeniden dener.
const CONNECT_RETRY: Duration = Duration::from_millis(500);

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("geçersiz uygulama adı ([a-z0-9_-]{{1,32}} olmalı)")]
    InvalidName,
    #[error("argüman sekme, satır sonu ya da NUL içeremez")]
    InvalidArgument,
    #[error("XDG_RUNTIME_DIR tanımlı değil")]
    NoRuntimeDir,
    #[error("güvensiz IPC yolu: {0}")]
    Insecure(String),
    #[error("uygulama çalışmıyor")]
    NotRunning,
    #[error("zaman aşımı")]
    Timeout,
    #[error("satır çok uzun (en çok {MAX_LINE} bayt)")]
    TooLong,
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// `try_single_instance` sonucu.
#[derive(Debug)]
pub enum Instance {
    /// Kilit bizde, socket bağlandı; `RunOptions::ipc`'ye verilir.
    Primary(Listener),
    /// Başka bir örnek çalışıyor; komutu ona iletip çıkın.
    Secondary(Client),
}

/// Birincil örneğin socket'i ve kilidi. Drop'ta socket dosyası (hâlâ bizimse) silinir.
#[derive(Debug)]
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
    /// Silerken başkasının socket'ini silmemek için (dev, inode).
    id: (u64, u64),
    /// Bağlantı kabulünden yanıta dek pencere (varsayılan [`TIMEOUT`]).
    reply_timeout: Duration,
    _lock: File,
}

impl Listener {
    /// Sunucunun yanıt penceresi: [`Reply::send`] bağlantı kabulünden bu süre içinde
    /// çağrılmalıdır, sonrası düşer. [`MIN_REPLY_TIMEOUT`]..=[`MAX_REPLY_TIMEOUT`] aralığına
    /// kırpılır; varsayılan [`TIMEOUT`]. İstek satırının okunması bundan bağımsız olarak en çok
    /// `min(pencere, TIMEOUT)` bekler (yazmayan istemci bağlantı yerini uzun tutmaz). Pencereyi
    /// uzatan uygulamanın istemcileri [`Client::with_timeout`] ile en az bu kadar beklemelidir.
    #[must_use]
    pub fn with_reply_timeout(mut self, timeout: Duration) -> Self {
        self.reply_timeout = timeout.clamp(MIN_REPLY_TIMEOUT, MAX_REPLY_TIMEOUT);
        self
    }

    pub fn reply_timeout(&self) -> Duration {
        self.reply_timeout
    }
}

impl std::os::fd::AsFd for Listener {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.listener.as_fd()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        if let Ok(meta) = std::fs::symlink_metadata(&self.path)
            && (meta.dev(), meta.ino()) == self.id
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Çalışan birincil örneğe istemci. Zaman aşımı istemci başınadır (varsayılan [`TIMEOUT`]).
#[derive(Debug, Clone)]
pub struct Client {
    path: PathBuf,
    /// Bağlanamazsa (socket yok/reddedildi) bu süre boyunca yeniden dener.
    retry: Duration,
    timeout: Duration,
}

impl Client {
    /// `app`'in birincil örneğine istemci (bağlanmaz; her `send` yeni bağlantıdır). Birincil
    /// yoksa `send` hemen [`IpcError::NotRunning`] döner.
    pub fn new(app: &str) -> Result<Self, IpcError> {
        let dir = ipc_dir()?;
        check_dir(&dir)?;
        Ok(Self::at(socket_path(&dir, app)?, Duration::ZERO))
    }

    fn at(path: PathBuf, retry: Duration) -> Self {
        Self {
            path,
            retry,
            timeout: TIMEOUT,
        }
    }

    /// İstek başına zaman aşımı (bağlanma + yazma + yanıt okuma toplamı);
    /// [`MIN_CLIENT_TIMEOUT`]..=[`MAX_CLIENT_TIMEOUT`] aralığına kırpılır.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.clamp(MIN_CLIENT_TIMEOUT, MAX_CLIENT_TIMEOUT);
        self
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Ham satır gönderir (argümanlar zaten `\t` ile ayrılmış olmalı).
    pub fn send(&self, line: &str) -> Result<String, IpcError> {
        request(&self.path, line, self.retry, self.timeout)
    }

    pub fn send_args(&self, args: &[impl AsRef<str>]) -> Result<String, IpcError> {
        self.send(&encode_args(args)?)
    }
}

/// CLI'lar için: `app`'in birincil örneğine argümanları gönderir ve yanıtı döndürür.
pub fn send_args(app: &str, args: &[impl AsRef<str>]) -> Result<String, IpcError> {
    send(app, &encode_args(args)?)
}

/// Ham satır gönderir.
pub fn send(app: &str, line: &str) -> Result<String, IpcError> {
    Client::new(app)?.send(line)
}

/// Birincil örnek mi, değil mi? Birincilse socket bağlanmış bir `Listener` döner.
pub fn try_single_instance(app: &str) -> Result<Instance, IpcError> {
    try_single_instance_in(&ipc_dir()?, app)
}

fn ipc_dir() -> Result<PathBuf, IpcError> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or(IpcError::NoRuntimeDir)?;
    let runtime = PathBuf::from(runtime);
    if !runtime.is_absolute() {
        return Err(IpcError::NoRuntimeDir);
    }
    Ok(runtime.join("nuxut"))
}

fn valid_name(app: &str) -> bool {
    (1..=32).contains(&app.len())
        && app
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn socket_path(dir: &Path, app: &str) -> Result<PathBuf, IpcError> {
    if !valid_name(app) {
        return Err(IpcError::InvalidName);
    }
    Ok(dir.join(format!("{app}.sock")))
}

/// Bu sürecin (etkin) uid'i: `/proc/self`'in sahibi.
fn own_uid() -> io::Result<u32> {
    Ok(std::fs::metadata("/proc/self")?.uid())
}

/// Dizin symlink değil, bizim ve yalnızca bize açık olmalı.
fn check_dir(dir: &Path) -> Result<(), IpcError> {
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(IpcError::NotRunning),
        Err(e) => return Err(e.into()),
    };
    if !meta.file_type().is_dir() {
        return Err(IpcError::Insecure(format!("{} dizin değil", dir.display())));
    }
    if meta.uid() != own_uid()? {
        return Err(IpcError::Insecure(format!(
            "{} başka kullanıcıya ait",
            dir.display()
        )));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(IpcError::Insecure(format!(
            "{} izinleri {:o}",
            dir.display(),
            meta.mode() & 0o777
        )));
    }
    Ok(())
}

pub(crate) fn try_single_instance_in(dir: &Path, app: &str) -> Result<Instance, IpcError> {
    let sock = socket_path(dir, app)?;
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    check_dir(dir)?;

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join(format!("{app}.lock")))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Ok(Instance::Secondary(Client::at(sock, CONNECT_RETRY)));
        }
        Err(TryLockError::Error(e)) => return Err(e.into()),
    }

    // Kilit bizde: kalan socket dosyası önceki (ölmüş) birincildendir.
    match std::fs::symlink_metadata(&sock) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == own_uid()? => {
            std::fs::remove_file(&sock)?
        }
        Ok(_) => {
            return Err(IpcError::Insecure(format!(
                "{} socket değil ya da bizim değil",
                sock.display()
            )));
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let listener = UnixListener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let meta = std::fs::symlink_metadata(&sock)?;
    Ok(Instance::Primary(Listener {
        listener,
        path: sock,
        id: (meta.dev(), meta.ino()),
        reply_timeout: TIMEOUT,
        _lock: lock,
    }))
}

fn encode_args(args: &[impl AsRef<str>]) -> Result<String, IpcError> {
    let mut line = String::new();
    for (i, arg) in args.iter().enumerate() {
        let arg = arg.as_ref();
        if arg.contains(['\t', '\n', '\0']) {
            return Err(IpcError::InvalidArgument);
        }
        if i > 0 {
            line.push('\t');
        }
        line.push_str(arg);
    }
    Ok(line)
}

fn decode_args(line: &str) -> Vec<String> {
    if line.is_empty() {
        Vec::new()
    } else {
        line.split('\t').map(str::to_owned).collect()
    }
}

fn request(
    path: &Path,
    line: &str,
    retry: Duration,
    timeout: Duration,
) -> Result<String, IpcError> {
    if line.contains('\n') {
        return Err(IpcError::InvalidArgument);
    }
    if line.len() > MAX_LINE {
        return Err(IpcError::TooLong);
    }
    let start = Instant::now();
    let remaining = |start: Instant| {
        timeout
            .checked_sub(start.elapsed())
            .filter(|d| !d.is_zero())
    };

    let mut stream = loop {
        match UnixStream::connect(path) {
            Ok(stream) => break stream,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                if start.elapsed() >= retry {
                    return Err(IpcError::NotRunning);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    };
    let timed_out = |e: io::Error| match e.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => IpcError::Timeout,
        _ => IpcError::Io(e),
    };

    stream.set_write_timeout(Some(remaining(start).ok_or(IpcError::Timeout)?))?;
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    stream.write_all(&buf).map_err(timed_out)?;

    let mut reply = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        stream.set_read_timeout(Some(remaining(start).ok_or(IpcError::Timeout)?))?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                let take = n.min(MAX_REPLY - reply.len());
                reply.extend_from_slice(&chunk[..take]);
                if reply.len() >= MAX_REPLY {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(timed_out(e)),
        }
    }
    Ok(String::from_utf8_lossy(&reply).into_owned())
}

/// Bağlantı sayacındaki bir yer; düşünce serbest kalır.
#[derive(Debug)]
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn acquire(count: &Arc<AtomicUsize>) -> Option<Self> {
        count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CONNECTIONS).then_some(n + 1)
            })
            .ok()
            .map(|_| Self(count.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Bir komutun yanıt kanalı. Başka thread'e taşınabilir; zaman aşımına kadar yanıtlanmalı.
/// Yanıtlanmadan düşerse istemci boş yanıt alır.
#[derive(Debug)]
pub struct Reply {
    stream: UnixStream,
    deadline: Instant,
    _slot: Slot,
}

impl Reply {
    /// Yanıtı yazar ve bağlantıyı kapatır. 64 KiB'tan uzun metin kesilir; zaman aşımından
    /// sonra çağrılırsa sessizce düşer.
    pub fn send(self, text: &str) {
        let Some(remaining) = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
        else {
            tracing::debug!("IPC yanıtı zaman aşımından sonra geldi; düşürüldü");
            return;
        };
        let mut end = text.len().min(MAX_REPLY);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut stream = &self.stream;
        let result = stream
            .set_nonblocking(false)
            .and_then(|()| stream.set_write_timeout(Some(remaining)))
            .and_then(|()| stream.write_all(&text.as_bytes()[..end]))
            .and_then(|()| stream.shutdown(std::net::Shutdown::Write));
        if let Err(e) = result {
            tracing::debug!("IPC yanıtı yazılamadı: {e}");
        }
    }
}

#[derive(Default)]
struct Connections {
    next: u64,
    /// Bağlantı → zaman aşımı timer'ı.
    timers: HashMap<u64, RegistrationToken>,
}

/// `listener`'ı döngüye bağlar; her geçerli istek `on_command`'a gider. Dönen token kapanışta
/// kaldırılmalı: döngünün kendisi (sctk klavye tekrarı gibi başka tutamaç döngüleri yüzünden)
/// hiç düşmeyebilir; kaynağı kaldırmak `Listener`'ı düşürür ve socket dosyası silinir.
pub(crate) fn serve<D: 'static>(
    handle: &LoopHandle<'static, D>,
    listener: Listener,
    on_command: impl Fn(&mut D, Vec<String>, Reply) + 'static,
) -> io::Result<RegistrationToken> {
    let window = listener.reply_timeout;
    let count = Arc::new(AtomicUsize::new(0));
    let conns: Rc<RefCell<Connections>> = Rc::default();
    let on_command = Rc::new(on_command);
    // Kapanışlar döngünün içinde yaşar: güçlü `LoopHandle` Rc döngüsü kurar ve döngü (dolayısıyla
    // Listener'ın socket'i silen Drop'u) hiç düşmez. Bu yüzden her yerde zayıf tutamaç.
    let loop_handle = handle.downgrade();
    let source = Generic::new(listener, Interest::READ, Mode::Level);
    let token = handle
        .insert_source(source, move |_, listener, _| {
            loop {
                let stream = match listener.listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!("IPC accept: {e}");
                        break;
                    }
                };
                // Sınır aşıldıysa bağlantı hemen kapanır (stream düşer).
                let Some(slot) = Slot::acquire(&count) else {
                    tracing::warn!("IPC: eşzamanlı bağlantı sınırı ({MAX_CONNECTIONS}) aşıldı");
                    continue;
                };
                let Some(loop_handle) = loop_handle.upgrade() else {
                    break;
                };
                if let Err(e) =
                    accept_connection(&loop_handle, &conns, &on_command, stream, slot, window)
                {
                    tracing::warn!("IPC bağlantısı kurulamadı: {e}");
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| io::Error::other(e.error.to_string()))?;
    tracing::debug!("IPC dinleniyor");
    Ok(token)
}

fn accept_connection<D: 'static, F: Fn(&mut D, Vec<String>, Reply) + 'static>(
    handle: &LoopHandle<'static, D>,
    conns: &Rc<RefCell<Connections>>,
    on_command: &Rc<F>,
    stream: UnixStream,
    slot: Slot,
    window: Duration,
) -> io::Result<()> {
    stream.set_nonblocking(true)?;
    let accepted = Instant::now();
    // Yanıt penceresi uygulamanındır; satırı okuma süresi varsayılanı aşmaz.
    let deadline = accepted + window;
    let read_deadline = accepted + window.min(TIMEOUT);
    let id = {
        let mut c = conns.borrow_mut();
        c.next += 1;
        c.next
    };
    let mut buf: Vec<u8> = Vec::with_capacity(256);
    let mut slot = Some(slot);
    // Okuma bitti/koptu: zaman aşımı timer'ını kaldır (zayıf tutamaçla, bkz. `serve`).
    let finish = {
        let conns = conns.clone();
        let handle = handle.downgrade();
        move || {
            if let Some(timer) = conns.borrow_mut().timers.remove(&id)
                && let Some(handle) = handle.upgrade()
            {
                handle.remove(timer);
            }
        }
    };
    let on_command = on_command.clone();
    let source = Generic::new(stream, Interest::READ, Mode::Level);
    let token = handle
        .insert_source(source, move |_, stream, data| {
            let stream: &UnixStream = stream;
            let mut chunk = [0u8; 1024];
            let line = loop {
                match (&*stream).read(&mut chunk) {
                    Ok(0) => {
                        // EOF: satır sonu olmadan da kabul et; boşsa istemci vazgeçti.
                        if buf.is_empty() {
                            finish();
                            return Ok(PostAction::Remove);
                        }
                        break Ok(std::mem::take(&mut buf));
                    }
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                            buf.truncate(pos);
                            break if pos > MAX_LINE {
                                Err("satır çok uzun")
                            } else {
                                Ok(std::mem::take(&mut buf))
                            };
                        }
                        if buf.len() > MAX_LINE {
                            break Err("satır çok uzun");
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        return Ok(PostAction::Continue);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        finish();
                        return Ok(PostAction::Remove);
                    }
                }
            };
            finish();
            let (Some(slot), Ok(clone)) = (slot.take(), stream.try_clone()) else {
                return Ok(PostAction::Remove);
            };
            let reply = Reply {
                stream: clone,
                deadline,
                _slot: slot,
            };
            match line.map(String::from_utf8) {
                Ok(Ok(line)) => on_command(data, decode_args(&line), reply),
                Ok(Err(_)) => reply.send("error: satır UTF-8 değil"),
                Err(msg) => reply.send(&format!("error: {msg} (en çok {MAX_LINE} bayt)")),
            }
            Ok(PostAction::Remove)
        })
        .map_err(|e| io::Error::other(e.error.to_string()))?;

    // Zaman aşımı: okuma bitmediyse bağlantıyı kapat (kaynak düşünce slot da serbest kalır).
    let conns_t = conns.clone();
    let handle_t = handle.downgrade();
    let timer = handle
        .insert_source(Timer::from_deadline(read_deadline), move |_, _, _| {
            if conns_t.borrow_mut().timers.remove(&id).is_some()
                && let Some(handle) = handle_t.upgrade()
            {
                tracing::debug!(id, "IPC bağlantısı zaman aşımına uğradı");
                handle.remove(token);
            }
            TimeoutAction::Drop
        })
        .map_err(|e| io::Error::other(e.error.to_string()));
    match timer {
        Ok(timer) => {
            conns.borrow_mut().timers.insert(id, timer);
            Ok(())
        }
        Err(e) => {
            handle.remove(token);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU32;

    use super::*;

    /// Test başına `0700` geçici dizin (XDG_RUNTIME_DIR'e dokunmadan); düşünce silinir.
    struct TempDir(PathBuf);

    impl std::ops::Deref for TempDir {
        type Target = Path;
        fn deref(&self) -> &Path {
            &self.0
        }
    }

    impl AsRef<Path> for TempDir {
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if let Some(base) = self.0.parent() {
                let _ = std::fs::remove_dir_all(base);
            }
        }
    }

    fn temp_dir() -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let base = std::env::temp_dir().join(format!(
            "nuxut-ipc-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&base)
            .unwrap();
        TempDir(base.join("nuxut"))
    }

    #[test]
    fn names_and_args() {
        assert!(valid_name("nuxut-shell"));
        assert!(!valid_name(""));
        assert!(!valid_name("../x"));
        assert!(!valid_name("Shell"));
        assert_eq!(
            encode_args(&["panel", "toggle", "sağ kenar"]).unwrap(),
            "panel\ttoggle\tsağ kenar"
        );
        assert!(matches!(
            encode_args(&["a\tb"]),
            Err(IpcError::InvalidArgument)
        ));
        assert!(matches!(
            encode_args(&["a\nb"]),
            Err(IpcError::InvalidArgument)
        ));
        assert!(matches!(
            encode_args(&["a\0"]),
            Err(IpcError::InvalidArgument)
        ));
        assert_eq!(decode_args(""), Vec::<String>::new());
        assert_eq!(decode_args("a\t\tb"), vec!["a", "", "b"]);
    }

    #[test]
    fn single_instance_and_stale_socket() {
        let dir = temp_dir();
        let Instance::Primary(primary) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!("birincil olmalı")
        };
        assert!(matches!(
            try_single_instance_in(&dir, "t").unwrap(),
            Instance::Secondary(_)
        ));
        let mode = std::fs::metadata(dir.join("t.sock")).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
        drop(primary);
        assert!(!dir.join("t.sock").exists(), "Drop socket'i silmeli");

        // Ölmüş birincilden kalan socket dosyası: kilit boşta → temizlenip yeniden bağlanır.
        drop(UnixListener::bind(dir.join("t.sock")).unwrap());
        assert!(dir.join("t.sock").exists());
        assert!(matches!(
            try_single_instance_in(&dir, "t").unwrap(),
            Instance::Primary(_)
        ));
    }

    #[test]
    fn insecure_dir_is_rejected() {
        let dir = temp_dir();
        DirBuilder::new().mode(0o755).create(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(
            try_single_instance_in(&dir, "t"),
            Err(IpcError::Insecure(_))
        ));
    }

    #[test]
    fn request_reply_roundtrip() {
        let dir = temp_dir();
        let Instance::Primary(listener) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!()
        };
        let Instance::Secondary(client) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!()
        };

        let mut event_loop: calloop::EventLoop<'static, Vec<Vec<String>>> =
            calloop::EventLoop::try_new().unwrap();
        serve(
            &event_loop.handle(),
            listener,
            |got: &mut Vec<Vec<String>>, args, reply| {
                match args.first().map(String::as_str) {
                    Some("echo") => reply.send(&args[1..].join(" ")),
                    Some("silent") => drop(reply),
                    _ => reply.send("error: bilinmeyen"),
                }
                got.push(args);
            },
        )
        .unwrap();

        let worker = std::thread::spawn(move || {
            let a = client.send_args(&["echo", "merhaba", "dünya bir"]).unwrap();
            let b = client.send_args(&["silent"]).unwrap();
            let c = client.send(&"x".repeat(MAX_LINE + 1));
            let d = client.send_args(&[] as &[&str]).unwrap();
            (a, b, c, d)
        });
        let mut got = Vec::new();
        let start = Instant::now();
        while !worker.is_finished() && start.elapsed() < Duration::from_secs(5) {
            event_loop
                .dispatch(Duration::from_millis(20), &mut got)
                .unwrap();
        }
        let (a, b, c, d) = worker.join().unwrap();
        assert_eq!(a, "merhaba dünya bir");
        assert_eq!(b, "");
        assert!(matches!(c, Err(IpcError::TooLong)));
        assert_eq!(d, "error: bilinmeyen");
        assert_eq!(
            got,
            vec![
                vec!["echo".to_owned(), "merhaba".into(), "dünya bir".into()],
                vec!["silent".into()],
                vec![]
            ]
        );
        // Döngü düşünce Listener da düşmeli (Rc döngüsü yok) ve socket silinmeli.
        drop(event_loop);
        assert!(
            !dir.join("t.sock").exists(),
            "döngü düşünce socket silinmeli"
        );
    }

    #[test]
    fn client_timeout_is_per_client_and_clamped() {
        let dir = temp_dir();
        DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("t.sock");
        // Kabul eden (çekirdek kuyruğu) ama hiç yanıtlamayan sunucu.
        let _silent = UnixListener::bind(&path).unwrap();
        let client = Client::at(path.clone(), Duration::ZERO);
        assert_eq!(client.timeout(), TIMEOUT);

        let fast = client.clone().with_timeout(Duration::from_millis(150));
        assert_eq!(fast.timeout(), Duration::from_millis(150));
        let t = Instant::now();
        assert!(matches!(fast.send("x"), Err(IpcError::Timeout)));
        let took = t.elapsed();
        assert!(
            took >= Duration::from_millis(140) && took < Duration::from_millis(1500),
            "{took:?}"
        );
        // Klon ayrı: özgün istemcinin varsayılanı değişmez.
        assert_eq!(client.timeout(), TIMEOUT);

        assert_eq!(
            client.clone().with_timeout(Duration::ZERO).timeout(),
            MIN_CLIENT_TIMEOUT
        );
        assert_eq!(
            client
                .clone()
                .with_timeout(Duration::from_secs(3600))
                .timeout(),
            MAX_CLIENT_TIMEOUT
        );
        assert_eq!(
            client.with_timeout(Duration::MAX).timeout(),
            MAX_CLIENT_TIMEOUT
        );

        // Birincil yok: zaman aşımından bağımsız, hemen NotRunning.
        let gone =
            Client::at(dir.join("yok.sock"), Duration::ZERO).with_timeout(MAX_CLIENT_TIMEOUT);
        let t = Instant::now();
        assert!(matches!(gone.send("x"), Err(IpcError::NotRunning)));
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn overlong_raw_line_gets_error_and_idle_connection_times_out() {
        let dir = temp_dir();
        let Instance::Primary(listener) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!()
        };
        let path = dir.join("t.sock");
        let mut event_loop: calloop::EventLoop<'static, u32> =
            calloop::EventLoop::try_new().unwrap();
        serve(&event_loop.handle(), listener, |n: &mut u32, _, reply| {
            *n += 1;
            reply.send("ok");
        })
        .unwrap();

        let p = path.clone();
        let worker = std::thread::spawn(move || {
            // İstemci denetimini atlayıp sunucuya doğrudan uzun satır yaz.
            let mut s = UnixStream::connect(&p).unwrap();
            s.write_all(&vec![b'a'; MAX_LINE + 10]).unwrap();
            let mut long = String::new();
            s.read_to_string(&mut long).unwrap();
            // Hiç yazmayan bağlantı zaman aşımında kapanır (EOF).
            let mut idle = UnixStream::connect(&p).unwrap();
            let t = Instant::now();
            let mut rest = Vec::new();
            idle.read_to_end(&mut rest).unwrap();
            (long, t.elapsed())
        });
        let mut n = 0;
        let start = Instant::now();
        while !worker.is_finished() && start.elapsed() < Duration::from_secs(6) {
            event_loop
                .dispatch(Duration::from_millis(20), &mut n)
                .unwrap();
        }
        let (long, idle_for) = worker.join().unwrap();
        assert!(long.starts_with("error:"), "{long}");
        assert!(
            idle_for >= Duration::from_millis(1500) && idle_for < Duration::from_secs(4),
            "{idle_for:?}"
        );
        assert_eq!(n, 0);
    }

    /// Yanıtları `delay` sonra yazan sunucuya `client` ile iki istek: (yanıt, süre).
    fn delayed_reply(
        window: Option<Duration>,
        delay: Duration,
        client_timeout: Duration,
    ) -> (Result<String, IpcError>, Duration) {
        let dir = temp_dir();
        let Instance::Primary(mut listener) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!()
        };
        if let Some(window) = window {
            listener = listener.with_reply_timeout(window);
        }
        let Instance::Secondary(client) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!()
        };
        let client = client.with_timeout(client_timeout);

        let mut event_loop: calloop::EventLoop<'static, Vec<(Instant, Reply)>> =
            calloop::EventLoop::try_new().unwrap();
        serve(
            &event_loop.handle(),
            listener,
            move |held: &mut Vec<_>, _, reply| {
                held.push((Instant::now() + delay, reply));
            },
        )
        .unwrap();

        let worker = std::thread::spawn(move || {
            let t = Instant::now();
            (client.send("yavaş"), t.elapsed())
        });
        let mut held = Vec::new();
        let start = Instant::now();
        while !worker.is_finished() && start.elapsed() < Duration::from_secs(10) {
            event_loop
                .dispatch(Duration::from_millis(10), &mut held)
                .unwrap();
            let now = Instant::now();
            let (due, rest): (Vec<_>, Vec<_>) = held.drain(..).partition(|(at, _)| *at <= now);
            held = rest;
            for (_, reply) in due {
                reply.send("geç ama geçerli");
            }
        }
        worker.join().unwrap()
    }

    #[test]
    fn reply_window_is_configurable_and_clamped() {
        let dir = temp_dir();
        let Instance::Primary(listener) = try_single_instance_in(&dir, "t").unwrap() else {
            panic!()
        };
        assert_eq!(listener.reply_timeout(), TIMEOUT);
        let listener = listener.with_reply_timeout(Duration::ZERO);
        assert_eq!(listener.reply_timeout(), MIN_REPLY_TIMEOUT);
        let listener = listener.with_reply_timeout(Duration::MAX);
        assert_eq!(listener.reply_timeout(), MAX_REPLY_TIMEOUT);
        let listener = listener.with_reply_timeout(Duration::from_secs(5));
        assert_eq!(listener.reply_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn long_reply_window_allows_reply_after_default_timeout() {
        // Varsayılan pencereyi (2 sn) aşan yanıt, pencere 4 sn ise istemciye ulaşır.
        let delay = TIMEOUT + Duration::from_millis(300);
        let (reply, took) =
            delayed_reply(Some(Duration::from_secs(4)), delay, Duration::from_secs(6));
        assert_eq!(reply.unwrap(), "geç ama geçerli");
        assert!(took >= delay, "{took:?}");
    }

    #[test]
    fn short_reply_window_drops_late_reply() {
        // Pencere 150 ms: 400 ms sonraki yanıt düşer, istemci boş yanıt alır (bağlantı kapanır).
        let (reply, took) = delayed_reply(
            Some(Duration::from_millis(150)),
            Duration::from_millis(400),
            Duration::from_secs(3),
        );
        assert_eq!(reply.unwrap(), "");
        assert!(
            took >= Duration::from_millis(350) && took < Duration::from_secs(2),
            "{took:?}"
        );
    }
}
