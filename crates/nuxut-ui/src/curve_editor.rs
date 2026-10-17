//! Hareket eğrisi düzenleyici (Ayarlar → hareket):
//! - **cubic-bezier:** 0..1 kutusunda eğri ve iki sürüklenebilir tutamaç (x ∈ [0, 1],
//!   y ∈ [-1, 2]; `CubicBezier::parse` kuralları).
//! - **spring:** `Spring::evaluate`'ten zaman–konum grafiği; grafikte sürükleme `response`
//!   (yatay) ve `damping`'i (dikey) değiştirir.
//!
//! Önizleme (opsiyonel): eğri üstünde ilerleyen nokta + altta kayan top, döngüsel; yalnızca
//! çizilirken (görünürken) ve hareket sürerken kare ister. Döngü sonundaki beklemede top
//! durağandır: kare istenmez, sonraki döngünün başı [`Ui::wake_at`] ile bildirilir. Eğri `Painter::polyline`, kılavuzlar kesikli `line`.

use std::time::{Duration, Instant};

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::{CubicBezier, Spring};
use nuxut_render::RectStyle;
use nuxut_theme::Theme;

use crate::input::{Button, CursorShape, PointerEvent, PointerKind};
use crate::style::HAIRLINE;
use crate::ui::{DamageState, Response, Ui};

/// Tutamaç y sınırları (`CubicBezier::parse` ile aynı).
pub const BEZIER_Y_MIN: f32 = -1.0;
pub const BEZIER_Y_MAX: f32 = 2.0;
/// Yay düzenleme sınırları (tema doğrulamasıyla uyumlu alt küme).
pub const SPRING_RESPONSE: (f32, f32) = (0.05, 2.0);
pub const SPRING_DAMPING: (f32, f32) = (0.1, 2.0);

/// Eğri polyline'ının en fazla örnek sayısı.
const MAX_CURVE_SAMPLES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveValue {
    Bezier(CubicBezier),
    Spring(Spring),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveEvent {
    Bezier(CubicBezier),
    Spring(Spring),
}

#[derive(Debug, Clone)]
pub struct CurveEditor {
    value: CurveValue,
    rect: Rect,
    drag: Option<usize>,
    hovered: Option<usize>,
    /// `Some(süre)`: önizleme açık; bezier için eğrinin süresi.
    preview: Option<Duration>,
    preview_start: Option<Instant>,
    damage: DamageState,
    /// Önizleme nokta + top bölgesinin son hâli (yalnızca onun hasarı bildirilir).
    last_preview: Option<Rect>,
    /// Son çizimdeki grafik ve tutamaçlar (imleç için).
    last_plot: Rect,
    last_handles: Vec<Point>,
    handle_hit: f32,
}

impl CurveEditor {
    pub fn new(value: CurveValue) -> Self {
        Self { value: sanitize(value), rect: Rect::ZERO, drag: None, hovered: None, preview: None, preview_start: None, damage: DamageState::new(), last_preview: None, last_plot: Rect::ZERO, last_handles: Vec::new(), handle_hit: 0.0 }
    }

    pub fn value(&self) -> CurveValue {
        self.value
    }

    pub fn set_value(&mut self, value: CurveValue) {
        let v = sanitize(value);
        if v != self.value {
            self.value = v;
            self.preview_start = None;
            self.damage.mark();
        }
    }

    /// Önizleme: `Some(süre)` açar (bezier geçiş süresi; yayda kullanılmaz, yay kendi oturma
    /// süresini kullanır), `None` kapatır.
    pub fn set_preview(&mut self, preview: Option<Duration>) {
        if self.preview != preview {
            self.preview = preview;
            self.preview_start = None;
            self.damage.mark();
        }
    }

    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        let plot_h = (width * 0.7).round();
        let preview = if self.preview.is_some() { theme.space(10) } else { 0.0 };
        Size::new(width, plot_h + preview)
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn plot(&self, theme: &Theme) -> Rect {
        let preview = if self.preview.is_some() { theme.space(10) } else { 0.0 };
        let h = (self.rect.height - preview).max(0.0);
        Rect::new(self.rect.x, self.rect.y, self.rect.width, h)
    }

    /// Grafiğin iç alanı (kenar boşluğu tutamaçlar taşabilsin diye).
    fn inner(&self, theme: &Theme) -> Rect {
        self.plot(theme).inset(nuxut_core::Insets::uniform(theme.space(4)))
    }

    /// Dikey değer aralığı: 0..1 her zaman, tutamaçlar ya da taşma varsa genişler.
    fn y_range(&self) -> (f32, f32) {
        match self.value {
            CurveValue::Bezier(b) => (b.y1.min(b.y2).min(-0.1), b.y1.max(b.y2).max(1.1)),
            CurveValue::Spring(s) => {
                let peak = spring_curve(s, 48).iter().fold(1.0f32, |m, p| m.max(p.1));
                (-0.1, peak.max(1.1))
            }
        }
    }

    fn value_to_screen(&self, theme: &Theme, x: f32, y: f32) -> Point {
        let r = self.inner(theme);
        let (lo, hi) = self.y_range();
        Point::new(r.x + x * r.width, r.bottom() - (y - lo) / (hi - lo) * r.height)
    }

    fn screen_to_value(&self, theme: &Theme, p: Point) -> (f32, f32) {
        let r = self.inner(theme);
        let (lo, hi) = self.y_range();
        ((p.x - r.x) / r.width.max(1.0), lo + (r.bottom() - p.y) / r.height.max(1.0) * (hi - lo))
    }

    fn handles(&self, theme: &Theme) -> Vec<Point> {
        match self.value {
            CurveValue::Bezier(b) => vec![self.value_to_screen(theme, b.x1, b.y1), self.value_to_screen(theme, b.x2, b.y2)],
            CurveValue::Spring(_) => Vec::new(),
        }
    }

    fn handle_at(&self, theme: &Theme, p: Point) -> Option<usize> {
        let r = theme.space(4);
        self.handles(theme).iter().position(|h| (h.x - p.x).hypot(h.y - p.y) <= r)
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent) -> Response<CurveEvent> {
        let p = event.position;
        match event.kind {
            PointerKind::Press(Button::Left) if self.plot(theme).contains(p) => {
                match self.value {
                    CurveValue::Bezier(_) => self.drag = self.handle_at(theme, p),
                    CurveValue::Spring(_) => {
                        self.drag = Some(0);
                        return self.drag_to(theme, p);
                    }
                }
                Response::none().with_repaint(self.drag.is_some())
            }
            PointerKind::Motion if self.drag.is_some() => self.drag_to(theme, p),
            PointerKind::Release(Button::Left) | PointerKind::Leave if self.drag.is_some() => {
                self.drag = None;
                self.damage.mark();
                Response::repaint()
            }
            PointerKind::Motion | PointerKind::Leave => {
                let h = if event.kind == PointerKind::Leave { None } else { self.handle_at(theme, p) };
                if h != self.hovered {
                    self.hovered = h;
                    self.damage.mark();
                    return Response::repaint();
                }
                Response::none()
            }
            _ => Response::none(),
        }
    }

    fn drag_to(&mut self, theme: &Theme, p: Point) -> Response<CurveEvent> {
        let Some(h) = self.drag else { return Response::none() };
        self.damage.mark();
        self.preview_start = None;
        let plot = self.plot(theme);
        let (x, y) = self.screen_to_value(theme, p);
        match &mut self.value {
            CurveValue::Bezier(b) => {
                let x = x.clamp(0.0, 1.0);
                let y = y.clamp(BEZIER_Y_MIN, BEZIER_Y_MAX);
                if h == 0 {
                    b.x1 = x;
                    b.y1 = y;
                } else {
                    b.x2 = x;
                    b.y2 = y;
                }
                Response::event(CurveEvent::Bezier(*b))
            }
            CurveValue::Spring(_) => {
                let r = plot;
                let fx = ((p.x - r.x) / r.width.max(1.0)).clamp(0.0, 1.0);
                let fy = ((r.bottom() - p.y) / r.height.max(1.0)).clamp(0.0, 1.0);
                let response = SPRING_RESPONSE.0 + fx * (SPRING_RESPONSE.1 - SPRING_RESPONSE.0);
                // Yukarı = az sönüm (daha çok taşma).
                let damping = SPRING_DAMPING.1 - fy * (SPRING_DAMPING.1 - SPRING_DAMPING.0);
                let s = Spring::new(response, damping);
                self.value = CurveValue::Spring(s);
                Response::event(CurveEvent::Spring(s))
            }
        }
    }

    /// Eğri örnekleri (x, y), 0..1 zaman ekseninde.
    fn samples(&self, n: usize) -> Vec<(f32, f32)> {
        let n = n.max(2);
        match self.value {
            CurveValue::Bezier(b) => (0..=n).map(|i| {
                let t = i as f32 / n as f32;
                (t, b.ease(t))
            }).collect(),
            CurveValue::Spring(s) => spring_curve(s, n),
        }
    }

    /// Önizlemenin ilerleme kesri (0..1), değeri ve döngü sonundaki beklemedeyse sonraki
    /// döngünün başladığı an (o ana kadar görüntü değişmez).
    fn preview_state(&mut self, now: Instant) -> Option<(f32, f32, Option<Instant>)> {
        let dur = self.preview?;
        let span = match self.value {
            CurveValue::Bezier(_) => dur.as_secs_f32().max(0.05),
            CurveValue::Spring(s) => settle_time(s),
        };
        // Döngü: hareket + bekleme (%30).
        let cycle = span * 1.3;
        let start = *self.preview_start.get_or_insert(now);
        let t = now.saturating_duration_since(start).as_secs_f32() % cycle;
        let f = (t / span).min(1.0);
        let v = match self.value {
            CurveValue::Bezier(b) => b.ease(f),
            CurveValue::Spring(s) => 1.0 + s.evaluate(-1.0, 0.0, f64::from(f * span)).x as f32,
        };
        let resume = (t >= span).then(|| now + Duration::from_secs_f32((cycle - t).max(0.0)));
        Some((f, v, resume))
    }

    /// İmleç: tutamaç `Grab`, sürüklerken `Grabbing`; yay grafiği bütünüyle sürüklenebilir.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.drag.is_some() {
            return Some(CursorShape::Grabbing);
        }
        if self.last_handles.iter().any(|h| (h.x - p.x).hypot(h.y - p.y) <= self.handle_hit) {
            return Some(CursorShape::Grab);
        }
        if !self.last_plot.contains(p) {
            return None;
        }
        Some(match self.value {
            CurveValue::Spring(_) => CursorShape::Grab,
            CurveValue::Bezier(_) => CursorShape::Default,
        })
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let plot = self.plot(theme);
        self.last_plot = plot;
        self.last_handles = self.handles(theme);
        self.handle_hit = theme.space(4);
        ui.painter.rect(plot, &RectStyle::filled(ui.fade(c.surface_bright), theme.shape.medium()));
        // Referans çizgileri: 0 ve 1 düzeyi.
        let inner = self.inner(theme);
        for level in [0.0, 1.0] {
            let p = self.value_to_screen(theme, 0.0, level);
            let line = Rect::new(inner.x, (p.y - HAIRLINE * 0.5).round(), inner.width, HAIRLINE);
            ui.painter.rect(line, &RectStyle::filled(ui.fade(crate::style::divider(theme)), Corners::ZERO));
        }
        let thick = theme.space(1) * 0.5;
        // Yarı saydam polyline'da ek yerleri iki kez karışır: solarken renk grafiğin zeminine
        // önceden karıştırılıp opak çizilir.
        let alpha = ui.alpha();
        let opaque = |col: Color| c.surface_bright.lerp(&col, alpha).with_alpha(1.0);
        if let CurveValue::Bezier(_) = self.value {
            // Kontrol kolları: kesikli çizgi.
            let a0 = self.value_to_screen(theme, 0.0, 0.0);
            let a1 = self.value_to_screen(theme, 1.0, 1.0);
            let h = self.handles(theme);
            for (from, to) in [(a0, h[0]), (a1, h[1])] {
                dashed(ui, from, to, thick, theme.space(1), opaque(c.on_surface_variant));
            }
        }
        // Eğri: ekranda ~3 px'te bir örnek, tek polyline.
        let n = ((inner.width / 3.0) as usize).clamp(16, MAX_CURVE_SAMPLES);
        let pts: Vec<Point> = self.samples(n).into_iter().map(|(x, y)| self.value_to_screen(theme, x, y)).collect();
        ui.painter.polyline(&pts, thick * 2.0, opaque(c.primary));
        for (i, h) in self.handles(theme).into_iter().enumerate() {
            let r = if self.hovered == Some(i) || self.drag == Some(i) { theme.space(2) * 1.25 } else { theme.space(2) };
            let rect = Rect::new(h.x - r, h.y - r, r * 2.0, r * 2.0);
            ui.painter.rect(rect.expand(HAIRLINE * 1.5), &RectStyle::filled(ui.fade(c.surface), Corners::uniform(r + HAIRLINE * 1.5)));
            ui.painter.rect(rect, &RectStyle::filled(ui.fade(c.primary), Corners::uniform(r)));
        }

        let mut animating = false;
        let mut preview_damage: Option<Rect> = None;
        if let Some((f, v, resume)) = self.preview_state(now) {
            match resume {
                Some(at) => ui.wake_at(at),
                None => animating = true,
            }
            let dot = self.value_to_screen(theme, f, v);
            let r = theme.space(1) * 1.5;
            ui.painter.rect(Rect::new(dot.x - r, dot.y - r, r * 2.0, r * 2.0), &RectStyle::filled(ui.fade(c.on_surface), Corners::uniform(r)));
            // Altta kayan top (değer 0..1 → ray boyunca; taşma raydan taşar).
            let track_y = plot.bottom() + theme.space(5);
            let track = Rect::new(inner.x, track_y - HAIRLINE, inner.width, HAIRLINE * 2.0);
            ui.painter.rect(track, &RectStyle::filled(ui.fade(crate::style::divider(theme)), Corners::uniform(HAIRLINE)));
            let ball_r = theme.space(2);
            let bx = inner.x + v * inner.width;
            // Önizleme yalnızca nokta ve topun bölgesini hasarlı bildirir (eğri altta yeniden
            // çizilir ama kırpılır); grafiğin tamamı her karede boyanmaz.
            let dot_r = Rect::new(dot.x - r, dot.y - r, r * 2.0, r * 2.0).expand(1.0);
            let ball = Rect::new(bx - ball_r, track_y - ball_r, ball_r * 2.0, ball_r * 2.0).expand(1.0);
            preview_damage = Some(dot_r.union(&ball));
            ui.painter.rect(Rect::new(bx - ball_r, track_y - ball_r, ball_r * 2.0, ball_r * 2.0), &RectStyle::filled(ui.fade(c.primary), Corners::uniform(ball_r)));
        }
        if animating {
            ui.request_frame();
        }
        let extent = self.rect.expand(theme.space(3));
        match preview_damage {
            // Yalnızca önizleme değişti: eski ve yeni nokta/top bölgeleri.
            Some(d) if !self.damage.is_dirty() => {
                for r in [self.last_preview.take(), Some(d)].into_iter().flatten() {
                    ui.damage(r);
                }
                self.last_preview = Some(d);
                self.damage.report(ui, extent, false);
            }
            _ => {
                self.last_preview = preview_damage;
                self.damage.report(ui, extent, false);
            }
        }
    }
}

/// `a`'dan `b`'ye kesikli çizgi: `dash` uzunluğunda parçalar, aynı uzunlukta boşluk.
fn dashed(ui: &mut Ui, a: Point, b: Point, width: f32, dash: f32, color: Color) {
    let len = (b.x - a.x).hypot(b.y - a.y);
    if len <= 0.0 || dash <= 0.0 {
        return;
    }
    let (dx, dy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    let mut s = 0.0;
    while s < len {
        let e = (s + dash).min(len);
        ui.painter.line(Point::new(a.x + dx * s, a.y + dy * s), Point::new(a.x + dx * e, a.y + dy * e), width, color);
        s += dash * 2.0;
    }
}

/// Yayın oturma süresi (|sapma| < %0.1); grafiğin zaman ekseni.
fn settle_time(s: Spring) -> f32 {
    if s.is_instant() {
        return 0.05;
    }
    let mut last = 0.0;
    let mut t = 0.0f32;
    while t < 10.0 {
        let st = s.evaluate(-1.0, 0.0, f64::from(t));
        if st.x.abs() > 0.001 || st.v.abs() > 0.01 {
            last = t;
        }
        t += 0.01;
    }
    (last + 0.02).max(0.05)
}

/// Yayın 0..1 normalize zaman ekseninde konumu (0 → 1, taşma > 1).
fn spring_curve(s: Spring, n: usize) -> Vec<(f32, f32)> {
    let span = settle_time(s);
    (0..=n)
        .map(|i| {
            let f = i as f32 / n as f32;
            (f, 1.0 + s.evaluate(-1.0, 0.0, f64::from(f * span)).x as f32)
        })
        .collect()
}

fn sanitize(v: CurveValue) -> CurveValue {
    let fin = |x: f32, d: f32| if x.is_finite() { x } else { d };
    match v {
        CurveValue::Bezier(b) => CurveValue::Bezier(CubicBezier::new(
            fin(b.x1, 0.0).clamp(0.0, 1.0),
            fin(b.y1, 0.0).clamp(BEZIER_Y_MIN, BEZIER_Y_MAX),
            fin(b.x2, 1.0).clamp(0.0, 1.0),
            fin(b.y2, 1.0).clamp(BEZIER_Y_MIN, BEZIER_Y_MAX),
        )),
        CurveValue::Spring(s) => CurveValue::Spring(Spring::new(
            fin(s.response, 0.4).clamp(SPRING_RESPONSE.0, SPRING_RESPONSE.1),
            fin(s.damping, 0.8).clamp(SPRING_DAMPING.0, SPRING_DAMPING.1),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn editor(v: CurveValue) -> (CurveEditor, Theme, Instant) {
        let theme = Theme::default();
        let mut e = CurveEditor::new(v);
        e.set_rect(Rect::new(0.0, 0.0, 300.0, 210.0));
        (e, theme, Instant::now())
    }

    #[test]
    fn drag_bezier_handle_clamped_to_parse_rules() {
        let (mut e, theme, _t0) = editor(CurveValue::Bezier(CubicBezier::STANDARD));
        let h = e.handles(&theme)[0];
        let _ = e.pointer(&theme, &press(h.x, h.y));
        assert_eq!(e.drag, Some(0));
        let r = e.pointer(&theme, &motion(-500.0, -5000.0));
        let Some(CurveEvent::Bezier(b)) = r.event else { panic!("{r:?}") };
        assert_eq!(b.x1, 0.0);
        assert_eq!(b.y1, BEZIER_Y_MAX);
        // Sonuç her zaman ayrıştırılabilir bir eğri.
        let text = format!("cubic-bezier({}, {}, {}, {})", b.x1, b.y1, b.x2, b.y2);
        assert!(CubicBezier::parse(&text).is_ok(), "{text}");
        let _ = e.pointer(&theme, &release(0.0, 0.0));
        assert!(e.drag.is_none());
    }

    #[test]
    fn spring_drag_maps_axes() {
        let (mut e, theme, _t0) = editor(CurveValue::Spring(Spring::new(0.4, 0.7)));
        let plot = e.plot(&theme);
        let r = e.pointer(&theme, &press(plot.right() - 1.0, plot.y + 1.0));
        let Some(CurveEvent::Spring(s)) = r.event else { panic!() };
        assert!(s.response > 1.9 && s.damping < 0.15, "{s:?}");
    }

    #[test]
    fn spring_curve_overshoots_when_underdamped() {
        let under = spring_curve(Spring::new(0.4, 0.5), 64);
        assert!(under.iter().any(|p| p.1 > 1.05));
        let crit = spring_curve(Spring::new(0.4, 1.0), 64);
        assert!(crit.iter().all(|p| p.1 <= 1.0 + 1e-3));
        assert!((under.last().unwrap().1 - 1.0).abs() < 0.01);
    }

    #[test]
    fn preview_only_when_enabled() {
        let (mut e, _theme, t0) = editor(CurveValue::Bezier(CubicBezier::EASE));
        assert!(e.preview_state(t0).is_none());
        e.set_preview(Some(Duration::from_millis(300)));
        let (_, v0, r0) = e.preview_state(t0).unwrap();
        let (_, v1, r1) = e.preview_state(t0 + Duration::from_millis(150)).unwrap();
        assert!(v0 < v1);
        assert!(r0.is_none() && r1.is_none(), "hareket sürerken uyanma yok (normal kare)");
    }

    #[test]
    fn preview_pause_wakes_at_next_cycle() {
        let (mut e, _theme, t0) = editor(CurveValue::Bezier(CubicBezier::EASE));
        e.set_preview(Some(Duration::from_millis(300)));
        let _ = e.preview_state(t0);
        // 300 ms hareket + %30 (90 ms) bekleme: 350 ms'de bekleme, 390 ms'de yeni döngü.
        let pause = t0 + Duration::from_millis(350);
        let (f, v, resume) = e.preview_state(pause).unwrap();
        assert!(f == 1.0 && (v - 1.0).abs() < 1e-3, "beklemede top uçta durağan: {f} {v}");
        let resume = resume.expect("beklemede uyanma anı");
        let cycle_end = t0 + Duration::from_millis(390);
        assert!(resume.max(cycle_end) - resume.min(cycle_end) < Duration::from_millis(1), "{resume:?}");
        // Yeni döngü başladı: yeniden hareket.
        let (f, _, r) = e.preview_state(cycle_end + Duration::from_millis(5)).unwrap();
        assert!(f < 0.1 && r.is_none());
    }

    #[test]
    fn nan_values_are_sanitized() {
        let e = CurveEditor::new(CurveValue::Spring(Spring::new(f32::NAN, 99.0)));
        assert_eq!(e.value(), CurveValue::Spring(Spring::new(0.4, SPRING_DAMPING.1)));
    }
}
