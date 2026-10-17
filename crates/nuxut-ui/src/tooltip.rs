//! İpucu baloncuğu (kısaltılmış başlığın tam metni, ikon düğmesinin adı, kısa açıklama).
//!
//! - **Gecikme bileşenin işidir:** [`Tooltip::hover`] bekleme başlatır ve uyanma anını döndürür;
//!   çizimde de [`Ui::wake_at`] ile bildirir. Uygulama o ana tek bir zamanlayıcı kurar, beklerken
//!   kare çizilmez (boşta sıfır iş). Süreler `theme.timing`'den (`hover_delay`, `hover_warm`).
//! - **Isınmış pencere:** ipucu görünürken başka öğeye geçmek ya da kapandıktan sonra
//!   `hover_warm` içinde yeni bir öğeye gelmek beklemesiz gösterir (yalnızca yer değiştirir).
//! - **Yerleşim:** `side` tarafında; sığmazsa karşı tarafa döner, sonra `bounds` içine kırpılır
//!   (`bounds` boşsa yüzeyin tamamı). Kaplama katmanında çizilir: kapsayıcı kırpmasına takılmaz.
//!   Kenardaki bar'dan açılış için [`MenuSide::away_from`].
//! - **Görünüş:** tonal (`surface_bright` dolgu, `on_surface` metin, kenar çizgisi yok), gölge
//!   temadan. Metin `MAX_TEXT_WIDTH` genişliğinde sarılır, `MAX_LINES` satırdan sonra "…".
//! - **Hareket:** bağlantıya bakan kenardan hafifçe (`GROW_FROM` → 1) `spatial_fast` yayıyla
//!   büyür, opaklık `effects` ile; ikisi de kesilebilir (yarıda tersine dönebilir).

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, ShadowStyle, TextStyle, TextSystem};
use nuxut_theme::{Theme, Timing};

use crate::label::Label;
use crate::menu::MenuSide;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Ui};

/// Metin sarma genişliği, boşluk ızgarası biriminde (60 × 4 = 240 px ≈ 40 karakter Caption).
const MAX_TEXT_WIDTH: u32 = 60;
/// Uzun metinde gösterilen en çok satır; fazlası "…" ile kesilir.
pub const TOOLTIP_MAX_LINES: usize = 4;
/// Büyümenin başladığı ölçek (abartısız: yalnızca bağlantıdan çıktığı sezilsin).
const GROW_FROM: f32 = 0.92;

#[derive(Debug, Clone)]
pub struct Tooltip {
    label: Label,
    anchor: Rect,
    bounds: Rect,
    side: MenuSide,
    visible: bool,
    /// Gösterim bekleniyor: bu anda belirir.
    pending: Option<Instant>,
    /// Son kapanma anı (ısınmış pencere için).
    hidden_at: Option<Instant>,
    timing: Timing,
    /// 0 = bağlantıya büzülmüş (`GROW_FROM`), 1 = tam boy.
    grow: Animated<f32>,
    alpha: Animated<f32>,
    damage: DamageState,
}

fn label_style(theme: &Theme) -> TextStyle {
    text_style(theme, TextRole::Caption).with_max_lines(TOOLTIP_MAX_LINES)
}

impl Tooltip {
    pub fn new(theme: &Theme) -> Self {
        let mut label = Label::new("", label_style(theme));
        label.set_max_width(Some(theme.space(MAX_TEXT_WIDTH)));
        Self {
            label,
            anchor: Rect::ZERO,
            bounds: Rect::ZERO,
            side: MenuSide::Below,
            visible: false,
            pending: None,
            hidden_at: None,
            timing: theme.timing,
            grow: Animated::new(0.0, theme.motion.spatial_fast),
            alpha: Animated::new(0.0, theme.motion.effects),
            damage: DamageState::new(),
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.label.set_style(label_style(theme));
        self.label.set_max_width(Some(theme.space(MAX_TEXT_WIDTH)));
        self.timing = theme.timing;
        self.grow.set_motion(theme.motion.spatial_fast, now);
        self.alpha.set_motion(theme.motion.effects, now);
        self.damage.mark();
    }

    /// Görünür (ya da beliriyor). Bekleme sırasında `false`.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Bekleyen gösterimin anı (uygulamanın zamanlayıcısı için).
    pub fn deadline(&self) -> Option<Instant> {
        self.pending
    }

    /// Gösterilen (ya da bekleyen) metin.
    pub fn text(&self) -> &str {
        self.label.text()
    }

    /// Fare `anchor` öğesinin üstünde (her motion'da çağrılabilir). Görünürse ya da ısınmış
    /// penceredeyse hemen gösterir/yer değiştirir ve `None` döner; değilse bekleme başlatır
    /// (aynı öğede sürüyorsa korur) ve belirme anını döndürür — uygulama o ana zamanlayıcı
    /// kurar, zamanlayıcıda kare ister (çizim [`Self::poll`] eder).
    pub fn hover(&mut self, text: &str, anchor: Rect, side: MenuSide, bounds: Rect, now: Instant) -> Option<Instant> {
        let same = self.anchor == anchor && self.label.text() == text;
        self.set_content(text, anchor, side, bounds);
        if self.visible {
            return None;
        }
        if self.is_warm(now) || self.timing.hover_delay.is_zero() {
            self.appear(now);
            return None;
        }
        match self.pending {
            Some(at) if same => Some(at),
            _ => {
                let at = now + self.timing.hover_delay;
                self.pending = Some(at);
                Some(at)
            }
        }
    }

    /// Fare öğeden çıktı: bekleme iptal, görünüyorsa kapanır (ısınmış pencere başlar).
    pub fn unhover(&mut self, now: Instant) {
        self.hide(now);
    }

    /// Beklemesiz gösterir (klavye odağı, betikli demo). Görünürken başka bir öğeye geçince
    /// yeniden büyümez, yalnızca yer değiştirir.
    pub fn show(&mut self, text: &str, anchor: Rect, side: MenuSide, bounds: Rect, now: Instant) {
        self.set_content(text, anchor, side, bounds);
        self.appear(now);
    }

    pub fn hide(&mut self, now: Instant) {
        self.pending = None;
        if self.visible {
            self.visible = false;
            self.hidden_at = Some(now);
            self.grow.set_target(0.0, now);
            self.alpha.set_target(0.0, now);
            self.damage.mark();
        }
    }

    /// Bekleme dolduysa gösterir; `true`: görünüş değişti (kare gerekir). `draw` da çağırır.
    pub fn poll(&mut self, now: Instant) -> bool {
        match self.pending {
            Some(at) if now >= at => {
                self.appear(now);
                true
            }
            _ => false,
        }
    }

    fn set_content(&mut self, text: &str, anchor: Rect, side: MenuSide, bounds: Rect) {
        let changed = self.label.set_text(text) | (self.anchor != anchor) | (self.side != side) | (self.bounds != bounds);
        self.anchor = anchor;
        self.side = side;
        self.bounds = bounds;
        if changed {
            self.damage.mark();
        }
    }

    fn is_warm(&self, now: Instant) -> bool {
        self.hidden_at.is_some_and(|t| now.saturating_duration_since(t) <= self.timing.hover_warm)
    }

    fn appear(&mut self, now: Instant) {
        self.pending = None;
        if !self.visible {
            self.visible = true;
            self.grow.set_target(1.0, now);
            self.alpha.set_target(1.0, now);
            self.damage.mark();
        }
    }

    fn padding(&mut self, text: &mut TextSystem, theme: &Theme) -> (f32, f32) {
        let multiline = self.label.layout(text).line_count() > 1;
        (theme.space(2), if multiline { theme.space(2) } else { theme.space(1) })
    }

    fn size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        let m = self.label.measure(text);
        let (px, py) = self.padding(text, theme);
        Size::new((m.width + px * 2.0).ceil(), (m.height + py * 2.0).ceil().max(theme.space(6)))
    }

    /// Tam boy dikdörtgen ve gerçek taraf: tercih edilen tarafa sığmıyor ve karşısına
    /// sığıyorsa döner; sonra `bounds` içine kırpılır.
    fn placement(&mut self, text: &mut TextSystem, theme: &Theme, surface: Rect) -> (Rect, MenuSide) {
        let s = self.size(text, theme);
        let gap = theme.space(1);
        let a = self.anchor;
        let b = if self.bounds.is_empty() { surface } else { self.bounds };
        let fits = |side: MenuSide| match side {
            MenuSide::Below => a.bottom() + gap + s.height <= b.bottom(),
            MenuSide::Above => a.y - gap - s.height >= b.y,
            MenuSide::Right => a.right() + gap + s.width <= b.right(),
            MenuSide::Left => a.x - gap - s.width >= b.x,
        };
        let side = if !fits(self.side) && fits(self.side.opposite()) { self.side.opposite() } else { self.side };
        let (x, y) = match side {
            MenuSide::Below => (a.center().x - s.width * 0.5, a.bottom() + gap),
            MenuSide::Above => (a.center().x - s.width * 0.5, a.y - gap - s.height),
            MenuSide::Right => (a.right() + gap, a.center().y - s.height * 0.5),
            MenuSide::Left => (a.x - gap - s.width, a.center().y - s.height * 0.5),
        };
        let x = x.clamp(b.x, (b.right() - s.width).max(b.x));
        let y = y.clamp(b.y, (b.bottom() - s.height).max(b.y));
        (Rect::new(x, y, s.width, s.height), side)
    }

    fn animating(&mut self, now: Instant) -> bool {
        !self.grow.is_settled(now) | !self.alpha.is_settled(now)
    }

    /// Bu karede bir şey çizilecek mi? `false` iken çizim yalnızca son bölgenin hasarını
    /// bildirir (kapanmanın izi silinir).
    fn needs_paint(&mut self, now: Instant) -> bool {
        self.alpha.value(now) > 0.0 || self.animating(now)
    }

    /// Kaplama katmanına çizer ([`Ui::overlay`]): kırpılı bir kapsayıcının (kaydırma alanı,
    /// kart) içinden çağrılsa da kırpılmaz, her şeyin üstünde kalır. `bounds` boşsa
    /// ([`Rect::ZERO`]) yüzeyin tamamına sığdırılır.
    pub fn draw(&mut self, ui: &mut Ui) {
        ui.overlay(|ui| self.draw_layer(ui));
    }

    fn draw_layer(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        self.poll(now);
        if let Some(at) = self.pending {
            ui.wake_at(at);
        }
        if !self.needs_paint(now) {
            self.damage.report_hidden(ui);
            return;
        }
        let alpha = self.alpha.value(now).clamp(0.0, 1.0);
        let surface = ui.surface_rect();
        let (full, side) = self.placement(ui.painter.text_system(), theme, surface);
        let g = self.grow.value(now).max(0.0);
        // Bağlantıya bakan kenara doğru hafif büzülme (yay hafifçe taşabilir; sakin ön ayarda
        // algı eşiğinin altında).
        let k = GROW_FROM + (1.0 - GROW_FROM) * g;
        let pivot = match side {
            MenuSide::Below => Point::new(full.center().x, full.y),
            MenuSide::Above => Point::new(full.center().x, full.bottom()),
            MenuSide::Right => Point::new(full.x, full.center().y),
            MenuSide::Left => Point::new(full.right(), full.center().y),
        };
        let r = Rect::new(
            pivot.x + (full.x - pivot.x) * k,
            pivot.y + (full.y - pivot.y) * k,
            full.width * k,
            full.height * k,
        );
        if alpha > 0.0 {
            let radii = theme.shape.small();
            if theme.shadow.enabled {
                let sh = ShadowStyle {
                    color: ui.fade(theme.shadow.color.multiply_alpha(alpha)),
                    blur: theme.shadow.blur * 0.5,
                    offset: Point::new(0.0, theme.shadow.offset_y),
                    spread: 0.0,
                };
                ui.painter.shadow(r, radii, &sh);
            }
            ui.painter.rect(r, &RectStyle::filled(ui.fade(theme.colors.surface_bright.multiply_alpha(alpha)), radii));
            let m = self.label.measure_in(ui.painter);
            let c = r.center();
            // Sarılmış metin sola hizalı: blok kutunun ortasında.
            let origin = Point::new((c.x - m.width * 0.5).round(), (c.y - m.height * 0.5).round());
            self.label.draw(ui.painter, origin, ui.fade(theme.colors.on_surface.multiply_alpha(alpha)));
        }
        let animating = self.animating(now);
        if animating {
            ui.request_frame();
        }
        let extent = full.expand(theme.shadow.blur * 1.5 + theme.space(1));
        self.damage.report(ui, extent, animating);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::menu::ScreenEdge;

    const BOUNDS: Rect = Rect::new(0.0, 0.0, 400.0, 300.0);
    const LONG: &str = "Belgeler — çok uzun bir pencere başlığı, barda kısaltılmış olarak görünen ve ipucunda tam metni okunması gereken bir metin; üstelik birkaç cümle daha sürüyor, satır sınırını aşsın diye uzatıldı ve hâlâ bitmedi.";

    fn settle(t: Instant) -> Instant {
        t + Duration::from_secs(5)
    }

    #[test]
    fn delay_then_visible_and_no_restart_on_motion() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = Tooltip::new(&theme);
        let a = Rect::new(100.0, 0.0, 40.0, 30.0);
        let at = t.hover("Ayarlar", a, MenuSide::Below, BOUNDS, t0).expect("gecikme bekler");
        assert_eq!(at, t0 + theme.timing.hover_delay);
        assert!(!t.is_visible());
        assert!(!t.needs_paint(t0), "beklerken çizilecek bir şey yok (kare istenmez)");
        // Aynı öğede fare kıpırdadı: bekleme yeniden başlamaz.
        let t1 = t0 + Duration::from_millis(200);
        assert_eq!(t.hover("Ayarlar", a, MenuSide::Below, BOUNDS, t1), Some(at));
        assert!(!t.poll(at - Duration::from_millis(1)), "süre dolmadan görünmez");
        assert!(!t.is_visible());
        assert!(t.poll(at));
        assert!(t.is_visible());
        assert_eq!(t.deadline(), None);
    }

    #[test]
    fn moving_to_other_item_restarts_delay_while_pending() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = Tooltip::new(&theme);
        t.hover("A", Rect::new(0.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t0);
        let t1 = t0 + Duration::from_millis(300);
        let at = t.hover("B", Rect::new(40.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t1);
        assert_eq!(at, Some(t1 + theme.timing.hover_delay));
        t.unhover(t1);
        assert_eq!(t.deadline(), None, "çıkınca bekleme iptal");
        assert!(!t.poll(settle(t1)));
    }

    #[test]
    fn warm_window_and_move_while_visible_are_immediate() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = Tooltip::new(&theme);
        t.show("A", Rect::new(0.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t0);
        // Görünürken başka öğe: beklemesiz yer değiştirir.
        let t1 = settle(t0);
        assert_eq!(t.hover("B", Rect::new(40.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t1), None);
        assert!(t.is_visible());
        assert_eq!(t.text(), "B");
        // Kapandı, ısınmış pencere içinde yeni öğe: beklemesiz.
        t.unhover(t1);
        assert!(!t.is_visible());
        let t2 = t1 + theme.timing.hover_warm / 2;
        assert_eq!(t.hover("C", Rect::new(80.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t2), None);
        assert!(t.is_visible());
        // Pencere geçtikten sonra yine bekler.
        t.unhover(t2);
        let t3 = t2 + theme.timing.hover_warm + Duration::from_millis(1);
        assert!(t.hover("D", Rect::new(120.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t3).is_some());
        assert!(!t.is_visible());
    }

    #[test]
    fn hide_damages_then_goes_idle() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = Tooltip::new(&theme);
        t.show("A", Rect::new(0.0, 0.0, 30.0, 30.0), MenuSide::Below, BOUNDS, t0);
        let t1 = settle(t0);
        assert!(t.needs_paint(t1));
        // Son çizimden sonraki temiz durum (`Default`: kirli değil).
        t.damage = DamageState::default();
        t.hide(t1);
        assert!(t.damage.is_dirty(), "kapanma hasar bildirir");
        assert!(t.needs_paint(t1 + Duration::from_millis(16)), "solarken çizilir");
        assert!(!t.needs_paint(settle(t1)), "oturunca çizim yok: draw yalnızca son bölgeyi siler (report_hidden)");
        // Kapalıyken tekrar hide hasar üretmez.
        t.damage = DamageState::default();
        t.hide(settle(t1));
        assert!(!t.damage.is_dirty());
    }

    fn placed(t: &mut Tooltip, text: &mut TextSystem, theme: &Theme, anchor: Rect, side: MenuSide) -> (Rect, MenuSide) {
        t.show("Wi-Fi ayarları", anchor, side, BOUNDS, Instant::now());
        t.placement(text, theme, Rect::ZERO)
    }

    fn inside(r: Rect) -> bool {
        r.x >= BOUNDS.x && r.y >= BOUNDS.y && r.right() <= BOUNDS.right() && r.bottom() <= BOUNDS.bottom()
    }

    #[test]
    fn flips_and_clamps_four_ways() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut t = Tooltip::new(&theme);
        let cases = [
            // Alt kenarda, aşağı istenir → yukarı döner.
            (Rect::new(380.0, 280.0, 20.0, 20.0), MenuSide::Below, MenuSide::Above),
            // Üst kenarda, yukarı istenir → aşağı.
            (Rect::new(0.0, 0.0, 20.0, 20.0), MenuSide::Above, MenuSide::Below),
            // Sağ kenarda, sağa istenir → sola.
            (Rect::new(380.0, 140.0, 20.0, 20.0), MenuSide::Right, MenuSide::Left),
            // Sol kenarda, sola istenir → sağa.
            (Rect::new(0.0, 140.0, 20.0, 20.0), MenuSide::Left, MenuSide::Right),
        ];
        for (anchor, want, got) in cases {
            let (r, side) = placed(&mut t, &mut text, &theme, anchor, want);
            assert_eq!(side, got, "{want:?}");
            assert!(inside(r), "{want:?}: {r:?}");
            assert!(r.intersect(&anchor).is_empty(), "{want:?}: bağlantıyı örtmez");
        }
        // Sığıyorsa dönmez; köşedeki bağlantıda çapraz eksende kırpılır.
        let (r, side) = placed(&mut t, &mut text, &theme, Rect::new(390.0, 0.0, 10.0, 10.0), MenuSide::Below);
        assert_eq!(side, MenuSide::Below);
        assert!(inside(r));
    }

    #[test]
    fn opens_away_from_bar_edge() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut t = Tooltip::new(&theme);
        let bars = [
            (ScreenEdge::Top, Rect::new(180.0, 0.0, 40.0, 30.0)),
            (ScreenEdge::Bottom, Rect::new(180.0, 270.0, 40.0, 30.0)),
            (ScreenEdge::Left, Rect::new(0.0, 130.0, 30.0, 40.0)),
            (ScreenEdge::Right, Rect::new(370.0, 130.0, 30.0, 40.0)),
        ];
        for (edge, anchor) in bars {
            let (r, side) = placed(&mut t, &mut text, &theme, anchor, MenuSide::away_from(edge));
            assert_eq!(side, MenuSide::away_from(edge));
            match edge {
                ScreenEdge::Top => assert!(r.y >= anchor.bottom()),
                ScreenEdge::Bottom => assert!(r.bottom() <= anchor.y),
                ScreenEdge::Left => assert!(r.x >= anchor.right()),
                ScreenEdge::Right => assert!(r.right() <= anchor.x),
            }
        }
        assert_eq!(MenuSide::away_from(ScreenEdge::Top), MenuSide::Below);
        assert_eq!(MenuSide::away_from(ScreenEdge::Bottom), MenuSide::Above);
        assert_eq!(MenuSide::away_from(ScreenEdge::Left), MenuSide::Right);
        assert_eq!(MenuSide::away_from(ScreenEdge::Right), MenuSide::Left);
    }

    #[test]
    fn long_text_wraps_within_max_width_and_line_limit() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut t = Tooltip::new(&theme);
        t.show("Kısa", Rect::new(100.0, 0.0, 20.0, 20.0), MenuSide::Below, Rect::new(0.0, 0.0, 1000.0, 800.0), Instant::now());
        let short = t.size(&mut text, &theme);
        assert_eq!(t.label.layout(&mut text).line_count(), 1);
        t.show(LONG, Rect::new(100.0, 0.0, 20.0, 20.0), MenuSide::Below, Rect::new(0.0, 0.0, 1000.0, 800.0), Instant::now());
        let m = t.label.measure(&mut text);
        let lines = t.label.layout(&mut text).line_count();
        if m.width == 0.0 {
            return; // Fontsuz CI ortamı.
        }
        assert!(m.width <= theme.space(MAX_TEXT_WIDTH) + 0.5, "sarma genişliği aşılmaz: {}", m.width);
        assert!(lines > 1, "uzun metin sarılır");
        assert!(lines <= TOOLTIP_MAX_LINES, "satır sınırı: {lines}");
        let single = TextSystem::layout(&mut text, "Ag", &label_style(&theme), None).size().height;
        assert!(m.height <= single * TOOLTIP_MAX_LINES as f32 + 0.5, "kesilir ('…'): {}", m.height);
        let long = t.size(&mut text, &theme);
        assert!(long.width > short.width && long.height > short.height);
        assert!(short.width < theme.space(MAX_TEXT_WIDTH), "kısa metin geniş kutu almaz");
    }
}
