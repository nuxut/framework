//! StatusNotifierItem `IconPixmap`: `a(iiay)` — genişlik, yükseklik, ARGB32 **ağ bayt
//! sırasında** (büyük endian: A, R, G, B), düz alpha.

use crate::{Image, ImageError, Limits};

/// Tek bir SNI pixmap'ini premultiplied RGBA görsele çevirir. Boyutlar `Limits::ICON` ile
/// sınırlıdır; veri uzunluğu `w*h*4` olmalı.
pub fn sni_pixmap_to_image(width: i32, height: i32, argb: &[u8]) -> Result<Image, ImageError> {
    let (Ok(w), Ok(h)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(ImageError::InvalidData(format!("negatif boyut {width}x{height}")));
    };
    Limits::ICON.check_dimensions(w, h)?;
    let len = w as usize * h as usize * 4;
    if argb.len() != len {
        return Err(ImageError::InvalidData(format!("{w}x{h} için {} bayt", argb.len())));
    }
    let mut rgba = Vec::with_capacity(len);
    for px in argb.chunks_exact(4) {
        rgba.extend_from_slice(&[px[1], px[2], px[3], px[0]]);
    }
    Image::from_straight(w, h, rgba)
}

/// Pixmap listesinden `px` fiziksel boyuta en uygun olanın indeksi: `px`'ten büyük ya da eşit
/// olanların en küçüğü (küçültme kaliteli), yoksa en büyüğü. Geçersiz boyutlar atlanır.
pub fn best_sni_pixmap(sizes: impl IntoIterator<Item = (i32, i32)>, px: u32) -> Option<usize> {
    let px = px as i64;
    let mut best: Option<(usize, i64)> = None;
    for (i, (w, h)) in sizes.into_iter().enumerate() {
        if w <= 0 || h <= 0 {
            continue;
        }
        let side = i64::from(w.max(h));
        let better = match best {
            None => true,
            Some((_, b)) => {
                if b >= px { side >= px && side < b } else { side > b }
            }
        };
        if better {
            best = Some((i, side));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_network_order_argb() {
        // Yarı saydam kırmızı + opak mavi.
        let data = [128, 255, 0, 0, 255, 0, 0, 255];
        let img = sni_pixmap_to_image(2, 1, &data).unwrap();
        assert_eq!(img.pixels(), &[128, 0, 0, 128, 0, 0, 255, 255]);
    }

    #[test]
    fn rejects_bad_pixmaps() {
        assert!(sni_pixmap_to_image(-1, 4, &[]).is_err());
        assert!(sni_pixmap_to_image(2, 2, &[0; 15]).is_err());
        assert!(sni_pixmap_to_image(100_000, 1, &[]).is_err());
    }

    #[test]
    fn picks_smallest_sufficient() {
        let sizes = [(16, 16), (64, 64), (32, 32), (0, 0)];
        assert_eq!(best_sni_pixmap(sizes, 24), Some(2));
        assert_eq!(best_sni_pixmap(sizes, 128), Some(1));
        assert_eq!(best_sni_pixmap(sizes, 16), Some(0));
        assert_eq!(best_sni_pixmap([], 16), None);
    }
}
