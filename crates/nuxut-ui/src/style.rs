//! Token'lardan türetilen ortak renk/biçim yardımcıları. Hepsi `Theme`'den hesaplanır; burada
//! renk değeri yoktur, yalnızca token karışım oranları vardır.

use nuxut_core::{Color, Corners, Rect};
use nuxut_theme::Theme;
use nuxut_theme::contrast::{NON_TEXT_MIN, TEXT_MIN};

/// Devre dışı içerik opaklığı (Material: %38).
pub const DISABLED_CONTENT: f32 = 0.38;
/// Devre dışı kap (container) opaklığı (Material: %12).
pub const DISABLED_CONTAINER: f32 = 0.12;
/// Tonal kapta `primary`'nin `surface_container` üzerine karışım oranı.
pub const TONAL_MIX: f32 = 0.18;
/// İnce çizgi (ayırıcı, kenarlık) kalınlığı; mantıksal piksel.
pub const HAIRLINE: f32 = 1.0;
/// Klavye odak halkası kalınlığı (DESIGN.md: 2px).
pub const FOCUS_RING: f32 = 2.0;

/// Geri sayım halkası (`ConfirmButton`) çizgi kalınlığı; mantıksal piksel.
pub const COUNTDOWN_RING: f32 = 2.0;
/// Geri sayım halkasının izi: içerik renginin bu opaklığı (kalan süre tam opak).
pub const COUNTDOWN_TRACK: f32 = 0.24;

/// Küçük geçmiş grafiği (`Sparkline`) çizgi kalınlığı; mantıksal piksel.
pub const SPARKLINE_STROKE: f32 = 2.0;
/// Sparkline dolgusunun çizgi altındaki (tepedeki) opaklığı; tabana doğru sıfıra söner.
pub const SPARKLINE_FILL: f32 = 0.28;
/// Eşik rengi (`Threshold`) için varsayılan geri dönüş payı: eşiği aşan değer, eşiğin bu kadar
/// altına inmeden normal renge dönmez (eşik çevresinde titreşen ölçümde renk yanıp sönmez).
/// Birim: tam ölçeğin oranı (0..=1 değerde mutlak pay); başka birimde `Threshold::for_range` /
/// `Threshold::percent` bunu ölçekle çarpar.
pub const THRESHOLD_HYSTERESIS: f32 = 0.03;

/// `outlines = "none"` iken ayırıcı/kılavuz çizgilerinin `on_surface` opaklığı (DESIGN: %6–8).
pub const DIVIDER_TONE: f32 = 0.07;

/// "Satırı parlat" vurgusunun `primary` opaklığı (DESIGN.md "Bileşen kuralları"; seçili
/// state layer'dan belirgin, metni bastırmayacak kadar hafif).
pub const FLASH_TONE: f32 = 0.16;

/// Standart satırın (ayar satırı, açılır satır başlığı) en az yüksekliği: `space(16)` = 64 px
/// (DESIGN "Satır yüksekliği"). Aynı sayfadaki satırlar bu ritmi paylaşır; gezinme listesi
/// (`List`, `ListDensity`) kendi yoğunluk ölçüsünü kullanır.
pub fn row_height(theme: &Theme) -> f32 {
    theme.space(16)
}

/// Ayırıcı / kılavuz çizgisi rengi: kenar kipi açıksa `theme.outline()`, değilse çok hafif
/// `on_surface` tonu.
pub fn divider(theme: &Theme) -> Color {
    theme.outline().unwrap_or_else(|| theme.colors.on_surface.multiply_alpha(DIVIDER_TONE))
}

/// İkon yer tutucusunun `on_surface_variant` opaklığı (yüklenmemiş/hatalı uygulama ikonu).
pub const ICON_PLACEHOLDER_TONE: f32 = 0.16;

/// İkon yer tutucusu rengi: hem alan zemininde (`field_fill`) hem menü yüzeyinde seçilir.
pub fn icon_placeholder(theme: &Theme) -> Color {
    theme.colors.on_surface_variant.multiply_alpha(ICON_PLACEHOLDER_TONE)
}

/// Dolgulu girdi alanının (metin kutusu, açılır seçim) zemini: kaplardan (`surface_container`)
/// ayrışsın diye bir ton açık `surface_bright` (DESIGN erişilebilirlik kuralı).
pub fn field_fill(theme: &Theme) -> Color {
    theme.colors.surface_bright
}

/// `primary` ile hafifçe boyanmış kap (tonal düğme, seçili öğe zemini). Yüksek karşıtlıkta
/// karışım, üstündeki metin rolleri (`on_surface`, `on_surface_variant`) ≥ 4.5 kalacak kadarla
/// sınırlanır ([`tinted`]); `primary` içerik bu zeminde [`accent_on`] ile çizilir.
pub fn tonal_container(theme: &Theme) -> Color {
    let c = &theme.colors;
    tinted(theme, c.surface_container, c.primary, TONAL_MIX, &[c.on_surface, c.on_surface_variant])
}

// ---------------------------------------------------------------------------------------------
// Yüksek karşıtlık: bileşenlerin kendi karışımları (docs/design/accessibility.md §1).
//
// Palet token'ları yüksek kipte zaten eşiktedir; ama bileşen token'ları karıştırır (soluk metin,
// state layer, tonal kap, iz, ayraç). Aşağıdaki yardımcılar bu karışımların TEK yeridir:
// standart kipte bayt bayt eski ifadeyi verir; yüksek kipte karışımı (opaklık ya da oran)
// eşiği tutturacak en az değişimle ayarlar. Bileşen sihirli sayı yazmaz, bunları çağırır.
// "Zemin" her yerde gerçek birleşimdir: alfalı dolgular tema yüzeylerinin (`surface`,
// `surface_container`, `surface_bright`) üstüne sırayla bindirilir ([`backdrops`]).
// ---------------------------------------------------------------------------------------------

/// Eşik ölçümündeki pay: çizimde 8 bit niceleme oranı ±0.03 oynatabilir.
const CONTRAST_MARGIN: f32 = 0.05;
/// İkili arama adımı (2^-14 ≈ 6e-5; 8 bit nicelemeden çok ince).
const SEARCH_STEPS: u32 = 14;

fn high(theme: &Theme) -> bool {
    theme.contrast.is_high()
}

/// Bileşenlerin üzerine çizildiği tema yüzeyleri.
pub fn surfaces(theme: &Theme) -> [Color; 3] {
    let c = &theme.colors;
    [c.surface, c.surface_container, c.surface_bright]
}

/// `fills` (alfalı olabilir, alttan üste sırayla) her tema yüzeyinin üstünde: içeriğin gerçek
/// zeminleri. Opak bir dolgu altındakileri örter.
pub fn backdrops(theme: &Theme, fills: &[Color]) -> [Color; 3] {
    surfaces(theme).map(|s| fills.iter().fold(Color { a: 1.0, ..s }, |bg, f| over(*f, bg)))
}

/// 8 bit nicelenmiş opak renk (ekrandaki piksel; ölçüm bununla yapılır, palet dönüşümü gibi).
fn quantized(c: Color) -> Color {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
    Color::rgba(b(c.r), b(c.g), b(c.b), 1.0)
}

/// `fg`'nin (alfasıyla) opak `bg` üstündeki birleşimi ile `bg` arasındaki oran, ekranda görüneceği
/// gibi (8 bit).
fn shown_ratio(fg: Color, bg: Color) -> f32 {
    contrast_ratio(quantized(over(fg, bg)), quantized(bg))
}

/// `fg`'nin (alfasıyla) her zeminin üstündeki birleşimi ile o zemin arasındaki en kötü oran
/// (8 bit nicelenmiş, ekrandaki piksel).
pub fn worst_over(fg: Color, bgs: &[Color]) -> f32 {
    bgs.iter().map(|b| shown_ratio(fg, *b)).fold(f32::INFINITY, f32::min)
}

/// `[lo, hi]` aralığında `ok`'u sağlayan en küçük değer; `ok(hi)` doğru varsayılır (dönen değer
/// her zaman `ok`'u sağlar, tekdüze olmayan durumda da).
fn least(mut lo: f32, mut hi: f32, ok: impl Fn(f32) -> bool) -> f32 {
    for _ in 0..SEARCH_STEPS {
        let mid = (lo + hi) * 0.5;
        if ok(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi
}

/// Yüksek karşıtlıkta `fg`'yi (alfalı olabilir) `bgs`'nin hepsinde `min` orana getirir: önce
/// opaklık en az gereken kadar artar; tam opak da yetmezse renk siyaha/beyaza (hangisi daha iyi
/// ayrışıyorsa) en az gereken kadar karışır. Standart kipte, ya da eşik zaten tutuyorsa `fg`
/// aynen döner.
pub fn legible(theme: &Theme, fg: Color, bgs: &[Color], min: f32) -> Color {
    if !high(theme) || worst_over(fg, bgs) >= min {
        return fg;
    }
    let goal = min + CONTRAST_MARGIN;
    let opaque = fg.with_alpha(1.0);
    if worst_over(opaque, bgs) >= goal {
        return fg.with_alpha(least(fg.a.max(0.0), 1.0, |a| worst_over(fg.with_alpha(a), bgs) >= goal));
    }
    // renk-matematiği: uç tonlar (siyah/beyaz) karışım hedefi, tema rengi değil.
    let pole = if worst_over(Color::WHITE, bgs) >= worst_over(Color::BLACK, bgs) { Color::WHITE } else { Color::BLACK };
    if worst_over(pole, bgs) < goal {
        // Zıt parlaklıkta zeminler: en iyi çaba uçtur.
        return pole;
    }
    let t = least(0.0, 1.0, |t| worst_over(opaque.lerp(&pole, t), bgs) >= goal);
    opaque.lerp(&pole, t)
}

/// Yüksek karşıtlıkta `layer`'ın (alfalı) opaklığını, `bgs` üstüne bindiğinde her `contents`
/// renginin birleşime karşı `min`'de kalacağı kadar azaltır (ölçek 0..=1; renk değişmez).
/// İçerik katmansız zeminde zaten `min`'in altındaysa ölçütü katmansız oranıdır (katman onu
/// daha da düşüremez). Standart kipte `layer` aynen döner.
pub fn overlay(theme: &Theme, layer: Color, bgs: &[Color], contents: &[Color], min: f32) -> Color {
    if !high(theme) || layer.a <= 0.0 {
        return layer;
    }
    let ok = |s: f32| {
        let l = layer.multiply_alpha(s);
        bgs.iter().all(|b| {
            let under = over(l, *b);
            contents.iter().all(|fg| {
                let base = shown_ratio(*fg, *b);
                let need = if base >= min { min } else { base };
                shown_ratio(*fg, under) >= need
            })
        })
    };
    if ok(1.0) {
        return layer;
    }
    // `least` en küçük "geçmeyen-değil" değeri bulur: 1 − s üzerinde ara.
    let cut = least(0.0, 1.0, |u| ok(1.0 - u));
    layer.multiply_alpha(1.0 - cut)
}

/// Hover/basılı state layer'ı: `content` renginin `opacity` opaklığı, `fills` (kap; boş =
/// doğrudan yüzey) üstünde. Yüksek karşıtlıkta opaklık, `content` ≥ 4.5 kalacak kadarla
/// sınırlanır (tipik paletlerde değişmez; sınırdaki paletlerde hover hafifler).
pub fn state_layer(theme: &Theme, content: Color, fills: &[Color], opacity: f32) -> Color {
    state_layer_under(theme, content, opacity, fills, &[content])
}

/// [`state_layer`], katman rengi ile üstündeki içerik(ler) ayrı olduğunda.
pub fn state_layer_under(theme: &Theme, layer: Color, opacity: f32, fills: &[Color], contents: &[Color]) -> Color {
    let l = layer.multiply_alpha(opacity);
    if !high(theme) {
        return l;
    }
    overlay(theme, l, &backdrops(theme, fills), contents, TEXT_MIN)
}

/// Soluk ama anlam taşıyan METİN (ay dışı gün, geçmiş etkinlik, ikincil etiket): standartta
/// `color`'ın `alpha` opaklığı; yüksek karşıtlıkta en az 4.5:1 (`fills` üstündeki gerçek zemine).
pub fn faded_text(theme: &Theme, color: Color, alpha: f32, fills: &[Color]) -> Color {
    let c = color.multiply_alpha(alpha);
    if !high(theme) {
        return c;
    }
    legible(theme, c, &backdrops(theme, fills), TEXT_MIN)
}

/// Soluk METİN DIŞI işaret (etkinlik noktası, kaydırma göstergesi, geri sayım izi): yüksek
/// karşıtlıkta en az 3:1.
pub fn faded_mark(theme: &Theme, color: Color, alpha: f32, fills: &[Color]) -> Color {
    let c = color.multiply_alpha(alpha);
    if !high(theme) {
        return c;
    }
    legible(theme, c, &backdrops(theme, fills), NON_TEXT_MIN)
}

/// Devre dışı içerik. Standartta `color` × [`DISABLED_CONTENT`] (%38). WCAG 1.4.3/1.4.11 devre
/// dışı bileşeni muaf tutar; Nuxut yüksek karşıtlıkta yine de okunurluğu garanti eder: gerçek
/// zemine (`fills`: devre dışı kap vb.) en az 3:1 (metin dışı eşiği). Etkin metin ≥ 4.5 (hedef 7)
/// olduğundan devre dışı hâl ayırt edilir kalır (karar: accessibility.md §1).
pub fn disabled_content(theme: &Theme, color: Color, fills: &[Color]) -> Color {
    let c = color.multiply_alpha(DISABLED_CONTENT);
    if !high(theme) {
        return c;
    }
    legible(theme, c, &backdrops(theme, fills), NON_TEXT_MIN)
}

/// Devre dışı kap: `on_surface` × [`DISABLED_CONTAINER`] (%12; her kipte aynı, muaf).
pub fn disabled_container(theme: &Theme) -> Color {
    theme.colors.on_surface.multiply_alpha(DISABLED_CONTAINER)
}

/// Opak `base`'in `tint` ile `t` oranında karışımı (tonal kap, etkinlik zemini). Yüksek
/// karşıtlıkta oran, `contents` metinleri karışıma karşı ≥ 4.5 kalacak kadarla sınırlanır.
pub fn tinted(theme: &Theme, base: Color, tint: Color, t: f32, contents: &[Color]) -> Color {
    if !high(theme) {
        return base.lerp(&tint, t);
    }
    let opaque = Color { a: 1.0, ..base };
    let l = overlay(theme, tint.with_alpha(t), &[opaque], contents, TEXT_MIN);
    base.lerp(&tint, l.a)
}

/// Vurgu (`primary`) içerik palet dışı bir zeminde (`bg`: tonal kap, renkli dolgu; alfalıysa
/// yüzeylere bindirilir): yüksek karşıtlıkta o zemine ≥ 4.5.
pub fn accent_on(theme: &Theme, bg: Color) -> Color {
    legible(theme, theme.colors.primary, &backdrops(theme, &[bg]), TEXT_MIN)
}

/// Değer göstergelerinin (kaydırıcı, ilerleme, gösterge, döner) etkin olmayan izi. Standartta
/// [`tonal_container`]; yüksek karşıtlıkta HCT tonu yüzeylerden en az 3:1 ayrışacak kadar itilir
/// (hue korunur; `nuxut_theme::contrast::ensure_contrast`). Etkin kısım (`primary`) yüzeye
/// ≥ 4.5'tir ve değer konumu topuz/boşlukla işaretlenir.
pub fn track(theme: &Theme) -> Color {
    let t = tonal_container(theme);
    if !high(theme) {
        return t;
    }
    nuxut_theme::contrast::ensure_contrast(t, &surfaces(theme), NON_TEXT_MIN + CONTRAST_MARGIN)
}

/// Klavye odak halkası: `primary`; yüksek karşıtlıkta halkanın değdiği renklerin hepsine ≥ 3:1:
/// tema yüzeyleri (halka dışa, boşluklu çizilir) her zaman, artı `adjacent` (halkanın bitiştiği
/// bileşen dolguları; alfalılar yüzeylere bindirilir).
pub fn focus_ring(theme: &Theme, adjacent: &[Color]) -> Color {
    let p = theme.colors.primary;
    if !high(theme) {
        return p;
    }
    let mut bgs: Vec<Color> = surfaces(theme).to_vec();
    for a in adjacent {
        bgs.extend(backdrops(theme, &[*a]));
    }
    legible(theme, p, &bgs, NON_TEXT_MIN)
}

/// Bileşen içi ince ayraç (`color` × `alpha`, `fills` zemininde): yüksek karşıtlıkta ≥ 3:1.
pub fn hairline(theme: &Theme, color: Color, alpha: f32, fills: &[Color]) -> Color {
    faded_mark(theme, color, alpha, fills)
}

/// Tam yuvarlak (hap) köşeler.
pub fn pill(rect: Rect) -> Corners {
    Corners::uniform(rect.width.min(rect.height) * 0.5)
}

/// İç içe yarıçap = dış yarıçap − iç boşluk (DESIGN.md), en az `radius_small`'ın yarısı.
pub fn nested_radius(theme: &Theme, outer: f32, inset: f32) -> f32 {
    (outer - inset).max(theme.shape.radius_small * 0.5)
}

/// Dış kaynaklı metni (D-Bus, tepsi) `max` bayta, karakter sınırından keser.
pub fn clip_text(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// HSV renk (ton 0..360, doygunluk/değer 0..1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: f32,
}

impl Hsv {
    pub fn new(h: f32, s: f32, v: f32) -> Self {
        Self { h: h.rem_euclid(360.0), s: s.clamp(0.0, 1.0), v: v.clamp(0.0, 1.0), a: 1.0 }
    }

    pub fn from_color(c: Color) -> Self {
        let max = c.r.max(c.g).max(c.b);
        let min = c.r.min(c.g).min(c.b);
        let d = max - min;
        let h = if d <= f32::EPSILON {
            0.0
        } else if max == c.r {
            60.0 * ((c.g - c.b) / d).rem_euclid(6.0)
        } else if max == c.g {
            60.0 * ((c.b - c.r) / d + 2.0)
        } else {
            60.0 * ((c.r - c.g) / d + 4.0)
        };
        let s = if max <= f32::EPSILON { 0.0 } else { d / max };
        Self { h, s, v: max, a: c.a }
    }

    pub fn to_color(self) -> Color {
        let c = self.v * self.s;
        let hp = self.h.rem_euclid(360.0) / 60.0;
        let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match hp as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        Color::rgba(r + m, g + m, b + m, self.a)
    }

    pub fn with_hue(self, h: f32) -> Self {
        Self { h: h.rem_euclid(360.0), ..self }
    }
}

/// Algısal parlaklık (sRGB ağırlıkları, yaklaşık).
pub fn luminance(c: Color) -> f32 {
    0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
}

/// WCAG göreli parlaklığı (doğrusallaştırılmış sRGB).
fn relative_luminance(c: Color) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG kontrast oranı (1..=21); renkler opak kabul edilir (önce [`over`] ile birleştir).
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (x, y) = (relative_luminance(a), relative_luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `fg`'yi (alfasıyla) opak `bg` üstüne birleştirir.
pub fn over(fg: Color, bg: Color) -> Color {
    Color { a: 1.0, ..bg.lerp(&Color { a: 1.0, ..fg }, fg.a) }
}

/// `bg` üzerinde aday renklerden en yüksek kontrastlısı.
pub fn most_contrasting(bg: Color, candidates: &[Color]) -> Color {
    candidates.iter().copied().fold(Color::TRANSPARENT, |best, c| {
        if best.a == 0.0 || contrast_ratio(c, bg) > contrast_ratio(best, bg) { c } else { best }
    })
}

/// `bg` üzerinde okunur içerik rengi: temanın açık ya da koyu metin token'ı. Yüksek
/// karşıtlıkta (rastgele renk örneği, avatar zemini) ≥ 4.5'e itilir ([`legible`]).
pub fn content_on(theme: &Theme, bg: Color) -> Color {
    let light = theme.colors.on_surface;
    let dark = theme.colors.on_primary;
    let (bl, ll, dl) = (luminance(bg), luminance(light), luminance(dark));
    let pick = if (bl - ll).abs() >= (bl - dl).abs() { light } else { dark };
    legible(theme, pick, &[Color { a: 1.0, ..bg }], TEXT_MIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for hex in ["#9ec5ff", "#12141a", "#ff0000", "#00ff80", "#808080"] {
            let c = Color::from_hex(hex).unwrap();
            assert_eq!(Hsv::from_color(c).to_color().to_hex(), hex);
        }
    }

    #[test]
    fn divider_follows_outline_mode() {
        let none = Theme::default();
        assert!(none.outline().is_none());
        assert!((divider(&none).a - DIVIDER_TONE).abs() < 1e-6);
        let (full, _) = Theme::parse("[shape]\noutlines = \"full\"\n").unwrap();
        assert_eq!(Some(divider(&full)), full.outline());
    }

    #[test]
    fn content_on_picks_contrast() {
        let t = Theme::default();
        assert_eq!(content_on(&t, t.colors.primary), t.colors.on_primary);
        assert_eq!(content_on(&t, t.colors.surface), t.colors.on_surface);
    }
}
