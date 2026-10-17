//! Glyph atlas'ı: raf (shelf) paketleyici + GL dokusu. Maske glyph'leri tek kanallı (R8),
//! renkli glyph'ler (emoji) RGBA8 ayrı bir dokuda tutulur.

use std::collections::HashMap;

use cosmic_text::{CacheKey, SwashContent};
use glow::HasContext;

use crate::error::RenderError;
use crate::text::TextSystem;

pub(crate) const INITIAL_SIZE: u32 = 512;
pub(crate) const MAX_SIZE: u32 = 4096;
/// Glyph'ler arası boşluk (doğrusal örneklemede komşu taşmasın).
const GAP: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Mask,
    Color,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    pub kind: Kind,
    /// Atlas içindeki piksel konumu.
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Glyph orijinine göre bitmap'in sol-üst köşesi (y yukarı pozitif, swash düzeni).
    pub left: i32,
    pub top: i32,
}

/// Raf paketleyici. Saf mantık; GL bilmez (test edilebilir).
#[derive(Debug)]
pub(crate) struct ShelfPacker {
    size: u32,
    shelves: Vec<Shelf>,
    next_y: u32,
}

#[derive(Debug)]
struct Shelf {
    y: u32,
    height: u32,
    x: u32,
}

impl ShelfPacker {
    pub fn new(size: u32) -> Self {
        Self { size, shelves: Vec::new(), next_y: 0 }
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        let (w, h) = (w + GAP, h + GAP);
        if w > self.size || h > self.size {
            return None;
        }
        // En az boşa giden raf: yükseklik yeterli ama çok fazla değil.
        let mut best: Option<usize> = None;
        for (i, s) in self.shelves.iter().enumerate() {
            let fits = s.height >= h && s.height <= h + h / 2 + 2 && self.size - s.x >= w;
            if fits && best.is_none_or(|b| self.shelves[b].height > s.height) {
                best = Some(i);
            }
        }
        if let Some(i) = best {
            let s = &mut self.shelves[i];
            let pos = (s.x, s.y);
            s.x += w;
            return Some(pos);
        }
        if self.size - self.next_y < h {
            return None;
        }
        let y = self.next_y;
        self.next_y += h;
        self.shelves.push(Shelf { y, height: h, x: w });
        Some((0, y))
    }
}

#[derive(Debug)]
pub(crate) struct Page {
    pub texture: glow::Texture,
    packer: ShelfPacker,
    kind: Kind,
}

impl Page {
    fn new(gl: &glow::Context, kind: Kind, size: u32) -> Result<Self, RenderError> {
        // SAFETY: context güncel; doku boyutu MAX_SIZE ile sınırlı, veri yok (NULL ile
        // ayrılır), parametreler geçerli GLES 3.0 sabitleri.
        let texture = unsafe {
            let tex = gl.create_texture().map_err(RenderError::Gl)?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            let (internal, format) = match kind {
                Kind::Mask => (glow::R8, glow::RED),
                Kind::Color => (glow::RGBA8, glow::RGBA),
            };
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                internal as i32,
                size as i32,
                size as i32,
                0,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            tex
        };
        Ok(Self { texture, packer: ShelfPacker::new(size), kind })
    }

    pub fn size(&self) -> u32 {
        self.packer.size()
    }

    fn destroy(&self, gl: &glow::Context) {
        // SAFETY: doku bu sayfaya aittir ve başka yerde tutulmaz.
        unsafe { gl.delete_texture(self.texture) };
    }
}

/// Atlas dolu: çağıran o sayfayı büyütüp (ya da temizleyip) yeniden dener.
#[derive(Debug)]
pub(crate) struct Full(pub Kind);

/// Maske ve renk sayfaları **bağımsız** büyür; renk sayfası (RGBA8, 4 B/px) yalnızca ilk renkli
/// glyph (emoji) geldiğinde ayrılır. Önceden iki sayfa birlikte büyüyordu: yalnız metin
/// ağırlıklı bir yüzeyde maske 4096'ya çıkınca hiç kullanılmayan renk sayfası da 4096² × 4 =
/// 64 MB oluyordu (perf raporu 2026-10, shell docs/research/perf-2026-10.md).
#[derive(Debug)]
pub(crate) struct GlyphAtlas {
    pub mask: Page,
    /// İlk renkli glyph'e dek yok.
    pub color: Option<Page>,
    /// `None` = boş glyph (boşluk vb.), tekrar rasterleştirilmez.
    entries: HashMap<CacheKey, Option<Entry>>,
}

impl GlyphAtlas {
    pub fn new(gl: &glow::Context) -> Result<Self, RenderError> {
        Ok(Self { mask: Page::new(gl, Kind::Mask, INITIAL_SIZE)?, color: None, entries: HashMap::new() })
    }

    /// Renk sayfasının ilk boyutu (emoji az; maskeden küçük başlar).
    const COLOR_INITIAL: u32 = INITIAL_SIZE / 2;

    pub fn page_size(&self, kind: Kind) -> u32 {
        match kind {
            Kind::Mask => self.mask.size(),
            Kind::Color => self.color.as_ref().map_or(0, Page::size),
        }
    }

    pub fn color_texture(&self) -> Option<glow::Texture> {
        self.color.as_ref().map(|p| p.texture)
    }

    pub fn get_or_insert(
        &mut self,
        gl: &glow::Context,
        text: &mut TextSystem,
        key: CacheKey,
    ) -> Result<Option<Entry>, Full> {
        if let Some(e) = self.entries.get(&key) {
            return Ok(*e);
        }
        let Some(image) = text.rasterize(key) else {
            self.entries.insert(key, None);
            return Ok(None);
        };
        let (w, h) = (image.placement.width, image.placement.height);
        if w == 0 || h == 0 {
            self.entries.insert(key, None);
            return Ok(None);
        }
        let (kind, data) = match image.content {
            SwashContent::Mask => (Kind::Mask, image.data),
            // Alt piksel maskesi istemiyoruz ama gelirse renkli gibi davran.
            SwashContent::Color | SwashContent::SubpixelMask => (Kind::Color, premultiply(image.data)),
        };
        let expected = (w * h) as usize * if kind == Kind::Mask { 1 } else { 4 };
        if data.len() < expected {
            tracing::warn!("glyph verisi eksik ({} < {expected}); atlandı", data.len());
            self.entries.insert(key, None);
            return Ok(None);
        }
        let page = match kind {
            Kind::Mask => &mut self.mask,
            Kind::Color => match &mut self.color {
                Some(p) => p,
                slot @ None => match Page::new(gl, Kind::Color, Self::COLOR_INITIAL) {
                    Ok(p) => {
                        tracing::debug!("renkli glyph sayfası ayrıldı ({}px)", Self::COLOR_INITIAL);
                        slot.insert(p)
                    }
                    Err(e) => {
                        tracing::warn!("renkli glyph sayfası ayrılamadı: {e}");
                        self.entries.insert(key, None);
                        return Ok(None);
                    }
                },
            },
        };
        let (x, y) = page.packer.allocate(w, h).ok_or(Full(kind))?;
        let format = if page.kind == Kind::Mask { glow::RED } else { glow::RGBA };
        // SAFETY: (x, y, w, h) paketleyiciden gelir ve doku sınırları içindedir; `data`
        // uzunluğu yukarıda w*h*bpp'ye karşı doğrulandı; UNPACK_ALIGNMENT 1.
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(page.texture));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                x as i32,
                y as i32,
                w as i32,
                h as i32,
                format,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&data[..expected])),
            );
        }
        let entry = Entry { kind, x, y, width: w, height: h, left: image.placement.left, top: image.placement.top };
        self.entries.insert(key, Some(entry));
        Ok(Some(entry))
    }

    /// Dolu `kind` sayfasını (mümkünse iki kat büyüterek) sıfırdan kurar; yalnız o sayfanın
    /// girdileri düşer, öbür sayfa ve girdileri olduğu gibi kalır. Büyüyemezse `false` (aynı
    /// boyutta temizlenmiş olarak kalır).
    pub fn grow(&mut self, gl: &glow::Context, kind: Kind) -> Result<bool, RenderError> {
        let old = self.page_size(kind).max(1);
        let size = (old * 2).min(MAX_SIZE);
        let grown = old < MAX_SIZE;
        let page = Page::new(gl, kind, size)?;
        let slot = match kind {
            Kind::Mask => Some(std::mem::replace(&mut self.mask, page)),
            Kind::Color => self.color.replace(page),
        };
        if let Some(old) = slot {
            old.destroy(gl);
        }
        self.entries.retain(|_, e| e.is_none_or(|e| e.kind != kind));
        tracing::debug!("glyph atlas'ı {kind:?} sayfası {size}px oldu");
        Ok(grown)
    }

    pub fn destroy(&self, gl: &glow::Context) {
        self.mask.destroy(gl);
        if let Some(c) = &self.color {
            c.destroy(gl);
        }
    }
}

fn premultiply(mut rgba: Vec<u8>) -> Vec<u8> {
    for px in rgba.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_without_overlap_until_full() {
        let mut p = ShelfPacker::new(64);
        let mut rects = Vec::new();
        while let Some((x, y)) = p.allocate(10, 12) {
            rects.push((x, y));
            assert!(x + 10 <= 64 && y + 12 <= 64);
        }
        // 64 / 11 = 5 sütun, 64 / 13 = 4 raf.
        assert_eq!(rects.len(), 20);
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                let overlap = a.0 < b.0 + 11 && b.0 < a.0 + 11 && a.1 < b.1 + 13 && b.1 < a.1 + 13;
                assert!(!overlap, "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn rejects_oversized() {
        assert!(ShelfPacker::new(32).allocate(40, 4).is_none());
    }

    #[test]
    fn premultiplies() {
        assert_eq!(premultiply(vec![255, 128, 0, 128]), vec![128, 64, 0, 128]);
    }
}
