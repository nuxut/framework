//! Hızlı ayar kutucuğu ([`Tile`]) ve ızgarası ([`TileGrid`]): ikon + etiket + alt metin;
//! açıkken `primary` dolgu / `on_primary` içerik, kapalıyken tonal. Sağda "›" alanı alt sayfaya
//! geçişi ister ([`TileEvent::Expand`]); geçişin kendisi [`PageStack`](crate::PageStack).
//! Basınca köşeler sıkışır (düğmelerle aynı dil), açık/kapalı rengi `effects` ile geçer.

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::RectStyle;
use nuxut_theme::Theme;

use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, KeyEvent, PointerEvent};
use crate::interaction::Interaction;
use crate::label::Label;
use crate::pressable::Pressable;
use crate::style::{clip_text, disabled_container, disabled_content};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// Etiket ve alt metnin en uzun hâli (bayt).
pub const MAX_TILE_TEXT: usize = 128;

#[derive(Clone, PartialEq)]
pub struct TileItem {
    pub icon: Icon,
    pub label: String,
    pub subtitle: Option<String>,
    pub on: bool,
    /// Sağda "›" alanı (alt sayfa var).
    pub expandable: bool,
}

/// Alt metin bağlı ağ/cihaz adı olabilir: `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for TileItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TileItem")
            .field("icon", &self.icon)
            .field("label", &crate::redact::Len(&self.label))
            .field("subtitle", &crate::redact::OptLen(self.subtitle.as_deref()))
            .field("on", &self.on)
            .field("expandable", &self.expandable)
            .finish()
    }
}

impl TileItem {
    pub fn new(icon: Icon, label: impl Into<String>) -> Self {
        Self { icon, label: label.into(), subtitle: None, on: false, expandable: false }
    }

    pub fn subtitle(mut self, s: impl Into<String>) -> Self {
        self.subtitle = Some(s.into());
        self
    }

    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }

    pub fn expandable(mut self, on: bool) -> Self {
        self.expandable = on;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileEvent {
    /// Gövde tıklaması: yeni durum.
    Toggled(bool),
    /// "›" alanı: alt sayfa istendi.
    Expand,
}

#[derive(Debug, Clone)]
pub struct Tile {
    core: Pressable,
    expand: Interaction,
    icon: IconView,
    chevron: IconView,
    label: Label,
    subtitle: Option<Label>,
    on: bool,
    expandable: bool,
    tint: Animated<f32>,
}

impl Tile {
    pub fn new(theme: &Theme, item: TileItem) -> Self {
        let on = item.on;
        Self {
            core: Pressable::new(theme),
            expand: Interaction::new(theme),
            icon: IconView::new(theme, item.icon).filled(on),
            chevron: IconView::with_role(theme, Icon::named(IconName::ChevronRight), TextRole::IconSmall),
            label: Label::new(clip_text(&item.label, MAX_TILE_TEXT), text_style(theme, TextRole::Label).with_max_lines(1)),
            subtitle: item
                .subtitle
                .map(|s| Label::new(clip_text(&s, MAX_TILE_TEXT), text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1))),
            on,
            expandable: item.expandable,
            tint: Animated::new(if on { 1.0 } else { 0.0 }, theme.motion.effects),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Dış durum (ör. Wi-Fi gerçekten açıldı).
    pub fn set_on(&mut self, on: bool, now: Instant) {
        if self.on != on {
            self.on = on;
            self.tint.set_target(if on { 1.0 } else { 0.0 }, now);
            self.icon.set_filled(on, now);
            self.core.damage.mark();
        }
    }

    pub fn set_subtitle(&mut self, theme: &Theme, s: Option<&str>) {
        match (s, &mut self.subtitle) {
            (Some(t), Some(l)) => {
                l.set_text(clip_text(t, MAX_TILE_TEXT));
            }
            (Some(t), None) => self.subtitle = Some(Label::new(clip_text(t, MAX_TILE_TEXT), text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1))),
            (None, _) => self.subtitle = None,
        }
        self.core.damage.mark();
    }

    pub fn set_icon(&mut self, icon: Icon) {
        if self.icon.set_icon(icon) {
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
        self.expand.set_theme(theme, now);
        self.icon.set_theme(theme, now);
        self.chevron.set_theme(theme, now);
        self.label.set_style(text_style(theme, TextRole::Label).with_max_lines(1));
        if let Some(s) = &mut self.subtitle {
            s.set_style(text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1));
        }
        self.tint.set_motion(theme.motion.effects, now);
    }

    /// Yükseklik `space(16)`.
    pub fn height(theme: &Theme) -> f32 {
        theme.space(16)
    }

    fn expand_rect(&self, theme: &Theme) -> Option<Rect> {
        self.expandable.then(|| {
            let r = self.core.rect;
            let w = theme.space(12);
            Rect::new(r.right() - w, r.y, w, r.height)
        })
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.core.set_rect(rect) {
            let ex = if self.expandable { theme.space(12) } else { theme.space(4) };
            let text_w = rect.width - theme.space(4) - text_style(theme, TextRole::Icon).size - theme.space(3) - ex;
            self.label.set_max_width(Some(text_w.max(0.0)));
            if let Some(s) = &mut self.subtitle {
                s.set_max_width(Some(text_w.max(0.0)));
            }
        }
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<TileEvent> {
        if let Some(er) = self.expand_rect(theme) {
            if !self.core.enabled {
                return Response::none();
            }
            let tap = self.expand.handle(er, event, now);
            if tap.repaint {
                self.core.damage.mark();
            }
            if tap.clicked {
                return Response::event(TileEvent::Expand);
            }
            if er.contains(event.position) {
                let leave = event.with_kind(event.position, crate::input::PointerKind::Leave);
                return self.core.pointer(&leave, now).discard().with_repaint(tap.repaint);
            }
        }
        let r = self.core.pointer(event, now);
        self.after(r, now)
    }

    /// Enter/Space çevirir; Sağ ok alt sayfayı ister.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<TileEvent> {
        if self.expandable && self.core.enabled && event.state != crate::input::KeyState::Released && event.key == crate::input::Key::Right {
            return Response::event(TileEvent::Expand);
        }
        let r = self.core.key(event, now);
        self.after(r, now)
    }

    fn after(&mut self, r: Response, now: Instant) -> Response<TileEvent> {
        if r.event.is_none() {
            return r.discard();
        }
        let on = !self.on;
        self.set_on(on, now);
        Response::event(TileEvent::Toggled(on))
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.core.rect;
        let t = self.tint.value(now).clamp(0.0, 1.0);
        let enabled = self.core.enabled;
        // Sıkışma: büyük yarıçaptan küçüğe.
        let rest = theme.shape.radius_large.min(rect.height * 0.5);
        let r = rest + (theme.shape.radius_small - rest) * self.core.squish(now);
        let radii = Corners::uniform(r.max(0.0));
        let (fill, content, sub) = tile_colors(theme, enabled, t);
        ui.painter.rect(rect, &RectStyle::filled(ui.fade(fill), radii));
        let layer = c.on_surface.lerp(&c.on_primary, t);
        self.core.interaction.draw_state_layer_under(ui, rect, radii, layer, fill, &[content, sub]);
        if let Some(er) = self.expand_rect(theme) {
            // "›" alanı: ince ayraç + kendi hover katmanı.
            let sep = Rect::new(er.x, er.y + theme.space(3), crate::style::HAIRLINE, er.height - theme.space(6));
            let line = crate::style::hairline(theme, content, crate::style::DIVIDER_TONE * 2.0, &[fill]);
            ui.painter.rect(sep, &RectStyle::filled(ui.fade(line), Corners::ZERO));
            let er_r = Corners::new(0.0, radii.top_right, radii.bottom_right, 0.0);
            self.expand.draw_state_layer_under(ui, er, er_r, layer, fill, &[content]);
            self.chevron.draw_centered(ui.painter, er.center(), ui.fade(content));
        }
        self.core.draw_focus(ui, rect, radii);
        let pad = theme.space(4);
        let iw = self.icon.size().width;
        let mid = rect.center().y;
        self.icon.draw_centered(ui.painter, Point::new(rect.x + pad + iw * 0.5, mid), ui.fade(content));
        let tx = rect.x + pad + iw + theme.space(3);
        let ls = self.label.measure_in(ui.painter);
        match &mut self.subtitle {
            Some(s) => {
                let ss = s.measure_in(ui.painter);
                let top = mid - (ls.height + ss.height) * 0.5;
                self.label.draw(ui.painter, Point::new(tx, top), ui.fade(content));
                s.draw(ui.painter, Point::new(tx, top + ls.height), ui.fade(sub));
            }
            None => self.label.draw(ui.painter, Point::new(tx, mid - ls.height * 0.5), ui.fade(content)),
        }
        let animating = !self.tint.is_settled(now) | self.expand.is_animating(now) | self.icon.animate(now);
        if animating {
            ui.request_frame();
            self.core.damage.mark();
        }
        self.core.finish(ui, self.core.extent());
    }
}

/// Kutucuk ızgarası: genişliğe göre 2–4 sütun.
#[derive(Debug, Clone)]
pub struct TileGrid {
    tiles: Vec<Tile>,
    columns: usize,
    rect: Rect,
}

impl TileGrid {
    pub fn new(theme: &Theme, items: Vec<TileItem>) -> Self {
        Self { tiles: items.into_iter().map(|i| Tile::new(theme, i)).collect(), columns: 2, rect: Rect::ZERO }
    }

    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    pub fn tiles_mut(&mut self) -> &mut [Tile] {
        &mut self.tiles
    }

    pub fn columns(&self) -> usize {
        self.columns
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for t in &mut self.tiles {
            t.set_theme(theme, now);
        }
    }

    /// Sütun sayısı: bir kutucuk en az `space(40)` geniş; 2..=4.
    pub fn columns_for(theme: &Theme, width: f32) -> usize {
        ((width + theme.space(2)) / (theme.space(40) + theme.space(2))).floor().clamp(2.0, 4.0) as usize
    }

    /// Yerleştirir; ızgaranın yüksekliğini döner.
    pub fn layout(&mut self, theme: &Theme, origin: Point, width: f32) -> f32 {
        let gap = theme.space(2);
        self.columns = Self::columns_for(theme, width);
        let n = self.columns as f32;
        let w = ((width - gap * (n - 1.0)) / n).floor();
        let h = Tile::height(theme);
        for (i, t) in self.tiles.iter_mut().enumerate() {
            let (row, col) = (i / self.columns, i % self.columns);
            t.set_rect(theme, Rect::new(origin.x + col as f32 * (w + gap), origin.y + row as f32 * (h + gap), w, h));
        }
        let rows = self.tiles.len().div_ceil(self.columns) as f32;
        let total = (rows * (h + gap) - gap).max(0.0);
        self.rect = Rect::new(origin.x, origin.y, width, total);
        total
    }

    pub fn size_for(theme: &Theme, count: usize, width: f32) -> Size {
        let cols = Self::columns_for(theme, width);
        let rows = count.div_ceil(cols) as f32;
        Size::new(width, (rows * (Tile::height(theme) + theme.space(2)) - theme.space(2)).max(0.0))
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<(usize, TileEvent)> {
        let mut out = Response::none();
        for (i, t) in self.tiles.iter_mut().enumerate() {
            let r = t.pointer(theme, event, now);
            out.repaint |= r.repaint;
            if let Some(e) = r.event {
                return Response::event((i, e));
            }
        }
        out
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.tiles.iter().find_map(|t| t.cursor(p))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        for t in &mut self.tiles {
            t.draw(ui);
        }
    }
}

/// Döşeme renkleri (dolgu, içerik, ikincil) — saf; `t` 0 = kapalı, 1 = açık tonu. Açık/kapalı
/// uçlarda palet çiftleridir; devre dışı `style::disabled_*`.
pub(crate) fn tile_colors(theme: &Theme, enabled: bool, t: f32) -> (Color, Color, Color) {
    let c = &theme.colors;
    if enabled {
        (c.surface_bright.lerp(&c.primary, t), c.on_surface.lerp(&c.on_primary, t), c.on_surface_variant.lerp(&c.on_primary, t))
    } else {
        let fill = disabled_container(theme);
        let d = disabled_content(theme, c.on_surface, &[fill]);
        (fill, d, d)
    }
}

// Erişilebilirlik (ADR-0011 isteği).
impl Tile {
    /// `Role::Switch` (AT-SPI ToggleButton, `pressed`): ad etiket, açıklama alt metin. Alt sayfası
    /// varsa `Expand` eylemi. AT `Click` çevirir, `Expand` alt sayfa ister ([`Tile::access_action`]).
    pub fn access<'t>(&self, tree: &'t mut crate::access::AccessTree, key: impl std::hash::Hash) -> crate::access::AccessNode<'t> {
        let node = tree
            .push(key, crate::access::Role::Switch)
            .label(self.label.text())
            .description(self.subtitle.as_ref().map_or("", |s| s.text()))
            .bounds(self.core.rect)
            .enabled(self.core.enabled)
            .toggled(self.on)
            .clickable();
        if self.expandable { node.action(crate::access::Action::Expand) } else { node }
    }

    /// AT eylemi tıklamayla aynı olay: `Click` → `Toggled(yeni)`, `Expand` → `Expand`.
    pub fn access_action(&mut self, action: crate::access::AccessAction, now: Instant) -> Response<TileEvent> {
        use crate::access::AccessAction;
        if !self.core.enabled {
            return Response::none();
        }
        match action {
            AccessAction::Click => {
                self.set_on(!self.on, now);
                Response::event(TileEvent::Toggled(self.on))
            }
            AccessAction::Expand if self.expandable => Response::event(TileEvent::Expand),
            _ => Response::none(),
        }
    }
}

impl TileGrid {
    /// `Role::Group` kabı; kutucuklar `tile_key(i)` anahtarıyla (kalıcı kimlik). Kabın adı çağıranın.
    pub fn access<'t, K: std::hash::Hash>(
        &self,
        tree: &'t mut crate::access::AccessTree,
        key: impl std::hash::Hash,
        tile_key: impl Fn(usize) -> K,
    ) -> crate::access::AccessNode<'t> {
        let id = tree.push(key, crate::access::Role::Group).id();
        tree.within(id, |tree| {
            for (i, t) in self.tiles().iter().enumerate() {
                t.access(tree, tile_key(i));
            }
        });
        tree.get(id).expect("az önce eklendi")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn grid() -> (TileGrid, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let items = vec![
            TileItem::new(Icon::named(IconName::Wifi), "Wi-Fi").subtitle("ornek-ag").on(true).expandable(true),
            TileItem::new(Icon::named(IconName::Bluetooth), "Bluetooth").expandable(true),
            TileItem::new(Icon::named(IconName::DarkMode), "Karanlık tema").on(true),
            TileItem::new(Icon::named(IconName::DoNotDisturb), "Rahatsız etme"),
            TileItem::new(Icon::named(IconName::Flight), "Uçak modu"),
        ];
        let mut g = TileGrid::new(&theme, items);
        g.layout(&theme, Point::ZERO, 400.0);
        (g, theme, t0)
    }

    #[test]
    fn body_toggles_chevron_expands() {
        let (mut g, theme, t0) = grid();
        let r = g.tiles()[0].rect();
        let body = Point::new(r.x + 20.0, r.center().y);
        let _ = g.pointer(&theme, &press(body.x, body.y), t0);
        assert_eq!(g.pointer(&theme, &release(body.x, body.y), t0).event, Some((0, TileEvent::Toggled(false))));
        let chev = Point::new(r.right() - 10.0, r.center().y);
        let _ = g.pointer(&theme, &motion(chev.x, chev.y), t0);
        let _ = g.pointer(&theme, &press(chev.x, chev.y), t0);
        assert_eq!(g.pointer(&theme, &release(chev.x, chev.y), t0).event, Some((0, TileEvent::Expand)));
        assert!(!g.tiles()[0].is_on(), "› çevirmez");
        assert_eq!(g.tiles_mut()[1].key(&key(crate::input::Key::Right), t0).event, Some(TileEvent::Expand));
    }

    #[test]
    fn columns_follow_width() {
        let theme = Theme::default();
        assert_eq!(TileGrid::columns_for(&theme, 300.0), 2);
        assert_eq!(TileGrid::columns_for(&theme, 520.0), 3);
        assert_eq!(TileGrid::columns_for(&theme, 2000.0), 4);
        let (g, theme, _) = grid();
        assert_eq!(g.columns(), 2);
        assert_eq!(TileGrid::size_for(&theme, 5, 400.0).height, 3.0 * Tile::height(&theme) + 2.0 * theme.space(2));
    }

    #[test]
    fn debug_is_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let theme = Theme::default();
        let item = TileItem::new(Icon::named(IconName::Wifi), SECRET).subtitle(SECRET);
        assert_redacted(&item);
        assert_redacted(&TileGrid::new(&theme, vec![item]));
    }
}
