//! Glif ızgarası ([`GlyphGrid`]): emoji / sembol / karakter seçimi (başlatıcının emoji kipi,
//! karakter haritası). Tasarım: `docs/design/glyph-grid.md`.
//!
//! - **Hücreler:** sabit kare (`cell_size`), sütun sayısı genişlikten ([`GlyphColumns::Auto`]) ya
//!   da sabit ([`GlyphColumns::Fixed`]); ızgara yatayda ortalanır. Hücrede glif metni olağan
//!   metin yolundan çizilir (renkli emoji `TextSystem`'in RGBA atlasıyla, sembol `on_surface`).
//! - **Bölümler:** isteğe bağlı kategori başlıkları ([`GlyphSection`]); her bölüm yeni satırda
//!   başlar. İndeksler bölümlerden bağımsız, düz (`0..len`).
//! - **Sanallaştırma:** yalnızca görünen satırlar (`ScrollView::visible_range`, ikili arama)
//!   şekillendirilir ve çizilir; glif layout'ları sınırlı bir LRU'da tutulur.
//! - **Klavye (2B):** ←→ doğrusal (satır/bölüm sınırını geçer), ↑↓ aynı sütun (kısa satırda son
//!   hücreye kırpılır, sütun "yapışkan" hatırlanır), PageUp/PageDown sayfa sayfa (görünüm
//!   yüksekliği kadar, görünüm de kayar), sağdan solda ([`GlyphGrid::set_rtl`]) ←/→ ters,
//!   Home/End satır başı/sonu, Ctrl+Home/End ilk/son öğe, Enter etkinleştirir. Seçim görünür
//!   tutulur (bölümün ilk satırında başlığıyla).
//! - **Durum:** seçili hücre tonal kap, hover state layer, klavye odağında halka (tema rolleri;
//!   yüksek karşıtlıkta seçili hücrede her zaman ince halka — seçim yalnız tonla anlatılmaz).
//! - **Hareket:** seçim vurgusu hücreler arası `spatial_fast` yayla kayar; azaltılmış harekette
//!   anında. Oturunca kare istemez (boşta sıfır kare).
//! - **Erişilebilirlik:** her öğenin adı ([`GlyphItem::label`]) tutulur (`label(i)`), ileride
//!   a11y ağacına verilecek.

use std::collections::HashMap;
use std::time::Instant;

use nuxut_core::{Color, Corners, Insets, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle, TextLayout, TextStyle, TextSystem};
use nuxut_theme::{STATE_HOVER, STATE_PRESSED, Theme};

use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::scroll::ScrollView;
use crate::style::{FOCUS_RING, HAIRLINE, clip_text, focus_ring, state_layer_under, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// En fazla öğe (dış veri sınırlı; Unicode emoji ~3800, karakter haritası için bol pay).
pub const MAX_GLYPH_ITEMS: usize = 65_536;
/// Glif metninin en uzun hâli (bayt). En uzun emoji ZWJ dizileri ~35 bayt.
pub const MAX_GLYPH_TEXT: usize = 64;
/// Erişilebilir ad / bölüm başlığının en uzun hâli (bayt).
pub const MAX_GLYPH_LABEL: usize = 256;
/// Glif layout önbelleğinin taban sığası (görünen hücre sayısının iki katından az olmaz).
pub const GLYPH_CACHE_CELLS: usize = 512;

/// Tek glif: çizilen metin (bir ya da birkaç kod noktası; ZWJ dizisi) + erişilebilir ad.
#[derive(Clone, PartialEq, Eq)]
pub struct GlyphItem {
    pub glyph: String,
    /// Ekran okuyucu adı / ipucu ("sırıtan yüz"); çizilmez.
    pub label: String,
}

impl GlyphItem {
    pub fn new(glyph: impl Into<String>, label: impl Into<String>) -> Self {
        Self { glyph: glyph.into(), label: label.into() }
    }
}

/// Son kullanılanlar listesi kullanıcı verisi sayılır: `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for GlyphItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphItem").field("glyph", &crate::redact::Len(&self.glyph)).field("label", &crate::redact::Len(&self.label)).finish()
    }
}

/// Kategori: başlık (boşsa başlık satırı yok) + öğeler. Boş bölüm yer kaplamaz.
#[derive(Clone, PartialEq, Eq)]
pub struct GlyphSection {
    pub title: String,
    pub items: Vec<GlyphItem>,
}

impl GlyphSection {
    pub fn new(title: impl Into<String>, items: Vec<GlyphItem>) -> Self {
        Self { title: title.into(), items }
    }
}

impl std::fmt::Debug for GlyphSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphSection").field("title", &crate::redact::Len(&self.title)).field("items", &self.items.len()).finish()
    }
}

/// Sütun sayısı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphColumns {
    /// Genişliğe sığan kadar `cell_size` kare hücre (en az 1).
    #[default]
    Auto,
    /// Sabit sütun; hücre kenarı `genişlik / n` (aşağı yuvarlı, kare).
    Fixed(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphGridEvent {
    /// Klavyeyle seçim değişti (önizleme: ad/kod noktası göstermek için).
    Selected(usize),
    /// Enter ya da tıklama.
    Activated(usize),
}

/// Hücre renkleri (saf; yüksek karşıtlık testi `contrast_tests` bunları ölçer).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CellColors {
    /// Seçili hücrenin kabı (seçili değilse saydam).
    pub fill: Color,
    /// Hover/basılı katmanı (`layer_op` opaklığında, `fill` üstünde).
    pub layer: Color,
    /// Sembol glifinin rengi (renkli emoji kendi renginde).
    pub glyph: Color,
    /// Odak halkası / yüksek karşıtlıkta seçim kenarı.
    pub ring: Color,
}

impl CellColors {
    pub fn new(theme: &Theme, selected: bool, layer_op: f32) -> Self {
        let c = &theme.colors;
        let fill = if selected { tonal_container(theme) } else { Color::TRANSPARENT };
        let layer = state_layer_under(theme, c.on_surface, layer_op, &[fill], &[c.on_surface]);
        Self { fill, layer, glyph: c.on_surface, ring: focus_ring(theme, &[fill]) }
    }
}

/// Bölüm başlığı rengi (yüzey üstünde ikincil metin).
pub(crate) fn header_color(theme: &Theme) -> Color {
    theme.colors.on_surface_variant
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Header(usize),
    Cells { start: usize, len: usize },
}

#[derive(Debug, Clone, Copy)]
struct RowSlot {
    y: f32,
    h: f32,
    kind: RowKind,
}

#[derive(Debug, Clone)]
struct SectionMeta {
    title: String,
    start: usize,
    len: usize,
    /// Başlık satırının (yoksa ilk hücre satırının) `rows` indisi.
    first_row: usize,
}

#[derive(Debug, Clone)]
struct CellCache {
    layout: TextLayout,
    used: u64,
}

/// Bölüm başlıkları / öğeler veri: `Debug` yalnızca sayıları yazar.
impl std::fmt::Debug for GlyphGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphGrid")
            .field("items", &crate::redact::Count(self.items.len()))
            .field("sections", &self.sections.len())
            .field("cols", &self.cols)
            .field("selected", &self.selected)
            .field("rect", &self.rect)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct GlyphGrid {
    items: Vec<GlyphItem>,
    sections: Vec<SectionMeta>,
    rows: Vec<RowSlot>,
    row_starts: Vec<f32>,
    /// Hücre satırlarının `rows` indisleri (öğe → satır ikili aramayla).
    cell_rows: Vec<usize>,
    columns: GlyphColumns,
    cols: usize,
    cell: f32,
    /// Izgaranın görünüm içindeki sol boşluğu (ortalama).
    grid_x: f32,
    /// Sağdan sola düzen: sütun 0 sağda, ←/→ ters (bkz. [`GlyphGrid::set_rtl`]).
    rtl: bool,
    scroll: ScrollView,
    rect: Rect,
    selected: usize,
    /// ↑↓ gezinmede hatırlanan sütun (kısa satırdan geçince geri döner).
    want_col: Option<usize>,
    reveal_pending: bool,
    hovered: Option<usize>,
    pressed: Option<usize>,
    focus_visible: bool,
    /// Seçim vurgusunun içerik koordinatındaki sol-üstü.
    highlight: Animated<Point>,
    cache: HashMap<usize, CellCache>,
    headers: HashMap<usize, TextLayout>,
    tick: u64,
    glyph_style: TextStyle,
    header_style: TextStyle,
    shaped: u64,
    /// Son karede çizilen hücre (ölçüm/test: sanallaştırma).
    drawn_cells: usize,
    damage: DamageState,
}

/// Mantıksal sütunun soldan görsel sütunu: sağdan solda aynalı (involüsyon: görselden mantıksala
/// da aynı dönüşüm).
fn visual_col(col: usize, cols: usize, rtl: bool) -> usize {
    if rtl { cols.saturating_sub(1).saturating_sub(col) } else { col }
}

impl GlyphGrid {
    pub fn new(theme: &Theme) -> Self {
        Self {
            items: Vec::new(),
            sections: Vec::new(),
            rows: Vec::new(),
            row_starts: Vec::new(),
            cell_rows: Vec::new(),
            columns: GlyphColumns::Auto,
            cols: 1,
            cell: Self::cell_size(theme),
            grid_x: 0.0,
            rtl: false,
            scroll: ScrollView::new(theme),
            rect: Rect::ZERO,
            selected: 0,
            want_col: None,
            reveal_pending: false,
            hovered: None,
            pressed: None,
            focus_visible: false,
            highlight: Animated::new(Point::ZERO, theme.motion.spatial_fast),
            cache: HashMap::new(),
            headers: HashMap::new(),
            tick: 0,
            glyph_style: Self::glyph_style(theme),
            header_style: Self::header_style(theme),
            shaped: 0,
            drawn_cells: 0,
            damage: DamageState::new(),
        }
    }

    /// Kare hücre kenarı (`Auto`): en az dokunma hedefi.
    pub fn cell_size(theme: &Theme) -> f32 {
        theme.space(11)
    }

    /// Bölüm başlığı satırının yüksekliği.
    pub fn header_height(theme: &Theme) -> f32 {
        theme.space(8)
    }

    /// Glif: arayüz fontu (emoji/sembol fontlarına düşer), `Display` boyunda, tek satır, ortalı.
    fn glyph_style(theme: &Theme) -> TextStyle {
        let mut s = text_style(theme, TextRole::Body).with_max_lines(1);
        s.size = text_style(theme, TextRole::Display).size;
        s
    }

    fn header_style(theme: &Theme) -> TextStyle {
        text_style(theme, TextRole::Label).with_max_lines(1)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn item(&self, i: usize) -> Option<&GlyphItem> {
        self.items.get(i)
    }

    /// Öğenin erişilebilir adı.
    pub fn label(&self, i: usize) -> Option<&str> {
        self.items.get(i).map(|it| it.label.as_str())
    }

    pub fn selected(&self) -> Option<usize> {
        (!self.items.is_empty()).then_some(self.selected)
    }

    /// Geçerli sütun sayısı (son `set_rect`'e göre).
    pub fn columns(&self) -> usize {
        self.cols
    }

    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// Öğenin bölümü (boş bölümler sayılır: `set_sections`'taki sıra).
    pub fn section_of(&self, i: usize) -> Option<usize> {
        if i >= self.items.len() {
            return None;
        }
        let s = self.sections.partition_point(|m| m.start <= i).checked_sub(1)?;
        // Aynı başlangıçlı boş bölümler atlanır: öğeyi içeren sonuncusu.
        (self.sections[s].len > 0).then_some(s)
    }

    /// Bölümün ilk öğesi (boş bölümde `None`).
    pub fn section_start(&self, s: usize) -> Option<usize> {
        self.sections.get(s).filter(|m| m.len > 0).map(|m| m.start)
    }

    /// Şekillendirilen glif sayısı (önbellek isabeti sayılmaz; ölçüm/test).
    pub fn shaped_cells(&self) -> u64 {
        self.shaped
    }

    /// Bölümsüz düz liste (başlık yok).
    pub fn set_items(&mut self, theme: &Theme, items: Vec<GlyphItem>, now: Instant) {
        self.set_sections(theme, vec![GlyphSection::new("", items)], now);
    }

    /// Yeni veri (arama/kategori değişti): seçim ve kaydırma başa, önbellek boşalır. Toplam
    /// [`MAX_GLYPH_ITEMS`]'ı aşan öğeler atılır.
    pub fn set_sections(&mut self, theme: &Theme, sections: Vec<GlyphSection>, now: Instant) {
        self.load_sections(sections, 0);
        self.reveal_pending = false;
        self.relayout(theme, now);
        self.scroll.scroll_to(0.0, now);
        self.highlight.jump_to(self.cell_origin(0).unwrap_or(Point::ZERO));
    }

    /// Aynı görünümün verisi yenilendi (dil değişti, kayıt güncellendi): kaydırma konumu ve
    /// süren hareketi korunur — yalnız yeni içerik yüksekliğine sınırlanır —, seçim `selected`'a
    /// (taşarsa son öğeye) olaysız ve kaydırmasız geçer, vurgu yeni yerine atlar; önbellek boşalır.
    /// Arama/kategori değişimi gibi "yeni liste"de [`set_sections`](Self::set_sections).
    pub fn replace_sections(&mut self, theme: &Theme, sections: Vec<GlyphSection>, selected: usize, now: Instant) {
        self.load_sections(sections, selected);
        // Yerleşimden önceyse seçim ilk `set_rect`'te görünür kılınır (`set_selected` gibi).
        self.reveal_pending = self.rect.height <= 0.0 && !self.items.is_empty();
        self.relayout(theme, now);
    }

    /// Veriyi yükler, seçimi `selected`'a kırpar, geçici durumu sıfırlar (kaydırmaya dokunmaz).
    fn load_sections(&mut self, sections: Vec<GlyphSection>, selected: usize) {
        self.items.clear();
        self.sections.clear();
        for sec in sections {
            let start = self.items.len();
            let room = MAX_GLYPH_ITEMS - start;
            let mut items = sec.items;
            items.truncate(room);
            self.items.extend(items);
            let mut title = sec.title;
            title.truncate(clip_text(&title, MAX_GLYPH_LABEL).len());
            self.sections.push(SectionMeta { title, start, len: self.items.len() - start, first_row: 0 });
        }
        self.cache.clear();
        self.headers.clear();
        self.selected = selected.min(self.items.len().saturating_sub(1));
        self.want_col = None;
        self.hovered = None;
        self.pressed = None;
        self.damage.mark();
    }

    pub fn set_columns(&mut self, theme: &Theme, columns: GlyphColumns, now: Instant) {
        if self.columns != columns {
            self.columns = columns;
            // Başlık genişliği ızgara kenar boşluğuna (sütun sayısına) bağlı.
            self.headers.clear();
            self.relayout(theme, now);
        }
    }

    /// Yazı yönü (çağıran `nuxut_i18n::current().direction() == Direction::Rtl` verir; dil
    /// değişince yeniden). Sağdan solda sütun 0 sağdadır (kısa satır sağa yaslanır), ← sonraki
    /// öğe, → önceki öğe; başlıklar sağa yaslanır. ↑↓, Home/End (satırın mantıksal başı/sonu) ve
    /// indisler değişmez. Seçim vurgusu yeni yerine atlar.
    pub fn set_rtl(&mut self, rtl: bool) {
        if self.rtl != rtl {
            self.rtl = rtl;
            if let Some(p) = self.cell_origin(self.selected) {
                self.highlight.jump_to(p);
            }
            self.damage.mark();
        }
    }

    pub fn is_rtl(&self) -> bool {
        self.rtl
    }

    /// Mantıksal sütunun görsel sütunu (soldan; sağdan solda aynalı).
    fn visual_col(&self, col: usize) -> usize {
        visual_col(col, self.cols, self.rtl)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.scroll.set_theme(theme, now);
        self.highlight.set_motion(theme.motion.spatial_fast, now);
        self.glyph_style = Self::glyph_style(theme);
        self.header_style = Self::header_style(theme);
        self.cache.clear();
        self.headers.clear();
        self.relayout(theme, now);
    }

    /// Odak halkası (klavye odağı; [`FocusScope::ring_visible_for`](crate::FocusScope)).
    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        if self.focus_visible == on {
            return false;
        }
        self.focus_visible = on;
        self.damage.mark();
        true
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        if self.rect != rect {
            if self.rect.width != rect.width {
                self.headers.clear();
            }
            self.rect = rect;
            self.relayout(theme, now);
        }
        if self.reveal_pending && rect.height > 0.0 && !self.items.is_empty() {
            self.reveal_pending = false;
            let (top, bottom) = self.reveal_span(self.selected);
            self.scroll.ensure_visible_now(top, bottom, now);
        }
    }

    /// Sütunları ve satır tablosunu yeniden kurar; seçim vurgusu yeni yerine atlar.
    fn relayout(&mut self, theme: &Theme, now: Instant) {
        let w = self.rect.width.max(0.0);
        let base = Self::cell_size(theme);
        let (cols, cell) = match self.columns {
            GlyphColumns::Auto => (((w / base).floor() as usize).max(1), base),
            GlyphColumns::Fixed(n) => {
                let n = n.clamp(1, MAX_GLYPH_ITEMS);
                (n, (w / n as f32).floor().max(1.0))
            }
        };
        self.cols = cols;
        self.cell = cell;
        self.grid_x = ((w - cols as f32 * cell) * 0.5).floor().max(0.0);
        let header_h = Self::header_height(theme);
        self.rows.clear();
        self.cell_rows.clear();
        let mut y = 0.0;
        for (s, sec) in self.sections.iter_mut().enumerate() {
            sec.first_row = self.rows.len();
            if sec.len == 0 {
                continue;
            }
            if !sec.title.is_empty() {
                self.rows.push(RowSlot { y, h: header_h, kind: RowKind::Header(s) });
                y += header_h;
            }
            let mut at = 0;
            while at < sec.len {
                let len = cols.min(sec.len - at);
                self.cell_rows.push(self.rows.len());
                self.rows.push(RowSlot { y, h: cell, kind: RowKind::Cells { start: sec.start + at, len } });
                y += cell;
                at += len;
            }
        }
        self.row_starts.clear();
        self.row_starts.extend(self.rows.iter().map(|r| r.y));
        self.scroll.set_geometry(self.rect, y, now);
        if let Some(p) = self.cell_origin(self.selected) {
            self.highlight.jump_to(p);
        }
        self.damage.mark();
    }

    /// Öğenin hücre satırı (`cell_rows` sırası) ve sütunu.
    fn locate(&self, i: usize) -> Option<(usize, usize)> {
        if i >= self.items.len() {
            return None;
        }
        let k = self
            .cell_rows
            .partition_point(|&r| matches!(self.rows[r].kind, RowKind::Cells { start, .. } if start <= i))
            .checked_sub(1)?;
        match self.rows[self.cell_rows[k]].kind {
            RowKind::Cells { start, len } if i < start + len => Some((k, i - start)),
            _ => None,
        }
    }

    fn row_cells(&self, k: usize) -> (usize, usize) {
        match self.rows[self.cell_rows[k]].kind {
            RowKind::Cells { start, len } => (start, len),
            RowKind::Header(_) => (0, 0),
        }
    }

    /// Hücrenin içerik koordinatındaki sol-üstü (görünüm sol kenarına göre x).
    fn cell_origin(&self, i: usize) -> Option<Point> {
        let (k, col) = self.locate(i)?;
        let row = self.rows[self.cell_rows[k]];
        Some(Point::new(self.grid_x + self.visual_col(col) as f32 * self.cell, row.y))
    }

    /// Görünür tutulacak dikey aralık: hücre satırı; bölümün ilk satırıysa başlığıyla.
    fn reveal_span(&self, i: usize) -> (f32, f32) {
        let Some((k, _)) = self.locate(i) else { return (0.0, 0.0) };
        let r = self.cell_rows[k];
        let row = self.rows[r];
        let top = match r.checked_sub(1).map(|p| self.rows[p]) {
            Some(RowSlot { y, kind: RowKind::Header(_), .. }) => y,
            _ => row.y,
        };
        (top, row.y + row.h)
    }

    fn cell_at(&self, p: Point, now: Instant) -> Option<usize> {
        if !self.rect.contains(p) {
            return None;
        }
        let origin = self.scroll.content_origin(now);
        let y = p.y - origin.y;
        let r = self.row_starts.partition_point(|&s| s <= y).checked_sub(1)?;
        let row = self.rows[r];
        let RowKind::Cells { start, len } = row.kind else { return None };
        if y >= row.y + row.h {
            return None;
        }
        let x = p.x - origin.x - self.grid_x;
        if x < 0.0 {
            return None;
        }
        let v = (x / self.cell).floor() as usize;
        if v >= self.cols {
            return None;
        }
        let col = self.visual_col(v);
        (col < len).then_some(start + col)
    }

    /// Seçimi olaysız ayarlar ve görünür kılar. Yerleşimden önce çağrılırsa ilk `set_rect`'te
    /// animasyonsuz görünür kılınır.
    pub fn set_selected(&mut self, i: usize, now: Instant) {
        let _ = self.select(i, now);
        self.want_col = None;
        if let Some(p) = self.cell_origin(self.selected) {
            self.highlight.jump_to(p);
        }
        if self.rect.height <= 0.0 && !self.items.is_empty() {
            self.reveal_pending = true;
        }
    }

    /// Kategori atlaması (sekme çubuğu): bölüm başlığı görünümün tepesine kayar, seçim bölümün
    /// ilk öğesine geçer (olay yok). Boş/geçersiz bölümde `false`.
    pub fn reveal_section(&mut self, s: usize, now: Instant) -> bool {
        let Some(start) = self.section_start(s) else { return false };
        let _ = self.select(start, now);
        self.want_col = None;
        let y = self.rows[self.sections[s].first_row].y;
        self.scroll.scroll_to(y.min(self.scroll.max_offset()), now);
        self.damage.mark();
        true
    }

    fn select(&mut self, i: usize, now: Instant) -> Response<GlyphGridEvent> {
        if self.items.is_empty() {
            return Response::none();
        }
        let i = i.min(self.items.len() - 1);
        let (top, bottom) = self.reveal_span(i);
        self.scroll.ensure_visible(top, bottom, now);
        if i == self.selected {
            return Response::repaint();
        }
        self.selected = i;
        if let Some(p) = self.cell_origin(i) {
            self.highlight.set_target(p, now);
        }
        self.damage.mark();
        Response::event(GlyphGridEvent::Selected(i))
    }

    /// Hücre satırı kadar dikey hareket; sütun yapışkan, kısa satırda son hücreye kırpılır.
    fn move_rows(&mut self, delta: isize) -> Option<usize> {
        let (k, col) = self.locate(self.selected)?;
        let target = (k as isize + delta).clamp(0, self.cell_rows.len() as isize - 1) as usize;
        Some(self.at_row(target, col))
    }

    /// `k` hücre satırında yapışkan sütundaki öğe (kısa satırda son hücre).
    fn at_row(&mut self, k: usize, col: usize) -> usize {
        let want = *self.want_col.get_or_insert(col);
        let (start, len) = self.row_cells(k);
        start + want.min(len.saturating_sub(1))
    }

    /// Sayfa: görünüm yüksekliği kadar (piksel; başlık satırları da sayılır) aşağı/yukarıdaki
    /// hücre satırı — en az bir satır —, görünüm de aynı mesafe kayar: seçim ekrandaki yerinde
    /// kalır, içerik sayfa sayfa akar (uçlarda kırpılır; `select` yine görünür kılar).
    fn move_page(&mut self, down: bool, now: Instant) -> Option<usize> {
        let (k, col) = self.locate(self.selected)?;
        let last = self.cell_rows.len() - 1;
        let row_y = |g: &Self, j: usize| g.rows[g.cell_rows[j]].y;
        let y = row_y(self, k);
        let page = self.rect.height.max(self.cell);
        let target = if down {
            let goal = y + page;
            let j = self.cell_rows.partition_point(|&r| self.rows[r].y <= goal).saturating_sub(1);
            j.max(k + 1).min(last)
        } else {
            let goal = y - page;
            let j = self.cell_rows.partition_point(|&r| self.rows[r].y < goal);
            j.min(k.saturating_sub(1))
        };
        if target != k {
            let delta = row_y(self, target) - y;
            self.scroll.scroll_to(self.scroll.target_offset() + delta, now);
        }
        Some(self.at_row(target, col))
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<GlyphGridEvent> {
        if event.state == KeyState::Released || self.items.is_empty() {
            return Response::none();
        }
        let n = self.items.len();
        let s = self.selected.min(n - 1);
        let ctrl = event.modifiers.ctrl;
        let next = match event.key {
            // Yatay oklar görsel yöndedir: sağdan solda ← sonraki öğe.
            Key::Left | Key::Right => {
                self.want_col = None;
                if (event.key == Key::Right) != self.rtl { (s + 1).min(n - 1) } else { s.saturating_sub(1) }
            }
            Key::Up => self.move_rows(-1).unwrap_or(s),
            Key::Down => self.move_rows(1).unwrap_or(s),
            Key::PageUp => self.move_page(false, now).unwrap_or(s),
            Key::PageDown => self.move_page(true, now).unwrap_or(s),
            Key::Home | Key::End if ctrl => {
                self.want_col = None;
                if event.key == Key::Home { 0 } else { n - 1 }
            }
            Key::Home | Key::End => {
                self.want_col = None;
                let Some((k, _)) = self.locate(s) else { return Response::none() };
                let (start, len) = self.row_cells(k);
                if event.key == Key::Home { start } else { start + len - 1 }
            }
            Key::Enter => return Response::event(GlyphGridEvent::Activated(s)),
            _ => return Response::none(),
        };
        self.select(next, now)
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<GlyphGridEvent> {
        let mut out = self.scroll.pointer(theme, event, now).discard();
        if out.repaint {
            self.damage.mark();
        }
        for ev in self.scroll.forward(event) {
            let r = self.pointer_content(&ev, now);
            if r.event.is_some() {
                return r.with_repaint(out.repaint);
            }
            out.repaint |= r.repaint;
        }
        out
    }

    fn pointer_content(&mut self, event: &PointerEvent, now: Instant) -> Response<GlyphGridEvent> {
        let mut out = Response::none();
        let cell = if event.kind == PointerKind::Leave { None } else { self.cell_at(event.position, now) };
        if cell != self.hovered {
            self.hovered = cell;
            self.damage.mark();
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Press(Button::Left) => {
                if self.pressed != cell {
                    self.damage.mark();
                    out.repaint = true;
                }
                self.pressed = cell;
            }
            PointerKind::Release(Button::Left) => {
                let pressed = self.pressed.take();
                if pressed.is_some() {
                    self.damage.mark();
                    out.repaint = true;
                }
                if let Some(i) = cell.filter(|c| pressed == Some(*c)) {
                    self.want_col = None;
                    let _ = self.select(i, now);
                    return Response::event(GlyphGridEvent::Activated(i));
                }
            }
            PointerKind::Leave => self.pressed = None,
            _ => {}
        }
        out
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        if let Some(c) = self.scroll.cursor(p) {
            return Some(c);
        }
        // Zaman bağımsız: `hovered` son işaretçi olayından (cursor her zaman pointer'dan sonra).
        Some(if self.hovered.is_some() { CursorShape::Pointer } else { CursorShape::Default })
    }

    /// Glifin önbellekteki layout'u; yoksa şekillendirilir. Sığa aşılınca en eski yarısı atılır
    /// (O(n) seçim; ekleme başına tarama yok).
    fn cell_layout(&mut self, text: &mut TextSystem, i: usize, cap: usize) {
        self.tick += 1;
        let tick = self.tick;
        if let Some(c) = self.cache.get_mut(&i) {
            c.used = tick;
            return;
        }
        if self.cache.len() >= cap {
            let mut used: Vec<u64> = self.cache.values().map(|c| c.used).collect();
            let mid = used.len() / 2;
            let (_, &mut cut, _) = used.select_nth_unstable(mid);
            self.cache.retain(|_, c| c.used > cut);
        }
        let layout = text.layout(clip_text(&self.items[i].glyph, MAX_GLYPH_TEXT), &self.glyph_style, None);
        self.shaped += 1;
        self.cache.insert(i, CellCache { layout, used: tick });
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let range = self.scroll.visible_range(&self.row_starts, now);
        // Görünen hücreler önbellekte: sığa en az görünenin iki katı (kaydırırken sürtünmesiz).
        let visible: usize = self.rows[range.clone()].iter().map(|r| if let RowKind::Cells { len, .. } = r.kind { len } else { 0 }).sum();
        let cap = GLYPH_CACHE_CELLS.max(visible * 2);
        let header_w = (self.rect.width - self.grid_x * 2.0 - theme.space(2)).max(0.0);
        for r in range.clone() {
            match self.rows[r].kind {
                RowKind::Cells { start, len } => {
                    for i in start..start + len {
                        self.cell_layout(ui.painter.text_system(), i, cap);
                    }
                }
                RowKind::Header(s) => {
                    if !self.headers.contains_key(&s) {
                        let l = ui.painter.text_system().layout(&self.sections[s].title, &self.header_style, Some(header_w));
                        self.headers.insert(s, l);
                    }
                }
            }
        }
        let cell = self.cell;
        let grid_x = self.grid_x;
        let (rtl, cols, width) = (self.rtl, self.cols, self.rect.width);
        let vcol = move |col: usize| visual_col(col, cols, rtl);
        let gap = FOCUS_RING;
        let radius = theme.shape.radius_medium.min(cell * 0.5 - gap).max(0.0);
        let radii = Corners::uniform(radius);
        let ring_radii = Corners::uniform(radius + gap);
        let high = theme.contrast.is_high();
        let hl = self.highlight.value(now);
        let (selected, hovered, pressed, focus_visible) = (self.selected, self.hovered, self.pressed, self.focus_visible);
        let has_items = !self.items.is_empty();
        let (rows, cache, headers) = (&self.rows, &self.cache, &self.headers);
        let mut drawn = 0;
        self.scroll.draw(ui, |ui, origin, span| {
            let sel = CellColors::new(theme, true, 0.0);
            let tile = |p: Point| Rect::new(p.x, p.y, cell, cell).inset(Insets::uniform(gap));
            // Seçim vurgusu (kayan): glifin altında.
            let sel_tile = tile(Point::new(origin.x + hl.x, origin.y + hl.y));
            let sel_visible = has_items && hl.y + cell > span.start && hl.y < span.end;
            if sel_visible {
                ui.painter.rect(sel_tile, &RectStyle::filled(ui.fade(sel.fill), radii));
            }
            for row in &rows[range] {
                match row.kind {
                    RowKind::Header(s) => {
                        let Some(l) = headers.get(&s) else { continue };
                        let x = if rtl { width - grid_x - theme.space(1) - l.size().width } else { grid_x + theme.space(1) };
                        let at = Point::new(origin.x + x, origin.y + row.y + row.h - l.size().height - theme.space(1));
                        ui.painter.text(l, at, ui.fade(header_color(theme)));
                    }
                    RowKind::Cells { start, len } => {
                        for i in start..start + len {
                            let p = Point::new(origin.x + grid_x + vcol(i - start) as f32 * cell, origin.y + row.y);
                            let t = tile(p);
                            if hovered == Some(i) || pressed == Some(i) {
                                let op = if pressed == Some(i) { STATE_PRESSED } else { STATE_HOVER };
                                let colors = CellColors::new(theme, i == selected, op);
                                ui.painter.rect(t, &RectStyle::filled(ui.fade(colors.layer), radii));
                            }
                            let Some(c) = cache.get(&i) else { continue };
                            let sz = c.layout.size();
                            let at = Point::new((t.center().x - sz.width * 0.5).round(), (t.center().y - sz.height * 0.5).round());
                            ui.painter.text(&c.layout, at, ui.fade(sel.glyph));
                            drawn += 1;
                        }
                    }
                }
            }
            // Halka en üstte: klavye odağında kalın; yüksek karşıtlıkta seçim her zaman çerçeveli.
            if sel_visible && (focus_visible || high) {
                let width = if focus_visible { FOCUS_RING } else { HAIRLINE };
                let ring = sel_tile.expand(gap);
                let style = RectStyle { fill: Color::TRANSPARENT, radii: ring_radii, border: Some(Border { width, color: ui.fade(sel.ring) }) };
                ui.painter.rect(ring, &style);
            }
        });
        self.drawn_cells = drawn;
        let animating = !self.highlight.is_settled(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.rect, animating);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;
    use crate::input::Modifiers;
    use nuxut_render::{DrawKind, Recorder};

    const W: f32 = 440.0; // 10 sütun × 44
    const H: f32 = 220.0; // 5 satır

    fn items(n: usize) -> Vec<GlyphItem> {
        (0..n).map(|i| GlyphItem::new(char::from_u32(0x1F600 + (i as u32 % 80)).unwrap().to_string(), format!("yüz {i}"))).collect()
    }

    fn grid(n: usize) -> (GlyphGrid, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = GlyphGrid::new(&theme);
        g.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        g.set_items(&theme, items(n), t0);
        (g, theme, t0)
    }

    fn sel(g: &mut GlyphGrid, k: Key, t: Instant) -> Option<GlyphGridEvent> {
        g.key(&key(k), t).event
    }

    #[test]
    fn columns_from_width_or_fixed() {
        let (mut g, theme, t0) = grid(25);
        assert_eq!(GlyphGrid::cell_size(&theme), 44.0);
        assert_eq!(g.columns(), 10);
        g.set_rect(&theme, Rect::new(0.0, 0.0, 470.0, H), t0);
        assert_eq!(g.columns(), 10, "artan yer ortalanır");
        assert_eq!(g.grid_x, 15.0);
        g.set_rect(&theme, Rect::new(0.0, 0.0, 10.0, H), t0);
        assert_eq!(g.columns(), 1, "en az bir sütun");
        g.set_columns(&theme, GlyphColumns::Fixed(5), t0);
        g.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, H), t0);
        assert_eq!((g.columns(), g.cell), (5, 60.0));
        g.set_columns(&theme, GlyphColumns::Fixed(0), t0);
        assert_eq!(g.columns(), 1);
    }

    #[test]
    fn arrow_navigation_respects_bounds_and_short_last_row() {
        // 25 öğe, 10 sütun: satırlar 10, 10, 5.
        let (mut g, _, t0) = grid(25);
        assert_eq!(sel(&mut g, Key::Left, t0), None, "başta sol: olay yok");
        assert_eq!(g.selected(), Some(0));
        assert_eq!(sel(&mut g, Key::Up, t0), None, "ilk satırda yukarı: yerinde");
        assert_eq!(sel(&mut g, Key::Right, t0), Some(GlyphGridEvent::Selected(1)));
        assert_eq!(sel(&mut g, Key::Down, t0), Some(GlyphGridEvent::Selected(11)));
        // Sütun 8 → eksik son satırda son hücreye (24) kırpılır; geri çıkınca sütun 8'e döner.
        g.set_selected(18, t0);
        assert_eq!(sel(&mut g, Key::Down, t0), Some(GlyphGridEvent::Selected(24)));
        assert_eq!(sel(&mut g, Key::Down, t0), None, "son satırda aşağı: yerinde");
        assert_eq!(sel(&mut g, Key::Up, t0), Some(GlyphGridEvent::Selected(18)), "yapışkan sütun");
        // Satır sonundan sağ: sonraki satırın başı; Home/End satır içinde.
        g.set_selected(9, t0);
        assert_eq!(sel(&mut g, Key::Right, t0), Some(GlyphGridEvent::Selected(10)));
        assert_eq!(sel(&mut g, Key::End, t0), Some(GlyphGridEvent::Selected(19)));
        assert_eq!(sel(&mut g, Key::Home, t0), Some(GlyphGridEvent::Selected(10)));
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        assert_eq!(g.key(&key_mod(Key::End, ctrl), t0).event, Some(GlyphGridEvent::Selected(24)));
        assert_eq!(sel(&mut g, Key::Right, t0), None, "sonda sağ: yerinde");
        assert_eq!(g.key(&key_mod(Key::Home, ctrl), t0).event, Some(GlyphGridEvent::Selected(0)));
        assert_eq!(sel(&mut g, Key::Enter, t0), Some(GlyphGridEvent::Activated(0)));
        assert_eq!(sel(&mut g, Key::Char('a'), t0), None);
    }

    #[test]
    fn page_keys_move_by_visible_rows() {
        let (mut g, _, t0) = grid(1000);
        // 5 görünen satır = 50 öğe.
        assert_eq!(sel(&mut g, Key::PageDown, t0), Some(GlyphGridEvent::Selected(50)));
        assert_eq!(sel(&mut g, Key::PageUp, t0), Some(GlyphGridEvent::Selected(0)));
        g.set_selected(997, t0);
        assert_eq!(sel(&mut g, Key::PageDown, t0), None, "son satırda");
        assert_eq!(sel(&mut g, Key::PageUp, t0), Some(GlyphGridEvent::Selected(947)));
    }

    /// Başlıklı bölümlerde sayfa piksel cinsindendir: seçim en fazla bir görünüm yüksekliği
    /// ilerler (başlıklar dahil) ve görünüm aynı mesafe kayar — seçim ekranda yerinde kalır.
    #[test]
    fn page_keys_page_by_viewport_across_headers() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let later = t0 + std::time::Duration::from_secs(5);
        let mut g = GlyphGrid::new(&theme);
        g.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        // 25 öğe = 3 satır; her bölümün başlığı var.
        let secs = (0..20).map(|s| GlyphSection::new(format!("B{s}"), items(25))).collect();
        g.set_sections(&theme, secs, t0);
        let y_of = |g: &GlyphGrid, i: usize| g.rows[g.cell_rows[g.locate(i).unwrap().0]].y;
        let mut i = 0;
        let mut pages = 0;
        while let Some(GlyphGridEvent::Selected(j)) = sel(&mut g, Key::PageDown, t0) {
            let dy = y_of(&g, j) - y_of(&g, i);
            assert!(dy > 0.0 && dy <= H, "sayfa görünümü aşmaz: {dy}");
            let last = g.locate(j).unwrap().0 == g.cell_rows.len() - 1;
            assert!(last || dy > H - GlyphGrid::cell_size(&theme) - GlyphGrid::header_height(&theme), "sayfa dolu: {dy}");
            let screen = y_of(&g, j) - g.scroll.offset(later);
            assert!((0.0..=H - GlyphGrid::cell_size(&theme) + 0.5).contains(&screen), "seçim görünür: {screen}");
            i = j;
            pages += 1;
            assert!(pages < 100);
        }
        assert_eq!(g.locate(i).unwrap().0, g.cell_rows.len() - 1, "son satıra varılır");
        // Ortada: seçim ekrandaki yerini korur.
        g.set_selected(250, later);
        let o0 = g.scroll.offset(later + std::time::Duration::from_secs(5));
        let s0 = y_of(&g, 250) - o0;
        let t1 = later + std::time::Duration::from_secs(10);
        let Some(GlyphGridEvent::Selected(j)) = sel(&mut g, Key::PageDown, t1) else { panic!() };
        let s1 = y_of(&g, j) - g.scroll.offset(t1 + std::time::Duration::from_secs(5));
        assert!((s1 - s0).abs() < 0.5, "ekran konumu {s0} → {s1}");
        let Some(GlyphGridEvent::Selected(k)) = sel(&mut g, Key::PageUp, t1) else { panic!() };
        assert!(k <= 250 && y_of(&g, j) - y_of(&g, k) <= H, "geri bir sayfa: {k}");
    }

    /// Sağdan sola: sütun 0 sağda, kısa satır sağa yaslı; ← sonraki, → önceki; tıklama aynalı
    /// sütunu bulur; ↑↓ ve Home/End mantıksal.
    #[test]
    fn rtl_mirrors_columns_and_horizontal_arrows() {
        let (mut g, theme, t0) = grid(25);
        let cell = GlyphGrid::cell_size(&theme);
        g.set_rtl(true);
        assert!(g.is_rtl());
        assert_eq!(g.cell_origin(0).unwrap().x, g.grid_x + 9.0 * cell, "ilk öğe sağda");
        assert_eq!(g.cell_origin(9).unwrap().x, g.grid_x, "satır sonu solda");
        assert_eq!(g.cell_origin(24).unwrap().x, g.grid_x + 5.0 * cell, "kısa satır sağa yaslı");
        assert_eq!(sel(&mut g, Key::Right, t0), None, "başta → (önceki): yerinde");
        assert_eq!(sel(&mut g, Key::Left, t0), Some(GlyphGridEvent::Selected(1)));
        assert_eq!(sel(&mut g, Key::Down, t0), Some(GlyphGridEvent::Selected(11)));
        assert_eq!(sel(&mut g, Key::Right, t0), Some(GlyphGridEvent::Selected(10)));
        assert_eq!(sel(&mut g, Key::End, t0), Some(GlyphGridEvent::Selected(19)));
        // İşaretçi: en sağ hücre öğe 0, en sol hücre öğe 9; kısa satırın solu boş.
        let at = |g: &GlyphGrid, col: f32, row: f32| g.cell_at(Point::new(g.grid_x + col * cell + 2.0, row * cell + 2.0), t0);
        assert_eq!(at(&g, 9.0, 0.0), Some(0));
        assert_eq!(at(&g, 0.0, 0.0), Some(9));
        assert_eq!(at(&g, 5.0, 2.0), Some(24));
        assert_eq!(at(&g, 4.0, 2.0), None);
        // Vurgu yeni yerine atlar; geri LTR.
        g.set_rtl(false);
        assert_eq!(g.cell_origin(0).unwrap().x, g.grid_x);
        assert_eq!(g.highlight.value(t0), g.cell_origin(19).unwrap());
        assert_eq!(sel(&mut g, Key::Right, t0), Some(GlyphGridEvent::Selected(20)));
        // Çizim (RTL) panik/boşta kare yok.
        g.set_rtl(true);
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        rec.record(&mut text, (500, 300), 1.0, |p| {
            let mut ui = Ui::new(p, &theme, t0 + std::time::Duration::from_secs(5));
            g.draw(&mut ui);
            assert!(!ui.wants_frame());
        });
        assert!(rec.draws().iter().any(|d| d.kind == DrawKind::Glyphs));
    }

    #[test]
    fn empty_grid_is_inert() {
        let (mut g, theme, t0) = grid(0);
        assert_eq!(g.selected(), None);
        assert_eq!(g.key(&key(Key::Down), t0), Response::none());
        assert_eq!(g.key(&key(Key::Enter), t0), Response::none());
        g.set_selected(5, t0);
        assert!(!g.reveal_section(0, t0));
        assert_eq!(g.pointer(&theme, &press(10.0, 10.0), t0).event, None);
        let t0 = t0 + std::time::Duration::from_secs(5);
        let mut rec = Recorder::new();
        let mut text = TextSystem::new();
        rec.record(&mut text, (500, 300), 1.0, |p| {
            let mut ui = Ui::new(p, &theme, t0);
            g.draw(&mut ui);
            assert!(!ui.wants_frame());
        });
        assert!(rec.draws().iter().all(|d| d.kind != DrawKind::Glyphs));
    }

    #[test]
    fn sections_start_new_rows_and_navigation_crosses_them() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = GlyphGrid::new(&theme);
        g.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        let secs = vec![
            GlyphSection::new("Yüzler", items(12)),
            GlyphSection::new("Boş", vec![]),
            GlyphSection::new("", items(3)),
            GlyphSection::new("Hayvanlar", items(4)),
        ];
        g.set_sections(&theme, secs, t0);
        assert_eq!(g.len(), 19);
        // Satırlar: başlık, 10, 2 | (başlıksız) 3 | başlık, 4.
        let kinds: Vec<_> = g.rows.iter().map(|r| r.kind).collect();
        assert_eq!(kinds, vec![
            RowKind::Header(0),
            RowKind::Cells { start: 0, len: 10 },
            RowKind::Cells { start: 10, len: 2 },
            RowKind::Cells { start: 12, len: 3 },
            RowKind::Header(3),
            RowKind::Cells { start: 15, len: 4 },
        ]);
        let hh = GlyphGrid::header_height(&theme);
        assert_eq!(g.rows[5].y, hh * 2.0 + 44.0 * 3.0);
        assert_eq!((g.section_of(11), g.section_of(12), g.section_of(18), g.section_of(19)), (Some(0), Some(2), Some(3), None));
        assert_eq!((g.section_start(1), g.section_start(3)), (None, Some(15)));
        // Sütun 7 → 2 öğelik satır (11) → 3 öğelik bölüm (14) → 4 öğelik bölüm (18) → geri 7.
        g.set_selected(7, t0);
        for want in [11, 14, 18] {
            assert_eq!(sel(&mut g, Key::Down, t0), Some(GlyphGridEvent::Selected(want)));
        }
        for want in [14, 11, 7] {
            assert_eq!(sel(&mut g, Key::Up, t0), Some(GlyphGridEvent::Selected(want)));
        }
        assert_eq!(g.label(15), Some("yüz 0"));
        assert!(g.reveal_section(3, t0));
        assert_eq!(g.selected(), Some(15));
    }

    #[test]
    fn selection_scrolls_into_view_with_its_header() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let later = t0 + std::time::Duration::from_secs(5);
        let mut g = GlyphGrid::new(&theme);
        g.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        g.set_sections(&theme, vec![GlyphSection::new("A", items(200)), GlyphSection::new("B", items(200))], t0);
        let visible = |g: &GlyphGrid, i: usize, t: Instant| {
            let (top, bottom) = g.reveal_span(i);
            let o = g.scroll.offset(t);
            top >= o - 0.5 && bottom <= o + H + 0.5
        };
        let _ = g.key(&key_mod(Key::End, Modifiers { ctrl: true, ..Modifiers::default() }), t0);
        assert!(visible(&g, 399, later), "son öğe görünür");
        assert!(g.scroll.offset(later) > 0.0);
        // Yukarı: B'nin ilk satırına gelince başlığı da görünür.
        g.set_selected(205, t0);
        let (top, _) = g.reveal_span(205);
        assert_eq!(top, g.rows[g.sections[1].first_row].y, "ilk satırda başlık dahil");
        assert!(visible(&g, 205, later));
        for _ in 0..30 {
            let _ = g.key(&key(Key::Down), later);
        }
        assert!(visible(&g, g.selected().unwrap(), later + std::time::Duration::from_secs(5)));
        // Yerleşimden önce verilen seçim ilk set_rect'te animasyonsuz görünür kılınır.
        let mut h = GlyphGrid::new(&theme);
        h.set_items(&theme, items(500), t0);
        h.set_selected(450, t0);
        h.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        assert!(visible(&h, 450, t0), "aynı anda görünür");
    }

    #[test]
    fn replace_sections_keeps_scroll_and_takes_selection() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let s = std::time::Duration::from_secs(5);
        let mut g = GlyphGrid::new(&theme);
        g.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        g.set_sections(&theme, vec![GlyphSection::new("A", items(200)), GlyphSection::new("B", items(200))], t0);
        g.scroll.scroll_to(1000.0, t0);
        let off = g.scroll.offset(t0 + s);
        assert_eq!(off, 1000.0);
        // Oturmuş konum: aynı kalır; seçim olaysız, kaydırmasız (görünüm dışında da olsa) geçer.
        g.replace_sections(&theme, vec![GlyphSection::new("A", items(200)), GlyphSection::new("B", items(200))], 3, t0 + s);
        assert_eq!(g.selected(), Some(3));
        assert_eq!(g.scroll.offset(t0 + s * 2), off);
        assert_eq!(g.highlight.value(t0 + s), g.cell_origin(3).unwrap(), "vurgu atlar");
        // Süren hareket kesilmez: hedefe sürer.
        let t1 = t0 + s * 3;
        g.scroll.scroll_to(1400.0, t1);
        let mid = t1 + std::time::Duration::from_millis(40);
        let before = g.scroll.offset(mid);
        g.replace_sections(&theme, vec![GlyphSection::new("A", items(200)), GlyphSection::new("B", items(200))], 250, mid);
        assert_eq!(g.scroll.offset(mid), before, "konum sıçramaz");
        assert_eq!(g.scroll.offset(mid + s), 1400.0);
        // İçerik kısalırsa konum yeni sınıra iner; taşan seçim son öğeye kırpılır.
        g.replace_sections(&theme, vec![GlyphSection::new("A", items(30))], 999, mid + s);
        assert_eq!(g.selected(), Some(29));
        assert!((g.scroll.offset(mid + s * 2) - g.scroll.max_offset()).abs() < 0.5);
        // `set_sections` ise başa döner.
        g.set_sections(&theme, vec![GlyphSection::new("A", items(200))], mid + s * 2);
        assert_eq!(g.selected(), Some(0));
        assert!(g.scroll.offset(mid + s * 3).abs() < 0.5);
        // Yerleşimden önce: seçim ilk set_rect'te görünür kılınır.
        let mut h = GlyphGrid::new(&theme);
        h.replace_sections(&theme, vec![GlyphSection::new("", items(500))], 450, t0);
        h.set_rect(&theme, Rect::new(0.0, 0.0, W, H), t0);
        let (top, bottom) = h.reveal_span(450);
        let o = h.scroll.offset(t0);
        assert!(top >= o - 0.5 && bottom <= o + H + 0.5);
        // Boş veri: seçim yok, panik yok.
        h.replace_sections(&theme, Vec::new(), 7, t0);
        assert_eq!(h.selected(), None);
    }

    #[test]
    fn pointer_hover_click_and_cursor() {
        let (mut g, theme, t0) = grid(25);
        assert!(g.pointer(&theme, &motion(50.0, 50.0), t0).repaint);
        assert_eq!(g.hovered, Some(11));
        assert_eq!(g.cursor(Point::new(50.0, 50.0)), Some(CursorShape::Pointer));
        let _ = g.pointer(&theme, &press(50.0, 50.0), t0);
        assert_eq!(g.pointer(&theme, &release(50.0, 50.0), t0).event, Some(GlyphGridEvent::Activated(11)));
        assert_eq!(g.selected(), Some(11));
        // Basıp başka hücrede bırakmak etkinleştirmez; son satırın boş kısmı hücre değil.
        let _ = g.pointer(&theme, &press(10.0, 10.0), t0);
        assert_eq!(g.pointer(&theme, &release(100.0, 10.0), t0).event, None);
        assert!(g.pointer(&theme, &motion(400.0, 100.0), t0).repaint);
        assert_eq!(g.hovered, None, "son satır 5 hücre");
        assert_eq!(g.cursor(Point::new(400.0, 100.0)), Some(CursorShape::Default));
        assert_eq!(g.cursor(Point::new(-5.0, 10.0)), None);
    }

    fn glyph_draws(g: &mut GlyphGrid, theme: &Theme, text: &mut TextSystem, rec: &mut Recorder, t: Instant) -> (usize, bool) {
        let mut wants = false;
        rec.record(text, (500, 300), 1.0, |p| {
            let mut ui = Ui::new(p, theme, t);
            g.draw(&mut ui);
            wants = ui.wants_frame();
        });
        (g.drawn_cells, wants)
    }

    #[test]
    fn only_visible_rows_are_shaped_and_drawn() {
        let (mut g, theme, t0) = grid(4000);
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        let (drawn, wants) = glyph_draws(&mut g, &theme, &mut text, &mut rec, t0);
        // 5 tam satır görünür (220 / 44); kenar satırı yarım görünse de 6 satırı geçmez.
        assert!((50..=60).contains(&drawn), "çizilen glif {drawn}");
        assert!(g.shaped_cells() <= 60, "şekillendirilen {}", g.shaped_cells());
        assert!(!wants, "boşta kare yok");
        // Aynı kare tekrar: önbellek isabeti, yeniden şekillendirme yok.
        let before = g.shaped_cells();
        let _ = glyph_draws(&mut g, &theme, &mut text, &mut rec, t0);
        assert_eq!(g.shaped_cells(), before);
        // Sona atla: yalnız oradaki satırlar şekillenir; önbellek sığada kalır.
        g.set_selected(3999, t0);
        let later = t0 + std::time::Duration::from_secs(5);
        let (drawn, _) = glyph_draws(&mut g, &theme, &mut text, &mut rec, later);
        assert!(drawn <= 60 && drawn > 0);
        assert!(g.shaped_cells() <= before + 60);
        // Baştan sona sayfa sayfa kaydır: her konumda yalnız görünenler; önbellek sığada kalır.
        let mut t = later;
        let max = g.scroll.max_offset();
        let mut off = 0.0;
        while off <= max {
            g.scroll.scroll_to(off, t);
            t += std::time::Duration::from_secs(5);
            let (drawn, _) = glyph_draws(&mut g, &theme, &mut text, &mut rec, t);
            assert!(drawn <= 60, "{off}: {drawn}");
            assert!(g.cache.len() <= GLYPH_CACHE_CELLS, "önbellek sınırlı: {}", g.cache.len());
            off += 44.0 * 5.0;
        }
        assert!(g.shaped_cells() >= 3900, "her glif bir kez görüldü: {}", g.shaped_cells());
    }

    #[test]
    fn highlight_settles_and_stops_frames() {
        let (mut g, theme, t0) = grid(100);
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        let _ = g.key(&key(Key::Down), t0);
        let (_, wants) = glyph_draws(&mut g, &theme, &mut text, &mut rec, t0);
        assert!(wants, "vurgu kayıyor");
        let later = t0 + std::time::Duration::from_secs(5);
        let (_, wants) = glyph_draws(&mut g, &theme, &mut text, &mut rec, later);
        assert!(!wants, "oturunca kare yok");
        assert_eq!(g.highlight.value(later), Point::new(0.0, 44.0));
    }

    #[test]
    fn input_is_bounded_and_debug_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = GlyphGrid::new(&theme);
        let big = vec![GlyphItem::new("x", ""); MAX_GLYPH_ITEMS + 10];
        g.set_sections(&theme, vec![GlyphSection::new("a".repeat(1000), big), GlyphSection::new("b", items(3))], t0);
        assert_eq!(g.len(), MAX_GLYPH_ITEMS);
        assert_eq!(g.sections[0].title.len(), MAX_GLYPH_LABEL);
        assert_eq!(g.section_start(1), None, "taşan bölüm boş");
        let it = GlyphItem::new(SECRET, SECRET);
        assert_redacted(&it);
        assert_redacted(&GlyphSection::new(SECRET, vec![it.clone()]));
        g.set_items(&theme, vec![it], t0);
        assert_redacted(&g);
    }
}
