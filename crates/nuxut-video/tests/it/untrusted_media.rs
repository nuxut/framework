//! Güvenilmeyen medya (düşmanca): mpv yalnızca güvenli açılmış yerel normal dosyayı okur.
//! Ağ adresi, sembolik bağ, FIFO ve iç başvurular (playlist, EDL, HLS, ffconcat) reddedilir;
//! hiçbiri ağa (yerel dinleyiciye) bağlanmaz ya da başka bir dosyayı açmaz.

use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::Path;
use nuxut_image::CancelFlag;

use nuxut_video::{ImportError, ImportRequest, Imported, Importer, SourceError, VideoError, VideoPlayer};

fn run(imp: &Importer, source: &Path) -> Result<Imported, ImportError> {
    imp.run(&ImportRequest { source: source.to_owned(), target_px: (320, 180) }, &CancelFlag::new(), &|_| {})
}

fn mpv() -> bool {
    let ok = nuxut_video::available();
    if !ok {
        eprintln!("libmpv yok; test atlandı");
    }
    ok
}

/// Bağlantı gelirse yakalayan yerel dinleyici.
fn listener() -> (TcpListener, u16) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.set_nonblocking(true).unwrap();
    let port = l.local_addr().unwrap().port();
    (l, port)
}

fn assert_no_connection(l: &TcpListener) {
    std::thread::sleep(std::time::Duration::from_millis(200));
    match l.accept() {
        Err(e) if e.kind() == ErrorKind::WouldBlock => {}
        Ok(_) => panic!("ağa bağlanıldı"),
        Err(e) => panic!("{e}"),
    }
}

fn write_video(path: &Path) {
    let status = std::process::Command::new("mpv")
        .args(["--no-config", "--really-quiet", "av://lavfi:testsrc2=size=320x180:rate=12:duration=1", "--ovc=libx264", "--of=matroska"])
        .arg(format!("--o={}", path.display()))
        .status();
    assert!(status.is_ok_and(|s| s.success()) && path.is_file(), "test videosu üretilemedi");
}

#[test]
fn rejects_network_urls_as_source() {
    let (l, port) = listener();
    let imp = Importer::in_process_unsandboxed(tempfile::tempdir().unwrap().path().join("cache"));
    for url in [
        format!("http://127.0.0.1:{port}/a.mkv"),
        format!("rtsp://127.0.0.1:{port}/a"),
        format!("ytdl://127.0.0.1:{port}/a"),
        "av://lavfi:testsrc".to_owned(),
        format!("edl://http://127.0.0.1:{port}/a.mkv"),
    ] {
        let r = run(&imp, Path::new(&url));
        assert!(matches!(r, Err(ImportError::Source(SourceError::NotLocal))), "{url}: {r:?}");
    }
    assert_no_connection(&l);
}

#[test]
fn rejects_symlinks_and_special_files() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("gercek.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])).save(&real).unwrap();
    let link = dir.path().join("bag.png");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    assert!(matches!(run(&imp, &link), Err(ImportError::Source(SourceError::Symlink))));
    // Dizin ve FIFO normal dosya değil (FIFO açılırken asılı kalmaz).
    assert!(matches!(run(&imp, dir.path()), Err(ImportError::Source(SourceError::NotRegular))));
    let fifo = dir.path().join("boru");
    rustix::fs::mknodat(rustix::fs::CWD, &fifo, rustix::fs::FileType::Fifo, rustix::fs::Mode::from_raw_mode(0o600), 0).unwrap();
    assert!(matches!(run(&imp, &fifo), Err(ImportError::Source(SourceError::NotRegular))));
    // Gerçek dosya kabul edilir.
    assert!(run(&imp, &real).is_ok());
}

#[test]
fn internal_references_are_not_followed() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mkv");
    write_video(&clip);
    let (l, port) = listener();
    let url = format!("http://127.0.0.1:{port}/a.mkv");
    let cases = [
        ("liste.m3u", format!("#EXTM3U\n{url}\n{}\n", clip.display())),
        ("liste.pls", format!("[playlist]\nFile1={url}\nFile2={}\nNumberOfEntries=2\n", clip.display())),
        ("kurgu.edl", format!("# mpv EDL v0\n{url}\n{}\n", clip.display())),
        ("akis.m3u8", format!("#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\n{url}\n#EXTINF:1,\n{}\n#EXT-X-ENDLIST\n", clip.display())),
        ("birlesim.ffconcat", format!("ffconcat version 1.0\nfile '{url}'\nfile '{}'\n", clip.display())),
        // Yalnızca yerel başvuru da izlenmez (başka dosya açılmaz).
        ("yerel.ffconcat", format!("ffconcat version 1.0\nfile '{}'\n", clip.display())),
    ];
    let imp = Importer::in_process_unsandboxed(dir.path().join("cache"));
    for (name, body) in cases {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        let r = run(&imp, &p);
        assert!(r.is_err(), "{name} kabul edilmemeliydi: {r:?}");
    }
    assert_no_connection(&l);
    // Gerçek video hâlâ içe aktarılır.
    assert!(run(&imp, &clip).is_ok());
}

#[test]
fn player_loads_only_safe_local_files() {
    if !mpv() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("clip.mkv");
    std::fs::write(&clip, b"x").unwrap();
    let link = dir.path().join("bag.mkv");
    std::os::unix::fs::symlink(&clip, &link).unwrap();
    let mut p = VideoPlayer::new(|| {}).unwrap();
    assert!(matches!(p.load(&link), Err(VideoError::Source(SourceError::Symlink))));
    assert!(matches!(p.load(Path::new("http://127.0.0.1:1/a.mkv")), Err(VideoError::Source(SourceError::NotLocal))));
    assert!(p.load(&clip).is_ok());
}
