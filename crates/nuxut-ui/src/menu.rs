//! Menü: genel [`MenuModel`] ağacı + bağlantı noktasından blob olarak büyüyen [`Menu`].
//!
//! - **Açılış:** menü bağlantı noktasındaki (bar'daki ikon, imleç) küçük bir tohumdan
//!   `spatial` yayıyla büyür; hız şekli esnetir (`Deform`). Kapanırken aynı noktaya geri erir.
//! - **Blob:** menü kendi şeklini [`Menu::shape`] ile dışarı verir; shell onu kendi blob
//!   alanına ekler (bar ile yumuşak birleşir) ve içeriği [`Menu::draw_content`] ile çizer.
//!   Kendi blob alanı olmayan uygulama [`Menu::draw`] kullanır.
//! - **Alt menü kaskad açılmaz:** menü alt sayfaya *dönüşür*. Şekil yeni sayfanın boyutuna yayla
//!   morf olur; eski içerik sola kayıp söner, yeni içerik sağdan gelir (geri dönüşte tersi).
//!   Alt sayfanın üstünde "‹ Üst menü adı" geri başlığı vardır. Geri: başlığa tıklama, Sol ok,
//!   Backspace. Sayfa yığını derinliği sınırsızdır.
//! - **Klavye:** Yukarı/Aşağı (dolanır), Home/End, Enter/Space, Sağ (alt menüye gir), Sol /
//!   Backspace (geri), Esc (kapat), harf (o harfle başlayan sonraki öğe). Hover seçimi taşır;
//!   seçim vurgusu öğeler arasında yayla kayar.
//! - İçerik şeklin görünen dikdörtgenine (`visual_rect`) yerleşir ve ona kırpılır; sığmayan
//!   metin `…` ile kısaltılır, uzun sayfa kaydırılır.

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{BlobShape, RectStyle, TextSystem};
use nuxut_theme::{STATE_HOVER, STATE_PRESSED, Theme};

use crate::grow::{Grow, Slide};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::label::Label;
use crate::style::{DISABLED_CONTENT, HAIRLINE, clip_text, disabled_content, state_layer_under};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Öğe kimliği (dbusmenu'de `i32` id; uygulamalar kendi sayılarını verir).
pub type MenuItemId = u64;

/// Bir sayfada gösterilecek en fazla öğe (dış kaynaklı menüler için sınır).
pub const MAX_MENU_ITEMS: usize = 512;
/// Öğe etiketinin en uzun hâli (bayt); uzun tepsi etiketleri layout'u kilitlemesin.
pub const MAX_MENU_LABEL: usize = 256;
/// Kısayol metninin en uzun hâli (bayt).
pub const MAX_MENU_SHORTCUT: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuItemKind {
    Action,
    /// Onay kutusu (işaretli mi).
    Check(bool),
    /// Radyo düğmesi (seçili mi). İkonlu radyo öğesinde solda ikon, seçiliyse sağda onay işareti.
    Radio(bool),
    Separator,
    /// Bölüm başlığı: seçilemeyen, etkinleştirilemeyen etiket satırı (ör. "Diğer"). Klavye
    /// gezinmesi, harfle arama ve işaretçi onu atlar; imleç `Default`.
    Header,
    /// Alt menü. Çocuklar boş olabilir: [`MenuEvent::SubmenuOpened`] ile tembel doldurulur
    /// ([`Menu::set_model`]).
    Submenu(Vec<MenuItem>),
}

#[derive(Clone, PartialEq)]
pub struct MenuItem {
    pub id: MenuItemId,
    pub label: String,
    pub icon: Option<Icon>,
    /// Sağda gösterilen kısayol metni ("Ctrl+Q").
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub kind: MenuItemKind,
}

/// Etiket ve kısayol uygulamadan gelir (tepsi menüsü): `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for MenuItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuItem")
            .field("id", &self.id)
            .field("label", &crate::redact::Len(&self.label))
            .field("icon", &self.icon.is_some())
            .field("shortcut", &crate::redact::OptLen(self.shortcut.as_deref()))
            .field("enabled", &self.enabled)
            .field("kind", &self.kind)
            .finish()
    }
}

impl MenuItem {
    pub fn action(id: MenuItemId, label: impl Into<String>) -> Self {
        Self { id, label: label.into(), icon: None, shortcut: None, enabled: true, kind: MenuItemKind::Action }
    }

    pub fn check(id: MenuItemId, label: impl Into<String>, checked: bool) -> Self {
        Self { kind: MenuItemKind::Check(checked), ..Self::action(id, label) }
    }

    pub fn radio(id: MenuItemId, label: impl Into<String>, selected: bool) -> Self {
        Self { kind: MenuItemKind::Radio(selected), ..Self::action(id, label) }
    }

    pub fn separator() -> Self {
        Self { kind: MenuItemKind::Separator, ..Self::action(0, "") }
    }

    /// Bölüm başlığı (seçilemez). `id` yalnızca model içi kimlik; etkinleştirme olayı üretmez.
    pub fn header(id: MenuItemId, label: impl Into<String>) -> Self {
        Self { kind: MenuItemKind::Header, enabled: false, ..Self::action(id, label) }
    }

    pub fn submenu(id: MenuItemId, label: impl Into<String>, children: Vec<MenuItem>) -> Self {
        Self { kind: MenuItemKind::Submenu(children), ..Self::action(id, label) }
    }

    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn children(&self) -> Option<&[MenuItem]> {
        match &self.kind {
            MenuItemKind::Submenu(c) => Some(c),
            _ => None,
        }
    }
}

/// Menü ağacı (kök sayfa).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuModel {
    pub items: Vec<MenuItem>,
}

impl MenuModel {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self { items }
    }

    /// Kimlik yolu → öğe listesi (kök için boş yol).
    fn items_at(&self, ids: &[MenuItemId]) -> Option<&[MenuItem]> {
        let mut items: &[MenuItem] = &self.items;
        for id in ids {
            items = items.iter().find(|i| i.id == *id)?.children()?;
        }
        Some(items)
    }

    /// Kimlikle öğeyi arar (derinlik öncelikli, yığınla; özyineleme yok).
    pub fn find(&self, id: MenuItemId) -> Option<&MenuItem> {
        let mut stack: Vec<&[MenuItem]> = vec![&self.items];
        while let Some(items) = stack.pop() {
            for item in items {
                if item.id == id && !matches!(item.kind, MenuItemKind::Separator | MenuItemKind::Header) {
                    return Some(item);
                }
                if let Some(c) = item.children() {
                    stack.push(c);
                }
            }
        }
        None
    }
}

/// dbusmenu/GTK anımsatıcısını temizler: `_Dosya` → `Dosya`, `__` → `_`.
pub fn strip_mnemonic(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' {
            if chars.peek() == Some(&'_') {
                out.push('_');
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Menünün bağlantı noktasına göre hangi tarafta açılacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuSide {
    #[default]
    Below,
    Above,
    Left,
    Right,
}

/// Ekran kenarı (bar, çekmece). Kenara yapışık bir öğeden açılan menü/ipucu ekranın
/// içine doğru açılır: [`MenuSide::away_from`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreenEdge {
    #[default]
    Top,
    Right,
    Bottom,
    Left,
}

impl MenuSide {
    /// `edge` kenarındaki bir öğeden ekranın içine doğru: üst → aşağı, alt → yukarı,
    /// sol → sağa, sağ → sola.
    pub fn away_from(edge: ScreenEdge) -> Self {
        match edge {
            ScreenEdge::Top => Self::Below,
            ScreenEdge::Bottom => Self::Above,
            ScreenEdge::Left => Self::Right,
            ScreenEdge::Right => Self::Left,
        }
    }

    /// Karşı taraf (sığmayınca dönülen).
    pub fn opposite(self) -> Self {
        match self {
            Self::Below => Self::Above,
            Self::Above => Self::Below,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Çapraz eksende hizalama (Below/Above için yatay, Left/Right için dikey).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuAlign {
    /// Menünün başı bağlantının başıyla hizalı (imleç menüsü: sol-üst köşe imleçte).
    Start,
    #[default]
    Center,
    End,
}

/// Menünün büyüyeceği yer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuAnchor {
    /// Bağlantı öğesi (tray ikonu) ya da sıfır boyutlu nokta (imleç).
    pub rect: Rect,
    pub side: MenuSide,
    pub align: MenuAlign,
    /// Bağlantı ile menü arası. `<= 0`: menü bağlantıya yapışık (bar'dan büyür; o kenarın
    /// köşeleri düz, blob filetosu birleştirir).
    pub gap: f32,
}

impl MenuAnchor {
    pub fn below(rect: Rect) -> Self {
        Self { rect, side: MenuSide::Below, align: MenuAlign::Center, gap: 0.0 }
    }

    /// İmleç menüsü: sol-üst köşe noktada, küçük boşlukla.
    pub fn at_point(p: Point, gap: f32) -> Self {
        Self { rect: Rect::new(p.x, p.y, 0.0, 0.0), side: MenuSide::Below, align: MenuAlign::Start, gap }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEvent {
    /// Öğe etkinleştirildi (eylem, onay, radyo). Menü kapanmaya başladı. Onay/radyo
    /// durumunu modelde değiştirmek sağlayıcının işidir.
    Activated(MenuItemId),
    /// Alt sayfa açıldı (dbusmenu `AboutToShow` için); çocuklar sonra `set_model` ile gelebilir.
    SubmenuOpened(MenuItemId),
    /// Esc ya da dışarı tıklama ile kapandı.
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Back,
    Item,
    Separator,
    Header,
}

#[derive(Debug, Clone)]
struct Row {
    kind: RowKind,
    id: MenuItemId,
    y: f32,
    height: f32,
    enabled: bool,
    submenu: bool,
    leading: Option<IconView>,
    label: Option<Label>,
    shortcut: Option<Label>,
    trailing: Option<IconView>,
}

impl Row {
    fn selectable(&self) -> bool {
        self.enabled && matches!(self.kind, RowKind::Back | RowKind::Item)
    }
}

#[derive(Debug, Clone)]
struct Page {
    /// Kökten bu sayfaya kimlik yolu.
    path: Vec<MenuItemId>,
    rows: Vec<Row>,
    /// İçerik boyutu (kırpılmamış).
    size: Size,
    scroll: f32,
    selected: Option<usize>,
    /// Öğe satırlarında ikon sütunu var mı (etiketler hizalı).
    has_leading: bool,
    /// İkon yuvası kipi: ikonsuz/yüklenmemiş öğede yer tutucu.
    icon_slots: bool,
}

impl Page {
    fn row_at(&self, y: f32) -> Option<usize> {
        let y = y + self.scroll;
        self.rows.iter().position(|r| y >= r.y && y < r.y + r.height)
    }

    fn select_id(&self, id: MenuItemId) -> Option<usize> {
        self.rows.iter().position(|r| r.kind == RowKind::Item && r.id == id && r.selectable())
    }
}

#[derive(Debug, Clone)]
struct Transition {
    old: Page,
    slide: Slide,
}

/// Bağlantıdan blob olarak büyüyen, alt menüleri sayfaya dönüştüren menü.
#[derive(Debug, Clone)]
pub struct Menu {
    model: MenuModel,
    grow: Grow,
    pages: Vec<Page>,
    transition: Option<Transition>,
    /// Seçim vurgusu (sayfa içerik koordinatı) ve opaklığı.
    highlight: Animated<Rect>,
    highlight_alpha: Animated<f32>,
    pressed_row: Option<usize>,
    pointer_inside: bool,
    /// Sayfaların en az genişliği (ör. Select alanının genişliği).
    min_width: f32,
    icons_pending: bool,
    /// İkon yuvası kipi (Select): her öğe satırında ikon yeri ayrılır; ikonu olmayan ya da dokusu
    /// henüz yüklenmemiş öğede yer tutucu çizilir.
    icon_slots: bool,
    damage: DamageState,
}

impl Menu {
    pub fn new(theme: &Theme) -> Self {
        Self {
            model: MenuModel::default(),
            grow: Grow::new(theme),
            pages: Vec::new(),
            transition: None,
            highlight: Animated::new(Rect::ZERO, theme.motion.spatial_fast),
            highlight_alpha: Animated::new(0.0, theme.motion.effects),
            pressed_row: None,
            pointer_inside: false,
            min_width: 0.0,
            icons_pending: false,
            icon_slots: false,
            damage: DamageState::new(),
        }
    }

    /// Sayfaların en az genişliği (sonraki `open`/`set_model`'da uygulanır).
    pub fn set_min_width(&mut self, width: f32) {
        self.min_width = if width.is_finite() { width.max(0.0) } else { 0.0 };
    }

    /// İkon yuvası kipi (sonraki `open`/`set_model`'da uygulanır): öğe satırlarında ikon yeri
    /// her zaman ayrılır; ikonsuz ya da dokusu yüklenmemiş öğe yer tutucu çizer (yerleşim
    /// ikon gelince zıplamaz). Select ikonlu seçeneklerde açar.
    pub(crate) fn set_icon_slots(&mut self, on: bool) {
        self.icon_slots = on;
    }

    pub fn model(&self) -> &MenuModel {
        &self.model
    }

    /// Kullanıcıya göre açık (kapanış animasyonu sayılmaz).
    pub fn is_open(&self) -> bool {
        self.grow.is_open()
    }

    /// Ekranda bir şey var mı (açık ya da kapanıyor).
    pub fn is_visible(&self) -> bool {
        self.grow.is_visible()
    }

    /// Geçerli sayfa derinliği (0 = kök).
    pub fn depth(&self) -> usize {
        self.pages.len().saturating_sub(1)
    }

    /// Menüyü açar. `bounds`: menünün sığması gereken alan (yüzey koordinatı; ör. çerçevenin
    /// iç dikdörtgeni). Açıkken çağrılırsa yeni bağlantıya morf olur.
    pub fn open(&mut self, text: &mut TextSystem, theme: &Theme, model: MenuModel, anchor: MenuAnchor, bounds: Rect, now: Instant) {
        self.model = model;
        self.transition = None;
        self.pressed_row = None;
        let root = build_page(text, theme, &self.model, Vec::new(), bounds, self.min_width, self.icon_slots);
        let size = root.size;
        self.pages = vec![root];
        if !self.grow.is_visible() {
            self.highlight_alpha.jump_to(0.0);
        }
        self.grow.open(anchor, bounds, size, now);
        self.damage.mark();
    }

    /// Kapanış: tohuma geri erir.
    pub fn close(&mut self, now: Instant) {
        if self.grow.close(now) {
            self.highlight_alpha.set_target(0.0, now);
            self.pressed_row = None;
            self.damage.mark();
        }
    }

    /// Model değişti (dbusmenu `LayoutUpdated`, tembel alt menü): açık sayfa yığını kimlik
    /// yolundan yeniden kurulur; artık var olmayan sayfalar düşer, seçim kimlikle korunur.
    pub fn set_model(&mut self, text: &mut TextSystem, theme: &Theme, model: MenuModel, now: Instant) {
        self.model = model;
        if !self.is_visible() {
            return;
        }
        let bounds = self.grow.bounds;
        let old = std::mem::take(&mut self.pages);
        for page in old {
            if self.model.items_at(&page.path).is_none() {
                break;
            }
            let mut new = build_page(text, theme, &self.model, page.path.clone(), bounds, self.min_width, self.icon_slots);
            let sel_id = page.selected.and_then(|i| page.rows.get(i)).map(|r| r.id);
            new.selected = sel_id.and_then(|id| new.select_id(id));
            new.scroll = page.scroll.min((new.size.height - bounds.height).max(0.0));
            self.pages.push(new);
        }
        if self.pages.is_empty() {
            self.pages.push(build_page(text, theme, &self.model, Vec::new(), bounds, self.min_width, self.icon_slots));
        }
        self.pressed_row = None;
        self.grow.resize(self.page().size, now);
        self.sync_highlight(theme, now, false);
        self.damage.mark();
    }

    pub fn set_theme(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        self.set_theme_motion(theme, now);
        let model = self.model.clone();
        self.set_model(text, theme, model, now);
    }

    /// Temanın yalnızca hareket kısmı (ölçüm gerektirmez); sayfaların yeniden ölçümü
    /// çağıranın sorumluluğunda ([`set_theme`](Self::set_theme) ya da [`set_model`](Self::set_model)).
    pub(crate) fn set_theme_motion(&mut self, theme: &Theme, now: Instant) {
        self.grow.set_theme(theme, now);
        self.highlight.set_motion(theme.motion.spatial_fast, now);
        self.highlight_alpha.set_motion(theme.motion.effects, now);
    }

    /// Kullanılabilir alan değişti (çıkış boyutu).
    pub fn set_bounds(&mut self, text: &mut TextSystem, theme: &Theme, bounds: Rect, now: Instant) {
        if self.grow.bounds == bounds {
            return;
        }
        self.grow.bounds = bounds;
        let model = self.model.clone();
        self.set_model(text, theme, model, now);
    }

    // --- Yerleşim ---------------------------------------------------------------------

    fn page(&self) -> &Page {
        self.pages.last().expect("menü sayfası yok")
    }

    fn page_mut(&mut self) -> &mut Page {
        self.pages.last_mut().expect("menü sayfası yok")
    }

    /// Çizilecek blob şekli (shell kendi alanına ekler). Kapalıyken `None`. Karede bir kez
    /// çağrılmalı (esneme durumu hızla güncellenir).
    pub fn shape(&mut self, now: Instant, theme: &Theme) -> Option<BlobShape> {
        self.grow.shape(now, theme.shape.radius_large)
    }

    /// İçeriğin yerleştiği dikdörtgen (şeklin görünen hali).
    pub fn content_rect(&mut self, now: Instant, theme: &Theme) -> Option<Rect> {
        self.shape(now, theme).map(|s| s.visual_rect())
    }

    /// Input bölgesi için: açıkken hedef dikdörtgen (kapanırken `None`, tıklama geçer).
    pub fn input_rect(&self) -> Option<Rect> {
        self.is_open().then(|| self.grow.target())
    }

    /// Animasyon sürüyor mu (kare istemek için; `draw_content` bunu `Ui`'ye zaten bildirir).
    pub fn is_animating(&mut self, now: Instant) -> bool {
        let t = self.transition.as_mut().is_some_and(|t| !t.slide.is_settled(now));
        self.grow.is_animating(now) | !self.highlight.is_settled(now) | !self.highlight_alpha.is_settled(now) | t
    }

    /// Kare sonu bakımı: açılış/kapanış bitti mi.
    fn settle_phase(&mut self, now: Instant) {
        if self.grow.settle(now) {
            self.pages.clear();
            self.transition = None;
        }
        if self.transition.as_mut().is_some_and(|t| t.slide.is_settled(now)) {
            self.transition = None;
        }
    }

    // --- Gezinme ----------------------------------------------------------------------

    fn enter_submenu(&mut self, text: &mut TextSystem, theme: &Theme, row: usize, now: Instant) -> Option<MenuEvent> {
        let page = self.page();
        let r = page.rows.get(row)?;
        if !r.submenu || !r.enabled {
            return None;
        }
        let id = r.id;
        let mut path = page.path.clone();
        path.push(id);
        let mut new = build_page(text, theme, &self.model, path, self.grow.bounds, self.min_width, self.icon_slots);
        new.selected = None;
        self.push_transition(1.0, now);
        self.pages.push(new);
        self.after_page_change(theme, now);
        Some(MenuEvent::SubmenuOpened(id))
    }

    fn go_back(&mut self, theme: &Theme, now: Instant) -> bool {
        if self.pages.len() <= 1 {
            return false;
        }
        self.push_transition(-1.0, now);
        let child = self.pages.pop().expect("sayfa");
        // Geri dönünce alt menüye giden öğe seçili kalsın.
        if let Some(&id) = child.path.last() {
            let p = self.page_mut();
            p.selected = p.select_id(id);
        }
        self.after_page_change(theme, now);
        true
    }

    fn push_transition(&mut self, dir: f32, now: Instant) {
        let old = self.page().clone();
        self.transition = Some(Transition { old, slide: Slide::new(dir, self.grow.motion(), now) });
    }

    fn after_page_change(&mut self, theme: &Theme, now: Instant) {
        self.pressed_row = None;
        self.grow.resize(self.page().size, now);
        self.sync_highlight(theme, now, true);
        self.damage.mark();
    }

    /// Vurguyu seçili satıra taşır. `jump`: yeni sayfada kaymadan belirir.
    fn sync_highlight(&mut self, theme: &Theme, now: Instant, jump: bool) {
        let Some(page) = self.pages.last() else { return };
        match page.selected.and_then(|i| page.rows.get(i)) {
            Some(row) => {
                let inset = theme.space(2);
                let r = Rect::new(inset, row.y, page.size.width - inset * 2.0, row.height);
                if jump || self.highlight_alpha.target() == 0.0 {
                    self.highlight.jump_to(r);
                } else {
                    self.highlight.set_target(r, now);
                }
                self.highlight_alpha.set_target(1.0, now);
            }
            None => self.highlight_alpha.set_target(0.0, now),
        }
    }

    fn select(&mut self, theme: &Theme, row: Option<usize>, now: Instant, scroll_into_view: bool) -> bool {
        let view_h = self.grow.target().height.max(1.0);
        let page = self.page_mut();
        if page.selected == row {
            return false;
        }
        page.selected = row;
        if scroll_into_view && let Some((top, bottom)) = row.and_then(|i| page.rows.get(i)).map(|r| (r.y, r.y + r.height)) {
            let p = page;
            if top < p.scroll {
                p.scroll = top;
            } else if bottom > p.scroll + view_h {
                p.scroll = bottom - view_h;
            }
        }
        self.sync_highlight(theme, now, false);
        self.damage.mark();
        true
    }

    fn move_selection(&mut self, theme: &Theme, delta: i32, now: Instant) -> bool {
        let page = self.page();
        let n = page.rows.len();
        let selectable: Vec<usize> = (0..n).filter(|&i| page.rows[i].selectable() && page.rows[i].kind == RowKind::Item).collect();
        if selectable.is_empty() {
            return false;
        }
        let next = match page.selected.and_then(|s| selectable.iter().position(|&i| i == s)) {
            Some(pos) => selectable[(pos as i32 + delta).rem_euclid(selectable.len() as i32) as usize],
            None if delta >= 0 => selectable[0],
            None => selectable[selectable.len() - 1],
        };
        self.select(theme, Some(next), now, true)
    }

    fn activate(&mut self, text: &mut TextSystem, theme: &Theme, row: usize, now: Instant) -> Response<MenuEvent> {
        let Some(r) = self.page().rows.get(row) else { return Response::none() };
        if !r.selectable() {
            return Response::none();
        }
        match r.kind {
            RowKind::Back => Response::repaint().with_repaint(self.go_back(theme, now)),
            RowKind::Item if r.submenu => match self.enter_submenu(text, theme, row, now) {
                Some(e) => Response::event(e),
                None => Response::none(),
            },
            RowKind::Item => {
                let id = r.id;
                self.close(now);
                Response::event(MenuEvent::Activated(id))
            }
            RowKind::Separator | RowKind::Header => Response::none(),
        }
    }

    // --- Girdi ------------------------------------------------------------------------

    /// İçerik koordinatına çeviri için: geçerli sayfanın içerik dikdörtgeni (hedef şekil).
    fn content_frame(&self) -> Rect {
        self.grow.target()
    }

    /// İşaretçi. Dışarı basma menüyü kapatır (`Dismissed`).
    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<MenuEvent> {
        if !self.is_open() {
            return Response::none();
        }
        let frame = self.content_frame();
        let inside = frame.contains(event.position);
        let local_y = event.position.y - frame.y;
        let row = if inside { self.page().row_at(local_y) } else { None };
        let hover_row = row.filter(|&i| self.page().rows[i].selectable());
        let mut out = Response::none();
        match event.kind {
            PointerKind::Leave => {
                self.pointer_inside = false;
                self.pressed_row = None;
                return Response::none().with_repaint(self.select(theme, None, now, false));
            }
            PointerKind::Motion | PointerKind::Enter if inside || self.pointer_inside => {
                out.repaint |= self.select(theme, hover_row, now, false);
            }
            PointerKind::Press(_) if !inside => {
                self.close(now);
                return Response::event(MenuEvent::Dismissed);
            }
            PointerKind::Press(Button::Left | Button::Right) => {
                self.pressed_row = hover_row;
                self.highlight_alpha.set_target(1.0, now);
                self.damage.mark();
                out.repaint = true;
            }
            PointerKind::Release(Button::Left | Button::Right) => {
                let pressed = self.pressed_row.take();
                if let Some(r) = hover_row
                    && pressed == Some(r)
                {
                    return self.activate(text, theme, r, now);
                }
                self.damage.mark();
                out.repaint = true;
            }
            PointerKind::Scroll { delta, steps } if inside => {
                let row_h = theme.space(10);
                let dy = if steps.y != 0.0 { steps.y * row_h } else { delta.y };
                let max = (self.page().size.height - frame.height).max(0.0);
                let p = self.page_mut();
                let s = (p.scroll + dy).clamp(0.0, max);
                if s != p.scroll {
                    p.scroll = s;
                    self.damage.mark();
                    out.repaint = true;
                }
            }
            _ => {}
        }
        self.pointer_inside = inside;
        out
    }

    pub fn key(&mut self, text: &mut TextSystem, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<MenuEvent> {
        if !self.is_open() || event.state == KeyState::Released {
            return Response::none();
        }
        let selected = self.page().selected;
        match event.key {
            Key::Escape => {
                self.close(now);
                Response::event(MenuEvent::Dismissed)
            }
            Key::Down | Key::Tab => Response::none().with_repaint(self.move_selection(theme, 1, now)),
            Key::Up => Response::none().with_repaint(self.move_selection(theme, -1, now)),
            Key::Home => {
                self.page_mut().selected = None;
                Response::none().with_repaint(self.move_selection(theme, 1, now))
            }
            Key::End => {
                self.page_mut().selected = None;
                Response::none().with_repaint(self.move_selection(theme, -1, now))
            }
            Key::Enter | Key::Space => match selected {
                Some(r) => self.activate(text, theme, r, now),
                None => Response::none().with_repaint(self.move_selection(theme, 1, now)),
            },
            Key::Right => match selected {
                Some(r) => match self.enter_submenu(text, theme, r, now) {
                    Some(e) => Response::event(e),
                    None => Response::none(),
                },
                None => Response::none(),
            },
            Key::Left | Key::Backspace => Response::none().with_repaint(self.go_back(theme, now)),
            Key::Char(c) => Response::none().with_repaint(self.typeahead(theme, c, now)),
            _ => Response::none(),
        }
    }

    /// Harfle başlayan bir sonraki öğeye atlar.
    fn typeahead(&mut self, theme: &Theme, c: char, now: Instant) -> bool {
        let lower: String = c.to_lowercase().collect();
        let page = self.page();
        let n = page.rows.len();
        let start = page.selected.map_or(0, |s| s + 1);
        let found = (0..n).map(|k| (start + k) % n).find(|&i| {
            let r = &page.rows[i];
            r.kind == RowKind::Item
                && r.selectable()
                && r.label.as_ref().is_some_and(|l| l.text().to_lowercase().starts_with(&lower))
        });
        found.is_some_and(|i| self.select(theme, Some(i), now, true))
    }

    /// İmleç: açıkken içerde seçilebilir öğe `Pointer`, ayraç/devre dışı/boş alan `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.is_open() {
            return None;
        }
        let frame = self.content_frame();
        if !frame.contains(p) {
            return None;
        }
        let page = self.pages.last()?;
        let row = page.row_at(p.y - frame.y).map(|i| &page.rows[i]);
        Some(if row.is_some_and(Row::selectable) { CursorShape::Pointer } else { CursorShape::Default })
    }

    // --- Çizim ------------------------------------------------------------------------

    /// Kendi blob alanı olmayanlar için: şekli (gölgeli) ve içeriği çizer.
    pub fn draw(&mut self, ui: &mut Ui) {
        let shape = self.shape(ui.now, ui.theme);
        self.grow.draw_standalone(ui, shape);
        match shape {
            Some(shape) => self.draw_content_with(ui, shape),
            None => {
                self.damage.report_hidden(ui);
                self.settle_phase(ui.now);
            }
        }
    }

    /// Bağımsız kullanım, kaplama katmanında: [`draw`](Self::draw) gibi ama en üstte ve
    /// kapsayıcı kırpmasından bağımsız ([`Ui::overlay`]); açıkken girdi bölgesini bildirir
    /// ([`Ui::overlay_input`]). Tetikleyicinin yanında, kırpılı kapsayıcının içinden
    /// çağrılabilir.
    pub fn draw_overlay(&mut self, ui: &mut Ui) {
        if let Some(r) = self.input_rect() {
            ui.overlay_input(r);
        }
        ui.overlay(|ui| self.draw(ui));
    }

    /// Yalnızca içerik (shell şekli kendi blob alanında çizer). `shape`'ten sonra çağrılır.
    pub fn draw_content(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        // `shape` aynı karede zaten çağrıldıysa esneme durumu değişmez (aynı `now`).
        if let Some(shape) = self.shape(ui.now, theme) {
            self.draw_content_with(ui, shape);
        } else {
            self.damage.report_hidden(ui);
            self.settle_phase(ui.now);
        }
    }

    fn draw_content_with(&mut self, ui: &mut Ui, shape: BlobShape) {
        let now = ui.now;
        let theme = ui.theme;
        let visual = shape.visual_rect();
        // İçerik görünürlüğü: açılırken/kapanırken şeklin ana eksendeki doluluğuna göre.
        let reveal = self.grow.reveal(visual);
        if reveal > 0.0 && visual.width > 0.5 && visual.height > 0.5 {
            ui.painter.push_clip(visual);
            ui.with_alpha(reveal, |ui| {
                let slide = theme.space(12);
                if let Some(t) = &mut self.transition {
                    let (of, nf) = t.slide.frames(now, slide);
                    let (old_alpha, new_alpha) = (of.alpha, nf.alpha);
                    let old_origin = Point::new(visual.x + of.dx, visual.y);
                    let new_origin = Point::new(visual.x + nf.dx, visual.y);
                    let old = &mut t.old;
                    ui.with_alpha(old_alpha, |ui| draw_page(ui, old, old_origin, visual.width, None));
                    let page = self.pages.last_mut().expect("sayfa");
                    ui.with_alpha(new_alpha, |ui| draw_page(ui, page, new_origin, visual.width, None));
                } else {
                    let hl = (self.highlight.value(now), self.highlight_alpha.value(now), self.pressed_row.is_some());
                    let page = self.pages.last_mut().expect("sayfa");
                    draw_page(ui, page, visual.origin(), visual.width, Some(hl));
                }
            });
            ui.painter.pop_clip();
        }
        // Doku ikonları GPU'ya ulaşana kadar kare istenir; ulaştığı karede yeniden boyanır.
        let pending = self.pages.last().is_some_and(|p| p.rows.iter().any(|r| r.leading.as_ref().is_some_and(IconView::is_pending)));
        if self.icons_pending && !pending {
            self.damage.mark();
        }
        self.icons_pending = pending;
        let animating = self.is_animating(now) | pending;
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, visual.expand(1.0), animating);
        self.settle_phase(now);
    }
}

/// Bir sayfanın satırlarını kurar (metin ölçümü dahil).
fn build_page(text: &mut TextSystem, theme: &Theme, model: &MenuModel, path: Vec<MenuItemId>, bounds: Rect, min_width: f32, icon_slots: bool) -> Page {
    let items = model.items_at(&path).unwrap_or(&[]);
    let items = &items[..items.len().min(MAX_MENU_ITEMS)];
    let body = text_style(theme, TextRole::Body).with_max_lines(1);
    let caption = text_style(theme, TextRole::Caption).with_max_lines(1);
    let pad_v = theme.space(2);
    let item_h = theme.space(10);
    let sep_h = theme.space(3);
    let inset = theme.space(2);
    let inner = theme.space(3);
    let gap = theme.space(3);
    let header_h = theme.space(8);
    let header_style = text_style(theme, TextRole::Label).with_max_lines(1);

    let has_leading = icon_slots || items.iter().any(|i| i.icon.is_some() || matches!(i.kind, MenuItemKind::Check(_) | MenuItemKind::Radio(_)));
    let mut rows = Vec::with_capacity(items.len() + 1);
    let mut y = pad_v;

    // Geri başlığı: "‹ Üst menü adı".
    if let Some(&parent) = path.last() {
        let parent_items = model.items_at(&path[..path.len() - 1]).unwrap_or(&[]);
        let title = parent_items.iter().find(|i| i.id == parent).map_or("", |i| clip_text(&i.label, MAX_MENU_LABEL));
        rows.push(Row {
            kind: RowKind::Back,
            id: parent,
            y,
            height: item_h,
            enabled: true,
            submenu: false,
            leading: Some(IconView::with_role(theme, Icon::named(IconName::ChevronLeft), TextRole::IconSmall)),
            label: Some(Label::new(title, text_style(theme, TextRole::Label).with_max_lines(1))),
            shortcut: None,
            trailing: None,
        });
        y += item_h;
        rows.push(Row {
            kind: RowKind::Separator,
            id: 0,
            y,
            height: sep_h,
            enabled: false,
            submenu: false,
            leading: None,
            label: None,
            shortcut: None,
            trailing: None,
        });
        y += sep_h;
    }

    for item in items {
        if item.kind == MenuItemKind::Separator {
            rows.push(Row {
                kind: RowKind::Separator,
                id: item.id,
                y,
                height: sep_h,
                enabled: false,
                submenu: false,
                leading: None,
                label: None,
                shortcut: None,
                trailing: None,
            });
            y += sep_h;
            continue;
        }
        if item.kind == MenuItemKind::Header {
            rows.push(Row {
                kind: RowKind::Header,
                id: item.id,
                y,
                height: header_h,
                enabled: false,
                submenu: false,
                leading: None,
                label: Some(Label::new(clip_text(&item.label, MAX_MENU_LABEL), header_style.clone())),
                shortcut: None,
                trailing: None,
            });
            y += header_h;
            continue;
        }
        // İkonlu radyo (ya da ikon yuvası kipi): solda ikon, seçiliyse sağda onay işareti.
        let icon_radio = matches!(item.kind, MenuItemKind::Radio(_)) && (item.icon.is_some() || icon_slots);
        let leading = match (&item.kind, &item.icon) {
            (MenuItemKind::Check(on), _) => Some(Icon::named(if *on { IconName::CheckboxChecked } else { IconName::CheckboxUnchecked })),
            (MenuItemKind::Radio(on), None) if !icon_radio => Some(Icon::named(if *on { IconName::RadioChecked } else { IconName::RadioUnchecked })),
            (_, Some(icon)) => Some(icon.clone()),
            _ => None,
        }
        .map(|icon| IconView::with_role(theme, icon, TextRole::IconSmall));
        let submenu = matches!(item.kind, MenuItemKind::Submenu(_));
        let trailing = if submenu {
            Some(IconView::with_role(theme, Icon::named(IconName::ChevronRight), TextRole::IconSmall))
        } else if icon_radio && item.kind == MenuItemKind::Radio(true) {
            Some(IconView::with_role(theme, Icon::named(IconName::Check), TextRole::IconSmall))
        } else {
            None
        };
        rows.push(Row {
            kind: RowKind::Item,
            id: item.id,
            y,
            height: item_h,
            enabled: item.enabled,
            submenu,
            leading,
            label: Some(Label::new(clip_text(&item.label, MAX_MENU_LABEL), body.clone())),
            shortcut: item
                .shortcut
                .as_ref()
                .filter(|s| !s.is_empty())
                .map(|s| Label::new(clip_text(s, MAX_MENU_SHORTCUT), caption.clone())),
            trailing,
        });
        y += item_h;
    }
    if rows.iter().all(|r| r.kind != RowKind::Item) {
        // Boş sayfa (tembel alt menü henüz dolmadı): tek satırlık yer.
        y += item_h;
    }
    y += pad_v;

    // Genişlik: en geniş satır; sınırlar içinde. Sığmayan etiket `…` ile kısalır.
    let icon_w = text_style(theme, TextRole::IconSmall).size;
    let lead_w = if has_leading { icon_w + gap } else { 0.0 };
    let mut label_w: f32 = 0.0;
    let mut trail_w: f32 = 0.0;
    for r in &mut rows {
        if let Some(l) = &mut r.label {
            let extra = if r.kind == RowKind::Back { icon_w + gap } else { 0.0 };
            label_w = label_w.max(l.measure(text).width + extra);
        }
        if let Some(s) = &mut r.shortcut {
            trail_w = trail_w.max(s.measure(text).width + gap * 2.0);
        }
        if r.trailing.is_some() {
            trail_w = trail_w.max(icon_w + gap);
        }
    }
    let chrome = (inset + inner) * 2.0 + lead_w + trail_w;
    // En geniş: space(90) ya da istenen en az genişlik; ikisi de alana sığar. (clamp'ta
    // min ≤ max her zaman.)
    let max_w = theme.space(90).max(min_width).min(bounds.width.max(theme.space(44)));
    let min_w = theme.space(44).max(min_width).min(max_w);
    let width = (chrome + label_w).clamp(min_w, max_w).ceil();
    let avail = width - chrome;
    if label_w > avail {
        for r in &mut rows {
            let extra = if r.kind == RowKind::Back { icon_w + gap } else { 0.0 };
            if let Some(l) = &mut r.label {
                l.set_max_width(Some((avail - extra).max(0.0)));
            }
        }
    }
    Page { path, rows, size: Size::new(width, y), scroll: 0.0, selected: None, has_leading, icon_slots }
}

/// Sayfayı `origin`'den çizer. `highlight`: (dikdörtgen, opaklık, basılı mı).
fn draw_page(ui: &mut Ui, page: &mut Page, origin: Point, width: f32, highlight: Option<(Rect, f32, bool)>) {
    let theme = ui.theme;
    let c = &theme.colors;
    let inset = theme.space(2);
    let inner = theme.space(3);
    let gap = theme.space(3);
    let icon_w = text_style(theme, TextRole::IconSmall).size;
    let has_leading = page.has_leading;
    let o = Point::new(origin.x, origin.y - page.scroll);
    let radius = (theme.shape.radius_large - inset).max(theme.shape.radius_small);

    if let Some((hl, alpha, pressed)) = highlight
        && alpha > 0.0
    {
        let layer = if pressed { STATE_PRESSED } else { STATE_HOVER };
        let r = Rect::new(o.x + hl.x, o.y + hl.y, (width - inset * 2.0).max(0.0), hl.height);
        let layer = state_layer_under(theme, c.on_surface, layer * alpha.clamp(0.0, 1.0), &[], &[c.on_surface, c.on_surface_variant, c.primary]);
        let color = ui.fade(layer);
        ui.painter.rect(r, &RectStyle::filled(color, Corners::uniform(radius)));
    }

    let view_top = origin.y;
    let view_bottom = origin.y + ui.painter.size().height;
    for row in &mut page.rows {
        let top = o.y + row.y;
        // Sanallaştırma: görünmeyen satır çizilmez.
        if top + row.height < view_top || top > view_bottom {
            continue;
        }
        let mid = top + row.height * 0.5;
        match row.kind {
            RowKind::Separator => {
                let line = Rect::new(o.x + inset + inner, (mid - HAIRLINE * 0.5).round(), (width - (inset + inner) * 2.0).max(0.0), HAIRLINE);
                ui.painter.rect(line, &RectStyle::filled(ui.fade(crate::style::divider(theme)), Corners::ZERO));
            }
            RowKind::Header => {
                // Bölüm başlığı öğe etiketleriyle aynı sütunda, ikincil renkte.
                let x = o.x + inset + inner + if has_leading { icon_w + gap } else { 0.0 };
                if let Some(l) = &mut row.label {
                    let m = l.measure_in(ui.painter);
                    l.draw(ui.painter, Point::new(x, mid - m.height * 0.5), ui.fade(c.on_surface_variant));
                }
            }
            RowKind::Back | RowKind::Item => {
                let content = if row.enabled { c.on_surface } else { disabled_content(theme, c.on_surface, &[]) };
                let secondary = if row.enabled { c.on_surface_variant } else { disabled_content(theme, c.on_surface_variant, &[]) };
                let mut x = o.x + inset + inner;
                if row.kind == RowKind::Back {
                    if let Some(icon) = &mut row.leading {
                        icon.draw_centered(ui.painter, Point::new(x + icon_w * 0.5, mid), ui.fade(c.primary));
                    }
                    x += icon_w + gap;
                } else if has_leading {
                    let center = Point::new(x + icon_w * 0.5, mid);
                    // İkon yuvası kipinde ikonsuz ya da dokusu henüz yüklenmemiş öğe: yer tutucu.
                    if page.icon_slots && row.leading.as_ref().is_none_or(IconView::is_pending) {
                        let tone = if row.enabled { 1.0 } else { DISABLED_CONTENT };
                        let color = ui.fade(crate::style::icon_placeholder(theme).multiply_alpha(tone));
                        crate::icon::draw_icon_placeholder(ui.painter, theme, center, icon_w, color);
                    } else if let Some(icon) = &mut row.leading {
                        icon.draw_centered(ui.painter, center, ui.fade(secondary));
                    }
                    x += icon_w + gap;
                }
                let right = o.x + width - inset - inner;
                if let Some(t) = &mut row.trailing {
                    // Alt menü chevron'u ikincil; ikonlu radyonun onay işareti vurgu renginde.
                    let tint = if row.submenu || !row.enabled { secondary } else { c.primary };
                    t.draw_centered(ui.painter, Point::new(right - icon_w * 0.5, mid), ui.fade(tint));
                }
                if let Some(s) = &mut row.shortcut {
                    let m = s.measure_in(ui.painter);
                    s.draw(ui.painter, Point::new(right - m.width, mid - m.height * 0.5), ui.fade(secondary));
                }
                if let Some(l) = &mut row.label {
                    let m = l.measure_in(ui.painter);
                    let color = if row.kind == RowKind::Back { c.primary } else { content };
                    l.draw(ui.painter, Point::new(x, mid - m.height * 0.5), ui.fade(color));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    fn model() -> MenuModel {
        MenuModel::new(vec![
            MenuItem::action(1, "Aç").with_icon(Icon::named(IconName::OpenInNew)).with_shortcut("Ctrl+O"),
            MenuItem::check(2, "Sessiz", false),
            MenuItem::separator(),
            MenuItem::submenu(
                3,
                "Daha fazla",
                vec![
                    MenuItem::radio(31, "Düşük", true),
                    MenuItem::radio(32, "Yüksek", false),
                    MenuItem::submenu(33, "En derin", vec![MenuItem::action(331, "Son")]),
                ],
            ),
            MenuItem::action(4, "Devre dışı").enabled(false),
            MenuItem::action(5, "Çık"),
        ])
    }

    fn open_menu() -> (Menu, TextSystem, Theme, Instant) {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut m = Menu::new(&theme);
        let anchor = MenuAnchor::below(Rect::new(400.0, 0.0, 32.0, 40.0));
        m.open(&mut text, &theme, model(), anchor, Rect::new(0.0, 40.0, 1000.0, 700.0), t0);
        (m, text, theme, t0)
    }

    #[test]
    fn debug_is_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let secret = MenuModel::new(vec![
            MenuItem::action(1, SECRET).with_shortcut(SECRET),
            MenuItem::submenu(2, SECRET, vec![MenuItem::check(21, SECRET, true)]),
        ]);
        assert_redacted(&secret);
        let mut m = Menu::new(&theme);
        m.open(&mut text, &theme, secret, MenuAnchor::below(Rect::new(0.0, 0.0, 32.0, 40.0)), Rect::new(0.0, 40.0, 1000.0, 700.0), t0);
        assert_redacted(&m);
    }

    fn settle(m: &mut Menu, t: Instant) {
        m.settle_phase(t);
        while m.is_animating(t) {
            m.settle_phase(t);
        }
        m.settle_phase(t);
    }

    #[test]
    fn opens_from_anchor_and_closes_back() {
        let (mut m, _text, theme, t0) = open_menu();
        // Tohum bağlantının altında, sıfır yükseklikte.
        let seed = m.grow.current(t0);
        assert_eq!(seed.height, 0.0);
        assert_eq!(seed.y, 40.0);
        assert!(m.shape(t0, &theme).is_some());
        let t1 = t0 + Duration::from_secs(3);
        settle(&mut m, t1);
        assert!(m.is_open());
        let open = m.grow.current(t1);
        assert!(open.height > 100.0 && open.y == 40.0);
        // Bağlantı merkezine göre ortalı.
        assert!((open.center().x - 416.0).abs() < 1.0);
        m.close(t1);
        let t2 = t1 + Duration::from_secs(3);
        settle(&mut m, t2);
        assert!(!m.is_visible());
        assert!(m.shape(t2, &theme).is_none());
    }

    #[test]
    fn keyboard_navigation_skips_separators_and_disabled() {
        let (mut m, mut text, theme, t0) = open_menu();
        let down = key(Key::Down);
        let ids: Vec<MenuItemId> = (0..5)
            .map(|_| {
                let _ = m.key(&mut text, &theme, &down, t0);
                m.page().rows[m.page().selected.unwrap()].id
            })
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 5, 1], "ayırıcı ve devre dışı atlanır, dolanır");
        let up = m.key(&mut text, &theme, &key(Key::Up), t0);
        assert!(up.repaint);
        assert_eq!(m.page().rows[m.page().selected.unwrap()].id, 5);
        let r = m.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(MenuEvent::Activated(5)));
        assert!(!m.is_open());
    }

    #[test]
    fn submenu_becomes_page_and_back_restores_selection() {
        let (mut m, mut text, theme, t0) = open_menu();
        let root_size = m.grow.target();
        for _ in 0..3 {
            let _ = m.key(&mut text, &theme, &key(Key::Down), t0);
        }
        let r = m.key(&mut text, &theme, &key(Key::Right), t0);
        assert_eq!(r.event, Some(MenuEvent::SubmenuOpened(3)));
        assert_eq!(m.depth(), 1);
        assert!(m.transition.is_some());
        // Alt sayfada geri başlığı var ve şekil yeni boyuta hedeflendi.
        assert_eq!(m.page().rows[0].kind, RowKind::Back);
        assert_ne!(m.grow.target().height, root_size.height);
        // Derinlik sınırsız: bir kat daha.
        let _ = m.key(&mut text, &theme, &key(Key::End), t0);
        let r = m.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(MenuEvent::SubmenuOpened(33)));
        assert_eq!(m.depth(), 2);
        assert!(m.key(&mut text, &theme, &key(Key::Backspace), t0).repaint);
        assert!(m.key(&mut text, &theme, &key(Key::Left), t0).repaint);
        assert_eq!(m.depth(), 0);
        assert_eq!(m.page().rows[m.page().selected.unwrap()].id, 3);
    }

    #[test]
    fn hover_click_activates_and_outside_dismisses() {
        let (mut m, mut text, theme, t0) = open_menu();
        let frame = m.grow.target();
        // İlk öğe: pad + yarım satır.
        let p = Point::new(frame.x + 30.0, frame.y + theme.space(2) + theme.space(5));
        let _ = m.pointer(&mut text, &theme, &motion(p.x, p.y), t0);
        assert_eq!(m.page().selected, Some(0));
        let _ = m.pointer(&mut text, &theme, &press(p.x, p.y), t0);
        let r = m.pointer(&mut text, &theme, &release(p.x, p.y), t0);
        assert_eq!(r.event, Some(MenuEvent::Activated(1)));

        let (mut m, mut text, theme, t0) = open_menu();
        let r = m.pointer(&mut text, &theme, &press(5.0, 900.0), t0);
        assert_eq!(r.event, Some(MenuEvent::Dismissed));
        assert!(!m.is_open());
    }

    #[test]
    fn back_header_click_goes_back() {
        let (mut m, mut text, theme, t0) = open_menu();
        let row = m.page().rows.iter().position(|r| r.id == 3).unwrap();
        let _ = m.activate(&mut text, &theme, row, t0);
        assert_eq!(m.depth(), 1);
        let frame = m.grow.target();
        let p = Point::new(frame.x + 30.0, frame.y + theme.space(2) + 5.0);
        let _ = m.pointer(&mut text, &theme, &press(p.x, p.y), t0);
        let r = m.pointer(&mut text, &theme, &release(p.x, p.y), t0);
        assert!(r.repaint);
        assert_eq!(m.depth(), 0);
    }

    #[test]
    fn model_update_keeps_stack_by_id() {
        let (mut m, mut text, theme, t0) = open_menu();
        let row = m.page().rows.iter().position(|r| r.id == 3).unwrap();
        let _ = m.activate(&mut text, &theme, row, t0);
        // Tembel doldurma: alt menüye öğe eklendi.
        let mut new = model();
        if let MenuItemKind::Submenu(c) = &mut new.items[3].kind {
            c.push(MenuItem::action(34, "Yeni"));
        }
        m.set_model(&mut text, &theme, new.clone(), t0);
        assert_eq!(m.depth(), 1);
        assert!(m.page().rows.iter().any(|r| r.id == 34));
        // Alt menü kaldırıldı: yığın köke düşer.
        new.items.remove(3);
        m.set_model(&mut text, &theme, new, t0);
        assert_eq!(m.depth(), 0);
    }

    #[test]
    fn typeahead_and_mnemonics() {
        let (mut m, mut text, theme, t0) = open_menu();
        let _ = m.key(&mut text, &theme, &typed('ç'), t0);
        assert_eq!(m.page().rows[m.page().selected.unwrap()].id, 5);
        assert_eq!(strip_mnemonic("_Dosya__adı"), "Dosya_adı");
        assert_eq!(model().find(331).unwrap().label, "Son");
    }

    #[test]
    fn min_width_wider_than_default_max_does_not_panic() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let model = MenuModel::new(vec![MenuItem::radio(1, "a", true)]);
        let page = build_page(&mut text, &theme, &model, Vec::new(), Rect::new(0.0, 0.0, 2000.0, 800.0), 405.0, false);
        assert_eq!(page.size.width, 405.0);
        let page = build_page(&mut text, &theme, &model, Vec::new(), Rect::new(0.0, 0.0, 300.0, 800.0), 405.0, false);
        assert!(page.size.width <= 300.0);
    }

    #[test]
    fn long_labels_are_ellipsized_within_max_width() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let long = "Çok uzun bir menü öğesi etiketi ".repeat(8);
        let model = MenuModel::new(vec![MenuItem::action(1, long)]);
        let page = build_page(&mut text, &theme, &model, Vec::new(), Rect::new(0.0, 0.0, 2000.0, 800.0), 0.0, false);
        assert!(page.size.width <= theme.space(90));
    }

    #[test]
    fn headers_are_skipped_and_icon_radio_gets_check() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let model = MenuModel::new(vec![
            MenuItem::header(10, "Önerilen"),
            MenuItem::radio(1, "Firefox", true).with_icon(Icon::named(IconName::Language)),
            MenuItem::radio(2, "Chromium", false),
            MenuItem::header(11, "Diğer"),
            MenuItem::action(3, "Düzenle"),
        ]);
        assert!(model.find(10).is_none(), "başlık find ile bulunmaz");
        let mut m = Menu::new(&theme);
        m.set_icon_slots(true);
        m.open(&mut text, &theme, model, MenuAnchor::below(Rect::new(400.0, 0.0, 32.0, 40.0)), Rect::new(0.0, 40.0, 1000.0, 700.0), t0);
        let page = m.page();
        assert_eq!(page.rows[0].kind, RowKind::Header);
        assert!(!page.rows[0].selectable());
        assert!(page.has_leading && page.icon_slots);
        // İkonlu (ya da yuva kipindeki) radyo: solda ikon, seçiliyse sağda onay; radyo glifi yok.
        assert_eq!(page.rows[1].leading.as_ref().map(|i| i.icon().clone()), Some(Icon::named(IconName::Language)));
        assert_eq!(page.rows[1].trailing.as_ref().map(|i| i.icon().clone()), Some(Icon::named(IconName::Check)));
        assert!(page.rows[2].leading.is_none(), "yuva kipinde ikonsuz radyo: yer tutucu (glif değil)");
        assert!(page.rows[2].trailing.is_none());
        // Gezinme başlıkları atlar ve dolanır.
        let ids: Vec<MenuItemId> = (0..4)
            .map(|_| {
                let _ = m.key(&mut text, &theme, &key(Key::Down), t0);
                m.page().rows[m.page().selected.unwrap()].id
            })
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 1]);
        // Harfle arama başlığı ("Diğer") seçmez; "d" → "Düzenle".
        let _ = m.key(&mut text, &theme, &key(Key::Char('d')), t0);
        assert_eq!(m.page().rows[m.page().selected.unwrap()].id, 3);
        // Başlıkta imleç Default; tıklama olay üretmez.
        let frame = m.input_rect().unwrap();
        let p = Point::new(frame.center().x, frame.y + theme.space(2) + theme.space(4));
        assert_eq!(m.cursor(p), Some(CursorShape::Default));
        let _ = m.pointer(&mut text, &theme, &press(p.x, p.y), t0);
        let r = m.pointer(&mut text, &theme, &release(p.x, p.y), t0);
        assert_eq!(r.event, None);
        assert!(m.is_open());
    }

    #[test]
    fn plain_radio_keeps_radio_glyph() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let model = MenuModel::new(vec![MenuItem::radio(1, "A", true), MenuItem::radio(2, "B", false)]);
        let page = build_page(&mut text, &theme, &model, Vec::new(), Rect::new(0.0, 0.0, 2000.0, 800.0), 0.0, false);
        assert_eq!(page.rows[0].leading.as_ref().map(|i| i.icon().clone()), Some(Icon::named(IconName::RadioChecked)));
        assert!(page.rows[0].trailing.is_none());
    }
}
