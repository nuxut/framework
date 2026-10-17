//! Arama alanı: hap biçimli [`TextInput`] + baştaki arama ikonu + metin varken beliren temizle
//! düğmesi. Esc metni temizler (boşsa `Cancel` iletilir).

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;

use crate::button::{IconButton, IconButtonVariant};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::text_input::{TextInput, TextInputEvent};
use crate::typography::TextRole;
use crate::ui::{Response, Ui};

#[derive(Debug)]
pub struct SearchField {
    input: TextInput,
    icon: IconView,
    clear: IconButton,
    rect: Rect,
}

impl SearchField {
    pub fn new(theme: &Theme, placeholder: &str) -> Self {
        let side = theme.space(10);
        Self {
            input: TextInput::new(theme).with_placeholder(placeholder).with_insets(side - theme.space(2), side - theme.space(2)).pill_shaped(true),
            icon: IconView::with_role(theme, Icon::named(IconName::Search), TextRole::IconSmall),
            clear: IconButton::new(theme, Icon::named(IconName::Close), IconButtonVariant::Standard),
            rect: Rect::ZERO,
        }
    }

    pub fn text(&self) -> &str {
        self.input.text()
    }

    pub fn input(&self) -> &TextInput {
        &self.input
    }

    pub fn input_mut(&mut self) -> &mut TextInput {
        &mut self.input
    }

    /// Yer tutucu metni (ör. dil değişince); aynı metin iş yapmaz, farklıysa yeniden çizilir.
    pub fn set_placeholder(&mut self, text: &str) {
        self.input.set_placeholder(text);
    }

    pub fn set_focused(&mut self, focused: bool, now: Instant) {
        self.input.set_focused(focused, now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.input.set_theme(theme, now);
        self.icon.set_theme(theme, now);
        self.clear.set_theme(theme, now);
    }

    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        self.input.preferred_size(theme, width)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        self.rect = rect;
        self.input.set_rect(rect);
        let s = theme.space(8);
        self.clear.set_rect(Rect::new(rect.right() - theme.space(2) - s, rect.center().y - s * 0.5, s, s));
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn clear_text(&mut self) -> Response<TextInputEvent> {
        if self.input.text().is_empty() {
            return Response::none();
        }
        self.input.set_text("");
        Response::event(TextInputEvent::Changed)
    }

    /// Metin girişi olayları; temizle düğmesi `Changed` üretir.
    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<TextInputEvent> {
        if !self.input.text().is_empty() {
            let r = self.clear.pointer(event, now);
            if r.event.is_some() {
                return self.clear_text();
            }
            if self.clear.rect().contains(event.position) {
                return r.discard();
            }
        }
        self.input.pointer(text, theme, event, now)
    }

    /// Esc: metin varsa temizler, yoksa `Cancel`.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<TextInputEvent> {
        if event.key == Key::Escape && event.state != KeyState::Released && !self.input.text().is_empty() {
            return self.clear_text();
        }
        self.input.key(event, now)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.input.text().is_empty()
            && let Some(c) = self.clear.cursor(p)
        {
            return Some(c);
        }
        self.input.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        self.input.draw(ui);
        let iw = self.icon.size().width;
        let c = Point::new(self.rect.x + theme.space(4) + iw * 0.5, self.rect.center().y);
        self.icon.draw_centered(ui.painter, c, ui.fade(theme.colors.on_surface_variant));
        if !self.input.text().is_empty() {
            self.clear.draw(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn placeholder_can_change() {
        let theme = Theme::default();
        let mut s = SearchField::new(&theme, "Ara…");
        assert_eq!(s.input().placeholder(), "Ara…");
        s.set_placeholder("Search…");
        assert_eq!(s.input().placeholder(), "Search…");
        assert_eq!(s.text(), "", "metin değişmez");
    }

    #[test]
    fn clear_button_and_escape() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = SearchField::new(&theme, "Ara…");
        s.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 48.0));
        s.set_focused(true, t0);
        for ch in "wifi".chars() {
            let _ = s.key(&typed(ch), t0);
        }
        assert_eq!(s.text(), "wifi");
        let x = s.clear.rect().center();
        assert_eq!(s.cursor(x), Some(CursorShape::Pointer));
        assert_eq!(s.cursor(Point::new(100.0, 20.0)), Some(CursorShape::Text));
        let _ = s.pointer(&mut text, &theme, &motion(x.x, x.y), t0);
        let _ = s.pointer(&mut text, &theme, &press(x.x, x.y), t0);
        assert_eq!(s.pointer(&mut text, &theme, &release(x.x, x.y), t0).event, Some(TextInputEvent::Changed));
        assert_eq!(s.text(), "");
        let _ = s.key(&typed('a'), t0);
        assert_eq!(s.key(&key(Key::Escape), t0).event, Some(TextInputEvent::Changed));
        assert_eq!(s.key(&key(Key::Escape), t0).event, Some(TextInputEvent::Cancel));
    }
}
