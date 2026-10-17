//! Snackbar / toast: alanın altında (ortalı) `spatial` hareketle yükselerek giren kısa bildirim;
//! opsiyonel eylem düğmesi. Zaman aşımı çağıranın zamanlayıcısıdır (boşta iş yok):
//! uygulama `show` sonrası bir timer kurar, dolunca `hide` der.

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, ShadowStyle, TextSystem};
use nuxut_theme::Theme;

use crate::button::{Button, ButtonVariant};
use crate::input::{CursorShape, PointerEvent};
use crate::label::Label;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Mesajın en uzun hâli (bayt).
pub const MAX_SNACKBAR_TEXT: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnackbarEvent {
    /// Eylem düğmesine basıldı (snackbar kapanmaya başlar).
    Action,
}

#[derive(Debug, Clone)]
pub struct Snackbar {
    label: Label,
    action: Option<Button>,
    visible: bool,
    /// 0 = alanın altında gizli, 1 = yerinde.
    rise: Animated<f32>,
    fade: Animated<f32>,
    area: Rect,
    rect: Rect,
    damage: DamageState,
}

impl Snackbar {
    pub fn new(theme: &Theme) -> Self {
        Self {
            label: Label::new("", text_style(theme, TextRole::Body).with_max_lines(2)),
            action: None,
            visible: false,
            rise: Animated::new(0.0, theme.motion.spatial),
            fade: Animated::new(0.0, theme.motion.effects),
            area: Rect::ZERO,
            rect: Rect::ZERO,
            damage: DamageState::new(),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.label.set_style(text_style(theme, TextRole::Body).with_max_lines(2));
        self.rise.set_motion(theme.motion.spatial, now);
        self.fade.set_motion(theme.motion.effects, now);
        if let Some(a) = &mut self.action {
            a.set_theme(theme, now);
        }
        self.damage.mark();
    }

    /// Gösterir (görünürken çağrılırsa içerik değişir, yeniden yükselmez). `area`: snackbar'ın
    /// alt kenarına ortalandığı alan.
    pub fn show(&mut self, text: &mut TextSystem, theme: &Theme, message: &str, action: Option<&str>, area: Rect, now: Instant) {
        self.label.set_text(crate::style::clip_text(message, MAX_SNACKBAR_TEXT));
        self.action = action.map(|a| Button::new(theme, ButtonVariant::Text).with_label(theme, a));
        self.area = area;
        if !self.visible {
            self.visible = true;
            self.rise.set_target(1.0, now);
            self.fade.set_target(1.0, now);
        }
        self.layout(text, theme);
        self.damage.mark();
    }

    pub fn hide(&mut self, now: Instant) {
        if self.visible {
            self.visible = false;
            self.rise.set_target(0.0, now);
            self.fade.set_target(0.0, now);
            self.damage.mark();
        }
    }

    fn layout(&mut self, text: &mut TextSystem, theme: &Theme) {
        let pad = theme.space(4);
        let max_w = (self.area.width - theme.space(8)).min(theme.space(140)).max(theme.space(40));
        let action_s = self.action.as_mut().map_or(Size::ZERO, |a| a.preferred_size(text, theme));
        let label_max = max_w - pad * 2.0 - action_s.width - if self.action.is_some() { theme.space(2) } else { 0.0 };
        self.label.set_max_width(Some(label_max.max(0.0)));
        let ls = self.label.measure(text);
        let w = (pad * 2.0 + ls.width + action_s.width + if self.action.is_some() { theme.space(2) } else { 0.0 }).ceil();
        let h = (ls.height + pad * 2.0).max(theme.space(12)).max(action_s.height + theme.space(2)).ceil();
        let x = (self.area.center().x - w * 0.5).round();
        let y = self.area.bottom() - theme.space(6) - h;
        self.rect = Rect::new(x, y, w, h);
        if let Some(a) = &mut self.action {
            a.set_rect(Rect::new(self.rect.right() - theme.space(1) - action_s.width, self.rect.center().y - action_s.height * 0.5, action_s.width, action_s.height));
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<SnackbarEvent> {
        if !self.visible {
            return Response::none();
        }
        let Some(a) = &mut self.action else { return Response::none() };
        let r = a.pointer(event, now);
        if r.event.is_some() {
            self.hide(now);
            return Response::event(SnackbarEvent::Action);
        }
        if r.repaint {
            self.damage.mark();
        }
        r.discard()
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.visible || !self.rect.contains(p) {
            return None;
        }
        Some(self.action.as_ref().and_then(|a| a.cursor(p)).unwrap_or(CursorShape::Default))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let a = self.fade.value(now).clamp(0.0, 1.0);
        let rise = self.rise.value(now);
        let animating = !self.fade.is_settled(now) | !self.rise.is_settled(now);
        // Kapalıyken alanın altına kadar iner.
        let drop = (self.area.bottom() - self.rect.y) * (1.0 - rise);
        let r = self.rect.translate(Point::new(0.0, drop));
        if a > 0.0 {
            let radii = theme.shape.medium();
            ui.with_alpha(a, |ui| {
                if theme.shadow.enabled {
                    let sh = ShadowStyle { color: ui.fade(theme.shadow.color), blur: theme.shadow.blur, offset: Point::new(0.0, theme.shadow.offset_y), spread: 0.0 };
                    ui.painter.shadow(r, radii, &sh);
                }
                ui.painter.rect(r, &RectStyle::filled(ui.fade(theme.colors.surface_bright), radii));
                let m = self.label.measure_in(ui.painter);
                let pad = theme.space(4);
                self.label.draw(ui.painter, Point::new(r.x + pad, r.center().y - m.height * 0.5), ui.fade(theme.colors.on_surface));
                if let Some(b) = &mut self.action {
                    let br = b.rect();
                    b.set_rect(br.translate(Point::new(0.0, drop)));
                    b.draw(ui);
                    b.set_rect(br);
                }
            });
        }
        if animating {
            ui.request_frame();
        }
        let extent = self.rect.union(&r).expand(theme.shadow.blur * 1.5 + theme.space(2));
        if a > 0.0 || animating {
            self.damage.report(ui, extent, animating);
        } else {
            self.damage.report_hidden(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn rises_bottom_center_and_action_fires() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = Snackbar::new(&theme);
        s.show(&mut text, &theme, "Ayarlar kaydedildi", Some("Geri al"), Rect::new(0.0, 0.0, 800.0, 600.0), t0);
        let r = s.rect();
        assert!((r.center().x - 400.0).abs() <= 1.0 && r.bottom() < 600.0);
        assert_eq!(s.rise.value(t0), 0.0, "aşağıdan yükselir");
        assert!(s.rise.is_settled(t0 + Duration::from_secs(3)));
        let b = s.action.as_ref().unwrap().rect().center();
        assert_eq!(s.cursor(b), Some(CursorShape::Pointer));
        let _ = s.pointer(&motion(b.x, b.y), t0);
        let _ = s.pointer(&press(b.x, b.y), t0);
        assert_eq!(s.pointer(&release(b.x, b.y), t0).event, Some(SnackbarEvent::Action));
        assert!(!s.is_visible());
    }
}
