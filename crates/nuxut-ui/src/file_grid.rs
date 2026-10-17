//! Dosya ızgarası ([`FileGrid`]): dosya seçici / dosya yöneticisi simge görünümü. Tasarım:
//! `docs/design/file-grid.md`.
//!
//! - **Hücre:** üstte ikon kutusu (küçük resim varsa o, yüklenene kadar ikon), altta en çok iki
//!   satır ortalı etiket (`…` ile kırpılır). Sütun sayısı genişlikten; hücreler genişliği eşit
//!   paylaşır.
//! - **Sanallaştırma:** satırlar eşit yükseklikte; yalnız görünen hücreler şekillenir ve çizilir
//!   (etiket + ikon LRU önbelleği). [`FileGrid::visible_range`] küçük resim isteği içindir.
//! - **Seçim:** [`Selection`] kuralları (tekli/çoklu; Ctrl/Shift tık, Shift+ok, Ctrl+A,
//!   Ctrl+Space). Boş alana tık seçimi boşaltır.
//! - **Klavye:** oklar, PgUp/PgDn, Home/End (ilk/son öğe), Enter. Çift tık ve Enter `Activated`.
//! - **GlyphGrid ile ortaklık:** stil (`CellColors`: seçili kap, hover katmanı, odak halkası) ve
//!   `ScrollView` ortak. Yerleşim ve gezinme ayrı: GlyphGrid'in bölüm başlıklı değişken satır
//!   tablosu burada gereksiz (satırlar eşit, indis aritmetiği yeter), çoklu seçim de GlyphGrid'de
//!   yok. Ortak satır tablosu soyutlaması iki tarafı da karmaşıklaştırırdı.
//! - Adlar kullanıcı içeriği: `Debug` yalnız sayı yazar (değişmez 9).

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;
use std::time::Instant;

use nuxut_core::{Color, Corners, Insets, Point, Rect};
use nuxut_render::{Border, ImageFit, ImageStyle, RectStyle, TextAlign, TextLayout, TextStyle, TextSystem, Texture};
use nuxut_theme::{STATE_HOVER, STATE_PRESSED, Theme};

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};
use crate::glyph_grid::CellColors;
use crate::icon::{Icon, IconView};
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, Modifiers, PointerEvent, PointerKind};
use crate::interaction::DOUBLE_CLICK;
use crate::scroll::{ScrollView, visible_range_uniform};
use crate::selection::Selection;
use crate::style::{FOCUS_RING, HAIRLINE, clip_text};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// En fazla öğe (çok büyük dizinler; bit kümesi ~128 KB).
pub const MAX_FILE_ITEMS: usize = 1 << 20;
/// Görünen adın en uzun hâli (bayt; NAME_MAX 255 + görünen ad payı).
pub const MAX_FILE_LABEL: usize = 1024;
/// Hücre önbelleğinin taban sığası (görünen hücre sayısının iki katından az olmaz).
pub const FILE_CACHE_CELLS: usize = 256;
/// Etiket satırı sınırı.
pub const FILE_LABEL_LINES: usize = 2;

/// Tek dosya/klasör: görünen ad + ikon (tür ikonu; tema ikonu dokusu ya da `IconName`).
#[derive(Clone, PartialEq)]
pub struct FileItem {
    pub label: String,
    pub icon: Icon,
}

impl FileItem {
    pub fn new(label: impl Into<String>, icon: Icon) -> Self {
        Self { label: label.into(), icon }
    }
}

impl std::fmt::Debug for FileItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileItem").field("label", &crate::redact::Len(&self.label)).field("icon", &self.icon).finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileGridEvent {
    /// Seçim kümesi değişti (`selection()`); imleç `cursor()`.
    SelectionChanged,
    /// Çift tık ya da Enter: imleçteki öğe (çoklu seçimde uygulama `selection()`'a bakar).
    Activated(usize),
}

#[derive(Clone)]
struct CellCache {
    label: TextLayout,
    icon: IconView,
    used: u64,
}

#[derive(Clone)]
pub struct FileGrid {
    items: Vec<FileItem>,
    thumbs: HashMap<usize, Texture>,
    selection: Selection,
    cols: usize,
    cell_w: f32,
    cell_h: f32,
    scroll: ScrollView,
    rect: Rect,
    hovered: Option<usize>,
    pressed: Option<usize>,
    /// Son tık: öğe, an, konum (çift tık).
    last_click: Option<(usize, Instant, Point)>,
    focus_visible: bool,
    reveal_pending: bool,
    cache: HashMap<usize, CellCache>,
    tick: u64,
    label_style: TextStyle,
    shaped: u64,
    drawn_cells: usize,
    /// Son karede GPU yüklemesi bekleyen küçük resim/ikon vardı.
    textures_pending: bool,
    damage: DamageState,
}

impl std::fmt::Debug for FileGrid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileGrid")
            .field("items", &crate::redact::Count(self.items.len()))
            .field("selected", &self.selection.count())
            .field("cursor", &self.selection.cursor())
            .field("cols", &self.cols)
            .field("rect", &self.rect)
            .finish_non_exhaustive()
    }
}

impl FileGrid {
    /// `multiple`: çoklu seçim (portal `multiple`).
    pub fn new(theme: &Theme, multiple: bool) -> Self {
        let label_style = Self::label_style(theme);
        Self {
            items: Vec::new(),
            thumbs: HashMap::new(),
            selection: Selection::new(multiple),
            cols: 1,
            cell_w: Self::min_cell_width(theme),
            cell_h: Self::cell_height(theme, &label_style),
            scroll: ScrollView::new(theme),
            rect: Rect::ZERO,
            hovered: None,
            pressed: None,
            last_click: None,
            focus_visible: false,
            reveal_pending: false,
            cache: HashMap::new(),
            tick: 0,
            label_style,
            shaped: 0,
            drawn_cells: 0,
            textures_pending: false,
            damage: DamageState::new(),
        }
    }

    // ── Ölçüler (tema) ──────────────────────────────────────────────────────────

    /// Hücrenin en dar hâli; artan genişlik sütunlara eşit dağılır.
    pub fn min_cell_width(theme: &Theme) -> f32 {
        theme.space(28)
    }

    /// İkon/küçük resim kutusu kenarı.
    pub fn thumbnail_size(theme: &Theme) -> f32 {
        theme.space(16)
    }

    /// Tür ikonunun boyu (kutunun içinde ortalı).
    pub fn icon_size(theme: &Theme) -> f32 {
        theme.space(12)
    }

    fn pad(theme: &Theme) -> f32 {
        theme.space(2)
    }

    fn label_style(theme: &Theme) -> TextStyle {
        text_style(theme, TextRole::Body).with_max_lines(FILE_LABEL_LINES).with_align(TextAlign::Center)
    }

    fn line_height(style: &TextStyle) -> f32 {
        (style.size * style.line_height).ceil()
    }

    fn cell_height(theme: &Theme, style: &TextStyle) -> f32 {
        let pad = Self::pad(theme);
        (pad + Self::thumbnail_size(theme) + theme.space(1) + Self::line_height(style) * FILE_LABEL_LINES as f32 + pad).ceil()
    }

    // ── Veri ────────────────────────────────────────────────────────────────────

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn item(&self, i: usize) -> Option<&FileItem> {
        self.items.get(i)
    }

    /// Yeni dizin içeriği: seçim boşalır, imleç ve kaydırma başa, önbellek ve küçük resimler
    /// düşer. [`MAX_FILE_ITEMS`]'ı aşan öğeler atılır.
    pub fn set_items(&mut self, items: Vec<FileItem>, now: Instant) {
        self.items = items;
        self.items.truncate(MAX_FILE_ITEMS);
        for it in &mut self.items {
            let keep = clip_text(&it.label, MAX_FILE_LABEL).len();
            it.label.truncate(keep);
        }
        self.thumbs.clear();
        self.cache.clear();
        self.selection.reset(self.items.len());
        self.hovered = None;
        self.pressed = None;
        self.last_click = None;
        self.reveal_pending = false;
        self.relayout(now);
        self.scroll.scroll_to(0.0, now);
        self.damage.mark();
    }

    /// Tür ikonunu değiştirir (ör. tema ikonu sonradan yüklendi).
    pub fn set_icon(&mut self, i: usize, icon: Icon) {
        if let Some(it) = self.items.get_mut(i)
            && it.icon != icon
        {
            if let Some(c) = self.cache.get_mut(&i) {
                c.icon.set_icon(icon.clone());
            }
            it.icon = icon;
            self.damage.mark();
        }
    }

    /// Küçük resmi verir (`None`: düşer, ikon görünür). Yüklenene dek ikon çizilir.
    pub fn set_thumbnail(&mut self, i: usize, texture: Option<Texture>) {
        if i >= self.items.len() {
            return;
        }
        match texture {
            Some(t) => {
                self.thumbs.insert(i, t);
            }
            None => {
                if self.thumbs.remove(&i).is_none() {
                    return;
                }
            }
        }
        self.damage.mark();
    }

    pub fn has_thumbnail(&self, i: usize) -> bool {
        self.thumbs.contains_key(&i)
    }

    // ── Seçim ───────────────────────────────────────────────────────────────────

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// İmleç (klavye odağındaki öğe); boşsa `None`.
    pub fn cursor(&self) -> Option<usize> {
        (!self.items.is_empty()).then_some(self.selection.cursor())
    }

    pub fn is_multiple(&self) -> bool {
        self.selection.is_multiple()
    }

    pub fn set_multiple(&mut self, multiple: bool) {
        if self.selection.set_multiple(multiple) {
            self.damage.mark();
        }
    }

    /// Olaysız: yalnız `i` seçili, imleç orada ve görünür (yerleşimden önceyse ilk `set_rect`'te).
    pub fn select_only(&mut self, i: usize, now: Instant) {
        if self.selection.select_only(i) {
            self.damage.mark();
        }
        self.reveal_cursor(now);
        if self.rect.height <= 0.0 && !self.items.is_empty() {
            self.reveal_pending = true;
        }
    }

    /// Olaysız: seçimi boşaltır.
    pub fn clear_selection(&mut self) {
        if self.selection.clear() {
            self.damage.mark();
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        if self.focus_visible == on {
            return false;
        }
        self.focus_visible = on;
        self.damage.mark();
        true
    }

    // ── Yerleşim ────────────────────────────────────────────────────────────────

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.scroll.set_theme(theme, now);
        self.label_style = Self::label_style(theme);
        self.cell_h = Self::cell_height(theme, &self.label_style);
        self.cache.clear();
        self.relayout_theme(theme, now);
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        if self.rect != rect {
            if self.rect.width != rect.width {
                // Etiket genişliği hücreye bağlı.
                self.cache.clear();
            }
            self.rect = rect;
            self.relayout_theme(theme, now);
        }
        if self.reveal_pending && rect.height > 0.0 && !self.items.is_empty() {
            self.reveal_pending = false;
            let (top, bottom) = self.row_span(self.selection.cursor());
            self.scroll.ensure_visible_now(top, bottom, now);
        }
    }

    fn relayout_theme(&mut self, theme: &Theme, now: Instant) {
        let min = Self::min_cell_width(theme);
        let w = self.rect.width.max(0.0);
        self.cols = ((w / min).floor() as usize).max(1);
        self.cell_w = (w / self.cols as f32).floor().max(1.0);
        self.relayout(now);
    }

    fn relayout(&mut self, now: Instant) {
        self.scroll.set_geometry(self.rect, self.rows() as f32 * self.cell_h, now);
        self.damage.mark();
    }

    /// Geçerli sütun sayısı.
    pub fn columns(&self) -> usize {
        self.cols
    }

    fn rows(&self) -> usize {
        self.items.len().div_ceil(self.cols)
    }

    fn row_span(&self, i: usize) -> (f32, f32) {
        let top = (i / self.cols) as f32 * self.cell_h;
        (top, top + self.cell_h)
    }

    /// Hücrenin ekran dikdörtgeni (kaydırma dahil).
    fn cell_rect(&self, i: usize, now: Instant) -> Rect {
        let o = self.scroll.content_origin(now);
        Rect::new(o.x + (i % self.cols) as f32 * self.cell_w, o.y + (i / self.cols) as f32 * self.cell_h, self.cell_w, self.cell_h)
    }

    /// Görünen öğe aralığı (kısmen görünen satırlar dahil). Küçük resim isteği için: uygulama
    /// her kareden/kaydırmadan sonra bakar, aralıktaki eksik küçük resimleri ister.
    pub fn visible_range(&self, now: Instant) -> Range<usize> {
        let rows = visible_range_uniform(self.scroll.visible_span(now), self.cell_h, self.rows());
        (rows.start * self.cols).min(self.items.len())..(rows.end * self.cols).min(self.items.len())
    }

    fn cell_at(&self, p: Point, now: Instant) -> Option<usize> {
        if !self.rect.contains(p) {
            return None;
        }
        let o = self.scroll.content_origin(now);
        let (x, y) = (p.x - o.x, p.y - o.y);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let col = (x / self.cell_w).floor() as usize;
        let row = (y / self.cell_h).floor() as usize;
        let i = row * self.cols + col;
        (col < self.cols && i < self.items.len()).then_some(i)
    }

    fn reveal_cursor(&mut self, now: Instant) {
        if self.items.is_empty() {
            return;
        }
        let (top, bottom) = self.row_span(self.selection.cursor());
        self.scroll.ensure_visible(top, bottom, now);
    }

    // ── Girdi ───────────────────────────────────────────────────────────────────

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<FileGridEvent> {
        if event.state == KeyState::Released || self.items.is_empty() {
            return Response::none();
        }
        let n = self.items.len();
        let cols = self.cols;
        let cur = self.selection.cursor().min(n - 1);
        let page = ((self.rect.height / self.cell_h).floor() as usize).max(1) * cols;
        let m = event.modifiers;
        let target = match event.key {
            Key::Left => cur.saturating_sub(1),
            Key::Right => (cur + 1).min(n - 1),
            // Son satır kısaysa aşağı ok son öğeye iner (dosya yöneticilerinde ortak).
            Key::Up => cur.checked_sub(cols).unwrap_or(cur),
            Key::Down if cur / cols + 1 < self.rows() => (cur + cols).min(n - 1),
            Key::Down => cur,
            Key::PageUp => cur.saturating_sub(page),
            Key::PageDown => (cur + page).min(n - 1),
            Key::Home => 0,
            Key::End => n - 1,
            Key::Enter => return Response::event(FileGridEvent::Activated(cur)),
            Key::Char('a' | 'A') if m.ctrl => {
                return if self.selection.select_all() {
                    self.damage.mark();
                    Response::event(FileGridEvent::SelectionChanged)
                } else {
                    Response::none()
                };
            }
            Key::Space if m.ctrl => {
                self.focus_visible = true;
                self.damage.mark();
                return if self.selection.toggle_cursor() { Response::event(FileGridEvent::SelectionChanged) } else { Response::repaint() };
            }
            _ => return Response::none(),
        };
        self.focus_visible = true;
        let moved = target != cur;
        let changed = self.selection.navigate(target, m);
        self.reveal_cursor(now);
        self.damage.mark();
        if changed {
            Response::event(FileGridEvent::SelectionChanged)
        } else {
            Response::none().with_repaint(moved)
        }
    }

    /// `modifiers`: güncel değiştiriciler (`App::modifiers_changed` / son `KeyEvent`'ten).
    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, modifiers: Modifiers, now: Instant) -> Response<FileGridEvent> {
        let mut out = self.scroll.pointer(theme, event, now).discard();
        if out.repaint {
            self.damage.mark();
        }
        if self.scroll.is_dragging_bar() {
            return out;
        }
        let cell = if event.kind == PointerKind::Leave { None } else { self.cell_at(event.position, now) };
        if cell != self.hovered {
            self.hovered = cell;
            self.damage.mark();
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Press(Button::Left) if self.rect.contains(event.position) && self.scroll.cursor(event.position).is_none() => {
                self.pressed = cell;
                self.focus_visible = false;
                self.damage.mark();
                out.repaint = true;
                let Some(i) = cell else {
                    // Boş alana tık: seçim boşalır (değiştiricisiz).
                    self.last_click = None;
                    if !modifiers.ctrl && !modifiers.shift && self.selection.clear() {
                        return Response::event(FileGridEvent::SelectionChanged);
                    }
                    return out;
                };
                let double = self.last_click.is_some_and(|(j, t, p)| {
                    j == i && now.saturating_duration_since(t) < DOUBLE_CLICK && { let d = p - event.position; d.x.hypot(d.y) < theme.space(1) }
                });
                if double && !modifiers.ctrl && !modifiers.shift {
                    self.last_click = None;
                    return Response::event(FileGridEvent::Activated(i));
                }
                self.last_click = Some((i, now, event.position));
                // Basışta seç (dosya yöneticilerinde ortak; sürükle-bırak gelince basılı tık
                // seçili öğede kümeyi koruyacak).
                if self.selection.click(i, modifiers) {
                    return Response::event(FileGridEvent::SelectionChanged);
                }
            }
            PointerKind::Release(Button::Left) if self.pressed.take().is_some() => {
                self.damage.mark();
                out.repaint = true;
            }
            PointerKind::Leave => self.pressed = None,
            _ => {}
        }
        out
    }

    pub fn cursor_shape(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        if let Some(c) = self.scroll.cursor(p) {
            return Some(c);
        }
        Some(if self.hovered.is_some() { CursorShape::Pointer } else { CursorShape::Default })
    }

    // ── Çizim ───────────────────────────────────────────────────────────────────

    /// Şekillendirilen etiket sayısı (önbellek isabeti sayılmaz; ölçüm/test).
    pub fn shaped_cells(&self) -> u64 {
        self.shaped
    }

    fn ensure_cell(&mut self, theme: &Theme, text: &mut TextSystem, i: usize, cap: usize) {
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
        let width = (self.cell_w - Self::pad(theme) * 2.0).max(1.0);
        let label = text.layout(&self.items[i].label, &self.label_style, Some(width));
        let mut icon = IconView::with_role(theme, self.items[i].icon.clone(), TextRole::IconLarge).with_size(theme, Self::icon_size(theme));
        icon.measure(text);
        self.shaped += 1;
        self.cache.insert(i, CellCache { label, icon, used: tick });
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let range = self.visible_range(now);
        let cap = FILE_CACHE_CELLS.max(range.len() * 2);
        for i in range.clone() {
            self.ensure_cell(theme, ui.painter.text_system(), i, cap);
        }
        let (cell_w, cell_h, cols) = (self.cell_w, self.cell_h, self.cols);
        let gap = FOCUS_RING;
        let pad = Self::pad(theme);
        let thumb = Self::thumbnail_size(theme);
        let radius = theme.shape.radius_medium;
        let radii = Corners::uniform(radius);
        let ring_radii = Corners::uniform(radius + gap);
        let thumb_radii = Corners::uniform(theme.shape.radius_small);
        let high = theme.contrast.is_high();
        let (hovered, pressed, focus_visible) = (self.hovered, self.pressed, self.focus_visible);
        let cursor = self.selection.cursor();
        let (selection, cache, thumbs) = (&self.selection, &mut self.cache, &self.thumbs);
        let mut drawn = 0;
        let mut pending = false;
        self.scroll.draw(ui, |ui, origin, _span| {
            for i in range.clone() {
                let p = Point::new(origin.x + (i % cols) as f32 * cell_w, origin.y + (i / cols) as f32 * cell_h);
                let tile = Rect::new(p.x, p.y, cell_w, cell_h).inset(Insets::uniform(gap));
                let selected = selection.is_selected(i);
                let op = if pressed == Some(i) {
                    STATE_PRESSED
                } else if hovered == Some(i) {
                    STATE_HOVER
                } else {
                    0.0
                };
                let colors = CellColors::new(theme, selected, op);
                if selected {
                    ui.painter.rect(tile, &RectStyle::filled(ui.fade(colors.fill), radii));
                }
                if op > 0.0 {
                    ui.painter.rect(tile, &RectStyle::filled(ui.fade(colors.layer), radii));
                }
                let Some(c) = cache.get_mut(&i) else { continue };
                let center = Point::new(tile.center().x, p.y + pad + thumb * 0.5);
                match thumbs.get(&i) {
                    Some(t) if t.is_ready() => {
                        let r = Rect::new(center.x - thumb * 0.5, center.y - thumb * 0.5, thumb, thumb);
                        let style = ImageStyle::new(ImageFit::Contain).with_radii(thumb_radii).with_opacity(ui.alpha());
                        ui.painter.image(r, t, &style);
                    }
                    other => {
                        // Küçük resim GPU'ya yüklenene dek ikon; yüklenince bir kare daha.
                        pending |= other.is_some() || c.icon.is_pending();
                        c.icon.draw_centered(ui.painter, center, ui.fade(colors.glyph));
                    }
                }
                let sz = c.label.size();
                let at = Point::new((tile.center().x - sz.width * 0.5).round(), (p.y + pad + thumb + theme.space(1)).round());
                ui.painter.text(&c.label, at, ui.fade(colors.glyph));
                drawn += 1;
                // Halka: klavye odağında imleçte; yüksek karşıtlıkta seçili hücreler hep çerçeveli.
                let ring = (focus_visible && i == cursor).then_some(FOCUS_RING).or((high && selected).then_some(HAIRLINE));
                if let Some(width) = ring {
                    let style = RectStyle { fill: Color::TRANSPARENT, radii: ring_radii, border: Some(Border { width, color: ui.fade(colors.ring) }) };
                    ui.painter.rect(tile.expand(gap), &style);
                }
            }
        });
        self.drawn_cells = drawn;
        if pending {
            ui.request_frame();
        }
        // Yüklenmeyi bekleyen doku: hazır olduğu kare de hasarlı (küçük resim ilk kez görünür).
        let was = std::mem::replace(&mut self.textures_pending, pending);
        self.damage.report(ui, self.rect, pending || was);
    }
}

// Erişilebilirlik (ADR-0011).
impl FileGrid {
    /// `Role::ListBox` kabı (çoklu kipte `multiselectable`); çocukları görünen hücreler (+ görünmese
    /// de imleç), `Role::ListBoxOption`: ad etiketten, `selected`, "3 / 120" konumu. Hücre anahtarı
    /// `item_key(indis)`: öğenin kalıcı kimliği (dosya adı karması) olmalı. `focused`: ızgara
    /// klavye odağındaysa imleç odak olur. Kabın adı çağıranın: `.label(..)`.
    pub fn access<'t, K: Hash>(&self, tree: &'t mut AccessTree, key: impl Hash, focused: bool, now: Instant, item_key: impl Fn(usize) -> K) -> AccessNode<'t> {
        let mut node = tree.push(key, Role::ListBox).bounds(self.rect);
        if self.selection.is_multiple() {
            node.node_mut().set_multiselectable();
        }
        let id = node.id();
        let n = self.items.len();
        let cursor = self.selection.cursor();
        let visible = self.visible_range(now);
        tree.within(id, |tree| {
            let extra = (n > 0 && !visible.contains(&cursor)).then_some(cursor);
            for i in visible.clone().chain(extra) {
                tree.push(item_key(i), Role::ListBoxOption)
                    .label(&self.items[i].label)
                    .bounds(self.cell_rect(i, now))
                    .selected(self.selection.is_selected(i))
                    .position(i + 1, n)
                    .clickable()
                    .focused_if(focused && i == cursor);
            }
        });
        tree.get(id).expect("az önce eklendi")
    }

    /// AT isteği: hücrede `Click` → yalnız onu seçer (`SelectionChanged`), `Focus` /
    /// `ScrollIntoView` → imleci taşır ve görünür kılar (seçim değişmez). `container`: `access`'in
    /// döndürdüğü kabın kimliği.
    pub fn access_action<K: Hash>(&mut self, container: NodeId, request: AccessRequest, item_key: impl Fn(usize) -> K, now: Instant) -> Response<FileGridEvent> {
        let range = self.visible_range(now);
        let cursor = self.selection.cursor();
        let Some(i) = range.chain(std::iter::once(cursor)).find(|&i| i < self.items.len() && child_id(container, item_key(i)) == request.node) else {
            return Response::none();
        };
        let changed = match request.action {
            AccessAction::Click => self.selection.click(i, Modifiers::default()),
            AccessAction::Focus | AccessAction::ScrollIntoView => {
                let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
                self.selection.navigate(i, ctrl)
            }
            _ => return Response::none(),
        };
        self.reveal_cursor(now);
        self.damage.mark();
        if changed { Response::event(FileGridEvent::SelectionChanged) } else { Response::repaint() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_names::IconName;
    use crate::input::test_util::*;
    use std::time::Duration;

    const CTRL: Modifiers = Modifiers { ctrl: true, shift: false, alt: false, logo: false, caps_lock: false, num_lock: false };
    const SHIFT: Modifiers = Modifiers { shift: true, ctrl: false, alt: false, logo: false, caps_lock: false, num_lock: false };

    fn items(n: usize) -> Vec<FileItem> {
        (0..n).map(|i| FileItem::new(format!("dosya-{i}.txt"), Icon::named(IconName::File))).collect()
    }

    /// 4 sütun (4 × 112 = 448), görünür ~3 satır.
    fn grid(n: usize, multiple: bool) -> (FileGrid, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = FileGrid::new(&theme, multiple);
        g.set_rect(&theme, Rect::new(0.0, 0.0, 460.0, 400.0), t0);
        g.set_items(items(n), t0);
        (g, theme, t0)
    }

    fn center(g: &FileGrid, i: usize, t: Instant) -> Point {
        g.cell_rect(i, t).center()
    }

    fn click(g: &mut FileGrid, theme: &Theme, p: Point, m: Modifiers, t: Instant) -> Option<FileGridEvent> {
        let a = g.pointer(theme, &press(p.x, p.y), m, t).event;
        let b = g.pointer(theme, &release(p.x, p.y), m, t).event;
        a.or(b)
    }

    fn selected(g: &FileGrid) -> Vec<usize> {
        g.selection().iter().collect()
    }

    #[test]
    fn columns_fill_width() {
        let (g, theme, _) = grid(10, true);
        assert_eq!(g.columns(), 4);
        assert_eq!(g.cell_w, 115.0, "artan genişlik paylaşılır");
        assert!(g.cell_h > FileGrid::thumbnail_size(&theme));
    }

    #[test]
    fn keyboard_grid_navigation() {
        let (mut g, _, t0) = grid(10, true); // satırlar: 0-3, 4-7, 8-9
        let k = |g: &mut FileGrid, key_: Key| {
            let _ = g.key(&key(key_), t0);
            g.cursor().unwrap()
        };
        assert_eq!(k(&mut g, Key::Down), 4);
        assert_eq!(k(&mut g, Key::Right), 5);
        assert_eq!(k(&mut g, Key::Down), 9, "kısa son satırda son öğeye");
        assert_eq!(k(&mut g, Key::Down), 9);
        assert_eq!(k(&mut g, Key::Up), 5);
        assert_eq!(k(&mut g, Key::Home), 0);
        assert_eq!(k(&mut g, Key::Up), 0);
        assert_eq!(k(&mut g, Key::End), 9);
        assert_eq!(selected(&g), [9]);
        assert_eq!(g.key(&key(Key::Enter), t0).event, Some(FileGridEvent::Activated(9)));
    }

    #[test]
    fn shift_arrows_extend_and_ctrl_a() {
        let (mut g, _, t0) = grid(10, true);
        g.select_only(1, t0);
        assert_eq!(g.key(&key_mod(Key::Right, SHIFT), t0).event, Some(FileGridEvent::SelectionChanged));
        let _ = g.key(&key_mod(Key::Down, SHIFT), t0);
        assert_eq!(selected(&g), [1, 2, 3, 4, 5, 6]);
        assert_eq!(g.key(&key_mod(Key::Char('a'), CTRL), t0).event, Some(FileGridEvent::SelectionChanged));
        assert_eq!(g.selection().count(), 10);
        // Tekli kipte Shift yalnız imleç.
        let (mut s, _, _) = grid(10, false);
        let _ = s.key(&key_mod(Key::Right, SHIFT), t0);
        assert_eq!(selected(&s), [1]);
        assert_eq!(s.key(&key_mod(Key::Char('a'), CTRL), t0).event, None);
    }

    #[test]
    fn pointer_selection_and_double_click() {
        let (mut g, theme, t0) = grid(10, true);
        let p2 = center(&g, 2, t0);
        assert_eq!(click(&mut g, &theme, p2, Modifiers::default(), t0), Some(FileGridEvent::SelectionChanged));
        assert_eq!({ let p = center(&g, 5, t0); click(&mut g, &theme, p, CTRL, t0) }, Some(FileGridEvent::SelectionChanged));
        assert_eq!({ let p = center(&g, 7, t0); click(&mut g, &theme, p, SHIFT, t0) }, Some(FileGridEvent::SelectionChanged));
        assert_eq!(selected(&g), [5, 6, 7], "Shift aralığı çapadan (5) öncekinin yerine");
        // Çift tık (süre içinde, aynı yerde) Activated; geç ikinci tık yalnız seçer.
        let p = center(&g, 3, t0);
        let _ = click(&mut g, &theme, p, Modifiers::default(), t0);
        assert_eq!(click(&mut g, &theme, p, Modifiers::default(), t0 + Duration::from_millis(200)), Some(FileGridEvent::Activated(3)));
        let t1 = t0 + Duration::from_secs(2);
        let _ = click(&mut g, &theme, p, Modifiers::default(), t1);
        assert_ne!(click(&mut g, &theme, p, Modifiers::default(), t1 + DOUBLE_CLICK), Some(FileGridEvent::Activated(3)));
        // Boş alana tık seçimi boşaltır.
        let empty = Point::new(center(&g, 9, t0).x + 2.0 * g.cell_w, center(&g, 9, t0).y);
        assert_eq!(click(&mut g, &theme, empty, Modifiers::default(), t1), Some(FileGridEvent::SelectionChanged));
        assert!(g.selection().is_empty());
    }

    #[test]
    fn virtualized_draw_and_visible_range() {
        use nuxut_render::Recorder;
        let (mut g, theme, t0) = grid(5000, true);
        let vr = g.visible_range(t0);
        assert_eq!(vr.start, 0);
        assert!(vr.len() <= 4 * 4, "{vr:?}");
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        rec.record(&mut text, (460, 400), 1.0, |painter| {
            let mut ui = Ui::new(painter, &theme, t0);
            g.draw(&mut ui);
            assert!(!ui.wants_frame(), "boşta kare yok");
        });
        assert!(g.drawn_cells <= 16 && g.drawn_cells > 0);
        assert_eq!(g.shaped_cells(), g.drawn_cells as u64);
        // Sona git: aralık kayar, öncekiler yeniden şekillenmez.
        let _ = g.key(&key(Key::End), t0);
        let later = t0 + Duration::from_secs(5);
        let vr = g.visible_range(later);
        assert_eq!(vr.end, 5000);
    }

    #[test]
    fn thumbnail_slot_and_items_reset() {
        let (mut g, _, t0) = grid(10, true);
        g.select_only(3, t0);
        assert!(!g.has_thumbnail(2));
        g.set_items(items(4), t0);
        assert!(g.selection().is_empty());
        assert_eq!(g.cursor(), Some(0));
        g.set_icon(1, Icon::named(IconName::Folder));
        assert_eq!(g.item(1).unwrap().icon, Icon::named(IconName::Folder));
        g.set_thumbnail(99, None); // panik yok
    }

    #[test]
    fn debug_is_redacted() {
        let theme = Theme::default();
        let mut g = FileGrid::new(&theme, false);
        g.set_items(vec![FileItem::new(crate::redact::SECRET, Icon::named(IconName::Folder))], Instant::now());
        crate::redact::assert_redacted(&g);
        crate::redact::assert_redacted(&FileItem::new(crate::redact::SECRET, Icon::named(IconName::Folder)));
    }
}
