//! İçe aktarma bütçesi (ImportLimits): süre ve kare sınırı, sınırda geçme, akışta kesme.
//! Zaman aşımında süreç temizliği ayrı ikilide (`import_timeout.rs`): iş parçacığı sayımı
//! başka testlerden etkilenmesin.

use std::path::Path;
use nuxut_image::CancelFlag;
use std::time::Duration;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, RgbaImage};
use nuxut_video::{BudgetExceeded, ImportError, ImportLimits, ImportRequest, Imported, Importer};

fn req(source: &Path, w: u32, h: u32) -> ImportRequest {
    ImportRequest { source: source.to_owned(), target_px: (w, h) }
}

fn run(imp: &Importer, r: &ImportRequest) -> Result<Imported, ImportError> {
    imp.run(r, &CancelFlag::new(), &|_| {})
}

fn mpv() -> bool {
    let ok = nuxut_video::available();
    if !ok {
        eprintln!("libmpv yok; test atlandı");
    }
    ok
}

/// 12 kare × 80 ms = 960 ms.
fn write_gif(path: &Path) {
    let mut enc = GifEncoder::new(std::fs::File::create(path).unwrap());
    enc.set_repeat(Repeat::Infinite).unwrap();
    for i in 0..12u32 {
        let img = RgbaImage::from_fn(96, 64, |x, _| image::Rgba([((x + i * 8) % 96 * 2) as u8, 80, 200, 255]));
        enc.encode_frame(image::Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(80, 1))).unwrap();
    }
}

/// Kodlama kipiyle test videosu: MPEG-4 Part 2 (pahalı sayılır → her hedefte yeniden kodlanır),
/// 640x360, 30 fps, `secs` saniye, sessiz.
fn write_mpeg4(path: &Path, secs: u32) {
    let status = std::process::Command::new("mpv")
        .args(["--no-config", "--really-quiet", "--audio=no", "--ovc=mpeg4", "--of=matroska"])
        .arg(format!("av://lavfi:testsrc2=size=640x360:rate=30:duration={secs}"))
        .arg(format!("--o={}", path.display()))
        .status();
    assert!(status.is_ok_and(|s| s.success()) && path.is_file(), "test videosu üretilemedi");
}

fn limits(max_duration: Duration, max_frames: u64) -> ImportLimits {
    ImportLimits { max_duration, max_frames, ..ImportLimits::DEFAULT }
}

/// Önbellekte çıktı ya da geçici dosya kalmadı.
fn assert_no_output(cache: &Path) {
    let left: Vec<_> = std::fs::read_dir(cache)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".mkv") || n.contains(".tmp"))
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn animation_over_frame_budget_is_rejected_before_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("anim.gif");
    write_gif(&src);
    let cache = dir.path().join("cache");
    let imp = Importer::in_process_unsandboxed(cache.clone());
    imp.set_limits(limits(Duration::from_secs(60), 11));
    let e = run(&imp, &req(&src, 48, 48)).unwrap_err();
    assert!(matches!(e, ImportError::Budget(BudgetExceeded::Frames { actual: 12, limit: 11 })), "{e:?}");
    assert_eq!(e.to_string(), "içe aktarılamadı, kaynakta çok fazla kare: 12 (sınır 11)");
    assert_no_output(&cache);
}

#[test]
fn animation_over_duration_budget_is_rejected_before_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("anim.gif");
    write_gif(&src);
    let cache = dir.path().join("cache");
    let imp = Importer::in_process_unsandboxed(cache.clone());
    imp.set_limits(limits(Duration::from_millis(959), 1000));
    let e = run(&imp, &req(&src, 48, 48)).unwrap_err();
    assert!(
        matches!(e, ImportError::Budget(BudgetExceeded::Duration { actual, .. }) if actual == Duration::from_millis(960)),
        "{e:?}"
    );
    assert_no_output(&cache);
}

#[test]
fn animation_exactly_at_budget_is_encoded_whole() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("anim.gif");
    write_gif(&src);
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    imp.set_limits(limits(Duration::from_millis(960), 12));
    let Imported::Video(mkv) = run(&imp, &req(&src, 48, 48)).unwrap() else { panic!("video bekleniyordu") };
    // mpv'nin kesmesi (`end`, `frames`) sınırdaki animasyonun karesini atmadı: tüm 12 kare.
    let frames_dir = dir.path().join("frames");
    std::fs::create_dir(&frames_dir).unwrap();
    let status = std::process::Command::new("mpv")
        .args(["--no-config", "--really-quiet", "--ao=null", "--untimed", "--vo=image", "--vo-image-format=png"])
        .arg(format!("--vo-image-outdir={}", frames_dir.display()))
        .arg(&mkv)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_dir(&frames_dir).unwrap().count(), 12);
}

#[test]
fn video_over_duration_budget_is_rejected_before_encoding() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("clip.mkv");
    write_mpeg4(&src, 3);
    let cache = dir.path().join("cache");
    let imp = Importer::in_process_unsandboxed(cache.clone());
    imp.set_limits(limits(Duration::from_secs(2), 1_000_000));
    let t = std::time::Instant::now();
    let e = run(&imp, &req(&src, 320, 180)).unwrap_err();
    assert!(matches!(e, ImportError::Budget(BudgetExceeded::Duration { .. })), "{e:?}");
    assert!(e.to_string().starts_with("içe aktarılamadı, kaynak çok uzun: 2 sn"), "{e}");
    assert!(t.elapsed() < Duration::from_secs(20));
    assert_no_output(&cache);
    // Sınır kaynağın süresini karşılıyorsa kodlanır.
    imp.set_limits(limits(Duration::from_secs(4), 1_000_000));
    assert!(matches!(run(&imp, &req(&src, 320, 180)), Ok(Imported::Video(_))));
}

/// Kap kare hızı bildirmiyor (Matroska): kare sayısı süre × birkaç çözülen kareden tahmin
/// edilen hızla bulunur, aşan video kodlanmaz. (Üst veri hiç yoksa mpv'nin kendi sayacının
/// kesmesi: `hwcaps::tests::endless_source_is_cut_at_frame_budget`.)
#[test]
fn video_over_frame_budget_is_rejected_from_estimated_fps() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("clip.mkv");
    write_mpeg4(&src, 30);
    let cache = dir.path().join("cache");
    let imp = Importer::in_process_unsandboxed(cache.clone());
    imp.set_limits(limits(Duration::from_secs(600), 300));
    let e = run(&imp, &req(&src, 640, 360)).unwrap_err();
    assert!(matches!(e, ImportError::Budget(BudgetExceeded::Frames { limit: 300, actual }) if actual > 300), "{e:?}");
    assert_no_output(&cache);
}
