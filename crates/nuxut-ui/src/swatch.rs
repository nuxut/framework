//! Küçük palet renk seçici ([`SwatchPicker`]): satırlara sarılan renk örnekleri, seçili örnek
//! boşluklu halka + ✓ + hafif büyüme (DESIGN "gap ring"); isteğe bağlı sonda "özel renk"
//! düğmesi (çağıran tam [`ColorPicker`](crate::ColorPicker)'ı açar).
//!
//! Renkler veridir (takvim/etiket renkleri) ya da [`swatch_palette`] ile temadan üretilir;
//! koda gömülü palet yoktur. Ok tuşları örnekler arasında gezer, Enter/Space seçer.

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle};
use nuxut_theme::{STATE_HOVER, STATE_PRESSED, Theme};

use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::style::{DISABLED_CONTENT, FOCUS_RING, HAIRLINE, Hsv, content_on, field_fill, luminance};
use crate::typography::TextRole;
use crate::ui::{DamageState, Response, Ui};

/// Örnek ile zemin arasındaki parlaklık farkı bunun altındaysa örneğe hafif sınır çizilir.
const LOW_CONTRAST: f32 = 0.06;
/// En fazla örnek (sınırsız veri ızgarayı taşırmasın).
pub const MAX_SWATCHES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SwatchEvent {
    Selected(Color),
    /// "Özel renk" düğmesi.
    Custom,
}

/// Temadan `n` renklik uyumlu palet: tonlar `primary`'nin tonundan başlayarak eşit aralıklı,
/// doygunluk/değer temanın vurgusundan (koyu temada daha açık) — her temada aynı "ağırlıkta".
pub fn swatch_palette(theme: &Theme, n: usize) -> Vec<Color> {
    let base = Hsv::from_color(theme.colors.primary);
    let dark = luminance(theme.colors.surface) < 0.5;
    let s = base.s.clamp(0.45, 0.7);
    let v = if dark { base.v.clamp(0.78, 0.92) } else { base.v.clamp(0.55, 0.72) };
    let n = n.clamp(1, MAX_SWATCHES);
    (0..n).map(|i| Hsv::new(base.h + 360.0 * i as f32 / n as f32, s, v).to_color()).collect()
}

#[derive(Debug, Clone)]
pub struct SwatchPicker {
    colors: Vec<Color>,
    selected: Option<usize>,
    custom: Option<IconView>,
    check: IconView,
    rect: Rect,
    /// Örnek başına büyüme (0..1); seçili/üzerinde olan büyür.
    grow: Vec<Animated<f32>>,
    hovered: Option<usize>,
    pressed: Option<usize>,
    /// Klavye odağındaki örnek (görünür halka).
    focus: Option<usize>,
    focus_visible: bool,
    damage: DamageState,
}

impl SwatchPicker {
    /// `selected` palette yoksa (ör. sağlayıcının özel rengi) hiçbir örnek seçili görünmez.
    pub fn new(theme: &Theme, colors: Vec<Color>, selected: Option<Color>) -> Self {
        let mut p = Self {
            colors: Vec::new(),
            selected: None,
            custom: None,
            check: IconView::with_role(theme, Icon::named(IconName::Check), TextRole::IconSmall),
            rect: Rect::ZERO,
            grow: Vec::new(),
            hovered: None,
            pressed: None,
            focus: None,
            focus_visible: false,
            damage: DamageState::new(),
        };
        p.set_colors(theme, colors, selected);
        p
    }

    /// Sona "özel renk" düğmesi ekler ([`SwatchEvent::Custom`]).
    pub fn with_custom(mut self, theme: &Theme) -> Self {
        self.custom = Some(IconView::with_role(theme, Icon::named(IconName::Palette), TextRole::IconSmall));
        self.grow.push(Animated::new(0.0, theme.motion.spatial_fast));
        self
    }

    pub fn set_colors(&mut self, theme: &Theme, mut colors: Vec<Color>, selected: Option<Color>) {
        colors.truncate(MAX_SWATCHES);
        let n = colors.len() + usize::from(self.custom.is_some());
        self.colors = colors;
        self.grow = (0..n).map(|_| Animated::new(0.0, theme.motion.spatial_fast)).collect();
        self.hovered = None;
        self.pressed = None;
        self.focus = self.focus.filter(|&i| i < n);
        self.selected = selected.and_then(|c| self.colors.iter().position(|x| *x == c));
        if let Some(i) = self.selected {
            self.grow[i].jump_to(1.0);
        }
        self.damage.mark();
    }

    pub fn colors(&self) -> &[Color] {
        &self.colors
    }

    pub fn selected(&self) -> Option<Color> {
        self.selected.map(|i| self.colors[i])
    }

    /// Dışarıdan seçim (ör. özel renk seçildi → palette yoksa seçim kalkar).
    pub fn set_selected(&mut self, color: Option<Color>, now: Instant) {
        let idx = color.and_then(|c| self.colors.iter().position(|x| *x == c));
        if idx != self.selected {
            self.selected = idx;
            self.retarget(now);
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for g in &mut self.grow {
            g.set_motion(theme.motion.spatial_fast, now);
        }
        self.check.set_theme(theme, now);
        if let Some(c) = &mut self.custom {
            c.set_theme(theme, now);
        }
        self.damage.mark();
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        if self.focus_visible == on {
            return false;
        }
        self.focus_visible = on;
        if on && self.focus.is_none() {
            self.focus = Some(self.selected.unwrap_or(0));
        }
        self.damage.mark();
        true
    }

    fn count(&self) -> usize {
        self.grow.len()
    }

    fn metrics(theme: &Theme) -> (f32, f32) {
        (theme.space(8), theme.space(2))
    }

    fn per_row(theme: &Theme, width: f32) -> usize {
        let (d, gap) = Self::metrics(theme);
        ((width + gap) / (d + gap)).floor().max(1.0) as usize
    }

    /// Genişlik verilir; satır sayısı ondan.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        let (d, gap) = Self::metrics(theme);
        let rows = self.count().div_ceil(Self::per_row(theme, width)).max(1) as f32;
        Size::new(width, rows * d + (rows - 1.0) * gap)
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    fn swatch_rect(&self, theme: &Theme, i: usize) -> Rect {
        let (d, gap) = Self::metrics(theme);
        let per = Self::per_row(theme, self.rect.width);
        let (row, col) = (i / per, i % per);
        Rect::new(self.rect.x + col as f32 * (d + gap), self.rect.y + row as f32 * (d + gap), d, d)
    }

    fn hit(&self, theme: &Theme, p: Point) -> Option<usize> {
        let (_, gap) = Self::metrics(theme);
        (0..self.count()).find(|&i| self.swatch_rect(theme, i).expand(gap * 0.5).contains(p))
    }

    fn is_custom(&self, i: usize) -> bool {
        self.custom.is_some() && i == self.colors.len()
    }

    fn retarget(&mut self, now: Instant) {
        for (i, g) in self.grow.iter_mut().enumerate() {
            let on = self.selected == Some(i) || self.hovered == Some(i);
            g.set_target(if on { 1.0 } else { 0.0 }, now);
        }
        self.damage.mark();
    }

    fn activate(&mut self, i: usize, now: Instant) -> Response<SwatchEvent> {
        if self.is_custom(i) {
            return Response::event(SwatchEvent::Custom);
        }
        self.selected = Some(i);
        self.retarget(now);
        Response::event(SwatchEvent::Selected(self.colors[i]))
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<SwatchEvent> {
        let hit = if event.kind == PointerKind::Leave { None } else { self.hit(theme, event.position) };
        let mut out = Response::none();
        if hit != self.hovered {
            self.hovered = hit;
            self.retarget(now);
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Press(Button::Left) if hit.is_some() => {
                self.pressed = hit;
                self.focus_visible = false;
                self.damage.mark();
                out.repaint = true;
            }
            PointerKind::Release(Button::Left) => {
                let pressed = self.pressed.take();
                if pressed.is_some() {
                    self.damage.mark();
                    out.repaint = true;
                }
                if let Some(i) = pressed.filter(|&i| Some(i) == hit) {
                    self.focus = Some(i);
                    return self.activate(i, now).with_repaint(true);
                }
            }
            _ => {}
        }
        out
    }

    /// Oklar gezinir (satırlar arası yukarı/aşağı), Home/End uçlar, Enter/Space seçer.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<SwatchEvent> {
        if event.state == KeyState::Released || self.count() == 0 {
            return Response::none();
        }
        let n = self.count();
        let per = Self::per_row(theme, self.rect.width) as isize;
        let cur = self.focus.or(self.selected).unwrap_or(0);
        let step = |d: isize| (cur as isize + d).clamp(0, n as isize - 1) as usize;
        let next = match event.key {
            Key::Left => step(-1),
            Key::Right => step(1),
            Key::Up => step(-per),
            Key::Down => step(per),
            Key::Home => 0,
            Key::End => n - 1,
            Key::Enter | Key::Space => return self.activate(cur, now),
            _ => return Response::none(),
        };
        self.focus = Some(next);
        self.focus_visible = true;
        self.damage.mark();
        Response::repaint()
    }

    pub fn cursor(&self, theme: &Theme, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        Some(if self.hit(theme, p).is_some() { CursorShape::Pointer } else { CursorShape::Default })
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let (_, gap) = Self::metrics(theme);
        let mut animating = false;
        for i in 0..self.count() {
            let base = self.swatch_rect(theme, i);
            let g = self.grow[i].value(now);
            animating |= !self.grow[i].is_settled(now);
            let rr = base.expand(theme.space(1) * 0.5 * g);
            let disc = |r: Rect| Corners::uniform(r.height * 0.5);
            let custom = self.is_custom(i);
            let fill = if custom { field_fill(theme) } else { self.colors[i] };
            if self.selected == Some(i) {
                // Boşluklu halka: aradaki zemin görünür kalır (dolgu yok, yalnız kenar).
                let ring = rr.expand(gap * 0.5 + FOCUS_RING);
                let style = RectStyle { fill: Color::TRANSPARENT, radii: disc(ring), border: Some(Border { width: FOCUS_RING, color: ui.fade(c.on_surface) }) };
                ui.painter.rect(ring, &style);
            }
            let low_contrast = (luminance(fill) - luminance(c.surface_container)).abs() < LOW_CONTRAST;
            let border = theme
                .outline()
                .or_else(|| low_contrast.then(|| c.on_surface_variant.multiply_alpha(DISABLED_CONTENT)))
                .map(|color| Border { width: HAIRLINE, color: ui.fade(color) });
            ui.painter.rect(rr, &RectStyle { fill: ui.fade(fill), radii: disc(rr), border });
            let content = content_on(theme, fill);
            let layer = if self.pressed == Some(i) {
                STATE_PRESSED
            } else if self.hovered == Some(i) {
                STATE_HOVER
            } else {
                0.0
            };
            if layer > 0.0 {
                let layer = crate::style::state_layer(theme, content, &[fill], layer);
                ui.painter.rect(rr, &RectStyle::filled(ui.fade(layer), disc(rr)));
            }
            if custom {
                if let Some(icon) = &mut self.custom {
                    icon.draw_centered(ui.painter, rr.center(), ui.fade(c.on_surface_variant));
                }
            } else if self.selected == Some(i) {
                self.check.draw_centered(ui.painter, rr.center(), ui.fade(content));
            }
            if self.focus_visible && self.focus == Some(i) && self.selected != Some(i) {
                let ring = rr.expand(FOCUS_RING);
                let style = RectStyle { fill: Color::TRANSPARENT, radii: disc(ring), border: Some(Border { width: FOCUS_RING, color: ui.fade(crate::style::focus_ring(theme, &[fill])) }) };
                ui.painter.rect(ring, &style);
            } else if self.focus_visible && self.focus == Some(i) {
                // Seçili örnekte halka zaten var: odak onu `primary` yapar.
                let ring = rr.expand(gap * 0.5 + FOCUS_RING);
                let style = RectStyle { fill: Color::TRANSPARENT, radii: disc(ring), border: Some(Border { width: FOCUS_RING, color: ui.fade(c.primary) }) };
                ui.painter.rect(ring, &style);
            }
        }
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.rect.expand(gap + FOCUS_RING * 2.0), animating);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn picker(custom: bool) -> (SwatchPicker, Theme) {
        let theme = Theme::default();
        let colors = swatch_palette(&theme, 12);
        let sel = colors[2];
        let mut p = SwatchPicker::new(&theme, colors, Some(sel));
        if custom {
            p = p.with_custom(&theme);
        }
        let s = p.preferred_size(&theme, 200.0);
        p.set_rect(Rect::new(0.0, 0.0, s.width, s.height));
        (p, theme)
    }

    #[test]
    fn palette_is_distinct_and_theme_derived() {
        let theme = Theme::default();
        let p = swatch_palette(&theme, 12);
        assert_eq!(p.len(), 12);
        for (i, a) in p.iter().enumerate() {
            for b in &p[i + 1..] {
                assert_ne!(a.to_hex(), b.to_hex());
            }
        }
        // İlk ton temanın vurgusunun tonu.
        let h0 = Hsv::from_color(p[0]).h;
        let hp = Hsv::from_color(theme.colors.primary).h;
        assert!((h0 - hp).abs() < 2.0 || (h0 - hp).abs() > 358.0);
    }

    #[test]
    fn wraps_rows() {
        let (p, theme) = picker(true);
        let per = SwatchPicker::per_row(&theme, 200.0);
        assert_eq!(p.preferred_size(&theme, 200.0).height, {
            let rows = 13usize.div_ceil(per) as f32;
            rows * theme.space(8) + (rows - 1.0) * theme.space(2)
        });
    }

    #[test]
    fn click_selects_and_custom_emits() {
        let (mut p, theme) = picker(true);
        let t0 = Instant::now();
        assert_eq!(p.selected(), Some(p.colors()[2]));
        let c = p.swatch_rect(&theme, 5).center();
        let _ = p.pointer(&theme, &press(c.x, c.y), t0);
        let r = p.pointer(&theme, &release(c.x, c.y), t0);
        assert_eq!(r.event, Some(SwatchEvent::Selected(p.colors()[5])));
        assert_eq!(p.selected(), Some(p.colors()[5]));
        let c = p.swatch_rect(&theme, 12).center();
        let _ = p.pointer(&theme, &press(c.x, c.y), t0);
        assert_eq!(p.pointer(&theme, &release(c.x, c.y), t0).event, Some(SwatchEvent::Custom));
        assert_eq!(p.selected(), Some(p.colors()[5]), "özel düğme seçimi değiştirmez");
        assert_eq!(p.cursor(&theme, c), Some(CursorShape::Pointer));
    }

    #[test]
    fn keyboard_moves_focus_then_selects() {
        let (mut p, theme) = picker(false);
        let t0 = Instant::now();
        let _ = p.key(&theme, &key(Key::Right), t0);
        assert_eq!(p.focus, Some(3));
        let _ = p.key(&theme, &key(Key::End), t0);
        assert_eq!(p.key(&theme, &key(Key::Enter), t0).event, Some(SwatchEvent::Selected(p.colors()[11])));
    }

    #[test]
    fn unknown_selection_shows_none() {
        let theme = Theme::default();
        let mut p = SwatchPicker::new(&theme, swatch_palette(&theme, 4), Some(theme.colors.on_surface));
        assert_eq!(p.selected(), None);
        let first = p.colors()[0];
        p.set_selected(Some(first), Instant::now());
        assert_eq!(p.selected(), Some(first));
    }
}
