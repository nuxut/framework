//! Sürükle-bırak ile sıralanan liste. Tutamaçtan tutulan öğe yükselir (gölge + hafif büyüme),
//! imleci gecikmesiz izler; diğerleri `spatial` hareketle yer açar. Bırakınca öğe yuvasına
//! yaylanır. Liste dışına bırakma `DraggedOut` (listeler arası taşıma; kaldırmak çağıranın).
//! Satır gövdesinden de sürüklenir; sürükleme eşiğinin altında kalan bas-bırak tıklamadır
//! (`Activated`). Klavye: Yukarı/Aşağı odak, Alt+Yukarı/Aşağı taşı, Enter/Boşluk etkinleştir,
//! Delete kaldır. Öğe başına kaldır düğmesi.

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, ShadowStyle};
use nuxut_theme::Theme;

use crate::button::{IconButton, IconButtonVariant};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::style::FOCUS_RING;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReorderItem {
    pub label: String,
    pub icon: Option<Icon>,
}

impl ReorderItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), icon: None }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReorderEvent {
    Moved { from: usize, to: usize },
    /// Öğe listeden kaldırıldı (kaldır düğmesi ya da Delete).
    Removed(usize),
    /// Öğe liste dışına bırakıldı; liste değişmedi (hedef liste ekler, bu liste kaldırır).
    DraggedOut { index: usize, position: Point },
    /// Satıra tıklandı (eşik altı hareketle bas-bırak) ya da odaktayken Enter/Boşluk.
    Activated(usize),
}

#[derive(Debug, Clone)]
struct Entry {
    item: ReorderItem,
    label: Label,
    icon: Option<IconView>,
    handle: IconView,
    remove: IconButton,
    /// Satırın üst kenarı (liste içi koordinat).
    y: Animated<f32>,
    /// 0 = yerinde, 1 = kaldırılmış (sürükleniyor).
    lift: Animated<f32>,
    hover: Interaction,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Drag {
    index: usize,
    /// İmleç ile satır üstü arası.
    grab: f32,
    pointer: Point,
    /// Şu anki ekleme yeri.
    insert: usize,
    /// Basılan nokta.
    origin: Point,
    /// Eşik bir kez aşıldı: başlangıca geri dönülse de bırakış tıklama sayılmaz.
    moved: bool,
}

/// Gövdeye basıldı, henüz sürükleme eşiği aşılmadı.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    index: usize,
    grab: f32,
    origin: Point,
}

#[derive(Debug, Clone)]
pub struct ReorderList {
    entries: Vec<Entry>,
    rect: Rect,
    drag: Option<Drag>,
    press: Option<Press>,
    focus: Option<usize>,
    damage: DamageState,
    /// Satır başına (satır, tutamaç, kaldır) bölgeleri; imleç için son konumlar.
    hits: Vec<(Rect, Rect, Rect)>,
}

impl ReorderList {
    pub fn new(theme: &Theme, items: Vec<ReorderItem>) -> Self {
        let mut list = Self { entries: Vec::new(), rect: Rect::ZERO, drag: None, press: None, focus: None, damage: DamageState::new(), hits: Vec::new() };
        for (i, item) in items.into_iter().enumerate() {
            let e = Self::entry(theme, item, Self::slot_y_static(theme, i));
            list.entries.push(e);
        }
        list
    }

    fn entry(theme: &Theme, item: ReorderItem, y: f32) -> Entry {
        Entry {
            label: Label::new(item.label.clone(), text_style(theme, TextRole::Body).with_max_lines(1)),
            icon: item.icon.clone().map(|i| IconView::with_role(theme, i, TextRole::IconSmall)),
            handle: IconView::with_role(theme, Icon::named(IconName::DragHandle), TextRole::IconSmall),
            remove: IconButton::new(theme, Icon::named(IconName::Close), IconButtonVariant::Standard),
            y: Animated::new(y, theme.motion.spatial),
            lift: Animated::new(0.0, theme.motion.spatial_fast),
            hover: Interaction::new(theme),
            item,
        }
    }

    pub fn items(&self) -> Vec<&ReorderItem> {
        self.entries.iter().map(|e| &e.item).collect()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Dışarıdan öğe ekler (listeler arası taşımada hedef); yeni öğe yerine süzülür.
    pub fn insert(&mut self, theme: &Theme, index: usize, item: ReorderItem, now: Instant) {
        let index = index.min(self.entries.len());
        let start = Self::slot_y_static(theme, index) - theme.space(4);
        self.entries.insert(index, Self::entry(theme, item, start));
        self.relayout(theme, now);
    }

    pub fn remove(&mut self, theme: &Theme, index: usize, now: Instant) -> Option<ReorderItem> {
        if index >= self.entries.len() {
            return None;
        }
        let e = self.entries.remove(index);
        self.drag = None;
        self.press = None;
        self.focus = self.focus.filter(|_| !self.entries.is_empty()).map(|f| f.min(self.entries.len() - 1));
        self.relayout(theme, now);
        Some(e.item)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for e in &mut self.entries {
            e.label.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
            e.y.set_motion(theme.motion.spatial, now);
            e.lift.set_motion(theme.motion.spatial_fast, now);
            e.remove.set_theme(theme, now);
            e.hover.set_theme(theme, now);
            e.handle.set_theme(theme, now);
            if let Some(i) = &mut e.icon {
                i.set_theme(theme, now);
            }
        }
        self.relayout(theme, now);
    }

    fn row_h(theme: &Theme) -> f32 {
        theme.space(12)
    }

    fn slot_y_static(theme: &Theme, i: usize) -> f32 {
        i as f32 * (Self::row_h(theme) + theme.space(1))
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn preferred_height(&self, theme: &Theme) -> f32 {
        Self::slot_y_static(theme, self.entries.len()) - theme.space(1)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        if self.rect != rect {
            self.rect = rect;
            self.relayout(theme, now);
        }
    }

    /// Hedef yuvalar: sürükleme varsa ekleme yeri boş bırakılır.
    fn relayout(&mut self, theme: &Theme, now: Instant) {
        let drag = self.drag;
        let mut slot = 0;
        for i in 0..self.entries.len() {
            if drag.is_some_and(|d| d.index == i) {
                continue;
            }
            if drag.is_some_and(|d| d.insert == slot) {
                slot += 1;
            }
            self.entries[i].y.set_target(Self::slot_y_static(theme, slot), now);
            slot += 1;
        }
        let w = self.rect.width - theme.space(10) * 2.0 - theme.space(8);
        for e in &mut self.entries {
            e.label.set_max_width(Some(w.max(0.0)));
        }
        self.damage.mark();
    }

    fn row_rect(&self, theme: &Theme, i: usize, now: Instant) -> Rect {
        let y = match self.drag {
            Some(d) if d.index == i => d.pointer.y - d.grab - self.rect.y,
            _ => self.entries[i].y.value(now),
        };
        Rect::new(self.rect.x, self.rect.y + y, self.rect.width, Self::row_h(theme))
    }

    fn handle_rect(theme: &Theme, row: Rect) -> Rect {
        Rect::new(row.x, row.y, theme.space(10), row.height)
    }

    fn remove_rect(theme: &Theme, row: Rect) -> Rect {
        let s = theme.space(8);
        Rect::new(row.right() - s - theme.space(2), row.center().y - s * 0.5, s, s)
    }

    /// İmlecin y'sine göre ekleme yeri.
    fn insert_at(&self, theme: &Theme, y: f32) -> usize {
        let pitch = Self::row_h(theme) + theme.space(1);
        let local = y - self.rect.y;
        ((local / pitch).floor().max(0.0) as usize).min(self.entries.len().saturating_sub(1))
    }

    /// Tıklamayı sürüklemeden ayıran hareket eşiği.
    fn slop(theme: &Theme) -> f32 {
        theme.space(2)
    }

    fn beyond_slop(theme: &Theme, origin: Point, p: Point) -> bool {
        let d = p - origin;
        d.x.hypot(d.y) >= Self::slop(theme)
    }

    fn start_drag(&mut self, theme: &Theme, index: usize, grab: f32, p: Point, origin: Point, now: Instant) {
        let insert = self.insert_at(theme, p.y - grab + Self::row_h(theme) * 0.5);
        let moved = Self::beyond_slop(theme, origin, p);
        self.drag = Some(Drag { index, grab, pointer: p, insert, origin, moved });
        self.entries[index].lift.set_target(1.0, now);
        self.focus = None;
        self.relayout(theme, now);
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<ReorderEvent> {
        let p = event.position;
        if let Some(pr) = self.press {
            match event.kind {
                PointerKind::Motion if Self::beyond_slop(theme, pr.origin, p) => {
                    self.press = None;
                    self.entries[pr.index].hover.set_state(true, false, now);
                    self.start_drag(theme, pr.index, pr.grab, p, pr.origin, now);
                    return Response::repaint();
                }
                PointerKind::Motion => return Response::none(),
                PointerKind::Release(Button::Left) => {
                    self.press = None;
                    let row = self.row_rect(theme, pr.index, now);
                    self.entries[pr.index].hover.set_state(row.contains(p), false, now);
                    self.damage.mark();
                    return if row.contains(p) { Response::event(ReorderEvent::Activated(pr.index)) } else { Response::repaint() };
                }
                PointerKind::Leave => self.press = None,
                _ => return Response::none(),
            }
        }
        if let Some(mut d) = self.drag {
            match event.kind {
                PointerKind::Motion => {
                    d.pointer = p;
                    d.moved |= Self::beyond_slop(theme, d.origin, p);
                    let center = p.y - d.grab + Self::row_h(theme) * 0.5;
                    d.insert = self.insert_at(theme, center);
                    let changed = self.drag.map(|o| o.insert) != Some(d.insert);
                    self.drag = Some(d);
                    if changed {
                        self.relayout(theme, now);
                    }
                    self.damage.mark();
                    return Response::repaint();
                }
                PointerKind::Release(Button::Left) | PointerKind::Leave => {
                    self.drag = None;
                    let from = d.index;
                    // Bırakılan öğe imlecin olduğu yerden yuvasına yaylanır.
                    let y = p.y - d.grab - self.rect.y;
                    self.entries[from].y.jump_to(y);
                    self.entries[from].lift.set_target(0.0, now);
                    // Tutamaçtan bas-bırak, eşik hiç aşılmadıysa tıklamadır.
                    if event.kind != PointerKind::Leave && !d.moved && !Self::beyond_slop(theme, d.origin, p) {
                        self.relayout(theme, now);
                        return Response::event(ReorderEvent::Activated(from));
                    }
                    let out_of_list = !self.rect.expand(theme.space(6)).contains(p);
                    if out_of_list && event.kind != PointerKind::Leave {
                        self.relayout(theme, now);
                        return Response::event(ReorderEvent::DraggedOut { index: from, position: p });
                    }
                    let to = d.insert;
                    if to != from {
                        let e = self.entries.remove(from);
                        self.entries.insert(to, e);
                        self.focus = self.focus.map(|_| to);
                    }
                    self.relayout(theme, now);
                    return if to != from { Response::event(ReorderEvent::Moved { from, to }) } else { Response::repaint() };
                }
                _ => return Response::none(),
            }
        }

        let mut out = Response::none();
        self.hits.clear();
        for i in 0..self.entries.len() {
            let row = self.row_rect(theme, i, now);
            self.hits.push((row, Self::handle_rect(theme, row), Self::remove_rect(theme, row)));
            self.entries[i].remove.set_rect(Self::remove_rect(theme, row));
            let r = self.entries[i].remove.pointer(event, now);
            out.repaint |= r.repaint;
            if r.event.is_some() {
                self.remove(theme, i, now);
                return Response::event(ReorderEvent::Removed(i));
            }
            let hovered = row.contains(p) && event.kind != PointerKind::Leave;
            if self.entries[i].hover.set_state(hovered, false, now) {
                out.repaint = true;
                self.damage.mark();
            }
            if let PointerKind::Press(Button::Left) = event.kind
                && row.contains(p)
                && !Self::remove_rect(theme, row).contains(p)
            {
                // Tutamaç hemen kaldırır; gövde eşik aşılana dek tıklama adayıdır.
                if Self::handle_rect(theme, row).contains(p) {
                    self.start_drag(theme, i, p.y - row.y, p, p, now);
                } else {
                    self.press = Some(Press { index: i, grab: p.y - row.y, origin: p });
                    self.entries[i].hover.set_state(true, true, now);
                    self.damage.mark();
                }
                return Response::repaint();
            }
        }
        out
    }

    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<ReorderEvent> {
        if event.state == KeyState::Released || self.entries.is_empty() {
            return Response::none();
        }
        let n = self.entries.len();
        let f = self.focus.unwrap_or(0);
        let alt = event.modifiers.alt;
        match event.key {
            Key::Up | Key::Down if alt => {
                let to = if event.key == Key::Up { f.saturating_sub(1) } else { (f + 1).min(n - 1) };
                if self.focus.is_none() || to == f {
                    self.focus = Some(f);
                    self.damage.mark();
                    return Response::repaint();
                }
                let e = self.entries.remove(f);
                self.entries.insert(to, e);
                self.focus = Some(to);
                self.relayout(theme, now);
                Response::event(ReorderEvent::Moved { from: f, to })
            }
            Key::Up => {
                self.focus = Some(if self.focus.is_some() { f.saturating_sub(1) } else { 0 });
                self.damage.mark();
                Response::repaint()
            }
            Key::Down => {
                self.focus = Some(if self.focus.is_some() { (f + 1).min(n - 1) } else { 0 });
                self.damage.mark();
                Response::repaint()
            }
            Key::Enter | Key::Space if self.focus.is_some() => Response::event(ReorderEvent::Activated(f)),
            Key::Delete if self.focus.is_some() => {
                self.remove(theme, f, now);
                Response::event(ReorderEvent::Removed(f))
            }
            _ => Response::none(),
        }
    }

    /// İmleç: sürüklerken `Grabbing`, tutamaç `Grab`, kaldır düğmesi ve satır gövdesi `Pointer`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.drag.is_some() {
            return Some(CursorShape::Grabbing);
        }
        let (_, handle, _) = self.hits.iter().find(|(row, _, _)| row.contains(p))?;
        Some(if handle.contains(p) { CursorShape::Grab } else { CursorShape::Pointer })
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let radii = Corners::uniform(theme.shape.radius_medium);
        self.hits.resize(self.entries.len(), (Rect::ZERO, Rect::ZERO, Rect::ZERO));
        // Sürüklenen öğe en üstte çizilir.
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        if let Some(d) = self.drag {
            order.retain(|&i| i != d.index);
            order.push(d.index);
        } else if let Some(top) = (0..self.entries.len()).find(|&i| !self.entries[i].lift.is_settled(now)) {
            order.retain(|&i| i != top);
            order.push(top);
        }
        let mut animating = self.drag.is_some();
        for i in order {
            let mut row = self.row_rect(theme, i, now);
            let lift = self.entries[i].lift.value(now).max(0.0);
            // Kaldırılan öğe hafifçe büyür.
            row = row.expand(theme.space(1) * 0.5 * lift);
            if lift > 0.01 && theme.shadow.enabled {
                let sh = ShadowStyle {
                    color: ui.fade(theme.shadow.color.multiply_alpha(lift.min(1.0))),
                    blur: theme.shadow.blur * lift,
                    offset: Point::new(0.0, theme.shadow.offset_y + theme.space(1) * lift),
                    spread: 0.0,
                };
                ui.painter.shadow(row, radii, &sh);
            }
            let fill = c.surface_container.lerp(&c.surface_bright, lift.min(1.0));
            ui.painter.rect(row, &RectStyle::filled(ui.fade(fill), radii));
            let e = &mut self.entries[i];
            e.hover.draw_state_layer_under(ui, row, radii, c.on_surface, fill, &[c.on_surface, c.on_surface_variant]);
            if self.focus == Some(i) {
                let style = RectStyle {
                    fill: nuxut_core::Color::TRANSPARENT,
                    radii,
                    border: Some(nuxut_render::Border { width: FOCUS_RING, color: ui.fade(crate::style::focus_ring(theme, &[fill])) }),
                };
                ui.painter.rect(row, &style);
            }
            let hr = Self::handle_rect(theme, row);
            if let Some(h) = self.hits.get_mut(i) {
                *h = (row, hr, Self::remove_rect(theme, row));
            }
            e.handle.draw_centered(ui.painter, hr.center(), ui.fade(c.on_surface_variant));
            let mut x = hr.right();
            if let Some(icon) = &mut e.icon {
                let w = icon.size().width;
                icon.draw_centered(ui.painter, Point::new(x + w * 0.5, row.center().y), ui.fade(c.primary));
                x += w + theme.space(3);
            }
            let ls = e.label.measure_in(ui.painter);
            e.label.draw(ui.painter, Point::new(x, row.center().y - ls.height * 0.5), ui.fade(c.on_surface));
            e.remove.set_rect(Self::remove_rect(theme, row));
            e.remove.draw(ui);
            animating |= !e.y.is_settled(now) | !e.lift.is_settled(now) | e.hover.is_animating(now);
        }
        if animating {
            ui.request_frame();
        }
        let h = self.preferred_height(theme);
        let extent = Rect::new(self.rect.x, self.rect.y, self.rect.width, h.max(self.rect.height))
            .expand(theme.shadow.blur * 2.0 + theme.space(2));
        let extent = match self.drag {
            Some(d) => extent.union(&Rect::new(self.rect.x, d.pointer.y - d.grab, self.rect.width, Self::row_h(theme)).expand(theme.shadow.blur * 2.0 + theme.space(2))),
            None => extent,
        };
        self.damage.report(ui, extent, animating);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;
    use crate::input::test_util::*;

    fn list() -> (ReorderList, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let items = ["Saat", "Çalışma alanları", "Tepsi", "Pil"].map(ReorderItem::new).to_vec();
        let mut l = ReorderList::new(&theme, items);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 300.0), t0);
        (l, theme, t0)
    }

    fn labels(l: &ReorderList) -> Vec<String> {
        l.items().iter().map(|i| i.label.clone()).collect()
    }

    #[test]
    fn drag_handle_moves_item_and_others_make_room() {
        let (mut l, theme, t0) = list();
        let pitch = ReorderList::row_h(&theme) + theme.space(1);
        let _ = l.pointer(&theme, &press(10.0, 10.0), t0);
        assert!(l.drag.is_some());
        let _ = l.pointer(&theme, &motion(10.0, 10.0 + pitch * 2.0), t0);
        // Diğerleri yer açar: 1 ve 2 yukarı kayar.
        assert_eq!(l.entries[1].y.target(), 0.0);
        let r = l.pointer(&theme, &release(10.0, 10.0 + pitch * 2.0), t0);
        assert_eq!(r.event, Some(ReorderEvent::Moved { from: 0, to: 2 }));
        assert_eq!(labels(&l), ["Çalışma alanları", "Tepsi", "Saat", "Pil"]);
    }

    #[test]
    fn drop_outside_reports_dragged_out() {
        let (mut l, theme, t0) = list();
        let _ = l.pointer(&theme, &press(10.0, 10.0), t0);
        let _ = l.pointer(&theme, &motion(900.0, 10.0), t0);
        let r = l.pointer(&theme, &release(900.0, 10.0), t0);
        assert_eq!(r.event, Some(ReorderEvent::DraggedOut { index: 0, position: Point::new(900.0, 10.0) }));
        assert_eq!(l.len(), 4, "liste değişmez");
    }

    #[test]
    fn keyboard_move_and_delete() {
        let (mut l, theme, t0) = list();
        let _ = l.key(&theme, &key(Key::Down), t0);
        let _ = l.key(&theme, &key(Key::Down), t0);
        let alt = Modifiers { alt: true, ..Default::default() };
        let r = l.key(&theme, &key_mod(Key::Up, alt), t0);
        assert_eq!(r.event, Some(ReorderEvent::Moved { from: 1, to: 0 }));
        assert_eq!(labels(&l)[0], "Çalışma alanları");
        assert_eq!(l.key(&theme, &key(Key::Delete), t0).event, Some(ReorderEvent::Removed(0)));
        assert_eq!(l.len(), 3);
    }

    #[test]
    fn click_below_slop_activates() {
        let (mut l, theme, t0) = list();
        let pitch = ReorderList::row_h(&theme) + theme.space(1);
        let y = pitch + 10.0;
        // Gövde: eşik altı titreme tıklamadır, liste değişmez.
        let _ = l.pointer(&theme, &press(150.0, y), t0);
        let _ = l.pointer(&theme, &motion(151.0, y + 1.0), t0);
        assert!(l.drag.is_none());
        assert_eq!(l.pointer(&theme, &release(151.0, y + 1.0), t0).event, Some(ReorderEvent::Activated(1)));
        // Tutamaç: bas-bırak da tıklamadır.
        let _ = l.pointer(&theme, &press(10.0, y), t0);
        assert_eq!(l.pointer(&theme, &release(10.0, y), t0).event, Some(ReorderEvent::Activated(1)));
        assert_eq!(labels(&l), ["Saat", "Çalışma alanları", "Tepsi", "Pil"]);
    }

    #[test]
    fn body_drag_past_slop_moves_not_activates() {
        let (mut l, theme, t0) = list();
        let pitch = ReorderList::row_h(&theme) + theme.space(1);
        let _ = l.pointer(&theme, &press(150.0, 10.0), t0);
        let _ = l.pointer(&theme, &motion(150.0, 10.0 + pitch * 2.0), t0);
        assert!(l.drag.is_some(), "eşik aşılınca gövdeden sürükleme başlar");
        let r = l.pointer(&theme, &release(150.0, 10.0 + pitch * 2.0), t0);
        assert_eq!(r.event, Some(ReorderEvent::Moved { from: 0, to: 2 }));
        // Eşiği aşıp aynı yuvaya dönen sürükleme tıklama değildir.
        let _ = l.pointer(&theme, &press(150.0, 10.0), t0);
        let _ = l.pointer(&theme, &motion(150.0, 10.0 + theme.space(4)), t0);
        assert_eq!(l.pointer(&theme, &release(150.0, 10.0 + theme.space(4)), t0).event, None);
        // Yüzeyden çıkış bekleyen tıklamayı iptal eder.
        let _ = l.pointer(&theme, &press(150.0, 10.0), t0);
        let _ = l.pointer(&theme, &PointerEvent::new(Point::new(150.0, 10.0), PointerKind::Leave), t0);
        assert_eq!(l.pointer(&theme, &release(150.0, 10.0), t0).event, None);
    }

    #[test]
    fn drag_returning_to_origin_is_not_a_click() {
        let (mut l, theme, t0) = list();
        let far = theme.space(4);
        // Tutamaçtan: eşiği aş, başlangıca dön, bırak.
        let _ = l.pointer(&theme, &press(10.0, 10.0), t0);
        let _ = l.pointer(&theme, &motion(10.0, 10.0 + far), t0);
        let _ = l.pointer(&theme, &motion(10.0, 10.0), t0);
        assert_eq!(l.pointer(&theme, &release(10.0, 10.0), t0).event, None);
        // Gövdeden: aynısı.
        let _ = l.pointer(&theme, &press(150.0, 10.0), t0);
        let _ = l.pointer(&theme, &motion(150.0, 10.0 + far), t0);
        let _ = l.pointer(&theme, &motion(150.0, 10.0), t0);
        assert_eq!(l.pointer(&theme, &release(150.0, 10.0), t0).event, None);
        // Hareket olayı gelmeden uzakta bırakılan tutamaç basışı da tıklama değildir.
        let _ = l.pointer(&theme, &press(10.0, 10.0), t0);
        assert_eq!(l.pointer(&theme, &release(10.0, 10.0 + far), t0).event, None);
        assert_eq!(labels(&l), ["Saat", "Çalışma alanları", "Tepsi", "Pil"]);
    }

    #[test]
    fn enter_activates_focused() {
        let (mut l, theme, t0) = list();
        assert_eq!(l.key(&theme, &key(Key::Enter), t0).event, None, "odak yokken etkinleştirme yok");
        let _ = l.key(&theme, &key(Key::Down), t0);
        let _ = l.key(&theme, &key(Key::Down), t0);
        assert_eq!(l.key(&theme, &key(Key::Enter), t0).event, Some(ReorderEvent::Activated(1)));
        assert_eq!(l.key(&theme, &key(Key::Space), t0).event, Some(ReorderEvent::Activated(1)));
    }

    #[test]
    fn remove_button() {
        let (mut l, theme, t0) = list();
        let rr = ReorderList::remove_rect(&theme, l.row_rect(&theme, 3, t0)).center();
        let _ = l.pointer(&theme, &motion(rr.x, rr.y), t0);
        let _ = l.pointer(&theme, &press(rr.x, rr.y), t0);
        assert_eq!(l.pointer(&theme, &release(rr.x, rr.y), t0).event, Some(ReorderEvent::Removed(3)));
        assert_eq!(labels(&l), ["Saat", "Çalışma alanları", "Tepsi"]);
    }
}
