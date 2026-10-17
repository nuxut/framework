//! Sınır eşitliği (düşmanca): her biçim ve her yol (durağan, animasyon, akış, ham piksel) aynı
//! kapıdan geçer. Dev boyutlu başlık çözülmeden reddedilir; çok kareli animasyonun çözme işi
//! bütçelidir; kare süresi sınırlıdır.

use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};

use image::{ImageEncoder, RgbaImage};
use nuxut_image::{DecodeOptions, Decoded, Fit, ImageError, Limits, LoadRequest, Source, TargetSize, decode_bytes};

fn le16(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}

/// 1×1'lik GIF görüntü bloğu (LZW en küçük kod 2: clear, 0, EOI).
fn gif_image(out: &mut Vec<u8>, w: u16, h: u16, delay_cs: Option<u16>) {
    if let Some(d) = delay_cs {
        out.extend_from_slice(&[0x21, 0xF9, 0x04, 0x00]);
        out.extend_from_slice(&le16(d));
        out.extend_from_slice(&[0x00, 0x00]);
    }
    out.push(0x2C);
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&le16(w));
    out.extend_from_slice(&le16(h));
    out.push(0x00);
    out.extend_from_slice(&[0x02, 0x02, 0x44, 0x01, 0x00]);
}

/// Mantıksal ekran `sw×sh`, iki renkli palet, `frames` adet `fw×fh` kare.
fn gif(sw: u16, sh: u16, frames: usize, fw: u16, fh: u16, delay_cs: Option<u16>) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&le16(sw));
    out.extend_from_slice(&le16(sh));
    out.extend_from_slice(&[0x80, 0x00, 0x00, 0, 0, 0, 255, 255, 255]);
    // Sonsuz döngü (NETSCAPE2.0).
    out.extend_from_slice(&[0x21, 0xFF, 0x0B]);
    out.extend_from_slice(b"NETSCAPE2.0");
    out.extend_from_slice(&[0x03, 0x01, 0x00, 0x00, 0x00]);
    for _ in 0..frames {
        gif_image(&mut out, fw, fh, delay_cs);
    }
    out.push(0x3B);
    out
}

fn jpeg_header(w: u16, h: u16) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08];
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&[0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9]);
    out
}

fn webp_vp8x(w: u32, h: u32, animated: bool) -> Vec<u8> {
    let mut chunk = vec![if animated { 0x02 } else { 0x00 }, 0, 0, 0];
    chunk.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
    chunk.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(4 + 8 + chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WEBPVP8X");
    out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(&chunk);
    out
}

fn qoi_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"qoif".to_vec();
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&[4, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    out
}

/// JPEG XL çıplak kod akışı: SizeHeader (div8 = 0, U32 seçici 3 → 1 + u(30)).
fn jxl_header(w: u32, h: u32) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let put = |v: u32, n: u32, bits: &mut Vec<bool>| (0..n).for_each(|i| bits.push(v >> i & 1 == 1));
    put(0, 1, &mut bits);
    put(3, 2, &mut bits);
    put(h - 1, 30, &mut bits);
    put(0, 3, &mut bits);
    put(3, 2, &mut bits);
    put(w - 1, 30, &mut bits);
    put(1, 1, &mut bits); // ImageMetadata all_default
    let mut out = vec![0xFF, 0x0A];
    for chunk in bits.chunks(8) {
        out.push(chunk.iter().enumerate().fold(0u8, |b, (i, &x)| b | (u8::from(x) << i)));
    }
    out.resize(out.len() + 64, 0);
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = kind.to_vec();
    c.extend_from_slice(data);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

/// IHDR `w×h`, `acTL` ile APNG.
fn apng_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    png_chunk(&mut out, b"acTL", &[0, 0, 0, 2, 0, 0, 0, 0]);
    png_chunk(&mut out, b"IEND", &[]);
    out
}

fn rejected_fast(name: &str, bytes: &[u8], opts: &DecodeOptions) {
    let start = Instant::now();
    let r = decode_bytes(bytes, opts);
    assert!(r.is_err(), "{name}: reddedilmeliydi ({:?})", r.map(|d| d.size()));
    assert!(start.elapsed() < Duration::from_secs(2), "{name}: yavaş red {:?}", start.elapsed());
}

#[test]
fn rejects_giant_headers_in_every_format_and_path() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("gif", gif(65_535, 65_535, 1, 1, 1, None)),
        ("gif-anim", gif(65_535, 65_535, 3, 1, 1, Some(5))),
        ("jpeg", jpeg_header(65_535, 65_535)),
        ("webp", webp_vp8x(1 << 24, 1 << 24, false)),
        ("webp-anim", webp_vp8x(1 << 24, 1 << 24, true)),
        ("qoi", qoi_header(1 << 30, 1 << 30)),
        ("apng", apng_header(100_000, 100_000)),
        ("jxl", jxl_header(1 << 29, 1 << 29)),
    ];
    for animate in [true, false] {
        for limits in [Limits::DEFAULT, Limits::ICON] {
            let opts = DecodeOptions { limits, target: Some(TargetSize::new(64, 64, Fit::Contain)), animate };
            for (name, bytes) in &cases {
                rejected_fast(name, bytes, &opts);
            }
        }
    }
}

#[test]
fn gif_frame_larger_than_canvas_is_rejected() {
    // Ekran 1×1, kare tanımlayıcısı 65535×65535: kare tamponu ekrandan değil kareden ayrılırdı.
    for animate in [true, false] {
        let opts = DecodeOptions { animate, ..Default::default() };
        rejected_fast("gif-frame", &gif(1, 1, 2, 65_535, 65_535, None), &opts);
    }
}

fn encode(format: &str, w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
    let mut out = Vec::new();
    let c = Cursor::new(&mut out);
    let t = image::ExtendedColorType::Rgba8;
    match format {
        "png" => image::codecs::png::PngEncoder::new(c).write_image(img.as_raw(), w, h, t).unwrap(),
        "webp" => image::codecs::webp::WebPEncoder::new_lossless(c).write_image(img.as_raw(), w, h, t).unwrap(),
        "gif" => image::codecs::gif::GifEncoder::new(c).encode(img.as_raw(), w, h, t).unwrap(),
        "qoi" => image::codecs::qoi::QoiEncoder::new(c).write_image(img.as_raw(), w, h, t).unwrap(),
        "jpeg" => {
            let rgb = image::DynamicImage::ImageRgba8(img).into_rgb8();
            image::codecs::jpeg::JpegEncoder::new(c).write_image(rgb.as_raw(), w, h, image::ExtendedColorType::Rgb8).unwrap();
        }
        _ => unreachable!(),
    }
    out
}

#[test]
fn every_format_honours_the_callers_limits() {
    // Geçerli 5000×2 görsel: DEFAULT'ta geçer, ICON'da (en fazla 4096 kenar) her biçimde reddedilir.
    for format in ["png", "jpeg", "webp", "gif", "qoi"] {
        let bytes = encode(format, 5000, 2);
        for animate in [true, false] {
            let ok = DecodeOptions { animate, ..Default::default() };
            assert!(decode_bytes(&bytes, &ok).is_ok(), "{format}: DEFAULT'ta geçmeliydi");
            let icon = DecodeOptions { limits: Limits::ICON, animate, ..Default::default() };
            let r = decode_bytes(&bytes, &icon);
            assert!(matches!(r, Err(ImageError::TooLarge { .. })), "{format}/{animate}: {:?}", r.map(|d| d.size()));
        }
    }
}

#[test]
fn raw_pixel_sources_honour_the_request_limits() {
    let px: Arc<[u8]> = vec![0u8; 5000 * 4].into();
    let mut raw = LoadRequest::raw(px.clone(), 5000, 1, 5000 * 4, true, 64);
    raw.options.limits = Limits::ICON;
    assert!(matches!(raw.run(), Err(ImageError::TooLarge { .. })));
    let argb = LoadRequest {
        source: Source::Argb32 { pixels: px, width: 5000, height: 1, max_side: 64 },
        options: DecodeOptions { limits: Limits::ICON, ..Default::default() },
    };
    assert!(matches!(argb.run(), Err(ImageError::TooLarge { .. })));
}

#[test]
fn eager_animation_decode_work_is_budgeted() {
    // 512×512 tuval, 400 küçük kare, küçük hedef: bellek bütçesi hiç dolmaz ama her kare tam
    // tuval birleştirilir (400 × 262k px). İş bütçesi 10 kare: sonrası akışa geçer.
    let bytes = gif(512, 512, 400, 1, 1, Some(5));
    let limits = Limits { max_animation_pixels: 512 * 512 * 10, max_animation_bytes: u64::MAX, ..Limits::DEFAULT };
    let opts = DecodeOptions { limits: limits.with_trust(nuxut_image::Trust::System), target: Some(TargetSize::new(16, 16, Fit::Contain)), animate: true };
    let r = decode_bytes(&bytes, &opts).unwrap();
    assert!(matches!(r, Decoded::Streamed(_)), "akış bekleniyordu");
}

#[test]
fn giant_frame_delays_are_clamped() {
    // 65535 cs ≈ 11 dk: oynatıcı zamanlayıcısı bu kadar uzağa kurulmaz.
    let Decoded::Animated(a) = decode_bytes(&gif(4, 4, 2, 1, 1, Some(65_535)), &DecodeOptions::default().with_trust(nuxut_image::Trust::System)).unwrap() else {
        panic!("animasyon bekleniyordu")
    };
    assert!(a.frames.iter().all(|f| f.delay <= nuxut_image::MAX_FRAME_DELAY), "{:?}", a.frames[0].delay);
}

/// `is_animated` (duvar kağıdı içe aktarma) aynı kapıdan geçer ve hiç piksel çözmez: dev tuvalli
/// GIF/APNG/WebP başlığı hızla `TooLarge`, sınırı aşan bayt dizisi `FileTooLarge`. Eski hâli iki
/// GIF karesini tuval boyutunda, sınırsız çözüyordu (16000×16000 tuval: kare başına ~1 GB).
#[test]
fn is_animated_goes_through_the_gate() {
    use nuxut_image::is_animated;
    let giant: Vec<(&str, Vec<u8>)> = vec![
        ("gif", gif(16_000, 16_000, 2, 1, 1, Some(5))),
        ("gif-u16", gif(u16::MAX, u16::MAX, 3, 1, 1, Some(5))),
        ("apng", apng_header(100_000, 100_000)),
        ("webp", webp_vp8x(100_000, 100_000, true)),
    ];
    for limits in [Limits::DEFAULT, Limits::ICON] {
        for (name, bytes) in &giant {
            let start = Instant::now();
            let r = is_animated(bytes, &limits);
            assert!(matches!(r, Err(ImageError::TooLarge { .. })), "{name}: {r:?}");
            assert!(start.elapsed() < Duration::from_millis(200), "{name}: yavaş {:?}", start.elapsed());
        }
    }
    let small = Limits { max_file_bytes: 64, ..Limits::DEFAULT };
    let r = is_animated(&gif(8, 8, 20, 1, 1, Some(5)), &small);
    assert!(matches!(r, Err(ImageError::FileTooLarge { .. })), "{r:?}");
}

#[test]
fn is_animated_counts_gif_frames_without_decoding() {
    use nuxut_image::is_animated;
    let l = Limits::DEFAULT;
    assert!(!is_animated(&gif(8, 8, 1, 1, 1, None), &l).unwrap());
    assert!(is_animated(&gif(8, 8, 2, 1, 1, Some(5)), &l).unwrap());
    // Çok kareli dosya: ilk ikiden sonra durur (süre dosya boyundan bağımsız küçük).
    let many = gif(8, 8, 200_000, 1, 1, Some(1));
    let start = Instant::now();
    assert!(is_animated(&many, &l).unwrap());
    assert!(start.elapsed() < Duration::from_millis(50), "{:?}", start.elapsed());
    assert!(is_animated(&apng_header(64, 64), &l).unwrap());
    assert!(is_animated(&webp_vp8x(64, 64, true), &l).unwrap());
    assert!(!is_animated(&webp_vp8x(64, 64, false), &l).unwrap());
    // Kesik ve yalancı alt blok uzunlukları: panik yok, taşma yok; sayılabilen kadar.
    let full = gif(8, 8, 2, 1, 1, Some(5));
    for cut in 0..full.len() {
        let _ = is_animated(&full[..cut], &l);
    }
    let mut lying = gif(8, 8, 1, 1, 1, None);
    let at = lying.len() - 2;
    lying[at] = 0xFF; // son alt blok 255 bayt diyor, dosya bitiyor
    assert!(!is_animated(&lying, &l).unwrap());
    // Görsel olmayan bayt dizisi.
    assert!(!is_animated(b"not an image at all", &l).unwrap());
}

/// `animation_info` (duvar kağıdı içe aktarma bütçesi): kare sayısı ve süre yalnızca kap
/// yapısından, oynatıcının gecikme kuralıyla (≤ 10 ms → 100 ms) ama üst sınırsız.
#[test]
fn animation_info_counts_frames_and_duration_from_structure() {
    use nuxut_image::{AnimationInfo, animation_info};
    let info = |frames: u64, ms: u64| Some(AnimationInfo { frames, duration: Duration::from_millis(ms) });
    // GIF: 5 × 70 cs; gecikmesiz kare 100 ms sayılır; dev gecikme kırpılmaz (65 535 cs).
    assert_eq!(animation_info(&gif(8, 8, 5, 1, 1, Some(70))), info(5, 3_500));
    assert_eq!(animation_info(&gif(8, 8, 3, 1, 1, None)), info(3, 300));
    assert_eq!(animation_info(&gif(8, 8, 2, 1, 1, Some(u16::MAX))), info(2, 1_310_700));
    // Tek kare ve durağan biçim animasyon değil.
    assert_eq!(animation_info(&gif(8, 8, 1, 1, 1, Some(5))), None);
    assert_eq!(animation_info(&jpeg_header(8, 8)), None);
    // Çok kareli GIF sayısı sınırsız (ayırma yok).
    assert_eq!(animation_info(&gif(8, 8, 50_000, 1, 1, Some(2))).map(|i| i.frames), Some(50_000));

    // APNG: fcTL başına bir kare; gecikme num/den sn (payda 0 → 100).
    let mut apng = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = 8u32.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&8u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut apng, b"IHDR", &ihdr);
    png_chunk(&mut apng, b"acTL", &[0, 0, 0, 3, 0, 0, 0, 0]);
    for (seq, (num, den)) in [(1u16, 4u16), (25, 0), (0, 1)].into_iter().enumerate() {
        let mut fctl = (seq as u32).to_be_bytes().to_vec();
        fctl.extend_from_slice(&[0; 16]);
        fctl.extend_from_slice(&num.to_be_bytes());
        fctl.extend_from_slice(&den.to_be_bytes());
        fctl.extend_from_slice(&[0, 0]);
        png_chunk(&mut apng, b"fcTL", &fctl);
    }
    png_chunk(&mut apng, b"IEND", &[]);
    // 250 + 250 + 100 (0 ms → 100).
    assert_eq!(animation_info(&apng), info(3, 600));
    // acTL'siz PNG animasyon değil.
    assert_eq!(animation_info(&encode("png", 4, 4)), None);

    // WebP: ANMF başına bir kare, 24 bit ms süre; tek bayt doldurmalı parça atlanır.
    let mut webp = webp_vp8x(8, 8, true);
    let mut push = |kind: &[u8; 4], data: &[u8]| {
        webp.extend_from_slice(kind);
        webp.extend_from_slice(&(data.len() as u32).to_le_bytes());
        webp.extend_from_slice(data);
        if data.len() % 2 == 1 {
            webp.push(0);
        }
    };
    push(b"ANIM", &[0; 6]);
    push(b"ICCP", &[0; 3]);
    for ms in [40u32, 5, 0x00FF_FFFF] {
        let mut anmf = vec![0u8; 12];
        anmf.extend_from_slice(&ms.to_le_bytes()[..3]);
        anmf.push(0);
        push(b"ANMF", &anmf);
    }
    assert_eq!(animation_info(&webp), info(3, 40 + 100 + 0x00FF_FFFF));
    assert_eq!(animation_info(&webp_vp8x(8, 8, false)), None);
}
