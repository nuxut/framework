//! Yüksek kaliteli küçültme (`fast_image_resize`, SIMD, Lanczos3).

use fast_image_resize::images::{Image as FrImage, ImageRef};
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};

use crate::{Fit, Image, ImageError, TargetSize};

/// `src` boyutundaki görselin `target`'a göre çözüleceği boyut. Yalnızca küçültür; en-boy
/// oranı `Cover`/`Contain`'de korunur (her kenar ≥ 1). `Cover` + [`TargetSize::crop`]: taşan
/// eksen kutuya kırpılır (çıktı her iki eksende ≤ kutu).
pub fn fit_size(src: (u32, u32), target: TargetSize) -> (u32, u32) {
    plan(src, target).dst
}

/// Kaynak dikdörtgende ortadan kırpma (kesirli piksel; `fast_image_resize` biçimi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Crop {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// Küçültme planı: çıktı boyutu ve (varsa) kaynaktaki kırpma dikdörtgeni.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Plan {
    pub dst: (u32, u32),
    pub crop: Option<Crop>,
}

impl Plan {
    /// Kaynağa dokunmak gerekiyor mu (küçültme ya da kırpma).
    pub fn is_identity(&self, src: (u32, u32)) -> bool {
        self.dst == src && self.crop.is_none()
    }
}

pub(crate) fn plan(src: (u32, u32), target: TargetSize) -> Plan {
    let (w, h) = (src.0.max(1) as f64, src.1.max(1) as f64);
    let (tw, th) = (target.width.max(1) as f64, target.height.max(1) as f64);
    let s = match target.fit {
        Fit::Fill => {
            let dst = ((w.min(tw).round() as u32).max(1), (h.min(th).round() as u32).max(1));
            return Plan { dst, crop: None };
        }
        Fit::Cover => (tw / w).max(th / h).min(1.0),
        Fit::Contain => (tw / w).min(th / h).min(1.0),
    };
    let full = (((w * s).round() as u32).max(1), ((h * s).round() as u32).max(1));
    if !target.crops() {
        return Plan { dst: full, crop: None };
    }
    let dst = (full.0.min(target.width.max(1)), full.1.min(target.height.max(1)));
    if dst == full {
        return Plan { dst, crop: None };
    }
    // Kırpma kaynakta: çıktı kenarı / ölçek, ortalanmış. Ölçek 1'de (yalnız kırpma) kayma tam
    // piksel: yarım piksellik kayma kopyayı yeniden örneklerdi.
    let (cw, ch) = (dst.0 as f64 / s, dst.1 as f64 / s);
    let (mut left, mut top) = ((w - cw) / 2.0, (h - ch) / 2.0);
    if s >= 1.0 {
        (left, top) = (left.floor(), top.floor());
    }
    Plan { dst, crop: Some(Crop { left: left.max(0.0), top: top.max(0.0), width: cw.min(w), height: ch.min(h) }) }
}

fn options(filter: FilterType, crop: Option<Crop>) -> ResizeOptions {
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(filter)).use_alpha(false);
    match crop {
        Some(c) => opts.crop(c.left, c.top, c.width, c.height),
        None => opts,
    }
}

impl Image {
    /// `target`'a küçültür (`Cover` + `crop`'ta kırpar; Lanczos3, asla büyütmez); zaten
    /// uygunsa kopyasız aynı görsel.
    pub fn fit(self, target: TargetSize) -> Result<Self, ImageError> {
        fit(self, Some(target))
    }

    /// `target`'a küçültülmüş yeni görsel; zaten uygunsa `None`. Kaynak kopyalanmaz.
    pub fn resized(&self, target: TargetSize) -> Result<Option<Self>, ImageError> {
        fit_premultiplied(self.pixels(), self.size(), target)
    }
}

/// Görseli `target`'a göre küçültür (ve `Cover` + `crop`'ta kırpar); zaten uygunsa olduğu
/// gibi döner.
pub(crate) fn fit(image: Image, target: Option<TargetSize>) -> Result<Image, ImageError> {
    let Some(target) = target else { return Ok(image) };
    let p = plan(image.size(), target);
    if p.is_identity(image.size()) {
        return Ok(image);
    }
    resize_cropped(&image, p.dst, p.crop)
}

/// Premultiplied pikselleri (`size`) `target`'a göre küçültür/kırpar; zaten uygunsa `None`
/// (çağıran kaynağı olduğu gibi kullanır). Kaynak kopyalanmaz (duvar kağıdı türetmesi).
pub(crate) fn fit_premultiplied(pixels: &[u8], size: (u32, u32), target: TargetSize) -> Result<Option<Image>, ImageError> {
    let p = plan(size, target);
    if p.is_identity(size) {
        return Ok(None);
    }
    resize_slice(pixels, size, p.dst, p.crop).map(Some)
}

/// Premultiplied görseli tam `w`×`h`'ye (varsa `crop` kırpmasıyla) yeniden örnekler.
/// Premultiplied veride doğrusal filtre doğrudur; Lanczos'un negatif lobları rengi alpha'nın
/// üstüne taşırabildiği için sonuç `c ≤ a`'ya sıkıştırılır.
pub(crate) fn resize_cropped(image: &Image, dst: (u32, u32), crop: Option<Crop>) -> Result<Image, ImageError> {
    resize_slice(image.pixels(), image.size(), dst, crop)
}

fn resize_slice(pixels: &[u8], (sw, sh): (u32, u32), (w, h): (u32, u32), crop: Option<Crop>) -> Result<Image, ImageError> {
    let src = ImageRef::new(sw, sh, pixels, PixelType::U8x4).map_err(|e| ImageError::Resize(e.to_string()))?;
    let mut dst = FrImage::new(w, h, PixelType::U8x4);
    let (cw, ch) = crop.map_or((f64::from(sw), f64::from(sh)), |c| (c.width, c.height));
    let filter = if f64::from(w) < cw || f64::from(h) < ch { FilterType::Lanczos3 } else { FilterType::CatmullRom };
    Resizer::new().resize(&src, &mut dst, &options(filter, crop)).map_err(|e| ImageError::Resize(e.to_string()))?;
    let mut pixels = dst.into_vec();
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3];
        for c in &mut px[..3] {
            *c = (*c).min(a);
        }
    }
    Image::from_premultiplied(w, h, pixels)
}

/// Opak RGB8 kaynağı doğrudan küçültüp RGBA8'e genişletir: büyük JPEG'lerde tam boyutlu RGBA
/// kopyası (tepe bellek) oluşmaz.
pub(crate) fn resize_rgb(rgb: &[u8], src: (u32, u32), p: Plan) -> Result<Image, ImageError> {
    let (w, h) = p.dst;
    let small = resize_channels(rgb, PixelType::U8x3, src, p, FilterType::Lanczos3)?;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for px in small.chunks_exact(3) {
        rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    Image::from_premultiplied(w, h, rgba)
}

/// Gri (L8) kaynağı tek kanalda küçültüp RGBA8'e genişletir (opak).
pub(crate) fn resize_luma(luma: &[u8], src: (u32, u32), p: Plan) -> Result<Image, ImageError> {
    let (w, h) = p.dst;
    let small = resize_channels(luma, PixelType::U8, src, p, FilterType::Lanczos3)?;
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for &l in &small {
        rgba.extend_from_slice(&[l, l, l, 255]);
    }
    Image::from_premultiplied(w, h, rgba)
}

/// Düz alfalı gri (LA8) kaynağı yerinde premultiply edip iki kanalda küçültür, sonra RGBA8'e
/// genişletir. Premultiply ve `c ≤ a` sıkıştırması RGBA yolundakiyle ([`resize`]) aynı.
pub(crate) fn resize_luma_alpha(mut la: Vec<u8>, src: (u32, u32), p: Plan) -> Result<Image, ImageError> {
    let (w, h) = p.dst;
    for px in la.chunks_exact_mut(2) {
        let a = u16::from(px[1]);
        if a != 255 {
            px[0] = ((u16::from(px[0]) * a + 127) / 255) as u8;
        }
    }
    let small = resize_channels(&la, PixelType::U8x2, src, p, FilterType::Lanczos3)?;
    drop(la);
    let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
    for px in small.chunks_exact(2) {
        let (l, a) = (px[0].min(px[1]), px[1]);
        rgba.extend_from_slice(&[l, l, l, a]);
    }
    Image::from_premultiplied(w, h, rgba)
}

/// `pixel` biçimindeki sıkı satırları `p.dst`'ye (varsa `p.crop` kırpmasıyla) yeniden örnekler
/// (kanallar bağımsız).
fn resize_channels(src: &[u8], pixel: PixelType, (sw, sh): (u32, u32), p: Plan, filter: FilterType) -> Result<Vec<u8>, ImageError> {
    let src = ImageRef::new(sw, sh, src, pixel).map_err(|e| ImageError::Resize(e.to_string()))?;
    let mut dst = FrImage::new(p.dst.0, p.dst.1, pixel);
    Resizer::new().resize(&src, &mut dst, &options(filter, p.crop)).map_err(|e| ImageError::Resize(e.to_string()))?;
    Ok(dst.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_upscales() {
        assert_eq!(fit_size((100, 50), TargetSize::new(400, 400, Fit::Cover)), (100, 50));
        assert_eq!(fit_size((100, 50), TargetSize::new(400, 400, Fit::Fill)), (100, 50));
    }

    #[test]
    fn cover_and_contain_keep_aspect() {
        // 4000x2000 → 1920x1080: cover yüksekliği doldurur, contain genişliği.
        assert_eq!(fit_size((4000, 2000), TargetSize::new(1920, 1080, Fit::Cover)), (2160, 1080));
        assert_eq!(fit_size((4000, 2000), TargetSize::new(1920, 1080, Fit::Contain)), (1920, 960));
        assert_eq!(fit_size((4000, 2000), TargetSize::new(1920, 1080, Fit::Fill)), (1920, 1080));
    }

    #[test]
    fn resize_keeps_premultiplied_invariant() {
        // Sert kenarlı yarı saydam desen: Lanczos taşması c ≤ a ile sıkıştırılmalı.
        let mut px = Vec::new();
        for y in 0..64u32 {
            for x in 0..64u32 {
                if (x / 4 + y / 4) % 2 == 0 { px.extend_from_slice(&[255, 255, 255, 255]) } else { px.extend_from_slice(&[0, 0, 0, 0]) }
            }
        }
        let img = Image::from_premultiplied(64, 64, px).unwrap();
        let out = resize_cropped(&img, (13, 9), None).unwrap();
        assert_eq!(out.size(), (13, 9));
        for p in out.pixels().chunks_exact(4) {
            assert!(p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]);
        }
    }
}
