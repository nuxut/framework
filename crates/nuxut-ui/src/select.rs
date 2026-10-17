//! Açılır seçim: seçili değeri gösteren alan; tıklayınca [`Menu`]'nün radyo öğeleriyle
//! alanın altında (sığmazsa üstünde) blob olarak açılır. Menü kodu yeniden kullanılır.
//!
//! Seçenekler düz metin ([`Select::new`]) ya da [`SelectEntry`] listesi
//! ([`Select::with_entries`]): bölüm başlığı ve ayraç seçilemez; seçenek başına isteğe bağlı
//! ikon ([`Icon`]: semantik ad ya da uygulama ikonu dokusu). Seçili indeks her zaman
//! *seçenekler* arasındaki sıradır (başlık/ayraç sayılmaz) — düz metin API'siyle aynı.
//!
//! İkon yuvası: herhangi bir seçeneğin ikonu varsa (ya da [`Select::with_icon_slot`]) hem
//! alanda hem listede ikon yeri ayrılır; ikonsuz ya da dokusu henüz yüklenmemiş seçenekte
//! yer tutucu çizilir — ikon sonradan gelince ([`Select::set_option_icon`]) yerleşim zıplamaz.

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_render::{Border, BlobShape, RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::icon::{Icon, IconView, draw_icon_placeholder};
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::label::Label;
use crate::menu::{MAX_MENU_ITEMS, Menu, MenuAlign, MenuAnchor, MenuEvent, MenuItem, MenuItemId, MenuModel, MenuSide};
use crate::pressable::Pressable;
use crate::style::{DISABLED_CONTENT, FOCUS_RING, HAIRLINE, clip_text, disabled_content, field_fill, icon_placeholder};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// En fazla girdi (seçenek + başlık + ayraç; menü sayfası sınırı).
pub const MAX_SELECT_OPTIONS: usize = MAX_MENU_ITEMS;
/// Seçenek/başlık etiketinin en uzun hâli (bayt).
pub const MAX_SELECT_LABEL: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectEvent {
    /// Seçenek indeksi (başlık/ayraç sayılmaz).
    Changed(usize),
}

/// Seçilebilir bir seçenek: etiket + isteğe bağlı ikon.
#[derive(Clone, PartialEq)]
pub struct SelectOption {
    pub label: String,
    pub icon: Option<Icon>,
}

impl SelectOption {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), icon: None }
    }

    pub fn with_icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

impl From<String> for SelectOption {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

impl From<&str> for SelectOption {
    fn from(label: &str) -> Self {
        Self::new(label)
    }
}

/// Etiket kullanıcı verisi olabilir (uygulama adı): `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for SelectOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SelectOption").field("label", &crate::redact::Len(&self.label)).field("icon", &self.icon.is_some()).finish()
    }
}

/// Seçim listesinin bir satırı.
#[derive(Clone, PartialEq)]
pub enum SelectEntry {
    Option(SelectOption),
    /// Bölüm başlığı (ör. "Diğer"): seçilemez; klavye ve işaretçi atlar.
    Header(String),
    /// İnce ayraç: seçilemez.
    Separator,
}

impl SelectEntry {
    pub fn option(label: impl Into<String>) -> Self {
        Self::Option(SelectOption::new(label))
    }

    pub fn header(label: impl Into<String>) -> Self {
        Self::Header(label.into())
    }
}

impl From<SelectOption> for SelectEntry {
    fn from(option: SelectOption) -> Self {
        Self::Option(option)
    }
}

impl std::fmt::Debug for SelectEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Option(o) => o.fmt(f),
            Self::Header(h) => f.debug_tuple("Header").field(&crate::redact::Len(h)).finish(),
            Self::Separator => f.write_str("Separator"),
        }
    }
}

/// İç sıra: seçenekler `options`/`icons`'ta, başlık/ayraç yalnızca burada.
#[derive(Clone)]
enum Slot {
    Option(usize),
    Header(String),
    Separator,
}

/// Başlık/ayraç menü kimlikleri seçenek indekslerinden ayrı aralıkta.
const NON_OPTION_ID: MenuItemId = 1 << 32;

#[derive(Clone)]
pub struct Select {
    options: Vec<String>,
    icons: Vec<Option<Icon>>,
    slots: Vec<Slot>,
    /// İkon yuvası açık mı (bir seçeneğin ikonu var ya da çağıran istedi).
    icon_slot: bool,
    selected: usize,
    core: Pressable,
    label: Label,
    /// Alanda seçili seçeneğin ikonu (ikon yuvası kipinde).
    icon: IconView,
    /// Alan ikonu dokusu son çizimde bekliyordu (hazır olduğu karede yeniden boya).
    icon_pending: bool,
    chevron: IconView,
    menu: Menu,
    /// Menünün sığması gereken alan.
    bounds: Rect,
    /// Tema değişti, ölçüme bağlı yerleşim (etiket genişliği, açık menünün sayfaları) henüz
    /// yenilenmedi: ölçüm gerektirmeyen [`set_theme`](Select::set_theme) bırakır, ilk
    /// `set_rect`/`draw`/`draw_overlay`/`pointer`/`key` tembelce yeniler.
    stale: bool,
}

/// Seçenekler kullanıcı verisi olabilir (ağ, cihaz adları): `Debug` yalnızca sayı yazar.
impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("options", &crate::redact::Count(self.options.len()))
            .field("entries", &crate::redact::Count(self.slots.len()))
            .field("icon_slot", &self.icon_slot)
            .field("selected", &self.selected)
            .field("menu", &self.menu)
            .field("bounds", &self.bounds)
            .finish_non_exhaustive()
    }
}

impl Select {
    /// Düz metin seçenekler (başlıksız, ikonsuz).
    pub fn new(theme: &Theme, options: Vec<String>, selected: usize) -> Self {
        Self::with_entries(theme, options.into_iter().map(|o| SelectEntry::Option(o.into())).collect(), selected)
    }

    /// Başlık/ayraç ve ikon destekli girdiler. `selected`: seçenek indeksi (başlık/ayraç
    /// sayılmaz). En fazla [`MAX_SELECT_OPTIONS`] girdi alınır.
    pub fn with_entries(theme: &Theme, entries: Vec<SelectEntry>, selected: usize) -> Self {
        let mut options = Vec::new();
        let mut icons = Vec::new();
        let mut slots = Vec::new();
        for entry in entries.into_iter().take(MAX_SELECT_OPTIONS) {
            match entry {
                SelectEntry::Option(o) => {
                    slots.push(Slot::Option(options.len()));
                    options.push(clip_text(&o.label, MAX_SELECT_LABEL).to_owned());
                    icons.push(o.icon);
                }
                SelectEntry::Header(h) => slots.push(Slot::Header(clip_text(&h, MAX_SELECT_LABEL).to_owned())),
                SelectEntry::Separator => slots.push(Slot::Separator),
            }
        }
        let selected = selected.min(options.len().saturating_sub(1));
        let label = Label::new(options.get(selected).cloned().unwrap_or_default(), text_style(theme, TextRole::Body).with_max_lines(1));
        let icon_slot = icons.iter().any(Option::is_some);
        let field_icon = icons.get(selected).cloned().flatten().unwrap_or(Icon::Named(None));
        let mut menu = Menu::new(theme);
        menu.set_icon_slots(icon_slot);
        Self {
            options,
            icons,
            slots,
            icon_slot,
            selected,
            core: Pressable::new(theme),
            label,
            icon: IconView::with_role(theme, field_icon, TextRole::Icon),
            icon_pending: false,
            chevron: IconView::with_role(theme, Icon::named(IconName::ChevronDown), TextRole::IconSmall),
            menu,
            bounds: Rect::ZERO,
            stale: false,
        }
    }

    /// İkon yuvasını, hiçbir seçeneğin ikonu henüz yokken de açar (ikonlar sonradan
    /// [`set_option_icon`](Self::set_option_icon) ile gelecekse yerleşim zıplamasın).
    pub fn with_icon_slot(mut self, on: bool) -> Self {
        self.icon_slot = on || self.icons.iter().any(Option::is_some);
        self.menu.set_icon_slots(self.icon_slot);
        self
    }

    pub fn has_icon_slot(&self) -> bool {
        self.icon_slot
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Seçenek etiketleri (başlık/ayraç hariç; indeksler `selected` ile aynı).
    pub fn options(&self) -> &[String] {
        &self.options
    }

    /// Seçeneğin ikonu.
    pub fn option_icon(&self, index: usize) -> Option<&Icon> {
        self.icons.get(index).and_then(Option::as_ref)
    }

    /// Seçeneğin ikonunu değiştirir (ör. uygulama ikonu yüklendi ya da yüklenemedi → `None`,
    /// yer tutucu). İkon yuvası açılır. Alan ve açık liste bir sonraki çizim/girdide yenilenir.
    pub fn set_option_icon(&mut self, index: usize, icon: Option<Icon>) {
        let Some(slot) = self.icons.get_mut(index) else { return };
        if *slot == icon {
            return;
        }
        *slot = icon;
        if !self.icon_slot && slot.is_some() {
            self.icon_slot = true;
            self.menu.set_icon_slots(true);
        }
        // Etiket genişliği (yuva açıldıysa) ve açık liste bir sonraki çizim/girdide yenilenir.
        self.stale = true;
        if index == self.selected {
            self.icon.set_icon(self.icons[index].clone().unwrap_or(Icon::Named(None)));
        }
        self.core.damage.mark();
    }

    pub fn set_selected(&mut self, index: usize) {
        let index = index.min(self.options.len().saturating_sub(1));
        if index != self.selected {
            self.selected = index;
            self.label.set_text(self.options.get(index).map_or("", String::as_str));
            self.icon.set_icon(self.icons.get(index).cloned().flatten().unwrap_or(Icon::Named(None)));
            self.core.damage.mark();
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        self.core.set_enabled(enabled, now);
        if !enabled {
            self.menu.close(now);
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) {
        self.core.set_focus_visible(on);
    }

    /// Tema değişimi; `TextSystem` gerekmez. Ölçüme bağlı yerleşim (etiket genişliği, açık
    /// menünün sayfaları) bir sonraki `set_rect`/`draw`/`draw_overlay`/`pointer`/`key`'te
    /// yenilenir (çağıran zaten yeni kare ister: hasar işaretlenir).
    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.label.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
        self.chevron.set_theme(theme, now);
        self.icon.set_theme(theme, now);
        self.menu.set_theme_motion(theme, now);
        self.stale = true;
        self.core.damage.mark();
    }

    /// Bekleyen yerleşimi uygular: tema ([`set_theme`](Self::set_theme)) ya da ikon değişimi
    /// ([`set_option_icon`](Self::set_option_icon)); açık liste güncel modelle yeniden kurulur.
    fn refresh(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        if !std::mem::take(&mut self.stale) {
            return;
        }
        self.fit_label(theme);
        if self.menu.is_visible() {
            let model = self.menu_model();
            self.menu.set_model(text, theme, model, now);
        }
    }

    fn fit_label(&mut self, theme: &Theme) {
        let w = self.core.rect.width - theme.space(4) * 2.0 - self.chevron.size().width - theme.space(2) - self.leading_width(theme);
        self.label.set_max_width(Some(w.max(0.0)));
    }

    /// Alandaki ikon yuvasının genişliği (ikon + boşluk; yuva kapalıysa 0).
    fn leading_width(&self, theme: &Theme) -> f32 {
        if self.icon_slot { self.icon.size().width + theme.space(3) } else { 0.0 }
    }

    /// Yükseklik `space(12)`; genişlik çağıranın.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(12))
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        // Etiket genişliği ölçüm istemez; menü yenilemesi (ölçüm) `stale` ile ertelenir.
        if self.core.set_rect(rect) || self.stale {
            self.fit_label(theme);
        }
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    /// Menünün sığması gereken alan (ör. yüzey ya da kart).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    pub fn is_open(&self) -> bool {
        self.menu.is_open()
    }

    /// Menü modeli: seçenek kimliği = seçenek indeksi; başlık/ayraç ayrı aralıkta.
    fn menu_model(&self) -> MenuModel {
        let items = self
            .slots
            .iter()
            .enumerate()
            .map(|(k, slot)| match slot {
                Slot::Option(i) => {
                    let item = MenuItem::radio(*i as MenuItemId, self.options[*i].clone(), *i == self.selected);
                    match &self.icons[*i] {
                        Some(icon) => item.with_icon(icon.clone()),
                        None => item,
                    }
                }
                Slot::Header(h) => MenuItem::header(NON_OPTION_ID + k as MenuItemId, h.clone()),
                Slot::Separator => MenuItem { id: NON_OPTION_ID + k as MenuItemId, ..MenuItem::separator() },
            })
            .collect();
        MenuModel::new(items)
    }

    fn open(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        let anchor = MenuAnchor { rect: self.core.rect, side: MenuSide::Below, align: MenuAlign::Start, gap: theme.space(1) };
        let bounds = if self.bounds.is_empty() { Rect::new(0.0, 0.0, f32::MAX / 4.0, f32::MAX / 4.0) } else { self.bounds };
        self.menu.set_min_width(self.core.rect.width);
        self.menu.open(text, theme, self.menu_model(), anchor, bounds, now);
        self.core.damage.mark();
    }

    fn after_menu(&mut self, r: Response<MenuEvent>) -> Response<SelectEvent> {
        match r.event {
            Some(MenuEvent::Activated(id)) if (id as usize) < self.options.len() => {
                let i = id as usize;
                let changed = i != self.selected;
                self.set_selected(i);
                if changed { Response::event(SelectEvent::Changed(i)) } else { Response::repaint() }
            }
            Some(_) => Response::repaint(),
            None => r.discard(),
        }
    }

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<SelectEvent> {
        self.refresh(text, theme, now);
        if self.menu.is_open() {
            let r = self.menu.pointer(text, theme, event, now);
            return self.after_menu(r);
        }
        let r = self.core.pointer(event, now);
        if r.event.is_some() {
            self.open(text, theme, now);
            return Response::repaint();
        }
        r.discard()
    }

    /// Kapalıyken Enter/Space/Aşağı açar; açıkken menüye gider.
    pub fn key(&mut self, text: &mut TextSystem, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<SelectEvent> {
        self.refresh(text, theme, now);
        if self.menu.is_open() {
            let r = self.menu.key(text, theme, event, now);
            return self.after_menu(r);
        }
        if self.core.enabled && event.state == KeyState::Pressed && matches!(event.key, Key::Enter | Key::Space | Key::Down) {
            self.core.pulse(now);
            self.open(text, theme, now);
            return Response::repaint();
        }
        Response::none()
    }

    /// İmleç: açık listenin öğeleri önce; alan `Pointer` (devre dışı `Default`).
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.menu.cursor(p).or_else(|| self.core.cursor(p))
    }

    /// Alan.
    pub fn draw(&mut self, ui: &mut Ui) {
        self.refresh(ui.painter.text_system(), ui.theme, ui.now);
        let open = self.menu.is_open();
        // Doku ikonu GPU'ya ulaşana kadar kare istenir; ulaştığı karede yeniden boyanır.
        let pending = self.icon_slot && self.icon.is_pending();
        if pending || self.icon_pending {
            self.core.damage.mark();
        }
        if pending {
            ui.request_frame();
        }
        self.icon_pending = pending;
        let leading = self.icon_slot.then(|| FieldLeading { placeholder: pending || matches!(self.icon.icon(), Icon::Named(None)), icon: &mut self.icon });
        draw_field_with(ui, &mut self.core, leading, &mut self.label, &mut self.chevron, open);
    }

    /// Açık menünün blob şekli (shell kendi alanına ekler); ardından [`draw_menu_content`].
    pub fn menu_shape(&mut self, now: Instant, theme: &Theme) -> Option<BlobShape> {
        self.menu.shape(now, theme)
    }

    pub fn draw_menu_content(&mut self, ui: &mut Ui) {
        self.refresh(ui.painter.text_system(), ui.theme, ui.now);
        self.menu.draw_content(ui);
    }

    /// Bağımsız kullanım: açık menüyü (gölgeli) kaplama katmanına çizer ([`Ui::overlay`]) ve
    /// girdi bölgesini bildirir ([`Ui::overlay_input`]). Alanın hemen ardından, kırpılı
    /// kapsayıcının (kaydırma alanı, kart) içinden çağrılabilir; menü kırpılmaz, en üstte
    /// çizilir. Sığdırma alanı [`set_bounds`](Self::set_bounds) (ör. `ui.surface_rect()`).
    pub fn draw_overlay(&mut self, ui: &mut Ui) {
        self.refresh(ui.painter.text_system(), ui.theme, ui.now);
        self.menu.draw_overlay(ui);
    }

    /// Açıkken menünün input bölgesi.
    pub fn input_rect(&self) -> Option<Rect> {
        self.menu.input_rect()
    }
}

/// Alanın solundaki ikon (Select ikon yuvası). `placeholder`: ikon yok ya da dokusu yüklenmedi.
pub(crate) struct FieldLeading<'a> {
    pub icon: &'a mut IconView,
    pub placeholder: bool,
}

/// Dolgulu alan (Select, DatePicker, TimePicker): `field_fill` zemin, kenar yalnızca kenar kipi
/// açıksa, `active` (açık) iken altta 2px `primary` vurgu; solda etiket, sağda ikon.
pub(crate) fn draw_field(ui: &mut Ui, core: &mut Pressable, label: &mut Label, trailing: &mut IconView, active: bool) {
    draw_field_with(ui, core, None, label, trailing, active);
}

/// [`draw_field`] + isteğe bağlı sol ikon (etiket ikonun ardından başlar).
pub(crate) fn draw_field_with(ui: &mut Ui, core: &mut Pressable, leading: Option<FieldLeading<'_>>, label: &mut Label, trailing: &mut IconView, active: bool) {
    let theme = ui.theme;
    let c = &theme.colors;
    let rect = core.rect;
    let radii = Corners::uniform(theme.shape.radius_small);
    ui.painter.rect(rect, &RectStyle::filled(ui.fade(field_fill(theme)), radii));
    if let Some(outline) = theme.outline() {
        let style = RectStyle { fill: Color::TRANSPARENT, radii, border: Some(Border { width: HAIRLINE, color: ui.fade(outline) }) };
        ui.painter.rect(rect, &style);
    }
    if active {
        let line = Rect::new(rect.x, rect.bottom() - FOCUS_RING, rect.width, FOCUS_RING);
        ui.painter.rect(line, &RectStyle::filled(ui.fade(c.primary), Corners::uniform(FOCUS_RING * 0.5)));
    }
    let field = field_fill(theme);
    core.interaction.draw_state_layer_under(ui, rect, radii, c.on_surface, field, &[c.on_surface, c.on_surface_variant]);
    core.draw_focus(ui, rect, radii);
    let content = if core.enabled { c.on_surface } else { disabled_content(theme, c.on_surface, &[field]) };
    let pad = theme.space(4);
    let mut x = rect.x + pad;
    if let Some(FieldLeading { icon, placeholder }) = leading {
        let size = icon.size().width;
        let center = Point::new(x + size * 0.5, rect.center().y);
        if placeholder {
            let tone = if core.enabled { 1.0 } else { DISABLED_CONTENT };
            draw_icon_placeholder(ui.painter, theme, center, size, ui.fade(icon_placeholder(theme).multiply_alpha(tone)));
        } else {
            let tint = if core.enabled { c.on_surface_variant } else { disabled_content(theme, c.on_surface_variant, &[field]) };
            icon.draw_centered(ui.painter, center, ui.fade(tint));
        }
        x += size + theme.space(3);
    }
    let ls = label.measure_in(ui.painter);
    label.draw(ui.painter, Point::new(x, rect.center().y - ls.height * 0.5), ui.fade(content));
    let iw = trailing.size().width;
    trailing.draw_centered(ui.painter, Point::new(rect.right() - pad - iw * 0.5, rect.center().y), ui.fade(c.on_surface_variant));
    core.finish(ui, core.extent());
}

// Erişilebilirlik (ADR-0011).
impl Select {
    /// `Role::ComboBox`: değer seçili seçeneğin etiketi, `expanded` menü açıkken; açıkken
    /// seçenekler çocuk `Role::ListBoxOption` (seçili olan `selected`, konum). Ad (satır
    /// başlığı) çağıranın: `.label(..)`. AT `Click` menüyü açar (tıklama yolu); seçim klavyeyle.
    pub fn access<'t>(&self, tree: &'t mut crate::access::AccessTree, key: impl std::hash::Hash) -> crate::access::AccessNode<'t> {
        use crate::access::Role;
        let open = self.is_open();
        let value = self.options.get(self.selected).map_or("", String::as_str);
        let id = tree
            .push(key, Role::ComboBox)
            .value(value)
            .bounds(self.core.rect)
            .enabled(self.core.enabled)
            .expanded(open)
            .clickable()
            .id();
        if open {
            let n = self.options.len();
            tree.within(id, |tree| {
                for (i, o) in self.options.iter().enumerate() {
                    tree.push(i, Role::ListBoxOption).label(o).selected(i == self.selected).position(i + 1, n);
                }
            });
        }
        tree.get(id).expect("az önce eklendi")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn debug_is_redacted() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = Select::new(&theme, vec![crate::redact::SECRET.into(), "gizli-2".into()], 0);
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.pointer(&mut text, &theme, &press(50.0, 30.0), t0);
        let _ = s.pointer(&mut text, &theme, &release(50.0, 30.0), t0);
        assert!(s.is_open());
        crate::redact::assert_redacted(&s);
        crate::redact::assert_redacted(&crate::list::ListTrailing::Text(crate::redact::SECRET.into()));
    }

    #[test]
    fn opens_below_selects_and_reports_change() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = Select::new(&theme, vec!["Açık".into(), "Koyu".into(), "Sistem".into()], 2);
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.pointer(&mut text, &theme, &press(50.0, 30.0), t0);
        let _ = s.pointer(&mut text, &theme, &release(50.0, 30.0), t0);
        assert!(s.is_open());
        let menu = s.input_rect().unwrap();
        assert!(menu.y >= 58.0, "alanın altında: {menu:?}");
        assert!(menu.width >= 240.0, "alan genişliğinde");
        // Klavyeyle ilk seçeneği seç.
        let _ = s.key(&mut text, &theme, &key(Key::Home), t0);
        let r = s.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(SelectEvent::Changed(0)));
        assert_eq!(s.selected(), 0);
        assert!(!s.is_open());
    }

    #[test]
    fn set_theme_defers_measurement_until_input() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = Select::new(&theme, vec!["Açık".into(), "Koyu".into()], 0);
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        assert!(s.is_open());
        // TextSystem'siz tema değişimi: yerleşim kirli kalır, menü açık kalır.
        s.set_theme(&theme, t0);
        assert!(s.stale);
        assert!(s.is_open());
        // İlk girdi (TextSystem'li) yerleşimi yeniler; menü hâlâ çalışır.
        let _ = s.key(&mut text, &theme, &key(Key::End), t0);
        assert!(!s.stale);
        let r = s.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(SelectEvent::Changed(1)));
        // Kapalıyken tema değişimi + set_rect: etiket genişliği yenilenir, ölçüm gerekmez.
        s.set_theme(&theme, t0);
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        assert!(s.stale, "menü yenilemesi girdi/çizime kadar bekler");
    }

    #[test]
    fn flips_above_when_no_room() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = Select::new(&theme, (0..6).map(|i| format!("Seçenek {i}")).collect(), 0);
        s.set_rect(&theme, Rect::new(10.0, 540.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        assert!(s.input_rect().unwrap().bottom() <= 540.0);
    }

    fn sectioned(theme: &Theme) -> Select {
        Select::with_entries(
            theme,
            vec![
                SelectEntry::header("Önerilen"),
                SelectOption::new("Firefox").with_icon(IconName::Language).into(),
                SelectEntry::option("Chromium"),
                SelectEntry::Separator,
                SelectEntry::header("Diğer"),
                SelectEntry::option("Elle komut"),
            ],
            0,
        )
    }

    #[test]
    fn keyboard_skips_headers_and_separators() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = sectioned(&theme);
        assert_eq!(s.options(), ["Firefox", "Chromium", "Elle komut"], "başlık/ayraç seçenek sayılmaz");
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        assert!(s.is_open());
        // Home ilk *seçeneğe* gider (başlık atlanır); Aşağı ayraç + "Diğer" başlığını atlar.
        let _ = s.key(&mut text, &theme, &key(Key::Home), t0);
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let r = s.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(SelectEvent::Changed(2)));
        assert_eq!(s.selected(), 2);
        // Harfle arama başlığa ("Diğer") gitmez: "d" ile eşleşen seçenek yok → seçim değişmez.
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let _ = s.key(&mut text, &theme, &key(Key::Home), t0);
        let _ = s.key(&mut text, &theme, &key(Key::Char('d')), t0);
        let r = s.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(SelectEvent::Changed(0)), "Home'daki seçenek kaldı");
        // Dolanma: End → son seçenek, Aşağı → ilk seçenek (baştaki başlık atlanır).
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let _ = s.key(&mut text, &theme, &key(Key::End), t0);
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let r = s.key(&mut text, &theme, &key(Key::Enter), t0);
        assert_eq!(r.event, None, "zaten seçili (0): olay yok");
        assert_eq!(s.selected(), 0);
    }

    #[test]
    fn header_and_separator_are_not_clickable() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = sectioned(&theme);
        s.set_selected(1);
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let frame = s.input_rect().unwrap();
        // Satır düzeni: üst boşluk space(2), başlık space(8), seçenekler space(10), ayraç space(3).
        let header_y = frame.y + theme.space(2) + theme.space(4);
        let x = frame.x + frame.width * 0.5;
        assert_eq!(s.cursor(Point::new(x, header_y)), Some(CursorShape::Default), "başlıkta imleç Default");
        let _ = s.pointer(&mut text, &theme, &motion(x, header_y), t0);
        let _ = s.pointer(&mut text, &theme, &press(x, header_y), t0);
        let r = s.pointer(&mut text, &theme, &release(x, header_y), t0);
        assert_eq!(r.event, None);
        assert!(s.is_open(), "başlık tıklaması menüyü kapatmaz");
        let sep_y = frame.y + theme.space(2) + theme.space(8) + theme.space(10) * 2.0 + theme.space(3) * 0.5;
        let _ = s.pointer(&mut text, &theme, &press(x, sep_y), t0);
        let r = s.pointer(&mut text, &theme, &release(x, sep_y), t0);
        assert_eq!(r.event, None);
        assert!(s.is_open(), "ayraç tıklanmaz");
        // İlk seçenek satırı tıklanır: olay seçenek indeksiyle (0), başlık sayılmaz.
        let first_y = frame.y + theme.space(2) + theme.space(8) + theme.space(5);
        assert_eq!(s.cursor(Point::new(x, first_y)), Some(CursorShape::Pointer));
        let _ = s.pointer(&mut text, &theme, &motion(x, first_y), t0);
        let _ = s.pointer(&mut text, &theme, &press(x, first_y), t0);
        let r = s.pointer(&mut text, &theme, &release(x, first_y), t0);
        assert_eq!(r.event, Some(SelectEvent::Changed(0)));
        assert!(!s.is_open());
    }

    #[test]
    fn icon_slot_measurement() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let long = "Çok uzun bir varsayılan uygulama adı örneği";
        let mut plain = Select::new(&theme, vec![long.into(), "Kısa".into()], 0);
        let mut icons = Select::with_entries(
            &theme,
            vec![SelectOption::new(long).with_icon(IconName::Language).into(), SelectEntry::option("Kısa")],
            0,
        );
        assert!(!plain.has_icon_slot());
        assert!(icons.has_icon_slot(), "bir seçeneğin ikonu yuvayı açar");
        // Alan yüksekliği ikonla değişmez.
        assert_eq!(plain.preferred_size(&theme, 200.0), icons.preferred_size(&theme, 200.0));
        let rect = Rect::new(10.0, 10.0, 200.0, 48.0);
        let bounds = Rect::new(0.0, 0.0, 1600.0, 900.0);
        for s in [&mut plain, &mut icons] {
            s.set_rect(&theme, rect);
            s.set_bounds(bounds);
        }
        assert_eq!(plain.leading_width(&theme), 0.0);
        let lead = icons.leading_width(&theme);
        assert!(lead > 0.0);
        // Alan etiketi ikon yuvası kadar daralır.
        let pw = plain.label.max_width().unwrap();
        let iw = icons.label.max_width().unwrap();
        assert!((pw - iw - lead).abs() < 0.01, "{pw} {iw} {lead}");
        // Liste ikon sütunu kadar genişler (uzun etiket alan genişliğini aşar).
        let _ = plain.key(&mut text, &theme, &key(Key::Down), t0);
        let _ = icons.key(&mut text, &theme, &key(Key::Down), t0);
        let (pm, im) = (plain.input_rect().unwrap(), icons.input_rect().unwrap());
        assert!(im.width > pm.width, "ikon sütunu: {pm:?} {im:?}");
        // Satır yükseklikleri aynı: ikon satırı uzatmaz.
        assert_eq!(pm.height, im.height);
        // Yuva sonradan açılır (ikonlar yüklenecek): ilk ikon gelince yerleşim değişmez.
        let mut later = Select::new(&theme, vec![long.into(), "Kısa".into()], 0).with_icon_slot(true);
        later.set_rect(&theme, rect);
        let before = later.label.max_width();
        later.set_option_icon(0, Some(Icon::named(IconName::Language)));
        later.set_rect(&theme, rect);
        assert_eq!(later.label.max_width(), before, "yuva baştan ayrılmış: zıplama yok");
    }

    #[test]
    fn field_shows_selected_option_icon() {
        let theme = Theme::default();
        let mut s = Select::with_entries(
            &theme,
            vec![
                SelectEntry::header("Önerilen"),
                SelectOption::new("Firefox").with_icon(IconName::Language).into(),
                SelectOption::new("Terminal").with_icon(IconName::Terminal).into(),
                SelectEntry::option("İkonsuz"),
            ],
            1,
        );
        assert_eq!(s.icon.icon(), &Icon::named(IconName::Terminal), "seçili seçeneğin ikonu alanda");
        s.set_selected(0);
        assert_eq!(s.icon.icon(), &Icon::named(IconName::Language));
        // İkonsuz seçenek: alan yer tutucu çizer (boş ad).
        s.set_selected(2);
        assert_eq!(s.icon.icon(), &Icon::Named(None));
        // Seçili seçeneğin ikonu sonradan gelir → alan hemen günceller.
        s.set_option_icon(2, Some(Icon::named(IconName::Apps)));
        assert_eq!(s.icon.icon(), &Icon::named(IconName::Apps));
        assert_eq!(s.option_icon(2), Some(&Icon::named(IconName::Apps)));
        // Başka seçeneğin ikonu alanı değiştirmez; hatalı yükleme → None → yer tutucu.
        s.set_option_icon(0, None);
        assert_eq!(s.icon.icon(), &Icon::named(IconName::Apps));
        assert_eq!(s.option_icon(0), None);
        // Menü modeli: başlık seçilemez, ikonlar seçeneklerde, kimlik = seçenek indeksi.
        let m = s.menu_model();
        assert_eq!(m.items[0].kind, crate::menu::MenuItemKind::Header);
        assert!(!m.items[0].enabled);
        assert!(m.items[0].id >= NON_OPTION_ID);
        assert_eq!(m.items[2].id, 1);
        assert_eq!(m.items[2].icon, Some(Icon::named(IconName::Terminal)));
        assert_eq!(m.items[3].kind, crate::menu::MenuItemKind::Radio(true));
    }

    #[test]
    fn entries_debug_is_redacted() {
        let theme = Theme::default();
        let entries = vec![SelectEntry::header(crate::redact::SECRET), SelectEntry::option(crate::redact::SECRET)];
        crate::redact::assert_redacted(&entries);
        crate::redact::assert_redacted(&Select::with_entries(&theme, entries, 0));
    }

    #[test]
    fn icon_arriving_while_open_updates_list() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut s = Select::new(&theme, vec!["Terminal".into(), "Dosyalar".into()], 0).with_icon_slot(true);
        s.set_rect(&theme, Rect::new(10.0, 10.0, 240.0, 48.0));
        s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = s.key(&mut text, &theme, &key(Key::Down), t0);
        let before = s.input_rect().unwrap();
        s.set_option_icon(1, Some(Icon::named(IconName::Folder)));
        let _ = s.key(&mut text, &theme, &key(Key::End), t0);
        assert_eq!(s.menu.model().items[1].icon, Some(Icon::named(IconName::Folder)));
        assert_eq!(s.input_rect().unwrap(), before, "yuva baştan ayrılmış: liste boyutu değişmez");
        assert!(s.is_open());
    }
}
