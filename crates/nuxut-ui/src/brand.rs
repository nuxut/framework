//! Marka işareti: Kara Sağan (kullanıcının seçtiği logo; slogan "The penguin can't fly. This
//! one can."). Kaynak: `assets/brand/kara-sagan.svg` (renkli, karolu uygulama ikonu) ve
//! `kara-sagan-mono.svg` (tek path, `currentColor`); geometri ekibin logo turundan aynen
//! kopyalanmıştır, burada değiştirilmez (gerekçe: `.team/logo/r3/10-kara-sagan.md`).
//!
//! **Renk temadan gelir.** SVG'ler derlemeye gömülüdür (kendi varlığımız, `Trust::System`
//! karşılığı: yalıtım gerekmez) ve yalnızca **alfa maskesi** olarak rasterleştirilir; renk
//! çizimde `ImageStyle::tint` ile verilir (çıktı = renk × maske alfası). Böylece:
//! - [`BrandStyle::Mono`] tek maske, bir tema rolüyle boyanır (varsayılan `on_surface`,
//!   [`BrandMark::accent`] → `primary`).
//! - [`BrandStyle::Duotone`] renkli sürümün iki düzlemi temadan türetilir: önce tüm siluet
//!   [`brand_shade`]`(primary)` ile, sonra ön kanat `primary` ile üstüne. Renkli SVG'deki
//!   üç renk varsayılan paletin rolleridir (zemin `surface` #12141a, ön kanat `primary`
//!   #9ec5ff, gövde = primary'nin koyu tonu #5d8fe8 — HSV'de ton +3°, doygunluk 1'e doğru %35, değer
//!   ×0,91); ikinci maske siluetin ÜSTÜNE çizildiğinden iki path'in ortak kenarında dikiş yok.
//! - [`BrandStyle::Tile`] uygulama ikonu görünümü: rol (varsayılan `surface`) rengi yuvarlatılmış karo (yarıçap
//!   SVG'deki `rx`), içinde 0,86 ölçekli ikili ton işaret (SVG'deki grup dönüşümü).
//!
//! Tema/kip değişince yeniden raster yok: yalnızca hasar işaretlenir, sonraki çizim yeni
//! rolleri okur. Maske anahtarı (parça, fiziksel kenar): mantıksal boyut × ölçek yuvarlanır,
//! çizim dikdörtgeni fiziksel piksele hizalanır (kesirli ölçekte de net). CPU maskeleri süreç
//! genelinde küçük bir LRU'da paylaşılır ([`brand_mask`]); GPU dokuları bileşen başınadır
//! (doku renderer'a bağlı). Canlanma yok: boşta kare istenmez (yalnızca ilk yükleme kare
//! bütçesine takılırsa bir kare daha).

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_image::{Fit, TargetSize, render_svg};
use nuxut_render::{ImageData, ImageFit, ImageStyle, Painter, RectStyle, Texture, TextureOptions};
use nuxut_theme::{ColorRole, Palette, Theme};

use crate::style::Hsv;
use crate::ui::{DamageState, Ui};

/// Renkli marka SVG'si (karolu uygulama ikonu; sabit renkli dış dosya — ikon dışa aktarımı).
pub const BRAND_SVG: &[u8] = include_bytes!("../assets/brand/kara-sagan.svg");
/// Tek renkli marka SVG'si (tek path, `currentColor`; `-symbolic` ikonu).
pub const BRAND_SYMBOLIC_SVG: &[u8] = include_bytes!("../assets/brand/kara-sagan-mono.svg");

/// Varsayılan mantıksal kenar (px).
pub const DEFAULT_BRAND_SIZE: f32 = 48.0;
/// Maske rasterinin en büyük fiziksel kenarı (daha büyük çizim ölçeklenir).
pub const MAX_BRAND_MASK_SIDE: u32 = 2048;
/// Süreç genelindeki CPU maske önbelleğinin girdi sınırı (LRU).
pub const BRAND_MASK_CACHE: usize = 16;

/// Gövde tonu: `primary`'nin tonu bu kadar (derece) döner…
const SHADE_HUE: f32 = 3.0;
/// …doygunluğu 1'e doğru bu oranda artar…
const SHADE_SATURATION: f32 = 0.35;
/// …ve değeri bu oranla çarpılır (varsayılan palette #9ec5ff → ≈#5d8fe8).
const SHADE_VALUE: f32 = 0.91;

/// İşaretin çizim biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrandStyle {
    /// Tek renk: rolün rengiyle boyanmış siluet.
    Mono(ColorRole),
    /// İki ton: siluet [`brand_shade`]`(primary)`, ön kanat `primary`.
    Duotone,
    /// Uygulama ikonu: rol renginde karo (ikondaki `surface`) + 0,86 ölçekli ikili ton işaret.
    /// Karo `surface` zemin üstünde görünmez; kart/panel içinde ya da başka rolle kullanın.
    Tile(ColorRole),
}

impl Default for BrandStyle {
    fn default() -> Self {
        Self::Mono(ColorRole::OnSurface)
    }
}

/// Rasterleştirilen alfa maskesi parçası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskPart {
    /// Tüm siluet (= mono SVG).
    Silhouette,
    /// Yalnızca ön kanat (renkli SVG'nin ikinci path'i).
    FrontWing,
    /// Karo içindeki siluet (SVG'nin grup dönüşümüyle).
    TileSilhouette,
    /// Karo içindeki ön kanat.
    TileFrontWing,
}

impl MaskPart {
    fn inset(self) -> bool {
        matches!(self, Self::TileSilhouette | Self::TileFrontWing)
    }

    fn front_only(self) -> bool {
        matches!(self, Self::FrontWing | Self::TileFrontWing)
    }
}

/// Çizim katmanı (alttan üste).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Layer {
    /// Yuvarlatılmış karo zemini.
    Tile,
    Mask(MaskPart),
}

/// Gövde/arka kanat tonu: `primary`'den türetilir (renkli SVG'deki koyu ton).
pub fn brand_shade(primary: Color) -> Color {
    let mut hsv = Hsv::from_color(primary);
    hsv.h = (hsv.h + SHADE_HUE).rem_euclid(360.0);
    hsv.s += (1.0 - hsv.s) * SHADE_SATURATION;
    hsv.v *= SHADE_VALUE;
    hsv.to_color()
}

/// Biçimin katmanları ve renkleri (alttan üste). Renkler her çizimde paletten okunur.
fn plan(style: BrandStyle, colors: &Palette) -> [Option<(Layer, Color)>; 3] {
    match style {
        BrandStyle::Mono(role) => [Some((Layer::Mask(MaskPart::Silhouette), colors.get(role))), None, None],
        BrandStyle::Duotone => [
            Some((Layer::Mask(MaskPart::Silhouette), brand_shade(colors.primary))),
            Some((Layer::Mask(MaskPart::FrontWing), colors.primary)),
            None,
        ],
        BrandStyle::Tile(background) => [
            Some((Layer::Tile, colors.get(background))),
            Some((Layer::Mask(MaskPart::TileSilhouette), brand_shade(colors.primary))),
            Some((Layer::Mask(MaskPart::TileFrontWing), colors.primary)),
        ],
    }
}

// --- Gömülü geometri -----------------------------------------------------------------------

/// Renkli SVG'den okunan geometri (dosya değişirse kod değişmez).
#[derive(Debug)]
struct Geometry {
    view_box: String,
    /// Gövde + arka kanat + kuyruk + baş.
    body: String,
    front: String,
    /// Karo köşe yarıçapı / kenar.
    tile_radius: f32,
    /// Karo içindeki grup dönüşümü.
    inset: String,
}

/// `name="…"` özniteliklerinin değerleri, belge sırasıyla.
fn attr_values<'a>(svg: &'a str, name: &str) -> Vec<&'a str> {
    let needle = format!(" {name}=\"");
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(i) = rest.find(&needle) {
        rest = &rest[i + needle.len()..];
        let Some(end) = rest.find('"') else { break };
        out.push(&rest[..end]);
        rest = &rest[end..];
    }
    out
}

fn parse_geometry(svg: &str) -> Option<Geometry> {
    let view_box = attr_values(svg, "viewBox").first()?.to_string();
    let side: f32 = view_box.split_whitespace().nth(2)?.parse().ok()?;
    let rx: f32 = attr_values(svg, "rx").first()?.parse().ok()?;
    let inset = attr_values(svg, "transform").first()?.to_string();
    let d = attr_values(svg, "d");
    let [body, front] = d.as_slice() else { return None };
    Some(Geometry { view_box, body: (*body).to_owned(), front: (*front).to_owned(), tile_radius: rx / side, inset })
}

fn geometry() -> Option<&'static Geometry> {
    static G: OnceLock<Option<Geometry>> = OnceLock::new();
    G.get_or_init(|| {
        let g = std::str::from_utf8(BRAND_SVG).ok().and_then(parse_geometry);
        if g.is_none() {
            tracing::warn!("gömülü marka SVG'si okunamadı; işaret çizilmeyecek");
        }
        g
    })
    .as_ref()
}

/// Parçanın tek path'li SVG'si (dolgu varsayılan: yalnızca alfa kullanılır).
fn part_svg(g: &Geometry, part: MaskPart) -> String {
    let d = if part.front_only() { g.front.clone() } else { format!("{} {}", g.body, g.front) };
    let (open, close) = if part.inset() { (format!("<g transform=\"{}\">", g.inset), "</g>") } else { (String::new(), "") };
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{}\">{open}<path d=\"{d}\"/>{close}</svg>", g.view_box)
}

// --- Maske önbelleği -------------------------------------------------------------------------

type MaskKey = (MaskPart, u32);

fn mask_cache() -> &'static Mutex<Vec<(MaskKey, ImageData)>> {
    static C: OnceLock<Mutex<Vec<(MaskKey, ImageData)>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::with_capacity(BRAND_MASK_CACHE)))
}

/// Parçanın `side_px` × `side_px` (1…[`MAX_BRAND_MASK_SIDE`]) premultiplied alfa maskesi
/// (RGB = 0). Süreç genelinde (parça, kenar) anahtarlı LRU'dan; pikseller `Arc` ile paylaşılır.
pub fn brand_mask(part: MaskPart, side_px: u32) -> Option<ImageData> {
    let side = side_px.clamp(1, MAX_BRAND_MASK_SIDE);
    let key = (part, side);
    {
        let mut cache = mask_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
            let hit = cache.remove(i);
            let data = hit.1.clone();
            cache.push(hit);
            return Some(data);
        }
    }
    let svg = part_svg(geometry()?, part);
    let img = match render_svg(svg.as_bytes(), TargetSize::new(side, side, Fit::Contain)) {
        Ok(img) => img,
        Err(e) => {
            tracing::warn!("marka maskesi rasterleştirilemedi: {e}");
            return None;
        }
    };
    let (width, height) = img.size();
    let data = ImageData { width, height, pixels: Arc::from(img.into_pixels()) };
    let mut cache = mask_cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(i) = cache.iter().position(|(k, _)| *k == key) {
        // Başka bir thread aynı anda üretti: onunkini kullan.
        return Some(cache[i].1.clone());
    }
    if cache.len() >= BRAND_MASK_CACHE {
        cache.remove(0);
    }
    cache.push((key, data.clone()));
    Some(data)
}

// --- Bileşen ---------------------------------------------------------------------------------

/// Marka işareti bileşeni. `new()` → `with_*` → `set_rect` → `draw`; etkileşimsiz.
#[derive(Debug, Clone)]
pub struct BrandMark {
    style: BrandStyle,
    /// Mantıksal kenar.
    size: f32,
    rect: Rect,
    /// Bu bileşenin GPU dokuları: (parça, fiziksel kenar, doku).
    textures: Vec<(MaskPart, u32, Texture)>,
    /// Son çizimde hazır olmayan doku vardı (bir kare daha gerekir).
    pending: bool,
    damage: DamageState,
}

impl Default for BrandMark {
    fn default() -> Self {
        Self::new()
    }
}

impl BrandMark {
    /// Tek renkli, `on_surface`, [`DEFAULT_BRAND_SIZE`].
    pub fn new() -> Self {
        Self { style: BrandStyle::default(), size: DEFAULT_BRAND_SIZE, rect: Rect::ZERO, textures: Vec::new(), pending: false, damage: DamageState::new() }
    }

    /// Son `draw`'da GPU'ya henüz inmemiş doku vardı: çağıran bir kare daha istemeli
    /// (`draw` kendisi de `request_frame` çağırır; bu, kendi kare kararını veren sahipler için).
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Tek renk, verilen rol.
    pub fn with_role(self, role: ColorRole) -> Self {
        self.with_style(BrandStyle::Mono(role))
    }

    /// Vurgu: tek renk `primary`.
    pub fn accent(self) -> Self {
        self.with_role(ColorRole::Primary)
    }

    /// İki tonlu (renkli sürümün temadan türetilmişi).
    pub fn duotone(self) -> Self {
        self.with_style(BrandStyle::Duotone)
    }

    /// Karolu uygulama ikonu görünümü (karo `surface`, dışa aktarılan ikonla aynı).
    pub fn tile(self) -> Self {
        self.tile_with(ColorRole::Surface)
    }

    /// Karolu görünüm, karo verilen rol renginde.
    pub fn tile_with(self, background: ColorRole) -> Self {
        self.with_style(BrandStyle::Tile(background))
    }

    pub fn with_style(mut self, style: BrandStyle) -> Self {
        self.set_style(style);
        self
    }

    /// Mantıksal kenar (px; sonlu ve > 0 olmalı, değilse yok sayılır).
    pub fn with_size(mut self, size: f32) -> Self {
        self.set_size(size);
        self
    }

    pub fn style(&self) -> BrandStyle {
        self.style
    }

    pub fn size(&self) -> f32 {
        self.size
    }

    pub fn set_style(&mut self, style: BrandStyle) {
        if self.style != style {
            self.style = style;
            // Gerekmeyen parçaların dokuları bırakılır (Mono ↔ Duotone silueti paylaşır).
            let parts: Vec<MaskPart> = plan(style, &Palette::default())
                .into_iter()
                .flatten()
                .filter_map(|(l, _)| match l {
                    Layer::Mask(p) => Some(p),
                    Layer::Tile => None,
                })
                .collect();
            self.textures.retain(|(p, _, _)| parts.contains(p));
            self.damage.mark();
        }
    }

    pub fn set_size(&mut self, size: f32) {
        if size.is_finite() && size > 0.0 && size != self.size {
            self.size = size;
            self.damage.mark();
        }
    }

    pub fn preferred_size(&self) -> Size {
        Size::new(self.size, self.size)
    }

    /// İşaret `rect`'in ortasına `size` kenarlı kare olarak çizilir (boş `rect`: sol-üst köşesine).
    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    /// Tema/kip değişimi: yeniden raster yok, sonraki çizim yeni rolleri okur.
    pub fn set_theme(&mut self, _theme: &Theme, _now: Instant) {
        self.damage.mark();
    }

    /// Fiziksel piksele hizalı çizim dikdörtgeni ve fiziksel kenar.
    pub fn device_rect(&self, scale: f32) -> (Rect, u32) {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let side_px = (self.size * scale).round().clamp(1.0, MAX_BRAND_MASK_SIDE as f32);
        let side = side_px / scale;
        let r = self.rect;
        let origin = if r.width > 0.0 && r.height > 0.0 {
            Point::new(r.x + (r.width - side) * 0.5, r.y + (r.height - side) * 0.5)
        } else {
            Point::new(r.x, r.y)
        };
        let x = (origin.x * scale).round() / scale;
        let y = (origin.y * scale).round() / scale;
        (Rect::new(x, y, side, side), side_px as u32)
    }

    /// Parçanın bu boydaki dokusu (yoksa maske yüklenir; aynı parçanın eski boyu bırakılır).
    fn texture(&mut self, painter: &mut Painter, part: MaskPart, side_px: u32) -> Option<Texture> {
        if let Some((_, _, t)) = self.textures.iter().find(|(p, s, _)| *p == part && *s == side_px) {
            return Some(t.clone());
        }
        let mask = brand_mask(part, side_px)?;
        let tex = painter.upload_texture(mask, TextureOptions::default());
        self.textures.retain(|(p, _, _)| *p != part);
        self.textures.push((part, side_px, tex.clone()));
        Some(tex)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let (r, side_px) = self.device_rect(ui.painter.scale());
        let mut pending = false;
        for (layer, color) in plan(self.style, &ui.theme.colors).into_iter().flatten() {
            let color = ui.fade(color);
            match layer {
                Layer::Tile => {
                    let radius = geometry().map_or(0.0, |g| g.tile_radius) * r.width;
                    ui.painter.rect(r, &RectStyle::filled(color, Corners::uniform(radius)));
                }
                Layer::Mask(part) => {
                    let Some(tex) = self.texture(ui.painter, part, side_px) else { continue };
                    pending |= !tex.is_ready();
                    ui.painter.image(r, &tex, &ImageStyle::new(ImageFit::Fill).with_tint(color));
                }
            }
        }
        // İlk yükleme kare bütçesine takıldıysa doku hazır olana dek bir kare daha.
        self.pending = pending;
        self.damage.report(ui, r, pending);
        if pending {
            ui.request_frame();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(img: &ImageData, x: u32, y: u32) -> u8 {
        img.pixels[((y * img.width + x) * 4 + 3) as usize]
    }

    fn mono_d() -> String {
        let svg = std::str::from_utf8(BRAND_SYMBOLIC_SVG).unwrap();
        attr_values(svg, "d")[0].to_owned()
    }

    #[test]
    fn embedded_geometry_parses_and_matches_mono() {
        let g = geometry().expect("gömülü renkli SVG ayrıştırılmalı");
        assert_eq!(g.view_box, "0 0 512 512");
        assert!((g.tile_radius - 116.0 / 512.0).abs() < 1e-6);
        assert!(g.inset.contains("scale(.86)"));
        // Renkli SVG'nin iki path'i = mono SVG'nin tek path'indeki iki alt path (aynı geometri).
        assert_eq!(format!("{} {}", g.body, g.front), mono_d());
        // Mono dosya tema rengine açık: tek path, currentColor.
        let mono = std::str::from_utf8(BRAND_SYMBOLIC_SVG).unwrap();
        assert_eq!(attr_values(mono, "fill"), ["currentColor"]);
    }

    #[test]
    fn embedded_svgs_decode() {
        for bytes in [BRAND_SVG, BRAND_SYMBOLIC_SVG] {
            let img = render_svg(bytes, TargetSize::new(64, 64, Fit::Contain)).expect("gömülü SVG çözülmeli");
            assert_eq!(img.size(), (64, 64));
        }
    }

    #[test]
    fn mask_sizes_follow_logical_size_and_scale() {
        for (size, scale, want) in [(16.0, 1.0, 16), (24.0, 1.0, 24), (48.0, 1.0, 48), (128.0, 1.0, 128), (16.0, 1.5, 24), (24.0, 1.5, 36), (48.0, 1.5, 72), (128.0, 1.5, 192), (16.0, 1.25, 20)] {
            let mark = BrandMark::new().with_size(size);
            let (r, px) = mark.device_rect(scale);
            assert_eq!(px, want, "{size}@{scale}");
            assert!((r.width * scale - want as f32).abs() < 1e-3);
            let m = brand_mask(MaskPart::Silhouette, px).unwrap();
            assert_eq!((m.width, m.height), (want, want));
        }
    }

    #[test]
    fn device_rect_is_pixel_aligned_and_centered() {
        let mut mark = BrandMark::new().with_size(24.0);
        mark.set_rect(Rect::new(10.3, 7.1, 100.0, 50.0));
        let (r, _) = mark.device_rect(1.5);
        for v in [r.x, r.y] {
            let px = v * 1.5;
            assert!((px - px.round()).abs() < 1e-3, "{v}");
        }
        assert!((r.center().x - 60.3).abs() < 1.0 && (r.center().y - 32.1).abs() < 1.0);
    }

    #[test]
    fn mask_is_alpha_only_and_shaped() {
        let m = brand_mask(MaskPart::Silhouette, 512).unwrap();
        // Köşe boş, ön kanadın içi dolu (viewBox koordinatı = piksel).
        assert_eq!(alpha(&m, 2, 2), 0);
        assert_eq!(alpha(&m, 360, 250), 255);
        assert!(m.pixels.chunks_exact(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
        // Ön kanat parçası gövdeyi içermez; karo parçası 0,86 ölçekli (ortaya doğru çekilir).
        let front = brand_mask(MaskPart::FrontWing, 512).unwrap();
        assert_eq!(alpha(&front, 360, 250), 255);
        assert_eq!(alpha(&front, 150, 100), 0);
        assert_eq!(alpha(&m, 150, 100), 255);
        let tile = brand_mask(MaskPart::TileSilhouette, 512).unwrap();
        let inside = |x: f32, y: f32| (x * 0.86 + 35.84, y * 0.86 + 35.84);
        let (tx, ty) = inside(360.0, 250.0);
        assert_eq!(alpha(&tile, tx as u32, ty as u32), 255);
    }

    #[test]
    fn mask_cache_shares_and_is_bounded() {
        let a = brand_mask(MaskPart::FrontWing, 33).unwrap();
        let b = brand_mask(MaskPart::FrontWing, 33).unwrap();
        assert!(Arc::ptr_eq(&a.pixels, &b.pixels), "aynı anahtar yeniden rasterleşmemeli");
        for side in 900..900 + BRAND_MASK_CACHE as u32 + 2 {
            brand_mask(MaskPart::Silhouette, side / 8).unwrap();
        }
        assert!(mask_cache().lock().unwrap().len() <= BRAND_MASK_CACHE);
        // Sınır dışı kenar kırpılır.
        let big = brand_mask(MaskPart::Silhouette, u32::MAX).unwrap();
        assert_eq!(big.width, MAX_BRAND_MASK_SIDE);
    }

    #[test]
    fn mono_tint_follows_role_and_palette() {
        let dark = Palette::default();
        let light = Palette::light();
        let colors = |style, p: &Palette| plan(style, p).into_iter().flatten().collect::<Vec<_>>();
        let m = colors(BrandMark::new().style(), &dark);
        assert_eq!(m, [(Layer::Mask(MaskPart::Silhouette), dark.on_surface)]);
        assert_eq!(colors(BrandMark::new().accent().style(), &dark)[0].1, dark.primary);
        assert_eq!(colors(BrandMark::new().with_role(ColorRole::Error).style(), &light)[0].1, light.error);
        // Kip değişimi: aynı maske (aynı katman), yeni renk — yeniden raster gerekmez.
        let l = colors(BrandMark::new().style(), &light);
        assert_eq!(l[0].0, m[0].0);
        assert_eq!(l[0].1, light.on_surface);
    }

    #[test]
    fn duotone_and_tile_derive_from_palette() {
        let dark = Palette::default();
        let d: Vec<_> = plan(BrandStyle::Duotone, &dark).into_iter().flatten().collect();
        assert_eq!(d[0], (Layer::Mask(MaskPart::Silhouette), brand_shade(dark.primary)));
        assert_eq!(d[1], (Layer::Mask(MaskPart::FrontWing), dark.primary));
        assert_eq!(BrandMark::new().tile().style(), BrandStyle::Tile(ColorRole::Surface));
        let t: Vec<_> = plan(BrandMark::new().tile().style(), &dark).into_iter().flatten().collect();
        assert_eq!(t[0], (Layer::Tile, dark.surface));
        let t: Vec<_> = plan(BrandMark::new().tile_with(ColorRole::SurfaceBright).style(), &dark).into_iter().flatten().collect();
        assert_eq!(t[0], (Layer::Tile, dark.surface_bright));
        assert_eq!(t[1].0, Layer::Mask(MaskPart::TileSilhouette));
        // Varsayılan palette gövde tonu renkli SVG'deki koyu tona (#5d8fe8) yakın.
        let svg = std::str::from_utf8(BRAND_SVG).unwrap();
        let body = attr_values(svg, "fill")[1];
        let want = Color::from_hex(body).unwrap();
        let got = brand_shade(dark.primary);
        for (a, b) in [(got.r, want.r), (got.g, want.g), (got.b, want.b)] {
            assert!((a - b).abs() < 3.0 / 255.0, "{got:?} ≠ {want:?}");
        }
        // Ön kanat ve zemin de SVG'dekilerle aynı rol renkleri.
        assert_eq!(Color::from_hex(attr_values(svg, "fill")[2]).unwrap(), dark.primary);
        assert_eq!(Color::from_hex(attr_values(svg, "fill")[0]).unwrap(), dark.surface);
    }

    #[test]
    fn style_change_drops_unneeded_textures_and_marks_damage() {
        let mut mark = BrandMark::new();
        mark.damage = DamageState::default();
        assert!(!mark.damage.is_dirty());
        mark.set_style(BrandStyle::Duotone);
        assert!(mark.damage.is_dirty());
        assert_eq!(mark.style(), BrandStyle::Duotone);
        mark.set_size(f32::NAN);
        assert_eq!(mark.size(), DEFAULT_BRAND_SIZE);
    }

    #[test]
    fn pending_texture_is_reported() {
        use nuxut_render::{Recorder, TextSystem};
        let theme = Theme::default();
        let mut mark = BrandMark::new().with_size(24.0);
        assert!(!mark.is_pending());
        mark.set_rect(Rect::new(0.0, 0.0, 32.0, 32.0));
        let mut text = TextSystem::new();
        let mut wants = false;
        // Kayıtçıda doku GL'e hiç inmez: hep bekliyor.
        Recorder::new().record(&mut text, (64, 64), 1.0, |p| {
            let mut ui = Ui::new(p, &theme, Instant::now());
            mark.draw(&mut ui);
            wants = ui.wants_frame();
        });
        assert!(mark.is_pending());
        assert!(wants);
    }
}
