//! Kaydırıcı (yatay/dikey): tonal hap iz, aynı hapta `primary` dolgu ve dolgunun ucunda izin
//! İÇİNDE yuvarlak topuz (Switch ile aynı dil; OSD ile ortak hap çizimi, [`crate::bar`]).
//!
//! - Topuz merkezi `[r, uzunluk − r]` aralığına eşlenir (`r` = iz yarı kalınlığı): uçlarda da
//!   izin kavsinin içinde kalır; dolgu en az topuz yuvası kadardır. Tıklama/sürükleme aynı
//!   eşlemenin tersini kullanır (imlecin altındaki nokta = topuz merkezi).
//! - Durumlar şekil + tonla: hover'da topuz büyür ve iz tonlanır, sürüklerken/klavye odağında
//!   topuz iz kalınlığına ulaşır; odakta ortasında `primary` nokta (outline yok).
//! - Değer balonu sürüklerken ve klavyeyle değişimde kısa süre görünür, sonra söner; topuzun
//!   yanında eksen üzerinde durur, bileşen dikdörtgeninin dışına çıkmaz. Görünür bekleme
//!   süresinde kare istenmez: sönme anı [`Ui::wake_at`] ile bildirilir — uygulama
//!   [`Ui::wake_deadline`]'ı tüketmezse balon bir sonraki kareye kadar görünür kalır.
//! - Değer görüntüsü yaylıdır (`spatial_fast`); sürüklerken topuz imleci gecikmesiz izler.
//! - Hiçbir katman bileşen dikdörtgeni dışına taşmaz; hasar = `rect`.

use std::hash::Hash;
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::RectStyle;
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessNode, AccessTree, Action, Role, accesskit};
use crate::bar::PillBar;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::style::{disabled_container, disabled_content, pill, state_layer_under, track};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Klavye/tekerlekle değişimden sonra değer balonunun görünür kaldığı süre.
const BUBBLE_HOLD: Duration = Duration::from_millis(1200);
/// Sürükleme bitince balonun kalma süresi.
const BUBBLE_HOLD_AFTER_DRAG: Duration = Duration::from_millis(500);
/// Hover'da topuz büyümesi (0 = dinlenme, 1 = iz kalınlığı).
const HOVER_GROW: f32 = 0.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub struct Slider {
    orientation: Orientation,
    rect: Rect,
    /// Normalize değer 0..=1 (gerçek değer).
    value: f32,
    /// Görüntülenen değer (yaylı).
    shown: Animated<f32>,
    /// Topuz büyümesi 0 (dinlenme) .. 1 (iz kalınlığı).
    grow: Animated<f32>,
    /// Değer balonu görünürlüğü 0..1.
    bubble: Animated<f32>,
    /// Balonun sönmeye başlayacağı an (klavye/tekerlek/sürükleme sonrası).
    bubble_until: Option<Instant>,
    bubble_label: Label,
    /// İz kalınlığı (temadan; eşleme çizimden bağımsız hesaplanır).
    thick: f32,
    /// `Some(n)`: değer `1/n` adımlarına oturur.
    steps: Option<u32>,
    dragging: bool,
    interaction: Interaction,
    enabled: bool,
    focus_visible: bool,
    damage: DamageState,
    /// Son çizimdeki topuz (imleç için).
    last_knob: Rect,
}

impl Slider {
    pub fn new(theme: &Theme, orientation: Orientation, value: f32) -> Self {
        let value = sanitize(value);
        Self {
            orientation,
            rect: Rect::ZERO,
            value,
            shown: Animated::new(value, theme.motion.spatial_fast),
            grow: Animated::new(0.0, theme.motion.spatial_fast),
            bubble: Animated::new(0.0, theme.motion.effects),
            bubble_until: None,
            bubble_label: Label::new("", Self::bubble_style(theme)),
            thick: Self::track_thickness(theme),
            steps: None,
            dragging: false,
            interaction: Interaction::new(theme),
            enabled: true,
            focus_visible: false,
            damage: DamageState::new(),
            last_knob: Rect::ZERO,
        }
    }

    /// İz kalınlığı: `space(5)`.
    pub fn track_thickness(theme: &Theme) -> f32 {
        theme.space(5)
    }

    fn bubble_style(theme: &Theme) -> nuxut_render::TextStyle {
        text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1)
    }

    pub fn with_steps(mut self, steps: u32) -> Self {
        self.steps = (steps > 0).then_some(steps);
        self.value = self.quantize(self.value);
        self.shown.jump_to(self.value);
        self
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Dışarıdan değer (ör. ses seviyesi değişti). Sürükleme sırasında yok sayılır
    /// (kullanıcının eli üstündeyken topuz zıplamaz).
    pub fn set_value(&mut self, value: f32, now: Instant) -> bool {
        let value = self.quantize(sanitize(value));
        if self.dragging || (value - self.value).abs() < f32::EPSILON {
            return false;
        }
        self.value = value;
        self.shown.set_target(value, now);
        self.damage.mark();
        true
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.dragging = false;
            self.interaction.reset(now);
            self.update_grow(now);
            self.damage.mark();
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) {
        if self.focus_visible != on {
            self.focus_visible = on;
            self.update_grow(Instant::now());
            self.damage.mark();
        }
    }

    /// Görünüş durumunu doğrudan verir (galeri, ekran görüntüsü testleri): `pressed` sürükleme
    /// görünümüdür (topuz iz kalınlığında, değer balonu açık). Bu kaydırıcıya olay gönderilmez.
    pub fn set_visual_state(&mut self, hovered: bool, pressed: bool, now: Instant) {
        let _ = self.interaction.set_state(hovered || pressed, pressed, now);
        self.grow.set_target(if pressed { 1.0 } else if hovered { HOVER_GROW } else { 0.0 }, now);
        if pressed {
            self.show_bubble(None, now);
        }
        self.damage.mark();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.shown.set_motion(theme.motion.spatial_fast, now);
        self.grow.set_motion(theme.motion.spatial_fast, now);
        self.bubble.set_motion(theme.motion.effects, now);
        self.bubble_label.set_style(Self::bubble_style(theme));
        self.thick = Self::track_thickness(theme);
        self.interaction.set_theme(theme, now);
        self.damage.mark();
    }

    /// Kalınlık (tıklama alanı) `space(11)`; görünen iz bunun ortasında daha incedir.
    pub fn preferred_size(&self, theme: &Theme, length: f32) -> Size {
        match self.orientation {
            Orientation::Horizontal => Size::new(length, theme.space(11)),
            Orientation::Vertical => Size::new(theme.space(11), length),
        }
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

    fn horizontal(&self) -> bool {
        self.orientation == Orientation::Horizontal
    }

    /// Eksen boyunca uzunluk ve iz kalınlığı (kısa bileşende kalınlık uzunluğu aşmaz).
    fn axis(&self) -> (f32, f32) {
        let (len, cross) = if self.horizontal() { (self.rect.width, self.rect.height) } else { (self.rect.height, self.rect.width) };
        (len, self.thick.min(cross).min(len))
    }

    fn quantize(&self, v: f32) -> f32 {
        match self.steps {
            Some(n) => (v * n as f32).round() / n as f32,
            None => v,
        }
    }

    fn step(&self) -> f32 {
        self.steps.map_or(0.05, |n| 1.0 / n as f32)
    }

    /// Ekran noktasının eksen üzerindeki uzaklığı (dikeyde aşağıdan).
    fn along(&self, p: Point) -> f32 {
        if self.horizontal() { p.x - self.rect.x } else { self.rect.bottom() - p.y }
    }

    /// Konumdan değer: [`knob_center`]'ın tersi.
    fn value_at(&self, p: Point) -> f32 {
        let (len, t) = self.axis();
        self.quantize(sanitize(value_from_center(len, t, self.along(p))))
    }

    fn update_grow(&mut self, now: Instant) {
        let target = if !self.enabled {
            0.0
        } else if self.dragging || self.focus_visible {
            1.0
        } else if self.interaction.is_hovered() {
            HOVER_GROW
        } else {
            0.0
        };
        self.grow.set_target(target, now);
    }

    fn show_bubble(&mut self, hold: Option<Duration>, now: Instant) {
        self.bubble.set_target(1.0, now);
        self.bubble_until = hold.map(|h| now + h);
    }

    /// Bekleme süresi dolduysa balon sönmeye başlar (sürüklerken süre işlemez).
    fn advance_bubble(&mut self, now: Instant) {
        if let Some(until) = self.bubble_until
            && now >= until
            && !self.dragging
        {
            self.bubble_until = None;
            self.bubble.set_target(0.0, now);
        }
    }

    /// Bu karenin zamanlama ihtiyacı: (canlanıyor → sonraki kare, uyanma anı). Balon görünür
    /// ve durağanken kare istenmez; sönmenin başlayacağı an uyanma olarak döner (uygulama
    /// [`Ui::wake_deadline`] için zamanlayıcı kurar).
    fn frame_need(&mut self, now: Instant) -> (bool, Option<Instant>) {
        let animating = !self.shown.is_settled(now)
            | !self.grow.is_settled(now)
            | !self.bubble.is_settled(now)
            | self.interaction.is_animating(now);
        let wake = self.bubble_until.filter(|_| !self.dragging);
        (animating, wake)
    }

    fn commit(&mut self, value: f32, now: Instant, follow: bool) -> Response<f32> {
        let changed = (value - self.value).abs() >= f32::EPSILON;
        self.value = value;
        if follow {
            self.shown.jump_to(value);
        } else {
            self.shown.set_target(value, now);
        }
        self.damage.mark();
        if changed { Response::event(value) } else { Response::repaint() }
    }

    /// Basma/sürükleme/tekerlek. Değer değişince `event = Some(yeni)`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<f32> {
        if !self.enabled {
            return Response::none();
        }
        let hit = self.rect.contains(event.position);
        let mut out = Response::none();
        match event.kind {
            PointerKind::Press(Button::Left) if hit => {
                self.dragging = true;
                self.focus_visible = false;
                self.show_bubble(None, now);
                // Tıklanan yere yayla gider; sürükleme başlayınca doğrudan izler.
                let v = self.value_at(event.position);
                out = self.commit(v, now, false);
            }
            PointerKind::Motion if self.dragging => {
                let v = self.value_at(event.position);
                out = self.commit(v, now, true);
            }
            // Dokunmada bırakmasız Leave = compositor diziyi iptal etti: sürükleme biter.
            PointerKind::Release(Button::Left) | PointerKind::Leave
                if self.dragging && (event.kind != PointerKind::Leave || event.is_touch()) =>
            {
                self.dragging = false;
                self.show_bubble(Some(BUBBLE_HOLD_AFTER_DRAG), now);
                self.damage.mark();
                out = Response::repaint();
            }
            PointerKind::Scroll { delta, steps } if hit => {
                let notches = if steps.y.abs() > 0.0 || steps.x.abs() > 0.0 {
                    -(steps.y + steps.x)
                } else {
                    -(delta.y + delta.x) / self.rect.width.max(self.rect.height).max(1.0) / self.step()
                };
                let v = self.quantize(sanitize(self.value + notches * self.step()));
                out = self.commit(v, now, false);
                if out.event.is_some() {
                    self.show_bubble(Some(BUBBLE_HOLD), now);
                }
            }
            _ => {}
        }
        let hovered = hit || self.dragging;
        if self.interaction.set_state(hovered, self.dragging, now) {
            self.damage.mark();
            out.repaint = true;
        }
        if event.kind == PointerKind::Leave && !self.dragging && self.interaction.reset(now) {
            out.repaint = true;
        }
        self.update_grow(now);
        out
    }

    /// Ok tuşları adım, PageUp/PageDown 4 adım, Home/End uçlar.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<f32> {
        if !self.enabled || event.state == KeyState::Released {
            return Response::none();
        }
        let s = self.step();
        let v = match event.key {
            Key::Right | Key::Up => self.value + s,
            Key::Left | Key::Down => self.value - s,
            Key::PageUp => self.value + s * 4.0,
            Key::PageDown => self.value - s * 4.0,
            Key::Home => 0.0,
            Key::End => 1.0,
            _ => return Response::none(),
        };
        self.focus_visible = true;
        self.update_grow(now);
        self.show_bubble(Some(BUBBLE_HOLD), now);
        let v = self.quantize(sanitize(v));
        self.commit(v, now, false)
    }

    /// İmleç: sürüklerken `Grabbing`, topuzda `Grab`, izde `Pointer`, devre dışı `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.dragging {
            return Some(CursorShape::Grabbing);
        }
        if !self.rect.contains(p) {
            return None;
        }
        Some(if !self.enabled {
            CursorShape::Default
        } else if self.last_knob.contains(p) {
            CursorShape::Grab
        } else {
            CursorShape::Pointer
        })
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let horizontal = self.horizontal();
        let (len, t) = self.axis();
        let rect = self.rect;
        let track = if horizontal {
            Rect::new(rect.x, rect.center().y - t * 0.5, len, t)
        } else {
            Rect::new(rect.center().x - t * 0.5, rect.y, t, len)
        };
        // Yaylı değer hedefi biraz aşabilir; topuz yine izin içinde kalır.
        let center = knob_center(len, t, self.shown.value(now).clamp(-0.02, 1.02));
        let colors = SliderColors::new(theme, self.enabled, self.interaction.opacity(now));
        let bar = PillBar {
            track,
            horizontal,
            fill: (center + t * 0.5).min(len),
            rest: ui.fade(colors.rest),
            fill_color: ui.fade(colors.fill),
            overflow: None,
        };
        bar.draw(ui.painter);

        let grow = self.grow.value(now).clamp(0.0, 1.0);
        let d = knob_diameter(theme, t, grow);
        let knob = {
            let r = bar.span(center - d * 0.5, d);
            if horizontal {
                Rect::new(r.x, track.center().y - d * 0.5, d, d)
            } else {
                Rect::new(track.center().x - d * 0.5, r.y, d, d)
            }
        };
        self.last_knob = knob;
        ui.painter.rect(knob, &RectStyle::filled(ui.fade(colors.knob), pill(knob)));
        if self.focus_visible && self.enabled {
            let dot = knob.expand(-d * 0.32);
            ui.painter.rect(dot, &RectStyle::filled(ui.fade(theme.colors.primary), pill(dot)));
        }

        // Değer balonu: topuzun yanında, eksen üzerinde; sığmazsa öbür yanda.
        self.advance_bubble(now);
        let alpha = self.bubble.value(now).clamp(0.0, 1.0);
        if alpha > 0.01 {
            self.bubble_label.set_text(&format!("{}", (self.value * 100.0).round() as u32));
            let m = self.bubble_label.measure_in(ui.painter);
            let pad = theme.space(1);
            let (bw, bh) = (m.width + pad * 2.0, m.height + pad * 0.5);
            let (along_size, cross_size) = if horizontal { (bw, bh) } else { (bh, bw) };
            let cross = if horizontal { rect.height } else { rect.width };
            if cross_size <= cross {
                let gap = pad;
                let after = center + d * 0.5 + gap;
                let before = center - d * 0.5 - gap - along_size;
                let start = if after + along_size <= len - gap { Some(after) } else { (before >= gap).then_some(before) };
                if let Some(start) = start {
                    let s = bar.span(start, along_size);
                    let b = if horizontal {
                        Rect::new(s.x, track.center().y - bh * 0.5, bw, bh)
                    } else {
                        Rect::new(track.center().x - bw * 0.5, s.y, bw, bh)
                    };
                    ui.with_alpha(alpha, |ui| {
                        let c = &ui.theme.colors;
                        ui.painter.rect(b, &RectStyle::filled(ui.fade(c.surface_bright), pill(b)));
                        let at = Point::new(b.center().x - m.width * 0.5, b.center().y - m.height * 0.5);
                        self.bubble_label.draw(ui.painter, at, ui.fade(c.on_surface));
                    });
                }
            }
        }

        let (animating, wake) = self.frame_need(now);
        if animating {
            ui.request_frame();
        }
        if let Some(at) = wake {
            ui.wake_at(at);
        }
        self.damage.report(ui, rect, animating);
    }
}

/// Durum renkleri. Hover/basılı state layer'ı (`layer` = opaklık) ize ve dolguya karışır. Devre
/// dışıyken bütün bileşen soluk: topuz dolgunun bir ton üstü (aynı soluk ton üst üste).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SliderColors {
    pub(crate) rest: Color,
    pub(crate) fill: Color,
    pub(crate) knob: Color,
}

impl SliderColors {
    pub(crate) fn new(theme: &Theme, enabled: bool, layer: f32) -> Self {
        let c = &theme.colors;
        if enabled {
            // Yüksek karşıtlıkta: etkin olmayan iz yüzeylere ≥ 3:1 (`style::track`); dolgudaki
            // hover katmanı topuzu (değer işareti) ≥ 4.5'te tutacak kadarla sınırlı.
            let fill_layer = if theme.contrast.is_high() {
                state_layer_under(theme, c.on_primary, layer, &[c.primary], &[c.on_primary]).a
            } else {
                layer
            };
            Self {
                rest: track(theme).lerp(&c.on_surface, layer),
                fill: c.primary.lerp(&c.on_primary, fill_layer),
                knob: c.on_primary,
            }
        } else {
            let rest = disabled_container(theme);
            let fill = disabled_content(theme, c.on_surface, &[rest]);
            // Devre dışı: bileşenin tamamı soluk okunur (WCAG 1.4.11 pasif bileşeni muaf tutar). Topuz
            // dolgunun bir ton üstü: aynı soluk içerik tonu dolgunun üstüne biner.
            Self { rest, fill, knob: fill }
        }
    }
}

/// Topuz merkezinin eksen üzerindeki yeri: `[r, len − r]` (`r = t / 2`).
fn knob_center(len: f32, t: f32, v: f32) -> f32 {
    let r = t * 0.5;
    (r + v * (len - t).max(0.0)).clamp(r, (len - r).max(r))
}

/// [`knob_center`]'ın tersi: eksen konumundan değer (sınırlanmamış).
fn value_from_center(len: f32, t: f32, along: f32) -> f32 {
    (along - t * 0.5) / (len - t).max(1.0)
}

/// Topuz çapı: dinlenmede izden `space(1)` içeride, `grow = 1`'de iz kalınlığı.
fn knob_diameter(theme: &Theme, t: f32, grow: f32) -> f32 {
    let base = (t - theme.space(1) * 2.0).max(t * 0.5);
    (base + (t - base) * grow).min(t)
}

fn sanitize(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

// Erişilebilirlik (ADR-0011).
impl Slider {
    /// `Role::Slider`; normalize değer `range`'e eşlenir (ör. `0.0..=100.0` yüzde, büyüteç
    /// `1.0..=8.0`). Ad görünmez: çağıran `.label(..)` verir; birimli metin gerekiyorsa
    /// `.value("%64")`.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash, range: RangeInclusive<f64>) -> AccessNode<'t> {
        let (lo, hi) = (*range.start(), *range.end());
        let span = hi - lo;
        let node = tree
            .push(key, Role::Slider)
            .bounds(self.rect)
            .enabled(self.enabled)
            .numeric(lo + f64::from(self.value) * span, lo, hi, f64::from(self.step()) * span)
            .action(Action::Focus)
            .action(Action::Increment)
            .action(Action::Decrement)
            .action(Action::SetValue);
        if self.orientation == Orientation::Vertical {
            let mut node = node;
            node.node_mut().set_orientation(accesskit::Orientation::Vertical);
            node
        } else {
            node
        }
    }

    /// AT eylemi (`Increment`/`Decrement` bir adım, `SetValue` `range` biriminde): değer
    /// değiştiyse `event = Some(normalize yeni değer)` (klavyeyle aynı; balon kısa süre görünür).
    pub fn access_action(&mut self, action: AccessAction, range: RangeInclusive<f64>, now: Instant) -> Response<f32> {
        if !self.enabled {
            return Response::none();
        }
        let v = match action {
            AccessAction::Increment => self.value + self.step(),
            AccessAction::Decrement => self.value - self.step(),
            AccessAction::SetValue(v) => {
                let (lo, hi) = (*range.start(), *range.end());
                if hi <= lo {
                    return Response::none();
                }
                ((v - lo) / (hi - lo)) as f32
            }
            _ => return Response::none(),
        };
        self.show_bubble(Some(BUBBLE_HOLD), now);
        let v = self.quantize(sanitize(v));
        self.commit(v, now, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;
    use crate::style::{contrast_ratio, over};

    fn slider() -> (Slider, Theme, Instant) {
        let theme = Theme::default();
        let mut s = Slider::new(&theme, Orientation::Horizontal, 0.5);
        s.set_rect(Rect::new(0.0, 0.0, 220.0, 44.0));
        (s, theme, Instant::now())
    }

    #[test]
    fn press_and_drag_follow_pointer() {
        let (mut s, theme, t0) = slider();
        let t = Slider::track_thickness(&theme);
        // İmlecin altındaki nokta topuz merkezi: x = r + v·(len − t).
        let x = |v: f32| t * 0.5 + v * (220.0 - t);
        assert_eq!(s.pointer(&press(x(0.25), 20.0), t0).event, Some(0.25));
        assert_eq!(s.pointer(&motion(x(0.75), 20.0), t0).event, Some(0.75));
        // Sürüklerken görüntü gecikmesiz.
        assert_eq!(s.shown.value(t0), 0.75);
        // İz dışına sürükleme sınırlanır.
        assert_eq!(s.pointer(&motion(500.0, 90.0), t0).event, Some(1.0));
        let _ = s.pointer(&release(500.0, 90.0), t0);
        assert!(s.pointer(&motion(10.0, 10.0), t0).event.is_none());
    }

    #[test]
    fn knob_stays_inside_track_ends_and_mapping_round_trips() {
        let theme = Theme::default();
        let t = Slider::track_thickness(&theme);
        let len = 220.0;
        for v in [-0.02, 0.0, 0.3, 1.0, 1.02] {
            let c = knob_center(len, t, v);
            let d = knob_diameter(&theme, t, 1.0);
            assert!(c - d * 0.5 >= -1e-3 && c + d * 0.5 <= len + 1e-3, "v={v}: {c}");
            // Dolgu en az topuz yuvası kadar.
            assert!(c + t * 0.5 >= t - 1e-3);
        }
        for v in [0.0, 0.1, 0.5, 0.9, 1.0] {
            assert!((value_from_center(len, t, knob_center(len, t, v)) - v).abs() < 1e-5, "v={v}");
        }
        // Dikey: aşağıdan yukarı aynı eşleme.
        let mut s = Slider::new(&theme, Orientation::Vertical, 0.0);
        s.set_rect(Rect::new(0.0, 0.0, 44.0, len));
        let t0 = Instant::now();
        let y = len - knob_center(len, t, 0.8);
        assert!((s.pointer(&press(22.0, y), t0).event.unwrap() - 0.8).abs() < 1e-5);
        // Uç yuvalar: 0'ın topuz merkezine basmak 0 verir, en uca basmak 1.
        assert_eq!(s.pointer(&motion(22.0, len - t * 0.5), t0).event, Some(0.0));
        assert_eq!(s.pointer(&motion(22.0, 0.0), t0).event, Some(1.0));
    }

    #[test]
    fn touch_cancel_ends_drag_but_mouse_leave_does_not() {
        let (mut s, _, t0) = slider();
        let _ = s.pointer(&press(100.0, 20.0), t0);
        let _ = s.pointer(&PointerEvent::new(Point::new(100.0, 20.0), PointerKind::Leave), t0);
        assert!(s.dragging, "fare: örtük tutuşta Leave sürüklemeyi bitirmez");
        let _ = s.pointer(&release(100.0, 20.0), t0);
        let _ = s.pointer(&PointerEvent::touch(Point::new(100.0, 20.0), PointerKind::Press(Button::Left)), t0);
        assert!(s.dragging);
        let _ = s.pointer(&PointerEvent::touch(Point::new(100.0, 20.0), PointerKind::Leave), t0);
        assert!(!s.dragging, "dokunma iptali sürüklemeyi bitirir");
        assert_eq!(s.pointer(&motion(150.0, 20.0), t0).event, None);
    }

    #[test]
    fn states_grow_knob_noticeably() {
        let (mut s, theme, t0) = slider();
        let later = t0 + Duration::from_secs(2);
        let _ = s.pointer(&motion(100.0, 20.0), t0);
        let hover = knob_diameter(&theme, s.thick, s.grow.value(later));
        let rest = knob_diameter(&theme, s.thick, 0.0);
        assert!(hover - rest >= theme.space(1), "hover farkı fark edilir: {rest} → {hover}");
        let _ = s.pointer(&press(100.0, 20.0), later);
        assert_eq!(s.grow.target(), 1.0, "sürüklerken iz kalınlığı");
        let _ = s.pointer(&release(100.0, 20.0), later);
        let _ = s.pointer(&PointerEvent::new(Point::new(400.0, 20.0), PointerKind::Leave), later);
        assert_eq!(s.grow.target(), 0.0);
        let _ = s.key(&key(Key::Right), later);
        assert_eq!(s.grow.target(), 1.0, "klavye odağında da büyür");
    }

    #[test]
    fn keyboard_change_shows_bubble_then_it_fades() {
        let (mut s, _, t0) = slider();
        let _ = s.key(&key(Key::Right), t0);
        assert_eq!(s.bubble.target(), 1.0);
        assert!(s.bubble_until.is_some_and(|u| u > t0));
    }

    #[test]
    fn bubble_hold_wakes_instead_of_frames() {
        let (mut s, _, t0) = slider();
        let _ = s.key(&key(Key::Right), t0);
        let until = t0 + BUBBLE_HOLD;
        // Belirme sürerken kare; sönme anı baştan uyanma olarak bildirilir.
        let (animating, wake) = s.frame_need(t0);
        assert!(animating);
        assert_eq!(wake, Some(until));
        // Balon görünür ve durağan: kare istenmez, uyanma anı aynı.
        let idle = until - Duration::from_millis(1);
        s.advance_bubble(idle);
        assert_eq!(s.bubble.value(idle), 1.0, "balon tam görünür");
        assert_eq!(s.frame_need(idle), (false, Some(until)));
        // An geldi: sönme başlar, normal kare; uyanma kalmaz.
        s.advance_bubble(until);
        assert_eq!(s.bubble.target(), 0.0);
        assert_eq!(s.frame_need(until), (true, None));
        let gone = until + Duration::from_secs(2);
        assert_eq!(s.frame_need(gone), (false, None), "söndükten sonra boşta sıfır iş");
    }

    #[test]
    fn dragging_holds_bubble_without_wake() {
        let (mut s, _, t0) = slider();
        let _ = s.pointer(&press(100.0, 20.0), t0);
        assert_eq!(s.frame_need(t0).1, None, "sürüklerken süre işlemez");
        let t1 = t0 + Duration::from_secs(2);
        let _ = s.pointer(&release(100.0, 20.0), t1);
        assert_eq!(s.frame_need(t1).1, Some(t1 + BUBBLE_HOLD_AFTER_DRAG));
    }

    #[test]
    fn knob_contrast_on_default_and_generated_palettes() {
        use nuxut_palette::{Mode, Palette, Variant};
        let mut palettes = vec![Theme::default().colors];
        for seed in ["#4285f4", "#c0602a", "#2e7d32", "#9c27b0", "#ffeb3b"] {
            for mode in [Mode::Dark, Mode::Light] {
                for v in Variant::ALL {
                    let p = Palette::generate(Color::from_hex(seed).unwrap(), v, mode, 0.0);
                    let mut colors = Theme::default().colors;
                    colors.apply(&nuxut_palette::export::generated_palette(&p, None).colors);
                    palettes.push(colors);
                }
            }
        }
        for colors in palettes {
            let theme = Theme { colors, ..Theme::default() };
            let bg = colors.surface_container;
            for (enabled, layer) in [(true, 0.0), (true, 0.08), (true, 0.12)] {
                let k = SliderColors::new(&theme, enabled, layer);
                let r = contrast_ratio(k.knob, over(k.fill, bg));
                assert!(r >= 3.0, "topuz/dolgu {r:.2} (etkin={enabled}, katman={layer}, {})", colors.primary.to_hex());
            }
            // Odak noktası topuza karşı.
            let r = contrast_ratio(colors.primary, colors.on_primary);
            assert!(r >= 3.0, "odak noktası {r:.2}");
            // Devre dışı soluk: topuz dolgudan ayırt edilir ama etkin topuzdan belirgin biçimde
            // daha az öne çıkar (kart zeminine göre); hiçbir parçası opak değil.
            let d = SliderColors::new(&theme, false, 0.0);
            let fill_seen = over(d.fill, bg);
            let knob_seen = over(d.knob, fill_seen);
            assert!(d.knob.a < 1.0 && d.fill.a < 1.0 && d.rest.a < 1.0);
            assert!(contrast_ratio(knob_seen, fill_seen) > 1.1, "topuz dolgudan ayırt edilir");
            let e = SliderColors::new(&theme, true, 0.0);
            assert!(contrast_ratio(knob_seen, bg) < contrast_ratio(e.knob, over(e.fill, bg)).max(contrast_ratio(e.fill, bg)));
        }
    }

    #[test]
    fn external_value_ignored_while_dragging() {
        let (mut s, _, t0) = slider();
        let _ = s.pointer(&press(50.0, 20.0), t0);
        assert!(!s.set_value(0.9, t0));
        let _ = s.pointer(&release(50.0, 20.0), t0);
        assert!(s.set_value(0.9, t0));
    }

    #[test]
    fn steps_and_keys() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = Slider::new(&theme, Orientation::Vertical, 0.33).with_steps(10);
        assert!((s.value() - 0.3).abs() < 1e-6);
        assert!((s.key(&key(Key::Up), t0).event.unwrap() - 0.4).abs() < 1e-6);
        assert_eq!(s.key(&key(Key::End), t0).event, Some(1.0));
        assert_eq!(s.key(&key(Key::Right), t0).event, None, "üst sınırda değişim yok");
    }

    #[test]
    fn nan_is_sanitized() {
        let (mut s, _, t0) = slider();
        s.set_value(f32::NAN, t0);
        assert_eq!(s.value(), 0.0);
    }
}
