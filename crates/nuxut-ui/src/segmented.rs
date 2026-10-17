//! Bölümlü düğme: 2–5 seçenek (ikon ve/veya etiket); seçili gösterge `spatial` hareketle
//! seçenekler arasında kayan bir hap. Sığmayan etiket `…` ile kırpılır.

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};
use crate::icon::{Icon, IconView};
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::style::{FOCUS_RING, disabled_container, disabled_content, focus_ring, pill};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Seçenek sayısı sınırları.
pub const MIN_SEGMENTS: usize = 2;
pub const MAX_SEGMENTS: usize = 5;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Segment {
    pub label: String,
    pub icon: Option<Icon>,
    /// Bu seçenek devre dışı: soluk çizilir, fare/klavye ile seçilemez, klavye gezinmesi atlar.
    pub disabled: bool,
}

impl Segment {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), icon: None, disabled: false }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Kurulurken devre dışı (sonradan: [`SegmentedButton::set_segment_enabled`]).
    pub fn with_disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Debug, Clone)]
struct Seg {
    icon: Option<IconView>,
    label: Label,
    interaction: Interaction,
    enabled: bool,
}

#[derive(Debug, Clone)]
pub struct SegmentedButton {
    segs: Vec<Seg>,
    selected: usize,
    rect: Rect,
    indicator: Animated<Rect>,
    enabled: bool,
    focus_visible: bool,
    damage: DamageState,
}

impl SegmentedButton {
    /// `segments` 2..=5 arasına kırpılır (eksikse boş seçenek eklenmez; 1 seçenek de çizilir).
    pub fn new(theme: &Theme, segments: Vec<Segment>, selected: usize) -> Self {
        let style = text_style(theme, TextRole::Label).with_max_lines(1);
        let segs: Vec<Seg> = segments
            .into_iter()
            .take(MAX_SEGMENTS)
            .map(|s| Seg {
                icon: s.icon.map(|i| IconView::with_role(theme, i, TextRole::IconSmall)),
                label: Label::new(s.label, style.clone()),
                interaction: Interaction::new(theme),
                enabled: !s.disabled,
            })
            .collect();
        debug_assert!(segs.len() >= MIN_SEGMENTS, "bölümlü düğme en az {MIN_SEGMENTS} seçenek ister");
        let selected = selected.min(segs.len().saturating_sub(1));
        Self {
            segs,
            selected,
            rect: Rect::ZERO,
            indicator: Animated::new(Rect::ZERO, theme.motion.spatial),
            enabled: true,
            focus_visible: false,
            damage: DamageState::new(),
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn len(&self) -> usize {
        self.segs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// Dış durum; gösterge yayla kayar.
    pub fn set_selected(&mut self, index: usize, now: Instant) -> bool {
        let index = index.min(self.segs.len().saturating_sub(1));
        if index == self.selected {
            return false;
        }
        self.selected = index;
        self.indicator.set_target(self.indicator_rect(index), now);
        self.damage.mark();
        true
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        if self.enabled != enabled {
            self.enabled = enabled;
            for s in &mut self.segs {
                s.interaction.reset(now);
            }
            self.damage.mark();
        }
    }

    /// Tek seçeneği etkinleştirir/devre dışı bırakır (aralık dışı indis etkisiz). Devre dışı
    /// seçenek soluk çizilir, tıklanamaz, klavye gezinmesi onu atlar. Seçili seçenek devre dışı
    /// yapılırsa seçim DEĞİŞMEZ — yalnız etkileşim kapanır (docs/design/segmented.md).
    pub fn set_segment_enabled(&mut self, index: usize, enabled: bool, now: Instant) {
        let Some(s) = self.segs.get_mut(index) else { return };
        if s.enabled != enabled {
            s.enabled = enabled;
            s.interaction.reset(now);
            self.damage.mark();
        }
    }

    /// Seçenek etkin mi (aralık dışı → `false`). Bileşenin tamamı kapalıysa da seçenek bazında
    /// değer korunur; etkileşim için ikisi birlikte gerekir.
    pub fn is_segment_enabled(&self, index: usize) -> bool {
        self.segs.get(index).is_some_and(|s| s.enabled)
    }

    pub fn set_focus_visible(&mut self, on: bool) {
        if self.focus_visible != on {
            self.focus_visible = on;
            self.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        let style = text_style(theme, TextRole::Label).with_max_lines(1);
        for s in &mut self.segs {
            s.label.set_style(style.clone());
            s.interaction.set_theme(theme, now);
            if let Some(i) = &mut s.icon {
                i.set_theme(theme, now);
            }
        }
        self.indicator.set_motion(theme.motion.spatial, now);
        self.damage.mark();
    }

    fn seg_padding(theme: &Theme) -> f32 {
        theme.space(4)
    }

    /// Doğal boyut: her seçenek en genişinin genişliğinde, yükseklik `space(10)`.
    pub fn preferred_size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        let gap = theme.space(2);
        let widest = self
            .segs
            .iter_mut()
            .map(|s| {
                let icon = s.icon.as_mut().map_or(0.0, |i| i.measure(text).width + gap);
                icon + s.label.measure(text).width
            })
            .fold(0.0, f32::max);
        let inset = theme.space(1);
        let w = (widest + Self::seg_padding(theme) * 2.0) * self.segs.len() as f32 + inset * 2.0;
        Size::new(w.ceil(), theme.space(10))
    }

    fn seg_rect(&self, i: usize) -> Rect {
        let n = self.segs.len().max(1) as f32;
        let w = self.rect.width / n;
        Rect::new(self.rect.x + w * i as f32, self.rect.y, w, self.rect.height)
    }

    fn indicator_rect(&self, i: usize) -> Rect {
        // Gösterge rayın içinde, kenarlardan eşit boşlukla.
        let inset = (self.rect.height * 0.1).round();
        let r = self.seg_rect(i);
        Rect::new(r.x + inset, r.y + inset, r.width - inset * 2.0, r.height - inset * 2.0)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.rect == rect {
            return;
        }
        self.rect = rect;
        let gap = theme.space(2);
        let pad = Self::seg_padding(theme);
        let n = self.segs.len().max(1) as f32;
        let seg_w = rect.width / n;
        for s in &mut self.segs {
            let icon = s.icon.as_ref().map_or(0.0, |i| i.size().width + gap);
            s.label.set_max_width(Some((seg_w - pad * 2.0 - icon).max(0.0)));
        }
        self.indicator.jump_to(self.indicator_rect(self.selected));
        self.damage.mark();
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Yeni seçimde `event = Some(indis)`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<usize> {
        if !self.enabled {
            return Response::none();
        }
        let mut out = Response::none();
        let mut clicked = None;
        for i in 0..self.segs.len() {
            if !self.segs[i].enabled {
                continue;
            }
            let r = self.seg_rect(i);
            let tap = self.segs[i].interaction.handle(r, event, now);
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
            if self.set_selected(i, now) {
                return Response::event(i);
            }
        }
        out
    }

    /// Odaktayken Sol/Sağ (ve Home/End) seçimi değiştirir; devre dışı seçenekler atlanır
    /// (o yönde etkin seçenek yoksa seçim yerinde kalır).
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<usize> {
        if !self.enabled || event.state == KeyState::Released {
            return Response::none();
        }
        let cur = self.selected;
        let enabled = |i: &usize| self.segs[*i].enabled;
        let next = match event.key {
            Key::Left => (0..cur).rev().find(enabled),
            Key::Right => (cur + 1..self.segs.len()).find(enabled),
            Key::Home => (0..self.segs.len()).find(enabled),
            Key::End => (0..self.segs.len()).rev().find(enabled),
            _ => return Response::none(),
        };
        self.focus_visible = true;
        match next {
            Some(i) if self.set_selected(i, now) => Response::event(i),
            _ => Response::repaint(),
        }
    }

    /// İmleç: etkin seçenekler `Pointer`; devre dışı bileşen/seçenek `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        let n = self.segs.len().max(1);
        let i = (((p.x - self.rect.x) / self.rect.width.max(1.0)) * n as f32) as usize;
        let on = self.enabled && self.is_segment_enabled(i.min(n - 1));
        Some(if on { CursorShape::Pointer } else { CursorShape::Default })
    }

    fn track(&self, theme: &Theme) -> Color {
        if self.enabled { theme.colors.surface_bright } else { disabled_container(theme) }
    }

    /// Seçili seçenek tek başına devre dışı mı (bileşen etkinken).
    fn selected_disabled(&self) -> bool {
        self.enabled && !self.is_segment_enabled(self.selected)
    }

    /// Gösterge dolgusu. Seçili seçenek tek başına devre dışıysa gösterge yerinde kalır ama
    /// vurgusunu yitirir: devre dışı kap (%12 `on_surface`) — seçim görünür, etkileşim yok.
    fn indicator_fill(&self, theme: &Theme) -> Color {
        let c = &theme.colors;
        if !self.enabled {
            disabled_content(theme, c.on_surface, &[self.track(theme)])
        } else if self.selected_disabled() {
            disabled_container(theme)
        } else {
            c.primary
        }
    }

    /// `i`. seçeneğin içerik rengi; `overlap` göstergeyle örtüşme oranı (0..=1, kayarken yumuşak).
    /// Devre dışı içerik `disabled_content`: yüksek karşıtlıkta altındaki zemine ≥ 3:1 yükseltilir.
    fn content_color(&self, theme: &Theme, i: usize, overlap: f32) -> Color {
        let c = &theme.colors;
        let track = self.track(theme);
        if !self.enabled {
            return disabled_content(theme, c.on_surface, &[track]);
        }
        let sel_disabled = self.selected_disabled();
        let normal = if sel_disabled { c.on_surface_variant } else { c.on_surface_variant.lerp(&c.on_primary, overlap) };
        if !sel_disabled && self.is_segment_enabled(i) {
            return normal;
        }
        let under = [track, disabled_container(theme)];
        let under: &[Color] = if sel_disabled && overlap > 0.0 { &under } else { &under[..1] };
        let dim = disabled_content(theme, c.on_surface, under);
        if self.is_segment_enabled(i) { normal.lerp(&dim, overlap) } else { dim }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.rect;
        let track = self.track(theme);
        ui.painter.rect(rect, &RectStyle::filled(ui.fade(track), pill(rect)));

        // Hover/basılı katmanları seçenek başına (hap içinde).
        for i in 0..self.segs.len() {
            let r = self.indicator_rect(i);
            self.segs[i].interaction.draw_state_layer_under(ui, r, pill(r), c.on_surface, track, &[c.on_surface_variant]);
        }
        // Gösterge: yayla kayan, taşan hap.
        let ind = self.indicator.value(now);
        if ind.width > 0.0 {
            let fill = self.indicator_fill(theme);
            ui.painter.rect(ind, &RectStyle::filled(ui.fade(fill), pill(ind)));
        }
        if self.focus_visible {
            let ring = rect.expand(FOCUS_RING);
            let style = RectStyle {
                fill: Color::TRANSPARENT,
                radii: pill(ring),
                border: Some(nuxut_render::Border { width: FOCUS_RING, color: ui.fade(focus_ring(theme, &[track])) }),
            };
            ui.painter.rect(ring, &style);
        }

        let gap = theme.space(2);
        for i in 0..self.segs.len() {
            let r = self.seg_rect(i);
            // İçerik rengi göstergeyle örtüşme oranına göre geçer (kayarken yumuşak).
            let overlap = (ind.right().min(r.right()) - ind.x.max(r.x)).max(0.0) / r.width.max(1.0);
            let color = ui.fade(self.content_color(theme, i, overlap.clamp(0.0, 1.0)));
            let s = &mut self.segs[i];
            let icon_w = s.icon.as_ref().map_or(0.0, |ic| ic.size().width);
            let ls = s.label.measure_in(ui.painter);
            let total = icon_w + if icon_w > 0.0 && ls.width > 0.0 { gap } else { 0.0 } + ls.width;
            let mut x = r.center().x - total * 0.5;
            if let Some(ic) = &mut s.icon {
                ic.draw_centered(ui.painter, Point::new(x + icon_w * 0.5, r.center().y), color);
                x += icon_w + gap;
            }
            s.label.draw(ui.painter, Point::new(x, r.center().y - ls.height * 0.5), color);
        }

        let mut animating = !self.indicator.is_settled(now);
        for s in &mut self.segs {
            animating |= s.interaction.is_animating(now);
        }
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, rect.expand(FOCUS_RING * 2.0), animating);
    }
}

// Erişilebilirlik (ADR-0011).
impl SegmentedButton {
    /// `Role::RadioGroup` kabı; her seçenek `Role::RadioButton` (ad etiketinden; yalnız ikonlu
    /// seçenekte çağıran sonradan `tree.get(SegmentedButton::segment_id(..))` ile ad verir),
    /// seçili olan işaretli. `focused`: bileşen klavye odağındaysa seçili seçenek odak olur.
    /// Kabın adı (satır başlığı) çağıranın: `.label(..)`.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash, focused: bool) -> AccessNode<'t> {
        let id = tree.push(key, Role::RadioGroup).bounds(self.rect).enabled(self.enabled).id();
        let n = self.segs.len();
        tree.within(id, |tree| {
            for (i, s) in self.segs.iter().enumerate() {
                let node = tree
                    .push(i, Role::RadioButton)
                    .bounds(self.seg_rect(i))
                    .enabled(self.enabled && s.enabled)
                    .toggled(i == self.selected)
                    .position(i + 1, n)
                    .clickable()
                    .focused_if(focused && i == self.selected);
                if !s.label.text().is_empty() {
                    node.label(s.label.text());
                }
            }
        });
        tree.get(id).expect("az önce eklendi")
    }

    /// `access` kabı `container` altındaki `index`. seçeneğin düğüm kimliği.
    pub fn segment_id(container: NodeId, index: usize) -> NodeId {
        child_id(container, index)
    }

    /// AT isteği seçeneklerden birine `Click` ise onu seçer (`event = Some(indis)`, tıklamayla
    /// aynı). `container`: `access`'in döndürdüğü kabın kimliği.
    pub fn access_action(&mut self, container: NodeId, request: AccessRequest, now: Instant) -> Response<usize> {
        if request.action != AccessAction::Click || !self.enabled {
            return Response::none();
        }
        let Some(i) = (0..self.segs.len()).find(|&i| child_id(container, i) == request.node) else {
            return Response::none();
        };
        if self.segs[i].enabled && self.set_selected(i, now) { Response::event(i) } else { Response::none() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_names::IconName;
    use crate::input::test_util::*;

    fn seg() -> (SegmentedButton, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = SegmentedButton::new(
            &theme,
            vec![Segment::new("Gün"), Segment::new("Hafta").with_icon(Icon::named(IconName::ViewWeek)), Segment::new("Ay")],
            0,
        );
        s.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 40.0));
        (s, theme, t0)
    }

    #[test]
    fn click_selects_and_indicator_springs() {
        let (mut s, _theme, t0) = seg();
        let _ = s.pointer(&press(150.0, 20.0), t0);
        assert_eq!(s.pointer(&release(150.0, 20.0), t0).event, Some(1));
        assert_eq!(s.indicator.value(t0).x, s.indicator_rect(0).x, "gösterge zıplamaz, kayar");
        assert_eq!(s.indicator.target(), s.indicator_rect(1));
        // Aynı seçeneğe tıklama olay üretmez.
        let _ = s.pointer(&press(150.0, 20.0), t0);
        assert!(s.pointer(&release(150.0, 20.0), t0).event.is_none());
    }

    #[test]
    fn keys_and_limits() {
        let (mut s, theme, t0) = seg();
        assert_eq!(s.key(&key(Key::End), t0).event, Some(2));
        assert!(s.key(&key(Key::Right), t0).event.is_none());
        assert_eq!(s.key(&key(Key::Left), t0).event, Some(1));
        let many = (0..8).map(|i| Segment::new(i.to_string())).collect();
        assert_eq!(SegmentedButton::new(&theme, many, 9).len(), MAX_SEGMENTS);
    }

    #[test]
    fn disabled_segment_is_not_clickable() {
        let (mut s, _theme, t0) = seg();
        s.set_segment_enabled(1, false, t0);
        assert!(!s.is_segment_enabled(1));
        assert!(!s.is_segment_enabled(9), "aralık dışı");
        s.set_segment_enabled(9, false, t0); // panik yok
        let _ = s.pointer(&press(150.0, 20.0), t0);
        let r = s.pointer(&release(150.0, 20.0), t0);
        assert!(r.event.is_none());
        assert_eq!(s.selected(), 0);
        assert_eq!(s.cursor(Point::new(150.0, 20.0)), Some(CursorShape::Default));
        assert_eq!(s.cursor(Point::new(250.0, 20.0)), Some(CursorShape::Pointer));
        // Yeniden etkin: tıklanır.
        s.set_segment_enabled(1, true, t0);
        let _ = s.pointer(&press(150.0, 20.0), t0);
        assert_eq!(s.pointer(&release(150.0, 20.0), t0).event, Some(1));
    }

    #[test]
    fn keyboard_skips_disabled_segments() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let segs = vec![
            Segment::new("A").with_disabled(true),
            Segment::new("B"),
            Segment::new("C").with_disabled(true),
            Segment::new("D"),
            Segment::new("E").with_disabled(true),
        ];
        let mut s = SegmentedButton::new(&theme, segs, 1);
        s.set_rect(&theme, Rect::new(0.0, 0.0, 500.0, 40.0));
        assert!(!s.is_segment_enabled(0));
        assert_eq!(s.key(&key(Key::Right), t0).event, Some(3), "C atlanır");
        assert!(s.key(&key(Key::Right), t0).event.is_none(), "sağda etkin yok: yerinde");
        assert_eq!(s.selected(), 3);
        assert_eq!(s.key(&key(Key::Home), t0).event, Some(1), "Home ilk etkin");
        assert!(s.key(&key(Key::Left), t0).event.is_none(), "solda etkin yok");
        assert_eq!(s.key(&key(Key::End), t0).event, Some(3), "End son etkin");
    }

    #[test]
    fn disabling_selected_keeps_selection() {
        let (mut s, theme, t0) = seg();
        s.set_segment_enabled(0, false, t0);
        assert_eq!(s.selected(), 0, "seçim değişmez, yalnız etkileşim kapanır");
        // Klavye yine çalışır, devre dışı seçiliden uzaklaşabilir ama geri dönemez.
        assert_eq!(s.key(&key(Key::Right), t0).event, Some(1));
        assert!(s.key(&key(Key::Home), t0).event.is_none());
        assert_eq!(s.selected(), 1);
        // Dış durum devre dışı seçeneği seçebilir (uygulama kararı).
        assert!(s.set_selected(0, t0));
        // Kurucuda devre dışı seçili de korunur.
        let s2 = SegmentedButton::new(&theme, vec![Segment::new("a").with_disabled(true), Segment::new("b")], 0);
        assert_eq!(s2.selected(), 0);
    }

    #[test]
    fn disabled_segment_draws_dim() {
        use nuxut_render::Recorder;
        for (mode, contrast) in [("dark", "standard"), ("light", "standard"), ("dark", "high"), ("light", "high")] {
            let (theme, _) = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"{contrast}\"\n")).unwrap();
            let c = &theme.colors;
            let segs = vec![Segment::new("A"), Segment::new("B").with_disabled(true), Segment::new("C")];
            let mut s = SegmentedButton::new(&theme, segs, 0);
            s.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 40.0));
            // Normal seçili gösterge; devre dışı seçenek soluk, etkin seçenekler değişmez.
            assert_eq!(s.indicator_fill(&theme), c.primary);
            assert_eq!(s.content_color(&theme, 0, 1.0).to_hex(), c.on_primary.to_hex());
            assert_eq!(s.content_color(&theme, 2, 0.0), c.on_surface_variant);
            let dim = disabled_content(&theme, c.on_surface, &[c.surface_bright]);
            assert_eq!(s.content_color(&theme, 1, 0.0), dim, "{mode}/{contrast}");
            assert_ne!(dim, c.on_surface_variant);
            // Seçili devre dışı: gösterge primary değil devre dışı kap; içerik kap üstünde soluk.
            assert!(s.set_selected(1, Instant::now()));
            assert_eq!(s.indicator_fill(&theme), disabled_container(&theme));
            let on_cap = disabled_content(&theme, c.on_surface, &[c.surface_bright, disabled_container(&theme)]);
            assert_eq!(s.content_color(&theme, 1, 1.0), on_cap);
            assert_eq!(s.content_color(&theme, 0, 0.0), c.on_surface_variant);
            // Çizim boşta kare istemez, panik yok.
            s.indicator.jump_to(s.indicator_rect(1));
            let mut text = TextSystem::new();
            let mut rec = Recorder::new();
            rec.record(&mut text, (320, 60), 1.0, |painter| {
                let mut ui = Ui::new(painter, &theme, Instant::now());
                s.draw(&mut ui);
                assert!(!ui.wants_frame(), "boşta kare yok");
            });
            assert!(!rec.is_empty());
        }
    }

    #[test]
    fn long_labels_are_clipped_to_segment() {
        let (mut s, theme, _) = seg();
        let mut text = TextSystem::new();
        s.segs[0].label.set_text(&"Çok uzun bir seçenek etiketi ".repeat(4));
        let w = s.segs[0].label.measure(&mut text).width;
        assert!(w <= 100.0 - SegmentedButton::seg_padding(&theme) * 2.0 + 0.5, "{w}");
    }
}
