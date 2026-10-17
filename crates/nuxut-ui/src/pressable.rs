//! Tıklanabilir bileşenlerin ortak çekirdeği: konum, hover/basılı state layer, basınca
//! "sıkışma" yayı, etkinlik, odak halkası, hasar.

use std::time::Instant;

use nuxut_core::{Color, Corners, Rect};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle};
use nuxut_theme::Theme;

use crate::input::{CursorShape, Key, KeyEvent, PointerEvent};
use crate::interaction::Interaction;
use crate::style::FOCUS_RING;
use crate::ui::{DamageState, Response, Ui};

#[derive(Debug, Clone)]
pub(crate) struct Pressable {
    pub rect: Rect,
    pub interaction: Interaction,
    /// 0 = serbest, 1 = basılı. `spatial_fast` yayıyla (az sönümlü: bırakınca hafif taşar).
    pub squish: Animated<f32>,
    pub enabled: bool,
    pub focus_visible: bool,
    pub damage: DamageState,
}

impl Pressable {
    pub fn new(theme: &Theme) -> Self {
        Self {
            rect: Rect::ZERO,
            interaction: Interaction::new(theme),
            squish: Animated::new(0.0, theme.motion.spatial_fast),
            enabled: true,
            focus_visible: false,
            damage: DamageState::new(),
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.interaction.set_theme(theme, now);
        self.squish.set_motion(theme.motion.spatial_fast, now);
        self.damage.mark();
    }

    pub fn set_rect(&mut self, rect: Rect) -> bool {
        if self.rect == rect {
            return false;
        }
        self.rect = rect;
        self.damage.mark();
        true
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) -> bool {
        if self.enabled == enabled {
            return false;
        }
        self.enabled = enabled;
        if !enabled {
            self.interaction.reset(now);
            self.squish.set_target(0.0, now);
        }
        self.damage.mark();
        true
    }

    pub fn set_visual_state(&mut self, hovered: bool, pressed: bool, now: Instant) {
        if self.interaction.set_state(hovered, pressed, now) {
            self.squish.set_target(if pressed { 1.0 } else { 0.0 }, now);
            self.damage.mark();
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        if self.focus_visible == on {
            return false;
        }
        self.focus_visible = on;
        self.damage.mark();
        true
    }

    /// İşaretçi olayı; tıklamada `event = Some(())`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        if !self.enabled {
            return Response::none();
        }
        let tap = self.interaction.handle(self.rect, event, now);
        if tap.repaint {
            self.damage.mark();
            self.squish.set_target(if self.interaction.is_pressed() { 1.0 } else { 0.0 }, now);
        }
        if tap.clicked {
            // Fare ile etkileşimde odak halkası görünmez (DESIGN.md).
            self.focus_visible = false;
            Response::event(())
        } else {
            Response { repaint: tap.repaint, event: None }
        }
    }

    /// Odaktayken Enter/Space etkinleştirir (kısa bir sıkışma darbesiyle).
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response {
        if !self.enabled || event.state != crate::input::KeyState::Pressed {
            return Response::none();
        }
        match event.key {
            Key::Enter | Key::Space => {
                self.pulse(now);
                Response::event(())
            }
            _ => Response::none(),
        }
    }

    /// Basılmadan etkinleşince (klavye) basma hissi: yay hedefe dokunmadan itilir.
    pub fn pulse(&mut self, now: Instant) {
        self.squish.kick(8.0, now);
        self.damage.mark();
    }

    pub fn squish(&self, now: Instant) -> f32 {
        self.squish.value(now)
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.interaction.is_animating(now) | !self.squish.is_settled(now)
    }

    /// Animasyon varsa kare ister ve hasarı bildirir (`extent` = taşma dahil kaplanan bölge).
    pub fn finish(&mut self, ui: &mut Ui, extent: Rect) {
        let animating = self.is_animating(ui.now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, extent, animating);
    }

    /// İmleç: tıklanabilir `Pointer`, devre dışı `Default`; dışarıda `None`.
    pub fn cursor(&self, p: nuxut_core::Point) -> Option<CursorShape> {
        self.rect.contains(p).then_some(if self.enabled { CursorShape::Pointer } else { CursorShape::Default })
    }

    /// Klavye odak halkası (`primary`, dışa doğru, araya yüzey rengi boşluk).
    pub fn draw_focus(&self, ui: &mut Ui, rect: Rect, radii: Corners) {
        self.draw_focus_on(ui, rect, radii, &[]);
    }

    /// Odak halkası, değdiği renkler `adjacent` iken (boş = tema yüzeyleri; halka bileşenin
    /// İÇİNE çizilecekse bileşenin dolgusu). Yüksek karşıtlıkta hepsine ≥ 3:1
    /// (`style::focus_ring`).
    pub fn draw_focus_on(&self, ui: &mut Ui, rect: Rect, radii: Corners, adjacent: &[Color]) {
        if !self.focus_visible {
            return;
        }
        let color = crate::style::focus_ring(ui.theme, adjacent);
        let gap = FOCUS_RING;
        let ring = rect.expand(gap);
        let r = Corners::new(radii.top_left + gap, radii.top_right + gap, radii.bottom_right + gap, radii.bottom_left + gap);
        let style = RectStyle {
            fill: Color::TRANSPARENT,
            radii: r,
            border: Some(Border { width: FOCUS_RING, color: ui.fade(color) }),
        };
        ui.painter.rect(ring, &style);
    }

    /// Odak halkası dahil kaplanan bölge.
    pub fn extent(&self) -> Rect {
        self.rect.expand(FOCUS_RING * 2.0)
    }
}
