//! Modal diyalog: zemini karartan scrim (temanın gölge rengi) + ortada `spatial` hareketle
//! büyüyerek beliren yüzey; başlık, gövde (çağıranın içeriği), sağda eylem düğmeleri.
//! Esc ya da scrim tıklaması `Dismissed` (kapatılamaz yapılabilir); Enter birincil eylem.

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, ShadowStyle, TextSystem};
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};
use crate::button::{Button, ButtonVariant};
use crate::input::{Button as MouseButton, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::label::Label;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Eylem en fazla (fazlası gösterilmez).
pub const MAX_DIALOG_ACTIONS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogActionKind {
    /// Metin düğmesi (İptal).
    #[default]
    Default,
    /// Dolu düğme; Enter bunu etkinleştirir.
    Primary,
    /// Yıkıcı (Sil): `error` rolü; Enter etkinleştirmez (kazara silme olmasın).
    Destructive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogAction {
    pub label: String,
    pub kind: DialogActionKind,
}

impl DialogAction {
    pub fn new(label: impl Into<String>, kind: DialogActionKind) -> Self {
        Self { label: label.into(), kind }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogEvent {
    /// Eylem indisi (düğme ya da Enter). Diyalog kapanmaya başlar.
    Action(usize),
    /// Esc ya da scrim tıklaması.
    Dismissed,
}

#[derive(Debug, Clone)]
pub struct Dialog {
    title: Label,
    actions: Vec<(DialogAction, Button)>,
    open: bool,
    dismissible: bool,
    /// 0 = kapalı, 1 = açık; yüzey ölçeği (taşmalı) ve opaklık bundan.
    grow: Animated<f32>,
    fade: Animated<f32>,
    bounds: Rect,
    content: Size,
    surface: Rect,
    title_h: f32,
    damage: DamageState,
}

impl Dialog {
    pub fn new(theme: &Theme) -> Self {
        Self {
            title: Label::new("", text_style(theme, TextRole::Title)),
            actions: Vec::new(),
            open: false,
            dismissible: true,
            grow: Animated::new(0.0, theme.motion.spatial),
            fade: Animated::new(0.0, theme.motion.effects),
            bounds: Rect::ZERO,
            content: Size::ZERO,
            surface: Rect::ZERO,
            title_h: 0.0,
            damage: DamageState::new(),
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Kapanış animasyonu dahil görünür mü.
    pub fn is_visible(&mut self, now: Instant) -> bool {
        self.open || !self.fade.is_settled(now) || self.fade.value(now) > 0.0
    }

    /// `false`: Esc ve scrim tıklaması kapatmaz (yalnızca eylemler).
    pub fn set_dismissible(&mut self, on: bool) {
        self.dismissible = on;
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.title.set_style(text_style(theme, TextRole::Title));
        self.grow.set_motion(theme.motion.spatial, now);
        self.fade.set_motion(theme.motion.effects, now);
        for (_, b) in &mut self.actions {
            b.set_theme(theme, now);
        }
        self.damage.mark();
    }

    fn pad(theme: &Theme) -> f32 {
        theme.space(6)
    }

    /// Açar. `content`: gövdenin boyutu (çağıran çizer); `bounds`: diyalogun ortalandığı alan
    /// (genelde tüm yüzey; scrim bunu kaplar).
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        text: &mut TextSystem,
        theme: &Theme,
        title: &str,
        actions: Vec<DialogAction>,
        content: Size,
        bounds: Rect,
        now: Instant,
    ) {
        self.title.set_text(title);
        self.actions = actions
            .into_iter()
            .take(MAX_DIALOG_ACTIONS)
            .map(|a| {
                let variant = match a.kind {
                    DialogActionKind::Default => ButtonVariant::Text,
                    DialogActionKind::Primary => ButtonVariant::Filled,
                    DialogActionKind::Destructive => ButtonVariant::Destructive,
                };
                let b = Button::new(theme, variant).with_label(theme, a.label.clone());
                (a, b)
            })
            .collect();
        self.content = content;
        self.bounds = bounds;
        self.open = true;
        self.grow.set_target(1.0, now);
        self.fade.set_target(1.0, now);
        self.layout(text, theme);
        self.damage.mark();
    }

    pub fn close(&mut self, now: Instant) {
        if self.open {
            self.open = false;
            self.grow.set_target(0.0, now);
            self.fade.set_target(0.0, now);
            self.damage.mark();
        }
    }

    fn layout(&mut self, text: &mut TextSystem, theme: &Theme) {
        let pad = Self::pad(theme);
        let ts = self.title.measure(text);
        self.title_h = ts.height;
        let sizes: Vec<Size> = self.actions.iter_mut().map(|(_, b)| b.preferred_size(text, theme)).collect();
        let actions_w: f32 = sizes.iter().map(|s| s.width).sum::<f32>() + theme.space(2) * sizes.len().saturating_sub(1) as f32;
        let min_w = theme.space(70);
        let w = (self.content.width.max(ts.width).max(actions_w) + pad * 2.0).max(min_w).min(self.bounds.width - theme.space(8));
        let actions_h = if self.actions.is_empty() { 0.0 } else { theme.space(10) + pad };
        let h = (pad + ts.height + theme.space(4) + self.content.height + actions_h + pad).min(self.bounds.height - theme.space(8));
        let c = self.bounds.center();
        self.surface = Rect::new((c.x - w * 0.5).round(), (c.y - h * 0.5).round(), w, h);
        // Eylemler sağa yaslı.
        let mut x = self.surface.right() - pad;
        let y = self.surface.bottom() - pad - theme.space(10);
        for ((_, b), s) in self.actions.iter_mut().zip(sizes).rev() {
            x -= s.width;
            b.set_rect(Rect::new(x, y, s.width, s.height));
            x -= theme.space(2);
        }
    }

    /// Gövdenin yeri (çağıran içeriği buraya yerleştirir).
    pub fn content_rect(&self, theme: &Theme) -> Rect {
        let pad = Self::pad(theme);
        let th = self.title_h;
        Rect::new(self.surface.x + pad, self.surface.y + pad + th + theme.space(4), self.content.width.min(self.surface.width - pad * 2.0), self.content.height)
    }

    pub fn surface_rect(&self) -> Rect {
        self.surface
    }

    fn finish(&mut self, i: usize, now: Instant) -> Response<DialogEvent> {
        self.close(now);
        Response::event(DialogEvent::Action(i))
    }

    /// Modal: açıkken tüm işaretçi olaylarını tüketir (altındakilere gitmez). İçerik
    /// olaylarını çağıran `content_rect` içinde kendisi iletir.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<DialogEvent> {
        if !self.open {
            return Response::none();
        }
        let mut out = Response::none();
        for i in 0..self.actions.len() {
            let r = self.actions[i].1.pointer(event, now);
            out.repaint |= r.repaint;
            if r.event.is_some() {
                return self.finish(i, now);
            }
        }
        if let PointerKind::Press(MouseButton::Left) = event.kind
            && !self.surface.contains(event.position)
            && self.dismissible
        {
            self.close(now);
            return Response::event(DialogEvent::Dismissed);
        }
        if out.repaint {
            self.damage.mark();
        }
        out
    }

    /// Esc → `Dismissed` (kapatılabilirse), Enter → birincil eylem.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<DialogEvent> {
        if !self.open || event.state == KeyState::Released {
            return Response::none();
        }
        match event.key {
            Key::Escape if self.dismissible => {
                self.close(now);
                Response::event(DialogEvent::Dismissed)
            }
            Key::Enter => match self.actions.iter().position(|(a, _)| a.kind == DialogActionKind::Primary) {
                Some(i) => self.finish(i, now),
                None => Response::none(),
            },
            _ => Response::none(),
        }
    }

    /// İmleç: düğmeler `Pointer`, diyalog ve scrim `Default` (modal: altındakilere sorulmaz).
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.open {
            return None;
        }
        Some(self.actions.iter().find_map(|(_, b)| b.cursor(p)).unwrap_or(CursorShape::Default))
    }

    /// Scrim + yüzey + başlık + eylemler; gövdeyi `f(ui, content_rect)` çizer (yüzeye kırpılır).
    pub fn draw(&mut self, ui: &mut Ui, f: impl FnOnce(&mut Ui, Rect)) {
        let now = ui.now;
        let theme = ui.theme;
        let a = self.fade.value(now).clamp(0.0, 1.0);
        let g = self.grow.value(now).max(0.0);
        let animating = !self.fade.is_settled(now) | !self.grow.is_settled(now);
        if a <= 0.0 && !animating {
            self.damage.report_hidden(ui);
            return;
        }
        // Scrim: temanın gölge rengi (opaklığı tema `shadow.opacity`).
        ui.painter.rect(self.bounds, &RectStyle::filled(ui.fade(theme.shadow.color.multiply_alpha(a)), nuxut_core::Corners::ZERO));
        // Yüzey ortadan büyür (%90 → %100, taşmalı).
        let k = 0.9 + 0.1 * g;
        let s = self.surface;
        let c = s.center();
        let r = Rect::new(c.x - s.width * k * 0.5, c.y - s.height * k * 0.5, s.width * k, s.height * k);
        let radii = theme.shape.large();
        ui.with_alpha(a, |ui| {
            if theme.shadow.enabled {
                let sh = ShadowStyle {
                    color: ui.fade(theme.shadow.color),
                    blur: theme.shadow.blur * 1.5,
                    offset: Point::new(0.0, theme.shadow.offset_y * 2.0),
                    spread: 0.0,
                };
                ui.painter.shadow(r, radii, &sh);
            }
            ui.painter.rect(r, &RectStyle::filled(ui.fade(theme.colors.surface_container), radii));
            ui.painter.push_clip(r);
            let pad = Self::pad(theme);
            self.title.draw(ui.painter, Point::new(s.x + pad, s.y + pad), ui.fade(theme.colors.on_surface));
            let content = self.content_rect(theme);
            f(ui, content);
            for (_, b) in &mut self.actions {
                b.draw(ui);
            }
            ui.painter.pop_clip();
        });
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.bounds, animating);
    }
}

// Erişilebilirlik (ADR-0011).
impl Dialog {
    /// Açıkken `Role::Dialog` (modal; ad başlık): çocukları gövde (`content` çağıranın
    /// bileşenlerini ekler) ve eylem düğmeleri (anahtar `("action", i)`). Kapalıyken (kapanış
    /// animasyonunda da) ağaçta yok: `None`.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash, content: impl FnOnce(&mut AccessTree)) -> Option<AccessNode<'t>> {
        if !self.open {
            return None;
        }
        let mut node = tree.push(key, Role::Dialog).label(self.title.text()).bounds(self.surface);
        node.node_mut().set_modal();
        let id = node.id();
        tree.within(id, |tree| {
            content(tree);
            for (i, (_, b)) in self.actions.iter().enumerate() {
                b.access(tree, ("action", i));
            }
        });
        tree.get(id)
    }

    /// AT `Click` eylem düğmesinde → `Action(i)` (tıklamayla aynı; diyalog kapanmaya başlar).
    /// `container`: `access`'in döndürdüğü kimlik.
    pub fn access_action(&mut self, container: NodeId, request: AccessRequest, now: Instant) -> Response<DialogEvent> {
        if !self.open || request.action != AccessAction::Click {
            return Response::none();
        }
        match (0..self.actions.len()).find(|&i| child_id(container, ("action", i)) == request.node) {
            Some(i) => self.finish(i, now),
            None => Response::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn dialog() -> (Dialog, Theme, Instant) {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut d = Dialog::new(&theme);
        d.open(
            &mut text,
            &theme,
            "Hesap silinsin mi?",
            vec![
                DialogAction::new("İptal", DialogActionKind::Default),
                DialogAction::new("Sil", DialogActionKind::Destructive),
                DialogAction::new("Kaydet", DialogActionKind::Primary),
            ],
            Size::new(300.0, 80.0),
            Rect::new(0.0, 0.0, 1000.0, 800.0),
            t0,
        );
        (d, theme, t0)
    }

    #[test]
    fn centered_and_actions_right_aligned() {
        let (d, theme, _) = dialog();
        let s = d.surface_rect();
        assert!((s.center().x - 500.0).abs() <= 1.0 && (s.center().y - 400.0).abs() <= 1.0);
        let last = d.actions.last().unwrap().1.rect();
        assert!((s.right() - Dialog::pad(&theme) - last.right()).abs() < 0.5);
        assert!(d.content_rect(&theme).width <= s.width);
    }

    #[test]
    fn escape_scrim_enter_and_buttons() {
        let (mut d, _theme, t0) = dialog();
        assert_eq!(d.key(&key(Key::Enter), t0).event, Some(DialogEvent::Action(2)), "Enter birincil");
        let (mut d, _theme, t0) = dialog();
        assert_eq!(d.pointer(&press(5.0, 5.0), t0).event, Some(DialogEvent::Dismissed));
        let (mut d, _theme, t0) = dialog();
        d.set_dismissible(false);
        assert!(d.key(&key(Key::Escape), t0).event.is_none());
        assert!(d.pointer(&press(5.0, 5.0), t0).event.is_none());
        let b = d.actions[1].1.rect().center();
        assert_eq!(d.cursor(b), Some(CursorShape::Pointer));
        assert_eq!(d.cursor(Point::new(5.0, 5.0)), Some(CursorShape::Default), "modal");
        let _ = d.pointer(&motion(b.x, b.y), t0);
        let _ = d.pointer(&press(b.x, b.y), t0);
        assert_eq!(d.pointer(&release(b.x, b.y), t0).event, Some(DialogEvent::Action(1)));
        assert!(!d.is_open());
    }

    #[test]
    fn access_tree_and_actions() {
        let (mut d, _theme, t0) = dialog();
        let mut tree = AccessTree::new();
        let id = d
            .access(&mut tree, "dlg", |tree| {
                tree.push("body", Role::Label).value("gövde");
            })
            .map(|n| n.id())
            .expect("açık");
        let node = tree.node(id).unwrap();
        assert_eq!((node.role(), node.label(), node.is_modal()), (Role::Dialog, Some("Hesap silinsin mi?"), true));
        let labels: Vec<_> = tree.nodes().filter(|(_, n)| n.role() == Role::Button).filter_map(|(_, n)| n.label()).collect();
        assert_eq!(labels, ["İptal", "Sil", "Kaydet"]);
        assert!(tree.node(child_id(id, "body")).is_some(), "gövde diyalogun çocuğu");
        assert!(tree.unlabeled().is_empty());
        let click = |i: usize| AccessRequest { node: child_id(id, ("action", i)), action: AccessAction::Click };
        assert!(d.access_action(id, AccessRequest { node: id, action: AccessAction::Click }, t0).event.is_none());
        assert_eq!(d.access_action(id, click(1), t0).event, Some(DialogEvent::Action(1)));
        assert!(!d.is_open());
        assert!(d.access_action(id, click(2), t0).event.is_none(), "kapalıyken eylem yok");
        assert!(d.access(&mut AccessTree::new(), "dlg", |_| {}).is_none());
    }
}
