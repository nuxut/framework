//! Popover: bağlantı dikdörtgeninden (Menu ile aynı hareketle) blob olarak büyüyen kap.
//! İçerik çağıranındır (ör. [`ColorPicker`](crate::ColorPicker)); popover onu iç boşluklu
//! dikdörtgene kırpar ve büyürken/sönerken soldurur. Dışarı basma ya da Esc → `Dismissed`.
//!
//! Shell kendi blob alanına [`Popover::shape`]'i ekler; bağımsız kullanımda
//! [`Popover::draw_background`] şekli gölgesiyle çizer. İçerik her iki durumda da
//! [`Popover::draw_content`] içinde çizilir.

use std::time::Instant;

use nuxut_core::{Insets, Rect, Size};
use nuxut_render::BlobShape;
use nuxut_theme::Theme;

use crate::grow::Grow;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::menu::MenuAnchor;
use crate::ui::{DamageState, Response, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopoverEvent {
    Dismissed,
}

#[derive(Debug, Clone)]
pub struct Popover {
    grow: Grow,
    content_size: Size,
    damage: DamageState,
}

impl Popover {
    pub fn new(theme: &Theme) -> Self {
        Self { grow: Grow::new(theme), content_size: Size::ZERO, damage: DamageState::new() }
    }

    /// İç boşluk (kenardan içeriğe).
    pub fn padding(theme: &Theme) -> f32 {
        theme.space(4)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.grow.set_theme(theme, now);
        self.damage.mark();
    }

    /// `content_size`: içeriğin boyutu (iç boşluk hariç). Açıkken çağrılırsa yeni bağlantıya morf.
    pub fn open(&mut self, theme: &Theme, anchor: MenuAnchor, content_size: Size, bounds: Rect, now: Instant) {
        self.content_size = content_size;
        self.grow.open(anchor, bounds, Self::outer(theme, content_size), now);
        self.damage.mark();
    }

    /// İçerik boyutu değişti: kap yeni boyuta yayla morf olur.
    pub fn set_content_size(&mut self, theme: &Theme, size: Size, now: Instant) {
        if self.content_size != size {
            self.content_size = size;
            self.grow.resize(Self::outer(theme, size), now);
            self.damage.mark();
        }
    }

    fn outer(theme: &Theme, content: Size) -> Size {
        let p = Self::padding(theme) * 2.0;
        Size::new(content.width + p, content.height + p)
    }

    pub fn close(&mut self, now: Instant) {
        if self.grow.close(now) {
            self.damage.mark();
        }
    }

    pub fn is_open(&self) -> bool {
        self.grow.is_open()
    }

    pub fn is_visible(&self) -> bool {
        self.grow.is_visible()
    }

    /// Açıkken input bölgesi (hedef dikdörtgen).
    pub fn input_rect(&self) -> Option<Rect> {
        self.is_open().then(|| self.grow.target())
    }

    /// İçeriğin dinlenme (hedef) yeri: girdi yönlendirmesi ve içerik yerleşimi için.
    pub fn content_target(&self, theme: &Theme) -> Rect {
        self.grow.target().inset(Insets::uniform(Self::padding(theme)))
    }

    /// Blob şekli (shell kendi alanına ekler; karede bir kez). Kapalıyken `None`.
    pub fn shape(&mut self, now: Instant, theme: &Theme) -> Option<BlobShape> {
        self.grow.shape(now, theme.shape.radius_large)
    }

    /// Bu karedeki içerik dikdörtgeni (görünen şekil, iç boşluklu).
    pub fn content_rect(&mut self, now: Instant, theme: &Theme) -> Option<Rect> {
        self.shape(now, theme).map(|s| s.visual_rect().inset(Insets::uniform(Self::padding(theme))))
    }

    /// Bağımsız kullanım: şekli `surface` dolgusu ve gölgeyle çizer.
    pub fn draw_background(&mut self, ui: &mut Ui) {
        let shape = self.shape(ui.now, ui.theme);
        self.grow.draw_standalone(ui, shape);
    }

    /// İçeriği çizdirir: `f(ui, rect)` görünen şeklin iç boşluklu dikdörtgenine kırpılır ve
    /// açılış/kapanış doluluğuna göre soldurulur. İçerik yerleşimi `content_target`'e göre
    /// yapılmalı (esneme sırasında içerik kaymasın); `rect` kırpma içindir.
    pub fn draw_content(&mut self, ui: &mut Ui, f: impl FnOnce(&mut Ui, Rect)) {
        let now = ui.now;
        let theme = ui.theme;
        let Some(shape) = self.shape(now, theme) else {
            self.damage.report_hidden(ui);
            return;
        };
        let visual = shape.visual_rect();
        let reveal = self.grow.reveal(visual);
        if reveal > 0.0 && visual.width > 0.5 && visual.height > 0.5 {
            let rect = self.content_target(theme);
            ui.painter.push_clip(visual);
            ui.with_alpha(reveal, |ui| f(ui, rect));
            ui.painter.pop_clip();
        }
        let animating = self.grow.is_animating(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, visual.expand(1.0), animating);
        self.grow.settle(now);
    }

    /// Bağımsız kullanım, kaplama katmanında: şekil + içerik en üstte, kapsayıcı kırpmasından
    /// bağımsız ([`Ui::overlay`]); açıkken girdi bölgesini bildirir ([`Ui::overlay_input`]).
    /// Tetikleyicinin yanında, kırpılı kapsayıcının (kaydırma alanı, kart) içinden çağrılabilir.
    pub fn draw_overlay(&mut self, ui: &mut Ui, f: impl FnOnce(&mut Ui, Rect)) {
        if let Some(r) = self.input_rect() {
            ui.overlay_input(r);
        }
        ui.overlay(|ui| {
            self.draw_background(ui);
            self.draw_content(ui, f);
        });
    }

    /// İmleç: açıkken kap içi `Default` (içeriğin kendi imleci önce sorulmalı), dışarıda `None`.
    pub fn cursor(&self, p: nuxut_core::Point) -> Option<CursorShape> {
        (self.is_open() && self.grow.target().contains(p)).then_some(CursorShape::Default)
    }

    /// Dışarı basma → kapanır ve `Dismissed`. İçerideki olayları çağıran içeriğe iletir.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<PopoverEvent> {
        if !self.is_open() {
            return Response::none();
        }
        if let PointerKind::Press(_) = event.kind
            && !self.grow.target().contains(event.position)
        {
            self.close(now);
            return Response::event(PopoverEvent::Dismissed);
        }
        Response::none()
    }

    /// Esc → kapanır ve `Dismissed`.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<PopoverEvent> {
        if self.is_open() && event.state == KeyState::Pressed && event.key == Key::Escape {
            self.close(now);
            return Response::event(PopoverEvent::Dismissed);
        }
        Response::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn opens_with_padding_and_dismisses() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut p = Popover::new(&theme);
        let anchor = MenuAnchor { gap: theme.space(1), ..MenuAnchor::below(Rect::new(100.0, 100.0, 40.0, 40.0)) };
        p.open(&theme, anchor, Size::new(200.0, 120.0), Rect::new(0.0, 0.0, 800.0, 600.0), t0);
        let target = p.input_rect().unwrap();
        let pad = Popover::padding(&theme);
        assert_eq!(target.size(), Size::new(200.0 + pad * 2.0, 120.0 + pad * 2.0));
        assert_eq!(p.content_target(&theme).size(), Size::new(200.0, 120.0));
        // İçeride basma kapatmaz.
        let c = target.center();
        assert!(p.pointer(&press(c.x, c.y), t0).event.is_none());
        assert_eq!(p.pointer(&press(5.0, 5.0), t0).event, Some(PopoverEvent::Dismissed));
        assert!(!p.is_open());
        p.open(&theme, anchor, Size::new(10.0, 10.0), Rect::new(0.0, 0.0, 800.0, 600.0), t0);
        assert_eq!(p.key(&key(Key::Escape), t0).event, Some(PopoverEvent::Dismissed));
    }
}
