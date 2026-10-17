//! Çözücü sınırları ve biçimler: dekompresyon bombası, animasyon, küçültme.

use std::io::Cursor;
use std::time::Duration;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, ImageEncoder, RgbaImage};
use nuxut_image::{DecodeOptions, Decoded, Fit, ImageError, Limits, TargetSize, Trust, decode_bytes, decode_file};

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .unwrap();
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

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut c = kind.to_vec();
    c.extend_from_slice(data);
    out.extend_from_slice(&c);
    out.extend_from_slice(&crc32(&c).to_be_bytes());
}

/// Başlığı 100000×100000 diyen, gövdesi birkaç bayt olan PNG (≈ 40 GB RGBA isteği).
fn png_bomb() -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&100_000u32.to_be_bytes());
    ihdr.extend_from_slice(&100_000u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01]);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn rejects_dimension_bomb_without_allocating() {
    let err = decode_bytes(&png_bomb(), &DecodeOptions::default()).unwrap_err();
    assert!(matches!(err, ImageError::TooLarge { width: 100_000, height: 100_000 }), "{err}");
}

#[test]
fn enforces_pixel_and_file_limits() {
    let bytes = png(100, 100, [1, 2, 3, 255]);
    let tight = DecodeOptions { limits: Limits { max_pixels: 9_999, ..Limits::DEFAULT }, ..Default::default() };
    assert!(matches!(decode_bytes(&bytes, &tight), Err(ImageError::TooLarge { .. })));
    let tiny_file = DecodeOptions { limits: Limits { max_file_bytes: 10, ..Limits::DEFAULT }, ..Default::default() };
    assert!(matches!(decode_bytes(&bytes, &tiny_file), Err(ImageError::FileTooLarge { .. })));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.png");
    std::fs::write(&path, &bytes).unwrap();
    assert!(matches!(decode_file(&path, &tiny_file), Err(ImageError::FileTooLarge { .. })));
    assert_eq!(decode_file(&path, &DecodeOptions::default()).unwrap().size(), (100, 100));
}

#[test]
fn rejects_garbage_and_truncated() {
    assert!(matches!(decode_bytes(b"hello world", &DecodeOptions::default()), Err(ImageError::UnknownFormat)));
    let bytes = png(64, 64, [9, 9, 9, 255]);
    assert!(decode_bytes(&bytes[..bytes.len() / 2], &DecodeOptions::default()).is_err());
}

#[test]
fn downscales_to_target_and_premultiplies() {
    let bytes = png(400, 200, [255, 0, 0, 128]);
    let opts = DecodeOptions { target: Some(TargetSize::new(100, 100, Fit::Cover)), ..Default::default() };
    let img = decode_bytes(&bytes, &opts).unwrap().into_first_frame();
    assert_eq!(img.size(), (200, 100));
    assert_eq!(&img.pixels()[..4], &[128, 0, 0, 128]);
}

fn gif(frames: usize, delay_ms: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = GifEncoder::new(&mut out);
        enc.set_repeat(Repeat::Infinite).unwrap();
        for i in 0..frames {
            let v = (i * 60) as u8;
            let img = RgbaImage::from_pixel(32, 16, image::Rgba([v, 255 - v, 0, 255]));
            let frame = image::Frame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(delay_ms, 1));
            enc.encode_frame(frame).unwrap();
        }
    }
    out
}

#[test]
fn decodes_animated_gif_with_timing() {
    let opts = DecodeOptions { target: Some(TargetSize::new(16, 16, Fit::Contain)), ..Default::default() }.with_trust(Trust::System);
    let Decoded::Animated(a) = decode_bytes(&gif(3, 70), &opts).unwrap() else { panic!("animasyon bekleniyordu") };
    assert_eq!(a.frames.len(), 3);
    assert_eq!(a.size(), (16, 8));
    assert_eq!(a.frames[1].delay, Duration::from_millis(70));
    assert_eq!(a.duration(), Duration::from_millis(210));
    // 0 gecikme tarayıcılar gibi 100 ms sayılır.
    let Decoded::Animated(z) = decode_bytes(&gif(2, 0), &DecodeOptions::default().with_trust(Trust::System)).unwrap() else { panic!() };
    assert_eq!(z.frames[0].delay, Duration::from_millis(100));
}

#[test]
fn animation_limits_fall_back_to_first_frame() {
    let few = DecodeOptions { limits: Limits { max_frames: 2, ..Limits::DEFAULT }, ..Default::default() }.with_trust(Trust::System);
    assert!(matches!(decode_bytes(&gif(5, 50), &few).unwrap(), Decoded::Still(_)));
    let still = DecodeOptions { animate: false, ..Default::default() };
    assert!(matches!(decode_bytes(&gif(5, 50), &still).unwrap(), Decoded::Still(_)));
}

#[test]
fn decodes_jpeg_and_webp() {
    let img = RgbaImage::from_pixel(40, 30, image::Rgba([10, 200, 30, 255]));
    let rgb = image::DynamicImage::ImageRgba8(img.clone()).into_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .write_image(rgb.as_raw(), 40, 30, image::ExtendedColorType::Rgb8)
        .unwrap();
    let d = decode_bytes(&jpeg, &DecodeOptions::default()).unwrap().into_first_frame();
    assert_eq!(d.size(), (40, 30));
    let p = d.straight_pixel(20, 15);
    assert!(p[1] > 180 && p[3] == 255, "{p:?}");

    let mut webp = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(Cursor::new(&mut webp))
        .write_image(img.as_raw(), 40, 30, image::ExtendedColorType::Rgba8)
        .unwrap();
    let d = decode_bytes(&webp, &DecodeOptions::default()).unwrap().into_first_frame();
    assert_eq!(d.straight_pixel(0, 0), [10, 200, 30, 255]);
}

#[test]
fn decodes_svg_by_content_and_extension() {
    let svg = br##"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><circle cx="5" cy="5" r="5" fill="#00f"/></svg>"##;
    let opts = DecodeOptions { target: Some(TargetSize::new(48, 48, Fit::Contain)), ..Default::default() }.with_trust(Trust::System);
    assert_eq!(decode_bytes(svg, &opts).unwrap().size(), (48, 48));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icon.svg");
    std::fs::write(&path, svg).unwrap();
    assert_eq!(decode_file(&path, &DecodeOptions::default().with_trust(Trust::System)).unwrap().size(), (10, 10));
    // Uç en-boy oranı + Cover: tuval sınırı.
    let thin = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="10000"/>"#;
    let cover = DecodeOptions { target: Some(TargetSize::new(4000, 4000, Fit::Cover)), ..Default::default() }.with_trust(Trust::System);
    assert!(matches!(decode_bytes(thin, &cover), Err(ImageError::TooLarge { .. })));
}

#[test]
fn large_animations_stream_frames_in_background() {
    // 32x16 RGBA kare = 2048 bayt; bütçe 3000 → ikinci karede akışa geçer.
    let small = DecodeOptions { limits: Limits { max_animation_bytes: 3000, ..Limits::DEFAULT }, ..Default::default() }.with_trust(Trust::System);
    let Decoded::Streamed(s) = decode_bytes(&gif(4, 50), &small).unwrap() else { panic!("akış bekleniyordu") };
    assert_eq!(s.size(), (32, 16));
    assert_eq!(s.first_frame().image.straight_pixel(0, 0), [0, 255, 0, 255]);
    // Kareler sırayla gelir ve döngü başa sarar: 1, 2, 3, 0, 1, …
    let mut reds = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while reds.len() < 6 && std::time::Instant::now() < deadline {
        match s.try_next() {
            Some(f) => reds.push(f.image.straight_pixel(0, 0)[0]),
            None => std::thread::sleep(Duration::from_millis(1)),
        }
    }
    assert_eq!(reds, [60, 120, 180, 0, 60, 120]);
    drop(s); // thread katılır; asılı kalmamalı.
}
