//! Güvenilmeyen görsellerin yalıtılmış çözümü (ADR-0007).
//!
//! [`Sandbox`] `nuxut_image::IsolatedDecoder`'ı uygular ve `Loader::with_sandbox` (ikonlar) ya
//! da `WallpaperDecoder::new` (duvar kağıdı, kullanıcı dosyaları) ile kurulur. Linux'ta her
//! çözüm ayrı, kısıtlı bir yardımcı süreçte (`nuxut-image-worker`; seccomp + landlock + rlimit)
//! yapılır. Her `Sandbox` tek bir [`WorkerProfile`]'a bağlıdır ([`SandboxConfig::profile`]):
//! ikon (varsayılan) ya da duvar kağıdı ([`SandboxConfig::wallpaper`]); havuz, kova, susturma
//! ve negatif önbellek profil başınadır.
//!
//! - **Tek kullanımlık işçi:** her istek girdi görmemiş taze bir işçiye gider, yanıttan sonra
//!   işçi öldürülür. Ele geçirilmiş bir işçi başka bir kaynağın (başka uygulamanın ikonu)
//!   yanıtını taklit edemez. Gecikmeyi saklamak için en fazla [`SandboxConfig::prewarm`] (1)
//!   taze işçi önceden ısıtılır; 30 sn boşta kalan kapatılır. Boşta ne süreç ne thread kalır.
//! - **El sıkışma** kilitten sonra gelir ve kilidi (seccomp, landlock düzeyi, test kancaları)
//!   ve profili beyan eder; ebeveyn seccomp'u, landlock'u ve kendi profilini zorunlu tutar,
//!   kancalı işçiyi reddeder.
//! - **Zaman aşımı:** istek başına duvar saati ([`SandboxConfig::timeout`]); dolunca işçi
//!   öldürülür ve beklenir (zombi yok).
//! - **Kapılar** (her yolda, süreç içi yedekten ÖNCE): belirteç kovası (30/sn, patlama 60),
//!   anahtar başına art arda 3 başarısızlıkta 60 sn susturma (anahtarsız istekler tek
//!   [`nuxut_image::ANON_KEY`] kovasında), işçiyi öldüren girdinin özeti (negatif önbellek,
//!   256): baytta içerik özeti, dosyada ([`Sandbox::decode_file`]) dosya kimliği.
//! - **Dosya akışı** ([`Sandbox::decode_file`]): dosya ebeveynin belleğine alınmaz, işçiye
//!   256 KiB parçalarla akıtılır; [`CancelFlag`] ≤ 50 ms'de işçiyi öldürür.
//! - **Kullanılamıyorsa** (ikili yok, kısıtlama kurulamıyor, el sıkışma tutmuyor, Linux dışı;
//!   ADR K2-b) ve [`SandboxConfig::in_process_fallback`] açıksa: yalnızca PNG/JPEG/WebP/GIF/QOI
//!   süreç içine düşer, ilk kare, AYNI kapılar ve sınırlarla (süreç başına bir kez uyarı);
//!   SVG/JXL/AVIF ve kapılardan geçemeyen (zehirli/susturulmuş) girdi asla.
//!
//! Log: yalnızca hata türü, süre, bayt uzunluğu ve çıkış kodu/sinyali; yol, URL, ad ve `key`
//! hiçbir seviyede yazılmaz (ARCHITECTURE değişmez 9).

// Bulgu 2b: test kancaları sürüm derlemesinde hiç var olamaz.
#[cfg(all(feature = "sandbox-test-hooks", not(debug_assertions)))]
compile_error!("sandbox-test-hooks must never be enabled in a release build (ADR-0007)");
mod guard;
#[cfg(feature = "sandbox-test-hooks")]
pub mod hooks;
pub mod locate;
mod profile;
pub mod protocol;
#[cfg(target_os = "linux")]
mod process;
#[cfg(all(target_os = "linux", feature = "worker"))]
mod worker;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use nuxut_image::{ANON_KEY, CancelFlag, DecodeOptions, DecodedFile, Image, ImageError, IsolatedDecoder, NO_KEY, SniffedFormat, Trust, sniff_format};
pub use nuxut_image::{Isolation, SandboxFailure};
pub use profile::{ProfileCaps, WorkerProfile};

use guard::{KeyGuard, NegativeCache, TokenBucket, digest};

/// İşçinin çıkış kodu: kısıtlamalar kurulamadı (eski çekirdek, kısıtlı kapsayıcı, desteklenmeyen
/// mimari, Linux dışı). Ebeveyn bunu `Unavailable` sayar.
pub const EXIT_LOCKDOWN: i32 = 70;
/// İşçinin çıkış kodu: istek başlığı geçersiz (ebeveynle eşzamanlılık bozuldu).
pub const EXIT_PROTOCOL: i32 = 65;

/// Dosya isteğinde biçim koklamak için okunan baş (SVG sınıfı ilk 4 KiB'e bakar).
const SNIFF_BYTES: u64 = 4096;

/// İşçi ikilisinin tek satırı (`worker` özelliği; dağıtılan ikili `nuxut-image-worker`
/// paketidir).
#[cfg(all(target_os = "linux", feature = "worker"))]
pub use worker::worker_main;

/// Linux dışında işçi yoktur; ikili hemen [`EXIT_LOCKDOWN`] ile çıkar.
#[cfg(all(not(target_os = "linux"), feature = "worker"))]
pub fn worker_main() -> ! {
    std::process::exit(EXIT_LOCKDOWN)
}

/// [`Sandbox`] ayarları. Varsayılanlar ADR-0007'deki ikon profilidir; duvar kağıdı için
/// [`SandboxConfig::wallpaper`].
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// İşçi ikilisi; `None` → [`locate::locate`] (ortam değişkeni → çalışan ikiliye göreli →
    /// `~/.local/lib/nuxut` → `/usr/lib/nuxut`).
    pub worker: Option<PathBuf>,
    /// İşçi profili (rlimit'ler, girdi/çıktı tavanı, öncelik). İşçi başlatılırken verilir ve el
    /// sıkışmada doğrulanır.
    pub profile: WorkerProfile,
    /// İstek başına duvar saati.
    pub timeout: Duration,
    /// Başlatma + kilitlenme + el sıkışma süresi (yüklü makinede soğuk başlangıç için pay).
    pub spawn_timeout: Duration,
    /// Aynı anda yaşayan (meşgul + önceden ısıtılmış) en fazla işçi.
    pub max_workers: usize,
    /// Boşta bekletilen, girdi görmemiş (önceden ısıtılmış) en fazla taze işçi; `max_workers`'a
    /// kırpılır. Varsayılan 1: tepsi/bildirim çözümleri çoğunlukla sıralı gelir, ikinci eşzamanlı
    /// istek soğuk başlar (~2 ms); her boştaki işçi bellek tutar (bkz. ARCHITECTURE). 0 = ön
    /// ısıtma yok (her istek soğuk).
    pub prewarm: usize,
    /// Önceden ısıtılmış ama kullanılmamış işçi bu kadar boşta kalınca kapatılır.
    pub idle_timeout: Duration,
    /// Belirteç kovası: saniyede iş ve patlama.
    pub rate_per_sec: u32,
    pub burst: u32,
    /// Negatif önbellek girdisi.
    pub negative_cache: usize,
    /// Zaman aşımı negatif önbelleğe girsin mi. İkonda evet; duvar kağıdında hayır (yük altında
    /// meşru dosya kalıcı karalisteye düşmesin; CPU'yu `RLIMIT_CPU` sınırlar, onun SIGXCPU'su
    /// `Killed` olarak yine girer).
    pub negative_on_timeout: bool,
    /// İşçi kullanılamıyorken (`Unavailable`) temel raster (PNG/JPEG/WebP/GIF/QOI) süreç içinde
    /// çözülsün mü (ADR-0007 K2-b; `Untrusted` kapısı, ilk kare, aynı sınırlar). `false`:
    /// kapalı-güvenli, her şey `Unavailable` (çağıran yedeğini gösterir).
    pub in_process_fallback: bool,
    /// Anahtar başına art arda başarısızlık eşiği ve susturma süresi.
    pub key_failure_threshold: u32,
    pub key_mute: Duration,
    /// Girdi baytı üst sınırı (en fazla profilin [`ProfileCaps::max_input_bytes`]'ı).
    pub max_input_bytes: u32,
    /// Yanıt (çıktı) piksel sınırı: ebeveynin bir yanıt için ayırabileceği en büyük tampon / 4
    /// (en fazla profilin [`ProfileCaps::max_out_pixels`]'ı).
    pub max_output_pixels: u64,
    /// İşçi başlatılamadıktan sonra yeniden deneme aralığı.
    pub unavailable_retry: Duration,
    /// Yalnızca testler: işçiye verilen ek argümanlar (`--test-no-seccomp` …).
    #[cfg(feature = "sandbox-test-hooks")]
    pub worker_args: Vec<String>,
    /// Yalnızca testler: seccomp'suz (landlock-yalnız) işçiyi kabul et.
    #[cfg(feature = "sandbox-test-hooks")]
    pub test_allow_unconfined: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            worker: None,
            profile: WorkerProfile::Icon,
            timeout: Duration::from_millis(1500),
            spawn_timeout: Duration::from_secs(3),
            max_workers: 2,
            prewarm: 1,
            idle_timeout: Duration::from_secs(30),
            rate_per_sec: 30,
            burst: 60,
            negative_cache: 256,
            negative_on_timeout: true,
            in_process_fallback: true,
            key_failure_threshold: 3,
            key_mute: Duration::from_secs(60),
            max_input_bytes: protocol::MAX_INPUT_BYTES,
            max_output_pixels: 4096 * 4096,
            unavailable_retry: Duration::from_secs(60),
            #[cfg(feature = "sandbox-test-hooks")]
            worker_args: Vec::new(),
            #[cfg(feature = "sandbox-test-hooks")]
            test_allow_unconfined: false,
        }
    }
}

impl SandboxConfig {
    /// Duvar kağıdı ve diğer kullanıcı dosyaları (kilit avatarı, içe aktarılan durağan görsel)
    /// profili (ARCHITECTURE "Duvar kağıdı yalıtımı"): girdi 256 MiB (akışla), 16K kenar,
    /// 1.5 GiB çözücü bütçesi, işçide AS 2.5 GiB / CPU 10/12 sn / nice 10 / `oom_score_adj`
    /// 1000; duvar saati 20 sn ve zaman aşımı negatif önbelleğe girmez; tek işçi, ön ısıtma
    /// yok (boşta süreç yok); kova 4/sn patlama 8; 3 hata → 30 sn susturma.
    pub fn wallpaper() -> Self {
        let caps = WorkerProfile::Wallpaper.caps();
        Self {
            profile: WorkerProfile::Wallpaper,
            timeout: Duration::from_secs(20),
            max_workers: 1,
            prewarm: 0,
            rate_per_sec: 4,
            burst: 8,
            negative_on_timeout: false,
            in_process_fallback: true,
            key_mute: Duration::from_secs(30),
            max_input_bytes: caps.max_input_bytes,
            max_output_pixels: caps.max_out_pixels,
            ..Self::default()
        }
    }
}

/// Tanı sayaçları (`msg`/test); içerik taşımaz.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SandboxStats {
    /// Başlatılan işçi süreci (toplam; önceden ısıtılanlar dahil).
    pub spawned: u64,
    /// Şu an yaşayan işçi (boşta + meşgul).
    pub live: usize,
    /// Şu an boşta bekleyen (önceden ısıtılmış, girdi görmemiş) işçi.
    pub idle: usize,
    /// Başarılı yalıtılmış çözüm.
    pub decoded: u64,
    /// Zaman aşımı / öldü / protokol.
    pub failures: u64,
    pub rate_limited: u64,
    /// Negatif önbellekten (işçi başlatmadan) reddedilen.
    pub negative_hits: u64,
    /// K2-b ile süreç içinde çözülen (işçi kullanılamıyorken).
    pub in_process_fallbacks: u64,
    /// Çağıranın iptal ettiği istek (başarısızlık sayılmaz).
    pub cancelled: u64,
    /// Son kabul edilen el sıkışmanın bildirdiği miras fd sayısı (yalnızca sayı).
    pub inherited_fds: u16,
}

#[derive(Debug)]
struct Guards {
    bucket: TokenBucket,
    keys: KeyGuard,
    negative: NegativeCache,
}

#[derive(Debug, Default)]
struct Pool {
    /// Önceden ısıtılmış, hiç girdi görmemiş işçiler.
    #[cfg(target_os = "linux")]
    idle: Vec<process::Worker>,
    live: usize,
    unavailable_until: Option<Instant>,
    /// Kullanılmış, temizlikçinin öldüreceği işçiler (yuvaları hâlâ `live`'da).
    #[cfg(target_os = "linux")]
    retired: Vec<process::Worker>,
    /// Temizlikçi bir taze işçi ısıtsın.
    want_prewarm: bool,
    janitor_running: bool,
    /// Temizlikçinin tutamağı: `Sandbox` düşerken beklenir (düştükten sonra süreç kalmaz).
    janitor: Option<std::thread::JoinHandle<()>>,
    shutdown: bool,
}

#[derive(Debug, Default)]
struct Counters {
    spawned: AtomicU64,
    decoded: AtomicU64,
    failures: AtomicU64,
    rate_limited: AtomicU64,
    negative_hits: AtomicU64,
    fallbacks: AtomicU64,
    cancelled: AtomicU64,
    inherited_fds: AtomicU64,
}

#[derive(Debug)]
struct Inner {
    cfg: SandboxConfig,
    worker_path: Option<PathBuf>,
    pool: Mutex<Pool>,
    /// Boş yuva bekleyenler.
    pool_cv: Condvar,
    /// Temizlikçi thread'in uykusu.
    janitor_cv: Condvar,
    guards: Mutex<Guards>,
    counters: Counters,
}

/// K2-b süreç içi yedeğinin uyarısı süreç başına bir kez (profil/örnek sayısından bağımsız).
static WARNED_FALLBACK: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Başarılı çözüm: tek kare + kaynak animasyonluydu (+ animasyon bilgisi).
type Answer = DecodedFile;

/// Girdinin kaynağı (iç).
#[derive(Clone, Copy)]
enum Input<'a> {
    Bytes(&'a [u8]),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    File { file: &'a std::fs::File, len: u64 },
}

/// Dosya isteğinin negatif önbellek anahtarı: `blake3(dev ‖ ino ‖ len ‖ mtime_ns ‖ ctime_ns)`.
/// Önceden bilinir (içerik iki kez okunmaz); dosya değişirse anahtar değişir.
fn file_digest(meta: &std::fs::Metadata) -> [u8; 16] {
    let mut b = Vec::with_capacity(40);
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let ns = |s: i64, n: i64| (i128::from(s) * 1_000_000_000 + i128::from(n)).to_le_bytes();
        b.extend_from_slice(&meta.dev().to_le_bytes());
        b.extend_from_slice(&meta.ino().to_le_bytes());
        b.extend_from_slice(&meta.len().to_le_bytes());
        b.extend_from_slice(&ns(meta.mtime(), meta.mtime_nsec()));
        b.extend_from_slice(&ns(meta.ctime(), meta.ctime_nsec()));
    }
    #[cfg(not(unix))]
    {
        b.extend_from_slice(&meta.len().to_le_bytes());
        let t = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).unwrap_or_default();
        b.extend_from_slice(&t.as_nanos().to_le_bytes());
    }
    // Bayt yolunun içerik özetleriyle çakışmasın.
    b.extend_from_slice(b"nuxut-file");
    digest(&b)
}

fn io_error(e: std::io::Error) -> ImageError {
    ImageError::Io { path: PathBuf::new(), source: e }
}

/// Yalıtılmış çözücü. `Arc` içinde `Loader::with_sandbox`'a ya da `WallpaperDecoder::new`'e
/// verilir; düşürülünce işçiler öldürülür.
#[derive(Debug)]
pub struct Sandbox {
    inner: Arc<Inner>,
}

impl Sandbox {
    /// Süreç başlatmaz (tembel). İşçi yolu burada bir kez çözülür.
    pub fn new(mut cfg: SandboxConfig) -> Self {
        let caps = cfg.profile.caps();
        cfg.max_input_bytes = cfg.max_input_bytes.min(caps.max_input_bytes);
        cfg.max_output_pixels = cfg.max_output_pixels.min(caps.max_out_pixels);
        cfg.max_workers = cfg.max_workers.max(1);
        cfg.prewarm = cfg.prewarm.min(cfg.max_workers);
        let worker_path = if cfg!(target_os = "linux") {
            match &cfg.worker {
                Some(p) => locate::check(p),
                None => locate::locate(),
            }
        } else {
            None
        };
        if cfg!(target_os = "linux") && worker_path.is_none() {
            tracing::warn!(profile = ?cfg.profile, "görsel işçisi bulunamadı; SVG/JXL/AVIF çözülmeyecek, temel raster süreç içinde (ADR-0007 K2-b)");
        }
        let now = Instant::now();
        let guards = Guards {
            bucket: TokenBucket::new(cfg.rate_per_sec, cfg.burst, now),
            keys: KeyGuard::new(cfg.key_failure_threshold, cfg.key_mute),
            negative: NegativeCache::new(cfg.negative_cache),
        };
        Self {
            inner: Arc::new(Inner {
                cfg,
                worker_path,
                pool: Mutex::new(Pool::default()),
                pool_cv: Condvar::new(),
                janitor_cv: Condvar::new(),
                guards: Mutex::new(guards),
                counters: Counters::default(),
            }),
        }
    }

    pub fn profile(&self) -> WorkerProfile {
        self.inner.cfg.profile
    }

    /// Bu oturumda güvenilmeyen görselin nasıl çözüleceği. Linux dışında her zaman
    /// `Unavailable` (yalnızca temel raster süreç içinde).
    pub fn isolation(&self) -> Isolation {
        if !cfg!(target_os = "linux") || self.inner.worker_path.is_none() {
            return Isolation::Unavailable;
        }
        let pool = lock(&self.inner.pool);
        match pool.unavailable_until {
            Some(t) if Instant::now() < t => Isolation::Unavailable,
            _ => Isolation::Process,
        }
    }

    /// Güvenilmeyen baytları çözer; sonuç her zaman tek karedir. Hatada çağıran yedek ikonu
    /// gösterir.
    pub fn decode(&self, key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions) -> Result<Image, ImageError> {
        self.decode_inner(key, bytes, opts, 0).map(|a| a.image)
    }

    /// Açık kullanıcı dosyasını (`nuxut_image::open_user_file`) çözer: dosya belleğe alınmaz,
    /// işçiye akıtılır (bkz. [`IsolatedDecoder::decode_file`]). Sıra: `fstat` → boyut kapısı
    /// (işçi başlamaz) → ilk 4 KiB ile biçim koklama → kapılar (dosya kimliği negatif önbellek
    /// anahtarıdır) → işçi. `cancel` ≤ 50 ms'de işçiyi öldürür; iptal başarısızlık sayılmaz.
    pub fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag) -> Result<DecodedFile, ImageError> {
        self.decode_file_inner(key, &file, opts, cancel, 0)
    }

    /// Yalnızca testler: işçiye bir düşmanca kanca isteği gönderir.
    #[cfg(feature = "sandbox-test-hooks")]
    pub fn decode_hooked(&self, key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions, hook: hooks::Hook) -> Result<Image, ImageError> {
        self.decode_inner(key, bytes, opts, hook as u8).map(|a| a.image)
    }

    /// Yalnızca testler: dosya isteğinde bir düşmanca kanca.
    #[cfg(feature = "sandbox-test-hooks")]
    pub fn decode_file_hooked(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag, hook: hooks::Hook) -> Result<DecodedFile, ImageError> {
        self.decode_file_inner(key, &file, opts, cancel, hook as u8)
    }

    /// Yalnızca testler: işçiyi bir süre kullanılamaz say (K2-b yolunu sınamak için).
    #[cfg(feature = "sandbox-test-hooks")]
    pub fn test_mark_unavailable(&self) {
        let mut pool = lock(&self.inner.pool);
        pool.unavailable_until = Some(Instant::now() + Duration::from_secs(3600));
    }

    pub fn stats(&self) -> SandboxStats {
        let c = &self.inner.counters;
        #[allow(unused_variables)]
        let pool = lock(&self.inner.pool);
        SandboxStats {
            spawned: c.spawned.load(Ordering::Relaxed),
            live: pool.live,
            #[cfg(target_os = "linux")]
            idle: pool.idle.len(),
            #[cfg(not(target_os = "linux"))]
            idle: 0,
            decoded: c.decoded.load(Ordering::Relaxed),
            failures: c.failures.load(Ordering::Relaxed),
            rate_limited: c.rate_limited.load(Ordering::Relaxed),
            negative_hits: c.negative_hits.load(Ordering::Relaxed),
            in_process_fallbacks: c.fallbacks.load(Ordering::Relaxed),
            cancelled: c.cancelled.load(Ordering::Relaxed),
            inherited_fds: c.inherited_fds.load(Ordering::Relaxed) as u16,
        }
    }

    fn decode_inner(&self, key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions, hook: u8) -> Result<Answer, ImageError> {
        let cfg = &self.inner.cfg;
        if bytes.len() as u64 > u64::from(cfg.max_input_bytes) {
            return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit: u64::from(cfg.max_input_bytes) });
        }
        let kind = sniff_format(&bytes);
        if hook == 0 && kind == SniffedFormat::Other {
            // İmzası tanınmayan bayt hiçbir çözücüye gitmez; işçi başlatmaya değmez.
            return Err(ImageError::UnknownFormat);
        }
        // Anahtarsız istek susturmadan kaçamaz (bulgu 4d).
        let key = if key == NO_KEY { ANON_KEY } else { key };
        let d = digest(&bytes);
        // Kapılar her yolda, süreç içi yedekten de önce (bulgu 1b).
        self.admit(key, &d)?;
        let result = self.dispatch(kind, Input::Bytes(&bytes), opts, hook, None);
        self.settle(key, d, bytes.len() as u64, result.as_ref().map(|_| ()));
        result
    }

    fn decode_file_inner(&self, key: u64, file: &std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag, hook: u8) -> Result<DecodedFile, ImageError> {
        if cancel.is_cancelled() {
            return Err(ImageError::Cancelled);
        }
        let meta = file.metadata().map_err(io_error)?;
        if !meta.is_file() {
            return Err(io_error(std::io::Error::new(std::io::ErrorKind::InvalidInput, "normal dosya değil")));
        }
        let len = meta.len();
        let limit = u64::from(self.inner.cfg.max_input_bytes).min(opts.limits.max_file_bytes);
        if len > limit {
            // Boyut kapısı: işçi başlamaz, dosya okunmaz.
            return Err(ImageError::FileTooLarge { size: len, limit });
        }
        if len == 0 {
            return Err(ImageError::Empty);
        }
        let head = read_head(file, len.min(SNIFF_BYTES))?;
        let kind = sniff_format(&head);
        if hook == 0 && kind == SniffedFormat::Other {
            return Err(ImageError::UnknownFormat);
        }
        let key = if key == NO_KEY { ANON_KEY } else { key };
        let d = file_digest(&meta);
        self.admit(key, &d)?;
        let result = self.dispatch(kind, Input::File { file, len }, opts, hook, Some(cancel));
        self.settle(key, d, len, result.as_ref().map(|_| ()));
        result
    }

    /// Kapılardan geçmiş isteği işçiye ya da (kullanılamıyorsa) K2-b yedeğine yönlendirir.
    #[allow(unused_variables)]
    fn dispatch(&self, kind: SniffedFormat, input: Input<'_>, opts: &DecodeOptions, hook: u8, cancel: Option<&CancelFlag>) -> Result<Answer, ImageError> {
        match self.isolation() {
            #[cfg(target_os = "linux")]
            Isolation::Process => self.decode_process(kind, input, opts, hook, cancel),
            _ => self.fallback(kind, input, opts, cancel),
        }
    }

    /// Kapılar: susturulmuş anahtar, negatif önbellek, belirteç kovası.
    fn admit(&self, key: u64, d: &[u8; 16]) -> Result<(), ImageError> {
        let now = Instant::now();
        let c = &self.inner.counters;
        let mut g = lock(&self.inner.guards);
        if g.keys.is_muted(key, now) {
            c.rate_limited.fetch_add(1, Ordering::Relaxed);
            return Err(ImageError::Sandbox(SandboxFailure::RateLimited));
        }
        if g.negative.hit(d) {
            c.negative_hits.fetch_add(1, Ordering::Relaxed);
            return Err(ImageError::Sandbox(SandboxFailure::Killed));
        }
        if !g.bucket.take(now) {
            c.rate_limited.fetch_add(1, Ordering::Relaxed);
            return Err(ImageError::Sandbox(SandboxFailure::RateLimited));
        }
        Ok(())
    }

    /// Sonucu kapılara işler: zehirli girdi negatif önbelleğe, anahtar sayacı. İptal hiçbir
    /// şey değiştirmez; zaman aşımı yalnızca [`SandboxConfig::negative_on_timeout`] ile
    /// negatif önbelleğe girer.
    fn settle(&self, key: u64, d: [u8; 16], len: u64, result: Result<(), &ImageError>) {
        let c = &self.inner.counters;
        let mut g = lock(&self.inner.guards);
        match result {
            Ok(()) => {
                c.decoded.fetch_add(1, Ordering::Relaxed);
                g.keys.success(key);
            }
            Err(ImageError::Cancelled) => {
                c.cancelled.fetch_add(1, Ordering::Relaxed);
            }
            Err(ImageError::Sandbox(f @ (SandboxFailure::Timeout | SandboxFailure::Killed | SandboxFailure::Protocol))) => {
                c.failures.fetch_add(1, Ordering::Relaxed);
                if *f != SandboxFailure::Timeout || self.inner.cfg.negative_on_timeout {
                    g.negative.insert(d);
                }
                g.keys.failure(key, Instant::now());
                tracing::warn!(failure = ?f, len, profile = ?self.inner.cfg.profile, "görsel yalıtılmış çözümde başarısız");
            }
            Err(ImageError::Sandbox(_)) => {}
            Err(_) => g.keys.failure(key, Instant::now()),
        }
    }

    /// K2-b: işçi kullanılamıyorken yalnızca temel raster süreç içinde, ilk kare, isteğin
    /// sınırlarıyla (boyut, piksel, çözücü bütçesi). Kapılardan geçmiş girdi için çağrılır.
    /// Çağıranın thread'inde çalışır (duvar kağıdında `WallpaperDecoder`'ın düşük öncelikli
    /// thread'i).
    fn fallback(&self, kind: SniffedFormat, input: Input<'_>, opts: &DecodeOptions, cancel: Option<&CancelFlag>) -> Result<Answer, ImageError> {
        if !kind.in_process_fallback() || !self.inner.cfg.in_process_fallback {
            return Err(ImageError::Sandbox(SandboxFailure::Unavailable));
        }
        if !WARNED_FALLBACK.swap(true, Ordering::Relaxed) {
            tracing::warn!("görsel işçisi kullanılamıyor; temel raster süreç içinde çözülüyor (ADR-0007 K2-b)");
        }
        self.inner.counters.fallbacks.fetch_add(1, Ordering::Relaxed);
        let owned;
        let bytes: &[u8] = match input {
            Input::Bytes(b) => b,
            Input::File { file, len } => {
                owned = read_head(file, len)?;
                &owned
            }
        };
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(ImageError::Cancelled);
        }
        // `Untrusted` güven kapısı yalnızca temel rasteri geçirir ve animasyonu kapatır.
        let caps = self.inner.cfg.profile.caps();
        let limits = caps.clamp(&opts.limits).with_trust(Trust::Untrusted);
        let opts = DecodeOptions { limits, animate: false, ..*opts };
        let r = std::panic::catch_unwind(|| {
            let img = nuxut_image::decode_bytes(bytes, &opts)?.into_first_frame();
            Ok(DecodedFile::from_source(img, bytes, &limits))
        });
        r.unwrap_or(Err(ImageError::Sandbox(SandboxFailure::Killed)))
    }

    #[cfg(target_os = "linux")]
    fn decode_process(&self, kind: SniffedFormat, input: Input<'_>, opts: &DecodeOptions, hook: u8, cancel: Option<&CancelFlag>) -> Result<Answer, ImageError> {
        let cfg = &self.inner.cfg;
        let limits = cfg.profile.caps().clamp(&opts.limits);
        let payload = match input {
            Input::Bytes(b) => process::Payload::Bytes(b),
            Input::File { file, len } => process::Payload::File { file, len },
        };
        let req = protocol::Request {
            payload_len: payload.len() as u32,
            kind,
            target: opts.target,
            hook,
            max_width: limits.max_width,
            max_height: limits.max_height,
            max_pixels: limits.max_pixels,
            max_out_pixels: cfg.max_output_pixels,
            max_decode_bytes: limits.max_decode_bytes,
        };
        let mut worker = match self.checkout() {
            Ok(w) => w,
            // Kapılardan zaten geçti; K2-b yedeği.
            Err(()) => return self.fallback(kind, input, opts, cancel),
        };
        let started = Instant::now();
        let outcome = worker.request(&req, payload, started + cfg.timeout, cancel, u64::from(cfg.max_input_bytes));
        let elapsed = started.elapsed();
        let result = match outcome {
            Ok(r) => r,
            Err(process::RequestFail::Cancelled) => {
                // Hemen (temizlikçiyi beklemeden) öldür: iptal ≤ 50 ms + bekleme.
                worker.kill();
                Err(ImageError::Cancelled)
            }
            Err(process::RequestFail::Input) => {
                worker.kill();
                Err(io_error(std::io::Error::other("dosya akış sırasında okunamadı")))
            }
            Err(process::RequestFail::Sandbox(f)) => {
                let (f, status) = worker.kill_classify(f);
                tracing::warn!(failure = ?f, ms = elapsed.as_millis() as u64, len = payload.len(), exit = %process::describe(status), "görsel işçisi düşürüldü");
                Err(ImageError::Sandbox(f))
            }
        };
        // Tek kullanımlık: girdi görmüş işçi asla başka isteğe gitmez (bulgu 3). Öldürme ve
        // yerine taze işçi başlatma temizlikçi thread'de, çağıranın yolunun dışında.
        self.inner.retire(worker);
        result
    }

    /// Taze (girdi görmemiş) işçi alır: önceden ısıtılmış varsa onu doğrular, yuva varsa
    /// başlatır, yoksa bekler. `Err`: kullanılamıyor.
    #[cfg(target_os = "linux")]
    fn checkout(&self) -> Result<process::Worker, ()> {
        let inner = &self.inner;
        if inner.worker_path.is_none() {
            return Err(());
        }
        let mut pool = lock(&inner.pool);
        loop {
            if pool.shutdown || pool.unavailable_until.is_some_and(|t| Instant::now() < t) {
                return Err(());
            }
            if let Some(w) = pool.idle.pop() {
                drop(pool);
                return self.verify(w);
            }
            if pool.live < inner.cfg.max_workers {
                pool.live += 1;
                drop(pool);
                return match inner.start() {
                    Some(w) => self.verify(w),
                    None => Err(()),
                };
            }
            pool = inner.pool_cv.wait(pool).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// El sıkışmayı okur ve kabul eder; reddedilirse yuvayı bırakır, yalıtım kullanılamaz.
    #[cfg(target_os = "linux")]
    fn verify(&self, mut w: process::Worker) -> Result<process::Worker, ()> {
        let inner = &self.inner;
        #[cfg(feature = "sandbox-test-hooks")]
        let (hooks, unconfined) = (true, inner.cfg.test_allow_unconfined);
        #[cfg(not(feature = "sandbox-test-hooks"))]
        let (hooks, unconfined) = (false, false);
        let started = Instant::now();
        match w.verify(started + inner.cfg.spawn_timeout, inner.cfg.profile, hooks, unconfined) {
            Ok(hs) => {
                inner.counters.inherited_fds.store(u64::from(hs.inherited_fds), Ordering::Relaxed);
                if hs.inherited_fds > 0 {
                    tracing::debug!(count = hs.inherited_fds, "görsel işçisine miras fd geçti (seccomp ile etkisiz)");
                }
                tracing::debug!(us = started.elapsed().as_micros() as u64, landlock = hs.landlock, "görsel işçisi doğrulandı");
                Ok(w)
            }
            Err(e) => {
                drop(w);
                inner.mark_unavailable(e);
                Err(())
            }
        }
    }
}

/// Dosyanın ilk `n` baytı (`n` ≤ dosya boyu, sınırdan geçmiş). Konumdan bağımsız okuma: aynı fd
/// sonra baştan akıtılır.
fn read_head(file: &std::fs::File, n: u64) -> Result<Vec<u8>, ImageError> {
    let n = usize::try_from(n).map_err(|_| ImageError::FileTooLarge { size: n, limit: usize::MAX as u64 })?;
    let mut buf = vec![0u8; n];
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(&mut buf, 0).map_err(io_error)?;
    }
    #[cfg(not(unix))]
    {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = file;
        f.seek(SeekFrom::Start(0)).map_err(io_error)?;
        f.read_exact(&mut buf).map_err(io_error)?;
    }
    Ok(buf)
}

#[cfg(target_os = "linux")]
impl Inner {
    /// Yuvası ayrılmış (`live` artırılmış) yeni bir süreç başlatır; başarısızsa yuvayı bırakır
    /// ve yalıtımı bir süre kullanılamaz sayar.
    fn start(&self) -> Option<process::Worker> {
        let path = self.worker_path.as_deref()?;
        #[cfg(feature = "sandbox-test-hooks")]
        let args = self.cfg.worker_args.as_slice();
        #[cfg(not(feature = "sandbox-test-hooks"))]
        let args: &[String] = &[];
        match process::Worker::start(path, self.cfg.profile, args) {
            Ok(w) => {
                self.counters.spawned.fetch_add(1, Ordering::Relaxed);
                Some(w)
            }
            Err(e) => {
                self.mark_unavailable(e);
                None
            }
        }
    }

    fn mark_unavailable(&self, e: process::SpawnError) {
        let mut pool = lock(&self.pool);
        pool.live = pool.live.saturating_sub(1);
        pool.unavailable_until = Some(Instant::now() + self.cfg.unavailable_retry);
        // Önceden ısıtılmışlar da aynı ikilidir: hepsi gider.
        let idle = std::mem::take(&mut pool.idle);
        pool.live = pool.live.saturating_sub(idle.len());
        self.pool_cv.notify_all();
        drop(pool);
        drop(idle);
        tracing::warn!(error = ?e, "görsel işçisi kullanılamıyor (ADR-0007 K2-b)");
    }

    /// Kullanılmış işçiyi temizlikçiye verir (öldürülür, yerine taze işçi ısıtılır). Yuvası
    /// temizlikçi onu öldürene dek dolu kalır.
    fn retire(self: &Arc<Self>, worker: process::Worker) {
        let mut pool = lock(&self.pool);
        pool.retired.push(worker);
        pool.want_prewarm = true;
        self.wake_janitor(&mut pool);
    }

    /// Temizlikçi çalışmıyorsa başlatır, çalışıyorsa uyandırır. Thread kurulamazsa iş
    /// yerinde yapılır (yalnızca öldürme; ısıtma atlanır).
    fn wake_janitor(self: &Arc<Self>, pool: &mut MutexGuard<'_, Pool>) {
        if pool.janitor_running {
            self.janitor_cv.notify_all();
            return;
        }
        let inner = self.clone();
        match std::thread::Builder::new().name("nuxut-image-janitor".into()).spawn(move || inner.janitor()) {
            Ok(h) => {
                pool.janitor_running = true;
                pool.janitor = Some(h);
            }
            Err(_) => {
                let retired = std::mem::take(&mut pool.retired);
                pool.live = pool.live.saturating_sub(retired.len());
                pool.want_prewarm = false;
                self.pool_cv.notify_all();
                drop(retired);
            }
        }
    }

    /// Temizlikçi: kullanılmış işçileri öldürür (kill + wait, zombi yok), yuva varsa bir taze
    /// işçiyi önceden başlatır, boşta `idle_timeout` kalan taze işçiyi kapatır. İş ve boşta
    /// işçi kalmayınca çıkar: boştayken ne süreç ne thread vardır.
    fn janitor(&self) {
        let mut pool = lock(&self.pool);
        loop {
            if !pool.retired.is_empty() {
                let retired = std::mem::take(&mut pool.retired);
                drop(pool);
                let n = retired.len();
                drop(retired);
                pool = lock(&self.pool);
                pool.live = pool.live.saturating_sub(n);
                self.pool_cv.notify_all();
                continue;
            }
            if pool.shutdown {
                break;
            }
            if std::mem::take(&mut pool.want_prewarm)
                && pool.unavailable_until.is_none()
                && pool.live < self.cfg.max_workers
                && pool.idle.len() < self.cfg.prewarm
            {
                pool.live += 1;
                drop(pool);
                let w = self.start();
                pool = lock(&self.pool);
                if let Some(mut w) = w {
                    w.last_used = Instant::now();
                    if pool.shutdown {
                        pool.retired.push(w);
                    } else {
                        pool.idle.push(w);
                        self.pool_cv.notify_one();
                    }
                }
                continue;
            }
            let now = Instant::now();
            let ttl = self.cfg.idle_timeout;
            let (expired, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut pool.idle).into_iter().partition(|w| now.saturating_duration_since(w.last_used) >= ttl);
            pool.idle = keep;
            if !expired.is_empty() {
                pool.retired.extend(expired);
                continue;
            }
            let Some(next) = pool.idle.iter().map(|w| w.last_used + ttl).min() else { break };
            pool = self.janitor_cv.wait_timeout(pool, next.saturating_duration_since(now)).unwrap_or_else(|e| e.into_inner()).0;
        }
        pool.janitor_running = false;
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let mut pool = lock(&self.inner.pool);
        pool.shutdown = true;
        let janitor = pool.janitor.take();
        #[cfg(target_os = "linux")]
        {
            let mut all = std::mem::take(&mut pool.idle);
            all.append(&mut pool.retired);
            pool.live = pool.live.saturating_sub(all.len());
            drop(pool);
            drop(all);
        }
        #[cfg(not(target_os = "linux"))]
        drop(pool);
        self.inner.pool_cv.notify_all();
        self.inner.janitor_cv.notify_all();
        // Temizlikçi o an bir işçi başlatıyor olabilir: o da öldürülsün, sonra çıksın.
        if let Some(h) = janitor {
            let _ = h.join();
        }
    }
}

impl IsolatedDecoder for Sandbox {
    fn isolation(&self) -> Isolation {
        Sandbox::isolation(self)
    }

    fn decode(&self, key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions) -> Result<Image, ImageError> {
        Sandbox::decode(self, key, bytes, opts)
    }

    fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag) -> Result<DecodedFile, ImageError> {
        Sandbox::decode_file(self, key, file, opts, cancel)
    }

    fn max_input_bytes(&self) -> u64 {
        u64::from(self.inner.cfg.max_input_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_image::{Fit, Limits, TargetSize};

    /// Bulgu 1a: işçisiz (ve Linux dışında) SVG asla süreç içinde çözülmez.
    #[test]
    fn without_worker_svg_never_decodes_in_process() {
        let svg: Arc<[u8]> = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64"/></svg>"##.to_vec().into();
        let opts = DecodeOptions { limits: Limits::ICON, target: Some(TargetSize::new(32, 32, Fit::Contain)), animate: false };
        let sb = Sandbox::new(SandboxConfig { worker: Some("/yok".into()), ..SandboxConfig::default() });
        assert_eq!(sb.isolation(), Isolation::Unavailable);
        assert!(matches!(sb.decode(0, svg, &opts), Err(ImageError::Sandbox(SandboxFailure::Unavailable))));
    }
}
