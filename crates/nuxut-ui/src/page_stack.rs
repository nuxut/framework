//! Sayfa yığını ([`PageStack`]): panel içinde alt sayfalara geçiş (hızlı ayarlar → Wi-Fi
//! listesi gibi). Menu ile aynı dil: kap boyutu yeni sayfanın boyutuna `spatial` hareketle morf
//! olur, eski içerik kayıp söner, yeni içerik karşı yönden gelir. İçerikleri çağıran çizer;
//! yığın yalnızca konum/opaklık ve boyutu yönetir. Üstte "‹ Başlık" geri satırı için
//! [`PageHeader`].

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::TextSystem;
use nuxut_theme::Theme;

use crate::grow::Slide;
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::label::Label;
use crate::pressable::Pressable;
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

#[derive(Debug, Clone)]
pub struct PageStack<K: Clone> {
    stack: Vec<K>,
    /// Geçişte eski sayfa.
    leaving: Option<(K, Slide)>,
    /// Kap boyutu (genişlik, yükseklik) — yaylı.
    size: Animated<Point>,
    motion: nuxut_motion::Motion,
}

impl<K: Clone> PageStack<K> {
    pub fn new(theme: &Theme, root: K, size: Size) -> Self {
        Self {
            stack: vec![root],
            leaving: None,
            size: Animated::new(Point::new(size.width, size.height), theme.motion.spatial),
            motion: theme.motion.spatial,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.motion = theme.motion.spatial;
        self.size.set_motion(theme.motion.spatial, now);
    }

    pub fn current(&self) -> &K {
        self.stack.last().expect("yığın boş olmaz")
    }

    pub fn depth(&self) -> usize {
        self.stack.len() - 1
    }

    /// Alt sayfaya geç; kap `size`'a morf olur.
    pub fn push(&mut self, page: K, size: Size, now: Instant) {
        let old = self.current().clone();
        self.leaving = Some((old, Slide::new(1.0, self.motion, now)));
        self.stack.push(page);
        self.set_size(size, now);
    }

    /// Bir üst sayfaya dön (kökteyse `None`). `size`: üst sayfanın boyutu.
    pub fn pop(&mut self, size: Size, now: Instant) -> Option<K> {
        if self.stack.len() <= 1 {
            return None;
        }
        let old = self.stack.pop()?;
        self.leaving = Some((old.clone(), Slide::new(-1.0, self.motion, now)));
        self.set_size(size, now);
        Some(old)
    }

    /// Geçerli sayfanın içerik boyutu değişti.
    pub fn set_size(&mut self, size: Size, now: Instant) {
        self.size.set_target(Point::new(size.width, size.height), now);
    }

    /// Bu karedeki kap boyutu (taşmalı yay).
    pub fn current_size(&self, now: Instant) -> Size {
        let p = self.size.value(now);
        Size::new(p.x.max(0.0), p.y.max(0.0))
    }

    pub fn target_size(&self) -> Size {
        let p = self.size.target();
        Size::new(p.x, p.y)
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        let moving = self.leaving.as_mut().is_some_and(|(_, s)| !s.is_settled(now));
        if !moving {
            self.leaving = None;
        }
        moving | !self.size.is_settled(now)
    }

    /// Sayfaları çizdirir: `f(ui, sayfa, köken)` geçişte iki kez (eski ve yeni) çağrılır;
    /// opaklık `ui.with_alpha` ile uygulanır. `clip` kap dikdörtgeni (kayan içerik kırpılır).
    pub fn draw(&mut self, ui: &mut Ui, clip: Rect, mut f: impl FnMut(&mut Ui, &K, Point)) {
        let now = ui.now;
        let slide = ui.theme.space(12);
        ui.painter.push_clip(clip);
        let current = self.current().clone();
        match &self.leaving {
            Some((old, s)) => {
                let (of, nf) = s.frames(now, slide);
                let old = old.clone();
                ui.with_alpha(of.alpha, |ui| f(ui, &old, Point::new(clip.x + of.dx, clip.y)));
                ui.with_alpha(nf.alpha, |ui| f(ui, &current, Point::new(clip.x + nf.dx, clip.y)));
            }
            None => f(ui, &current, clip.origin()),
        }
        ui.painter.pop_clip();
        if self.is_animating(now) {
            ui.request_frame();
            ui.damage(clip);
        }
    }
}

/// "‹ Başlık" geri satırı (alt sayfaların üstünde). Tıklama, Sol ok ya da Backspace → geri.
#[derive(Debug, Clone)]
pub struct PageHeader {
    core: Pressable,
    icon: IconView,
    title: Label,
}

impl PageHeader {
    pub fn new(theme: &Theme, title: &str) -> Self {
        Self {
            core: Pressable::new(theme),
            icon: IconView::with_role(theme, Icon::named(IconName::ChevronLeft), TextRole::Icon),
            title: Label::new(title, text_style(theme, TextRole::Title).with_max_lines(1)),
        }
    }

    pub fn set_title(&mut self, title: &str) {
        if self.title.set_text(title) {
            self.core.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.icon.set_theme(theme, now);
        self.title.set_style(text_style(theme, TextRole::Title).with_max_lines(1));
    }

    /// Yükseklik `space(12)`.
    pub fn preferred_size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        let w = theme.space(10) + self.title.measure(text).width + theme.space(4);
        Size::new(w.ceil(), theme.space(12))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    /// Geri istendi: `event = Some(())`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        self.core.pointer(event, now)
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response {
        if event.state != KeyState::Released && matches!(event.key, Key::Left | Key::Backspace) {
            return Response::event(());
        }
        self.core.key(event, now)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let c = &theme.colors;
        let r = self.core.rect;
        let radii = crate::style::pill(r);
        self.core.interaction.draw_state_layer_under(ui, r, radii, c.on_surface, nuxut_core::Color::TRANSPARENT, &[c.on_surface, c.primary]);
        self.core.draw_focus(ui, r, radii);
        let iw = self.icon.size().width;
        self.icon.draw_centered(ui.painter, Point::new(r.x + theme.space(2) + iw * 0.5, r.center().y), ui.fade(c.primary));
        let m = self.title.measure_in(ui.painter);
        self.title.draw(ui.painter, Point::new(r.x + theme.space(3) + iw, r.center().y - m.height * 0.5), ui.fade(c.on_surface));
        self.core.finish(ui, self.core.extent());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn push_pop_morphs_size() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = PageStack::new(&theme, "kök", Size::new(300.0, 200.0));
        s.push("wifi", Size::new(300.0, 420.0), t0);
        assert_eq!(*s.current(), "wifi");
        assert_eq!(s.current_size(t0).height, 200.0, "zıplamaz");
        let t1 = t0 + Duration::from_secs(3);
        assert!(!s.is_animating(t1));
        assert_eq!(s.current_size(t1).height, 420.0);
        assert_eq!(s.pop(Size::new(300.0, 200.0), t1), Some("wifi"));
        assert_eq!(s.pop(Size::new(300.0, 200.0), t1), None, "kökte dur");
        assert_eq!(s.depth(), 0);
    }

    #[test]
    fn header_back() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut h = PageHeader::new(&theme, "Wi-Fi");
        h.set_rect(Rect::new(0.0, 0.0, 200.0, 48.0));
        assert_eq!(h.key(&key(Key::Backspace), t0).event, Some(()));
        let _ = h.pointer(&press(10.0, 10.0), t0);
        assert_eq!(h.pointer(&release(10.0, 10.0), t0).event, Some(()));
    }
}
