//! Gezinme rayı / kenar menüsü ([`NavigationRail`]): ikon + etiket öğeleri dikey dizilir;
//! seçili gösterge `spatial` hareketle öğeler arasında kayan bir hap (SegmentedButton ile aynı
//! dil). Daraltılmış kipte yalnız ikonlar kalır; genişlik ve etiket opaklığı yayla geçer.

use std::time::Instant;

use nuxut_core::{Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::icon::{Icon, IconView};
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::style::{FOCUS_RING, accent_on, focus_ring, pill, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

#[derive(Debug, Clone, PartialEq)]
pub struct NavItem {
    pub label: String,
    pub icon: Icon,
}

impl NavItem {
    pub fn new(icon: Icon, label: impl Into<String>) -> Self {
        Self { label: label.into(), icon }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    icon: IconView,
    label: Label,
    hover: Interaction,
}

#[derive(Debug, Clone)]
pub struct NavigationRail {
    entries: Vec<Entry>,
    selected: usize,
    rect: Rect,
    collapsed: bool,
    /// 1 = açık (etiketler görünür), 0 = daraltılmış.
    expand: Animated<f32>,
    indicator: Animated<Rect>,
    focus_visible: bool,
    damage: DamageState,
    /// Öğe dikdörtgenleri (imleç için; `set_rect`'te).
    hits: Vec<Rect>,
}

impl NavigationRail {
    pub fn new(theme: &Theme, items: Vec<NavItem>, selected: usize) -> Self {
        let style = text_style(theme, TextRole::Label).with_max_lines(1);
        let entries: Vec<Entry> = items
            .into_iter()
            .map(|i| Entry {
                icon: IconView::new(theme, i.icon),
                label: Label::new(i.label, style.clone()),
                hover: Interaction::new(theme),
            })
            .collect();
        let selected = selected.min(entries.len().saturating_sub(1));
        let mut rail = Self {
            entries,
            selected,
            rect: Rect::ZERO,
            collapsed: false,
            expand: Animated::new(1.0, theme.motion.spatial),
            indicator: Animated::new(Rect::ZERO, theme.motion.spatial),
            focus_visible: false,
            damage: DamageState::new(),
            hits: Vec::new(),
        };
        if let Some(e) = rail.entries.get_mut(selected) {
            e.icon = e.icon.clone().filled(true);
        }
        rail
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    /// Daraltır/açar; genişlik `spatial` hareketle geçer ([`current_width`](Self::current_width)).
    pub fn set_collapsed(&mut self, theme: &Theme, collapsed: bool, now: Instant) {
        if self.collapsed != collapsed {
            self.collapsed = collapsed;
            self.expand.set_target(if collapsed { 0.0 } else { 1.0 }, now);
            self.indicator.set_target(self.indicator_rect(theme, self.selected, collapsed), now);
            self.damage.mark();
        }
    }

    pub fn set_selected(&mut self, theme: &Theme, index: usize, now: Instant) -> bool {
        let index = index.min(self.entries.len().saturating_sub(1));
        if index == self.selected {
            return false;
        }
        self.entries[self.selected].icon.set_filled(false, now);
        self.entries[index].icon.set_filled(true, now);
        self.selected = index;
        self.indicator.set_target(self.indicator_rect(theme, index, self.collapsed), now);
        self.damage.mark();
        true
    }

    pub fn set_focus_visible(&mut self, on: bool) {
        if self.focus_visible != on {
            self.focus_visible = on;
            self.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        let style = text_style(theme, TextRole::Label).with_max_lines(1);
        for e in &mut self.entries {
            e.label.set_style(style.clone());
            e.icon.set_theme(theme, now);
            e.hover.set_theme(theme, now);
        }
        self.expand.set_motion(theme.motion.spatial, now);
        self.indicator.set_motion(theme.motion.spatial, now);
        self.damage.mark();
    }

    fn item_h(theme: &Theme) -> f32 {
        theme.space(12)
    }

    fn collapsed_w(theme: &Theme) -> f32 {
        theme.space(20)
    }

    /// Açık kipte gereken genişlik (en uzun etiket).
    pub fn expanded_width(&mut self, text: &mut TextSystem, theme: &Theme) -> f32 {
        let label = self.entries.iter_mut().map(|e| e.label.measure(text).width).fold(0.0, f32::max);
        (theme.space(5) * 2.0 + theme.space(6) + theme.space(4) + label + theme.space(4)).ceil()
    }

    /// Bu karedeki genişlik: daraltılmış ile açık arasında yayla.
    pub fn current_width(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) -> f32 {
        let open = self.expanded_width(text, theme);
        let c = Self::collapsed_w(theme);
        c + (open - c) * self.expand.value(now).max(0.0)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.rect != rect {
            let first = self.rect == Rect::ZERO;
            self.rect = rect;
            if first {
                self.indicator.jump_to(self.indicator_rect(theme, self.selected, self.collapsed));
            }
            self.hits = (0..self.entries.len()).map(|i| self.item_rect(theme, i)).collect();
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn item_rect(&self, theme: &Theme, i: usize) -> Rect {
        let pad = theme.space(3);
        let h = Self::item_h(theme);
        Rect::new(self.rect.x + pad, self.rect.y + pad + (h + theme.space(1)) * i as f32, (self.rect.width - pad * 2.0).max(0.0), h)
    }

    /// Gösterge: açıkta öğe boyunca hap, daraltılmışta ikonu saran hap.
    fn indicator_rect(&self, theme: &Theme, i: usize, collapsed: bool) -> Rect {
        let r = self.item_rect(theme, i);
        if collapsed {
            let w = theme.space(14);
            let x = self.rect.x + (Self::collapsed_w(theme) - w) * 0.5;
            Rect::new(x, r.y + theme.space(1), w, r.height - theme.space(2))
        } else {
            let open_w = (self.rect.width - theme.space(3) * 2.0).max(0.0);
            Rect::new(r.x, r.y, open_w, r.height)
        }
    }

    /// Tıklamada yeni seçim: `event = Some(indis)`.
    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<usize> {
        let mut out = Response::none();
        let mut clicked = None;
        for i in 0..self.entries.len() {
            let r = self.item_rect(theme, i);
            let tap = self.entries[i].hover.handle(r, event, now);
            out.repaint |= tap.repaint;
            if tap.clicked {
                clicked = Some(i);
            }
        }
        if out.repaint {
            self.damage.mark();
        }
        if let Some(i) = clicked {
            self.focus_visible = false;
            if self.set_selected(theme, i, now) {
                return Response::event(i);
            }
        }
        out
    }

    /// Yukarı/Aşağı, Home/End seçimi değiştirir (odaktayken).
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<usize> {
        if event.state == KeyState::Released || self.entries.is_empty() {
            return Response::none();
        }
        let n = self.entries.len();
        let next = match event.key {
            Key::Up => self.selected.saturating_sub(1),
            Key::Down => (self.selected + 1).min(n - 1),
            Key::Home => 0,
            Key::End => n - 1,
            _ => return Response::none(),
        };
        self.focus_visible = true;
        if self.set_selected(theme, next, now) { Response::event(next) } else { Response::repaint() }
    }

    /// İmleç: öğeler `Pointer`, ray boşluğu `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        Some(if self.hits.iter().any(|r| r.contains(p)) { CursorShape::Pointer } else { CursorShape::Default })
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        // Açık kipteki gösterge genişliği rayın anlık genişliğini izler.
        if !self.collapsed && self.indicator.is_settled(now) {
            self.indicator.jump_to(self.indicator_rect(theme, self.selected, false));
        }
        let ind = self.indicator.value(now);
        let e = self.expand.value(now).clamp(0.0, 1.0);
        ui.painter.push_clip(self.rect);
        for i in 0..self.entries.len() {
            let r = self.item_rect(theme, i);
            let hr = if self.collapsed { self.indicator_rect(theme, i, true) } else { r };
            self.entries[i].hover.draw_state_layer_under(ui, hr, pill(hr), c.on_surface, nuxut_core::Color::TRANSPARENT, &[c.on_surface_variant]);
        }
        let tonal = tonal_container(theme);
        ui.painter.rect(ind, &RectStyle::filled(ui.fade(tonal), pill(ind)));
        if self.focus_visible {
            let ring = ind.expand(FOCUS_RING);
            let style = RectStyle {
                fill: nuxut_core::Color::TRANSPARENT,
                radii: pill(ring),
                border: Some(nuxut_render::Border { width: FOCUS_RING, color: ui.fade(focus_ring(theme, &[tonal])) }),
            };
            ui.painter.rect(ring, &style);
        }
        let mut animating = !self.indicator.is_settled(now) | !self.expand.is_settled(now);
        let icon_cx = self.rect.x + Self::collapsed_w(theme) * 0.5;
        for i in 0..self.entries.len() {
            let r = self.item_rect(theme, i);
            let selected = i == self.selected;
            let entry = &mut self.entries[i];
            animating |= entry.icon.animate(now) | entry.hover.is_animating(now);
            let ic = if selected { accent_on(theme, tonal) } else { c.on_surface_variant };
            // İkon daraltılmış konumla açık konum arasında (sol boşluk) kayar.
            let open_x = r.x + theme.space(5) + entry.icon.size().width * 0.5;
            let x = icon_cx + (open_x - icon_cx) * e;
            entry.icon.draw_centered(ui.painter, Point::new(x, r.center().y), ui.fade(ic));
            if e > 0.01 {
                let m = entry.label.measure_in(ui.painter);
                let lx = open_x + entry.icon.size().width * 0.5 + theme.space(4);
                let col = if selected { c.on_surface } else { c.on_surface_variant };
                entry.label.draw(ui.painter, Point::new(lx, r.center().y - m.height * 0.5), ui.fade(col.multiply_alpha(e)));
            }
        }
        ui.painter.pop_clip();
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.rect.expand(FOCUS_RING * 2.0), animating);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::icon_names::IconName;
    use crate::input::test_util::*;

    fn rail() -> (NavigationRail, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let items = vec![
            NavItem::new(Icon::named(IconName::Palette), "Görünüm"),
            NavItem::new(Icon::named(IconName::Wifi), "Ağ"),
            NavItem::new(Icon::named(IconName::VolumeHigh), "Ses"),
        ];
        let mut r = NavigationRail::new(&theme, items, 0);
        r.set_rect(&theme, Rect::new(0.0, 0.0, 240.0, 600.0));
        (r, theme, t0)
    }

    #[test]
    fn click_selects_and_indicator_slides() {
        let (mut r, theme, t0) = rail();
        let p = r.item_rect(&theme, 2).center();
        let _ = r.pointer(&theme, &press(p.x, p.y), t0);
        assert_eq!(r.pointer(&theme, &release(p.x, p.y), t0).event, Some(2));
        assert_eq!(r.indicator.value(t0), r.indicator_rect(&theme, 0, false), "kayar, zıplamaz");
        assert_eq!(r.cursor(p), Some(CursorShape::Pointer));
        assert_eq!(r.cursor(Point::new(10.0, 590.0)), Some(CursorShape::Default));
    }

    #[test]
    fn keys_and_collapse() {
        let (mut r, theme, t0) = rail();
        assert_eq!(r.key(&theme, &key(Key::End), t0).event, Some(2));
        assert!(r.key(&theme, &key(Key::Down), t0).event.is_none());
        r.set_collapsed(&theme, true, t0);
        let mut text = TextSystem::new();
        let open = r.expanded_width(&mut text, &theme);
        let w0 = r.current_width(&mut text, &theme, t0);
        let w1 = r.current_width(&mut text, &theme, t0 + Duration::from_secs(3));
        assert!((w0 - open).abs() < 1e-3 && w1 < open);
        assert!(r.indicator.target().width < 100.0, "daraltılmış gösterge ikon boyunda");
    }
}
