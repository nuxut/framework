//! Takvim etkinliği gösterimi: liste satırı ([`EventRow`]) ve ayrıntı kartı ([`EventCard`]).
//!
//! Renk takvimin kendi rengidir (veri; `None` = tema vurgusu) ve yalnızca sol şeritte /
//! noktada kullanılır — metin hep tema token'larıyla, böylece her takvim renginde okunur.
//! Zaman metnini çağıran biçimler (locale, 12/24 saat, "Tüm gün"); bileşen saat hesabı yapmaz.
//! Geçmiş etkinlik soluk, süren etkinlik zaman metni `primary`.

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect};
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, KeyEvent, PointerEvent};
use crate::label::Label;
use crate::pressable::Pressable;
use crate::style::{DISABLED_CONTENT, clip_text, faded_mark, faded_text, pill, state_layer_under};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Başlık, zaman, konum ve takvim adının en uzun hâli (bayt).
pub const MAX_AGENDA_TEXT: usize = 256;
/// Açıklamanın en uzun hâli (bayt); kart yine [`EventCard::with_description_lines`] satırda keser.
pub const MAX_AGENDA_DESCRIPTION: usize = 4096;

/// Bir etkinliğin gösterilecek alanları.
#[derive(Clone, PartialEq, Default)]
pub struct AgendaItem {
    pub title: String,
    /// Biçimlenmiş zaman: "14:30", "14:30 – 15:00", "Tüm gün".
    pub time: String,
    pub location: Option<String>,
    /// Takvim adı (yalnızca kartta).
    pub calendar: Option<String>,
    /// Açıklama (yalnızca kartta).
    pub description: Option<String>,
    /// Takvim rengi; `None` = `primary`.
    pub color: Option<Color>,
    /// Bitmiş etkinlik: soluk çizilir.
    pub past: bool,
    /// Şu an sürüyor: zaman metni `primary`.
    pub ongoing: bool,
}

/// Etkinlik içeriği kullanıcı verisidir: `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for AgendaItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgendaItem")
            .field("title", &crate::redact::Len(&self.title))
            .field("time", &crate::redact::Len(&self.time))
            .field("location", &crate::redact::OptLen(self.location.as_deref()))
            .field("calendar", &crate::redact::OptLen(self.calendar.as_deref()))
            .field("description", &crate::redact::OptLen(self.description.as_deref()))
            .field("color", &self.color.is_some())
            .field("past", &self.past)
            .field("ongoing", &self.ongoing)
            .finish()
    }
}

impl AgendaItem {
    pub fn new(title: impl Into<String>, time: impl Into<String>) -> Self {
        Self { title: title.into(), time: time.into(), ..Self::default() }
    }

    pub fn location(mut self, s: impl Into<String>) -> Self {
        self.location = Some(s.into());
        self
    }

    pub fn calendar(mut self, s: impl Into<String>) -> Self {
        self.calendar = Some(s.into());
        self
    }

    pub fn description(mut self, s: impl Into<String>) -> Self {
        self.description = Some(s.into());
        self
    }

    pub fn color(mut self, c: Option<Color>) -> Self {
        self.color = c;
        self
    }

    pub fn past(mut self, on: bool) -> Self {
        self.past = on;
        self
    }

    pub fn ongoing(mut self, on: bool) -> Self {
        self.ongoing = on;
        self
    }
}

/// Satır/kart metin renkleri: (ana, ikincil, zaman, şerit). Geçmiş etkinlik soluk ama okunur:
/// yüksek karşıtlıkta metin ≥ 4.5, şerit ≥ 3 (`style::faded_text` / `faded_mark`).
pub(crate) fn palette(theme: &Theme, item: &AgendaItem, fills: &[Color]) -> (Color, Color, Color, Color) {
    let c = &theme.colors;
    let fade = if item.past { DISABLED_CONTENT } else { 1.0 };
    let time = if item.ongoing { c.primary } else { c.on_surface_variant };
    let stripe = item.color.unwrap_or(c.primary);
    (
        faded_text(theme, c.on_surface, fade, fills),
        faded_text(theme, c.on_surface_variant, fade, fills),
        faded_text(theme, time, fade, fills),
        faded_mark(theme, stripe, fade, fills),
    )
}

/// "zaman · konum" alt satırı.
fn meta_text(item: &AgendaItem) -> String {
    let time = clip_text(&item.time, MAX_AGENDA_TEXT);
    match item.location.as_deref().filter(|l| !l.is_empty()) {
        Some(loc) if !time.is_empty() => format!("{time} · {}", clip_text(loc, MAX_AGENDA_TEXT)),
        Some(loc) => clip_text(loc, MAX_AGENDA_TEXT).to_owned(),
        None => time.to_owned(),
    }
}

/// Liste satırı: sol takvim şeridi, başlık, altında "zaman · konum". Tıklanınca/Enter'da
/// `Response::event(())` (çağıran ayrıntıyı açar).
#[derive(Debug, Clone)]
pub struct EventRow {
    core: Pressable,
    item: AgendaItem,
    title: Label,
    meta: Label,
}

impl EventRow {
    pub fn new(theme: &Theme, item: AgendaItem) -> Self {
        Self {
            core: Pressable::new(theme),
            title: Label::new(clip_text(&item.title, MAX_AGENDA_TEXT), Self::title_style(theme)),
            meta: Label::new(meta_text(&item), Self::meta_style(theme)),
            item,
        }
    }

    fn title_style(theme: &Theme) -> nuxut_render::TextStyle {
        text_style(theme, TextRole::Body).with_max_lines(1)
    }

    fn meta_style(theme: &Theme) -> nuxut_render::TextStyle {
        text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1)
    }

    pub fn item(&self) -> &AgendaItem {
        &self.item
    }

    /// İçerik değişti (yeniden senkron, "süren" durumu döndü).
    pub fn set_item(&mut self, item: AgendaItem) {
        if self.item == item {
            return;
        }
        self.title.set_text(clip_text(&item.title, MAX_AGENDA_TEXT));
        self.meta.set_text(&meta_text(&item));
        self.item = item;
        self.core.damage.mark();
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.title.set_style(Self::title_style(theme));
        self.meta.set_style(Self::meta_style(theme));
    }

    /// Yükseklik `space(14)` (iki satır metin).
    pub fn height(theme: &Theme) -> f32 {
        theme.space(14)
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    fn text_x(theme: &Theme, rect: Rect) -> f32 {
        rect.x + theme.space(3) + theme.space(1) + theme.space(3)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.core.set_rect(rect) {
            let w = (rect.right() - theme.space(3) - Self::text_x(theme, rect)).max(0.0);
            self.title.set_max_width(Some(w));
            self.meta.set_max_width(Some(w));
        }
    }

    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        self.core.pointer(event, now)
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response {
        self.core.key(event, now)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let rect = self.core.rect;
        let radii = Corners::uniform(theme.shape.radius_medium.min(rect.height * 0.5));
        // Hover katmanı tam tonlu metni ≥ 4.5 tutacak kadarla sınırlı; soluk (geçmiş) metin bu
        // katmanın üstündeki birleşime göre seçilir.
        let c = &theme.colors;
        let layer = state_layer_under(theme, c.on_surface, self.core.interaction.opacity(now), &[], &[c.on_surface, c.on_surface_variant, c.primary]);
        if layer.a > 0.0 {
            ui.painter.rect(rect, &RectStyle::filled(ui.fade(layer), radii));
        }
        self.core.draw_focus(ui, rect, radii);
        let (main, _, time, stripe) = palette(theme, &self.item, &[layer]);
        let bar = Rect::new(rect.x + theme.space(3), rect.y + theme.space(2), theme.space(1), rect.height - theme.space(4));
        ui.painter.rect(bar, &RectStyle::filled(ui.fade(stripe), pill(bar)));
        let x = Self::text_x(theme, rect);
        let ts = self.title.measure_in(ui.painter);
        let ms = self.meta.measure_in(ui.painter);
        let gap = theme.space(1) * 0.5;
        let top = rect.center().y - (ts.height + gap + ms.height) * 0.5;
        self.title.draw(ui.painter, Point::new(x, top), ui.fade(main));
        self.meta.draw(ui.painter, Point::new(x, top + ts.height + gap), ui.fade(time));
        self.core.finish(ui, self.core.extent());
    }
}

/// Ayrıntı kartı (popover/panel içeriği): renkli nokta + başlık, zaman, konum, takvim adı,
/// açıklama. Etkileşimsizdir; yüksekliği içerikten ([`EventCard::preferred_height`]).
#[derive(Debug, Clone)]
pub struct EventCard {
    item: AgendaItem,
    title: Label,
    /// (ikon, metin) satırları: zaman, konum, takvim.
    lines: Vec<(IconView, Label)>,
    description: Option<Label>,
    description_lines: usize,
    rect: Rect,
    damage: DamageState,
}

/// Kart açıklamasının varsayılan en çok satırı.
pub const CARD_DESCRIPTION_LINES: usize = 6;

impl EventCard {
    pub fn new(theme: &Theme, item: AgendaItem) -> Self {
        let mut c = Self {
            item: AgendaItem::default(),
            title: Label::new("", Self::title_style(theme)),
            lines: Vec::new(),
            description: None,
            description_lines: CARD_DESCRIPTION_LINES,
            rect: Rect::ZERO,
            damage: DamageState::new(),
        };
        c.rebuild(theme, item);
        c
    }

    /// Açıklama en çok `n` satır (`…` ile kesilir).
    pub fn with_description_lines(mut self, theme: &Theme, n: usize) -> Self {
        self.description_lines = n.max(1);
        if let Some(d) = &mut self.description {
            d.set_style(text_style(theme, TextRole::Document).with_max_lines(self.description_lines));
        }
        self
    }

    fn title_style(theme: &Theme) -> nuxut_render::TextStyle {
        text_style(theme, TextRole::Title).with_max_lines(2)
    }

    fn rebuild(&mut self, theme: &Theme, item: AgendaItem) {
        self.title.set_text(clip_text(&item.title, MAX_AGENDA_TEXT));
        let body = text_style(theme, TextRole::Body).with_max_lines(2);
        let mut lines = vec![(IconName::Schedule, clip_text(&item.time, MAX_AGENDA_TEXT).to_owned())];
        if let Some(l) = item.location.as_deref().filter(|s| !s.is_empty()) {
            lines.push((IconName::Location, clip_text(l, MAX_AGENDA_TEXT).to_owned()));
        }
        if let Some(l) = item.calendar.as_deref().filter(|s| !s.is_empty()) {
            lines.push((IconName::Event, clip_text(l, MAX_AGENDA_TEXT).to_owned()));
        }
        self.lines = lines
            .into_iter()
            .map(|(icon, text)| {
                let style = if icon == IconName::Schedule { body.clone().with_tabular_numbers() } else { body.clone() };
                (IconView::with_role(theme, Icon::named(icon), TextRole::IconSmall), Label::new(text, style))
            })
            .collect();
        self.description = item
            .description
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|d| Label::new(clip_text(d, MAX_AGENDA_DESCRIPTION), text_style(theme, TextRole::Document).with_max_lines(self.description_lines)));
        self.item = item;
        // Genişlik sınırları yeniden kurulsun.
        let r = self.rect;
        self.rect = Rect::ZERO;
        self.set_rect(theme, r);
        self.damage.mark();
    }

    pub fn item(&self) -> &AgendaItem {
        &self.item
    }

    pub fn set_item(&mut self, theme: &Theme, item: AgendaItem) {
        if self.item != item {
            self.rebuild(theme, item);
        }
    }

    pub fn set_theme(&mut self, theme: &Theme) {
        let item = std::mem::take(&mut self.item);
        self.title.set_style(Self::title_style(theme));
        self.rebuild(theme, item);
    }

    /// İç boşluk `space(4)`; metin sütunu ikon sütunundan sonra.
    fn text_x(theme: &Theme, rect: Rect) -> f32 {
        rect.x + theme.space(4) + text_style(theme, TextRole::IconSmall).size + theme.space(3)
    }

    fn apply_widths(&mut self, theme: &Theme, width: f32) {
        let rect = Rect::new(0.0, 0.0, width, 0.0);
        let w = (rect.right() - theme.space(4) - Self::text_x(theme, rect)).max(0.0);
        self.title.set_max_width(Some(w));
        for (_, l) in &mut self.lines {
            l.set_max_width(Some(w));
        }
        if let Some(d) = &mut self.description {
            d.set_max_width(Some(w));
        }
    }

    /// Verilen genişlikteki içerik yüksekliği.
    pub fn preferred_height(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> f32 {
        self.apply_widths(theme, width);
        let mut h = theme.space(4) + self.title.measure(text).height;
        for (_, l) in &mut self.lines {
            h += theme.space(2) + l.measure(text).height;
        }
        if let Some(d) = &mut self.description {
            h += theme.space(3) + d.measure(text).height;
        }
        h + theme.space(4)
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.rect == rect {
            return;
        }
        if self.rect.width != rect.width {
            self.apply_widths(theme, rect.width);
        }
        self.rect = rect;
        self.damage.mark();
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let rect = self.rect;
        let (main, sub, time, stripe) = palette(theme, &self.item, &[]);
        let pad = theme.space(4);
        let icon_w = text_style(theme, TextRole::IconSmall).size;
        let icon_cx = rect.x + pad + icon_w * 0.5;
        let x = Self::text_x(theme, rect);
        let mut y = rect.y + pad;
        let ts = self.title.measure_in(ui.painter);
        // Renk noktası başlığın ilk satırıyla ortalanır (ikon boyunda yuvarlatılmış kare).
        let line_h = text_style(theme, TextRole::Title).size * 1.3;
        let dot = icon_w * 0.75;
        let dot_r = Rect::new(icon_cx - dot * 0.5, y + (line_h.min(ts.height) - dot) * 0.5, dot, dot);
        ui.painter.rect(dot_r, &RectStyle::filled(ui.fade(stripe), Corners::uniform(theme.shape.radius_small.min(dot * 0.5))));
        self.title.draw(ui.painter, Point::new(x, y), ui.fade(main));
        y += ts.height;
        for (i, (icon, label)) in self.lines.iter_mut().enumerate() {
            y += theme.space(2);
            let m = label.measure_in(ui.painter);
            let first_line = m.height.min(text_style(theme, TextRole::Body).size * 1.4);
            icon.draw_centered(ui.painter, Point::new(icon_cx, y + first_line * 0.5), ui.fade(sub));
            label.draw(ui.painter, Point::new(x, y), ui.fade(if i == 0 { time } else { main }));
            y += m.height;
        }
        if let Some(d) = &mut self.description {
            y += theme.space(3);
            d.draw(ui.painter, Point::new(x, y), ui.fade(sub));
        }
        self.damage.report(ui, rect, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;
    use crate::redact::{SECRET, assert_redacted};

    fn item() -> AgendaItem {
        AgendaItem::new("Toplantı", "14:30 – 15:00").location("Oda 3").calendar("İş").description("Gündem")
    }

    #[test]
    fn meta_joins_time_and_location() {
        assert_eq!(meta_text(&item()), "14:30 – 15:00 · Oda 3");
        assert_eq!(meta_text(&AgendaItem::new("a", "Tüm gün")), "Tüm gün");
        assert_eq!(meta_text(&AgendaItem::new("a", "").location("Ev")), "Ev");
    }

    #[test]
    fn debug_is_redacted() {
        let i = AgendaItem::new(SECRET, SECRET).location(SECRET).calendar(SECRET).description(SECRET);
        assert_redacted(&i);
        let theme = Theme::default();
        assert_redacted(&EventRow::new(&theme, i.clone()));
        assert_redacted(&EventCard::new(&theme, i));
    }

    #[test]
    fn row_activates_on_click_and_enter() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut row = EventRow::new(&theme, item());
        row.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, EventRow::height(&theme)));
        let _ = row.pointer(&press(20.0, 20.0), t0);
        assert!(row.pointer(&release(20.0, 20.0), t0).event.is_some());
        assert!(row.key(&key(crate::input::Key::Enter), t0).event.is_some());
        assert_eq!(row.cursor(Point::new(20.0, 20.0)), Some(CursorShape::Pointer));
    }

    #[test]
    fn card_optional_lines() {
        let theme = Theme::default();
        let full = EventCard::new(&theme, item());
        assert_eq!(full.lines.len(), 3);
        assert!(full.description.is_some());
        let bare = EventCard::new(&theme, AgendaItem::new("a", "Tüm gün").location(""));
        assert_eq!(bare.lines.len(), 1, "boş konum satırı yok");
        assert!(bare.description.is_none());
    }

    #[test]
    fn long_text_is_clipped() {
        let long = "x".repeat(MAX_AGENDA_TEXT * 2);
        let theme = Theme::default();
        let row = EventRow::new(&theme, AgendaItem::new(long.clone(), "09:00"));
        assert_eq!(row.title.text().len(), MAX_AGENDA_TEXT);
    }
}
