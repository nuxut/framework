//! Chip'ler ve grup: filtre (çoklu seçim, seçiliyken ✓), seçim (tekli), giriş (× ile kaldırma)
//! ve yardım (eylem) chip'leri. [`ChipGroup`] chip'leri satıra sığmayınca alta sarar.
//!
//! Tonal dil: seçili chip `tonal_container` + `primary` içerik; seçisiz `surface_bright`; kenar
//! yalnızca kenar kipi açıksa (`theme.outline()`).

use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, KeyEvent, PointerEvent};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::pressable::Pressable;
use crate::style::{HAIRLINE, accent_on, clip_text, disabled_container, disabled_content, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// Chip etiketinin en uzun hâli (bayt).
pub const MAX_CHIP_LABEL: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChipKind {
    /// Aç/kapa; seçiliyken ✓ ikonu.
    #[default]
    Filter,
    /// Grupta tekli seçim.
    Choice,
    /// Kaldırılabilir (×).
    Input,
    /// Eylem.
    Assist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipEvent {
    /// Filtre/seçim chip'inin yeni durumu.
    Toggled(bool),
    /// Yardım chip'i ya da giriş chip'inin gövdesi.
    Clicked,
    /// Giriş chip'inin × düğmesi.
    Removed,
}

#[derive(Debug, Clone)]
pub struct Chip {
    core: Pressable,
    kind: ChipKind,
    label: Label,
    icon: Option<IconView>,
    check: IconView,
    remove_icon: IconView,
    remove: Interaction,
    selected: bool,
    /// ✓ ikonunun yer açması (0..1, `spatial_fast`).
    check_t: Animated<f32>,
}

impl Chip {
    pub fn new(theme: &Theme, kind: ChipKind, label: &str) -> Self {
        Self {
            core: Pressable::new(theme),
            kind,
            label: Label::new(clip_text(label, MAX_CHIP_LABEL), text_style(theme, TextRole::Label).with_max_lines(1)),
            icon: None,
            check: IconView::with_role(theme, Icon::named(IconName::Check), TextRole::IconSmall),
            remove_icon: IconView::with_role(theme, Icon::named(IconName::Close), TextRole::IconSmall),
            remove: Interaction::new(theme),
            selected: false,
            check_t: Animated::new(0.0, theme.motion.spatial_fast),
        }
    }

    pub fn with_icon(mut self, theme: &Theme, icon: Icon) -> Self {
        self.icon = Some(IconView::with_role(theme, icon, TextRole::IconSmall));
        self
    }

    pub fn selected(mut self, on: bool) -> Self {
        self.selected = on;
        self.check_t.jump_to(if on { 1.0 } else { 0.0 });
        self
    }

    pub fn kind(&self) -> ChipKind {
        self.kind
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }

    pub fn label(&self) -> &str {
        self.label.text()
    }

    pub fn set_selected(&mut self, on: bool, now: Instant) {
        if self.selected != on {
            self.selected = on;
            self.check_t.set_target(if on { 1.0 } else { 0.0 }, now);
            self.core.damage.mark();
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        self.core.set_enabled(enabled, now);
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.label.set_style(text_style(theme, TextRole::Label).with_max_lines(1));
        for i in [&mut self.check, &mut self.remove_icon].into_iter().chain(self.icon.as_mut()) {
            i.set_theme(theme, now);
        }
        self.remove.set_theme(theme, now);
        self.check_t.set_motion(theme.motion.spatial_fast, now);
    }

    fn icon_w(theme: &Theme) -> f32 {
        text_style(theme, TextRole::IconSmall).size
    }

    /// Yükseklik `space(8)`.
    pub fn preferred_size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        let pad = theme.space(3);
        let gap = theme.space(2);
        let iw = Self::icon_w(theme);
        let mut w = pad * 2.0 + self.label.measure(text).width;
        // Filtre chip'inde ✓ seçilince yer açar; ikonlu chip'te ✓ ikonun yerine geçer.
        if self.icon.is_some() || (self.kind == ChipKind::Filter && self.selected) {
            w += iw + gap;
        }
        if self.kind == ChipKind::Input {
            w += iw + gap;
        }
        Size::new(w.ceil(), theme.space(8))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    fn remove_rect(&self, theme: &Theme) -> Rect {
        let r = self.core.rect;
        let s = r.height;
        Rect::new(r.right() - s - theme.space(1) * 0.5, r.y, s, s)
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<ChipEvent> {
        if self.kind == ChipKind::Input && self.core.enabled {
            let rr = self.remove_rect(theme);
            let tap = self.remove.handle(rr, event, now);
            if tap.repaint {
                self.core.damage.mark();
            }
            if tap.clicked {
                return Response::event(ChipEvent::Removed);
            }
            if rr.contains(event.position) {
                // × üstünde gövdenin hover'ı sönsün.
                let leave = event.with_kind(event.position, crate::input::PointerKind::Leave);
                return self.core.pointer(&leave, now).discard().with_repaint(tap.repaint);
            }
        }
        let r = self.core.pointer(event, now);
        self.after(r, now)
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<ChipEvent> {
        let r = self.core.key(event, now);
        self.after(r, now)
    }

    fn after(&mut self, r: Response, now: Instant) -> Response<ChipEvent> {
        if r.event.is_none() {
            return r.discard();
        }
        match self.kind {
            ChipKind::Filter => {
                let on = !self.selected;
                self.set_selected(on, now);
                Response::event(ChipEvent::Toggled(on))
            }
            // Seçim chip'i tıklamayla seçilir; grup diğerlerini kaldırır.
            ChipKind::Choice => {
                self.set_selected(true, now);
                Response::event(ChipEvent::Toggled(true))
            }
            ChipKind::Input | ChipKind::Assist => Response::event(ChipEvent::Clicked),
        }
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.check_t.is_settled(now)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.core.rect;
        let radii = nuxut_core::Corners::uniform(theme.shape.radius_small);
        let enabled = self.core.enabled;
        let ChipColors { fill, content, label: label_col } = ChipColors::new(theme, enabled, self.selected);
        ui.painter.rect(rect, &RectStyle::filled(ui.fade(fill), radii));
        if let Some(outline) = theme.outline().filter(|_| !self.selected) {
            let style = RectStyle { fill: Color::TRANSPARENT, radii, border: Some(Border { width: HAIRLINE, color: ui.fade(outline) }) };
            ui.painter.rect(rect, &style);
        }
        self.core.interaction.draw_state_layer_under(ui, rect, radii, c.on_surface, fill, &[content, label_col]);
        self.core.draw_focus(ui, rect, radii);

        let pad = theme.space(3);
        let gap = theme.space(2);
        let iw = Self::icon_w(theme);
        let t = self.check_t.value(now).clamp(0.0, 1.0);
        let mut x = rect.x + pad;
        let mid = rect.center().y;
        let col = ui.fade(content);
        if self.kind == ChipKind::Filter && self.icon.is_none() {
            // ✓ yer açarak belirir.
            let w = (iw + gap) * t;
            if t > 0.01 {
                self.check.draw_centered(ui.painter, Point::new(x + iw * 0.5 * t, mid), col.multiply_alpha(t));
            }
            x += w;
        } else if let Some(icon) = &mut self.icon {
            // İkonlu filtre chip'inde seçilince ikon yerine ✓.
            if self.kind == ChipKind::Filter && t > 0.5 {
                self.check.draw_centered(ui.painter, Point::new(x + iw * 0.5, mid), col);
            } else {
                icon.draw_centered(ui.painter, Point::new(x + iw * 0.5, mid), col);
            }
            x += iw + gap;
        }
        let m = self.label.measure_in(ui.painter);
        self.label.draw(ui.painter, Point::new(x, mid - m.height * 0.5), ui.fade(label_col));
        if self.kind == ChipKind::Input {
            let rr = self.remove_rect(theme);
            let disc = Rect::new(rr.center().x - iw * 0.75, mid - iw * 0.75, iw * 1.5, iw * 1.5);
            self.remove.draw_state_layer_under(ui, disc, crate::style::pill(disc), c.on_surface, fill, &[content]);
            self.remove_icon.draw_centered(ui.painter, Point::new(rr.center().x, mid), col);
        }
        let animating = self.is_animating(now) | self.remove.is_animating(now);
        if animating {
            ui.request_frame();
        }
        self.core.finish(ui, self.core.extent());
    }
}

/// Chip grubu: satıra sığmayınca alta sarar. `single`: seçim chip'leri gibi tekli seçim.
/// Chip renkleri (saf; yüksek karşıtlık testleri bunun üstünden).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChipColors {
    pub fill: Color,
    /// İkon / ✓ / kaldır ikonu.
    pub content: Color,
    pub label: Color,
}

impl ChipColors {
    pub(crate) fn new(theme: &Theme, enabled: bool, selected: bool) -> Self {
        let c = &theme.colors;
        match (enabled, selected) {
            (false, _) => {
                let fill = disabled_container(theme);
                let d = disabled_content(theme, c.on_surface, &[fill]);
                Self { fill, content: d, label: d }
            }
            (true, true) => {
                let fill = tonal_container(theme);
                Self { fill, content: accent_on(theme, fill), label: c.on_surface }
            }
            (true, false) => Self { fill: c.surface_bright, content: c.on_surface_variant, label: c.on_surface_variant },
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChipGroup {
    chips: Vec<Chip>,
    single: bool,
    rect: Rect,
}

impl ChipGroup {
    pub fn new(chips: Vec<Chip>, single: bool) -> Self {
        Self { chips, single, rect: Rect::ZERO }
    }

    pub fn chips(&self) -> &[Chip] {
        &self.chips
    }

    pub fn chips_mut(&mut self) -> &mut Vec<Chip> {
        &mut self.chips
    }

    /// Seçili chip'lerin indisleri.
    pub fn selected(&self) -> Vec<usize> {
        self.chips.iter().enumerate().filter(|(_, c)| c.is_selected()).map(|(i, _)| i).collect()
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for c in &mut self.chips {
            c.set_theme(theme, now);
        }
    }

    /// Verilen genişlikte yerleştirir; kaplanan yüksekliği döner.
    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, origin: Point, width: f32) -> f32 {
        let gap = theme.space(2);
        let (mut x, mut y, mut row_h) = (origin.x, origin.y, 0.0f32);
        for c in &mut self.chips {
            let s = c.preferred_size(text, theme);
            let s = Size::new(s.width.min(width), s.height);
            if x > origin.x && x + s.width > origin.x + width {
                x = origin.x;
                y += row_h + gap;
                row_h = 0.0;
            }
            c.set_rect(Rect::new(x, y, s.width, s.height));
            x += s.width + gap;
            row_h = row_h.max(s.height);
        }
        let h = y + row_h - origin.y;
        self.rect = Rect::new(origin.x, origin.y, width, h);
        h
    }

    /// Olay `(chip indisi, olay)`. Kaldırılan chip gruptan çıkarılır; tekli grupta seçim tektir.
    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<(usize, ChipEvent)> {
        let mut out = Response::none();
        for i in 0..self.chips.len() {
            let r = self.chips[i].pointer(theme, event, now);
            out.repaint |= r.repaint;
            if let Some(e) = r.event {
                match e {
                    ChipEvent::Toggled(true) if self.single => {
                        for (j, c) in self.chips.iter_mut().enumerate() {
                            if j != i {
                                c.set_selected(false, now);
                            }
                        }
                    }
                    ChipEvent::Removed => {
                        self.chips.remove(i);
                        let (o, w) = (self.rect.origin(), self.rect.width);
                        self.layout(text, theme, o, w);
                    }
                    _ => {}
                }
                return Response::event((i, e));
            }
        }
        out
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.chips.iter().find_map(|c| c.cursor(p))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        for c in &mut self.chips {
            c.draw(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn click(theme: &Theme, c: &mut Chip, p: Point, t: Instant) -> Option<ChipEvent> {
        let _ = c.pointer(theme, &motion(p.x, p.y), t);
        let _ = c.pointer(theme, &press(p.x, p.y), t);
        c.pointer(theme, &release(p.x, p.y), t).event
    }

    #[test]
    fn filter_toggles_and_input_removes() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut f = Chip::new(&theme, ChipKind::Filter, "Okunmamış");
        f.set_rect(Rect::new(0.0, 0.0, 120.0, 32.0));
        assert_eq!(click(&theme, &mut f, Point::new(30.0, 16.0), t0), Some(ChipEvent::Toggled(true)));
        assert_eq!(click(&theme, &mut f, Point::new(30.0, 16.0), t0), Some(ChipEvent::Toggled(false)));
        let mut i = Chip::new(&theme, ChipKind::Input, "ayse@ornek.org");
        i.set_rect(Rect::new(0.0, 0.0, 160.0, 32.0));
        let x = i.remove_rect(&theme).center();
        assert_eq!(click(&theme, &mut i, x, t0), Some(ChipEvent::Removed));
        assert_eq!(click(&theme, &mut i, Point::new(20.0, 16.0), t0), Some(ChipEvent::Clicked));
    }

    #[test]
    fn group_single_select_and_wrap() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let chips = ["Gün", "Hafta", "Ay", "Yıl", "Ajanda"].iter().map(|l| Chip::new(&theme, ChipKind::Choice, l)).collect();
        let mut g = ChipGroup::new(chips, true);
        let h = g.layout(&mut text, &theme, Point::ZERO, 150.0);
        assert!(h > theme.space(8), "sarmalı: {h}");
        for idx in [1usize, 3] {
            let p = g.chips()[idx].rect().center();
            let _ = g.pointer(&mut text, &theme, &press(p.x, p.y), t0);
            let r = g.pointer(&mut text, &theme, &release(p.x, p.y), t0);
            assert_eq!(r.event, Some((idx, ChipEvent::Toggled(true))));
        }
        assert_eq!(g.selected(), vec![3]);
    }
}
