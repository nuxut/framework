//! Görselden kaynak renk: küçültülmüş piksellerde Celebi niceleme + Material puanlama.
//!
//! Görsel bu crate'te çözülmez: dosya yolu verilen [`IsolatedDecoder`]'a (settings'te duvar
//! kağıdı profilli `Sandbox`, CLI'da kendi `Sandbox`'ı) gider ya da renk zaten çözülmüş bir
//! kareden ([`source_color_from_frame`], `WallpaperDecoder` önbelleği) alınır. JPEG XL çözücüsü
//! palet ağacına girmez (ADR-0007).

use std::path::Path;

use material_colors::color::Argb;
use material_colors::quantize::{Quantizer, QuantizerCelebi};
use material_colors::score::Score;
use nuxut_core::Color;
use nuxut_image::{CancelFlag, DecodeOptions, Fit, Image, IsolatedDecoder, Limits, TargetSize, WallpaperFrame};

use crate::PaletteError;

/// Niceleme öncesi küçültme kutusu (matugen 112 px kullanır; kalite/hız dengesi). `Contain`:
/// Cover kırpması kenar renklerini atardı, matugen tam görsele bakar.
pub const SAMPLE_TARGET: TargetSize = TargetSize::new(128, 128, Fit::Contain);

/// Renk örneklemesinin çözüm seçenekleri: duvar kağıdı sınırları (`Limits::WALLPAPER`), hedef
/// [`SAMPLE_TARGET`], yalnızca ilk kare. Yalıtılmış çözücüye ya da `WallpaperDecoder`'a
/// (`Fit::Contain`, 128×128) aynı istek.
pub fn sample_options() -> DecodeOptions {
    DecodeOptions { limits: Limits::WALLPAPER, target: Some(SAMPLE_TARGET), animate: false }
}

/// Görsel dosyasından kaynak renk. Dosya bir kez açılır ve `decoder`'a (yalıtılmış işçi;
/// `decode_file`) verilir; ana süreç dosyayı ayrıştırmaz. `key`: kaynak anahtarı (susturma
/// kovası), önerilen kalıp `nuxut_image::file_key("wallpaper", path)`. Animasyonda ilk kare.
///
/// KIRICI (ADR-0007 takip): önceki `source_color_from_file(path)` dosyayı süreç içinde
/// `Trust::System` ile çözüyordu.
pub fn source_color_from_file(path: &Path, decoder: &dyn IsolatedDecoder, key: u64) -> Result<Color, PaletteError> {
    let file = nuxut_image::open_user_file(path)?;
    let decoded = decoder.decode_file(key, file, &sample_options(), &CancelFlag::new())?;
    Ok(source_color(&decoded.image))
}

/// Çözülmüş duvar kağıdı karesinden kaynak renk (settings: aynı dosyanın `WallpaperDecoder`
/// karesi elde; yeniden çözüm yok). Büyük kare [`SAMPLE_TARGET`]'a süreç içinde (güvenilir
/// RGBA) küçültülür; kaynak kopyalanmaz.
pub fn source_color_from_frame(frame: &WallpaperFrame) -> Color {
    match frame.fit_image(SAMPLE_TARGET) {
        Ok(img) => source_color(&img),
        // Boyutlar kurucuda doğrulandı; küçültme hatası olursa tema varsayılanı.
        Err(_) => nuxut_theme::Palette::default().primary,
    }
}

/// Görselden kaynak renk. [`SAMPLE_TARGET`]'tan büyük görsel önce oraya küçültülür (güvenilir
/// RGBA, süreç içi); yarı saydam pikseller elenir. Hiç opak piksel yoksa Material'in tema
/// varsayılanının `primary` rengi döner.
pub fn source_color(img: &Image) -> Color {
    match img.resized(SAMPLE_TARGET) {
        Ok(Some(small)) => quantize(&small),
        _ => quantize(img),
    }
}

fn quantize(img: &Image) -> Color {
    let straight = img.to_straight();
    let pixels: Vec<Argb> = straight
        .chunks_exact(4)
        .filter(|p| p[3] == 255)
        .map(|p| Argb::new(255, p[0], p[1], p[2]))
        .collect();
    if pixels.is_empty() {
        return nuxut_theme::Palette::default().primary;
    }
    let result = QuantizerCelebi::quantize(&pixels, 128);
    let ranked = Score::score(&result.color_to_count, None, None, None);
    ranked.first().map_or_else(|| nuxut_theme::Palette::default().primary, |c| crate::scheme::from_rgb(*c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Test çözücüsü: varsayılan `decode_file` gövdesi (okur, `decode`'a verir); `decode`
    /// süreç içinde, `System` (yalnızca test).
    #[derive(Debug)]
    struct InProcess;

    impl IsolatedDecoder for InProcess {
        fn isolation(&self) -> nuxut_image::Isolation {
            nuxut_image::Isolation::Unavailable
        }
        fn decode(&self, _key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions) -> Result<Image, nuxut_image::ImageError> {
            nuxut_image::decode_bytes(&bytes, &opts.with_trust(nuxut_image::Trust::System)).map(nuxut_image::Decoded::into_first_frame)
        }
    }

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Image {
        Image::from_premultiplied(w, h, [rgb[0], rgb[1], rgb[2], 255].repeat((w * h) as usize)).unwrap()
    }

    #[test]
    fn large_image_is_sampled_and_matches_small() {
        let big = solid(1000, 600, [200, 30, 40]);
        let small = solid(100, 60, [200, 30, 40]);
        assert_eq!(source_color(&big), source_color(&small));
    }

    #[test]
    fn frame_and_file_paths_agree() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("duvar.png");
        let img = image::RgbImage::from_fn(640, 400, |x, _| if x < 320 { image::Rgb([20, 120, 200]) } else { image::Rgb([230, 180, 30]) });
        img.save(&path).unwrap();
        let from_file = source_color_from_file(&path, &InProcess, nuxut_image::file_key("wallpaper", &path)).unwrap();
        let decoder = nuxut_image::WallpaperDecoder::in_process_unsandboxed(nuxut_image::WallpaperDecoderOptions::default());
        let (tx, rx) = std::sync::mpsc::channel();
        let _t = decoder.request(nuxut_image::WallpaperRequest::new(&path, (128, 128), Fit::Contain), move |r| {
            let _ = tx.send(r);
        });
        let frame = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap().unwrap();
        assert_eq!(source_color_from_frame(&frame), from_file, "aynı 128² Contain örneği");
    }

    #[test]
    fn missing_file_is_an_error_without_path() {
        let e = source_color_from_file(Path::new("/yok/kişi.png"), &InProcess, 1).unwrap_err();
        assert!(!format!("{e:?} {e}").contains("kişi"));
    }
}
