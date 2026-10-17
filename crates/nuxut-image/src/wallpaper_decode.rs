//! Duvar kağıdının paylaşılan çözümü (ARCHITECTURE "Duvar kağıdı yalıtımı"): süreç başına tek
//! [`WallpaperDecoder`]; shell bütün çıkışlarına, lock ve settings kendilerininkini verir.
//! Çözüm yalıtılmış işçidedir ([`IsolatedDecoder::decode_file`], duvar kağıdı profili); burada
//! yalnızca güvenilir premultiplied RGBA'yla iş yapılır (önbellek, türetme). `render`'a bağlı
//! değildir (settings ve palet de kullanır).
//!
//! - **Uçuş birleştirme:** aynı (yol, hedef, yerleşim) için kuyrukta ya da çözülmekte olan iş
//!   varsa yeni bekleyen ona eklenir (`joined`): çok monitörde aynı çözünürlük tek çözüm.
//! - **Önbellek:** anahtar `(yol, dev, ino, len, mtime, ctime) + hedef + yerleşim`; dosya bir kez
//!   açılır, anahtar aynı fd'nin `fstat`'ından, çözüm aynı fd'den (TOCTOU yok). Dosya değişirse
//!   yeniden çözülür.
//! - **Türetme:** aynı dosya + yerleşim için daha büyük bir kare elde varsa küçük kare ondan
//!   süreç içinde (güvenilir RGBA, Lanczos3) türetilir (`derived`): 4K + 1080p tek çözüm.
//!   `Contain`/`Fill` her zaman; kırpılmış `Cover` yalnızca en-boy oranı ±1 px aynıysa.
//!   Kuyruktan aynı dosyanın en büyük hedefi önce alınır.
//! - **Tek geçici thread** (`nice` 19): ilk istekte açılır; kuyruk boşalıp tutulan kareler
//!   [`WallpaperDecoderOptions::linger`] sonunda bırakılınca çıkar. **Boşta thread, işçi süreci
//!   ve çözülmüş kare yoktur.**
//! - **İptal:** [`WallpaperTicket`] düşünce bekleyen çıkarılır; işin hiç bekleyeni kalmazsa
//!   kuyruktaysa silinir, çözülüyorsa [`CancelFlag`] ile durdurulur (≤ 50 ms'de işçi ölür).
//!
//! Yol, anahtar ve hata ayrıntısı loglanmaz (ARCHITECTURE değişmez 9).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use crate::{CancelFlag, DecodeOptions, DecodedFile, Fit, Image, ImageError, IsolatedDecoder, Limits, TargetSize, Trust};

/// [`WallpaperDecoder`] ayarları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallpaperDecoderOptions {
    /// Tutulan çözülmüş karelerin bayt üst sınırı (LRU). Sınırı aşan kare teslim edilir ama
    /// tutulmaz. Varsayılan 96 MiB: iki farklı 4K kare ya da 4K + 5K.
    pub cache_bytes: u64,
    /// Kare son kullanımdan sonra bu kadar tutulur (geç gelen çıkış — hotplug, ölçek
    /// configure'u — yeniden çözmesin). Varsayılan 10 sn; lock `Duration::ZERO` (kuyruk
    /// boşalınca hemen bırakır; aynı turdaki diğer çıkışlar yine paylaşır).
    pub linger: Duration,
}

impl Default for WallpaperDecoderOptions {
    fn default() -> Self {
        Self { cache_bytes: 96 << 20, linger: Duration::from_secs(10) }
    }
}

/// Çözüm isteği.
#[derive(Clone, PartialEq, Eq)]
pub struct WallpaperRequest {
    pub path: PathBuf,
    /// Hedef kutu, fiziksel piksel (çıkışın `mantıksal × ölçek` boyutu).
    pub target_px: (u32, u32),
    /// `Cover` ortadan kırpılır (çıktı ≤ kutu); `Contain`/`Fill` kutuya sığar.
    pub fit: Fit,
}

/// Yol yazılmaz.
impl std::fmt::Debug for WallpaperRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WallpaperRequest").field("path", &format_args!("<gizli>")).field("target_px", &self.target_px).field("fit", &self.fit).finish()
    }
}

impl WallpaperRequest {
    pub fn new(path: impl Into<PathBuf>, target_px: (u32, u32), fit: Fit) -> Self {
        Self { path: path.into(), target_px: (target_px.0.max(1), target_px.1.max(1)), fit }
    }
}

fn target_for(fit: Fit, (w, h): (u32, u32)) -> TargetSize {
    match fit {
        Fit::Cover => TargetSize::cover_crop(w.max(1), h.max(1)),
        fit => TargetSize::new(w.max(1), h.max(1), fit),
    }
}

/// Çözülmüş duvar kağıdı karesi: premultiplied RGBA8, satırlar sıkı. Pikseller `Arc` ile
/// paylaşılır (çıkışlar ve önbellek aynı tamponu tutar; doku yüklemesine kopyasız).
#[derive(Clone, PartialEq, Eq)]
pub struct WallpaperFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    /// Kaynak animasyonluydu (GIF/WebP/APNG); kare ilktir (ADR-0007 K5). Animasyonlu duvar
    /// kağıdı için `nuxut-video` içe aktarımı önerilir.
    pub animated_source: bool,
}

impl std::fmt::Debug for WallpaperFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WallpaperFrame({}x{}, animated_source: {})", self.width, self.height, self.animated_source)
    }
}

/// Çağıranın kendi ürettiği güvenilir görsel (ör. gömülü SVG çizimi) → kare; pikseller
/// kopyalanmadan `Arc`'a taşınır. Aynı kare birden çok çıkışa (`Wallpaper::set_image`) klonla
/// paylaşılır.
impl From<Image> for WallpaperFrame {
    fn from(image: Image) -> Self {
        Self::from_image(image, false)
    }
}

impl WallpaperFrame {
    fn from_image(image: Image, animated_source: bool) -> Self {
        let (width, height) = image.size();
        Self { width, height, pixels: Arc::from(image.into_pixels()), animated_source }
    }

    /// Boyut ve tampon uzunluğu tutarlı mı (alanlar açık olduğu için elle kurulmuş kare).
    pub fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0 && (self.width as usize).checked_mul(self.height as usize).and_then(|n| n.checked_mul(4)) == Some(self.pixels.len())
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn byte_size(&self) -> u64 {
        self.pixels.len() as u64
    }

    /// `target`'a küçültülmüş bağımsız [`Image`] (kaynak kopyalanmaz; zaten uygunsa kopya).
    /// Palet örneklemesi önbellekteki kareden (settings).
    pub fn fit_image(&self, target: TargetSize) -> Result<Image, ImageError> {
        match crate::resize::fit_premultiplied(&self.pixels, self.size(), target)? {
            Some(img) => Ok(img),
            None => Ok(self.to_image()),
        }
    }

    /// Bağımsız [`Image`] kopyası (palet örneklemesi gibi okuyucular).
    pub fn to_image(&self) -> Image {
        match Image::from_premultiplied(self.width, self.height, self.pixels.to_vec()) {
            Ok(i) => i,
            // Kurucu boyutu zaten doğruladı; erişilemez.
            Err(_) => unreachable!("WallpaperFrame boyutu tutarlı"),
        }
    }

    /// Doku yükleme biçimi; pikseller kopyalanmaz (`Arc`).
    #[cfg(feature = "render")]
    pub fn to_image_data(&self) -> nuxut_render::ImageData {
        nuxut_render::ImageData { width: self.width, height: self.height, pixels: self.pixels.clone() }
    }
}

/// Tanı sayaçları (içerik taşımaz).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WallpaperDecodeStats {
    /// Çözücüye (işçi ya da süreç içi) giden çözüm.
    pub decodes: u64,
    /// Önbellekteki kareyle karşılanan istek.
    pub hits: u64,
    /// Kuyruktaki/süren aynı işe eklenen istek.
    pub joined: u64,
    /// Daha büyük karenin küçültülmesiyle karşılanan istek.
    pub derived: u64,
    /// Bekleyeni kalmadığı için iptal edilen iş (kuyruktan silinen ya da durdurulan).
    pub cancelled: u64,
    /// Hatayla biten iş.
    pub failed: u64,
    /// Tutulan karelerin baytı (CPU belleği).
    pub resident_bytes: u64,
    pub entries: usize,
    /// Kuyruktaki + süren iş.
    pub pending: usize,
    /// Çözüm thread'i yaşıyor.
    pub thread_alive: bool,
}

type Done = Box<dyn FnOnce(Result<WallpaperFrame, ImageError>) + Send>;

/// Dosya kimliği (önbellek anahtarının dosya kısmı). Yol, adı değiştirilen dosyanın aynı
/// içeriği eşleşmesin diye anahtardadır.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileId {
    path: PathBuf,
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}

impl FileId {
    fn new(path: &Path, meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let ns = |s: i64, n: i64| i128::from(s) * 1_000_000_000 + i128::from(n);
            Self {
                path: path.to_owned(),
                dev: meta.dev(),
                ino: meta.ino(),
                len: meta.len(),
                mtime_ns: ns(meta.mtime(), meta.mtime_nsec()),
                ctime_ns: ns(meta.ctime(), meta.ctime_nsec()),
            }
        }
        #[cfg(not(unix))]
        {
            let t = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).unwrap_or_default();
            Self { path: path.to_owned(), dev: 0, ino: 0, len: meta.len(), mtime_ns: t.as_nanos() as i128, ctime_ns: 0 }
        }
    }
}

/// Birleştirme kimliği: aynı anda açılacak aynı yol, hedef ve yerleşim.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ident {
    path: PathBuf,
    target_px: (u32, u32),
    fit: Fit,
}

struct Waiter {
    id: u64,
    done: Done,
}

struct Job {
    id: u64,
    ident: Ident,
    waiters: Vec<Waiter>,
    running: bool,
    cancel: CancelFlag,
}

struct Entry {
    file: FileId,
    target_px: (u32, u32),
    fit: Fit,
    frame: WallpaperFrame,
    last_used: Instant,
}

#[derive(Default)]
struct State {
    jobs: Vec<Job>,
    cache: Vec<Entry>,
    next_id: u64,
    thread_alive: bool,
    stats: WallpaperDecodeStats,
}

impl State {
    fn resident(&self) -> u64 {
        self.cache.iter().map(|e| e.frame.byte_size()).sum()
    }
}

enum Backend {
    Isolated(Arc<dyn IsolatedDecoder>),
    InProcessUnsandboxed,
}

/// Paylaşılan duvar kağıdı çözücüsü. `Arc` ile paylaşılır (`Send + Sync`); `Wallpaper::new`
/// zorunlu parametre olarak alır.
pub struct WallpaperDecoder {
    backend: Backend,
    opts: WallpaperDecoderOptions,
    state: Mutex<State>,
    /// Çözüm thread'inin uykusu (linger bekleyişi): yeni iş ya da `trim` uyandırır.
    wake: Condvar,
}

impl std::fmt::Debug for WallpaperDecoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.backend {
            Backend::Isolated(d) => format!("Isolated({:?})", d.isolation()),
            Backend::InProcessUnsandboxed => "InProcessUnsandboxed".into(),
        };
        f.debug_struct("WallpaperDecoder").field("backend", &backend).field("opts", &self.opts).finish_non_exhaustive()
    }
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Bekleyenler arasında hata çoğaltma (`ImageError` `Clone` değil; içerik taşımaz).
fn dup_error(e: &ImageError) -> ImageError {
    match e {
        ImageError::Io { path, source } => ImageError::Io { path: path.clone(), source: std::io::Error::new(source.kind(), source.to_string()) },
        ImageError::FileTooLarge { size, limit } => ImageError::FileTooLarge { size: *size, limit: *limit },
        ImageError::TooLarge { width, height } => ImageError::TooLarge { width: *width, height: *height },
        ImageError::Empty => ImageError::Empty,
        ImageError::UnknownFormat => ImageError::UnknownFormat,
        ImageError::NotFound(n) => ImageError::NotFound(n.clone()),
        ImageError::Decode(s) => ImageError::Decode(s.clone()),
        ImageError::Svg(s) => ImageError::Svg(s.clone()),
        ImageError::Resize(s) => ImageError::Resize(s.clone()),
        ImageError::InvalidData(s) => ImageError::InvalidData(s.clone()),
        ImageError::Unsupported(s) => ImageError::Unsupported(s),
        ImageError::Cancelled => ImageError::Cancelled,
        ImageError::Sandbox(f) => ImageError::Sandbox(*f),
    }
}

/// `want` hedefi `have` hedefine çözülmüş kareden türetilebilir mi (yeniden ayrıştırmadan).
fn derivable(fit: Fit, have: (u32, u32), want: (u32, u32)) -> bool {
    if want.0 > have.0 || want.1 > have.1 || want == have {
        return false;
    }
    match fit {
        Fit::Contain | Fit::Fill => true,
        // Kırpılmış Cover: kare `have`'in en-boy oranında kırpıldı; başka oran kırpılan bölgeyi
        // isteyebilir. ±1 px aynı oran.
        Fit::Cover => {
            let implied_w = f64::from(have.0) * f64::from(want.1) / f64::from(have.1);
            (implied_w - f64::from(want.0)).abs() <= 1.0
        }
    }
}

/// İşin nasıl karşılandığı (sayaç).
enum Served {
    Hit,
    Derived,
    Decoded,
}

impl WallpaperDecoder {
    /// Yalıtılmış çözücüyle (duvar kağıdı profilli `nuxut_image_sandbox::Sandbox`:
    /// `Sandbox::new(SandboxConfig::wallpaper())`). Thread açmaz (ilk istekte).
    pub fn new(decoder: Arc<dyn IsolatedDecoder>, opts: WallpaperDecoderOptions) -> Arc<Self> {
        Arc::new(Self { backend: Backend::Isolated(decoder), opts, state: Mutex::new(State::default()), wake: Condvar::new() })
    }

    /// **Yalnızca örnekler ve testler:** süreç İÇİNDE, `Trust::System`, ilk kare (JPEG XL
    /// yalnızca `jxl` özelliğiyle). Kullanıcı dosyası shell/lock/settings'in adres alanına
    /// girer; üründe kullanılmaz.
    pub fn in_process_unsandboxed(opts: WallpaperDecoderOptions) -> Arc<Self> {
        Arc::new(Self { backend: Backend::InProcessUnsandboxed, opts, state: Mutex::new(State::default()), wake: Condvar::new() })
    }

    /// İsteği kuyruğa koyar; `done` çözüm thread'inde çağrılır (isabette de). Dönen bilet
    /// düşünce bu bekleyen iptal edilir (`done` hiç çağrılmaz).
    pub fn request(self: &Arc<Self>, req: WallpaperRequest, done: impl FnOnce(Result<WallpaperFrame, ImageError>) + Send + 'static) -> WallpaperTicket {
        let ident = Ident { path: req.path, target_px: req.target_px, fit: req.fit };
        let mut st = lock(&self.state);
        st.next_id += 1;
        let waiter_id = st.next_id;
        let waiter = Waiter { id: waiter_id, done: Box::new(done) };
        let job_id = match st.jobs.iter_mut().find(|j| j.ident == ident && !j.cancel.is_cancelled()) {
            Some(job) => {
                job.waiters.push(waiter);
                let id = job.id;
                st.stats.joined += 1;
                id
            }
            None => {
                st.next_id += 1;
                let id = st.next_id;
                st.jobs.push(Job { id, ident, waiters: vec![waiter], running: false, cancel: CancelFlag::new() });
                id
            }
        };
        if st.thread_alive {
            self.wake.notify_all();
        } else {
            st.thread_alive = true;
            let me = self.clone();
            let spawned = std::thread::Builder::new().name("nuxut-wallpaper".into()).spawn(move || me.run());
            if spawned.is_err() {
                st.thread_alive = false;
                let jobs = std::mem::take(&mut st.jobs);
                drop(st);
                tracing::warn!("duvar kağıdı çözüm thread'i başlatılamadı");
                for w in jobs.into_iter().flat_map(|j| j.waiters) {
                    (w.done)(Err(ImageError::Decode("çözüm thread'i başlatılamadı".into())));
                }
                return WallpaperTicket { decoder: Weak::new(), job: 0, waiter: 0 };
            }
        }
        WallpaperTicket { decoder: Arc::downgrade(self), job: job_id, waiter: waiter_id }
    }

    /// Bekleyen işler dışında tutulan bütün kareleri bırakır (bellek baskısı, kilitlenme).
    pub fn trim(&self) {
        let mut st = lock(&self.state);
        st.cache.clear();
        self.wake.notify_all();
    }

    pub fn stats(&self) -> WallpaperDecodeStats {
        let st = lock(&self.state);
        WallpaperDecodeStats { resident_bytes: st.resident(), entries: st.cache.len(), pending: st.jobs.len(), thread_alive: st.thread_alive, ..st.stats }
    }

    fn cancel_waiter(&self, job: u64, waiter: u64) {
        let mut st = lock(&self.state);
        let Some(i) = st.jobs.iter().position(|j| j.id == job) else { return };
        let j = &mut st.jobs[i];
        j.waiters.retain(|w| w.id != waiter);
        if !j.waiters.is_empty() {
            return;
        }
        if j.running {
            j.cancel.cancel();
        } else {
            st.jobs.remove(i);
            st.stats.cancelled += 1;
        }
    }

    /// Sıradaki iş: kuyruğun başındaki bekleyen işle aynı dosya + yerleşimdeki en büyük hedef
    /// (küçükler ondan türetilsin).
    fn next_job(st: &State) -> Option<usize> {
        let front = st.jobs.iter().position(|j| !j.running)?;
        let f = &st.jobs[front].ident;
        let area = |t: (u32, u32)| u64::from(t.0) * u64::from(t.1);
        st.jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| !j.running && j.ident.path == f.path && j.ident.fit == f.fit)
            .max_by_key(|(i, j)| (area(j.ident.target_px), std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
    }

    /// Çözüm thread'i: iş varken çözer; yoksa tutulan karelerin süresini bekler, bitince çıkar.
    fn run(self: Arc<Self>) {
        crate::loader::lower_thread_priority();
        let mut st = lock(&self.state);
        loop {
            if let Some(i) = Self::next_job(&st) {
                let job = &mut st.jobs[i];
                job.running = true;
                let (id, ident, cancel) = (job.id, job.ident.clone(), job.cancel.clone());
                drop(st);
                let result = self.serve(&ident, &cancel);
                st = lock(&self.state);
                let Some(pos) = st.jobs.iter().position(|j| j.id == id) else { continue };
                let job = st.jobs.remove(pos);
                let result = match result {
                    Ok((frame, file, served)) => {
                        match served {
                            Served::Hit => st.stats.hits += 1,
                            Served::Derived => st.stats.derived += 1,
                            Served::Decoded => {}
                        }
                        self.insert(&mut st, file, &ident, &frame);
                        Ok(frame)
                    }
                    Err(ImageError::Cancelled) => {
                        st.stats.cancelled += 1;
                        Err(ImageError::Cancelled)
                    }
                    Err(e) => {
                        st.stats.failed += 1;
                        tracing::warn!(error = ?e, "duvar kağıdı çözülemedi");
                        Err(e)
                    }
                };
                drop(st);
                // Sonuç kilit dışında teslim edilir (bekleyen yeni istek atabilir).
                if !matches!(result, Err(ImageError::Cancelled)) {
                    for w in job.waiters {
                        let r = match &result {
                            Ok(f) => Ok(f.clone()),
                            Err(e) => Err(dup_error(e)),
                        };
                        (w.done)(r);
                    }
                }
                drop(result);
                st = lock(&self.state);
                continue;
            }
            // Kuyruk boş: süresi dolan kareleri bırak; kalmadıysa thread çıkar.
            let now = Instant::now();
            let linger = self.opts.linger;
            st.cache.retain(|e| now.saturating_duration_since(e.last_used) < linger);
            let Some(next) = st.cache.iter().map(|e| e.last_used + linger).min() else { break };
            st = self.wake.wait_timeout(st, next.saturating_duration_since(now)).unwrap_or_else(|e| e.into_inner()).0;
        }
        st.thread_alive = false;
    }

    fn insert(&self, st: &mut State, file: FileId, ident: &Ident, frame: &WallpaperFrame) {
        let now = Instant::now();
        if let Some(e) = st.cache.iter_mut().find(|e| e.file == file && e.target_px == ident.target_px && e.fit == ident.fit) {
            e.last_used = now;
            return;
        }
        let size = frame.byte_size();
        if size > self.opts.cache_bytes {
            return;
        }
        while st.resident() + size > self.opts.cache_bytes {
            let Some(oldest) = st.cache.iter().enumerate().min_by_key(|(_, e)| e.last_used).map(|(i, _)| i) else { break };
            st.cache.remove(oldest);
        }
        st.cache.push(Entry { file, target_px: ident.target_px, fit: ident.fit, frame: frame.clone(), last_used: now });
    }

    /// Tek iş: dosyayı aç, kimliğini al; önbellekte varsa isabet, türetilebiliyorsa türet,
    /// yoksa aynı fd'yi çözücüye ver.
    fn serve(&self, ident: &Ident, cancel: &CancelFlag) -> Result<(WallpaperFrame, FileId, Served), ImageError> {
        let file = crate::open_user_file(&ident.path)?;
        let meta = file.metadata().map_err(|source| ImageError::Io { path: PathBuf::new(), source })?;
        let id = FileId::new(&ident.path, &meta);
        let source = {
            let mut st = lock(&self.state);
            let now = Instant::now();
            if let Some(e) = st.cache.iter_mut().find(|e| e.file == id && e.target_px == ident.target_px && e.fit == ident.fit) {
                e.last_used = now;
                return Ok((e.frame.clone(), id, Served::Hit));
            }
            st.cache
                .iter_mut()
                .filter(|e| e.file == id && e.fit == ident.fit && derivable(ident.fit, e.target_px, ident.target_px))
                .min_by_key(|e| u64::from(e.target_px.0) * u64::from(e.target_px.1))
                .map(|e| {
                    e.last_used = now;
                    e.frame.clone()
                })
        };
        if cancel.is_cancelled() {
            return Err(ImageError::Cancelled);
        }
        if let Some(src) = source {
            let frame = match crate::resize::fit_premultiplied(&src.pixels, src.size(), target_for(ident.fit, ident.target_px))? {
                Some(img) => WallpaperFrame::from_image(img, src.animated_source),
                None => src,
            };
            return Ok((frame, id, Served::Derived));
        }
        lock(&self.state).stats.decodes += 1;
        let opts = DecodeOptions { limits: Limits::WALLPAPER, target: Some(target_for(ident.fit, ident.target_px)), animate: false };
        let decoded = match &self.backend {
            Backend::Isolated(d) => d.decode_file(crate::file_key("wallpaper", &ident.path), file, &opts, cancel)?,
            Backend::InProcessUnsandboxed => decode_in_process(file, &opts)?,
        };
        if cancel.is_cancelled() {
            return Err(ImageError::Cancelled);
        }
        Ok((WallpaperFrame::from_image(decoded.image, decoded.animated_source), id, Served::Decoded))
    }
}

/// Süreç içi (yalnızca örnek/test) çözüm: `System` güven, ilk kare.
fn decode_in_process(file: std::fs::File, opts: &DecodeOptions) -> Result<DecodedFile, ImageError> {
    let opts = DecodeOptions { limits: opts.limits.with_trust(Trust::System), ..*opts };
    let bytes = crate::decode::read_limited(file, Path::new(""), opts.limits.max_file_bytes)?;
    let image = crate::decode_bytes(&bytes, &opts)?.into_first_frame();
    Ok(DecodedFile::from_source(image, &bytes, &opts.limits))
}

/// [`WallpaperDecoder::request`] bileti: düşünce bekleyen iptal edilir.
#[derive(Debug)]
pub struct WallpaperTicket {
    decoder: Weak<WallpaperDecoder>,
    job: u64,
    waiter: u64,
}

impl Drop for WallpaperTicket {
    fn drop(&mut self) {
        if let Some(d) = self.decoder.upgrade() {
            d.cancel_waiter(self.job, self.waiter);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derivation_rules() {
        assert!(derivable(Fit::Contain, (3840, 2160), (1920, 1080)));
        assert!(derivable(Fit::Fill, (3840, 2160), (1920, 1200)));
        assert!(!derivable(Fit::Contain, (1920, 1080), (3840, 2160)), "büyütme yok");
        assert!(!derivable(Fit::Contain, (1920, 1080), (1920, 1080)), "aynı hedef isabettir");
        assert!(derivable(Fit::Cover, (3840, 2160), (1920, 1080)), "aynı oran");
        assert!(derivable(Fit::Cover, (3840, 2160), (1366, 768)), "±1 px");
        assert!(!derivable(Fit::Cover, (3840, 2160), (1366, 772)), "oran 2+ px farklı");
        assert!(!derivable(Fit::Cover, (3840, 2160), (1920, 1200)), "16:9 → 16:10 kırpılan bölgeyi ister");
    }

    #[test]
    fn errors_duplicate_without_content() {
        let e = ImageError::Io { path: PathBuf::new(), source: std::io::ErrorKind::NotFound.into() };
        assert!(matches!(dup_error(&e), ImageError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound));
        assert!(matches!(dup_error(&ImageError::Sandbox(crate::SandboxFailure::Timeout)), ImageError::Sandbox(crate::SandboxFailure::Timeout)));
    }

    #[test]
    fn request_debug_hides_path() {
        let r = WallpaperRequest::new("/home/kişi/tatil.jpg", (0, 5), Fit::Cover);
        assert_eq!(r.target_px, (1, 5));
        assert!(!format!("{r:?}").contains("kişi"));
    }
}
