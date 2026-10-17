//! Yol çubuğu (dosya seçici, dosya yöneticisi): klasör yolunun her parçası bir düğme, aralarında
//! `ChevronRight`. Son parça geçerli klasördür (vurgulu). Sığmayınca baştan katlanır: kalan
//! parçaların önüne `…` düğmesi gelir; ona basmak gizlenen en yakın üst klasöre (görünen ilk
//! parçanın bir üstü) gider. Son parça tek başına sığmazsa etiketi `…` ile kırpılır.
//!
//! - **Olay:** tıklanan / Enter'lanan parçanın indisi [`PathBarEvent::Navigate`]. Yolun
//!   kendisini (gerçek dizin yolu, görünen ad eşlemesi) uygulama tutar.
//! - **Düzenleme kipi:** uygulama kısayolu (Ctrl+L) yakalayıp [`PathBar::begin_edit`] ile
//!   metin girişine geçer; girişin olayları [`PathBarEvent::Edit`] ile döner. Esc kipten kendisi
//!   çıkar (`Edit(Cancel)` yine iletilir); Enter'da uygulama [`PathBar::edit_text`]'i okur,
//!   gidebildiyse [`PathBar::end_edit`] der. Pano/IME için [`PathBar::text_input_mut`].
//! - **Klavye (düğme kipi):** Sol/Sağ, Home/End odağı gezdirir; Enter/Space etkinleştirir.
//! - **Ölçüm:** genişlikler metne bağlı; katlama [`PathBar::layout`] (ya da `draw`) içinde,
//!   yalnız parçalar/dikdörtgen/tema değişince hesaplanır. İlk çizimden önce işaretçi olayı
//!   yoktur (ya da önce `layout` çağrılır).
//! - Parça adları kullanıcı içeriğidir: `Debug` yalnız uzunluk yazar (değişmez 9).

use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_render::{Border, RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::style::{FOCUS_RING, clip_text, focus_ring, pill};
use crate::text_input::{TextInput, TextInputEvent};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Parça adının en uzun hâli (bayt); dosya adı sınırı (NAME_MAX) kadar.
pub const MAX_PATH_SEGMENT: usize = 255;
/// En çok parça; daha derin yollarda en baştakiler atılmaz, sondan bu kadarı tutulur
/// (indisler yine verilen listenin indisleridir).
pub const MAX_PATH_SEGMENTS: usize = 256;

/// Bir yol parçası: görünen ad ve isteğe bağlı ikon (ev, sürücü).
#[derive(Clone, PartialEq, Default)]
pub struct PathSegment {
    pub label: String,
    pub icon: Option<Icon>,
}

impl PathSegment {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), icon: None }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

impl std::fmt::Debug for PathSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathSegment")
            .field("label", &crate::redact::Len(&self.label))
            .field("icon", &self.icon)
            .finish()
    }
}

/// [`PathBar`] olayı.
#[derive(Debug, Clone, PartialEq)]
pub enum PathBarEvent {
    /// Bu indisteki parçaya git (`…` için gizlenen en yakın üst klasör).
    Navigate(usize),
    /// Düzenleme kipindeki metin girişinin olayı (`Cancel`'da kip zaten kapandı).
    Edit(TextInputEvent),
}

#[derive(Debug, Clone)]
struct Seg {
    icon: Option<IconView>,
    label: Label,
    interaction: Interaction,
    /// Doğal genişlik (dolgu dahil); `layout` hesaplar.
    natural: f32,
    rect: Rect,
}

#[derive(Debug)]
pub struct PathBar {
    segs: Vec<Seg>,
    /// Listenin kesilen başı (MAX_PATH_SEGMENTS aşılınca): olay indisine eklenir.
    offset: usize,
    /// Görünen ilk parça; `> 0` ise önünde `…`.
    first: usize,
    ellipsis: Label,
    ellipsis_hit: Interaction,
    ellipsis_rect: Rect,
    separator: IconView,
    /// Ayraç yuvasının genişliği (`layout`).
    sep_w: f32,
    /// Odaktaki öğe: 0 = `…` (görünürse), sonra görünen parçalar. `items()` sırası.
    focus: usize,
    focus_visible: bool,
    edit: Option<Box<TextInput>>,
    rect: Rect,
    layout_dirty: bool,
    damage: DamageState,
}

impl PathBar {
    pub fn new(theme: &Theme, segments: Vec<PathSegment>) -> Self {
        let mut bar = Self {
            segs: Vec::new(),
            offset: 0,
            first: 0,
            ellipsis: Label::new("…", Self::label_style(theme)),
            ellipsis_hit: Interaction::new(theme),
            ellipsis_rect: Rect::ZERO,
            separator: IconView::with_role(theme, Icon::named(IconName::ChevronRight), TextRole::IconSmall),
            sep_w: 0.0,
            focus: 0,
            focus_visible: false,
            edit: None,
            rect: Rect::ZERO,
            layout_dirty: true,
            damage: DamageState::new(),
        };
        bar.set_segments(theme, segments);
        bar
    }

    fn label_style(theme: &Theme) -> nuxut_render::TextStyle {
        text_style(theme, TextRole::Label).with_max_lines(1)
    }

    /// Parçaları değiştirir (klasör değişti). Odak son parçaya döner.
    pub fn set_segments(&mut self, theme: &Theme, segments: Vec<PathSegment>) {
        self.offset = segments.len().saturating_sub(MAX_PATH_SEGMENTS);
        let style = Self::label_style(theme);
        self.segs = segments
            .into_iter()
            .skip(self.offset)
            .map(|s| Seg {
                icon: s.icon.map(|i| IconView::with_role(theme, i, TextRole::IconSmall)),
                label: Label::new(clip_text(&s.label, MAX_PATH_SEGMENT), style.clone()),
                interaction: Interaction::new(theme),
                natural: 0.0,
                rect: Rect::ZERO,
            })
            .collect();
        self.first = 0;
        self.focus = usize::MAX;
        self.layout_dirty = true;
        self.damage.mark();
    }

    /// Parça sayısı (verilen listenin uzunluğu).
    pub fn len(&self) -> usize {
        self.offset + self.segs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    /// Görünen ilk parçanın indisi (`> 0`: baştan katlandı, `…` gösteriliyor). `layout`'tan sonra.
    pub fn first_visible(&self) -> usize {
        self.offset + self.first
    }

    /// Parçanın son yerleşimdeki dikdörtgeni (gizliyse `None`). Erişilebilirlik/test için.
    pub fn segment_rect(&self, index: usize) -> Option<Rect> {
        let i = index.checked_sub(self.offset)?;
        (i >= self.first && i < self.segs.len()).then(|| self.segs[i].rect)
    }

    /// Parçanın görünen adı.
    pub fn segment_label(&self, index: usize) -> Option<&str> {
        let i = index.checked_sub(self.offset)?;
        self.segs.get(i).map(|s| s.label.text())
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        let style = Self::label_style(theme);
        for s in &mut self.segs {
            s.label.set_style(style.clone());
            s.interaction.set_theme(theme, now);
            if let Some(i) = &mut s.icon {
                i.set_theme(theme, now);
            }
        }
        self.ellipsis.set_style(style);
        self.ellipsis_hit.set_theme(theme, now);
        self.separator.set_theme(theme, now);
        if let Some(e) = &mut self.edit {
            e.set_theme(theme, now);
        }
        self.layout_dirty = true;
        self.damage.mark();
    }

    /// Yükseklik `space(10)`; genişlik çağıranın (katlanır).
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(10))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            if let Some(e) = &mut self.edit {
                e.set_rect(rect);
            }
            self.layout_dirty = true;
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn pad(theme: &Theme) -> f32 {
        theme.space(3)
    }

    /// Parçaları ölçer ve katlar. Değişiklik yoksa iş yapmaz; `draw` de çağırır.
    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme) {
        if !self.layout_dirty {
            return;
        }
        self.layout_dirty = false;
        let pad = Self::pad(theme);
        let gap = theme.space(1);
        // Ayraç yuvası: ok + iki yanında boşluk (katman haplarıyla, odak halkasıyla çakışmaz).
        let sep = (self.separator.measure(text).width + gap * 2.0).ceil();
        self.sep_w = sep;
        for s in &mut self.segs {
            s.label.set_max_width(None);
            let icon = s.icon.as_mut().map_or(0.0, |i| i.measure(text).width);
            let label = s.label.measure(text).width;
            let inner = icon + if icon > 0.0 && label > 0.0 { gap } else { 0.0 } + label;
            s.natural = (inner + pad * 2.0).ceil();
        }
        let ellipsis = (self.ellipsis.measure(text).width + pad * 2.0).ceil();
        let n = self.segs.len();
        // En küçük `first`: sondan geriye toplanan genişlik sığana dek.
        let width = self.rect.width;
        let mut first = n.saturating_sub(1);
        let mut used = self.segs.last().map_or(0.0, |s| s.natural);
        while first > 0 {
            let candidate = used + sep + self.segs[first - 1].natural;
            let lead = if first - 1 > 0 { ellipsis + sep } else { 0.0 };
            if candidate + lead > width {
                break;
            }
            used = candidate;
            first -= 1;
        }
        self.first = first;

        let y = self.rect.y;
        let h = self.rect.height;
        let mut x = self.rect.x;
        if first > 0 {
            self.ellipsis_rect = Rect::new(x, y, ellipsis, h);
            x += ellipsis + sep;
        } else {
            self.ellipsis_rect = Rect::ZERO;
        }
        for (i, s) in self.segs.iter_mut().enumerate() {
            if i < first {
                s.rect = Rect::ZERO;
                continue;
            }
            let mut w = s.natural;
            if i + 1 == n && x + w > self.rect.right() {
                // Son parça tek başına sığmıyor: etiket kırpılır.
                w = (self.rect.right() - x).max(0.0);
                let icon = s.icon.as_ref().map_or(0.0, |ic| ic.size().width + gap);
                s.label.set_max_width(Some((w - pad * 2.0 - icon).max(0.0)));
            }
            s.rect = Rect::new(x, y, w, h);
            x += w + sep;
        }
        let items = self.item_count();
        if self.focus >= items {
            self.focus = items.saturating_sub(1);
        }
    }

    /// Görünen odaklanabilir öğe sayısı (`…` + görünen parçalar).
    fn item_count(&self) -> usize {
        usize::from(self.first > 0) + self.segs.len() - self.first.min(self.segs.len())
    }

    /// Öğe → yerel parça indisi (`None` = `…`).
    fn item_seg(&self, item: usize) -> Option<usize> {
        if self.first > 0 {
            item.checked_sub(1).map(|k| self.first + k)
        } else {
            Some(item)
        }
    }

    /// Öğenin hedefi (dış indis).
    fn item_target(&self, item: usize) -> Option<usize> {
        match self.item_seg(item) {
            Some(i) if i < self.segs.len() => Some(self.offset + i),
            Some(_) => None,
            None => Some(self.offset + self.first - 1),
        }
    }

    fn item_rect(&self, item: usize) -> Rect {
        match self.item_seg(item) {
            Some(i) => self.segs.get(i).map_or(Rect::ZERO, |s| s.rect),
            None => self.ellipsis_rect,
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) {
        if self.focus_visible != on {
            self.focus_visible = on;
            self.damage.mark();
        }
    }

    // ── Düzenleme kipi ──────────────────────────────────────────────────────────

    /// Metin girişine geçer (`text`: tam yol; tümü seçili değil, imleç sonda). Giriş odaklı.
    pub fn begin_edit(&mut self, theme: &Theme, text: &str, now: Instant) {
        let mut input = TextInput::new(theme).with_text(text);
        input.set_rect(self.rect);
        input.set_focused(true, now);
        self.edit = Some(Box::new(input));
        self.damage.mark();
    }

    /// Düğme kipine döner.
    pub fn end_edit(&mut self) {
        if self.edit.take().is_some() {
            self.damage.mark();
        }
    }

    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    /// Düzenleme kipindeki metin (kip kapalıysa `None`).
    pub fn edit_text(&self) -> Option<&str> {
        self.edit.as_deref().map(TextInput::text)
    }

    /// Pano yapıştırma ve IME için giriş.
    pub fn text_input_mut(&mut self) -> Option<&mut TextInput> {
        self.edit.as_deref_mut()
    }

    fn edit_response(&mut self, r: Response<TextInputEvent>) -> Response<PathBarEvent> {
        if r.event == Some(TextInputEvent::Cancel) {
            self.end_edit();
        }
        r.map(PathBarEvent::Edit)
    }

    // ── Girdi ───────────────────────────────────────────────────────────────────

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<PathBarEvent> {
        if let Some(e) = &mut self.edit {
            let r = e.pointer(text, theme, event, now);
            return self.edit_response(r);
        }
        let mut out = Response::none();
        let mut clicked = None;
        if self.first > 0 {
            let tap = self.ellipsis_hit.handle(self.ellipsis_rect, event, now);
            out.repaint |= tap.repaint;
            if tap.clicked {
                clicked = Some(0);
            }
        }
        let lead = usize::from(self.first > 0);
        for i in self.first..self.segs.len() {
            let r = self.segs[i].rect;
            let tap = self.segs[i].interaction.handle(r, event, now);
            out.repaint |= tap.repaint;
            if tap.clicked {
                clicked = Some(lead + i - self.first);
            }
        }
        if out.repaint {
            self.damage.mark();
        }
        if let Some(item) = clicked {
            self.focus = item;
            self.focus_visible = false;
            if let Some(target) = self.item_target(item) {
                return Response::event(PathBarEvent::Navigate(target));
            }
        }
        out
    }

    /// Odaktayken. Düzenleme kipinde giriş alır.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<PathBarEvent> {
        if let Some(e) = &mut self.edit {
            let r = e.key(event, now);
            return self.edit_response(r);
        }
        if event.state == KeyState::Released {
            return Response::none();
        }
        let n = self.item_count();
        if n == 0 {
            return Response::none();
        }
        let cur = self.focus.min(n - 1);
        let next = match event.key {
            Key::Left => cur.saturating_sub(1),
            Key::Right => (cur + 1).min(n - 1),
            Key::Home => 0,
            Key::End => n - 1,
            Key::Enter | Key::Space => {
                self.focus_visible = true;
                return match self.item_target(cur) {
                    Some(t) => Response::event(PathBarEvent::Navigate(t)),
                    None => Response::none(),
                };
            }
            _ => return Response::none(),
        };
        let changed = next != self.focus || !self.focus_visible;
        self.focus = next;
        self.focus_visible = true;
        if changed {
            self.damage.mark();
        }
        Response::none().with_repaint(changed)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        if let Some(e) = &self.edit {
            return e.cursor(p);
        }
        let on = self.ellipsis_rect.contains(p) || self.segs[self.first.min(self.segs.len())..].iter().any(|s| s.rect.contains(p));
        Some(if on { CursorShape::Pointer } else { CursorShape::Default })
    }

    // ── Çizim ───────────────────────────────────────────────────────────────────

    pub fn draw(&mut self, ui: &mut Ui) {
        let rect = self.rect;
        if let Some(e) = &mut self.edit {
            e.draw(ui);
            self.damage.report(ui, rect.expand(FOCUS_RING * 2.0), false);
            return;
        }
        let theme = ui.theme;
        let now = ui.now;
        self.layout(ui.text(), theme);
        let c = &theme.colors;
        let gap = theme.space(1);
        // Katman alanı: düğme yüksekliğinin içinde, kenarlardan eşit boşluk.
        let inset = (rect.height * 0.1).round();
        let shrink = |r: Rect| Rect::new(r.x, r.y + inset, r.width, r.height - inset * 2.0);
        let mut animating = false;

        if self.first > 0 {
            let r = shrink(self.ellipsis_rect);
            self.ellipsis_hit.draw_state_layer(ui, r, pill(r), c.on_surface_variant, Color::TRANSPARENT);
            let s = self.ellipsis.measure_in(ui.painter);
            let color = ui.fade(c.on_surface_variant);
            self.ellipsis.draw(ui.painter, Point::new(r.center().x - s.width * 0.5, r.center().y - s.height * 0.5), color);
            animating |= self.ellipsis_hit.is_animating(now);
            self.draw_separator(ui, self.ellipsis_rect.right());
        }
        let n = self.segs.len();
        for i in self.first..n {
            let current = i + 1 == n;
            let content = if current { c.on_surface } else { c.on_surface_variant };
            let r = shrink(self.segs[i].rect);
            if current {
                // Geçerli klasör: hafif tonal kap (seçili gezinme öğesiyle aynı dil).
                ui.painter.rect(r, &RectStyle::filled(ui.fade(c.surface_bright), pill(r)));
            }
            let under = if current { c.surface_bright } else { Color::TRANSPARENT };
            let s = &mut self.segs[i];
            s.interaction.draw_state_layer(ui, r, pill(r), content, under);
            let color = ui.fade(content);
            let icon_w = s.icon.as_ref().map_or(0.0, |ic| ic.size().width);
            let ls = s.label.measure_in(ui.painter);
            let total = icon_w + if icon_w > 0.0 && ls.width > 0.0 { gap } else { 0.0 } + ls.width;
            let mut x = r.center().x - total * 0.5;
            if let Some(ic) = &mut s.icon {
                ic.draw_centered(ui.painter, Point::new(x + icon_w * 0.5, r.center().y), color);
                x += icon_w + gap;
            }
            s.label.draw(ui.painter, Point::new(x, r.center().y - ls.height * 0.5), color);
            animating |= s.interaction.is_animating(now);
            if !current {
                self.draw_separator(ui, self.segs[i].rect.right());
            }
        }
        if self.focus_visible && self.item_count() > 0 {
            let r = shrink(self.item_rect(self.focus.min(self.item_count() - 1))).expand(FOCUS_RING);
            let style = RectStyle {
                fill: Color::TRANSPARENT,
                radii: pill(r),
                border: Some(Border { width: FOCUS_RING, color: ui.fade(focus_ring(theme, &[c.surface])) }),
            };
            ui.painter.rect(r, &style);
        }
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, rect.expand(FOCUS_RING * 2.0), animating);
    }

    fn draw_separator(&mut self, ui: &mut Ui, left: f32) {
        let w = self.sep_w;
        let color = ui.fade(ui.theme.colors.on_surface_variant);
        let center = Point::new(left + w * 0.5, self.rect.center().y);
        self.separator.draw_centered(ui.painter, center, color);
    }
}

// Erişilebilirlik (ADR-0011).
impl PathBar {
    /// `Role::Navigation` kabı (adı çağıranın: `.label("Konum")`); düğme kipinde çocukları görünen
    /// parçalar `Role::Button` (ad parça adı; geçerli klasör `selected`) ve görünürse `…` düğmesi
    /// (ad `more_label`, ör. "Üst klasörler"). Düzenleme kipinde tek çocuk metin girişi.
    /// `focused`: çubuk klavye odağındaysa odaktaki öğe odak olur. Anahtarlar: parça indisi,
    /// `…` için `"more"`, giriş için `"edit"`.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl std::hash::Hash, focused: bool, more_label: &str) -> AccessNode<'t> {
        let id = tree.push(key, Role::Navigation).bounds(self.rect).id();
        tree.within(id, |tree| {
            if let Some(e) = &self.edit {
                let node = e.access(tree, "edit");
                if focused {
                    node.focused();
                }
                return;
            }
            let focus = self.focus.min(self.item_count().saturating_sub(1));
            if self.first > 0 {
                tree.push("more", Role::Button).label(more_label).bounds(self.ellipsis_rect).clickable().focused_if(focused && focus == 0);
            }
            let lead = usize::from(self.first > 0);
            let n = self.segs.len();
            for i in self.first..n {
                tree.push(self.offset + i, Role::Button)
                    .label(self.segs[i].label.text())
                    .bounds(self.segs[i].rect)
                    .selected(i + 1 == n)
                    .clickable()
                    .focused_if(focused && focus == lead + i - self.first);
            }
        });
        tree.get(id).expect("az önce eklendi")
    }

    /// AT isteği: parçada ya da `…`'da `Click` → `Navigate` (tıklamayla aynı). Düzenleme kipinde
    /// metin girişinin isteği girişe gider (`Edit` olayı yok; `true` dönerse yeniden çiz).
    pub fn access_action(&mut self, container: NodeId, request: AccessRequest) -> Response<PathBarEvent> {
        if let Some(e) = &mut self.edit {
            let changed = e.access_action(child_id(container, "edit"), request);
            return Response::none().with_repaint(changed);
        }
        if request.action != AccessAction::Click {
            return Response::none();
        }
        if self.first > 0 && request.node == child_id(container, "more") {
            return Response::event(PathBarEvent::Navigate(self.offset + self.first - 1));
        }
        (self.first..self.segs.len())
            .find(|&i| child_id(container, self.offset + i) == request.node)
            .map_or(Response::none(), |i| Response::event(PathBarEvent::Navigate(self.offset + i)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;
    use crate::input::test_util::*;

    fn segs(names: &[&str]) -> Vec<PathSegment> {
        names.iter().map(|n| PathSegment::new(*n)).collect()
    }

    fn bar(width: f32, names: &[&str]) -> (PathBar, Theme, TextSystem, Instant) {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut b = PathBar::new(&theme, segs(names));
        b.set_rect(Rect::new(0.0, 0.0, width, 40.0));
        b.layout(&mut text, &theme);
        (b, theme, text, Instant::now())
    }

    fn click(b: &mut PathBar, text: &mut TextSystem, theme: &Theme, p: Point, now: Instant) -> Option<PathBarEvent> {
        let _ = b.pointer(text, theme, &press(p.x, p.y), now);
        b.pointer(text, theme, &release(p.x, p.y), now).event
    }

    #[test]
    fn wide_bar_shows_all_and_click_returns_index() {
        let (mut b, theme, mut text, t0) = bar(2000.0, &["Ev", "Belgeler", "Faturalar"]);
        assert_eq!(b.first_visible(), 0);
        let r = b.segment_rect(1).unwrap();
        assert!(r.width > 0.0);
        assert_eq!(click(&mut b, &mut text, &theme, r.center(), t0), Some(PathBarEvent::Navigate(1)));
        // Sıralı, örtüşmeyen.
        assert!(b.segment_rect(0).unwrap().right() <= r.x);
    }

    #[test]
    fn narrow_bar_collapses_from_start() {
        let names = ["Ev", "Belgeler", "Projeler", "Nuxut", "framework", "crates", "nuxut-ui"];
        let (mut b, theme, mut text, t0) = bar(260.0, &names);
        let first = b.first_visible();
        assert!(first > 0, "katlanmalı");
        assert!(b.segment_rect(first - 1).is_none());
        assert!(b.segment_rect(6).unwrap().right() <= 260.5);
        // `…` gizlenen en yakın üst klasöre gider.
        let e = b.ellipsis_rect.center();
        assert_eq!(click(&mut b, &mut text, &theme, e, t0), Some(PathBarEvent::Navigate(first - 1)));
        // Genişleyince katlama açılır.
        b.set_rect(Rect::new(0.0, 0.0, 4000.0, 40.0));
        b.layout(&mut text, &theme);
        assert_eq!(b.first_visible(), 0);
    }

    #[test]
    fn last_segment_alone_is_clipped() {
        let long = "çok uzun bir klasör adı ".repeat(8);
        let (b, _, _, _) = bar(120.0, &["Ev", &long]);
        assert_eq!(b.first_visible(), 1);
        let r = b.segment_rect(1).unwrap();
        assert!(r.right() <= 120.0 + 0.5, "{r:?}");
    }

    #[test]
    fn keyboard_moves_and_activates() {
        let (mut b, _, _, t0) = bar(2000.0, &["a", "b", "c"]);
        // Başlangıçta odak son parçada (geçerli klasör).
        assert_eq!(b.key(&key(Key::Enter), t0).event, Some(PathBarEvent::Navigate(2)));
        let _ = b.key(&key(Key::Home), t0);
        assert_eq!(b.key(&key(Key::Space), t0).event, Some(PathBarEvent::Navigate(0)));
        let _ = b.key(&key(Key::Right), t0);
        assert_eq!(b.key(&key(Key::Enter), t0).event, Some(PathBarEvent::Navigate(1)));
        assert!(b.focus_visible);
    }

    #[test]
    fn edit_mode_roundtrip() {
        let (mut b, theme, _, t0) = bar(400.0, &["Ev", "Belgeler"]);
        b.begin_edit(&theme, "/home/x/Belgeler", t0);
        assert!(b.is_editing());
        let _ = b.key(&typed('/'), t0);
        assert_eq!(b.edit_text(), Some("/home/x/Belgeler/"));
        assert_eq!(b.key(&key(Key::Enter), t0).event, Some(PathBarEvent::Edit(TextInputEvent::Submit)));
        assert!(b.is_editing(), "Enter'da uygulama karar verir");
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        assert_eq!(b.key(&key_mod(Key::Char('v'), ctrl), t0).event, Some(PathBarEvent::Edit(TextInputEvent::PasteRequested)));
        assert_eq!(b.key(&key(Key::Escape), t0).event, Some(PathBarEvent::Edit(TextInputEvent::Cancel)));
        assert!(!b.is_editing(), "Esc kipten çıkar");
    }

    #[test]
    fn many_segments_keep_tail_indices() {
        let names: Vec<String> = (0..MAX_PATH_SEGMENTS + 10).map(|i| i.to_string()).collect();
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut b = PathBar::new(&theme, names.iter().map(PathSegment::new).collect());
        b.set_rect(Rect::new(0.0, 0.0, 300.0, 40.0));
        b.layout(&mut text, &theme);
        assert_eq!(b.len(), MAX_PATH_SEGMENTS + 10);
        let last = b.len() - 1;
        assert_eq!(b.segment_label(last), Some(names[last].as_str()));
        assert_eq!(b.key(&key(Key::End), Instant::now()).event, None);
        assert_eq!(b.key(&key(Key::Enter), Instant::now()).event, Some(PathBarEvent::Navigate(last)));
    }

    #[test]
    fn debug_is_redacted() {
        let theme = Theme::default();
        let b = PathBar::new(&theme, segs(&[crate::redact::SECRET]));
        crate::redact::assert_redacted(&b);
        crate::redact::assert_redacted(&PathSegment::new(crate::redact::SECRET));
    }

    #[test]
    fn draw_idle_requests_no_frame() {
        use nuxut_render::Recorder;
        let (mut b, theme, mut text, _) = bar(240.0, &["Ev", "Belgeler", "Projeler", "Nuxut"]);
        let mut rec = Recorder::new();
        rec.record(&mut text, (260, 60), 1.0, |painter| {
            let mut ui = Ui::new(painter, &theme, Instant::now());
            b.draw(&mut ui);
            assert!(!ui.wants_frame(), "boşta kare yok");
        });
        assert!(!rec.is_empty());
    }
}
