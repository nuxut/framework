//! Duvar kağıdı içe aktarma (ADR-0004): kaynak bir kez, en ucuz oynatılan biçime dönüştürülür
//! ve `$XDG_CACHE_HOME/nuxut/wallpapers/` altında saklanır. Kaynak dosyaya dokunulmaz.
//!
//! - Durağan görsel → çıkışa cover küçültülmüş QOI.
//! - Animasyonlu GIF/WebP/APNG → döngüsel H.264 (ya da VA-API'nin çözdüğü en ucuz codec) `.mkv`.
//! - Video → büyükse ya da codec'i pahalıysa hedef boyutta yeniden kodlanır; değilse kaynak
//!   olduğu gibi kullanılır (yalnızca küçük bir bağlantı kaydı yazılır).
//!
//! **Yalıtım (ADR-0007, `docs/design/video-import-sandbox.md`):** kullanıcı görselinin tüm
//! ayrıştırması (durağan görsel çözümü, animasyon tespiti ve kare sayısı/süre için kap yapısı
//! yürüyüşü) [`IsolatedDecoder`]'da — üretimde yalıtılmış işçide — yapılır; ana süreç yalnızca
//! ilk 4 KiB'lık imzaya bakar. Animasyonu ve videoyu mpv çözer (süreç içi, ADR-0002 riski).

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, UNIX_EPOCH};

use nuxut_image::{AnimationInfo, CancelFlag, DecodeOptions, DecodedFile, Fit, Image, ImageError, IsolatedDecoder, Limits, TargetSize};
use serde::{Deserialize, Serialize};

use crate::budget::{HumanDuration, ImportLimits, StreamBudget};
use crate::ffi::{Event, Mpv};
use crate::source::{MediaSource, SourceError, open_media};
use crate::hwcaps::{self, Codec, DecodeCaps, RunEnd};
use crate::VideoError;

#[derive(Clone, PartialEq, Eq)]
pub struct ImportRequest {
    pub source: PathBuf,
    /// Çıkışın fiziksel piksel boyutu.
    pub target_px: (u32, u32),
}

/// İçe aktarma sonucu: oynatılacak dosya.
#[derive(Clone, PartialEq, Eq)]
pub enum Imported {
    Image(PathBuf),
    Video(PathBuf),
}

// Yollar yazılmaz (loglarda kullanıcı içeriği yok).
impl std::fmt::Debug for ImportRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportRequest").field("source", &format_args!("<yol gizli>")).field("target_px", &self.target_px).finish()
    }
}

impl std::fmt::Debug for Imported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Image(_) => "Image(<yol gizli>)",
            Self::Video(_) => "Video(<yol gizli>)",
        })
    }
}

impl std::fmt::Debug for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { source, .. } => f.debug_struct("Io").field("path", &format_args!("<gizli>")).field("source", source).finish(),
            other => write!(f, "{other}"),
        }
    }
}

impl Imported {
    pub fn path(&self) -> &Path {
        match self {
            Self::Image(p) | Self::Video(p) => p,
        }
    }
}

#[derive(Debug)]
pub enum ImportEvent {
    /// 0…1.
    Progress(f32),
    Done(Result<Imported, ImportError>),
}

#[derive(thiserror::Error)]
pub enum ImportError {
    #[error("G/Ç hatası: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("görsel: {0}")]
    Image(#[from] nuxut_image::ImageError),
    #[error("{0}")]
    Video(#[from] VideoError),
    #[error("kaynak reddedildi: {0}")]
    Source(#[from] SourceError),
    #[error("desteklenmeyen dosya: {0}")]
    Unsupported(String),
    #[error("dönüştürme başarısız: {0}")]
    Encode(String),
    #[error("iptal edildi")]
    Cancelled,
    /// Kaynak süre ya da kare bütçesini aşıyor ([`ImportLimits`]); kodlanmadı ya da kodlama
    /// durduruldu, yarım çıktı silindi.
    #[error("içe aktarılamadı, {0}")]
    Budget(#[from] crate::BudgetExceeded),
    /// Kodlama [`ImportLimits::encode_timeout`] içinde bitmedi; durduruldu, yarım çıktı silindi.
    #[error("dönüştürme {} içinde bitmedi, durduruldu", HumanDuration(*.0))]
    TimedOut(Duration),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> ImportError + '_ {
    move |source| ImportError::Io { path: path.to_owned(), source }
}

/// Görsel çözümünde iptal/yalıtım hatası `ImportError`'a: iptal her yerde aynı varyant.
fn image_error(e: ImageError) -> ImportError {
    match e {
        ImageError::Cancelled => ImportError::Cancelled,
        e => ImportError::Image(e),
    }
}

/// Dönüştürme tutamağı. Düşürmek iptal etmez (iş arka planda biter ve önbelleğe yazar).
#[derive(Debug, Clone)]
pub struct ImportJob {
    cancel: CancelFlag,
}

impl ImportJob {
    /// Kodlama yoklamasında ve (süren görsel çözümünde) ≤ 50 ms'de işçiyi öldürerek durdurur.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// `IsolatedDecoder::decode_file` anahtar alanı (susturma kovası; `file_key`).
const KEY_DOMAIN: &str = "video-import";

/// Kullanıcı görselinin ayrıştırıldığı yer.
enum Backend {
    /// Üretim: yalıtılmış çözücü (`nuxut_image_sandbox::Sandbox`, `SandboxConfig::wallpaper()`).
    Isolated(Arc<dyn IsolatedDecoder>),
    /// Yalnızca örnekler/testler: süreç içi, `Trust::System` (ADR-0007 dışı; adı açıkça söylüyor).
    InProcessUnsandboxed,
}

/// Önbellek girdisi (kaynak kimliği → içerik hash'i): büyük videolar her açılışta yeniden
/// hash'lenmesin.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct IndexEntry {
    len: u64,
    mtime_ns: u128,
    hash: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    #[serde(default)]
    sources: BTreeMap<String, IndexEntry>,
}

const MAX_INDEX: usize = 4096;
/// Varsayılan önbellek sınırı.
pub const DEFAULT_CACHE_LIMIT: u64 = 2 << 30;
/// Hedef boyuttan bu orandan fazla büyük video küçültülür.
const DOWNSCALE_SLACK: f64 = 1.1;

struct Inner {
    dir: PathBuf,
    /// Önbellek bayt sınırı; aşılırsa en eski kullanılanlar silinir.
    limit: std::sync::atomic::AtomicU64,
    caps: OnceLock<DecodeCaps>,
    /// Kodlama bütçesi; her içe aktarmanın başında okunur.
    limits: Mutex<ImportLimits>,
    index: Mutex<Option<Index>>,
    backend: Backend,
}

/// Duvar kağıdı içe aktarıcısı. Ucuz klonlanır.
#[derive(Clone)]
pub struct Importer(Arc<Inner>);

impl std::fmt::Debug for Importer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Importer");
        s.field("dir", &self.0.dir);
        match &self.0.backend {
            Backend::Isolated(d) => s.field("decoder", &d.isolation()),
            Backend::InProcessUnsandboxed => s.field("decoder", &format_args!("InProcessUnsandboxed")),
        };
        s.finish()
    }
}

impl Importer {
    /// `$XDG_CACHE_HOME/nuxut/wallpapers`. `decoder`: kullanıcı görsellerini çözen yalıtılmış
    /// çözücü — zorunlu (ADR-0007; `Wallpaper::new` ile aynı ilke: unutulursa sessizce süreç
    /// içi çözüm olmasın). Duvar kağıdı profili: `Sandbox::new(SandboxConfig::wallpaper())`;
    /// süreçte zaten bir duvar kağıdı `Sandbox`'ı varsa o paylaşılır.
    pub fn new(decoder: Arc<dyn IsolatedDecoder>) -> Self {
        Self::with_dir(Self::default_dir(), decoder)
    }

    pub fn with_dir(dir: PathBuf, decoder: Arc<dyn IsolatedDecoder>) -> Self {
        Self::build(dir, Backend::Isolated(decoder))
    }

    /// Yalnızca örnekler/testler: kullanıcı görseli **süreç içinde**, `Trust::System` ile
    /// çözülür (yalıtım yok). Ürün kodu [`with_dir`](Self::with_dir) kullanır.
    pub fn in_process_unsandboxed(dir: PathBuf) -> Self {
        Self::build(dir, Backend::InProcessUnsandboxed)
    }

    /// `$XDG_CACHE_HOME/nuxut/wallpapers` (yoksa geçici dizin altında).
    pub fn default_dir() -> PathBuf {
        dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("nuxut").join("wallpapers")
    }

    fn build(dir: PathBuf, backend: Backend) -> Self {
        Self(Arc::new(Inner {
            dir,
            limit: std::sync::atomic::AtomicU64::new(DEFAULT_CACHE_LIMIT),
            caps: OnceLock::new(),
            limits: Mutex::new(ImportLimits::DEFAULT),
            index: Mutex::new(None),
            backend,
        }))
    }

    pub fn cache_dir(&self) -> &Path {
        &self.0.dir
    }

    /// Önbellek bayt sınırı (varsayılan 2 GiB). Yeni dönüştürmeden sonra en eski kullanılan
    /// girdiler (değiştirilme zamanı; her isabette yenilenir) silinir.
    pub fn set_cache_limit(&self, bytes: u64) {
        self.0.limit.store(bytes, Ordering::Relaxed);
    }

    /// Kodlama bütçesi (varsayılan [`ImportLimits::DEFAULT`]); sonraki içe aktarmalarda geçerli.
    pub fn set_limits(&self, limits: ImportLimits) {
        *self.0.limits.lock().unwrap_or_else(|e| e.into_inner()) = limits;
    }

    pub fn limits(&self) -> ImportLimits {
        *self.0.limits.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// VA-API kod çözme yetenekleri (ilk çağrıda yoklanır ya da önbellekten okunur; ilk
    /// yoklama birkaç saniye sürebilir — UI thread'inde çağırmayın).
    pub fn capabilities(&self) -> &DecodeCaps {
        self.0.caps.get_or_init(|| {
            if crate::available() {
                hwcaps::load_or_probe(&self.0.dir.join("hwcaps.toml"), &self.0.dir.join(".probe"))
            } else {
                DecodeCaps::default()
            }
        })
    }

    /// Önbellekte hazır sonuç (hash bilinen ve dosyası duran girdi için; dosya okumaz).
    pub fn lookup(&self, req: &ImportRequest) -> Option<Imported> {
        let hash = self.known_hash(&req.source)?;
        self.cached(&hash, req.target_px)
    }

    /// Arka planda içe aktarır; olaylar o thread'den gelir.
    pub fn start(&self, req: ImportRequest, events: impl Fn(ImportEvent) + Send + 'static) -> ImportJob {
        let cancel = CancelFlag::new();
        let job = ImportJob { cancel: cancel.clone() };
        let this = self.clone();
        let spawned = std::thread::Builder::new().name("nuxut-import".into()).spawn(move || {
            let r = this.run(&req, &cancel, &|p| events(ImportEvent::Progress(p)));
            events(ImportEvent::Done(r));
        });
        if let Err(e) = spawned {
            tracing::error!("içe aktarma thread'i başlatılamadı: {e}");
        }
        job
    }

    /// Çağıran thread'de içe aktarır (testler ve araçlar için). `cancel` kurulunca kodlama ve
    /// süren görsel çözümü (işçi ≤ 50 ms'de öldürülür) durur, `Err(Cancelled)`.
    pub fn run(&self, req: &ImportRequest, cancel: &CancelFlag, progress: &dyn Fn(f32)) -> Result<Imported, ImportError> {
        let (tw, th) = (req.target_px.0.max(1), req.target_px.1.max(1));
        let hash = self.hash(&req.source, cancel)?;
        progress(0.05);
        if let Some(hit) = self.cached(&hash, (tw, th)) {
            progress(1.0);
            return Ok(hit);
        }
        std::fs::create_dir_all(&self.0.dir).map_err(io(&self.0.dir))?;
        let stem = format!("{hash}-{tw}x{th}");
        let limits = self.limits();
        // Ana süreç yalnızca imzaya bakar (piksel/kap ayrıştırması yok).
        let head = read_head(&req.source)?;
        let result = if nuxut_image::is_image(&head) {
            // Tek ayrıştırma, yalıtılmış çözücüde: cover küçültülmüş ilk kare + animasyon
            // bayrağı + (animasyonsa) kap yapısından kare sayısı ve süre. Sınır kapıları
            // (dosya, kenar, piksel, çözücü bütçesi) çözücüdedir.
            let decoded = self.decode_image(open_media(&req.source)?, &req.source, TargetSize::new(tw, th, Fit::Cover), cancel)?;
            if decoded.animated_source {
                self.import_animation(req, &stem, decoded.animation, limits, cancel, progress)?
            } else {
                Imported::Image(self.encode_still(&decoded.image, &stem)?)
            }
        } else {
            self.import_video(req, &stem, limits, cancel, progress)?
        };
        self.prune(result.path());
        progress(1.0);
        Ok(result)
    }

    fn cached(&self, hash: &str, (w, h): (u32, u32)) -> Option<Imported> {
        let stem = self.0.dir.join(format!("{hash}-{w}x{h}"));
        let qoi = stem.with_extension("qoi");
        if qoi.is_file() {
            touch(&qoi);
            return Some(Imported::Image(qoi));
        }
        let mkv = stem.with_extension("mkv");
        if mkv.is_file() {
            touch(&mkv);
            return Some(Imported::Video(mkv));
        }
        // Dönüştürme gerekmeyen kaynak: bağlantı kaydı.
        let link_path = stem.with_extension("link");
        let link = std::fs::read_to_string(&link_path).ok()?;
        touch(&link_path);
        let (kind, path) = link.trim_end().split_once('\t')?;
        let path = PathBuf::from(path);
        path.is_file().then(|| if kind == "image" { Imported::Image(path) } else { Imported::Video(path) })
    }

    fn known_hash(&self, source: &Path) -> Option<String> {
        let (key, entry) = identity(source)?;
        let mut guard = self.0.index.lock().unwrap_or_else(|e| e.into_inner());
        let index = guard.get_or_insert_with(|| self.load_index());
        index.sources.get(&key).filter(|e| e.len == entry.0 && e.mtime_ns == entry.1).map(|e| e.hash.clone())
    }

    fn load_index(&self) -> Index {
        std::fs::read_to_string(self.0.dir.join("index.toml")).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default()
    }

    /// İçerik hash'i (BLAKE3, ilk 16 onaltılık hane). Kaynak kimliği değişmediyse indeksten.
    fn hash(&self, source: &Path, cancel: &CancelFlag) -> Result<String, ImportError> {
        if let Some(h) = self.known_hash(source) {
            return Ok(h);
        }
        let mut file = open_media(source)?;
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if cancel.is_cancelled() {
                return Err(ImportError::Cancelled);
            }
            let n = file.read(&mut buf).map_err(io(source))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let hash = hasher.finalize().to_hex()[..16].to_owned();
        if let Some((key, (len, mtime_ns))) = identity(source) {
            let mut guard = self.0.index.lock().unwrap_or_else(|e| e.into_inner());
            let index = guard.get_or_insert_with(|| self.load_index());
            if index.sources.len() >= MAX_INDEX {
                index.sources.clear();
            }
            index.sources.insert(key, IndexEntry { len, mtime_ns, hash: hash.clone() });
            if let Ok(text) = toml::to_string(&*index) {
                let _ = std::fs::create_dir_all(&self.0.dir);
                let _ = write_atomic(&self.0.dir.join("index.toml"), text.as_bytes());
            }
        }
        Ok(hash)
    }

    /// Kullanıcı görselini (ya da mpv'nin güvenilmeyen medyadan yazdığı kareyi) çözücüde açar:
    /// duvar kağıdı profili sınırları (`Limits::WALLPAPER`), `target` (kırpmasız Cover: QOI
    /// bugünkü gibi kutuyu kaplar, oynatıcı kendi yerleşimini uygular), ilk kare. `file` bir
    /// kez açılmış normal dosyadır (kullanıcı kaynağı `open_media`: sembolik bağ yok); aynı fd
    /// çözücüye akıtılır, `path` yalnızca anahtar içindir. Süreç içi tam okuma yok.
    fn decode_image(&self, file: std::fs::File, path: &Path, target: TargetSize, cancel: &CancelFlag) -> Result<DecodedFile, ImportError> {
        let opts = DecodeOptions { limits: Limits::WALLPAPER, target: Some(target), animate: false };
        match &self.0.backend {
            Backend::Isolated(d) => d.decode_file(nuxut_image::file_key(KEY_DOMAIN, path), file, &opts, cancel).map_err(image_error),
            Backend::InProcessUnsandboxed => decode_in_process(file, &opts, cancel),
        }
    }

    /// Animasyonlu görsel → döngüsel video. Bütçe çözücünün kap yapısından saydığı kesin kare
    /// sayısı ve süreyle önceden denetlenir (aşan animasyon mpv'ye hiç gitmez); tuval boyutu
    /// kodlayacak olan mpv'nin yoklamasından alınır (ana süreç kabı ayrıştırmaz).
    fn import_animation(
        &self,
        req: &ImportRequest,
        stem: &str,
        info: Option<AnimationInfo>,
        limits: ImportLimits,
        cancel: &CancelFlag,
        progress: &dyn Fn(f32),
    ) -> Result<Imported, ImportError> {
        if let Some(info) = info {
            limits.check(Some(info.duration), Some(info.frames))?;
        }
        let encoded = (|| {
            if !crate::available() {
                return Err(VideoError::Unavailable("libmpv yok".into()).into());
            }
            let probe = probe_video(&req.source)?;
            // Kodlayıcıya giden tuvalin üst sınırı (mpv her kareyi tam tuvalde çözer).
            Limits::DEFAULT.check_dimensions(probe.size.0, probe.size.1).map_err(AnimationFail::Hard)?;
            let size = nuxut_image::fit_size(probe.size, TargetSize::new(req.target_px.0.max(1), req.target_px.1.max(1), Fit::Cover));
            // Kare sayısı bilinmiyorsa (yapı tanınmadı) akışta kabın tahmini de denetlenir.
            let budget = StreamBudget { limits, estimate: info.is_none() };
            self.encode_video(req, stem, EncodeSpec { size, has_audio: false, budget }, cancel, progress).map_err(AnimationFail::from)
        })();
        match encoded {
            Ok(p) => Ok(Imported::Video(p)),
            Err(AnimationFail::Hard(e)) => Err(e.into()),
            Err(AnimationFail::Import(e @ (ImportError::Cancelled | ImportError::Budget(_) | ImportError::TimedOut(_)))) => Err(e),
            Err(AnimationFail::Import(e)) => {
                // mpv yok ya da bu animasyonu çözemiyor (ör. eski FFmpeg + animasyonlu WebP):
                // kaynak görsel olarak kalır; `Wallpaper` ilk kareyi yalıtılmış çözücüde açar.
                tracing::warn!("animasyon videoya dönüştürülemedi ({e}); kaynak kullanılacak");
                Ok(Imported::Image(req.source.clone()))
            }
        }
    }

    fn encode_still(&self, img: &Image, stem: &str) -> Result<PathBuf, ImportError> {
        let path = self.0.dir.join(format!("{stem}.qoi"));
        write_atomic(&path, &encode_qoi(img)?)?;
        Ok(path)
    }

    fn import_video(&self, req: &ImportRequest, stem: &str, limits: ImportLimits, cancel: &CancelFlag, progress: &dyn Fn(f32)) -> Result<Imported, ImportError> {
        let probe = probe_video(&req.source)?;
        let caps = self.capabilities();
        let (tw, th) = req.target_px;
        let (w, h) = probe.size;
        // Cover yerleşiminde gereken kaynak boyutu: iki kenardan biri hedefi karşılamalı.
        let scale = (f64::from(tw) / f64::from(w.max(1))).max(f64::from(th) / f64::from(h.max(1)));
        let too_big = scale * DOWNSCALE_SLACK < 1.0;
        let expensive = caps.expensive(probe.codec);
        if !too_big && !expensive {
            let link = format!("video\t{}\n", req.source.display());
            write_atomic(&self.0.dir.join(format!("{stem}.link")), link.as_bytes())?;
            return Ok(Imported::Video(req.source.clone()));
        }
        tracing::info!(
            "video dönüştürülüyor: {:?} {w}x{h} → {:?} (büyük: {too_big}, pahalı codec: {expensive})",
            probe.codec,
            caps.target_codec()
        );
        // Bütçe: kabın süresi ve kare hızı (çözmeden yoklandı); aşan video kodlanmaz. Kap kare
        // hızı bildirmiyorsa birkaç kareden tahmin (dalgalanır: sınırdaki kaynak yanlışlıkla
        // reddedilmesin diye %5 pay; kesin sınır kodlamadaki `frames`).
        limits.check_estimate(probe.duration, None)?;
        let fps = probe.fps.or_else(|| probe.duration.and_then(|_| estimate_fps(&req.source)).map(|f| f / 1.05));
        limits.check_estimate(probe.duration, fps)?;
        let size = if too_big { ((f64::from(w) * scale).round() as u32, (f64::from(h) * scale).round() as u32) } else { (w, h) };
        let budget = StreamBudget { limits, estimate: true };
        let path = self.encode_video(req, stem, EncodeSpec { size, has_audio: probe.has_audio, budget }, cancel, progress)?;
        Ok(Imported::Video(path))
    }

    /// mpv kodlama kipiyle `size` boyutunda (çift kenarlı, yuv420p) `.mkv` üretir.
    fn encode_video(
        &self,
        req: &ImportRequest,
        stem: &str,
        EncodeSpec { size, has_audio, budget }: EncodeSpec,
        cancel: &CancelFlag,
        progress: &dyn Fn(f32),
    ) -> Result<PathBuf, ImportError> {
        if !crate::available() {
            return Err(VideoError::Unavailable("libmpv yok".into()).into());
        }
        let codec = self.capabilities().target_codec();
        let (enc, encopts) = codec.encoder();
        let vf = format!("scale=w={}:h={},format=yuv420p", even(size.0), even(size.1));
        let final_path = self.0.dir.join(format!("{stem}.mkv"));
        let tmp = self.0.dir.join(format!(".{stem}.{}.tmp.mkv", std::process::id()));
        let tmp_s = tmp.to_str().ok_or_else(|| ImportError::Unsupported("UTF-8 olmayan önbellek yolu".into()))?.to_owned();
        let source = MediaSource::File(open_media(&req.source)?);
        let mut options: Vec<(&str, &str)> = vec![
            ("o", &tmp_s),
            ("of", "matroska"),
            ("ovc", enc),
            ("ovcopts", encopts),
            ("vf", &vf),
            ("loop-file", "no"),
        ];
        if has_audio {
            options.extend([("oac", "libopus"), ("oacopts", "b=160000")]);
        } else {
            options.push(("audio", "no"));
        }
        let timeout = budget.limits.encode_timeout;
        // run_mpv mpv örneğini dönmeden önce eşzamanlı yok eder: buradan sonra geçici dosyaya
        // yazan kalmaz, silmek güvenli.
        let end = hwcaps::run_mpv(&options, source, timeout, Some(budget), &mut |p| {
            progress(0.05 + 0.95 * p);
            !cancel.is_cancelled()
        });
        let fail = |e: ImportError| {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        };
        if cancel.is_cancelled() {
            return fail(ImportError::Cancelled);
        }
        match end {
            RunEnd::Done => {}
            RunEnd::OverBudget(e) => return fail(e.into()),
            RunEnd::TimedOut => {
                tracing::warn!("kodlama zaman aşımı ({timeout:?}); durduruldu");
                return fail(ImportError::TimedOut(timeout));
            }
            RunEnd::Stopped => return fail(ImportError::Cancelled),
            RunEnd::Failed => return fail(ImportError::Encode(format!("mpv {enc} kodlaması başarısız"))),
        }
        if std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0) == 0 {
            return fail(ImportError::Encode(format!("mpv {enc} kodlaması boş çıktı verdi")));
        }
        std::fs::rename(&tmp, &final_path).map_err(io(&final_path))?;
        Ok(final_path)
    }
}

/// Kodlama isteği: çıkış boyutu, ses izi korunsun mu, bütçe.
struct EncodeSpec {
    size: (u32, u32),
    has_audio: bool,
    budget: StreamBudget,
}

fn even(v: u32) -> u32 {
    (v.max(2)) & !1
}

/// Kaynak kimliği: kanonik yol + (boyut, mtime).
fn identity(source: &Path) -> Option<(String, (u64, u128))> {
    let path = std::fs::canonicalize(source).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some((path.to_string_lossy().into_owned(), (meta.len(), mtime)))
}

/// Animasyon dönüştürmesinin hatası: `Hard` her durumda döner (tuval sınırı), `Import`
/// iptal/bütçe/zaman aşımı dışında kaynağa (durağan gösterime) düşer.
enum AnimationFail {
    Hard(ImageError),
    Import(ImportError),
}

impl From<ImportError> for AnimationFail {
    fn from(e: ImportError) -> Self {
        Self::Import(e)
    }
}

impl From<VideoError> for AnimationFail {
    fn from(e: VideoError) -> Self {
        Self::Import(e.into())
    }
}

/// Yalnızca [`Importer::in_process_unsandboxed`] (örnek/test): süreç içi, `System` güven.
fn decode_in_process(file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag) -> Result<DecodedFile, ImportError> {
    let opts = DecodeOptions { limits: opts.limits.with_trust(nuxut_image::Trust::System), ..*opts };
    let limit = opts.limits.max_file_bytes;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).map_err(|source| ImportError::Io { path: PathBuf::new(), source })?;
    if bytes.len() as u64 > limit {
        return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit }.into());
    }
    if cancel.is_cancelled() {
        return Err(ImportError::Cancelled);
    }
    let image = nuxut_image::decode_bytes(&bytes, &opts)?.into_first_frame();
    Ok(DecodedFile::from_source(image, &bytes, &opts.limits))
}

/// Güvenilir (çözülmüş) RGBA'dan QOI.
fn encode_qoi(img: &Image) -> Result<Vec<u8>, ImportError> {
    let (w, h) = img.size();
    let mut out = Vec::with_capacity(img.byte_size() / 2);
    image::codecs::qoi::QoiEncoder::new(&mut out)
        .write_image(&img.to_straight(), w, h, image::ExtendedColorType::Rgba8)
        .map_err(|e| ImportError::Encode(e.to_string()))?;
    Ok(out)
}

fn read_head(path: &Path) -> Result<Vec<u8>, ImportError> {
    let mut head = Vec::with_capacity(4096);
    open_media(path)?.take(4096).read_to_end(&mut head).map_err(io(path))?;
    Ok(head)
}

struct VideoProbe {
    size: (u32, u32),
    codec: Option<Codec>,
    has_audio: bool,
    /// Kabın bildirdiği süre (sn) ve kare hızı; bilinmiyorsa `None` (akışta denetlenir).
    duration: Option<f64>,
    fps: Option<f64>,
}

/// Video bilgisi (mpv, `vo=null`, çözmeden: dosya yüklenince özellikler okunur).
fn probe_video(path: &Path) -> Result<VideoProbe, ImportError> {
    let file = open_media(path)?;
    let m = Mpv::new(&[("vo", "null"), ("ao", "null"), ("pause", "yes")])?;
    m.load(MediaSource::File(file), true)?;
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while std::time::Instant::now() < deadline {
        match m.wait_event(0.5) {
            Event::FileLoaded => {
                if !m.single_media() {
                    return Err(ImportError::Unsupported("playlist kabul edilmez".into()));
                }
                let w = m.get_i64("width").unwrap_or(0);
                let h = m.get_i64("height").unwrap_or(0);
                if w <= 0 || h <= 0 {
                    return Err(ImportError::Unsupported("video akışı yok".into()));
                }
                let codec = m.get_string("video-format").as_deref().and_then(Codec::from_ffmpeg_name);
                let has_audio = m.get_string("current-tracks/audio/codec").is_some();
                let (duration, fps) = (m.get_f64("duration"), m.get_f64("container-fps"));
                return Ok(VideoProbe { size: (w as u32, h as u32), codec, has_audio, duration, fps });
            }
            Event::EndFile { .. } | Event::Shutdown => break,
            _ => {}
        }
    }
    Err(ImportError::Unsupported("mpv açamadı".into()))
}

/// Kap kare hızı bildirmeyen (ör. Matroska) videoda kare hızı tahmini: ayrı bir örnekle en çok
/// 2 sn gerçek zamanlı (`vo=null`) oynatılır, mpv'nin çözülen karelerden tahmini
/// (`estimated-vf-fps`) okunur. Yalnızca kodlanacak videoda çağrılır.
fn estimate_fps(path: &Path) -> Option<f64> {
    let file = open_media(path).ok()?;
    let m = Mpv::new(&[("vo", "null"), ("ao", "null"), ("audio", "no")]).ok()?;
    m.load(MediaSource::File(file), true).ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match m.wait_event(0.05) {
            Event::FileLoaded if !m.single_media() => return None,
            Event::EndFile { .. } | Event::Shutdown => return None,
            _ => {}
        }
        if let Some(f) = m.get_f64("estimated-vf-fps").filter(|f| f.is_finite() && *f > 0.0) {
            return Some(f);
        }
    }
    None
}

/// Aynı dizinde geçici dosya + `rename`.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ImportError> {
    use std::io::Write;
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let r = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    r.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        ImportError::Io { path: path.to_owned(), source: e }
    })
}

use image::ImageEncoder as _;

/// LRU için erişim zamanı: değiştirilme zamanını şimdiye çeker (içerik değişmez).
fn touch(path: &Path) {
    if let Ok(f) = std::fs::File::options().append(true).open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

impl Importer {
    /// Önbellek sınırı aşıldıysa en eski kullanılan girdileri siler (`keep` hariç).
    fn prune(&self, keep: &Path) {
        let limit = self.0.limit.load(Ordering::Relaxed);
        let Ok(rd) = std::fs::read_dir(&self.0.dir) else { return };
        let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
            .filter_map(Result::ok)
            .filter(|e| {
                let p = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                !name.starts_with('.') && matches!(p.extension().and_then(|x| x.to_str()), Some("qoi" | "mkv" | "link"))
            })
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = entries.iter().map(|e| e.1).sum();
        if total <= limit {
            return;
        }
        entries.sort_by_key(|e| e.0);
        for (_, len, path) in entries {
            if total <= limit {
                break;
            }
            if path == keep {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                tracing::debug!("önbellekten silindi (LRU, {len} bayt)");
                total = total.saturating_sub(len);
            }
        }
    }

    /// Videonun bir karesinden durağan duvar kağıdı (QOI): donanım çözme yoksa video yerine
    /// önerilebilecek ucuz yedek. `at` saniye (videonun içinde kalacak şekilde sınırlanır).
    pub fn still_frame(&self, req: &ImportRequest, at: f64) -> Result<Imported, ImportError> {
        let cancel = CancelFlag::new();
        let (tw, th) = (req.target_px.0.max(1), req.target_px.1.max(1));
        let hash = self.hash(&req.source, &cancel)?;
        let path = self.0.dir.join(format!("{hash}-{tw}x{th}-frame.qoi"));
        if path.is_file() {
            touch(&path);
            return Ok(Imported::Image(path));
        }
        if !crate::available() {
            return Err(VideoError::Unavailable("libmpv yok".into()).into());
        }
        std::fs::create_dir_all(&self.0.dir).map_err(io(&self.0.dir))?;
        let png = self.0.dir.join(format!(".{hash}.{}.frame.png", std::process::id()));
        let (png_s, src) = (path_str(&png)?, MediaSource::File(open_media(&req.source)?));
        let start = format!("{:.3}", at.max(0.0));
        let ok = hwcaps::run_mpv(
            &[
                ("o", &png_s),
                ("of", "image2"),
                ("ovc", "png"),
                ("audio", "no"),
                ("start", &start),
                ("frames", "1"),
            ],
            src,
            Duration::from_secs(60),
            None,
            &mut |_| true,
        ) == RunEnd::Done;
        let result = (|| {
            if !ok || !png.is_file() {
                return Err(ImportError::Encode("kare alınamadı".into()));
            }
            // mpv'nin yazdığı PNG'nin içeriği güvenilmeyen medyadan gelir: aynı çözücüde açılır.
            let file = nuxut_image::open_user_file(&png)?;
            let img = self.decode_image(file, &png, TargetSize::new(tw, th, Fit::Cover), &cancel)?.image;
            write_atomic(&path, &encode_qoi(&img)?)?;
            Ok(Imported::Image(path.clone()))
        })();
        let _ = std::fs::remove_file(&png);
        if result.is_ok() {
            self.prune(&path);
        }
        result
    }
}

fn path_str(p: &Path) -> Result<String, ImportError> {
    p.to_str().map(str::to_owned).ok_or_else(|| ImportError::Unsupported("UTF-8 olmayan yol".into()))
}
