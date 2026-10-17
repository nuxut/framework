//! Maskot: **Fiyu** ("Geniş" oranı; ad ve oran kullanıcı kararı, 2026-10-13). Yedi ruh hâli
//! ([`Mood`]); kaynak `.team/mascot/genis/`, burada `assets/mascot/` altında İngilizce adlarla
//! aynen kopyalanmıştır. Yerleşim kuralı ve gerekçe: `docs/design/mascot.md`.
//!
//! **Renk temadan gelir.** İki tonlu SVG'lerde renk yoktur, yalnızca sınıf vardır
//! (`m-body`, `m-wing-front`, `m-detail`, `m-beak`, isteğe bağlı `m-wing-back`). Çizimde sınıflar
//! [`mascot_colors`] ile tema rollerinden türetilen renklere çevrilir: SVG metninde her
//! `class="…"` özniteliği `fill="#rrggbb"` ile değiştirilir ([`paint_svg`]; bilinmeyen sınıf →
//! çizim yok, sabit hex yok). Gömülü, derlemeye dahil kendi varlığımız olduğundan (`Trust::System`
//! karşılığı) süreç içinde nuxut-image'in SVG yolunda (resvg) rasterleşir. Tek renkli
//! (`-mono`) sürüm `currentColor` ile yazılmıştır: alfa maskesi olarak bir kez rasterleşir,
//! rengi çizimde `ImageStyle::tint` verir (tema değişince yeniden raster yok).
//!
//! Raster fiziksel boyuttadır (mantıksal kenar × ölçek, piksele hizalı). Süreç genelinde
//! (mood, biçim, fiziksel kenar, renkler) anahtarlı küçük bir LRU paylaşılır
//! ([`mascot_image`]); bileşen yalnızca son dokusunu tutar. Tema/ölçek/boyut/mood değişmedikçe
//! yeniden raster ve yükleme yoktur; canlanma yoktur (boşta 0 CPU, azaltılmış hareket dostu).
//!
//! Gizleme: `framework.toml [appearance] mascot = false` ([`Theme::appearance`]) iken
//! [`Mascot::measure`] sıfır döner ve [`Mascot::draw`] hiçbir şey çizmez; değişim
//! `theme_changed` → [`Mascot::set_theme`] yoluyla canlı uygulanır.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_image::{Fit, TargetSize, render_svg};
use nuxut_render::{ImageData, ImageFit, ImageStyle, Painter, Texture, TextureOptions};
use nuxut_theme::contrast::{NON_TEXT_MIN, ensure_contrast, relative_luminance};
use nuxut_theme::{ColorRole, Palette, Theme};

use crate::brand::brand_shade;
use crate::ui::{DamageState, Ui};

/// En küçük mantıksal kenar (px). Altında yüz okunmaz (README "Testler").
pub const MASCOT_MIN_SIZE: f32 = 48.0;
/// En büyük mantıksal kenar (px).
pub const MASCOT_MAX_SIZE: f32 = 160.0;
/// Varsayılan mantıksal kenar (px).
pub const DEFAULT_MASCOT_SIZE: f32 = 96.0;
/// Süreç genelindeki raster önbelleğinin girdi sınırı (LRU).
pub const MASCOT_CACHE: usize = 24;
/// Rasterin en büyük fiziksel kenarı (160 × 4 ölçek ile sınır).
const MAX_SIDE_PX: u32 = 640;
/// Açık kipte ön kanadın yüzeye karşı hedef karşıtlığı: `primary`'nin HCT tonu bu orana dek
/// açılır (hue/kroma korunur) — zeminde okunan en açık ton (3:1 + 8 bit nicemleme payı). README
/// "Bilinen zayıflıklar": önizlemedeki açık ton (#8FB6F6 / #F5F7FC ≈ 1,9:1) zeminde soluk kalıyordu.
const LIGHT_WING_TARGET: f32 = 3.3;
/// Açık kipte ön kanat (ve göz akı) ile gövde arasındaki en az karşıtlık. Ön kanat zemine 3:1
/// için orta tonda kalmak zorunda; ayrım gövdenin koyulaşmasıyla sağlanır. 1,8:1'de göz akı
/// gövdeye karışıyordu (canlı ekran görüntüsü, 2026-10-13).
const LIGHT_WING_SEPARATION: f32 = 2.2;
/// Koyu kipte ön kanat ile gövde arasındaki en az karşıtlık (varsayılan palette marka ikili tonu
/// zaten ~1,8:1 verir; yalnızca uç paletlerde devreye girer).
const DARK_WING_SEPARATION: f32 = 1.6;

/// Maskotun ruh hâli. Nerede kullanılacağı `docs/design/mascot.md`'de.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mood {
    /// Boş durumlar: bildirim yok, panel boş, pano boş.
    Empty,
    /// Başlatıcıda sonuç yok.
    NoResults,
    /// Bağlantı yok, servis ulaşılamadı.
    Error,
    /// Hoş geldin son adımı, güncelleme bitti.
    Celebrate,
    /// Hakkında, Hoş geldin ilk adımı.
    Hello,
    /// Uzun boşta kalma, gece.
    Rest,
    /// Yükleniyor / bekleme (isteğe bağlı).
    Thinking,
}

impl Mood {
    pub const ALL: [Self; 7] = [Self::Empty, Self::NoResults, Self::Error, Self::Celebrate, Self::Hello, Self::Rest, Self::Thinking];

    /// Varlık dosyasının adı (`assets/mascot/<name>.svg`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::NoResults => "no-results",
            Self::Error => "error",
            Self::Celebrate => "celebrate",
            Self::Hello => "hello",
            Self::Rest => "rest",
            Self::Thinking => "thinking",
        }
    }

    /// İki tonlu (sınıflı, renksiz) SVG.
    pub fn svg(self) -> &'static [u8] {
        match self {
            Self::Empty => include_bytes!("../assets/mascot/empty.svg"),
            Self::NoResults => include_bytes!("../assets/mascot/no-results.svg"),
            Self::Error => include_bytes!("../assets/mascot/error.svg"),
            Self::Celebrate => include_bytes!("../assets/mascot/celebrate.svg"),
            Self::Hello => include_bytes!("../assets/mascot/hello.svg"),
            Self::Rest => include_bytes!("../assets/mascot/rest.svg"),
            Self::Thinking => include_bytes!("../assets/mascot/thinking.svg"),
        }
    }

    /// Tek renkli (`currentColor`) SVG.
    pub fn mono_svg(self) -> &'static [u8] {
        match self {
            Self::Empty => include_bytes!("../assets/mascot/empty-mono.svg"),
            Self::NoResults => include_bytes!("../assets/mascot/no-results-mono.svg"),
            Self::Error => include_bytes!("../assets/mascot/error-mono.svg"),
            Self::Celebrate => include_bytes!("../assets/mascot/celebrate-mono.svg"),
            Self::Hello => include_bytes!("../assets/mascot/hello-mono.svg"),
            Self::Rest => include_bytes!("../assets/mascot/rest-mono.svg"),
            Self::Thinking => include_bytes!("../assets/mascot/thinking-mono.svg"),
        }
    }
}

/// Çizim biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MascotStyle {
    /// İki ton (varsayılan): sınıflar [`mascot_colors`] ile boyanır.
    #[default]
    Duotone,
    /// Tek renk: siluet rolün rengiyle; yüz gövdedeki deliklerden okunur.
    Mono(ColorRole),
}

/// Sınıfların çözülmüş renkleri (opak).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MascotColors {
    /// `m-body`: gövde, kuyruk, yağmur damlası.
    pub body: Color,
    /// `m-wing-back` (aynı zamanda `m-body`): arka kanat. Şimdilik gövdeyle aynı.
    pub wing_back: Color,
    /// `m-wing-front`: ön kanat, göz akı, süs öğeleri.
    pub wing_front: Color,
    /// `m-detail`: göz bebeği, kapalı göz yayı.
    pub detail: Color,
    /// `m-beak`: gaga (gövde üstünde; gövdeye karşı ≥ 3:1).
    pub beak: Color,
}

impl MascotColors {
    fn key(&self) -> [[u8; 3]; 5] {
        [self.body, self.wing_back, self.wing_front, self.detail, self.beak].map(rgb8)
    }
}

fn rgb8(c: Color) -> [u8; 3] {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [b(c.r), b(c.g), b(c.b)]
}

/// Sınıf → renk eşlemesi (docs/design/mascot.md "Renk rolleri"). Yalnızca palet rollerinden
/// türetilir; sonuçta ön kanat ve gövde `surface` ile `surface_container`'a karşı en az
/// [`NON_TEXT_MIN`] (3:1, WCAG 1.4.11) verir, göz bebeği göz akına karşı da 3:1.
/// - **Koyu zemin** (marka işaretiyle aynı ikili ton): ön kanat `primary`, gövde
///   [`brand_shade`]`(primary)`.
/// - **Açık zemin**: ön kanat `primary`'nin HCT tonu açılmış hâli (yüzeye 3:1 garanti edilerek,
///   hue/kroma korunur); gövde `primary`, ön kanada en az 2,2:1 olacak kadar koyulaştırılmış.
/// - Ayrım her kipte: koyu zeminde ön kanat gövdeden en az 1,6:1 açık (yetmezse ön kanat açılır;
///   böylece zemin karşıtlığı bozulmaz).
/// - Detay: `surface` ile `on_surface`'tan koyusu (göz bebeği her kipte koyu; koyu kipte yüzeyle
///   aynı — monodaki "delik" yüzle tutarlı).
/// - Gaga: `surface` (monodaki delik), gövdeye en az 3:1.
///
/// Yüksek karşıtlık kipi için ayrı kural yoktur: framework politikası metin dışı öğede de 3:1'dir
/// ve palet zaten `Palette::high_contrast` ile gelir (accessibility.md §1); eşikler testlidir.
pub fn mascot_colors(p: &Palette) -> MascotColors {
    let opaque = |c: Color| c.with_alpha(1.0);
    let surfaces = [opaque(p.surface), opaque(p.surface_container)];
    let primary = opaque(p.primary);
    let dark = relative_luminance(surfaces[0]) < 0.5;
    let (front, body) = if dark {
        (primary, brand_shade(primary))
    } else {
        (light_wing(primary, &surfaces), primary)
    };
    let front = ensure_contrast(front, &surfaces, NON_TEXT_MIN);
    let body = ensure_contrast(body, &surfaces, NON_TEXT_MIN);
    let (front, body) = if dark {
        // Koyu zeminde ön kanat gövdeden açıktır: açılırken zeminden de uzaklaşır. Beyaza dayanmış
        // birincil renkte (ör. parlak yeşil) yetmezse gövde koyulaşır; aradaki pay zemine 3:1'i
        // korur (ön kanat ≥ 10:1 iken gövde ≤ 1,6 kat koyu → hâlâ zeminin çok üstünde).
        let front = ensure_contrast(front, &[body], DARK_WING_SEPARATION);
        let body = ensure_contrast(body, &[front], DARK_WING_SEPARATION);
        (front, ensure_contrast(body, &surfaces, NON_TEXT_MIN))
    } else {
        // Açık zeminde ön kanat ~3:1 sınırında orta tondadır: gövde ondan koyuya itilir (zeminden
        // de uzaklaşır, 3:1 korunur).
        (front, ensure_contrast(body, &[front], LIGHT_WING_SEPARATION))
    };
    let (s, o) = (opaque(p.surface), opaque(p.on_surface));
    let detail = if relative_luminance(s) <= relative_luminance(o) { s } else { o };
    let detail = ensure_contrast(detail, &[front], NON_TEXT_MIN);
    // Gaga gövdenin üstündedir: monodaki "delik" gibi yüzey rengi, gövdeye 3:1. Açık kipte koyu
    // gaga imkânsızdır: ön kanat zemine 3:1 için orta tonda, gövde ondan 2,2:1 koyu → siyah bile
    // gövdeye < 3:1 kalır (≈ 2:1 idi). Gövde zemine zaten ≥ 3:1 olduğundan yüzey çoğunlukla aynen
    // geçer; koyu kipte yüzey = göz bebeği, görünüm değişmez.
    let beak = ensure_contrast(s, &[body], NON_TEXT_MIN);
    MascotColors { body, wing_back: body, wing_front: front, detail, beak }
}

/// Açık zeminde ön kanat: `primary`'nin, yüzeylere [`LIGHT_WING_TARGET`] veren en açık tonu.
/// `primary` bundan koyuysa HCT tonu açılır (ensure_contrast'ın `primary`'ye karşı oranı hedef
/// parlaklıktan hesaplanır); açıksa olduğu gibi kalır (çağıran 3:1'e geri iter).
fn light_wing(primary: Color, surfaces: &[Color]) -> Color {
    let darkest_surface = surfaces.iter().map(|s| relative_luminance(*s)).fold(1.0f32, f32::min);
    let want = (darkest_surface + 0.05) / LIGHT_WING_TARGET - 0.05;
    let have = relative_luminance(primary);
    if want <= have {
        return primary;
    }
    ensure_contrast(primary, &[primary], (want + 0.05) / (have + 0.05))
}

/// İki tonlu SVG'deki her `class="…"` özniteliğini sınıfın rengiyle `fill="#rrggbb"` yapar.
/// Bilinmeyen sınıf ya da bozuk öznitelik: `None` (varlık bozulmuş; çizilmez).
pub fn paint_svg(svg: &str, colors: &MascotColors) -> Option<String> {
    const NEEDLE: &str = "class=\"";
    let mut out = String::with_capacity(svg.len() + 64);
    let mut rest = svg;
    while let Some(i) = rest.find(NEEDLE) {
        out.push_str(&rest[..i]);
        rest = &rest[i + NEEDLE.len()..];
        let end = rest.find('"')?;
        let classes: Vec<&str> = rest[..end].split_whitespace().collect();
        let color = if classes.contains(&"m-wing-back") {
            colors.wing_back
        } else {
            match classes.as_slice() {
                ["m-body"] => colors.body,
                ["m-wing-front"] => colors.wing_front,
                ["m-detail"] => colors.detail,
                ["m-beak"] => colors.beak,
                _ => return None,
            }
        };
        out.push_str("fill=\"");
        out.push_str(&color.with_alpha(1.0).to_hex());
        out.push('"');
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

// --- Raster önbelleği ------------------------------------------------------------------------

/// Önbellek anahtarı: mono maske renkten bağımsızdır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RasterKey {
    Duotone { mood: Mood, side: u32, colors: [[u8; 3]; 5] },
    Mono { mood: Mood, side: u32 },
}

impl RasterKey {
    fn new(mood: Mood, style: MascotStyle, side_px: u32, colors: &MascotColors) -> Self {
        let side = side_px.clamp(1, MAX_SIDE_PX);
        match style {
            MascotStyle::Duotone => Self::Duotone { mood, side, colors: colors.key() },
            MascotStyle::Mono(_) => Self::Mono { mood, side },
        }
    }
}

/// Sınırlı LRU (en eski başta). Süreç genelinde tek örnek ([`mascot_image`]); testler kendi
/// örneğini kurar (paralel testler ortak örneği taşırabilir).
#[derive(Debug)]
struct RasterCache {
    entries: Vec<(RasterKey, ImageData)>,
    cap: usize,
    /// Yapılan raster sayısı (isabet testleri).
    rasters: u64,
}

impl RasterCache {
    const fn new(cap: usize) -> Self {
        Self { entries: Vec::new(), cap, rasters: 0 }
    }

    fn get(&mut self, key: &RasterKey) -> Option<ImageData> {
        let i = self.entries.iter().position(|(k, _)| k == key)?;
        let hit = self.entries.remove(i);
        let data = hit.1.clone();
        self.entries.push(hit);
        Some(data)
    }

    fn insert(&mut self, key: RasterKey, data: ImageData) -> ImageData {
        if let Some(i) = self.entries.iter().position(|(k, _)| *k == key) {
            // Yarış: başka thread aynı anahtarı bu arada üretti; paylaşılanı ver.
            return self.entries[i].1.clone();
        }
        self.rasters += 1;
        while self.entries.len() >= self.cap.max(1) {
            self.entries.remove(0);
        }
        self.entries.push((key, data.clone()));
        data
    }
}

static CACHE: Mutex<RasterCache> = Mutex::new(RasterCache::new(MASCOT_CACHE));

fn lock(c: &Mutex<RasterCache>) -> std::sync::MutexGuard<'_, RasterCache> {
    c.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `side_px` kenarlı premultiplied RGBA raster (mono: RGB = 0, yalnız alfa). Süreç genelinde
/// anahtarlı LRU'dan; pikseller `Arc` ile paylaşılır.
pub fn mascot_image(mood: Mood, style: MascotStyle, side_px: u32, colors: &MascotColors) -> Option<ImageData> {
    cached_image(&CACHE, mood, style, side_px, colors)
}

fn cached_image(cache: &Mutex<RasterCache>, mood: Mood, style: MascotStyle, side_px: u32, colors: &MascotColors) -> Option<ImageData> {
    let key = RasterKey::new(mood, style, side_px, colors);
    if let Some(hit) = lock(cache).get(&key) {
        return Some(hit);
    }
    // Kilit dışında rasterleşir (resvg ms sürebilir; diğer maskotları bekletmez).
    let data = rasterize(mood, style, key_side(&key), colors)?;
    Some(lock(cache).insert(key, data))
}

fn key_side(key: &RasterKey) -> u32 {
    match *key {
        RasterKey::Duotone { side, .. } | RasterKey::Mono { side, .. } => side,
    }
}

fn rasterize(mood: Mood, style: MascotStyle, side: u32, colors: &MascotColors) -> Option<ImageData> {
    let painted;
    let bytes = match style {
        MascotStyle::Duotone => {
            let Some(svg) = std::str::from_utf8(mood.svg()).ok().and_then(|s| paint_svg(s, colors)) else {
                tracing::warn!("gömülü maskot SVG'si ({}) boyanamadı; çizilmeyecek", mood.name());
                return None;
            };
            painted = svg;
            painted.as_bytes()
        }
        MascotStyle::Mono(_) => mood.mono_svg(),
    };
    let img = match render_svg(bytes, TargetSize::new(side, side, Fit::Contain)) {
        Ok(img) => img,
        Err(e) => {
            tracing::warn!("maskot rasterleştirilemedi ({}): {e}", mood.name());
            return None;
        }
    };
    let (width, height) = img.size();
    Some(ImageData { width, height, pixels: Arc::from(img.into_pixels()) })
}

// --- Bileşen ---------------------------------------------------------------------------------

/// Maskot bileşeni. `new(theme, mood, size)` → `set_rect` → `draw`; etkileşimsiz, canlanmasız.
#[derive(Debug, Clone)]
pub struct Mascot {
    mood: Mood,
    style: MascotStyle,
    /// Mantıksal kenar, [`MASCOT_MIN_SIZE`]..=[`MASCOT_MAX_SIZE`].
    size: f32,
    rect: Rect,
    /// `[appearance] mascot` (son tema).
    visible: bool,
    /// Son paletin çözülmüş renkleri (palet değişmedikçe yeniden hesaplanmaz).
    colors: Option<(Palette, MascotColors)>,
    /// Son dokunun anahtarı ve kendisi.
    texture: Option<(RasterKey, Texture)>,
    /// Bu bileşenin yaptığı doku yükleme sayısı (önbellek isabeti testleri ve tanılama).
    uploads: u32,
    pending: bool,
    damage: DamageState,
}

fn clamp_size(size: f32) -> Option<f32> {
    size.is_finite().then(|| size.clamp(MASCOT_MIN_SIZE, MASCOT_MAX_SIZE))
}

impl Mascot {
    /// İki tonlu maskot; `size_px` [`MASCOT_MIN_SIZE`]..=[`MASCOT_MAX_SIZE`]'a sıkıştırılır
    /// (sonlu değilse [`DEFAULT_MASCOT_SIZE`]).
    pub fn new(theme: &Theme, mood: Mood, size_px: f32) -> Self {
        Self {
            mood,
            style: MascotStyle::Duotone,
            size: clamp_size(size_px).unwrap_or(DEFAULT_MASCOT_SIZE),
            rect: Rect::ZERO,
            visible: theme.appearance.mascot,
            colors: None,
            texture: None,
            uploads: 0,
            pending: false,
            damage: DamageState::new(),
        }
    }

    /// Tek renkli kip, siluet `role` renginde (ör. küçük ya da sakin yüzeyler).
    pub fn mono(mut self, role: ColorRole) -> Self {
        self.set_style(MascotStyle::Mono(role));
        self
    }

    pub fn with_style(mut self, style: MascotStyle) -> Self {
        self.set_style(style);
        self
    }

    pub fn mood(&self) -> Mood {
        self.mood
    }

    pub fn style(&self) -> MascotStyle {
        self.style
    }

    pub fn size(&self) -> f32 {
        self.size
    }

    /// `[appearance] mascot` açık mı (son `set_theme`'e göre).
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Son `draw`'da GPU'ya henüz inmemiş doku vardı (bkz. `BrandMark::is_pending`).
    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Bu bileşenin şimdiye dek yüklediği doku sayısı.
    pub fn uploads(&self) -> u32 {
        self.uploads
    }

    pub fn set_mood(&mut self, mood: Mood) {
        if self.mood != mood {
            self.mood = mood;
            self.damage.mark();
        }
    }

    pub fn set_style(&mut self, style: MascotStyle) {
        if self.style != style {
            self.style = style;
            self.damage.mark();
        }
    }

    /// Mantıksal kenar; [`MASCOT_MIN_SIZE`]..=[`MASCOT_MAX_SIZE`]'a sıkıştırılır, sonlu değilse
    /// yok sayılır.
    pub fn set_size(&mut self, size_px: f32) {
        if let Some(size) = clamp_size(size_px)
            && size != self.size
        {
            self.size = size;
            self.damage.mark();
        }
    }

    /// Tema/kip/`[appearance]` değişimi. Renkler sonraki çizimde paletten yeniden çözülür
    /// (iki tonlu raster yeni renklerle yeniden üretilir; mono yalnızca boyanır).
    pub fn set_theme(&mut self, theme: &Theme, _now: Instant) {
        self.visible = theme.appearance.mascot;
        self.damage.mark();
    }

    /// İstenen boyut; gizliyse sıfır (yer kaplamaz).
    pub fn measure(&self) -> Size {
        if self.visible { Size::new(self.size, self.size) } else { Size::ZERO }
    }

    /// Maskot `rect`'in ortasına `size` kenarlı kare olarak çizilir (boş `rect`: sol-üst köşesi).
    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    /// Fiziksel piksele hizalı çizim dikdörtgeni ve fiziksel kenar.
    pub fn device_rect(&self, scale: f32) -> (Rect, u32) {
        let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
        let side_px = (self.size * scale).round().clamp(1.0, MAX_SIDE_PX as f32);
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

    /// Paletin renkleri (önbellekli).
    fn colors_for(&mut self, palette: &Palette) -> MascotColors {
        match self.colors {
            Some((p, c)) if p == *palette => c,
            _ => {
                let c = mascot_colors(palette);
                self.colors = Some((*palette, c));
                c
            }
        }
    }

    fn texture(&mut self, painter: &mut Painter, side_px: u32, colors: &MascotColors) -> Option<Texture> {
        let key = RasterKey::new(self.mood, self.style, side_px, colors);
        if let Some((k, t)) = &self.texture
            && *k == key
        {
            return Some(t.clone());
        }
        let image = mascot_image(self.mood, self.style, side_px, colors)?;
        let tex = painter.upload_texture(image, TextureOptions::default());
        self.uploads += 1;
        self.texture = Some((key, tex.clone()));
        Some(tex)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        self.visible = ui.theme.appearance.mascot;
        if !self.visible {
            // Gizli: doku bırakılır, eski bölge silinsin diye bildirilir.
            self.texture = None;
            self.pending = false;
            self.damage.report_hidden(ui);
            return;
        }
        let (r, side_px) = self.device_rect(ui.painter.scale());
        let colors = self.colors_for(&ui.theme.colors);
        let mut pending = false;
        if let Some(tex) = self.texture(ui.painter, side_px, &colors) {
            pending = !tex.is_ready();
            let style = ImageStyle::new(ImageFit::Fill);
            let style = match self.style {
                MascotStyle::Duotone => style.with_opacity(ui.alpha()),
                MascotStyle::Mono(role) => style.with_tint(ui.fade(ui.theme.colors.get(role))),
            };
            ui.painter.image(r, &tex, &style);
        }
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
    use nuxut_render::{DrawKind, Recorder, TextSystem};
    use nuxut_theme::contrast::{contrast_ratio, worst_ratio};
    use nuxut_theme::{ContrastMode, FrameworkConfig, ThemeMode};

    fn alpha_sum(img: &ImageData) -> u64 {
        img.pixels.chunks_exact(4).map(|p| u64::from(p[3])).sum()
    }

    fn themes() -> Vec<(String, Theme)> {
        let mut out = Vec::new();
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            for contrast in ContrastMode::ALL {
                let mut cfg = FrameworkConfig::default();
                cfg.colors.mode = nuxut_theme::Choice::new(match mode {
                    ThemeMode::Dark => nuxut_theme::ColorMode::Dark,
                    ThemeMode::Light => nuxut_theme::ColorMode::Light,
                });
                cfg.colors.contrast = nuxut_theme::Choice::new(contrast);
                let t = Theme::from_config(&cfg).0;
                assert_eq!(t.mode, mode);
                out.push((format!("{}/{}", mode.name(), contrast.name()), t));
            }
        }
        out
    }

    #[test]
    fn every_mood_renders_in_both_styles() {
        let colors = mascot_colors(&Palette::default());
        for mood in Mood::ALL {
            for style in [MascotStyle::Duotone, MascotStyle::Mono(ColorRole::Primary)] {
                for side in [48, 96, 160] {
                    let img = mascot_image(mood, style, side, &colors).unwrap_or_else(|| panic!("{mood:?} {style:?}"));
                    assert_eq!((img.width, img.height), (side, side));
                    // Boş değil, ama tüm kareyi de kaplamıyor (siluet).
                    let cover = alpha_sum(&img) as f64 / (255.0 * f64::from(side * side));
                    assert!(cover > 0.04 && cover < 0.6, "{mood:?} {style:?} {side}: {cover}");
                }
            }
        }
    }

    #[test]
    fn sources_have_only_known_classes_and_no_colors() {
        for mood in Mood::ALL {
            let svg = std::str::from_utf8(mood.svg()).unwrap();
            assert!(svg.contains("viewBox=\"0 0 256 256\""), "{mood:?}");
            assert!(!svg.contains("fill=") && !svg.contains('#') && !svg.contains("<style"), "{mood:?}: renk yok, yalnızca sınıf");
            let painted = paint_svg(svg, &mascot_colors(&Palette::default())).expect("bilinen sınıflar");
            assert!(!painted.contains("class="), "{mood:?}");
            let mono = std::str::from_utf8(mood.mono_svg()).unwrap();
            assert!(mono.contains("currentColor") && !mono.contains('#') && !mono.contains("class="), "{mood:?}");
        }
        // Bilinmeyen sınıf ya da bozuk öznitelik: çizilmez.
        let c = mascot_colors(&Palette::default());
        assert!(paint_svg("<path class=\"m-eye\"/>", &c).is_none());
        assert!(paint_svg("<path class=\"m-body", &c).is_none());
    }

    #[test]
    fn classes_map_to_theme_roles() {
        let c = MascotColors {
            body: Palette::default().primary,
            wing_back: Palette::default().on_primary,
            wing_front: Palette::default().surface,
            detail: Palette::default().error,
            beak: Palette::default().outline,
        };
        let svg = "<g class=\"m-body\"/><p class=\"m-body m-wing-back\"/><p class=\"m-wing-front\"/><p class=\"m-detail\" d=\"x\"/><p class=\"m-beak\"/>";
        let out = paint_svg(svg, &c).unwrap();
        let want = format!(
            "<g fill=\"{}\"/><p fill=\"{}\"/><p fill=\"{}\"/><p fill=\"{}\" d=\"x\"/><p fill=\"{}\"/>",
            c.body.to_hex(),
            c.wing_back.to_hex(),
            c.wing_front.to_hex(),
            c.detail.to_hex(),
            c.beak.to_hex()
        );
        assert_eq!(out, want);
        // Koyu varsayılan: marka işaretiyle aynı ikili ton; detay = yüzey.
        let dark = Palette::default();
        let d = mascot_colors(&dark);
        assert_eq!(d.wing_front, dark.primary);
        assert_eq!(d.body, brand_shade(dark.primary));
        assert_eq!(d.detail, dark.surface);
        assert_eq!(d.beak, dark.surface);
        // Açık: gövde primary (ya da ön kanattan ayrışacak kadar koyusu), detay on_surface, ön
        // kanat primary'nin açılmış tonu — zeminde okunan en açık ton (≈ LIGHT_WING_TARGET).
        let light = Palette::light();
        let l = mascot_colors(&light);
        assert!(relative_luminance(l.body) <= relative_luminance(light.primary));
        assert_eq!(l.detail, light.on_surface);
        // Gaga gövde üstünde: koyu gaga açık kipte ≈ 2:1 kalıyordu; yüzey tonu ≥ 3:1.
        assert!(contrast_ratio(l.detail, l.body) < NON_TEXT_MIN);
        assert!(contrast_ratio(l.beak, l.body) >= NON_TEXT_MIN, "gaga/gövde {}", contrast_ratio(l.beak, l.body));
        assert!(relative_luminance(l.wing_front) > relative_luminance(light.primary));
        let r = worst_ratio(l.wing_front, &[light.surface, light.surface_container]);
        assert!((NON_TEXT_MIN..LIGHT_WING_TARGET + 0.3).contains(&r), "ön kanat/zemin {r}");
        // Raster iki tonlu: ön kanadın içi ön kanat renginde (selam, sol ön kanat bölgesi değil;
        // göz akı merkezi). 256 px'te viewBox koordinatı = piksel.
        let img = mascot_image(Mood::Celebrate, MascotStyle::Duotone, 256, &d).unwrap();
        let px = |x: u32, y: u32| {
            let i = ((y * img.width + x) * 4) as usize;
            [img.pixels[i], img.pixels[i + 1], img.pixels[i + 2], img.pixels[i + 3]]
        };
        // Gövde merkezi (256·0.5 = 128, 266·0.5 = 133) gövde renginde ya da yüz öğesi; gövde
        // alt kısmı kesin gövde.
        let belly = px(128, 160);
        assert_eq!(belly[3], 255);
        assert_eq!(&belly[..3], &rgb8(d.body));
    }

    #[test]
    fn front_wing_and_body_meet_non_text_contrast() {
        for (name, t) in themes() {
            let p = &t.colors;
            let c = mascot_colors(p);
            let surfaces = [p.surface, p.surface_container];
            assert!(worst_ratio(c.wing_front, &surfaces) >= NON_TEXT_MIN, "{name}: ön kanat {}", worst_ratio(c.wing_front, &surfaces));
            assert!(worst_ratio(c.body, &surfaces) >= NON_TEXT_MIN, "{name}: gövde");
            assert!(contrast_ratio(c.detail, c.wing_front) >= NON_TEXT_MIN, "{name}: göz bebeği/göz akı {c:?} surf {}", p.surface.to_hex());
            assert!(contrast_ratio(c.beak, c.body) >= NON_TEXT_MIN, "{name}: gaga/gövde {:.2}", contrast_ratio(c.beak, c.body));
            assert!(contrast_ratio(c.body, c.wing_front) >= 1.4, "{name}: ön kanat gövdeden ayrışmalı");
        }
    }

    /// Statik paletlerin ötesi: çok sayıda kaynak renkten üretilen palet × varyant × kip, hem
    /// standart hem yüksek karşıtlık (`high_contrast`). Sarı/yeşil gibi açık birincil renkler açık
    /// zeminde, koyu birincil renkler koyu zeminde zor durumlardır.
    #[test]
    fn generated_palettes_keep_contrast_and_separation() {
        use nuxut_palette::{Mode, Variant};
        let sources = ["#000000", "#ffffff", "#808080", "#ff0000", "#00ff00", "#0000ff", "#ffff00", "#4285f4", "#c0602a", "#ffd400", "#e0006c", "#9ec5ff"];
        let mut checked = 0;
        for hex in sources {
            let src = Color::from_hex(hex).unwrap();
            for v in Variant::ALL {
                for (mode, tm) in [(Mode::Dark, ThemeMode::Dark), (Mode::Light, ThemeMode::Light)] {
                    let generated = nuxut_palette::Palette::generate(src, v, mode, 0.0);
                    let mut standard = Palette::for_mode(tm);
                    standard.apply(&nuxut_palette::export::generated_palette(&generated, None).colors);
                    for (kind, p) in [("standard", standard), ("high", standard.high_contrast())] {
                        let name = format!("{hex} {v} {} {kind}", tm.name());
                        let c = mascot_colors(&p);
                        let surfaces = [p.surface, p.surface_container];
                        assert!(worst_ratio(c.wing_front, &surfaces) >= NON_TEXT_MIN, "{name}: ön kanat/zemin");
                        assert!(worst_ratio(c.body, &surfaces) >= NON_TEXT_MIN, "{name}: gövde/zemin");
                        assert!(contrast_ratio(c.detail, c.wing_front) >= NON_TEXT_MIN, "{name}: göz bebeği/göz akı {c:?} surf {}", p.surface.to_hex());
                        assert!(contrast_ratio(c.beak, c.body) >= NON_TEXT_MIN, "{name}: gaga/gövde {:.2}", contrast_ratio(c.beak, c.body));
                        let sep = if tm == ThemeMode::Dark { DARK_WING_SEPARATION } else { LIGHT_WING_SEPARATION };
                        let got = contrast_ratio(c.body, c.wing_front);
                        assert!(got >= sep - 0.05, "{name}: ön kanat/gövde {got:.2} < {sep}");
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100);
    }

    fn draw_once(m: &mut Mascot, theme: &Theme, scale: f32) -> Vec<DrawKind> {
        let mut rec = Recorder::new();
        let mut text = TextSystem::new();
        rec.record(&mut text, (400, 400), scale, |p| {
            let mut ui = Ui::new(p, theme, Instant::now());
            m.draw(&mut ui);
        });
        rec.draws().into_iter().map(|d| d.kind).collect()
    }

    #[test]
    fn hidden_mascot_measures_zero_and_draws_nothing() {
        let shown = Theme::default();
        let mut hidden = shown.clone();
        hidden.appearance.mascot = false;
        let mut m = Mascot::new(&shown, Mood::Empty, 96.0);
        m.set_rect(Rect::new(0.0, 0.0, 96.0, 96.0));
        assert_eq!(m.measure(), Size::new(96.0, 96.0));
        assert_eq!(draw_once(&mut m, &shown, 1.0), [DrawKind::Image]);
        // Canlı gizleme: set_theme → ölçü 0, çizim yok (eski bölge report_hidden ile silinir).
        m.set_theme(&hidden, Instant::now());
        assert_eq!(m.measure(), Size::ZERO);
        let draws = draw_once(&mut m, &hidden, 1.0);
        assert!(draws.is_empty());
        assert_eq!(Mascot::new(&hidden, Mood::Hello, 48.0).measure(), Size::ZERO);
        // Geri açılır.
        m.set_theme(&shown, Instant::now());
        assert_eq!(m.measure().width, 96.0);
        assert_eq!(draw_once(&mut m, &shown, 1.0), [DrawKind::Image]);
    }

    #[test]
    fn cache_hit_avoids_reraster_and_reupload() {
        let theme = Theme::default();
        let c = mascot_colors(&theme.colors);
        // Yerel önbellek: paralel testler ortak örneği taşırabilir.
        let local = Mutex::new(RasterCache::new(3));
        let a = cached_image(&local, Mood::Rest, MascotStyle::Duotone, 77, &c).unwrap();
        let b = cached_image(&local, Mood::Rest, MascotStyle::Duotone, 77, &c).unwrap();
        assert!(Arc::ptr_eq(&a.pixels, &b.pixels), "aynı anahtar yeniden rasterleşmemeli");
        assert_eq!(lock(&local).rasters, 1);
        // Mono maske renkten bağımsız: başka paletle de isabet.
        let other = mascot_colors(&Palette::light());
        cached_image(&local, Mood::Rest, MascotStyle::Mono(ColorRole::Primary), 77, &c).unwrap();
        cached_image(&local, Mood::Rest, MascotStyle::Mono(ColorRole::OnSurface), 77, &other).unwrap();
        assert_eq!(lock(&local).rasters, 2);
        // Sınır: en eski düşer, yeniden istenince yeniden rasterleşir.
        for mood in [Mood::Empty, Mood::Hello, Mood::Error] {
            cached_image(&local, mood, MascotStyle::Duotone, 50, &c).unwrap();
        }
        assert_eq!(lock(&local).entries.len(), 3);
        cached_image(&local, Mood::Rest, MascotStyle::Duotone, 77, &c).unwrap();
        assert_eq!(lock(&local).rasters, 6);
        let mut m = Mascot::new(&theme, Mood::Thinking, 64.0);
        draw_once(&mut m, &theme, 1.0);
        draw_once(&mut m, &theme, 1.0);
        assert_eq!(m.uploads(), 1, "ikinci çizim aynı dokuyu kullanmalı");
        // Ölçek değişimi: fiziksel boyutta yeniden.
        draw_once(&mut m, &theme, 2.0);
        assert_eq!(m.uploads(), 2);
        // Tema değişimi: iki tonlu yeniden; mono yalnızca boyanır.
        let mut light = theme.clone();
        light.colors = Palette::light();
        m.set_theme(&light, Instant::now());
        draw_once(&mut m, &light, 2.0);
        assert_eq!(m.uploads(), 3);
        let mut mono = Mascot::new(&theme, Mood::Thinking, 64.0).mono(ColorRole::Primary);
        draw_once(&mut mono, &theme, 1.0);
        mono.set_theme(&light, Instant::now());
        draw_once(&mut mono, &light, 1.0);
        assert_eq!(mono.uploads(), 1);
        // Ortak önbellek de sınırlı.
        assert!(lock(&CACHE).entries.len() <= MASCOT_CACHE);
    }

    #[test]
    fn size_is_clamped_and_pixel_aligned() {
        let t = Theme::default();
        assert_eq!(Mascot::new(&t, Mood::Hello, 10.0).size(), MASCOT_MIN_SIZE);
        assert_eq!(Mascot::new(&t, Mood::Hello, 999.0).size(), MASCOT_MAX_SIZE);
        assert_eq!(Mascot::new(&t, Mood::Hello, f32::NAN).size(), DEFAULT_MASCOT_SIZE);
        let mut m = Mascot::new(&t, Mood::Hello, 100.0);
        m.set_size(f32::INFINITY);
        assert_eq!(m.size(), 100.0);
        m.set_size(1.0);
        assert_eq!(m.size(), MASCOT_MIN_SIZE);
        m.set_rect(Rect::new(10.3, 7.1, 200.0, 100.0));
        for (scale, want) in [(1.0, 48), (1.5, 72), (1.25, 60), (2.0, 96)] {
            let (r, px) = m.device_rect(scale);
            assert_eq!(px, want);
            for v in [r.x, r.y] {
                assert!((v * scale - (v * scale).round()).abs() < 1e-3);
            }
        }
    }
}
