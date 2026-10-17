use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{Painter, RectStyle};
use nuxut_theme::{STATE_HOVER, STATE_PRESSED, Theme};

use crate::input::{Button, PointerEvent, PointerKind};

/// Çift tıklama penceresi (metin kelime seçimi, dosya ızgarasında açma).
pub(crate) const DOUBLE_CLICK: std::time::Duration = std::time::Duration::from_millis(400);

/// [`Interaction::handle`] sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tap {
    /// Görünüş değişti.
    pub repaint: bool,
    /// Tıklama tamamlandı.
    pub clicked: bool,
}

/// Hover/basılı durumunu `motion.effects` yayıyla state layer opaklığına çevirir.
///
/// Tipik kullanım: pointer olayında [`pointer`](Self::pointer), çizimde
/// [`draw_layer`](Self::draw_layer); [`is_animating`](Self::is_animating) `true` iken kare
/// iste.
#[derive(Debug, Clone)]
pub struct Interaction {
    hovered: bool,
    pressed: bool,
    armed: bool,
    layer: Animated<f32>,
}

impl Interaction {
    pub fn new(theme: &Theme) -> Self {
        Self { hovered: false, pressed: false, armed: false, layer: Animated::new(0.0, theme.motion.effects) }
    }

    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    pub fn is_pressed(&self) -> bool {
        self.pressed
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.layer.set_motion(theme.motion.effects, now);
    }

    /// İmleç konumu (`None` = yüzeyden çıktı) ve düğme durumuyla günceller. Durum
    /// değiştiyse `true`.
    pub fn pointer(&mut self, bounds: Rect, position: Option<Point>, button_down: bool, now: Instant) -> bool {
        let hovered = position.is_some_and(|p| bounds.contains(p));
        // Dışarı sürüklenince bırakılmış sayılır.
        let pressed = hovered && button_down;
        self.set_state(hovered, pressed, now)
    }

    /// Olay tabanlı kullanım: hover/basılı durumunu olaydan günceller ve tıklamayı tanır
    /// (sol düğme içeride basılıp içeride bırakıldı). Basılıyken dışarı çıkılırsa basılı
    /// görünüm kalkar, geri girilince döner; dışarıda bırakılırsa tıklama yoktur.
    pub fn handle(&mut self, bounds: Rect, event: &PointerEvent, now: Instant) -> Tap {
        let inside = bounds.contains(event.position);
        let mut tap = Tap::default();
        match event.kind {
            PointerKind::Leave => {
                self.armed = false;
                tap.repaint = self.set_state(false, false, now);
                return tap;
            }
            PointerKind::Press(Button::Left) if inside => self.armed = true,
            PointerKind::Release(Button::Left) => {
                tap.clicked = self.armed && inside;
                self.armed = false;
            }
            _ => {}
        }
        tap.repaint = self.set_state(inside, inside && self.armed, now) || tap.clicked;
        tap
    }

    /// Basma içeride başladı ve düğme hâlâ basılı mı (sürükleme izleyen bileşenler için).
    pub fn is_armed(&self) -> bool {
        self.armed
    }

    pub fn reset(&mut self, now: Instant) -> bool {
        self.armed = false;
        self.set_state(false, false, now)
    }

    pub fn set_state(&mut self, hovered: bool, pressed: bool, now: Instant) -> bool {
        if hovered == self.hovered && pressed == self.pressed {
            return false;
        }
        self.hovered = hovered;
        self.pressed = pressed;
        let target = if pressed {
            STATE_PRESSED
        } else if hovered {
            STATE_HOVER
        } else {
            0.0
        };
        self.layer.set_target(target, now);
        true
    }

    /// State layer opaklığı (0..=0.12).
    pub fn opacity(&self, now: Instant) -> f32 {
        self.layer.value(now).max(0.0)
    }

    /// `on_surface` üzerinden state layer rengi (doğrudan yüzeyde; yüksek karşıtlıkta
    /// `style::state_layer` ile sınırlı).
    pub fn layer_color(&self, theme: &Theme, now: Instant) -> Color {
        crate::style::state_layer(theme, theme.colors.on_surface, &[], self.opacity(now))
    }

    /// State layer'ı `container` (kap dolgusu; `TRANSPARENT` = doğrudan yüzey) üstünde çizer:
    /// renk `content`'ten türetilir; yüksek karşıtlıkta `content` birleşime karşı ≥ 4.5 kalır
    /// (`style::state_layer`). Framework bileşenleri bunu kullanır;
    /// [`draw_layer_with`](Self::draw_layer_with) ham, sınırsız eşdeğerdir.
    pub fn draw_state_layer(&self, ui: &mut crate::ui::Ui, rect: Rect, radii: Corners, content: Color, container: Color) {
        self.draw_state_layer_under(ui, rect, radii, content, container, &[content]);
    }

    /// [`draw_state_layer`](Self::draw_state_layer), katman rengi (`layer`) üstündeki
    /// içeriklerden (`contents`) farklıyken.
    pub fn draw_state_layer_under(&self, ui: &mut crate::ui::Ui, rect: Rect, radii: Corners, layer: Color, container: Color, contents: &[Color]) {
        let color = crate::style::state_layer_under(ui.theme, layer, self.opacity(ui.now), &[container], contents);
        if color.a > 0.0 {
            let c = ui.fade(color);
            ui.painter.rect(rect, &RectStyle::filled(c, radii));
        }
    }

    pub fn draw_layer(&self, painter: &mut Painter, rect: Rect, radii: Corners, theme: &Theme, now: Instant) {
        let color = self.layer_color(theme, now);
        if color.a > 0.0 {
            painter.rect(rect, &RectStyle::filled(color, radii));
        }
    }

    /// Renkli zemin üzerinde: state layer içerik renginden (`on_primary` gibi) türetilir.
    pub fn draw_layer_with(&self, painter: &mut Painter, rect: Rect, radii: Corners, content: Color, now: Instant) {
        let color = content.multiply_alpha(self.opacity(now));
        if color.a > 0.0 {
            painter.rect(rect, &RectStyle::filled(color, radii));
        }
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.layer.is_settled(now)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn hover_then_press_then_leave() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = Interaction::new(&theme);
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(i.pointer(r, Some(Point::new(5.0, 5.0)), false, t0));
        assert!(i.is_animating(t0));
        let t1 = t0 + Duration::from_secs(2);
        assert!(!i.is_animating(t1));
        assert!((i.opacity(t1) - STATE_HOVER).abs() < 1e-3);
        assert!(i.pointer(r, Some(Point::new(5.0, 5.0)), true, t1));
        assert!(i.is_pressed());
        assert!(i.pointer(r, Some(Point::new(50.0, 5.0)), true, t1));
        assert!(!i.is_hovered() && !i.is_pressed());
        assert!(!i.pointer(r, None, false, t1));
    }

    #[test]
    fn effects_spring_does_not_overshoot() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = Interaction::new(&theme);
        i.set_state(true, false, t0);
        let max = (0..100).map(|k| i.opacity(t0 + Duration::from_millis(k * 10))).fold(0.0, f32::max);
        assert!(max <= STATE_HOVER + 1e-4);
    }
}
