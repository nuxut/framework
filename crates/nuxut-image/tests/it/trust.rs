//! Güven kapısı (düşmanca): `Untrusted` kaynaktaki SVG süreç içinde hiç ayrıştırılmaz. Biçim
//! içerikten tespit edilir (uzantı yalanı işe yaramaz); svgz, BOM/boşluk, uzun önsöz,
//! `data:` URI ve DOCTYPE kaçamakları SVG ayrıştırıcısına ulaşamaz. `System` yolu değişmez.

use std::sync::Arc;

use image::ImageEncoder;
use nuxut_image::{DecodeOptions, IconTheme, ImageError, Limits, LoadRequest, Trust, decode_bytes, decode_file};

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f00"/></svg>"##;

fn untrusted() -> DecodeOptions {
    DecodeOptions::default().with_trust(Trust::Untrusted)
}

fn system() -> DecodeOptions {
    DecodeOptions::default().with_trust(Trust::System)
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]));
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), 4, 4, image::ExtendedColorType::Rgba8).unwrap();
    out
}

/// SVG ayrıştırıcısına girilmediğinin kanıtı: hata hiçbir zaman `Svg(_)` değil.
fn assert_not_parsed(name: &str, r: Result<nuxut_image::Decoded, ImageError>) {
    match r {
        Err(ImageError::Unsupported(_) | ImageError::UnknownFormat) => {}
        other => panic!("{name}: SVG ayrıştırıcısına ulaşmamalıydı: {:?}", other.map(|d| d.size())),
    }
}

#[test]
fn untrusted_svg_is_never_parsed_but_system_is() {
    assert!(matches!(decode_bytes(SVG.as_bytes(), &untrusted()), Err(ImageError::Unsupported("SVG"))));
    assert_eq!(decode_bytes(SVG.as_bytes(), &system()).unwrap().size(), (8, 8));
    // Raster biçimler Untrusted'ta da çözülür.
    assert_eq!(decode_bytes(&png(), &untrusted()).unwrap().size(), (4, 4));
}

#[test]
fn content_not_extension_decides() {
    let dir = tempfile::tempdir().unwrap();
    let lie = dir.path().join("ikon.png");
    std::fs::write(&lie, SVG).unwrap();
    assert!(matches!(decode_file(&lie, &untrusted()), Err(ImageError::Unsupported("SVG"))));
    // .svg adlı PNG: içerik PNG, çözülür.
    let png_named_svg = dir.path().join("ikon.svg");
    std::fs::write(&png_named_svg, png()).unwrap();
    assert_eq!(decode_file(&png_named_svg, &untrusted()).unwrap().size(), (4, 4));
    // .svg adlı çöp: Untrusted'ta ayrıştırıcıya gitmez.
    let junk = dir.path().join("cop.svg");
    std::fs::write(&junk, b"rect width=8 height=8").unwrap();
    assert_not_parsed("çöp.svg", decode_file(&junk, &untrusted()));
}

#[test]
fn svg_evasions_are_rejected() {
    let long_prolog = format!("<?xml version=\"1.0\"?>\n<!--{}-->\n{SVG}", "x".repeat(8192));
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("bom+boşluk", [&[0xEF, 0xBB, 0xBF][..], b"  \n\t", SVG.as_bytes()].concat()),
        ("uzun önsöz", long_prolog.into_bytes()),
        ("doctype", format!("<!DOCTYPE svg>{SVG}").into_bytes()),
        // gzip başlığı (svgz).
        ("svgz", vec![0x1F, 0x8B, 0x08, 0x00, 0, 0, 0, 0, 0, 0x03, 0x01, 0x02]),
        ("data uri", b"data:image/svg+xml;base64,PHN2ZyB3aWR0aD0iOCIgaGVpZ2h0PSI4Ii8+".to_vec()),
        ("data uri düz", format!("data:image/svg+xml,{SVG}").into_bytes()),
    ];
    for (name, bytes) in cases {
        assert_not_parsed(name, decode_bytes(&bytes, &untrusted()));
    }
}

/// Güven belirtmeyen her çağrı `Untrusted`: varsayılanlar, sabit sınırlar, `LoadRequest::icon`.
#[test]
fn unspecified_trust_is_untrusted() {
    assert_eq!(Trust::default(), Trust::Untrusted);
    assert_eq!(DecodeOptions::default().trust(), Trust::Untrusted);
    assert_eq!(Limits::DEFAULT.trust, Trust::Untrusted);
    assert_eq!(Limits::ICON.trust, Trust::Untrusted);
    // Shell'in kurduğu biçim: `..Limits::ICON` güvenli varsayılanı taşır.
    let opts = DecodeOptions { limits: Limits { max_file_bytes: 1 << 20, ..Limits::ICON }, target: None, animate: false };
    assert!(matches!(decode_bytes(SVG.as_bytes(), &opts), Err(ImageError::Unsupported("SVG"))));
    let theme = Arc::new(IconTheme::with_dirs("yok", vec![], vec![]));
    assert_eq!(LoadRequest::icon(theme, "x", 8, 1).options.trust(), Trust::Untrusted);
}

/// Uygulamanın verdiği `IconThemePath` (kullanıcının yazabildiği dizin) arama dizinine konsa
/// bile içindeki SVG hiçbir istekte çözülmez; ad ya da yol olarak sorulsun fark etmez.
#[test]
fn malicious_icon_theme_path_svg_is_unsupported() {
    let app = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(app.path().join("hicolor/scalable/apps")).unwrap();
    std::fs::write(app.path().join("hicolor/index.theme"), "[Icon Theme]\nDirectories=scalable/apps\n[scalable/apps]\nSize=48\nType=Scalable\nMinSize=8\nMaxSize=512\n").unwrap();
    let icon = app.path().join("hicolor/scalable/apps/kotu.svg");
    std::fs::write(&icon, SVG).unwrap();
    std::fs::write(app.path().join("duz.svg"), SVG).unwrap();
    std::fs::write(app.path().join("raster.png"), png()).unwrap();
    let theme = Arc::new(IconTheme::with_dirs("hicolor", vec![app.path().to_path_buf()], vec![app.path().to_path_buf()]));
    assert_eq!(theme.lookup("kotu", 8, 1).as_deref(), Some(icon.as_path()), "dosya bulunuyor");
    for trust in [Trust::System, Trust::Untrusted] {
        for name in ["kotu".to_owned(), "duz".to_owned(), icon.display().to_string(), format!("file://{}", icon.display())] {
            assert!(matches!(theme.load_as(&name, 8, 1, trust), Err(ImageError::Unsupported("SVG"))), "{name} {trust:?}");
            let req = LoadRequest::icon(theme.clone(), name.clone(), 8, 1).with_trust(trust);
            assert!(matches!(req.run(), Err(ImageError::Unsupported("SVG"))), "{name} {trust:?}");
        }
    }
    // Raster ikon yine çözülür.
    assert_eq!(theme.load_as("raster", 4, 1, Trust::Untrusted).unwrap().size(), (4, 4));
}

/// Sistem temasındaki gerçek bir SVG: `System` istekte çözülür, `Untrusted` istekte asla.
#[test]
fn system_theme_svg_needs_system_request() {
    let sys = std::path::PathBuf::from("/usr/share/icons");
    let theme = IconTheme::with_dirs("hicolor", vec![sys], vec![]);
    let Some(name) = std::fs::read_dir("/usr/share/icons/hicolor/scalable/apps")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .find_map(|f| f.strip_suffix(".svg").map(str::to_owned))
    else {
        eprintln!("sistemde hicolor SVG yok; atlandı");
        return;
    };
    assert!(theme.load_as(&name, 16, 1, Trust::System).is_ok(), "{name}");
    assert!(matches!(theme.load_as(&name, 16, 1, Trust::Untrusted), Err(ImageError::Unsupported("SVG"))));
    #[allow(deprecated)]
    let old = theme.load(&name, 16, 1);
    assert!(matches!(old, Err(ImageError::Unsupported("SVG"))), "eski API güvenli varsayılanla");
    // Aynı dosya yol olarak: System istekte bile Untrusted (yolu biz seçmedik).
    let path = theme.lookup(&name, 16, 1).unwrap();
    assert!(matches!(theme.load_as(&path.display().to_string(), 16, 1, Trust::System), Err(ImageError::Unsupported("SVG"))));
}

#[test]
fn hostile_names_are_rejected() {
    let theme = IconTheme::with_dirs("hicolor", vec!["/usr/share/icons".into()], vec!["/usr/share/pixmaps".into()]);
    for name in ["../../../etc/passwd", "hicolor/../x", "a\0b", "..", ".hidden", "a/b"] {
        assert_eq!(theme.lookup(name, 16, 1), None, "{name:?}");
        assert!(matches!(theme.load_as(name, 16, 1, Trust::System), Err(ImageError::NotFound(_))), "{name:?}");
    }
}

/// Kullanıcının yazabildiği tema dizinine konmuş FIFO ve dizin: açılış ve okuma bloklamaz,
/// normal dosya olmadığı için reddedilir (ad ya da yol olarak, her güvenle).
#[cfg(unix)]
#[test]
fn fifo_icons_do_not_block() {
    use rustix::fs::{CWD, FileType, Mode, mknodat};
    let dir = tempfile::tempdir().unwrap();
    let apps = dir.path().join("hicolor/48x48/apps");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(dir.path().join("hicolor/index.theme"), "[Icon Theme]\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\n").unwrap();
    let fifo = apps.join("boru.png");
    mknodat(CWD, &fifo, FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
    std::fs::create_dir(apps.join("dizin.png")).unwrap();
    let theme = IconTheme::with_dirs("hicolor", vec![dir.path().to_path_buf()], vec![]);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        for trust in [Trust::System, Trust::Untrusted] {
            for name in ["boru", "dizin"] {
                out.push(matches!(theme.load_as(name, 48, 1, trust), Err(ImageError::Io { .. })));
            }
        }
        out.push(matches!(decode_file(&fifo, &untrusted()), Err(ImageError::Io { .. })));
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("FIFO thread'i blokladı");
    assert!(out.iter().all(|ok| *ok), "{out:?}");
}

/// Bulgu 4a: sandbox'sız süreç içi yolda güvenilmeyen JXL/AVIF hiç ayrıştırılmaz; yalnızca
/// temel raster (PNG/JPEG/WebP/GIF/QOI) geçer.
#[test]
fn untrusted_in_process_path_accepts_only_basic_raster() {
    // JXL kod akışı imzası + çöp; AVIF `ftyp avif` kutusu + çöp.
    let mut jxl = vec![0xFF, 0x0A];
    jxl.extend(std::iter::repeat_n(0xAB, 64));
    let mut avif = b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf".to_vec();
    avif.extend(std::iter::repeat_n(0, 64));
    for (name, bytes) in [("jxl", &jxl), ("avif", &avif)] {
        assert!(matches!(decode_bytes(bytes, &untrusted()), Err(ImageError::Unsupported(_))), "{name}");
        let r = LoadRequest { source: nuxut_image::Source::Bytes(Arc::from(bytes.as_slice())), options: untrusted() }.run();
        assert!(matches!(r, Err(ImageError::Unsupported(_))), "{name} (Loader, sandbox'sız)");
        // System yolu ayrıştırmayı dener (bozuk veri → çözme hatası, Unsupported değil).
        assert!(!matches!(decode_bytes(bytes, &system()), Err(ImageError::Unsupported(_))), "{name} System");
    }
    assert!(decode_bytes(&png(), &untrusted()).is_ok());
}

fn animated_gif() -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        for c in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
            let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(c));
            enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(50, 1))).unwrap();
        }
    }
    out
}

/// Bulgu 4b (K5): güvenilmeyen kaynakta süreç içinde de yalnızca ilk kare.
#[test]
fn untrusted_animation_decodes_first_frame_only() {
    let gif = animated_gif();
    let animate = DecodeOptions { animate: true, ..untrusted() };
    match decode_bytes(&gif, &animate).unwrap() {
        nuxut_image::Decoded::Still(i) => assert_eq!(i.straight_pixel(0, 0), [255, 0, 0, 255]),
        _ => panic!("güvenilmeyen animasyon tek kare olmalı"),
    }
    assert!(matches!(decode_bytes(&gif, &DecodeOptions { animate: true, ..system() }).unwrap(), nuxut_image::Decoded::Animated(_)));
}

/// Bulgu 2c: işçi kipi dışında `decode_isolated` hiçbir baytı ayrıştırmaz (bu test ikilisi
/// işçi kipine hiç girmez).
#[test]
fn decode_isolated_refuses_outside_worker_mode() {
    assert!(!nuxut_image::in_worker_mode());
    for bytes in [SVG.as_bytes().to_vec(), png()] {
        assert!(matches!(nuxut_image::decode_isolated(&bytes, &system()), Err(ImageError::Unsupported(_))));
    }
}
