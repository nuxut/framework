//! `Importer` yalıtımı (ADR-0007 takip, `docs/design/video-import-sandbox.md`): kullanıcı
//! görseli yalnızca verilen `IsolatedDecoder`'da çözülür; ana süreç piksel ya da kap yapısı
//! ayrıştırmaz, animasyon bütçesi çözücünün (üretimde işçinin, tel v4) beyanıyla denetlenir,
//! çözücü hatası süreç içi çözüme düşmez. İşçi yokken (S1) temel raster `Sandbox`'ın K2-b
//! yedeğinde, SVG asla.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nuxut_image::{
    AnimationInfo, CancelFlag, DecodeOptions, DecodedFile, Fit, Image, ImageError, IsolatedDecoder, Isolation, Limits, SandboxFailure,
    TargetSize,
};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};
use nuxut_video::{BudgetExceeded, ImportError, ImportRequest, Imported, Importer};

/// `decode_file` çağrısının kaydı.
#[derive(Debug, Clone, PartialEq)]
struct Call {
    key: u64,
    limits: Limits,
    target: Option<TargetSize>,
    animate: bool,
}

/// Sahte yalıtılmış çözücü: dosyaya hiç bakmadan verilen yanıtı döndürür ve çağrıyı kaydeder.
/// Bayt yolu (`decode`) kullanılmamalı.
#[derive(Debug)]
struct Fake {
    answer: Mutex<Option<Result<DecodedFile, ImageError>>>,
    calls: Mutex<Vec<Call>>,
}

impl Fake {
    fn new(answer: Result<DecodedFile, ImageError>) -> Arc<Self> {
        Arc::new(Self { answer: Mutex::new(Some(answer)), calls: Mutex::new(Vec::new()) })
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl IsolatedDecoder for Fake {
    fn isolation(&self) -> Isolation {
        Isolation::Process
    }

    fn decode(&self, _key: u64, _bytes: Arc<[u8]>, _opts: &DecodeOptions) -> Result<Image, ImageError> {
        panic!("Importer bayt yolunu kullanmamalı (dosya belleğe alınmaz)");
    }

    fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, _cancel: &CancelFlag) -> Result<DecodedFile, ImageError> {
        assert!(file.metadata().unwrap().is_file());
        self.calls.lock().unwrap().push(Call { key, limits: opts.limits, target: opts.target, animate: opts.animate });
        self.answer.lock().unwrap().take().expect("tek çağrı beklenir")
    }
}

/// Tek renk (opak) kare: çıktının çözücüden geldiği renginden anlaşılır.
fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Image {
    Image::from_premultiplied(w, h, rgba.repeat((w * h) as usize)).unwrap()
}

fn still(img: Image) -> DecodedFile {
    DecodedFile { image: img, animated_source: false, animation: None }
}

fn animated(animation: Option<AnimationInfo>) -> DecodedFile {
    DecodedFile { image: solid(4, 4, [0, 0, 0, 255]), animated_source: true, animation }
}

/// PNG imzası + çöp: süreç içinde çözülmeye ya da kabı yürünmeye kalkılsa hata verir.
fn hostile_png(dir: &Path) -> PathBuf {
    let p = dir.join("dusmanca.png");
    std::fs::write(&p, b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR bozuk kap yapisi, piksel yok").unwrap();
    p
}

fn run(imp: &Importer, source: &Path, px: (u32, u32)) -> Result<Imported, ImportError> {
    imp.run(&ImportRequest { source: source.to_owned(), target_px: px }, &CancelFlag::new(), &|_| {})
}

fn cache_files(cache: &Path) -> Vec<String> {
    std::fs::read_dir(cache)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| !n.ends_with(".toml")).collect())
        .unwrap_or_default()
}

/// Durağan görsel: tek `decode_file` (duvar kağıdı sınırları, Cover hedef, ilk kare, dosya
/// anahtarı) ve QOI çözücünün karesinden yazılır. Kaynak süreç içinde çözülemeyecek bir
/// dosyadır: başarı, ana sürecin onu ayrıştırmadığını gösterir.
#[test]
fn still_image_is_decoded_only_by_the_isolated_decoder() {
    let dir = tempfile::tempdir().unwrap();
    let src = hostile_png(dir.path());
    let fake = Fake::new(Ok(still(solid(7, 5, [200, 0, 100, 255]))));
    let imp = Importer::with_dir(dir.path().join("cache"), fake.clone());
    let Imported::Image(qoi) = run(&imp, &src, (64, 32)).unwrap() else { panic!("görsel bekleniyordu") };
    assert_eq!(
        fake.calls(),
        vec![Call {
            key: nuxut_image::file_key("video-import", &src),
            limits: Limits::WALLPAPER,
            target: Some(TargetSize::new(64, 32, Fit::Cover)),
            animate: false,
        }]
    );
    let out = nuxut_image::decode_file(&qoi, &Default::default()).unwrap().into_first_frame();
    assert_eq!(out.size(), (7, 5));
    assert_eq!(out.straight_pixel(3, 2), [200, 0, 100, 255]);
    assert!(format!("{imp:?}").contains("Process"), "{imp:?}");
}

/// Animasyon bütçesi işçinin kap yapısından saydığı değerle, kodlayıcıya (mpv) gitmeden
/// denetlenir; ana süreç kabı yürümez (kaynak çöp).
#[test]
fn animation_budget_uses_the_decoders_count() {
    let dir = tempfile::tempdir().unwrap();
    let src = hostile_png(dir.path());
    let cache = dir.path().join("cache");
    let info = AnimationInfo { frames: 10_000_000, duration: Duration::from_secs(100_000) };
    let imp = Importer::with_dir(cache.clone(), Fake::new(Ok(animated(Some(info)))));
    let e = run(&imp, &src, (64, 64)).unwrap_err();
    assert!(matches!(e, ImportError::Budget(BudgetExceeded::Duration { .. } | BudgetExceeded::Frames { .. })), "{e}");
    assert!(cache_files(&cache).is_empty(), "{:?}", cache_files(&cache));
}

/// Animasyon videoya dönüştürülemezse (mpv yok ya da bu kaynağı açamıyor) kaynak görsel olarak
/// kalır; ana süreç kendisi çözmeye kalkmaz.
#[test]
fn unconvertible_animation_stays_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = hostile_png(dir.path());
    let imp = Importer::with_dir(dir.path().join("cache"), Fake::new(Ok(animated(None))));
    assert_eq!(run(&imp, &src, (64, 64)).unwrap(), Imported::Image(src));
}

/// Çözücü hatası olduğu gibi döner (süreç içi yedeğe düşülmez, önbelleğe bir şey yazılmaz);
/// iptal `Cancelled`.
#[test]
fn decoder_errors_are_not_retried_in_process() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let src = dir.path().join("iyi.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).save(&src).unwrap();
    let fake = Fake::new(Err(ImageError::Sandbox(SandboxFailure::Unavailable)));
    let imp = Importer::with_dir(cache.clone(), fake.clone());
    let e = run(&imp, &src, (8, 8)).unwrap_err();
    assert!(matches!(e, ImportError::Image(ImageError::Sandbox(SandboxFailure::Unavailable))), "{e}");
    assert_eq!(fake.calls().len(), 1);
    assert!(cache_files(&cache).is_empty(), "{:?}", cache_files(&cache));
    let imp = Importer::with_dir(cache.clone(), Fake::new(Err(ImageError::Cancelled)));
    assert!(matches!(run(&imp, &src, (8, 8)), Err(ImportError::Cancelled)));
}

/// S1 (ADR-0007): işçi bulunamayan gerçek `Sandbox` ile temel raster süreç içinde (K2-b,
/// `Untrusted` kapısı) içe aktarılır; SVG asla süreç içinde çözülmez; yedek kapalıysa
/// (`in_process_fallback: false`) PNG de reddedilir.
#[test]
fn without_worker_basic_raster_falls_back_and_svg_never_decodes() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    let cfg = || SandboxConfig { worker: Some(dir.path().join("yok")), rate_per_sec: 1_000, burst: 1_000, ..SandboxConfig::wallpaper() };
    let sb = Arc::new(Sandbox::new(cfg()));
    assert_eq!(sb.isolation(), Isolation::Unavailable);
    let imp = Importer::with_dir(cache.clone(), sb.clone());
    let png = dir.path().join("w.png");
    image::RgbaImage::from_pixel(80, 40, image::Rgba([9, 99, 199, 255])).save(&png).unwrap();
    let Imported::Image(qoi) = run(&imp, &png, (20, 20)).unwrap() else { panic!("görsel bekleniyordu") };
    assert_eq!(nuxut_image::decode_file(&qoi, &Default::default()).unwrap().into_first_frame().size(), (40, 20));
    assert_eq!(sb.stats().in_process_fallbacks, 1);
    let svg = dir.path().join("w.svg");
    std::fs::write(&svg, br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#).unwrap();
    let e = run(&imp, &svg, (20, 20)).unwrap_err();
    assert!(matches!(e, ImportError::Image(ImageError::Sandbox(SandboxFailure::Unavailable))), "{e}");
    assert_eq!(sb.stats().in_process_fallbacks, 1);
    let strict = Arc::new(Sandbox::new(SandboxConfig { in_process_fallback: false, ..cfg() }));
    let imp = Importer::with_dir(dir.path().join("cache2"), strict);
    assert!(matches!(run(&imp, &png, (20, 20)), Err(ImportError::Image(ImageError::Sandbox(SandboxFailure::Unavailable)))));
}
