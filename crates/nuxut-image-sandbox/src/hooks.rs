//! Düşmanca test kancaları (ADR-0007 §Düşmanca test planı). YALNIZCA `sandbox-test-hooks`
//! özelliğiyle derlenir; dağıtılan işçide yoktur (orada 0 dışı kanca protokol ihlalidir).
//!
//! Erişim kancaları işlemi dener: çekirdek reddederse [`ErrorCode::HookDenied`], başarırsa
//! [`ErrorCode::HookAllowed`] (yalıtım delik) yanıtlanır; seccomp altında süreç zaten ölür.

use std::io::Write;

use crate::protocol::{ErrorCode, RESP_ANIMATED_SOURCE, Request, Response};

/// Kanca kimlikleri (istek başlığındaki `hook` baytı).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Hook {
    /// Sonsuz döngü (zaman aşımı; uzun süreyle `RLIMIT_CPU` → SIGXCPU).
    Spin = 1,
    /// 2 GiB ayırıp dokunur (`RLIMIT_AS`).
    Alloc2G = 2,
    /// `/etc/passwd` okumayı dener.
    ReadFile = 3,
    /// Girdideki yola dosya yaratmayı dener.
    CreateFile = 4,
    /// 127.0.0.1'e TCP bağlantısı dener.
    Connect = 5,
    /// Thread başlatmayı dener (`clone`).
    Thread = 6,
    /// `/bin/true` çalıştırmayı dener (`clone` + `execve`).
    Exec = 7,
    /// stderr'e (fd 2) yazar.
    WriteStderr = 8,
    /// stdout'a durmaksızın yazar.
    FloodStdout = 9,
    /// Panikler.
    Panic = 10,
    /// Hedef kutudan büyük boyut beyan eder.
    SpoofOversize = 11,
    /// 1×1 beyan eder, 2 bayt yazıp çıkar (kısa yanıt).
    SpoofShortExit = 12,
    /// 1×1 + 4 bayt + fazladan 100 bayt.
    SpoofExtra = 13,
    /// 65536×65536 (16 GiB) beyan eder.
    SpoofHuge = 14,
    /// Tanımsız durum baytı.
    SpoofStatus = 15,
    /// İşçinin PID'ini 1×1 görselin 4 baytı olarak döner (tek kullanımlık işçi testi).
    Pid = 16,
    /// Kırpılmış Cover isteğinde genişliği kutudan 1 px taşan boyut beyan eder (kırpmasız
    /// Cover'da geçerli olurdu).
    SpoofCropOverflow = 17,
    /// 3 GiB ayırıp dokunur (duvar kağıdı profilinin `RLIMIT_AS` 2.5 GiB'ını aşar).
    Alloc3G = 18,
    /// Girdiyi OKUMADAN sonsuz döngü: ebeveynin akışı boruda tıkanır (akış ortasında iptal).
    SpinBeforePayload = 19,
    /// Tel v4: `ANIMATED_SOURCE` ile tutarsız animasyon alanı (1 kare) beyan eder.
    SpoofAnimation = 20,
}

impl Hook {
    pub fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => Self::Spin,
            2 => Self::Alloc2G,
            3 => Self::ReadFile,
            4 => Self::CreateFile,
            5 => Self::Connect,
            6 => Self::Thread,
            7 => Self::Exec,
            8 => Self::WriteStderr,
            9 => Self::FloodStdout,
            10 => Self::Panic,
            11 => Self::SpoofOversize,
            12 => Self::SpoofShortExit,
            13 => Self::SpoofExtra,
            14 => Self::SpoofHuge,
            15 => Self::SpoofStatus,
            16 => Self::Pid,
            17 => Self::SpoofCropOverflow,
            18 => Self::Alloc3G,
            19 => Self::SpinBeforePayload,
            20 => Self::SpoofAnimation,
            _ => return None,
        })
    }
}

fn verdict<T, E>(r: Result<T, E>) -> (Response, Vec<u8>) {
    let code = if r.is_ok() { ErrorCode::HookAllowed } else { ErrorCode::HookDenied };
    (Response::error(code, 0, 0), Vec::new())
}

fn raw_exit(bytes: &[u8]) -> ! {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(bytes);
    let _ = out.flush();
    std::process::exit(0);
}

pub(crate) fn run(req: &Request, payload: &[u8]) -> (Response, Vec<u8>) {
    let Some(hook) = Hook::from_code(req.hook) else { std::process::exit(crate::EXIT_PROTOCOL) };
    match hook {
        Hook::Spin | Hook::SpinBeforePayload => spin(),
        Hook::Alloc2G => {
            let v = vec![1u8; 2 << 30];
            let s: u64 = v.iter().step_by(4096).map(|&b| u64::from(b)).sum();
            (Response::ok(1, 1), vec![s as u8; 4])
        }
        Hook::ReadFile => verdict(std::fs::read("/etc/passwd")),
        Hook::CreateFile => {
            let path = String::from_utf8_lossy(payload).into_owned();
            verdict(std::fs::File::create(path))
        }
        Hook::Connect => verdict(std::net::TcpStream::connect("127.0.0.1:9")),
        Hook::Thread => verdict(std::thread::Builder::new().spawn(|| ()).map(|h| h.join())),
        Hook::Exec => verdict(std::process::Command::new("/bin/true").status()),
        Hook::WriteStderr => verdict(std::io::stderr().write_all(b"x")),
        Hook::FloodStdout => {
            let chunk = vec![0xA5u8; 1 << 16];
            let mut out = std::io::stdout().lock();
            loop {
                if out.write_all(&chunk).is_err() {
                    std::process::exit(0);
                }
            }
        }
        Hook::Panic => panic!("test kancası"),
        Hook::SpoofOversize => {
            let (w, h) = req.target.map_or((req.max_width + 1, 1), |t| (t.width + 1000, t.height + 1000));
            (Response::ok(w, h), Vec::new())
        }
        Hook::SpoofShortExit => {
            let mut b = Response::ok(1, 1).encode().to_vec();
            b.extend_from_slice(&[1, 2]);
            raw_exit(&b)
        }
        Hook::SpoofExtra => {
            let mut px = vec![255u8; 4];
            px.extend_from_slice(&[0u8; 100]);
            (Response::ok(1, 1), px)
        }
        Hook::SpoofHuge => (Response::ok(65_536, 65_536), Vec::new()),
        Hook::SpoofStatus => (Response { status: 9, ..Response::ok(1, 1) }, Vec::new()),
        Hook::Pid => (Response::ok(1, 1), PID.get().copied().unwrap_or(0).to_le_bytes().to_vec()),
        Hook::SpoofCropOverflow => {
            let (w, h) = req.target.map_or((1, 1), |t| (t.width + 1, t.height));
            (Response::ok(w, h), Vec::new())
        }
        Hook::SpoofAnimation => {
            let resp = Response { frames: 1, duration_ms: 40, ..Response::ok(1, 1).with_flags(RESP_ANIMATED_SOURCE) };
            (resp, vec![255u8; 4])
        }
        Hook::Alloc3G => {
            let v = vec![1u8; 3 << 30];
            let s: u64 = v.iter().step_by(4096).map(|&b| u64::from(b)).sum();
            (Response::ok(1, 1), vec![s as u8; 4])
        }
    }
}

/// Kilitten önce kaydedilen PID (`getpid` seccomp'ta yasak).
static PID: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

pub(crate) fn remember_pid() {
    let _ = PID.set(std::process::id());
}

/// Sonsuz döngü (zaman aşımı; uzun süreyle `RLIMIT_CPU` → SIGXCPU).
pub(crate) fn spin() -> ! {
    let mut x = 0u64;
    loop {
        x = std::hint::black_box(x.wrapping_add(1));
    }
}
