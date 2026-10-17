//! Arka plan yükleyicisi: çözme, küçültme ve ikon araması UI thread'ini bloklamaz.
//!
//! ```ignore
//! let sender = cx.sender();
//! loader.load(LoadRequest::path(p, opts), move |r| { let _ = sender.send(Msg::Loaded(r)); });
//! ```
//!
//! Thread'ler ilk `load` çağrısında yaratılır: runtime'ın sinyal maskesini miras alsınlar diye
//! `Loader` olay işleyicilerinde (ör. ilk `output_added`) kullanılmalı (ARCHITECTURE: Sinyaller).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::cursor::{CursorRole, CursorTheme};
use crate::icon::IconTheme;
use crate::{DecodeOptions, Decoded, ImageError, IsolatedDecoder, NO_KEY, Trust};

/// Yüklenecek kaynak.
#[derive(Clone)]
pub enum Source {
    Path(PathBuf),
    Bytes(Arc<[u8]>),
    /// Temadan ikon: `size` mantıksal, `scale` tamsayı ölçek. Hedef boyut `size × scale`.
    Icon { theme: Arc<IconTheme>, name: String, size: u32, scale: u32 },
    /// Temadan adaylardan ilk bulunan ikon ([`IconTheme::load_first_as`]).
    IconAny { theme: Arc<IconTheme>, names: Arc<[String]>, size: u32, scale: u32 },
    /// XCursor teması imleci ([`CursorTheme::load`]); `size` fiziksel piksel nominal boyut.
    /// `options.animate` ise tüm kareler (`Decoded::Animated`), değilse ilk kare;
    /// `options.target` varsa kareler ona küçültülür (büyütülmez).
    Cursor { theme: Arc<CursorTheme>, role: CursorRole, size: u32 },
    /// Ham RGB(A) satırları (bildirim `image-data`); bkz. [`crate::downscale_rgba`].
    Raw { pixels: Arc<[u8]>, width: u32, height: u32, rowstride: u32, has_alpha: bool, max_side: u32 },
    /// Tepsi ARGB32 pixmap'i (ağ bayt sırası); bkz. [`crate::downscale_argb32`].
    Argb32 { pixels: Arc<[u8]>, width: i32, height: i32, max_side: u32 },
}

/// Yol, bayt ve ikon adı yazılmaz; yalnızca tür ve boyutlar (loglarda kullanıcı içeriği yok).
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(_) => f.write_str("Path(<yol gizli>)"),
            Self::Bytes(b) => write!(f, "Bytes({} bayt)", b.len()),
            Self::Icon { name, size, scale, .. } => write!(f, "Icon(<{} bayt ad gizli>, {size}@{scale})", name.len()),
            Self::IconAny { names, size, scale, .. } => write!(f, "IconAny({} ad, {size}@{scale})", names.len()),
            Self::Cursor { theme, role, size } => write!(f, "Cursor({:?}, {role:?}, {size})", theme.name()),
            Self::Raw { width, height, has_alpha, max_side, .. } => {
                write!(f, "Raw({width}x{height}, alpha: {has_alpha}, max_side: {max_side})")
            }
            Self::Argb32 { width, height, max_side, .. } => write!(f, "Argb32({width}x{height}, max_side: {max_side})"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadRequest {
    pub source: Source,
    pub options: DecodeOptions,
}

impl LoadRequest {
    pub fn path(path: impl Into<PathBuf>, options: DecodeOptions) -> Self {
        Self { source: Source::Path(path.into()), options }
    }

    /// Ham RGB(A) satırları, arka planda doğrulanıp `max_side`'a küçültülür.
    pub fn raw(pixels: Arc<[u8]>, width: u32, height: u32, rowstride: u32, has_alpha: bool, max_side: u32) -> Self {
        Self {
            source: Source::Raw { pixels, width, height, rowstride, has_alpha, max_side },
            options: DecodeOptions { limits: crate::Limits::DEFAULT, target: None, animate: false },
        }
    }

    pub fn icon(theme: Arc<IconTheme>, name: impl Into<String>, size: u32, scale: u32) -> Self {
        Self {
            source: Source::Icon { theme, name: name.into(), size, scale },
            options: DecodeOptions { limits: crate::Limits::ICON, target: None, animate: false },
        }
    }

    /// Adaylardan ilk bulunan ikon (`size × scale` fiziksel piksele). Güven varsayılanı
    /// `Untrusted`; sistem temasından bizim seçtiğimiz adlar için `.with_trust(Trust::System)`.
    pub fn icon_any(theme: Arc<IconTheme>, names: Arc<[String]>, size: u32, scale: u32) -> Self {
        Self {
            source: Source::IconAny { theme, names, size, scale },
            options: DecodeOptions { limits: crate::Limits::ICON, target: None, animate: false },
        }
    }

    /// İmleç: `size` fiziksel piksel nominal boyut; `animate` tüm kareler.
    pub fn cursor(theme: Arc<CursorTheme>, role: CursorRole, size: u32, animate: bool) -> Self {
        Self {
            source: Source::Cursor { theme, role, size },
            options: DecodeOptions { limits: crate::Limits::ICON, target: None, animate },
        }
    }

    /// İsteğin güveni (`options.limits.trust`). Varsayılan `Untrusted`; `System` yalnızca
    /// sistem temasından bizim seçtiğimiz ad ya da kullanıcının seçtiği dosya için verilir.
    pub fn with_trust(mut self, trust: crate::Trust) -> Self {
        self.options.limits.trust = trust;
        self
    }

    /// Çağıran thread'de, hemen (testler ve araçlar için).
    pub fn run(&self) -> Result<Decoded, ImageError> {
        match &self.source {
            Source::Path(p) => crate::decode_file(p, &self.options),
            Source::Bytes(b) => crate::decode_bytes(b, &self.options),
            Source::Icon { theme, name, size, scale } => theme.load_as(name, *size, *scale, self.options.trust()).map(Decoded::Still),
            Source::IconAny { theme, names, size, scale } => {
                theme.load_first_as(names, *size, *scale, self.options.trust()).map(Decoded::Still)
            }
            Source::Cursor { theme, role, size } => self.run_cursor(theme, *role, *size, None),
            // Ham pikseller de isteğin sınır kapısından geçer (biçimlerle eşit).
            Source::Raw { pixels, width, height, rowstride, has_alpha, max_side } => {
                self.options.limits.check_dimensions(*width, *height)?;
                crate::downscale_rgba(pixels, *width, *height, *rowstride, *has_alpha, *max_side).map(Decoded::Still)
            }
            Source::Argb32 { pixels, width, height, max_side } => {
                let (Ok(w), Ok(h)) = (u32::try_from(*width), u32::try_from(*height)) else {
                    return Err(ImageError::InvalidData(format!("negatif boyut {width}x{height}")));
                };
                self.options.limits.check_dimensions(w, h)?;
                crate::downscale_argb32(pixels, *width, *height, *max_side).map(Decoded::Still)
            }
        }
    }

    /// [`run`](Self::run), güvenilmeyen kaynak yalıtılmış işçide (ADR-0007): `Untrusted`
    /// `Path`/`Bytes`, etkin güveni `Untrusted`'a düşen `Icon`/`IconAny` dosyası (ör.
    /// `~/.local/share/icons` altındaki SVG ikon teması) ve kullanıcı dizinindeki yalnızca-SVG
    /// hyprcursor şekli burada okunur, baytları `sandbox`'a verilir; sonuç tek karedir (K5).
    /// Kullanıcı dizinindeki hyprcursor arşivinin PNG kareleri de işçiye gider (tek kare).
    /// `System` güven, Xcursor (kendi sınırlı ayrıştırıcısı) ve ham pikseller (`Raw`/`Argb32`:
    /// biçim ayrıştırması yok, yalnızca doğrulanıp küçültülür) süreç içinde kalır. `sandbox`
    /// `None` ise [`run`](Self::run) ile aynıdır.
    ///
    /// `key` [`NO_KEY`] ise yalıtılmış çözücüye kaynaktan türetilmiş bir anahtar gider (yol/ad
    /// özeti; bayt kaynağı [`crate::ANON_KEY`] kovası): anahtarsız güvenilmeyen iş de susturulur.
    pub fn run_with(&self, sandbox: Option<&dyn IsolatedDecoder>, key: u64) -> Result<Decoded, ImageError> {
        let Some(sandbox) = sandbox else { return self.run() };
        let key = if key == NO_KEY { self.source_key() } else { key };
        let untrusted = self.options.trust() == Trust::Untrusted;
        let isolated = |bytes: Arc<[u8]>, opts: &DecodeOptions| sandbox.decode(key, bytes, opts).map(Decoded::Still);
        let icon = |name: &str, theme: &IconTheme, size: u32, scale: u32| {
            let (file, path, opts) = theme.open_as(name, size, scale, self.options.trust())?;
            if opts.trust() == Trust::System {
                return crate::decode::decode_open_file(file, &path, &opts).map(|d| Decoded::Still(d.into_first_frame()));
            }
            let bytes = crate::decode::read_limited(file, &path, opts.limits.max_file_bytes.min(sandbox.max_input_bytes()))?;
            isolated(bytes.into(), &opts)
        };
        match &self.source {
            Source::Path(p) if untrusted => {
                let file = crate::decode::open_regular(p)?;
                let bytes = crate::decode::read_limited(file, p, self.options.limits.max_file_bytes.min(sandbox.max_input_bytes()))?;
                isolated(bytes.into(), &self.options)
            }
            Source::Bytes(b) if untrusted => isolated(b.clone(), &self.options),
            Source::Icon { theme, name, size, scale } => icon(name, theme, *size, *scale),
            Source::IconAny { theme, names, size, scale } => {
                let name = theme.first_name(names, *size, *scale).ok_or_else(|| ImageError::NotFound(String::new()))?;
                icon(name, theme, *size, *scale)
            }
            // Şekil arşivi zaten sınırlı okunmuştur (`zip::MAX_ARCHIVE_BYTES`, girdi `Limits::ICON`).
            Source::Cursor { theme, role, size } => {
                let decode = |bytes: Arc<[u8]>, opts: &DecodeOptions| sandbox.decode(key, bytes, opts);
                self.run_cursor(theme, *role, *size, Some(&decode))
            }
            _ => self.run(),
        }
    }

    /// Anahtarsız iş için yalıtılmış çözücü anahtarı (bulgu 4d). Yalnızca süreç içi kova
    /// kimliği; loglanmaz.
    fn source_key(&self) -> u64 {
        match &self.source {
            Source::Path(p) => crate::derive_key(&[b"path", p.to_string_lossy().as_bytes()]),
            Source::Icon { name, .. } => crate::derive_key(&[b"icon", name.as_bytes()]),
            Source::IconAny { names, .. } => {
                let mut parts: Vec<&[u8]> = vec![b"icon-any"];
                parts.extend(names.iter().map(|n| n.as_bytes()));
                crate::derive_key(&parts)
            }
            Source::Cursor { theme, role, .. } => {
                crate::derive_key(&[b"cursor", theme.name().as_bytes(), format!("{role:?}").as_bytes()])
            }
            _ => crate::ANON_KEY,
        }
    }

    /// İmleç işi: kareler hedefe sığdırılır (büyütülmez); tek kare `Still`.
    fn run_cursor(
        &self,
        theme: &CursorTheme,
        role: CursorRole,
        size: u32,
        isolated: Option<crate::cursor::hyprcursor::Isolated<'_>>,
    ) -> Result<Decoded, ImageError> {
        let target = self.options.target;
        let mut frames = theme
            .load_with(role, size, self.options.animate, isolated)?
            .into_iter()
            .map(|f| Ok(crate::Frame { image: crate::resize::fit(f.image, target)?, delay: f.delay }))
            .collect::<Result<Vec<_>, ImageError>>()?;
        if frames.len() > 1 {
            Ok(Decoded::Animated(crate::Animation { frames }))
        } else {
            frames.pop().map(|f| Decoded::Still(f.image)).ok_or(ImageError::Empty)
        }
    }
}

/// İptal tutamağı. Düşürmek iptal etmez; [`cancel`](Self::cancel) iş başlamadıysa onu atlar
/// (callback `Err(Cancelled)` ile çağrılır), sürüyorsa sonucu atar.
#[derive(Debug, Clone)]
pub struct LoadHandle {
    cancelled: Arc<AtomicBool>,
}

impl LoadHandle {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

/// Bir işin iki anahtarı ([`Loader::load_with_keys`]). İkisi de [`NO_KEY`] olabilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoadKeys {
    /// İptal/birleştirme anahtarı: aynı `cancel` ile yeni iş gelince eskisi iptal edilir;
    /// [`Loader::cancel_key`] bunu kullanır. `NO_KEY` = iptal birleştirmesi yok.
    pub cancel: u64,
    /// Kaynak anahtarı: yalıtılmış çözücüye gider (susturma, hız sınırı). `NO_KEY` = Loader
    /// kaynaktan türetir (yol/ad özeti; bayt kaynağı `ANON_KEY` kovası).
    pub source: u64,
}

impl LoadKeys {
    /// İki rol için aynı anahtar ([`Loader::load_keyed`]'ın davranışı).
    pub const fn same(key: u64) -> Self {
        Self { cancel: key, source: key }
    }
}

type Callback = Box<dyn FnOnce(Result<Decoded, ImageError>) + Send>;

struct Job {
    request: LoadRequest,
    keys: LoadKeys,
    cancelled: Arc<AtomicBool>,
    done: Callback,
}

/// Thread'lerin paylaştığı durum: yalıtılmış çözücü ve anahtar başına bekleyen son iş.
#[derive(Default)]
struct Shared {
    sandbox: Mutex<Option<Arc<dyn IsolatedDecoder>>>,
    /// Anahtar → en son kuyruğa giren işin iptal bayrağı (yenisi gelince eskisi iptal).
    pending: Mutex<HashMap<u64, Arc<AtomicBool>>>,
}

impl Shared {
    fn sandbox(&self) -> Option<Arc<dyn IsolatedDecoder>> {
        self.sandbox.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// İş bitti: anahtarın kaydı hâlâ bu işe aitse silinir (harita boşta büyümez).
    fn finish(&self, key: u64, flag: &Arc<AtomicBool>) {
        if key == NO_KEY {
            return;
        }
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if pending.get(&key).is_some_and(|f| Arc::ptr_eq(f, flag)) {
            pending.remove(&key);
        }
    }
}

/// [`Loader`] kurulum seçenekleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoaderOptions {
    /// İş parçacığı sayısı (≥ 1).
    pub threads: usize,
    /// Çözme thread'leri düşük öncelikte çalışır: Linux'ta thread başına `nice` 19
    /// (`setpriority(PRIO_PROCESS, 0)` yalnızca çağıran thread'i etkiler; önceliği düşürmek
    /// yetki istemez). UI thread'iyle aynı çekirdeği paylaşırken çözme arayüzü aç bırakmaz
    /// (ör. bildirim görselleri). Diğer platformlarda yok sayılır (POSIX'te aynı çağrı tüm
    /// süreci etkilerdi).
    pub low_priority: bool,
}

impl Default for LoaderOptions {
    fn default() -> Self {
        Self { threads: 2, low_priority: false }
    }
}

/// Düşük öncelikli çözme thread'inin `nice` değeri.
#[cfg(target_os = "linux")]
const LOW_PRIORITY_NICE: i32 = 19;

/// Çağıran thread'in önceliğini düşürür (yalnızca Linux; başarısızlık yalnızca loglanır).
pub(crate) fn lower_thread_priority() {
    #[cfg(target_os = "linux")]
    if let Err(e) = rustix::process::setpriority_process(None, LOW_PRIORITY_NICE) {
        tracing::debug!("görsel çözme thread'inin önceliği düşürülemedi: {e}");
    }
}

/// Sabit sayıda iş parçacıklı yükleyici. Düşünce kuyruk kapanır; bekleyen işler iptal
/// edilmiş sayılır, thread'ler mevcut işi bitirip katılır.
pub struct Loader {
    threads: usize,
    low_priority: bool,
    tx: Option<Sender<Job>>,
    rx: Arc<Mutex<Receiver<Job>>>,
    shutdown: Arc<AtomicBool>,
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for Loader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loader")
            .field("threads", &self.threads)
            .field("low_priority", &self.low_priority)
            .field("started", &self.workers.len())
            .field("sandbox", &self.shared.sandbox().map(|s| s.isolation()))
            .finish()
    }
}

impl Default for Loader {
    fn default() -> Self {
        Self::with_options(LoaderOptions::default())
    }
}

impl Loader {
    /// `threads` (≥ 1) normal öncelikli iş parçacığı; ilk `load`'da başlatılır.
    pub fn new(threads: usize) -> Self {
        Self::with_options(LoaderOptions { threads, ..LoaderOptions::default() })
    }

    /// Seçenekli kurulum (ör. düşük öncelikli çözme). Thread'ler yine ilk `load`'da başlar.
    pub fn with_options(options: LoaderOptions) -> Self {
        let (tx, rx) = channel();
        Self {
            threads: options.threads.max(1),
            low_priority: options.low_priority,
            tx: Some(tx),
            rx: Arc::new(Mutex::new(rx)),
            shutdown: Arc::new(AtomicBool::new(false)),
            shared: Arc::new(Shared::default()),
            workers: Vec::new(),
        }
    }

    /// Güvenilmeyen kaynakları yalıtılmış çözücüyle çözer (ADR-0007;
    /// `nuxut_image_sandbox::Sandbox`). Kurulmazsa bugünkü davranış sürer: `Untrusted` SVG
    /// reddedilir, raster süreç içinde çözülür. Sonraki işler için geçerlidir.
    pub fn with_sandbox(self, sandbox: Arc<dyn IsolatedDecoder>) -> Self {
        *self.shared.sandbox.lock().unwrap_or_else(|e| e.into_inner()) = Some(sandbox);
        self
    }

    /// İptal anahtarı ([`LoadKeys::cancel`]) `key` olan bekleyen (kuyruktaki ya da süren) işi
    /// iptal eder (kaynak anahtarına bakılmaz): callback `Err(Cancelled)`
    /// alır (sürüyorsa sonucu atılır). Böyle bir iş yoksa `false`. Tema önizlemelerinde tema
    /// değişince eski işleri düşürmek için ([`crate::preview::cancel_theme`]).
    pub fn cancel_key(&self, key: u64) -> bool {
        if key == NO_KEY {
            return false;
        }
        let pending = self.shared.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.get(&key).inspect(|f| f.store(true, Ordering::Relaxed)).is_some()
    }

    /// İşi kuyruğa ekler. `done` bir arka plan thread'inden çağrılır (tipik olarak
    /// `MessageSender::send` ile UI'a iletir).
    pub fn load(
        &mut self,
        request: LoadRequest,
        done: impl FnOnce(Result<Decoded, ImageError>) + Send + 'static,
    ) -> LoadHandle {
        self.load_keyed(NO_KEY, request, done)
    }

    /// [`load`](Self::load), tek anahtarla: iptal ve kaynak anahtarı aynı `key`
    /// ([`LoadKeys::same`]; bkz. [`load_with_keys`](Self::load_with_keys)). Aynı anahtarla
    /// bekleyen eski iş iptal edilir (`Err(Cancelled)`; sürüyorsa sonucu atılır): `NewIcon`
    /// fırtınası tek çözüme iner. [`NO_KEY`] = anahtarsız ([`load`](Self::load)).
    pub fn load_keyed(
        &mut self,
        key: u64,
        request: LoadRequest,
        done: impl FnOnce(Result<Decoded, ImageError>) + Send + 'static,
    ) -> LoadHandle {
        self.load_with_keys(LoadKeys::same(key), request, done)
    }

    /// İptal ve kaynak anahtarı ayrı: `keys.cancel` ile bekleyen eski iş iptal edilir
    /// ([`cancel_key`](Self::cancel_key) de yalnızca bunu kullanır); `keys.source` yalıtılmış
    /// çözücüye gider (hız sınırı, art arda başarısızlıkta susturma). Ör. bildirimde iptal =
    /// uygulama + yuva + bildirim kimliği (art arda gelen bildirimler birbirini iptal etmez),
    /// kaynak = uygulama (uygulama başına susturma korunur).
    pub fn load_with_keys(
        &mut self,
        keys: LoadKeys,
        request: LoadRequest,
        done: impl FnOnce(Result<Decoded, ImageError>) + Send + 'static,
    ) -> LoadHandle {
        self.start();
        let cancelled = Arc::new(AtomicBool::new(false));
        if keys.cancel != NO_KEY {
            let mut pending = self.shared.pending.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(old) = pending.insert(keys.cancel, cancelled.clone()) {
                old.store(true, Ordering::Relaxed);
            }
        }
        let job = Job { request, keys, cancelled: cancelled.clone(), done: Box::new(done) };
        if let Some(tx) = &self.tx
            && let Err(e) = tx.send(job)
        {
            self.shared.finish(keys.cancel, &cancelled);
            (e.0.done)(Err(ImageError::Cancelled));
        }
        LoadHandle { cancelled }
    }

    fn start(&mut self) {
        while self.workers.len() < self.threads {
            let rx = self.rx.clone();
            let shutdown = self.shutdown.clone();
            let shared = self.shared.clone();
            let name = format!("nuxut-image-{}", self.workers.len());
            let low_priority = self.low_priority;
            let spawned = std::thread::Builder::new().name(name).spawn(move || {
                if low_priority {
                    lower_thread_priority();
                }
                worker(&rx, &shutdown, &shared);
            });
            match spawned {
                Ok(h) => self.workers.push(h),
                Err(e) => {
                    tracing::error!("görsel yükleme thread'i başlatılamadı: {e}");
                    break;
                }
            }
        }
    }
}

fn worker(rx: &Mutex<Receiver<Job>>, shutdown: &AtomicBool, shared: &Shared) {
    loop {
        let job = {
            let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
            match rx.recv() {
                Ok(j) => j,
                Err(_) => return,
            }
        };
        if shutdown.load(Ordering::Relaxed) || job.cancelled.load(Ordering::Relaxed) {
            shared.finish(job.keys.cancel, &job.cancelled);
            (job.done)(Err(ImageError::Cancelled));
            continue;
        }
        let sandbox = shared.sandbox();
        let result = job.request.run_with(sandbox.as_deref(), job.keys.source);
        shared.finish(job.keys.cancel, &job.cancelled);
        if job.cancelled.load(Ordering::Relaxed) {
            (job.done)(Err(ImageError::Cancelled));
        } else {
            (job.done)(result);
        }
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.tx = None;
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn debug_hides_paths_bytes_and_icon_names() {
        let theme = Arc::new(IconTheme::with_dirs("Adwaita", vec!["/home/kişi/.icons".into()], vec![]));
        let reqs = [
            LoadRequest::path("/home/kişi/tatil.png", DecodeOptions::default()),
            LoadRequest { source: Source::Bytes(Arc::from(&b"kisi-sirri"[..])), options: DecodeOptions::default() },
            LoadRequest::icon(theme.clone(), "/home/kişi/özel.svg", 24, 2),
        ];
        for r in &reqs {
            let s = format!("{r:?}");
            assert!(!s.contains("kişi") && !s.contains("sirri") && !s.contains("tatil"), "{s}");
        }
        let s = format!("{theme:?}");
        assert!(s.contains("Adwaita") && !s.contains("kişi"), "{s}");
    }

    #[test]
    fn loads_in_background_and_reports_errors() {
        let mut loader = Loader::new(2);
        let (tx, rx) = mpsc::channel();
        let svg: Arc<[u8]> = Arc::from(&br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#[..]);
        let t1 = tx.clone();
        loader.load(LoadRequest { source: Source::Bytes(svg), options: DecodeOptions::default().with_trust(crate::Trust::System) }, move |r| {
            t1.send(("svg", r.map(|d| d.size()).map_err(|e| e.to_string()))).unwrap();
        });
        loader.load(LoadRequest::path("/nonexistent/x.png", DecodeOptions::default()), move |r| {
            tx.send(("missing", r.map(|d| d.size()).map_err(|e| e.to_string()))).unwrap();
        });
        let mut got: Vec<_> = (0..2).map(|_| rx.recv().unwrap()).collect();
        got.sort_by_key(|g| g.0);
        assert!(got[0].1.is_err());
        assert_eq!(got[1].1, Ok((4, 4)));
    }

    /// İş callback'i çözme thread'inde çalışır: o thread'in `nice` değeri okunur.
    #[cfg(target_os = "linux")]
    fn worker_nice(options: LoaderOptions) -> i32 {
        let mut loader = Loader::with_options(options);
        let (tx, rx) = mpsc::channel();
        loader.load(LoadRequest::path("/nonexistent", DecodeOptions::default()), move |_| {
            tx.send(rustix::process::getpriority_process(None).unwrap()).unwrap();
        });
        rx.recv().unwrap()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn low_priority_lowers_only_worker_threads() {
        let base = rustix::process::getpriority_process(None).unwrap();
        assert_eq!(worker_nice(LoaderOptions { threads: 1, low_priority: false }), base, "varsayılan öncelik değişmez");
        assert_eq!(worker_nice(LoaderOptions { threads: 1, low_priority: true }), LOW_PRIORITY_NICE);
        assert_eq!(rustix::process::getpriority_process(None).unwrap(), base, "çağıran thread etkilenmez");
    }

    #[test]
    fn cancelled_jobs_report_cancelled() {
        let mut loader = Loader::new(1);
        let (tx, rx) = mpsc::channel();
        // İlk iş thread'i meşgul tutarken ikinciyi iptal et.
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let g = gate.clone();
        let t0 = tx.clone();
        loader.load(LoadRequest::path("/nonexistent", DecodeOptions::default()), move |_| {
            drop(g.lock());
            t0.send(0).unwrap();
        });
        let h = loader.load(LoadRequest::path("/nonexistent", DecodeOptions::default()), move |r| {
            tx.send(if matches!(r, Err(ImageError::Cancelled)) { 1 } else { 2 }).unwrap();
        });
        h.cancel();
        drop(held);
        let mut got = vec![rx.recv().unwrap(), rx.recv().unwrap()];
        got.sort();
        assert_eq!(got, [0, 1]);
    }

    /// Çağrıları sayan, isteğe bağlı yavaş sahte yalıtılmış çözücü.
    #[derive(Debug, Default)]
    struct FakeSandbox {
        calls: std::sync::atomic::AtomicUsize,
        delay_ms: u64,
        last_key: std::sync::atomic::AtomicU64,
    }

    /// Bulgu 4d: anahtarsız güvenilmeyen iş çözücüye asla `NO_KEY` ile gitmez.
    #[test]
    fn keyless_untrusted_jobs_get_a_derived_key() {
        let fake = FakeSandbox::default();
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.svg"), dir.path().join("b.svg"));
        for p in [&a, &b] {
            std::fs::write(p, br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#).unwrap();
        }
        let key = |req: LoadRequest| {
            req.run_with(Some(&fake), NO_KEY).unwrap();
            fake.last_key.load(Ordering::SeqCst)
        };
        let (ka, kb) = (key(LoadRequest::path(&a, DecodeOptions::default())), key(LoadRequest::path(&b, DecodeOptions::default())));
        assert!(ka != NO_KEY && kb != NO_KEY && ka != kb, "yol başına ayrı anahtar");
        assert_eq!(key(LoadRequest::path(&a, DecodeOptions::default())), ka, "kararlı");
        let bytes = LoadRequest { source: Source::Bytes(Arc::from(&b"\x89PNG\r\n\x1a\n"[..])), options: DecodeOptions::default() };
        assert_eq!(key(bytes.clone()), crate::ANON_KEY, "bayt kaynağı anahtarsız kovaya");
        // Verilen anahtar aynen gider.
        bytes.run_with(Some(&fake), 42).unwrap();
        assert_eq!(fake.last_key.load(Ordering::SeqCst), 42);
    }

    impl IsolatedDecoder for FakeSandbox {
        fn isolation(&self) -> crate::Isolation {
            crate::Isolation::Process
        }
        fn decode(&self, key: u64, _bytes: Arc<[u8]>, _opts: &DecodeOptions) -> Result<crate::Image, ImageError> {
            self.last_key.store(key, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            crate::Image::from_premultiplied(1, 1, vec![0, 0, 0, 255])
        }
    }

    #[test]
    fn only_untrusted_encoded_sources_go_to_the_sandbox() {
        let fake = FakeSandbox::default();
        let svg: Arc<[u8]> = Arc::from(&br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#[..]);
        let untrusted = LoadRequest { source: Source::Bytes(svg.clone()), options: DecodeOptions::default() };
        let system = LoadRequest { source: Source::Bytes(svg), options: DecodeOptions::default().with_trust(Trust::System) };
        let raw = LoadRequest::raw(Arc::from(&[1u8, 2, 3, 255][..]), 1, 1, 4, true, 8);
        assert_eq!(untrusted.run_with(Some(&fake), NO_KEY).unwrap().size(), (1, 1));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        assert_eq!(system.run_with(Some(&fake), NO_KEY).unwrap().size(), (4, 4), "System süreç içinde");
        assert!(raw.run_with(Some(&fake), NO_KEY).is_ok(), "ham piksel süreç içinde");
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        // Sandbox'sız bugünkü davranış: Untrusted SVG reddedilir.
        assert!(matches!(untrusted.run_with(None, NO_KEY), Err(ImageError::Unsupported(_))));
        // Untrusted dosya yolu: bayt okunur, işçiye gider.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x.svg");
        std::fs::write(&p, br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#).unwrap();
        assert!(LoadRequest::path(&p, DecodeOptions::default()).run_with(Some(&fake), NO_KEY).is_ok());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    }

    /// Ev dizini dışındaki ama kök sahipli olmayan (geçici) tema dizini kullanıcı dizini gibi
    /// `Untrusted`'tır: `System` istense de SVG işçiye gider.
    #[test]
    fn untrusted_icon_any_svg_goes_to_the_sandbox() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("T/scalable/apps");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.path().join("T/index.theme"), "[Icon Theme]\nName=T\nDirectories=scalable/apps\n[scalable/apps]\nSize=48\nType=Scalable\nMinSize=8\nMaxSize=512\n").unwrap();
        std::fs::write(dir.join("kitty.svg"), br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#).unwrap();
        let theme = Arc::new(IconTheme::with_dirs("T", vec![root.path().to_owned()], Vec::new()));
        let names: Arc<[String]> = Arc::from(["utilities-terminal".to_owned(), "kitty".to_owned()]);
        let req = LoadRequest::icon_any(theme.clone(), names.clone(), 24, 1).with_trust(Trust::System);
        assert!(matches!(req.run_with(None, NO_KEY), Err(ImageError::Unsupported(_))), "işçisiz yer tutucu");
        let fake = FakeSandbox::default();
        assert_eq!(req.run_with(Some(&fake), 5).unwrap().size(), (1, 1));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
        // Hiçbir ad yoksa işçiye gidilmez.
        let none: Arc<[String]> = Arc::from(["yok-boyle-bir-ikon".to_owned()]);
        assert!(matches!(LoadRequest::icon_any(theme, none, 24, 1).run_with(Some(&fake), 5), Err(ImageError::NotFound(_))));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancel_key_cancels_only_that_pending_job() {
        let fake = Arc::new(FakeSandbox { delay_ms: 50, ..FakeSandbox::default() });
        let mut loader = Loader::new(1).with_sandbox(fake.clone());
        let (tx, rx) = mpsc::channel();
        let bytes: Arc<[u8]> = Arc::from(&b"\x89PNG\r\n\x1a\n...."[..]);
        for key in [11u64, 12, 13] {
            let tx = tx.clone();
            let req = LoadRequest { source: Source::Bytes(bytes.clone()), options: DecodeOptions::default() };
            loader.load_keyed(key, req, move |r| tx.send((key, matches!(r, Err(ImageError::Cancelled)))).unwrap());
        }
        assert!(loader.cancel_key(12));
        assert!(!loader.cancel_key(99) && !loader.cancel_key(NO_KEY));
        let mut got: Vec<_> = (0..3).map(|_| rx.recv().unwrap()).collect();
        got.sort();
        assert_eq!(got, [(11, false), (12, true), (13, false)]);
        assert!(lock_pending(&loader).is_empty());
    }

    /// İptal ve kaynak anahtarı ayrı: aynı kaynak + farklı iptal → iki iş de tamamlanır, ikisi de
    /// çözücüye aynı kaynak anahtarıyla gider; `cancel_key` kaynak anahtarını tanımaz.
    #[test]
    fn same_source_different_cancel_keys_both_complete() {
        let fake = Arc::new(FakeSandbox { delay_ms: 30, ..FakeSandbox::default() });
        let mut loader = Loader::new(1).with_sandbox(fake.clone());
        let (tx, rx) = mpsc::channel();
        let bytes: Arc<[u8]> = Arc::from(&b"\x89PNG\r\n\x1a\n...."[..]);
        for cancel in [101u64, 102] {
            let tx = tx.clone();
            let req = LoadRequest { source: Source::Bytes(bytes.clone()), options: DecodeOptions::default() };
            loader.load_with_keys(LoadKeys { cancel, source: 7 }, req, move |r| tx.send((cancel, r.is_ok())).unwrap());
        }
        assert!(!loader.cancel_key(7), "kaynak anahtarı iptal anahtarı değil");
        let mut got: Vec<_> = (0..2).map(|_| rx.recv().unwrap()).collect();
        got.sort();
        assert_eq!(got, [(101, true), (102, true)]);
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
        assert_eq!(fake.last_key.load(Ordering::SeqCst), 7, "çözücüye kaynak anahtarı gider");
        // Aynı iptal anahtarı (farklı kaynak) ise eskisi iptal edilir.
        let (tx, rx) = mpsc::channel();
        for source in [8u64, 9] {
            let tx = tx.clone();
            let req = LoadRequest { source: Source::Bytes(bytes.clone()), options: DecodeOptions::default() };
            loader.load_with_keys(LoadKeys { cancel: 200, source }, req, move |r| tx.send((source, r.is_ok())).unwrap());
        }
        let mut got: Vec<_> = (0..2).map(|_| rx.recv().unwrap()).collect();
        got.sort();
        assert_eq!(got, [(8, false), (9, true)]);
        assert!(lock_pending(&loader).is_empty());
    }

    #[test]
    fn same_key_storm_collapses_to_few_decodes() {
        let fake = Arc::new(FakeSandbox { delay_ms: 20, ..FakeSandbox::default() });
        let mut loader = Loader::new(1).with_sandbox(fake.clone());
        let (tx, rx) = mpsc::channel();
        let bytes: Arc<[u8]> = Arc::from(&b"\x89PNG\r\n\x1a\n...."[..]);
        for i in 0..500 {
            let tx = tx.clone();
            let req = LoadRequest { source: Source::Bytes(bytes.clone()), options: DecodeOptions::default() };
            loader.load_keyed(77, req, move |r| tx.send((i, r.is_ok())).unwrap());
        }
        drop(tx);
        let results: Vec<_> = (0..500).map(|_| rx.recv().unwrap()).collect();
        let ok: Vec<_> = results.iter().filter(|r| r.1).map(|r| r.0).collect();
        assert_eq!(ok, [499], "yalnızca en son istek sonuç verir");
        assert!(fake.calls.load(Ordering::SeqCst) <= 2, "{}", fake.calls.load(Ordering::SeqCst));
        // Bekleyen kayıt haritası boşaldı.
        assert!(lock_pending(&loader).is_empty());
    }

    fn lock_pending(loader: &Loader) -> std::sync::MutexGuard<'_, HashMap<u64, Arc<AtomicBool>>> {
        loader.shared.pending.lock().unwrap()
    }
}
