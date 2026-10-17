//! Yakalanan tamponun saf piksel işleri (Wayland'e dokunmaz; birim testli): biçim seçimi,
//! sığdırma, kutu filtresiyle küçültme + biçim çevirme, y-ters çevirme ve dönüşüm geri alma.
//!
//! Çıktı her zaman sıkı satırlı, **premultiplied** RGBA8 (`nuxut_render::ImageData`). Compositor
//! ARGB/ABGR'yi zaten premultiplied verir; kutu filtresi premultiplied uzayda alan ortalaması
//! aldığı için doğrudur. X biçimlerinde ve opak kaynakta (çıkış) alfa 255'tir.

/// Desteklenen 8 bit `wl_shm` biçimleri. Bayt düzeni little-endian bellekte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PixelFormat {
    /// `0xAARRGGBB` → bellekte B, G, R, A.
    Argb8888,
    /// `0xXXRRGGBB` → bellekte B, G, R, X.
    Xrgb8888,
    /// `0xAABBGGRR` → bellekte R, G, B, A.
    Abgr8888,
    /// `0xXXBBGGRR` → bellekte R, G, B, X.
    Xbgr8888,
}

/// `wl_shm.format` kodları (ARGB/XRGB özel: 0 ve 1; diğerleri fourcc).
const SHM_ARGB8888: u32 = 0;
const SHM_XRGB8888: u32 = 1;
const SHM_ABGR8888: u32 = 0x3432_4241; // 'AB24'
const SHM_XBGR8888: u32 = 0x3432_4258; // 'XB24'

impl PixelFormat {
    pub(crate) fn from_shm(code: u32) -> Option<Self> {
        match code {
            SHM_ARGB8888 => Some(Self::Argb8888),
            SHM_XRGB8888 => Some(Self::Xrgb8888),
            SHM_ABGR8888 => Some(Self::Abgr8888),
            SHM_XBGR8888 => Some(Self::Xbgr8888),
            _ => None,
        }
    }

    pub(crate) fn shm_code(self) -> u32 {
        match self {
            Self::Argb8888 => SHM_ARGB8888,
            Self::Xrgb8888 => SHM_XRGB8888,
            Self::Abgr8888 => SHM_ABGR8888,
            Self::Xbgr8888 => SHM_XBGR8888,
        }
    }

    /// Tercih sırası (küçük önce): alfalı ARGB pencerede saydamlığı korur.
    fn rank(self) -> u8 {
        match self {
            Self::Argb8888 => 0,
            Self::Xrgb8888 => 1,
            Self::Abgr8888 => 2,
            Self::Xbgr8888 => 3,
        }
    }

    /// Bellekteki R, G, B bayt indisleri ve alfanın indisi (X biçimde `None`).
    fn layout(self) -> ([usize; 3], Option<usize>) {
        match self {
            Self::Argb8888 => ([2, 1, 0], Some(3)),
            Self::Xrgb8888 => ([2, 1, 0], None),
            Self::Abgr8888 => ([0, 1, 2], Some(3)),
            Self::Xbgr8888 => ([0, 1, 2], None),
        }
    }
}

/// Önerilen kodlardan en iyi desteklenen biçim.
pub(crate) fn choose_format(offered: impl IntoIterator<Item = u32>) -> Option<PixelFormat> {
    offered
        .into_iter()
        .filter_map(PixelFormat::from_shm)
        .min_by_key(|f| f.rank())
}

/// `src`'yi en-boy oranını koruyarak `max` kutusuna sığdırır; büyütmez, en az 1×1.
pub(crate) fn fit_size(src: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (w, h) = (src.0.max(1), src.1.max(1));
    let (mw, mh) = (max.0.max(1), max.1.max(1));
    if w <= mw && h <= mh {
        return (w, h);
    }
    let scale = (f64::from(mw) / f64::from(w)).min(f64::from(mh) / f64::from(h));
    let fw = ((f64::from(w) * scale).round() as u32).clamp(1, mw);
    let fh = ((f64::from(h) * scale).round() as u32).clamp(1, mh);
    (fw, fh)
}

/// Kaynak tampon görünümü (eşlenmiş bellek).
#[derive(Clone, Copy)]
pub(crate) struct Source<'a> {
    pub bytes: &'a [u8],
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
    /// Alfa yok sayılır, 255 yazılır (çıkış kaynağı).
    pub opaque: bool,
}

impl Source<'_> {
    /// Tampon boyutları ve satır adımı tutarlı mı (taşma/kısa tampon yok).
    pub(crate) fn is_valid(&self) -> bool {
        let row = (self.width as usize).checked_mul(4);
        let need = (self.height as usize)
            .checked_sub(1)
            .and_then(|h| h.checked_mul(self.stride))
            .and_then(|n| row.and_then(|r| n.checked_add(r)));
        self.width > 0
            && self.height > 0
            && row.is_some_and(|r| self.stride >= r)
            && need.is_some_and(|n| n <= self.bytes.len())
    }
}

/// Kutu filtresiyle (alan ortalaması) `dst` boyutuna küçültür ve RGBA8'e çevirir. `dst` kaynaktan
/// büyük olamaz (çağıran `fit_size` kullanır). Geçersiz kaynakta `None`.
pub(crate) fn downscale(src: &Source<'_>, dst: (u32, u32)) -> Option<Vec<u8>> {
    if !src.is_valid() || dst.0 == 0 || dst.1 == 0 || dst.0 > src.width || dst.1 > src.height {
        return None;
    }
    let (dw, dh) = (dst.0 as usize, dst.1 as usize);
    let (sw, sh) = (src.width as usize, src.height as usize);
    let ([ri, gi, bi], alpha_idx) = src.format.layout();
    let alpha = if src.opaque { None } else { alpha_idx };
    let mut out = vec![0u8; dw * dh * 4];
    // Hedef sütunların kaynak aralıkları bir kez hesaplanır.
    let cols: Vec<(usize, usize)> = (0..dw)
        .map(|dx| {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1);
            (x0, x1)
        })
        .collect();
    let mut sums = vec![[0u64; 4]; dw];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1);
        sums.iter_mut().for_each(|s| *s = [0; 4]);
        for y in y0..y1 {
            let row = &src.bytes[y * src.stride..y * src.stride + sw * 4];
            for (dx, &(x0, x1)) in cols.iter().enumerate() {
                let s = &mut sums[dx];
                for px in row[x0 * 4..x1 * 4].chunks_exact(4) {
                    s[0] += u64::from(px[ri]);
                    s[1] += u64::from(px[gi]);
                    s[2] += u64::from(px[bi]);
                    s[3] += alpha.map_or(255, |a| u64::from(px[a]));
                }
            }
        }
        let rows = (y1 - y0) as u64;
        for (dx, &(x0, x1)) in cols.iter().enumerate() {
            let n = rows * (x1 - x0) as u64;
            let o = (dy * dw + dx) * 4;
            for c in 0..4 {
                // En yakına yuvarlanmış ortalama.
                out[o + c] = ((sums[dx][c] + n / 2) / n) as u8;
            }
        }
    }
    Some(out)
}

/// Satırları ters çevirir (wlr-screencopy `y_invert`).
pub(crate) fn flip_rows(pixels: &mut [u8], width: u32, height: u32) {
    let row = width as usize * 4;
    let h = height as usize;
    for y in 0..h / 2 {
        let (top, bottom) = pixels.split_at_mut((h - 1 - y) * row);
        top[y * row..y * row + row].swap_with_slice(&mut bottom[..row]);
    }
}

/// `wl_output.transform` kodu (0..=7; bilinmeyen 0 sayılır).
pub(crate) fn transform_swaps_axes(transform: u32) -> bool {
    matches!(transform, 1 | 3 | 5 | 7)
}

/// `transform`'un tersi: 90 ↔ 270, diğerleri kendi tersidir (çevrikler dahil).
pub(crate) fn inverse_transform(transform: u32) -> u32 {
    match transform {
        1 => 3,
        3 => 1,
        t if t <= 7 => t,
        _ => 0,
    }
}

/// `wl_output.transform` uygulanınca (x, y) pikselinin yeni yeri; `w`×`h` kaynak boyutu.
/// 90: saat yönünün tersine 90°; çevrik: önce yatay ayna, sonra döndürme.
fn transform_point(transform: u32, w: usize, h: usize, x: usize, y: usize) -> (usize, usize) {
    match transform {
        1 => (y, w - 1 - x),
        2 => (w - 1 - x, h - 1 - y),
        3 => (h - 1 - y, x),
        4 => (w - 1 - x, y),
        5 => (y, x),
        6 => (x, h - 1 - y),
        7 => (h - 1 - y, w - 1 - x),
        _ => (x, y),
    }
}

/// RGBA8 görsele `transform` uygular; yeni boyutlarla döner.
pub(crate) fn apply_transform(
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    transform: u32,
) -> (Vec<u8>, u32, u32) {
    if transform == 0 || transform > 7 {
        return (pixels, width, height);
    }
    let (w, h) = (width as usize, height as usize);
    let (nw, nh) = if transform_swaps_axes(transform) {
        (h, w)
    } else {
        (w, h)
    };
    let mut out = vec![0u8; pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let (tx, ty) = transform_point(transform, w, h, x, y);
            let s = (y * w + x) * 4;
            let d = (ty * nw + tx) * 4;
            out[d..d + 4].copy_from_slice(&pixels[s..s + 4]);
        }
    }
    (out, nw as u32, nh as u32)
}

/// Tam çevirme: kaynak tampon (compositor `transform`'u uygulanmış, `y_invert`'li olabilir) →
/// dik, `max` kutusuna sığmış RGBA8. Dönüş: (pikseller, genişlik, yükseklik).
pub(crate) fn render_preview(
    src: &Source<'_>,
    buffer_transform: u32,
    y_invert: bool,
    max: (u32, u32),
) -> Option<(Vec<u8>, u32, u32)> {
    let undo = inverse_transform(buffer_transform);
    let swaps = transform_swaps_axes(undo);
    // Son (dik) yöndeki boyut kutuya sığdırılır; küçültme tampon yönünde yapılır.
    let upright = if swaps {
        (src.height, src.width)
    } else {
        (src.width, src.height)
    };
    let (fw, fh) = fit_size(upright, max);
    let target = if swaps { (fh, fw) } else { (fw, fh) };
    let mut pixels = downscale(src, target)?;
    if y_invert {
        flip_rows(&mut pixels, target.0, target.1);
    }
    Some(apply_transform(pixels, target.0, target.1, undo))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        px.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    fn src(bytes: &[u8], w: u32, h: u32, format: PixelFormat, opaque: bool) -> Source<'_> {
        Source {
            bytes,
            width: w,
            height: h,
            stride: w as usize * 4,
            format,
            opaque,
        }
    }

    #[test]
    fn format_choice_prefers_alpha_and_ignores_unknown() {
        assert_eq!(choose_format([1, 0]), Some(PixelFormat::Argb8888));
        assert_eq!(choose_format([SHM_XBGR8888, 1]), Some(PixelFormat::Xrgb8888));
        assert_eq!(choose_format([SHM_XBGR8888]), Some(PixelFormat::Xbgr8888));
        // XRGB2101010 ('XR30') ve rastgele kod desteklenmez.
        assert_eq!(choose_format([0x3033_5258, 77]), None);
        assert_eq!(choose_format([]), None);
        for f in [
            PixelFormat::Argb8888,
            PixelFormat::Xrgb8888,
            PixelFormat::Abgr8888,
            PixelFormat::Xbgr8888,
        ] {
            assert_eq!(PixelFormat::from_shm(f.shm_code()), Some(f));
        }
    }

    #[test]
    fn fit_keeps_aspect_and_never_upscales() {
        assert_eq!(fit_size((1920, 1080), (320, 320)), (320, 180));
        assert_eq!(fit_size((1080, 1920), (320, 320)), (180, 320));
        assert_eq!(fit_size((100, 50), (320, 320)), (100, 50));
        assert_eq!(fit_size((10000, 1), (100, 100)), (100, 1), "en az 1");
        assert_eq!(fit_size((0, 0), (0, 0)), (1, 1));
    }

    #[test]
    fn byte_order_is_converted_to_rgba() {
        // ARGB8888 bellekte B,G,R,A: kırmızı = [0,0,255,255].
        let bytes = solid(2, 2, [0, 0, 255, 255]);
        let out = downscale(&src(&bytes, 2, 2, PixelFormat::Argb8888, false), (2, 2)).unwrap();
        assert_eq!(&out[..4], &[255, 0, 0, 255]);
        // ABGR8888 bellekte R,G,B,A.
        let bytes = solid(1, 1, [10, 20, 30, 40]);
        let out = downscale(&src(&bytes, 1, 1, PixelFormat::Abgr8888, false), (1, 1)).unwrap();
        assert_eq!(out, [10, 20, 30, 40]);
        // X biçimi: alfa 255, X baytı yok sayılır.
        let bytes = solid(1, 1, [1, 2, 3, 0]);
        let out = downscale(&src(&bytes, 1, 1, PixelFormat::Xrgb8888, false), (1, 1)).unwrap();
        assert_eq!(out, [3, 2, 1, 255]);
        // Opak kaynak: ARGB'de bile alfa 255.
        let bytes = solid(1, 1, [1, 2, 3, 0]);
        let out = downscale(&src(&bytes, 1, 1, PixelFormat::Argb8888, true), (1, 1)).unwrap();
        assert_eq!(out, [3, 2, 1, 255]);
    }

    #[test]
    fn box_filter_averages_premultiplied() {
        // 2×2: beyaz, siyah, siyah, beyaz → tek piksel gri (yuvarlanmış ortalama).
        let mut bytes = Vec::new();
        for px in [[255u8; 4], [0, 0, 0, 255], [0, 0, 0, 255], [255; 4]] {
            bytes.extend_from_slice(&px);
        }
        let out = downscale(&src(&bytes, 2, 2, PixelFormat::Argb8888, false), (1, 1)).unwrap();
        assert_eq!(out, [128, 128, 128, 255]);
        // Büyük düz alan düz kalır (taşma yok: 4096² × 255 u64'e sığar).
        let bytes = solid(512, 256, [40, 80, 120, 255]);
        let out = downscale(&src(&bytes, 512, 256, PixelFormat::Abgr8888, false), (3, 2)).unwrap();
        assert!(out.chunks_exact(4).all(|p| p == [40, 80, 120, 255]));
    }

    #[test]
    fn stride_padding_is_skipped_and_invalid_rejected() {
        // 1×2, satır adımı 8 (4 bayt dolgu).
        let bytes = [1, 2, 3, 255, 9, 9, 9, 9, 4, 5, 6, 255];
        let s = Source {
            bytes: &bytes,
            width: 1,
            height: 2,
            stride: 8,
            format: PixelFormat::Abgr8888,
            opaque: false,
        };
        assert_eq!(downscale(&s, (1, 2)).unwrap(), [1, 2, 3, 255, 4, 5, 6, 255]);
        // Kısa tampon, dar adım, büyütme, sıfır hedef reddedilir.
        let short = Source {
            bytes: &bytes[..10],
            ..s
        };
        assert!(downscale(&short, (1, 2)).is_none());
        let narrow = Source { stride: 3, ..s };
        assert!(downscale(&narrow, (1, 1)).is_none());
        assert!(downscale(&s, (2, 2)).is_none());
        assert!(downscale(&s, (0, 1)).is_none());
    }

    /// 3×2 görsel, her piksel kendi indisini taşır.
    fn indexed(w: u32, h: u32) -> Vec<u8> {
        (0..w * h).flat_map(|i| [i as u8, 0, 0, 255]).collect()
    }

    #[test]
    fn transforms_rotate_and_mirror() {
        // 3×2:  0 1 2
        //       3 4 5
        let img = indexed(3, 2);
        let at = |p: &[u8], w: u32, x: u32, y: u32| p[((y * w + x) * 4) as usize];
        // 90° saat yönü tersi: sağ üst (2) sol üste gelir; 2×3.
        let (p, w, h) = apply_transform(img.clone(), 3, 2, 1);
        assert_eq!((w, h), (2, 3));
        assert_eq!([at(&p, w, 0, 0), at(&p, w, 1, 0)], [2, 5]);
        assert_eq!([at(&p, w, 0, 2), at(&p, w, 1, 2)], [0, 3]);
        // 180°.
        let (p, w, _) = apply_transform(img.clone(), 3, 2, 2);
        assert_eq!(at(&p, w, 0, 0), 5);
        // Yatay ayna.
        let (p, w, _) = apply_transform(img.clone(), 3, 2, 4);
        assert_eq!([at(&p, w, 0, 0), at(&p, w, 2, 1)], [2, 3]);
        // Her dönüşüm tersiyle birlikte birim.
        for t in 0..8 {
            let (p, w, h) = apply_transform(img.clone(), 3, 2, t);
            let (back, bw, bh) = apply_transform(p, w, h, inverse_transform(t));
            assert_eq!((bw, bh), (3, 2), "t={t}");
            assert_eq!(back, img, "t={t}");
        }
        assert_eq!(inverse_transform(99), 0);
    }

    #[test]
    fn flip_rows_reverses_vertical_order() {
        let mut img = indexed(2, 3);
        flip_rows(&mut img, 2, 3);
        assert_eq!(img[0], 4);
        assert_eq!(img[4 * 4], 0);
        assert_eq!(img[2 * 4], 2, "orta satır yerinde");
    }

    #[test]
    fn preview_fits_upright_box_for_rotated_buffer() {
        // Tampon 400×100, compositor 90° uygulamış → dik görüntü 100×400; kutu 50×50.
        let bytes = solid(400, 100, [0, 0, 255, 255]);
        let s = src(&bytes, 400, 100, PixelFormat::Argb8888, true);
        let (p, w, h) = render_preview(&s, 1, false, (50, 50)).unwrap();
        assert_eq!((w, h), (13, 50));
        assert_eq!(p.len(), (w * h * 4) as usize);
        assert_eq!(&p[..4], &[255, 0, 0, 255]);
        // Dönüşümsüz.
        let (_, w, h) = render_preview(&s, 0, true, (50, 50)).unwrap();
        assert_eq!((w, h), (50, 13));
    }
}
