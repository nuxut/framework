//! İçe aktarma hattı uçtan uca. libmpv yoksa video testleri atlanır.

use std::path::Path;
use nuxut_image::CancelFlag;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, ImageEncoder, RgbaImage};
use nuxut_video::{ImportError, ImportRequest, Imported, Importer};

fn req(source: &Path, w: u32, h: u32) -> ImportRequest {
    ImportRequest { source: source.to_owned(), target_px: (w, h) }
}

fn run(imp: &Importer, r: &ImportRequest) -> Result<Imported, ImportError> {
    imp.run(r, &CancelFlag::new(), &|_| {})
}

fn write_png(path: &Path, w: u32, h: u32) {
    let img = RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255]));
    image::codecs::png::PngEncoder::new(std::fs::File::create(path).unwrap())
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
}

#[test]
fn still_image_becomes_cached_qoi() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("wall.png");
    write_png(&src, 800, 400);
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    let r = req(&src, 200, 200);
    let Imported::Image(qoi) = run(&imp, &r).unwrap() else { panic!("görsel bekleniyordu") };
    assert_eq!(qoi.extension().unwrap(), "qoi");
    assert!(qoi.starts_with(dir.path().join("cache")));
    // Cover: kısa kenar hedefi karşılar.
    let img = nuxut_image::decode_file(&qoi, &Default::default()).unwrap().into_first_frame();
    assert_eq!(img.size(), (400, 200));
    // Kaynak değişmedi; ikinci kez anında önbellekten.
    assert_eq!(imp.lookup(&r), Some(Imported::Image(qoi.clone())));
    use std::os::unix::fs::MetadataExt;
    let inode = std::fs::metadata(&qoi).unwrap().ino();
    assert_eq!(run(&imp, &r).unwrap(), Imported::Image(qoi.clone()));
    assert_eq!(std::fs::metadata(&qoi).unwrap().ino(), inode, "yeniden kodlanmamalı");
    // Aynı içerik başka adla: aynı önbellek girdisi (içerik hash'i).
    let copy = dir.path().join("kopya.png");
    std::fs::copy(&src, &copy).unwrap();
    assert_eq!(run(&imp, &req(&copy, 200, 200)).unwrap(), Imported::Image(qoi));
    // Kaynağa dokunulmadı.
    assert!(src.is_file());
}

#[test]
fn garbage_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("x.bin");
    std::fs::write(&src, b"bu bir video ya da gorsel degil").unwrap();
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    assert!(run(&imp, &req(&src, 100, 100)).is_err());
}

fn mpv() -> bool {
    let ok = nuxut_video::available();
    if !ok {
        eprintln!("libmpv yok; test atlandı");
    }
    ok
}

fn write_gif(path: &Path) {
    let mut enc = GifEncoder::new(std::fs::File::create(path).unwrap());
    enc.set_repeat(Repeat::Infinite).unwrap();
    for i in 0..12u32 {
        let img = RgbaImage::from_fn(96, 64, |x, _| image::Rgba([((x + i * 8) % 96 * 2) as u8, 80, 200, 255]));
        enc.encode_frame(image::Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(80, 1))).unwrap();
    }
}

/// mpv kodlama kipiyle test videosu (h264, 640x360, 2 sn, sesli).
fn write_video(path: &Path) {
    let status = std::process::Command::new("mpv")
        .args([
            "--no-config",
            "--really-quiet",
            "av://lavfi:testsrc2=size=640x360:rate=24:duration=2",
            "--audio-file=av://lavfi:sine=frequency=440:duration=2",
            "--ovc=libx264",
            "--oac=libopus",
            "--of=matroska",
        ])
        .arg(format!("--o={}", path.display()))
        .status();
    assert!(status.is_ok_and(|s| s.success()) && path.is_file(), "test videosu üretilemedi");
}

fn probe(path: &Path) -> (String, String) {
    let out = std::process::Command::new("mpv")
        .args(["--no-config", "--vo=null", "--ao=null", "--frames=1", "--term-playing-msg=PROBE ${video-format} ${width}x${height}"])
        .arg(path)
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().find(|l| l.starts_with("PROBE ")).unwrap_or_default();
    let mut it = line.split(' ').skip(1);
    (it.next().unwrap_or_default().to_owned(), it.next().unwrap_or_default().to_owned())
}

#[test]
fn animated_gif_becomes_looping_video() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("anim.gif");
    write_gif(&src);
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    let progress = std::sync::Mutex::new(Vec::new());
    let r = imp.run(&req(&src, 48, 48), &CancelFlag::new(), &|p| progress.lock().unwrap().push(p)).unwrap();
    let Imported::Video(mkv) = r else { panic!("video bekleniyordu: {r:?}") };
    assert_eq!(mkv.extension().unwrap(), "mkv");
    let (codec, size) = probe(&mkv);
    assert_eq!(codec, "h264");
    assert_eq!(size, "72x48", "cover küçültme, çift kenar");
    let p = progress.into_inner().unwrap();
    assert!(p.last() == Some(&1.0) && p.windows(2).all(|w| w[0] <= w[1] + 1e-6), "{p:?}");
}

#[test]
fn video_is_reencoded_only_when_needed() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("clip.mkv");
    write_video(&src);
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    // Hedef kaynaktan büyük ve codec ucuz (h264): kaynak olduğu gibi.
    assert_eq!(run(&imp, &req(&src, 1280, 720)).unwrap(), Imported::Video(src.clone()));
    assert_eq!(imp.lookup(&req(&src, 1280, 720)), Some(Imported::Video(src.clone())));
    // Kaynak hedeften çok büyük: küçültülür, ses korunur.
    let Imported::Video(small) = run(&imp, &req(&src, 320, 180)).unwrap() else { panic!() };
    assert_ne!(small, src);
    let (codec, size) = probe(&small);
    assert_eq!((codec.as_str(), size.as_str()), ("h264", "320x180"));
}

#[test]
fn cancellation_leaves_no_files() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("anim.gif");
    write_gif(&src);
    let cache = dir.path().join("cache");
    let imp = Importer::in_process_unsandboxed(cache.clone());
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert!(matches!(imp.run(&req(&src, 48, 48), &cancel, &|_| {}), Err(ImportError::Cancelled)));
    let leftovers: Vec<_> = std::fs::read_dir(&cache)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name()).filter(|n| n.to_string_lossy().contains(".tmp")).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn capabilities_are_probed_and_cached() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let imp = Importer::in_process_unsandboxed(dir.path().to_owned());
    let caps = imp.capabilities().clone();
    assert!(!caps.key.is_empty());
    assert!(dir.path().join("hwcaps.toml").is_file());
    // Hedef codec her zaman ucuz çözülen bir codec.
    let t = caps.target_codec();
    assert!(caps.hardware(t) || t == nuxut_video::Codec::H264);
    // İkinci içe aktarıcı yeniden yoklamaz, aynı sonucu okur.
    let again = Importer::in_process_unsandboxed(dir.path().to_owned());
    assert_eq!(again.capabilities(), &caps);
}

#[test]
fn still_frame_fallback_and_lru() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("clip.mkv");
    write_video(&src);
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    let Imported::Image(frame) = imp.still_frame(&req(&src, 320, 180), 1.0).unwrap() else { panic!() };
    let img = nuxut_image::decode_file(&frame, &Default::default()).unwrap().into_first_frame();
    assert_eq!(img.size(), (320, 180));
    // LRU: sınır en yeni girdiyi tutacak kadar küçük → eski kare silinir, yenisi kalır.
    imp.set_cache_limit(1);
    let png = dir.path().join("a.png");
    write_png(&png, 64, 32);
    let Imported::Image(newest) = run(&imp, &req(&png, 32, 32)).unwrap() else { panic!() };
    assert!(newest.is_file());
    assert!(!frame.is_file(), "eski girdi silinmeliydi");
}

#[test]
fn debug_and_errors_hide_paths() {
    let r = req(Path::new("/home/kişi/tatil.mp4"), 1920, 1080);
    let e = ImportError::Io { path: "/home/kişi/tatil.mp4".into(), source: std::io::ErrorKind::NotFound.into() };
    for s in [format!("{r:?}"), format!("{:?}", Imported::Video(r.source.clone())), format!("{e:?}"), e.to_string()] {
        assert!(!s.contains("kişi") && !s.contains("tatil"), "{s}");
    }
}
