//! Ham piksel girdileri: bildirimlerin `image-data` ipucu (`(iiibiiay)`: genişlik, yükseklik,
//! rowstride, has_alpha, bits_per_sample = 8, kanal 3/4, veri) ve tepsi ARGB32 pixmap'leri.
//! Dış (D-Bus) girdi olduğu için her alan doğrulanır; hata `Err`, panik yok. Çıktı en fazla
//! `max_side` kenarlı premultiplied RGBA'dır (büyük bildirim görselleri bellekte küçük kalır).

use crate::{Fit, Image, ImageError, Limits, TargetSize, resize};

/// Ham RGB(A) satırlarını doğrular, sıkı premultiplied RGBA'ya çevirir ve uzun kenarı
/// `max_side`'ı aşıyorsa en-boy korunarak küçültür (yüksek kaliteli, Lanczos3).
///
/// - `rowstride`: bir satırın bayt uzunluğu (dolgu içerebilir), ≥ `width × kanal`.
/// - `has_alpha`: 4 kanal (RGBA, düz alfa); değilse 3 kanal (RGB).
/// - `pixels.len()` ≥ `rowstride × (height − 1) + width × kanal` (son satır dolgusuz olabilir).
pub fn downscale_rgba(
    pixels: &[u8],
    width: u32,
    height: u32,
    rowstride: u32,
    has_alpha: bool,
    max_side: u32,
) -> Result<Image, ImageError> {
    let channels: u64 = if has_alpha { 4 } else { 3 };
    validate(pixels.len(), width, height, u64::from(rowstride), channels)?;
    let (w, h, stride) = (width as usize, height as usize, rowstride as usize);
    let row_bytes = w * channels as usize;
    let max_side = max_side.max(1);
    let target = TargetSize::new(max_side, max_side, Fit::Contain);
    let needs_resize = width > max_side || height > max_side;
    if !has_alpha && needs_resize {
        // Opak: RGB'de küçült, tam boyutlu RGBA kopyası yok.
        let mut rgb = Vec::with_capacity(row_bytes * h);
        for y in 0..h {
            rgb.extend_from_slice(&pixels[y * stride..y * stride + row_bytes]);
        }
        return resize::resize_rgb(&rgb, (width, height), resize::plan((width, height), target));
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &pixels[y * stride..y * stride + row_bytes];
        if has_alpha {
            rgba.extend_from_slice(row);
        } else {
            for px in row.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
    }
    let img = Image::from_straight(width, height, rgba)?;
    resize::fit(img, Some(target))
}

/// Tepsi (StatusNotifierItem) ARGB32 pixmap'i — ağ bayt sırası, düz alfa — en fazla
/// `max_side` kenara.
pub fn downscale_argb32(argb: &[u8], width: i32, height: i32, max_side: u32) -> Result<Image, ImageError> {
    let img = crate::icon::sni_pixmap_to_image(width, height, argb)?;
    resize::fit(img, Some(TargetSize::new(max_side.max(1), max_side.max(1), Fit::Contain)))
}

fn validate(len: usize, width: u32, height: u32, rowstride: u64, channels: u64) -> Result<(), ImageError> {
    Limits::DEFAULT.check_dimensions(width, height)?;
    let row = u64::from(width) * channels;
    if rowstride < row {
        return Err(ImageError::InvalidData(format!("rowstride {rowstride} < genişlik × kanal {row}")));
    }
    let needed = rowstride * (u64::from(height) - 1) + row;
    if (len as u64) < needed {
        return Err(ImageError::InvalidData(format!("{width}x{height} (rowstride {rowstride}) için {needed} bayt gerekir, {len} var")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_padded_rows_and_alpha() {
        // 2x2 RGBA, rowstride 12 (4 bayt dolgu); son satır dolgusuz.
        let px = [
            255, 0, 0, 255, 0, 255, 0, 128, 9, 9, 9, 9, //
            0, 0, 255, 255, 255, 255, 255, 0,
        ];
        let img = downscale_rgba(&px, 2, 2, 12, true, 64).unwrap();
        assert_eq!(img.size(), (2, 2));
        assert_eq!(&img.pixels()[..8], &[255, 0, 0, 255, 0, 128, 0, 128], "premultiplied");
        assert_eq!(&img.pixels()[12..], &[0, 0, 0, 0], "alfa 0 → 0");
    }

    #[test]
    fn rgb_without_alpha_is_opaque_and_downscaled() {
        let (w, h) = (300u32, 100u32);
        let stride = w * 3 + 2;
        let px = vec![200u8; (stride * h) as usize];
        let img = downscale_rgba(&px, w, h, stride, false, 60).unwrap();
        assert_eq!(img.size(), (60, 20));
        assert!(img.pixels().chunks_exact(4).all(|p| p[3] == 255 && (p[0] as i32 - 200).abs() <= 1));
        // Küçükse yalnızca dönüştürülür.
        assert_eq!(downscale_rgba(&px, w, h, stride, false, 512).unwrap().size(), (300, 100));
    }

    #[test]
    fn rejects_bad_dimensions() {
        let px = [0u8; 16];
        assert!(downscale_rgba(&px, 0, 2, 8, true, 8).is_err());
        assert!(downscale_rgba(&px, 2, 2, 7, true, 8).is_err(), "rowstride < genişlik × kanal");
        assert!(downscale_rgba(&px, 2, 3, 8, true, 8).is_err(), "veri kısa");
        assert!(downscale_rgba(&px, 100_000, 1, 400_000, true, 8).is_err(), "sınır");
        assert!(downscale_rgba(&px, u32::MAX, u32::MAX, u32::MAX, false, 8).is_err(), "taşma yok");
    }

    #[test]
    fn argb32_tray_pixmaps() {
        let argb: Vec<u8> = [255u8, 10, 20, 30].repeat(128 * 128);
        let img = downscale_argb32(&argb, 128, 128, 32).unwrap();
        assert_eq!(img.size(), (32, 32));
        assert_eq!(img.straight_pixel(5, 5), [10, 20, 30, 255]);
        assert!(downscale_argb32(&argb, 128, 127, 32).is_err());
    }
}
