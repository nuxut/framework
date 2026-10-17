//! İlerleme göstergeleri: [`Spinner`] (belirsiz: dönen, boyu nefes alan yay parçası —
//! Material 3 dairesel gösterge), [`Progress`] (belirli: ray + etkin kısım, değer yayla akar) ve
//! [`Gauge`] (dairesel gösterge: iz + değer yayı + ortada etiket). Progress ve Gauge rengi [`Tint`],
//! isteğe bağlı [`Threshold`] ile yüksek değerde eşik rengine geçer.
//!
//! Yaylar `Painter::arc` ile çizilir. Spinner yalnızca çizildiği (görünür olduğu) karelerde bir sonraki kareyi ister; gizli
//! bir spinner hiç iş yapmaz. Hızı tema hareketinden türetilir (`motion.spatial`'ın tepki
//! süresi / eğri süresi), sabit süre yoktur. Hareket kapalıyken (`Theme::reduced_motion`) belirsiz
//! spinner dönmez: tonal iz üstünde eşit aralıklı üç parçalı durağan halka çizilir, kare istenmez.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::{Animated, Motion};
use nuxut_render::{RectStyle, TextStyle};
use nuxut_theme::Theme;

use crate::label::Label;

use crate::tint::{Threshold, Tint, ToneState};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Ui};

/// Yay parçasının en kısa / en uzun açısı (tam turun kesri).
const SWEEP_MIN: f32 = 0.06;
const SWEEP_MAX: f32 = 0.75;
/// Tema hareketinin karakteristik süresine göre dönüş ve nefes periyotları (katsayı).
const SPIN_PERIODS: f32 = 3.5;
const BREATH_PERIODS: f32 = 3.0;
/// Hareket kapalıyken durağan belirsiz gösterge: parça sayısı ve her parçanın açısı (tur kesri).
const STILL_SEGMENTS: u32 = 3;
const STILL_SWEEP: f32 = 0.14;

/// Hareket stilinin karakteristik süresi (saniye): yayda tepki, eğride süre.
fn characteristic(m: Motion) -> f32 {
    let t = match m {
        Motion::Spring(s) => s.response,
        Motion::Curve(c) => c.duration,
    };
    if t.is_finite() && t > 0.0 { t } else { 0.4 }
}

#[derive(Debug, Clone)]
pub struct Spinner {
    center: Point,
    /// Dış yarıçap.
    radius: f32,
    start: Option<Instant>,
    period: f32,
    /// Hareket kapalı (`Theme::reduced_motion`): belirsiz kip durağan çizilir, kare istenmez.
    still: bool,
    /// `Some(v)`: belirli kip (yay `v` turunu kaplar, yayla akar).
    determinate: Option<Animated<f32>>,
    damage: DamageState,
}

impl Spinner {
    pub fn new(theme: &Theme) -> Self {
        Self {
            center: Point::ZERO,
            radius: theme.space(5),
            start: None,
            period: characteristic(theme.motion.spatial),
            still: theme.reduced_motion(),
            determinate: None,
            damage: DamageState::new(),
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.period = characteristic(theme.motion.spatial);
        self.still = theme.reduced_motion();
        if let Some(d) = &mut self.determinate {
            d.set_motion(theme.motion.spatial_fast, now);
        }
        self.damage.mark();
    }

    /// `None`: belirsiz (döner); `Some(0..=1)`: belirli.
    pub fn set_progress(&mut self, theme: &Theme, value: Option<f32>, now: Instant) {
        match (value, &mut self.determinate) {
            (None, d) => *d = None,
            (Some(v), Some(d)) => d.set_target(sanitize(v), now),
            (Some(v), d @ None) => {
                let mut a = Animated::new(0.0, theme.motion.spatial_fast);
                a.set_target(sanitize(v), now);
                *d = Some(a);
            }
        }
        self.damage.mark();
    }

    /// Kare kutusu: `space(10)` (dış yarıçap `space(5)`).
    pub fn preferred_size(&self, theme: &Theme) -> Size {
        Size::new(theme.space(10), theme.space(10))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        let c = rect.center();
        let r = rect.width.min(rect.height) * 0.5;
        if c != self.center || r != self.radius {
            self.center = c;
            self.radius = r;
            self.damage.mark();
        }
    }

    /// (başlangıç açısı, kapladığı açı), radyan; 0 = saat 12, saat yönünde.
    fn arc(&mut self, now: Instant) -> (f32, f32) {
        if let Some(d) = &self.determinate {
            return (0.0, d.value(now).clamp(0.0, 1.0) * TAU);
        }
        let start = *self.start.get_or_insert(now);
        let t = now.saturating_duration_since(start).as_secs_f32();
        let spin = t / (self.period * SPIN_PERIODS) * TAU;
        // Nefes: yay uzar (ucu ilerler) sonra kısalır (kuyruğu yetişir); ikisi de yumuşak.
        let phase = (t / (self.period * BREATH_PERIODS)).fract();
        let ease = |x: f32| x * x * (3.0 - 2.0 * x);
        let (head, tail) = if phase < 0.5 {
            (ease(phase * 2.0) * (SWEEP_MAX - SWEEP_MIN), 0.0)
        } else {
            (SWEEP_MAX - SWEEP_MIN, ease((phase - 0.5) * 2.0) * (SWEEP_MAX - SWEEP_MIN))
        };
        // Her döngüde kuyruk kadar ilerler: yay sürekli dönüyormuş gibi görünür.
        let cycles = (t / (self.period * BREATH_PERIODS)).floor();
        let base = spin + (cycles * (SWEEP_MAX - SWEEP_MIN) + tail) * TAU;
        (base, (SWEEP_MIN + head - tail) * TAU)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let thick = theme.space(1);
        let r = self.radius - thick * 0.5;
        if self.still && self.determinate.is_none() {
            // Hareket kapalı: sonsuz döngü yerine durağan "meşgul" halkası — tonal iz üstünde
            // eşit aralıklı üç parça. Dönel simetrik olduğu için bir ilerleme miktarı gibi
            // okunmaz (tek yay "%25" sanılırdı). Kare istenmez.
            draw_arc(ui, self.center, r, 0.0, TAU, thick, crate::style::track(theme));
            for i in 0..STILL_SEGMENTS {
                let a = i as f32 * TAU / STILL_SEGMENTS as f32;
                draw_arc(ui, self.center, r, a, STILL_SWEEP * TAU, thick, theme.colors.primary);
            }
            let bounds = Rect::new(self.center.x - self.radius, self.center.y - self.radius, self.radius * 2.0, self.radius * 2.0);
            self.damage.report(ui, bounds.expand(thick), false);
            return;
        }
        let (a0, sweep) = self.arc(now);
        if self.determinate.is_some() {
            // Belirli kipte kalan kısım tonal ray.
            draw_arc(ui, self.center, r, a0 + sweep, TAU - sweep, thick, crate::style::track(theme));
        }
        draw_arc(ui, self.center, r, a0, sweep, thick, theme.colors.primary);
        let animating = match &mut self.determinate {
            Some(d) => !d.is_settled(now),
            None => true,
        };
        if animating {
            ui.request_frame();
        }
        let bounds = Rect::new(self.center.x - self.radius, self.center.y - self.radius, self.radius * 2.0, self.radius * 2.0);
        self.damage.report(ui, bounds.expand(thick), animating);
    }
}

/// Çember yayı (`Painter::arc`, tek çağrı). Açılar burada saat 12'den saat yönünde; Painter'da 0 = +x.
fn draw_arc(ui: &mut Ui, c: Point, r: f32, start: f32, sweep: f32, thick: f32, color: Color) {
    if sweep <= 0.0 || r <= 0.0 {
        return;
    }
    let col = ui.fade(color);
    ui.painter.arc(c, r, start - FRAC_PI_2, sweep, thick, col);
}

/// Belirli doğrusal ilerleme: etkin kısım [`Tint`] (varsayılan `primary`), kalan tonal; arada küçük boşluk ve
/// uçta nokta (Material 3). Değer `spatial_fast` hareketle akar. İsteğe bağlı [`Threshold`]: değer
/// eşiği aşınca etkin kısım ve uç noktası eşik rengine (varsayılan `error`) `motion.effects` ile geçer.
#[derive(Debug, Clone)]
pub struct Progress {
    value: Animated<f32>,
    tone: ToneState,
    rect: Rect,
    damage: DamageState,
}

impl Progress {
    pub fn new(theme: &Theme, value: f32) -> Self {
        Self {
            value: Animated::new(sanitize(value), theme.motion.spatial_fast),
            tone: ToneState::new(theme),
            rect: Rect::ZERO,
            damage: DamageState::new(),
        }
    }

    /// Etkin kısmın rengi (varsayılan `primary`): `ColorRole` ya da `Color`.
    pub fn with_color(mut self, tint: impl Into<Tint>) -> Self {
        self.tone.set_base(tint.into());
        self
    }

    /// Eşik rengi (animasyonsuz başlar; sonraki değişimler geçişli).
    pub fn with_threshold(mut self, threshold: Threshold) -> Self {
        self.tone.set_threshold(Some(threshold), self.value.target(), Instant::now());
        self.tone.settle();
        self
    }

    pub fn value(&self) -> f32 {
        self.value.target()
    }

    /// Hedef değer (0..=1; sonlu olmayan → 0). Eşik kararı hedefe göre verilir.
    pub fn set_value(&mut self, value: f32, now: Instant) {
        let v = sanitize(value);
        self.value.set_target(v, now);
        self.tone.update(v, now);
        self.damage.mark();
    }

    /// Rengi değiştirir (anında; eşik geçişi ayrı ve geçişli).
    pub fn set_color(&mut self, tint: impl Into<Tint>) {
        if self.tone.set_base(tint.into()) {
            self.damage.mark();
        }
    }

    pub fn color(&self) -> Tint {
        self.tone.base()
    }

    pub fn set_threshold(&mut self, threshold: Option<Threshold>, now: Instant) {
        if self.tone.set_threshold(threshold, self.value.target(), now) {
            self.damage.mark();
        }
    }

    /// Değer eşiği aşmış mı (eşik rengi hedefte).
    pub fn is_alert(&self) -> bool {
        self.tone.is_alert()
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.value.set_motion(theme.motion.spatial_fast, now);
        self.tone.set_theme(theme, now);
        self.damage.mark();
    }

    /// Kalınlık `space(1)`.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(1))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let r = self.rect;
        let h = r.height;
        let v = self.value.value(now).clamp(0.0, 1.0);
        let gap = h;
        let active_w = (r.width * v).max(0.0);
        let radii = Corners::uniform(h * 0.5);
        let color = ui.fade(self.tone.color(theme, now));
        if active_w > 0.5 {
            ui.painter.rect(Rect::new(r.x, r.y, active_w, h), &RectStyle::filled(color, radii));
        }
        let rest_x = r.x + active_w + if active_w > 0.5 { gap } else { 0.0 };
        if r.right() - rest_x > 0.5 {
            ui.painter.rect(Rect::new(rest_x, r.y, r.right() - rest_x, h), &RectStyle::filled(ui.fade(crate::style::track(theme)), radii));
            // Uçta durma noktası.
            let d = Rect::new(r.right() - h, r.y, h, h);
            ui.painter.rect(d, &RectStyle::filled(color, radii));
        }
        let animating = !self.value.is_settled(now) | self.tone.is_animating(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, r.expand(1.0), animating);
    }
}

/// Dairesel gösterge (ring gauge): tonal iz üzerinde değer yayı, isteğe bağlı ortada etiket
/// (yüzde, sıcaklık; tabular rakam, tek satır, sığmazsa "…"). Tam halka saat 12'den saat
/// yönünde; [`with_sweep`](Self::with_sweep) < 1 ise açıklığı altta ortalanmış açık gösterge.
/// Değer ile iz arasında kalınlık kadar boşluk (Material 3); uçlarda boşluk ve iz yumuşakça
/// kapanır. Değer `spatial_fast` ile akar; renk [`Tint`] + isteğe bağlı [`Threshold`] (geçiş
/// `motion.effects`). Boşta kare yok; hasar = halkanın kutusu.
#[derive(Debug, Clone)]
pub struct Gauge {
    value: Animated<f32>,
    tone: ToneState,
    center: Point,
    /// Dış yarıçap.
    radius: f32,
    /// `None` = `space(1)`.
    thickness: Option<f32>,
    /// Kaplanan tur oranı (0, 1].
    sweep: f32,
    label: Option<Label>,
    label_role: TextRole,
    damage: DamageState,
}

impl Gauge {
    pub fn new(theme: &Theme, value: f32) -> Self {
        Self {
            value: Animated::new(sanitize(value), theme.motion.spatial_fast),
            tone: ToneState::new(theme),
            center: Point::ZERO,
            radius: 0.0,
            thickness: None,
            sweep: 1.0,
            label: None,
            label_role: TextRole::Label,
            damage: DamageState::new(),
        }
    }

    /// Değer yayının rengi (varsayılan `primary`).
    pub fn with_color(mut self, tint: impl Into<Tint>) -> Self {
        self.tone.set_base(tint.into());
        self
    }

    pub fn with_threshold(mut self, threshold: Threshold) -> Self {
        self.tone.set_threshold(Some(threshold), self.value.target(), Instant::now());
        self.tone.settle();
        self
    }

    /// Çizgi kalınlığı (mantıksal px; vars. `space(1)`; yarıçapın yarısıyla sınırlı).
    pub fn with_thickness(mut self, thickness: f32) -> Self {
        self.thickness = (thickness.is_finite() && thickness > 0.0).then_some(thickness);
        self
    }

    /// Kaplanan tur oranı: 1 = tam halka (vars.), ör. 0.75 = altı açık gösterge. (0, 1]'e kırpılır.
    pub fn with_sweep(mut self, turns: f32) -> Self {
        self.sweep = if turns.is_finite() { turns.clamp(0.05, 1.0) } else { 1.0 };
        self
    }

    /// Ortadaki etiket ([`TextRole::Label`], tabular rakam).
    pub fn with_label(mut self, theme: &Theme, text: impl Into<String>) -> Self {
        self.label = Some(Label::new(text, Self::label_style(theme, self.label_role)));
        self
    }

    /// Etiketin rolü (ör. büyük göstergede `Title`).
    pub fn with_label_role(mut self, theme: &Theme, role: TextRole) -> Self {
        self.label_role = role;
        if let Some(l) = &mut self.label {
            l.set_style(Self::label_style(theme, role));
        }
        self
    }

    fn label_style(theme: &Theme, role: TextRole) -> TextStyle {
        text_style(theme, role).with_tabular_numbers().with_max_lines(1)
    }

    pub fn value(&self) -> f32 {
        self.value.target()
    }

    pub fn set_value(&mut self, value: f32, now: Instant) {
        let v = sanitize(value);
        self.value.set_target(v, now);
        self.tone.update(v, now);
        self.damage.mark();
    }

    /// Ortadaki etiketi değiştirir (`with_label` ile açılmadıysa etkisiz); değiştiyse `true`.
    pub fn set_label(&mut self, text: &str) -> bool {
        let changed = self.label.as_mut().is_some_and(|l| l.set_text(text));
        if changed {
            self.damage.mark();
        }
        changed
    }

    pub fn set_color(&mut self, tint: impl Into<Tint>) {
        if self.tone.set_base(tint.into()) {
            self.damage.mark();
        }
    }

    pub fn color(&self) -> Tint {
        self.tone.base()
    }

    pub fn set_threshold(&mut self, threshold: Option<Threshold>, now: Instant) {
        if self.tone.set_threshold(threshold, self.value.target(), now) {
            self.damage.mark();
        }
    }

    pub fn is_alert(&self) -> bool {
        self.tone.is_alert()
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.value.set_motion(theme.motion.spatial_fast, now);
        self.tone.set_theme(theme, now);
        let style = Self::label_style(theme, self.label_role);
        if let Some(l) = &mut self.label {
            l.set_style(style);
        }
        self.damage.mark();
    }

    /// Kare kutusu: `space(12)`.
    pub fn preferred_size(&self, theme: &Theme) -> Size {
        Size::new(theme.space(12), theme.space(12))
    }

    /// Kare olmayan dikdörtgende kısa kenara sığar, ortalanır.
    pub fn set_rect(&mut self, rect: Rect) {
        let c = rect.center();
        let r = (rect.width.min(rect.height) * 0.5).max(0.0);
        if c != self.center || r != self.radius {
            self.center = c;
            self.radius = r;
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.center.x - self.radius, self.center.y - self.radius, self.radius * 2.0, self.radius * 2.0)
    }

    /// Halkanın açıları (Painter: radyan, 0 = +x, saat yönü): (başlangıç, toplam süpürme).
    fn span(&self) -> (f32, f32) {
        let total = TAU * self.sweep;
        let start = if self.sweep >= 1.0 { -FRAC_PI_2 } else { FRAC_PI_2 + (TAU - total) * 0.5 };
        (start, total)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let animating = !self.value.is_settled(now) | self.tone.is_animating(now);
        if animating {
            ui.request_frame();
        }
        let bounds = self.rect();
        if self.radius <= 0.0 {
            self.damage.report_hidden(ui);
            return;
        }
        let thick = self.thickness.unwrap_or(theme.space(1)).min(self.radius * 0.5);
        let r = self.radius - thick * 0.5;
        let (start, total) = self.span();
        let v = self.value.value(now).clamp(0.0, 1.0);
        let active = total * v;
        // Yuvarlak uçlar kalınlığın yarısı kadar taşar: açısal boşluk 2·kalınlık → görünen boşluk ≈ kalınlık.
        // Değer yayı kısayken boşluk da onunla küçülür (sıfırda iz kapalı halka, sıçrama yok).
        let active_px = active * r;
        let k = (active_px / thick).clamp(0.0, 1.0);
        let gap = 2.0 * thick / r * k;
        let full = self.sweep >= 1.0;
        let track_start = start + active + if v > 0.0 { gap } else { 0.0 };
        let track_end = if full { start + TAU - if v > 0.0 { gap } else { 0.0 } } else { start + total };
        let track_sweep = track_end - track_start;
        if track_sweep > 0.0 {
            // Kalan iz kısaldıkça söner (tam dolunca nokta kalmaz).
            let fade = if full || v > 0.0 { (track_sweep * r / thick).clamp(0.0, 1.0) } else { 1.0 };
            let track = ui.fade(crate::style::track(theme)).multiply_alpha(fade);
            ui.painter.arc(self.center, r, track_start, track_sweep.min(TAU), thick, track);
        }
        if v > 0.0 {
            // Sıfırdan çıkan değer nokta olarak belirmez: ilk yarım kalınlıkta solarak gelir.
            let appear = (active_px / (thick * 0.5)).clamp(0.0, 1.0);
            let color = ui.fade(self.tone.color(theme, now)).multiply_alpha(appear);
            ui.painter.arc(self.center, r, start, active, thick, color);
        }
        if let Some(label) = &mut self.label {
            let inner = ((r - thick * 0.5) * 2.0 - thick).max(0.0);
            label.set_max_width(Some(inner));
            let size = label.measure_in(ui.painter);
            let origin = Point::new(self.center.x - size.width * 0.5, self.center.y - size.height * 0.5);
            label.draw(ui.painter, origin, ui.fade(theme.colors.on_surface));
        }
        self.damage.report(ui, bounds.expand(1.0), animating);
    }
}

fn sanitize(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn spinner_arc_breathes_and_rotates() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = Spinner::new(&theme);
        let (a0, s0) = s.arc(t0);
        let p = s.period * BREATH_PERIODS;
        let (a1, s1) = s.arc(t0 + Duration::from_secs_f32(p * 0.5));
        assert!(s1 > s0, "yay uzar");
        assert!(a1 > a0, "döner");
        let (_, s2) = s.arc(t0 + Duration::from_secs_f32(p * 0.999));
        assert!(s2 < s1, "sonra kısalır");
        assert!((SWEEP_MIN * TAU..=SWEEP_MAX * TAU + 1e-3).contains(&s1));
    }

    #[test]
    fn determinate_springs_to_value() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = Spinner::new(&theme);
        s.set_progress(&theme, Some(0.5), t0);
        let (_, sweep) = s.arc(t0 + Duration::from_secs(3));
        assert!((sweep - TAU * 0.5).abs() < 1e-3);
        let mut p = Progress::new(&theme, f32::NAN);
        assert_eq!(p.value(), 0.0);
        p.set_value(2.0, t0);
        assert_eq!(p.value(), 1.0);
    }

    use nuxut_render::{DrawKind, Recorder, TextSystem};
    use nuxut_theme::ColorRole;

    /// Bir kare: (kare istendi mi, kayıtlı çizimler).
    fn frame(f: &mut dyn FnMut(&mut Ui), theme: &Theme, now: Instant) -> (bool, Vec<nuxut_render::RecordedDraw>) {
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        let mut wants = false;
        rec.record(&mut text, (200, 120), 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            f(&mut ui);
            wants = ui.wants_frame();
        });
        (wants, rec.draws())
    }

    #[test]
    fn progress_color_and_threshold_transition() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut p = Progress::new(&theme, 0.5).with_color(ColorRole::OnSurface);
        assert_eq!(p.color(), Tint::Role(ColorRole::OnSurface));
        p.set_rect(Rect::new(0.0, 0.0, 160.0, 4.0));
        assert_eq!(p.tone.color(&theme, t0), theme.colors.on_surface);
        p.set_threshold(Some(Threshold::new(0.9)), t0);
        assert!(!p.is_alert());
        // Eşik hedef değerle verilir; renk geçişi effects ile, kesilebilir.
        p.set_value(0.95, t0);
        assert!(p.is_alert());
        let (wants, _) = frame(&mut |ui| p.draw(ui), &theme, t0 + Duration::from_millis(30));
        assert!(wants);
        let late = t0 + Duration::from_secs(3);
        assert_eq!(p.tone.color(&theme, late), theme.colors.error);
        let (wants, _) = frame(&mut |ui| p.draw(ui), &theme, late);
        assert!(!wants, "oturunca kare yok");
        // NaN değer 0 olur, eşikten düşer.
        p.set_value(f32::NAN, late);
        assert_eq!(p.value(), 0.0);
        assert!(!p.is_alert());
    }

    #[test]
    fn gauge_draws_track_value_and_label() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = Gauge::new(&theme, 0.6).with_label(&theme, "60%");
        g.set_rect(Rect::new(10.0, 10.0, 48.0, 64.0));
        assert_eq!(g.rect(), Rect::new(10.0, 18.0, 48.0, 48.0), "kısa kenara sığar, ortalanır");
        let (wants, d) = frame(&mut |ui| g.draw(ui), &theme, t0);
        assert!(!wants, "başlangıç değeri animasyonsuz");
        assert_eq!(d.iter().filter(|d| d.kind == DrawKind::Stroke).count(), 2, "iz + değer yayı");
        assert_eq!(d.iter().filter(|d| d.kind == DrawKind::Glyphs).count(), 1, "etiket");
        for s in d.iter().filter(|d| d.kind == DrawKind::Stroke) {
            assert!(s.rect.x >= 9.0 && s.rect.right() <= 59.0, "halka kutusunda: {:?}", s.rect);
        }
        assert!(g.set_label("61%"));
        assert!(!g.set_label("61%"));
        // Değer akar (spatial_fast), sonra kare istenmez.
        g.set_value(0.9, t0);
        assert!(frame(&mut |ui| g.draw(ui), &theme, t0 + Duration::from_millis(16)).0);
        assert!(!frame(&mut |ui| g.draw(ui), &theme, t0 + Duration::from_secs(3)).0);
    }

    #[test]
    fn gauge_ends_have_no_stray_dots() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let strokes = |v: f32, sweep: f32| {
            let mut g = Gauge::new(&theme, v).with_sweep(sweep);
            g.set_rect(Rect::new(0.0, 0.0, 48.0, 48.0));
            frame(&mut |ui| g.draw(ui), &theme, t0).1.iter().filter(|d| d.kind == DrawKind::Stroke).count()
        };
        assert_eq!(strokes(0.0, 1.0), 1, "sıfırda yalnızca iz (kapalı halka)");
        assert_eq!(strokes(1.0, 1.0), 1, "doluyken iz yok");
        assert_eq!(strokes(0.0, 0.75), 1);
        assert_eq!(strokes(1.0, 0.75), 1);
        assert_eq!(strokes(f32::NAN, 1.0), 1, "NaN → 0");
        // Açık gösterge: açıklık altta ortalı.
        let g = Gauge::new(&theme, 0.0).with_sweep(0.75);
        let (start, total) = g.span();
        assert!((total - 0.75 * TAU).abs() < 1e-5);
        assert!(((start + total * 0.5) - (FRAC_PI_2 + std::f32::consts::PI)).abs() < 1e-5, "ortası saat 12");
        assert_eq!(Gauge::new(&theme, 0.0).with_sweep(f32::NAN).sweep, 1.0);
        assert_eq!(Gauge::new(&theme, 0.0).with_thickness(f32::INFINITY).thickness, None);
    }

    #[test]
    fn gauge_threshold_and_zero_rect() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = Gauge::new(&theme, 0.95).with_threshold(Threshold::new(0.9));
        assert!(g.is_alert());
        assert_eq!(g.tone.color(&theme, t0), theme.colors.error, "başlangıçta animasyonsuz");
        // Yerleşimsiz gösterge çizmez, kare istemez.
        let (wants, d) = frame(&mut |ui| g.draw(ui), &theme, t0);
        assert!(!wants);
        assert!(d.is_empty());
    }
}
