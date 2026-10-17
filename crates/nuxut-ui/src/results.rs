//! Arama sonuç listesi ([`ResultList`], launcher): binlerce öğede akıcı.
//!
//! - **Sanallaştırma:** yalnızca görünen satırlar (`ScrollView::visible_range`) yerleşir ve
//!   çizilir.
//! - **Layout önbelleği:** satır metinleri (eşleşme vurgulu zengin metin) yalnızca görünür
//!   satırlar için şekillendirilir ve sınırlı bir LRU'da tutulur; kaydırırken önbellekte olan
//!   satır yeniden şekillendirilmez.
//! - **Vurgu:** eşleşen bayt aralıkları kalın + `primary` (`layout_rich` + `SpanStyle`).
//! - **Klavye:** Yukarı/Aşağı, PageUp/PageDown, Home/End, Enter; seçim görünür tutulur.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;
use std::time::Instant;

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};

use nuxut_core::{Corners, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, SpanStyle, TextLayout, TextStyle, TextSystem};
use nuxut_theme::{STATE_HOVER, Theme};

use crate::badge::{draw_pill_frame, pill_padding, pill_size};
use crate::icon::{Icon, IconView};
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::scroll::ScrollView;
use crate::style::{clip_text, state_layer_under, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Önbellekte tutulan en fazla satır (görünenlerin birkaç katı).
pub const RESULT_CACHE_ROWS: usize = 96;
/// Başlık/alt metnin en uzun hâli (bayt).
pub const MAX_RESULT_TEXT: usize = 256;
/// Sağ metin yuvasının satır metin alanına oranla en fazla genişliği.
pub const TRAILING_MAX_FRACTION: f32 = 0.4;

#[derive(Clone, PartialEq)]
pub struct ResultItem {
    pub title: String,
    pub subtitle: Option<String>,
    pub icon: Option<Icon>,
    /// Başlıkta vurgulanacak bayt aralıkları (karakter sınırında; sıralı, çakışmasız).
    pub matches: Vec<Range<usize>>,
    /// Alt metnin (örnek) font ailesi (yoksa temanın); ağırlık ve boyut stilden. Başlık hep
    /// temanın fontundadır: simge fontlarında da okunur ve eşleşme vurgusu tutarlı kalır.
    pub subtitle_font: Option<String>,
    /// Sağ metin yuvası (ör. pano öğesinde "zaman · boyut"): sağa hizalı, ikincil renk, `Caption`,
    /// eşit genişlikli rakam. En fazla satır metin alanının [`TRAILING_MAX_FRACTION`] kadarı
    /// (taşan üç nokta); başlık ve alt metin kalan alanda kırpılır. Yoksa yerleşim yuvasızla aynı.
    pub trailing: Option<String>,
    /// Sağ yuva metinli rozet ([`crate::Pill`] renkleri, seçili satırda `surface` zemin)
    /// olarak çizilir; hapın tamamı aynı [`TRAILING_MAX_FRACTION`] sınırındadır. `false`: düz
    /// metin (varsayılan, eski davranış).
    pub trailing_pill: bool,
}

/// Başlık dosya adı/yolu olabilir: `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for ResultItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultItem")
            .field("title", &crate::redact::Len(&self.title))
            .field("subtitle", &crate::redact::OptLen(self.subtitle.as_deref()))
            .field("icon", &self.icon.is_some())
            .field("matches", &self.matches.len())
            .field("subtitle_font", &self.subtitle_font)
            .field("trailing", &crate::redact::OptLen(self.trailing.as_deref()))
            .field("trailing_pill", &self.trailing_pill)
            .finish()
    }
}

impl ResultItem {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), subtitle: None, icon: None, matches: Vec::new(), subtitle_font: None, trailing: None, trailing_pill: false }
    }

    pub fn subtitle(mut self, s: impl Into<String>) -> Self {
        self.subtitle = Some(s.into());
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// İkon yuvasına metin glifi (emoji seçici: `"😀"`); [`Icon::glyph`] kısa yolu. Glif ikon
    /// boyunda, yuvanın ortasında çizilir.
    pub fn glyph(self, glyph: &str) -> Self {
        self.icon(Icon::glyph(glyph))
    }

    /// Alt metni (örnek) bu font ailesiyle çiz (FontPicker). Kurulu değilse genel aileye düşer.
    pub fn subtitle_font(mut self, family: impl Into<String>) -> Self {
        self.subtitle_font = Some(family.into());
        self
    }

    /// Sağ metin yuvası ([`ResultItem::trailing`]).
    pub fn trailing(mut self, s: impl Into<String>) -> Self {
        self.trailing = Some(s.into());
        self
    }

    /// Sağ yuva metinli rozet olarak ("Yeni", "Beta"; `trailing_pill` alanı).
    pub fn trailing_pill(mut self, s: impl Into<String>) -> Self {
        self.trailing = Some(s.into());
        self.trailing_pill = true;
        self
    }

    pub fn matches(mut self, m: Vec<Range<usize>>) -> Self {
        self.matches = m;
        self
    }

    /// Eşleşmeleri karakter indekslerinden verir (nucleo `indices` gibi; sırasız ve tekrarlı
    /// olabilir): ardışık karakterler tek bayt aralığında birleşir, başlık dışındakiler atılır.
    pub fn match_chars(mut self, indices: &[u32]) -> Self {
        let mut idx: Vec<u32> = indices.to_vec();
        idx.sort_unstable();
        idx.dedup();
        let mut want = idx.into_iter().peekable();
        let mut out: Vec<Range<usize>> = Vec::new();
        for (ci, (b, ch)) in self.title.char_indices().enumerate() {
            let Some(&w) = want.peek() else { break };
            if w as usize != ci {
                continue;
            }
            want.next();
            let r = b..b + ch.len_utf8();
            match out.last_mut() {
                Some(last) if last.end == r.start => last.end = r.end,
                _ => out.push(r),
            }
        }
        self.matches = out;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultEvent {
    /// Seçim değişti (klavye/hover önizlemesi için).
    Selected(usize),
    /// Enter ya da tıklama.
    Activated(usize),
    /// → : seçili öğenin eylemler alt sayfası istendi (arama alanı odaktaysa çağıran yalnızca
    /// imleç metnin sonundayken iletir).
    Actions(usize),
}

/// Sağ yuva: ikincil tipografi, eşit genişlikli rakam (zaman/boyut sütunu satırdan satıra oynamaz).
fn trailing_style(theme: &Theme) -> TextStyle {
    text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1)
}

#[derive(Debug, Clone)]
struct RowCache {
    title: TextLayout,
    subtitle: Option<TextLayout>,
    trailing: Option<TextLayout>,
    /// Sağ yuva hap olarak çizilir.
    pill: bool,
    icon: Option<IconView>,
    used: u64,
}

#[derive(Debug, Clone)]
pub struct ResultList {
    items: Vec<ResultItem>,
    scroll: ScrollView,
    rect: Rect,
    selected: usize,
    /// Seçim yerleşimden önce verildi: ilk geçerli `set_rect`'te görünür kılınır.
    reveal_pending: bool,
    hovered: Option<usize>,
    pressed: Option<usize>,
    highlight: Animated<f32>,
    cache: HashMap<usize, RowCache>,
    tick: u64,
    /// Satır yüksekliği (temadan; `cursor` zamansız hesaplar).
    row_h: f32,
    title_style: TextStyle,
    sub_style: TextStyle,
    trail_style: TextStyle,
    /// Bu oturumda şekillendirilen satır sayısı (ölçüm/test için).
    shaped: u64,
    damage: DamageState,
}

impl ResultList {
    pub fn new(theme: &Theme) -> Self {
        Self {
            items: Vec::new(),
            scroll: ScrollView::new(theme),
            rect: Rect::ZERO,
            selected: 0,
            reveal_pending: false,
            hovered: None,
            pressed: None,
            highlight: Animated::new(0.0, theme.motion.spatial_fast),
            cache: HashMap::new(),
            tick: 0,
            row_h: Self::row_height(theme),
            title_style: text_style(theme, TextRole::Body).with_max_lines(1),
            sub_style: text_style(theme, TextRole::Caption).with_max_lines(1),
            trail_style: trailing_style(theme),
            shaped: 0,
            damage: DamageState::new(),
        }
    }

    pub fn row_height(theme: &Theme) -> f32 {
        theme.space(14)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn selected(&self) -> Option<usize> {
        (!self.items.is_empty()).then_some(self.selected)
    }

    /// Kaç satır şekillendirildi (önbellek isabeti sayılmaz).
    pub fn shaped_rows(&self) -> u64 {
        self.shaped
    }

    /// Yeni sonuçlar (arama değişti): seçim başa, kaydırma başa, önbellek boşalır.
    pub fn set_items(&mut self, theme: &Theme, items: Vec<ResultItem>, now: Instant) {
        self.items = items;
        self.cache.clear();
        self.selected = 0;
        self.reveal_pending = false;
        self.hovered = None;
        self.highlight.jump_to(0.0);
        self.scroll.set_geometry(self.rect, self.content_height(theme), now);
        self.scroll.scroll_to(0.0, now);
        self.damage.mark();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.scroll.set_theme(theme, now);
        self.highlight.set_motion(theme.motion.spatial_fast, now);
        self.row_h = Self::row_height(theme);
        self.title_style = text_style(theme, TextRole::Body).with_max_lines(1);
        self.sub_style = text_style(theme, TextRole::Caption).with_max_lines(1);
        self.trail_style = trailing_style(theme);
        self.cache.clear();
        self.damage.mark();
    }

    fn content_height(&self, theme: &Theme) -> f32 {
        self.items.len() as f32 * Self::row_height(theme)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        if self.rect != rect {
            if self.rect.width != rect.width {
                self.cache.clear();
            }
            self.rect = rect;
            self.scroll.set_geometry(rect, self.content_height(theme), now);
            self.damage.mark();
        }
        if self.reveal_pending && rect.height > 0.0 {
            self.reveal_pending = false;
            let h = Self::row_height(theme);
            let i = self.selected as f32;
            self.scroll.ensure_visible_now(i * h, (i + 1.0) * h, now);
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn row_at(&self, theme: &Theme, p: Point, now: Instant) -> Option<usize> {
        if !self.rect.contains(p) {
            return None;
        }
        let y = p.y - self.scroll.content_origin(now).y;
        let i = (y / Self::row_height(theme)).floor();
        (i >= 0.0 && (i as usize) < self.items.len()).then_some(i as usize)
    }

    /// Seçimi olaysız ayarlar ve satırı görünür kılar (dışarıdan gelen seçim). Yerleşimden önce
    /// çağrılırsa seçim saklanır, görünür kılma ilk `set_rect`'te animasyonsuz yapılır.
    pub fn set_selected(&mut self, theme: &Theme, i: usize, now: Instant) {
        let _ = self.select(theme, i, now);
        self.highlight.jump_to(self.selected as f32);
        if self.rect.height <= 0.0 && !self.items.is_empty() {
            self.reveal_pending = true;
        }
    }

    /// Tek satırın ikonunu değiştirir; yalnızca o satırın önbelleği düşer.
    pub fn set_item_icon(&mut self, i: usize, icon: Option<Icon>) {
        if let Some(item) = self.items.get_mut(i)
            && item.icon != icon
        {
            item.icon = icon;
            self.cache.remove(&i);
            self.damage.mark();
        }
    }

    fn select(&mut self, theme: &Theme, i: usize, now: Instant) -> Response<ResultEvent> {
        if self.items.is_empty() {
            return Response::none();
        }
        let i = i.min(self.items.len() - 1);
        let h = Self::row_height(theme);
        self.scroll.ensure_visible(i as f32 * h, (i + 1) as f32 * h, now);
        if i == self.selected {
            return Response::repaint();
        }
        self.selected = i;
        self.highlight.set_target(i as f32, now);
        self.damage.mark();
        Response::event(ResultEvent::Selected(i))
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<ResultEvent> {
        let mut out = self.scroll.pointer(theme, event, now).discard();
        if out.repaint {
            self.damage.mark();
        }
        for ev in self.scroll.forward(event) {
            let r = self.pointer_content(theme, &ev, now);
            if r.event.is_some() {
                return r.with_repaint(out.repaint);
            }
            out.repaint |= r.repaint;
        }
        out
    }

    fn pointer_content(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<ResultEvent> {
        let mut out = Response::none();
        let row = if event.kind == PointerKind::Leave { None } else { self.row_at(theme, event.position, now) };
        if row != self.hovered {
            self.hovered = row;
            self.damage.mark();
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Press(Button::Left) => self.pressed = row,
            PointerKind::Release(Button::Left) => {
                if let Some(i) = row.filter(|r| self.pressed == Some(*r)) {
                    self.pressed = None;
                    let _ = self.select(theme, i, now);
                    return Response::event(ResultEvent::Activated(i));
                }
                self.pressed = None;
            }
            _ => {}
        }
        out
    }

    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<ResultEvent> {
        if event.state == KeyState::Released || self.items.is_empty() {
            return Response::none();
        }
        let page = ((self.rect.height / Self::row_height(theme)).floor() as usize).max(1);
        let n = self.items.len();
        let s = self.selected;
        let next = match event.key {
            Key::Down => (s + 1).min(n - 1),
            Key::Up => s.saturating_sub(1),
            Key::PageDown => (s + page).min(n - 1),
            Key::PageUp => s.saturating_sub(page),
            Key::Home => 0,
            Key::End => n - 1,
            Key::Enter => return Response::event(ResultEvent::Activated(s)),
            Key::Right => return Response::event(ResultEvent::Actions(s)),
            _ => return Response::none(),
        };
        self.select(theme, next, now)
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        // İçerik kısaysa son satırın altı boş alandır (kaydırma yokken ofset 0).
        let content = self.items.len() as f32 * self.row_h;
        let row = if p.y - self.rect.y < content { CursorShape::Pointer } else { CursorShape::Default };
        Some(self.scroll.cursor(p).unwrap_or(row))
    }

    /// Satırın önbellekteki layout'u; yoksa şekillendirilir (LRU taşarsa en eskisi atılır).
    fn row_cache(&mut self, text: &mut TextSystem, theme: &Theme, i: usize, text_w: f32) -> &RowCache {
        self.tick += 1;
        let tick = self.tick;
        if !self.cache.contains_key(&i) {
            if self.cache.len() >= RESULT_CACHE_ROWS
                && let Some((&old, _)) = self.cache.iter().min_by_key(|(_, r)| r.used)
            {
                self.cache.remove(&old);
            }
            let item = &self.items[i];
            let title = clip_text(&item.title, MAX_RESULT_TEXT);
            let hl = SpanStyle { bold: true, color: Some(theme.colors.primary), ..SpanStyle::default() };
            let mut spans: Vec<(&str, SpanStyle)> = Vec::new();
            let mut at = 0;
            for m in &item.matches {
                let (a, b) = (m.start.min(title.len()), m.end.min(title.len()));
                if a < at || a >= b || !title.is_char_boundary(a) || !title.is_char_boundary(b) {
                    continue;
                }
                if a > at {
                    spans.push((&title[at..a], SpanStyle::default()));
                }
                spans.push((&title[a..b], hl));
                at = b;
            }
            if at < title.len() {
                spans.push((&title[at..], SpanStyle::default()));
            }
            let with_font = |style: &TextStyle| match &item.subtitle_font {
                Some(f) => TextStyle { family: clip_text(f, MAX_RESULT_TEXT).to_owned(), ..style.clone() },
                None => style.clone(),
            };
            // Sağ yuva önce: doğal genişliği (en fazla TRAILING_MAX_FRACTION) başlığa kalanı belirler.
            // Hapta sınır hapın tamamıdır: metne iç boşluk düşülmüş kalanı.
            let pill = item.trailing_pill;
            let pill_pad = if pill { pill_padding(theme).0 * 2.0 } else { 0.0 };
            let trailing = item.trailing.as_ref().map(|s| {
                let s = clip_text(s, MAX_RESULT_TEXT);
                let max = ((text_w * TRAILING_MAX_FRACTION).floor() - pill_pad).max(0.0);
                let free = text.layout(s, &self.trail_style, None);
                if free.size().width <= max { free } else { text.layout(s, &self.trail_style, Some(max)) }
            });
            let slot_w = |t: &TextLayout| if pill { pill_size(theme, t.size()).width } else { t.size().width };
            let main_w = match &trailing {
                Some(t) => (text_w - slot_w(t).min(text_w) - theme.space(3)).max(0.0),
                None => text_w,
            };
            let title = text.layout_rich(&spans, &self.title_style, Some(main_w));
            let subtitle = item.subtitle.as_ref().map(|s| text.layout(clip_text(s, MAX_RESULT_TEXT), &with_font(&self.sub_style), Some(main_w)));
            let icon = item.icon.clone().map(|ic| IconView::new(theme, ic));
            self.shaped += 1;
            self.cache.insert(i, RowCache { title, subtitle, trailing, pill, icon, used: tick });
        }
        let r = self.cache.get_mut(&i).expect("önbellek");
        r.used = tick;
        r
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let h = Self::row_height(theme);
        let span = self.scroll.visible_span(now);
        let range = crate::scroll::visible_range_uniform(span, h, self.items.len());
        let origin = self.scroll.content_origin(now);
        let pad = theme.space(4);
        let icon_w = text_style(theme, TextRole::Icon).size;
        let text_x = pad + icon_w + theme.space(3);
        let text_w = (self.rect.width - text_x - pad).max(0.0);
        let radii = Corners::uniform(theme.shape.radius_medium);
        let sel_y = self.highlight.value(now);
        let (hovered, selected) = (self.hovered, self.selected);
        // Önbelleği çizimden önce doldur (ödünç çakışmasız).
        for i in range.clone() {
            self.row_cache(ui.painter.text_system(), theme, i, text_w);
        }
        let cache = &mut self.cache;
        self.scroll.draw(ui, |ui, _, _| {
            for i in range {
                let row = Rect::new(origin.x, origin.y + i as f32 * h, 0.0, h);
                let Some(rc) = cache.get_mut(&i) else { continue };
                let rr = Rect::new(row.x, row.y, text_x + text_w + pad, h);
                // Seçim vurgusu satırlar arası yayla kayar.
                if i == selected {
                    let hl = Rect::new(rr.x, origin.y + sel_y * h, rr.width, h);
                    ui.painter.rect(hl, &RectStyle::filled(ui.fade(tonal_container(theme)), radii));
                }
                if hovered == Some(i) {
                    let under = if i == selected { tonal_container(theme) } else { nuxut_core::Color::TRANSPARENT };
                    let layer = state_layer_under(theme, c.on_surface, STATE_HOVER, &[under], &[c.on_surface, c.on_surface_variant]);
                    ui.painter.rect(rr, &RectStyle::filled(ui.fade(layer), radii));
                }
                let mid = rr.center().y;
                if let Some(icon) = &mut rc.icon {
                    icon.draw_centered(ui.painter, Point::new(rr.x + pad + icon_w * 0.5, mid), ui.fade(c.on_surface_variant));
                }
                let th = rc.title.size().height;
                let sh = rc.subtitle.as_ref().map_or(0.0, |s| s.size().height);
                let top = mid - (th + sh) * 0.5;
                ui.painter.text(&rc.title, Point::new(rr.x + text_x, top), ui.fade(c.on_surface));
                if let Some(s) = &rc.subtitle {
                    ui.painter.text(s, Point::new(rr.x + text_x, top + th), ui.fade(c.on_surface_variant));
                }
                if let Some(t) = &rc.trailing {
                    let ts = t.size();
                    let right = rr.x + text_x + text_w;
                    if rc.pill {
                        let ps = pill_size(theme, ts);
                        let pr = Rect::new((right - ps.width.min(text_w)).round(), (mid - ps.height * 0.5).round(), ps.width.min(text_w), ps.height);
                        let fg = draw_pill_frame(ui, pr, i == selected);
                        let at = Point::new(pr.x + (pr.width - ts.width) * 0.5, pr.y + (pr.height - ts.height) * 0.5);
                        ui.painter.text(t, at, fg);
                    } else {
                        let at = Point::new(right - ts.width.min(text_w), mid - ts.height * 0.5);
                        ui.painter.text(t, at, ui.fade(c.on_surface_variant));
                    }
                }
            }
        });
        let animating = !self.highlight.is_settled(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.rect, animating);
    }
}

// Erişilebilirlik (ADR-0011).
impl ResultList {
    /// `Role::ListBox` kabı; çocukları görünen satırlar (+ görünmese de seçili satır),
    /// `Role::ListBoxOption`: ad başlık, açıklama alt metin ve sağ yuva, seçili olan `selected`,
    /// "3 / 12" konumu. Satır anahtarı `item_key(indis)`: öğenin kalıcı kimliği (masaüstü
    /// kimliği, pano girdisi id'si) olmalı. Odak arama alanında kalır: alan
    /// `.active_descendant(list.access_selected(..))` ile seçili satırı AT'ye gösterir.
    pub fn access<'t, K: Hash>(&self, tree: &'t mut AccessTree, key: impl Hash, now: Instant, item_key: impl Fn(usize) -> K) -> AccessNode<'t> {
        let id = tree.push(key, Role::ListBox).bounds(self.rect).id();
        let n = self.items.len();
        let top = self.scroll.content_origin(now).y;
        let row = |i: usize| Rect::new(self.rect.x, top + i as f32 * self.row_h, self.rect.width, self.row_h);
        let visible = |i: usize| {
            let r = row(i);
            r.bottom() > self.rect.y && r.y < self.rect.bottom()
        };
        tree.within(id, |tree| {
            for i in (0..n).filter(|&i| visible(i) || i == self.selected) {
                let item = &self.items[i];
                let desc = [item.subtitle.as_deref().unwrap_or(""), item.trailing.as_deref().unwrap_or("")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                tree.push(item_key(i), Role::ListBoxOption)
                    .label(&item.title)
                    .description(&desc)
                    .bounds(row(i))
                    .selected(i == self.selected)
                    .position(i + 1, n)
                    .clickable();
            }
        });
        tree.get(id).expect("az önce eklendi")
    }

    /// Seçili satırın düğüm kimliği (`container`: `access`'in döndürdüğü). Liste boşsa `None`.
    pub fn access_selected<K: Hash>(&self, container: NodeId, item_key: impl Fn(usize) -> K) -> Option<NodeId> {
        (!self.items.is_empty()).then(|| child_id(container, item_key(self.selected)))
    }

    /// AT isteği: satırda `Click` → seçip `Activated`, `Focus`/`ScrollIntoView` → seçip
    /// `Selected` (klavyeyle aynı olaylar).
    pub fn access_action<K: Hash>(
        &mut self,
        theme: &Theme,
        container: NodeId,
        request: AccessRequest,
        item_key: impl Fn(usize) -> K,
        now: Instant,
    ) -> Response<ResultEvent> {
        let Some(i) = (0..self.items.len()).find(|&i| child_id(container, item_key(i)) == request.node) else {
            return Response::none();
        };
        match request.action {
            AccessAction::Click => {
                self.set_selected(theme, i, now);
                Response::event(ResultEvent::Activated(i))
            }
            AccessAction::Focus | AccessAction::ScrollIntoView => {
                self.set_selected(theme, i, now);
                Response::event(ResultEvent::Selected(i))
            }
            _ => Response::none(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn list(n: usize) -> (ResultList, Theme, TextSystem, Instant) {
        let theme = Theme::default();
        let text = TextSystem::new();
        let t0 = Instant::now();
        let mut l = ResultList::new(&theme);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 560.0), t0);
        let items = (0..n).map(|i| ResultItem::new(format!("Uygulama {i}")).matches(vec![Range { start: 0, end: 3 }])).collect();
        l.set_items(&theme, items, t0);
        (l, theme, text, t0)
    }

    #[test]
    fn selection_before_layout_is_revealed_on_first_rect() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut l = ResultList::new(&theme);
        let items = (0..100).map(|i| ResultItem::new(format!("Aile {i}"))).collect();
        l.set_items(&theme, items, t0);
        // Yerleşimsiz: seçim saklanır, kaydırma oynamaz.
        l.set_selected(&theme, 40, t0);
        assert_eq!(l.selected(), Some(40));
        assert_eq!(l.scroll.offset(t0), 0.0, "sıfır görünümde kaydırma yok");
        // Seçili satır başta görünürse ilk yerleşimde kaydırma yok.
        let mut top = ResultList::new(&theme);
        top.set_items(&theme, (0..100).map(|i| ResultItem::new(format!("Aile {i}"))).collect(), t0);
        top.set_selected(&theme, 2, t0);
        top.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 560.0), t0);
        assert_eq!(top.scroll.offset(t0), 0.0, "zaten görünür: bir satır bile kaymaz");
        // İlk yerleşim: animasyonsuz görünür kılınır (aynı anda).
        l.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 560.0), t0);
        let h = ResultList::row_height(&theme);
        let off = l.scroll.offset(t0);
        assert!((off - (41.0 * h - 560.0)).abs() < 1e-3, "satır 40 altta görünür: {off}");
        // Bekleyen görünür kılma bir kez uygulanır.
        l.scroll.scroll_to(0.0, t0);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 600.0), t0);
        let t1 = t0 + std::time::Duration::from_secs(3);
        assert!(l.scroll.offset(t1).abs() < 1e-3);
    }

    #[test]
    fn keyboard_navigation_and_activation() {
        let (mut l, theme, _, t0) = list(5000);
        assert_eq!(l.key(&theme, &key(Key::Down), t0).event, Some(ResultEvent::Selected(1)));
        let page = (560.0 / ResultList::row_height(&theme)) as usize;
        assert_eq!(l.key(&theme, &key(Key::PageDown), t0).event, Some(ResultEvent::Selected(1 + page)));
        assert_eq!(l.key(&theme, &key(Key::End), t0).event, Some(ResultEvent::Selected(4999)));
        assert_eq!(l.key(&theme, &key(Key::Enter), t0).event, Some(ResultEvent::Activated(4999)));
        assert!(l.scroll.max_offset() > 0.0);
        assert_eq!(l.key(&theme, &key(Key::Home), t0).event, Some(ResultEvent::Selected(0)));
    }

    #[test]
    fn lru_shapes_each_visible_row_once() {
        let (mut l, theme, mut text, _) = list(5000);
        for _ in 0..3 {
            for i in 0..10 {
                l.row_cache(&mut text, &theme, i, 300.0);
            }
        }
        assert_eq!(l.shaped_rows(), 10, "önbellek isabeti yeniden şekillendirmez");
        for i in 0..(RESULT_CACHE_ROWS + 20) {
            l.row_cache(&mut text, &theme, i, 300.0);
        }
        assert!(l.cache.len() <= RESULT_CACHE_ROWS);
    }

    #[test]
    fn click_activates_and_bad_ranges_are_ignored() {
        let (mut l, theme, mut text, t0) = list(10);
        l.items[0].matches = vec![Range { start: 5, end: 2 }, 0..999, 100..200];
        l.row_cache(&mut text, &theme, 0, 300.0);
        let y = ResultList::row_height(&theme) * 2.5;
        let _ = l.pointer(&theme, &press(50.0, y), t0);
        assert_eq!(l.pointer(&theme, &release(50.0, y), t0).event, Some(ResultEvent::Activated(2)));
        assert_eq!(l.cursor(Point::new(50.0, y)), Some(CursorShape::Pointer));
    }

    #[test]
    fn char_indices_become_merged_byte_ranges() {
        let it = ResultItem::new("Işık Ayarı").match_chars(&[2, 0, 1, 1, 5, 99]);
        assert_eq!(it.matches, vec![0..5, 7..8]);
        let (mut l, theme, _, t0) = list(3);
        assert_eq!(l.key(&theme, &key(Key::Right), t0).event, Some(ResultEvent::Actions(0)));
    }

    #[test]
    fn trailing_slot_measures() {
        let (mut l, theme, mut text, t0) = list(0);
        let long = "Çok uzun bir pano öğesi başlığı, satıra kesinlikle sığmayacak kadar uzun bir metin";
        l.set_items(
            &theme,
            vec![
                ResultItem::new(long).subtitle(long),
                ResultItem::new(long).subtitle(long).trailing("12:30 · 4 KB"),
                ResultItem::new(long).trailing("x".repeat(200)),
                ResultItem::new("Kısa"),
                ResultItem::new("Kısa").trailing("2 dk"),
            ],
            t0,
        );
        let w = 300.0;
        let plain = l.row_cache(&mut text, &theme, 0, w).clone();
        assert!(plain.trailing.is_none());
        // Yuvasız: başlık/alt metin tüm genişliği kullanır (bugünkü yerleşim).
        assert!(plain.title.size().width > w - 20.0 && plain.title.size().width <= w + 0.5);
        let slot = l.row_cache(&mut text, &theme, 1, w).clone();
        let tw = slot.trailing.as_ref().unwrap().size().width;
        let natural = text.layout("12:30 · 4 KB", &trailing_style(&theme), None).size().width;
        assert!((tw - natural).abs() < 0.5, "yuva doğal genişlikte: {tw} vs {natural}");
        let room = w - tw - theme.space(3);
        for t in [&slot.title, slot.subtitle.as_ref().unwrap()] {
            assert!(t.size().width <= room + 0.5 && t.size().width > room - 30.0, "kalan alanda kırpılır: {}", t.size().width);
            assert_eq!(t.line_count(), 1);
        }
        // Yuva taşmaz: en fazla TRAILING_MAX_FRACTION, başlığa yer kalır.
        let capped = l.row_cache(&mut text, &theme, 2, w).clone();
        let cw = capped.trailing.as_ref().unwrap().size().width;
        assert!(cw <= w * TRAILING_MAX_FRACTION + 0.5, "{cw}");
        assert!(capped.title.size().width >= w * (1.0 - TRAILING_MAX_FRACTION) - theme.space(3) - 30.0);
        // Kısa başlık yuvadan etkilenmez.
        let a = l.row_cache(&mut text, &theme, 3, w).title.size();
        let b = l.row_cache(&mut text, &theme, 4, w).title.size();
        assert_eq!(a, b);
    }

    #[test]
    fn glyph_rows_use_the_icon_slot() {
        let (mut l, theme, mut text, t0) = list(0);
        l.set_items(&theme, vec![ResultItem::new("Gülen yüz").glyph("😀"), ResultItem::new("Ara").icon(Icon::named(crate::IconName::Search))], t0);
        assert_eq!(l.items[0].icon, Some(Icon::Glyph("😀".to_owned())));
        let g = l.row_cache(&mut text, &theme, 0, 300.0).icon.clone().unwrap();
        let n = l.row_cache(&mut text, &theme, 1, 300.0).icon.clone().unwrap();
        assert_eq!(g.size(), n.size(), "glif ikonla aynı yuvada");
        assert_eq!(g.icon(), &Icon::Glyph("😀".to_owned()));
        // Çizim panik etmez (renkli emoji ya da yedek glif).
        let mut rec = nuxut_render::Recorder::new();
        rec.record(&mut text, (400, 600), 1.0, |p| {
            let mut ui = Ui::new(p, &theme, t0);
            l.draw(&mut ui);
        });
    }

    #[test]
    fn debug_is_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let item = ResultItem::new(SECRET).subtitle(SECRET).trailing(SECRET);
        assert_redacted(&item);
        let (mut l, theme, _, t0) = list(0);
        l.set_items(&theme, vec![item], t0);
        assert_redacted(&l);
        // Çizimden sonra satır önbelleği glif ikonunu da tutar.
        let (mut l, theme, mut text, t0) = list(0);
        l.set_items(&theme, vec![ResultItem::new(SECRET).glyph(SECRET)], t0);
        let mut rec = nuxut_render::Recorder::new();
        rec.record(&mut text, (400, 600), 1.0, |p| {
            let mut ui = Ui::new(p, &theme, t0);
            l.draw(&mut ui);
        });
        assert_redacted(&l);
    }

    /// Hap yuvası: hapın tamamı (metin + iç boşluk) TRAILING_MAX_FRACTION sınırında, başlık
    /// hapın solunda kalan alanda; çizimde zemin + metin; düz metin yuvası hap çizmez.
    #[test]
    fn trailing_pill_slot() {
        let (mut l, theme, mut text, t0) = list(0);
        let long = "Çok uzun bir başlık, satıra kesinlikle sığmayacak kadar uzun bir metin";
        l.set_items(
            &theme,
            vec![ResultItem::new(long).trailing_pill("Yeni"), ResultItem::new(long).trailing_pill("x".repeat(200)), ResultItem::new(long).trailing("Yeni")],
            t0,
        );
        assert!(l.items[0].trailing_pill && !l.items[2].trailing_pill);
        let w = 300.0;
        let pill = l.row_cache(&mut text, &theme, 0, w).clone();
        assert!(pill.pill);
        let ps = pill_size(&theme, pill.trailing.as_ref().unwrap().size());
        assert!(pill.title.size().width <= w - ps.width - theme.space(3) + 0.5);
        let capped = l.row_cache(&mut text, &theme, 1, w).clone();
        let cs = pill_size(&theme, capped.trailing.as_ref().unwrap().size());
        assert!(cs.width <= (w * TRAILING_MAX_FRACTION).floor() + 1.0, "hap tamamı sınırda: {}", cs.width);
        // Çizim: hap satırında yuva zemini (Quad) var, düz metin satırında yok.
        let mut quads = Vec::new();
        for i in [0, 2] {
            let mut one = ResultList::new(&theme);
            one.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 200.0), t0);
            one.set_items(&theme, vec![l.items[i].clone(), ResultItem::new("b")], t0);
            one.set_selected(&theme, 1, t0);
            let mut rec = nuxut_render::Recorder::new();
            rec.record(&mut text, (400, 200), 1.0, |p| {
                let mut ui = Ui::new(p, &theme, t0);
                one.draw(&mut ui);
            });
            quads.push(rec.quads().into_iter().filter(|q| q.rect.y < ResultList::row_height(&theme)).count());
        }
        assert_eq!(quads[0], quads[1] + 1, "{quads:?}");
    }
}
