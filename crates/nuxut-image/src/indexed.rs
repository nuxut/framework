//! Paletli (indeksli) PNG'yi küçültmeye doğrudan, en dar kanal biçiminde hazırlar.
//!
//! `image`'ın PNG çözücüsü paleti her zaman RGB8'e (tRNS varsa RGBA8'e) açar: gri paletli bir
//! tarama 1 yerine 3 B/px, saydamlıklı paletli görsel 4 B/px tam boyutlu tampon olur. Burada
//! indeksler `png` ile satır satır okunur ve her satır palet tablosundan doğrudan **gerekli en
//! dar** biçime yazılır: gri+opak → L8 (1 B/px), gri+saydam → LA8 (2). Ardından `resize`'ın dar
//! yolları (önce küçült, sonra RGBA'ya aç) kullanılır. Tam boyutlu ara tampon yalnızca bu dar
//! tampondur (+ tek satır).
//!
//! Renkli palette (RGB8/RGBA8) `image` zaten aynı genişlikte tampon üretir: bellek kazancı yok,
//! burada piksel piksel açmak `png`'nin genişletmesinden yavaş (ölçüm 2026-10-16, 6000² → 1920:
//! renkli+saydam 207 MB her iki yolda, süre ~240 → ~310 ms). Bu yüzden renkli palet `None`.
//!
//! Sonuç `image` yoluyla piksel piksel aynıdır: palet tablosu `png`'nin genişletmesiyle aynı
//! kuralla kurulur (eksik giriş opak siyah; paletten uzun tRNS yok sayılır) ve dar yollar
//! RGBA yoluyla aynı sonucu verir (test: `indexed_png_matches_expanded_path`).
//!
//! Yalnızca palet gri ise, küçültme gerekiyorsa, taramalı (interlaced) değilse ve EXIF yönü
//! yoksa devreye girer; öbür durumlarda `None` (çağıran olağan yoldan çözer).

use std::io::Cursor;

use crate::resize::{self, Plan};
use crate::{DecodeOptions, Image, ImageError};

/// Gri palet girdilerinin (ve eksik girişlerin) en dar karşılığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Narrow {
    Luma,
    LumaAlpha,
}

impl Narrow {
    fn channels(self) -> usize {
        match self {
            Self::Luma => 1,
            Self::LumaAlpha => 2,
        }
    }

    /// Renkli palet: `None` (kazanç yok, bkz. modül belgesi).
    fn of(lut: &[[u8; 4]; 256]) -> Option<Self> {
        if !lut.iter().all(|p| p[0] == p[1] && p[1] == p[2]) {
            return None;
        }
        Some(if lut.iter().all(|p| p[3] == 255) { Self::Luma } else { Self::LumaAlpha })
    }
}

/// `png`'nin palet genişletmesiyle aynı tablo: 256 giriş, varsayılan opak siyah; tRNS paletten
/// uzunsa geçersiz sayılıp yok sayılır, kısaysa kalan girişler opaktır.
fn palette_lut(palette: &[u8], trns: Option<&[u8]>) -> [[u8; 4]; 256] {
    let entries = palette.len() / 3;
    let trns = trns.filter(|t| t.len() <= entries).unwrap_or(&[]);
    let mut lut = [[0, 0, 0, 255]; 256];
    for (i, rgb) in palette.chunks_exact(3).take(256).enumerate() {
        lut[i] = [rgb[0], rgb[1], rgb[2], trns.get(i).copied().unwrap_or(255)];
    }
    lut
}

/// Paletli PNG'yi `opts.target`'a küçültür. Uygun değilse (paletli değil, küçültme yok,
/// taramalı, EXIF yönlü) `Ok(None)`. Boyut ve bellek kapısı çağıranda ([`crate::decode`]'un
/// `gate`'i) geçilmiş olmalı; burada `png`'ye aynı ayırma sınırı verilir.
pub(crate) fn decode_png_indexed(bytes: &[u8], opts: &DecodeOptions, oriented: bool) -> Result<Option<Image>, ImageError> {
    let Some(target) = opts.target else { return Ok(None) };
    if oriented {
        return Ok(None);
    }
    let limits = png::Limits { bytes: usize::try_from(opts.limits.max_decode_bytes).unwrap_or(usize::MAX) };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(png_error)?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    let src = (w, h);
    let plan = resize::plan(src, target);
    if info.color_type != png::ColorType::Indexed || info.interlaced || plan.is_identity(src) {
        return Ok(None);
    }
    let Some(palette) = info.palette.as_deref() else {
        return Err(ImageError::InvalidData("paletli PNG'de PLTE yok".into()));
    };
    let lut = palette_lut(palette, info.trns.as_deref());
    let Some(kind) = Narrow::of(&lut) else { return Ok(None) };
    let bits = info.bit_depth as u8;
    let ch = kind.channels();
    let width = w as usize;
    let mut out = Vec::with_capacity(width * h as usize * ch);
    while let Some(row) = reader.next_row().map_err(png_error)? {
        expand_row(row.data(), width, bits, &lut, kind, &mut out);
    }
    if out.len() != width * h as usize * ch {
        return Err(ImageError::InvalidData(format!("paletli PNG eksik: {} satır bekleniyordu", h)));
    }
    finish(out, src, plan, kind).map(Some)
}

/// Bir satırın indekslerini (`bits` bit, MSB önce paketli) tablodan `kind` biçiminde `out`'a ekler.
fn expand_row(data: &[u8], width: usize, bits: u8, lut: &[[u8; 4]; 256], kind: Narrow, out: &mut Vec<u8>) {
    let per_byte = 8 / usize::from(bits);
    let mask = ((1u16 << bits) - 1) as u8;
    let index = |x: usize| -> usize {
        if bits == 8 {
            usize::from(data[x])
        } else {
            let shift = 8 - usize::from(bits) * (x % per_byte + 1);
            usize::from((data[x / per_byte] >> shift) & mask)
        }
    };
    for x in 0..width {
        let p = lut[index(x)];
        match kind {
            Narrow::Luma => out.push(p[0]),
            Narrow::LumaAlpha => out.extend_from_slice(&[p[0], p[3]]),
        }
    }
}

fn finish(buf: Vec<u8>, src: (u32, u32), plan: Plan, kind: Narrow) -> Result<Image, ImageError> {
    match kind {
        Narrow::Luma => resize::resize_luma(&buf, src, plan),
        Narrow::LumaAlpha => resize::resize_luma_alpha(buf, src, plan),
    }
}

fn png_error(e: png::DecodingError) -> ImageError {
    match e {
        png::DecodingError::LimitsExceeded => ImageError::Decode("sınır aşıldı: png".into()),
        other => ImageError::Decode(other.to_string()),
    }
}
