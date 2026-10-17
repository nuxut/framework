//! Küçük göstergeler: [`Separator`] (ince çizgi), [`Badge`] (nokta ya da sayaç) ve [`Pill`]
//! (metinli rozet: "Yeni", "Beta").

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::label::Label;
use crate::slider::Orientation;
use crate::style::{HAIRLINE, accent_on, clip_text, disabled_container, disabled_content, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Ui};

/// İnce ayırıcı çizgi (`outline`). Verilen dikdörtgenin ortasına, uçlardan `inset` içeride.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Separator {
    pub orientation: Orientation,
    pub inset: f32,
}

impl Separator {
    pub fn horizontal(inset: f32) -> Self {
        Self { orientation: Orientation::Horizontal, inset }
    }

    pub fn vertical(inset: f32) -> Self {
        Self { orientation: Orientation::Vertical, inset }
    }

    /// Kapladığı kalınlık (çevresindeki boşluk dahil): `space(2)`.
    pub fn thickness(theme: &Theme) -> f32 {
        theme.space(2)
    }

    pub fn draw(&self, ui: &mut Ui, rect: Rect) {
        let line = match self.orientation {
            Orientation::Horizontal => Rect::new(
                rect.x + self.inset,
                (rect.center().y - HAIRLINE * 0.5).round(),
                (rect.width - self.inset * 2.0).max(0.0),
                HAIRLINE,
            ),
            Orientation::Vertical => Rect::new(
                (rect.center().x - HAIRLINE * 0.5).round(),
                rect.y + self.inset,
                HAIRLINE,
                (rect.height - self.inset * 2.0).max(0.0),
            ),
        };
        let color = ui.fade(crate::style::divider(ui.theme));
        ui.painter.rect(line, &RectStyle::filled(color, Corners::ZERO));
    }
}

/// Bildirim rozeti: `count = None` nokta, `Some(n)` sayaç (99'dan büyükse "99+"). Sayı
/// değişince kısa bir "pop" (ölçek yayına itme) yapar; 0'a inince küçülerek kaybolur.
#[derive(Debug, Clone)]
pub struct Badge {
    count: Option<u32>,
    visible: bool,
    label: Label,
    /// Ölçek 0..1 (görünürlük) + itme.
    scale: Animated<f32>,
    /// Sağ-üst köşenin oturacağı nokta (ikonun köşesi).
    anchor: Point,
    damage: DamageState,
}

/// Sayaçta gösterilecek en büyük sayı.
pub const BADGE_MAX: u32 = 99;

impl Badge {
    pub fn new(theme: &Theme, count: Option<u32>) -> Self {
        let visible = count != Some(0);
        Self {
            count,
            visible,
            label: Label::new(Self::text(count), text_style(theme, TextRole::Caption)),
            scale: Animated::new(if visible { 1.0 } else { 0.0 }, theme.motion.spatial_fast),
            anchor: Point::ZERO,
            damage: DamageState::new(),
        }
    }

    fn text(count: Option<u32>) -> String {
        match count {
            Some(n) if n > BADGE_MAX => format!("{BADGE_MAX}+"),
            Some(n) => n.to_string(),
            None => String::new(),
        }
    }

    pub fn count(&self) -> Option<u32> {
        self.count
    }

    pub fn set_count(&mut self, count: Option<u32>, now: Instant) -> bool {
        if self.count == count {
            return false;
        }
        self.count = count;
        self.label.set_text(&Self::text(count));
        let visible = count != Some(0);
        if visible != self.visible {
            self.visible = visible;
            self.scale.set_target(if visible { 1.0 } else { 0.0 }, now);
        } else if visible {
            self.scale.kick(6.0, now);
        }
        self.damage.mark();
        true
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.label.set_style(text_style(theme, TextRole::Caption));
        self.scale.set_motion(theme.motion.spatial_fast, now);
        self.damage.mark();
    }

    /// Rozetin sağ-üst köşesi bu noktaya oturur (tipik: ikon kutusunun sağ-üstü).
    pub fn set_anchor(&mut self, anchor: Point) {
        if self.anchor != anchor {
            self.anchor = anchor;
            self.damage.mark();
        }
    }

    pub fn size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        if self.count.is_none() {
            return Size::new(theme.space(2), theme.space(2));
        }
        let h = theme.space(4);
        let w = (self.label.measure(text).width + theme.space(2)).max(h);
        Size::new(w.ceil(), h)
    }

    fn rect_full(&mut self, text: &mut TextSystem, theme: &Theme) -> Rect {
        let s = self.size(text, theme);
        // Rozet ikonun sağ-üst köşesine merkezlenir; genişliği metinle büyür.
        Rect::new(self.anchor.x - s.width * 0.5, self.anchor.y - s.height * 0.5, s.width, s.height)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let full = self.rect_full(ui.painter.text_system(), theme);
        let k = self.scale.value(now).max(0.0);
        if k > 0.01 {
            let c = full.center();
            let r = Rect::new(c.x - full.width * k * 0.5, c.y - full.height * k * 0.5, full.width * k, full.height * k);
            let radii = Corners::uniform(r.height.min(r.width) * 0.5);
            ui.painter.rect(r, &RectStyle::filled(ui.fade(theme.colors.primary), radii));
            if self.count.is_some() && k > 0.6 {
                let a = ((k - 0.6) / 0.4).clamp(0.0, 1.0);
                let m = self.label.measure_in(ui.painter);
                let color = ui.fade(theme.colors.on_primary.multiply_alpha(a));
                self.label.draw(ui.painter, Point::new(c.x - m.width * 0.5, c.y - m.height * 0.5), color);
            }
        }
        let animating = !self.scale.is_settled(now);
        if animating {
            ui.request_frame();
        }
        // Pop taşması için pay.
        self.damage.report(ui, full.expand(full.height * 0.5), animating);
    }
}


/// Hap metninin en uzun hâli (bayt; dış girdi şekillendiriciyi yormasın). Genişlik sınırı ayrıca
/// [`Pill::set_max_width`] ile verilir (taşan `…`).
pub const MAX_PILL_TEXT: usize = 64;

/// Hap renkleri: `(zemin, metin)`. Zemin [`tonal_container`], metin onun üstünde okunur vurgu
/// ([`accent_on`]; yüksek karşıtlıkta zemine ≥ 4.5, `contrast_tests`). `on_tonal`: hap zaten
/// `tonal_container` üstünde (seçili liste satırı) — aynı zeminde kaybolmasın diye zemin
/// `surface` olur (oyuk görünüm), metin yine ona karşı okunur vurgu.
pub fn pill_colors(theme: &Theme, on_tonal: bool) -> (Color, Color) {
    let bg = if on_tonal { theme.colors.surface } else { tonal_container(theme) };
    (bg, accent_on(theme, bg))
}

/// Hap iç boşluğu: `(yatay, her yan; dikey, toplam)` = `(space(2), space(1))`.
pub(crate) fn pill_padding(theme: &Theme) -> (f32, f32) {
    (theme.space(2), theme.space(1))
}

/// Metin boyutundan hap boyutu (piksele yuvarlı).
pub(crate) fn pill_size(theme: &Theme, text: Size) -> Size {
    let (h, v) = pill_padding(theme);
    let height = (text.height + v).ceil();
    Size::new((text.width + h * 2.0).ceil().max(height), height)
}

/// Devre dışı hap: düğme/çip ile aynı soluk dil — `disabled_container` zemin, `on_surface`'ın
/// devre dışı tonu metin (yüksek karşıtlıkta yine okunur; bkz. [`disabled_content`]).
fn disabled_pill_colors(theme: &Theme) -> (Color, Color) {
    let bg = disabled_container(theme);
    (bg, disabled_content(theme, theme.colors.on_surface, &[bg]))
}

/// Hap zeminini `r`'ye çizer; üstüne çizilecek metnin rengini döner (soluklaşma uygulanmış).
pub(crate) fn draw_pill_frame(ui: &mut Ui, r: Rect, on_tonal: bool) -> Color {
    paint_pill_frame(ui, r, pill_colors(ui.theme, on_tonal))
}

fn paint_pill_frame(ui: &mut Ui, r: Rect, (bg, fg): (Color, Color)) -> Color {
    ui.painter.rect(r, &RectStyle::filled(ui.fade(bg), Corners::uniform(r.height * 0.5)));
    ui.fade(fg)
}

/// Metinli rozet ("Yeni", "Beta", "3 güncelleme"): `tonal_container` zeminli, `Caption` metinli,
/// tam yuvarlak hap. Satır içinde ([`Pill::place_inline`]: sol kenar + dikey orta) ya da bir
/// öğenin köşesine yaslı ([`Pill::place_corner`]: sağ-üst köşe, öğenin içine `space(1)` biner).
/// Etkileşimsiz, canlanmasız (boşta kare yok). Devre dışı ([`Pill::set_enabled`]) düğme/çip gibi
/// soluk çizilir. Liste satırında: [`crate::ListTrailing::Pill`] (satırın etkinliğini izler),
/// sonuç listesinde `ResultItem::trailing_pill`.
///
/// Akış: `new` → `size` (ölçüm) → `place_inline`/`place_corner`/`set_rect` → `draw`.
#[derive(Debug, Clone)]
pub struct Pill {
    label: Label,
    rect: Rect,
    on_tonal: bool,
    enabled: bool,
    damage: DamageState,
}

fn pill_style(theme: &Theme) -> nuxut_render::TextStyle {
    text_style(theme, TextRole::Caption).with_max_lines(1)
}

impl Pill {
    pub fn new(theme: &Theme, text: impl Into<String>) -> Self {
        let text: String = text.into();
        Self { label: Label::new(clip_text(&text, MAX_PILL_TEXT), pill_style(theme)), rect: Rect::ZERO, on_tonal: false, enabled: true, damage: DamageState::new() }
    }

    /// Metnin en geniş hâli (hap iç boşluğu hariç); taşan `…`. `None`: sınırsız (yalnız bayt sınırı).
    pub fn with_max_width(mut self, width: Option<f32>) -> Self {
        self.set_max_width(width);
        self
    }

    pub fn text(&self) -> &str {
        self.label.text()
    }

    /// Değiştiyse `true`.
    pub fn set_text(&mut self, text: &str) -> bool {
        let changed = self.label.set_text(clip_text(text, MAX_PILL_TEXT));
        if changed {
            self.damage.mark();
        }
        changed
    }

    pub fn set_max_width(&mut self, width: Option<f32>) {
        if self.label.set_max_width(width.map(|w| w.max(0.0))) {
            self.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, _now: Instant) {
        self.label.set_style(pill_style(theme));
        self.damage.mark();
    }

    /// Hap boyutu: metin + yatay `space(2)`, dikey toplam `space(1)` iç boşluk (en az kare).
    pub fn size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        pill_size(theme, self.label.measure(text))
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    /// Hap `tonal_container` zemin üstünde mi (seçili satır; bkz. [`pill_colors`]).
    pub fn set_on_tonal(&mut self, on_tonal: bool) {
        if self.on_tonal != on_tonal {
            self.on_tonal = on_tonal;
            self.damage.mark();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Devre dışı hap soluk çizilir (düğme/çip ile aynı `disabled_*` tonları); anında, canlanmasız.
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.damage.mark();
        }
    }

    /// Satır içi: sol kenarı `x`, dikey ortası `center_y`. Yerleşen dikdörtgen döner.
    pub fn place_inline(&mut self, text: &mut TextSystem, theme: &Theme, x: f32, center_y: f32) -> Rect {
        let s = self.size(text, theme);
        let r = Rect::new(x.round(), (center_y - s.height * 0.5).round(), s.width, s.height);
        self.set_rect(r);
        r
    }

    /// Köşeye yaslı: hapın sağ kenarı `corner.x`'in `space(1)` sağında, dikey ortası
    /// `corner.y`'de (öğenin sağ-üst köşesine biner). Yerleşen dikdörtgen döner.
    pub fn place_corner(&mut self, text: &mut TextSystem, theme: &Theme, corner: Point) -> Rect {
        let s = self.size(text, theme);
        let r = Rect::new((corner.x - s.width + theme.space(1)).round(), (corner.y - s.height * 0.5).round(), s.width, s.height);
        self.set_rect(r);
        r
    }

    /// `(zemin, metin)`: etkinse [`pill_colors`], değilse devre dışı tonlar.
    fn colors(&self, theme: &Theme) -> (Color, Color) {
        if self.enabled { pill_colors(theme, self.on_tonal) } else { disabled_pill_colors(theme) }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let r = self.rect;
        if r.is_empty() {
            self.damage.report_hidden(ui);
            return;
        }
        let fg = paint_pill_frame(ui, r, self.colors(ui.theme));
        let m = self.label.measure_in(ui.painter);
        self.label.draw(ui.painter, Point::new(r.x + (r.width - m.width) * 0.5, r.y + (r.height - m.height) * 0.5), fg);
        self.damage.report(ui, r, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_text_caps() {
        assert_eq!(Badge::text(Some(5)), "5");
        assert_eq!(Badge::text(Some(150)), "99+");
        assert_eq!(Badge::text(None), "");
    }

    use nuxut_render::{DrawKind, Recorder};

    #[test]
    fn pill_size_is_text_plus_padding_and_never_narrower_than_tall() {
        let theme = Theme::default();
        let s = pill_size(&theme, Size::new(30.0, 15.0));
        assert_eq!(s, Size::new(30.0 + theme.space(2) * 2.0, (15.0 + theme.space(1)).ceil()));
        let tiny = pill_size(&theme, Size::new(1.0, 15.0));
        assert!(tiny.width >= tiny.height, "tek harfte de hap en az daire");
    }

    #[test]
    fn pill_text_is_byte_capped_on_char_boundary() {
        let theme = Theme::default();
        let long = "ğ".repeat(100);
        let mut p = Pill::new(&theme, long.clone());
        assert!(p.text().len() <= MAX_PILL_TEXT && long.starts_with(p.text()));
        assert!(p.set_text("Yeni"));
        assert!(!p.set_text("Yeni"), "aynı metin değişiklik değil");
    }

    #[test]
    fn pill_inline_and_corner_placement() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut p = Pill::new(&theme, "Yeni");
        let s = p.size(&mut text, &theme);
        let r = p.place_inline(&mut text, &theme, 10.4, 50.0);
        assert_eq!(r.x, 10.0);
        assert!((r.center().y - 50.0).abs() <= 0.5);
        assert_eq!(r.size(), s);
        let c = p.place_corner(&mut text, &theme, Point::new(200.0, 20.0));
        assert_eq!(c.right(), 200.0 + theme.space(1), "köşeye biner");
        assert!((c.center().y - 20.0).abs() <= 0.5);
        assert_eq!(p.rect(), c);
    }

    #[test]
    fn pill_draws_tonal_frame_then_text() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut p = Pill::new(&theme, "Yeni");
        let mut rec = Recorder::new();
        rec.record(&mut text, (300, 100), 1.0, |painter| {
            let mut ui = Ui::new(painter, &theme, Instant::now());
            // Yerleşimsiz: çizim yok.
            p.draw(&mut ui);
        });
        assert!(rec.is_empty());
        let r = p.place_inline(&mut text, &theme, 10.0, 30.0);
        rec.record(&mut text, (300, 100), 1.0, |painter| {
            let mut ui = Ui::new(painter, &theme, Instant::now());
            p.draw(&mut ui);
            assert!(!ui.wants_frame(), "boşta kare yok");
        });
        let draws = rec.draws();
        let kinds: Vec<_> = draws.iter().map(|d| d.kind).collect();
        assert_eq!(kinds[0], DrawKind::Quad, "önce zemin");
        assert!(kinds[1..].iter().all(|k| *k == DrawKind::Glyphs), "{kinds:?}");
        assert_eq!(draws[0].rect, r);
        let (bg, fg) = pill_colors(&theme, false);
        assert_eq!(bg, tonal_container(&theme));
        assert_eq!(fg, accent_on(&theme, bg));
        let (sel, _) = pill_colors(&theme, true);
        assert_ne!(sel, bg, "seçili satırda hap zeminden ayrışır");
    }

    /// Devre dışı hap düğme/çip ile aynı soluk tonlardadır (her kip × karşıtlık); geri açılınca
    /// eski renkler döner.
    #[test]
    fn disabled_pill_is_dim_like_other_components() {
        for (mode, contrast) in [("dark", "standard"), ("light", "standard"), ("dark", "high"), ("light", "high")] {
            let (theme, _) = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"{contrast}\"\n")).unwrap();
            let mut p = Pill::new(&theme, "Yeni");
            assert!(p.is_enabled());
            let on = p.colors(&theme);
            p.set_enabled(false);
            let off = p.colors(&theme);
            let cap = disabled_container(&theme);
            assert_eq!(off, (cap, disabled_content(&theme, theme.colors.on_surface, &[cap])), "{mode}/{contrast}");
            let chip = crate::chip::ChipColors::new(&theme, false, false);
            assert_eq!(off, (chip.fill, chip.label), "çip ile aynı");
            assert_ne!(off, on);
            p.set_on_tonal(true);
            assert_eq!(p.colors(&theme), off, "seçili satırda da soluk");
            p.set_enabled(true);
            assert_eq!(p.colors(&theme), pill_colors(&theme, true));
        }
    }

    #[test]
    fn zero_hides() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut b = Badge::new(&theme, Some(3));
        assert!(b.visible);
        b.set_count(Some(0), t0);
        assert!(!b.visible);
        assert_eq!(b.scale.target(), 0.0);
    }
}
