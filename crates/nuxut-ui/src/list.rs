//! Liste: [`ListItem`] (başlık, alt metin, ön ikon, sağda değer/anahtar/ikon/ok) ve onları
//! sanallaştırılmış [`ScrollView`] içinde dizen [`List`].
//!
//! - **Gruplar:** [`List::set_groups`] öğeler arasına seçilemeyen, odaklanamayan başlık satırları
//!   koyar (`Caption`, `on_surface_variant`). Başlıklar öğe değildir: `items()` ve olayların
//!   `(usize, ListEvent)` indisi düz öğe indisidir; klavye gezinmesi başlıklara hiç uğramaz.
//!   İlk grup listenin başındaysa üstünde boşluk yoktur; sonraki grupların üstünde grup arası
//!   boşluk vardır.
//! - **Yoğunluk:** [`ListDensity::Compact`] tek satırı `space(10)` (kenar çubuğu), dolguyu
//!   `space(3)` yapar.
//! - **İkon şeridi:** [`List::set_rail`] yalnızca öncü ikonu satırın ortasında bırakır; başlık,
//!   alt metin ve sağ uç söner. Başlık üzerine gelince (gecikmeli) ve klavye odağında
//!   (beklemesiz) [`Tooltip`] olarak sağda görünür. Grup başlıkları ince ayırıcıya döner. Geçiş
//!   `spatial` yayıyla (kesilebilir); genişliği çağıran verir: [`List::animated_width`].

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::RectStyle;
use nuxut_theme::Theme;

use crate::access::{AccessNode, AccessTree, Role};
use crate::badge::{Pill, Separator};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, Modifiers, PointerEvent, PointerKind};
use crate::interaction::DOUBLE_CLICK;
use crate::selection::Selection;
use crate::label::Label;
use crate::menu::MenuSide;
use crate::pressable::Pressable;
use crate::scroll::ScrollView;
use crate::style::{FOCUS_RING, accent_on, disabled_content, tonal_container};
use crate::toggle::Toggle;
use crate::tooltip::Tooltip;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Satırın sağ ucu.
#[derive(Clone, PartialEq)]
pub enum ListTrailing {
    None,
    /// İkincil metin (değer: "%64", "Bağlı").
    Text(String),
    /// Metinli rozet ([`Pill`]: "Yeni", "Beta"); sağa yaslı, dikey ortalı. Zemin `tonal_container`,
    /// seçili (tonal) satırda `surface` (kaybolmasın); metin okunur vurgu ([`crate::pill_colors`]).
    Pill(String),
    /// Aç/kapa anahtarı; satıra tıklamak da çevirir.
    Toggle(bool),
    Icon(Icon),
    /// Alt sayfaya gider (›).
    Chevron,
}

impl std::fmt::Debug for ListTrailing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Text(s) => f.debug_tuple("Text").field(&crate::redact::Len(s)).finish(),
            Self::Pill(s) => f.debug_tuple("Pill").field(&crate::redact::Len(s)).finish(),
            Self::Toggle(on) => f.debug_tuple("Toggle").field(on).finish(),
            Self::Icon(i) => f.debug_tuple("Icon").field(i).finish(),
            Self::Chevron => f.write_str("Chevron"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListEvent {
    Clicked,
    /// Anahtarlı satırın yeni durumu.
    Toggled(bool),
}

/// Seçim kipindeki liste olayı ([`List::pointer_select`], [`List::key_select`]). Ayrı tür:
/// `ListEvent`'e varyant eklemek tüketicilerin kapsamlı eşlemelerini kırardı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListSelectEvent {
    /// Seçim kümesi değişti (`List::selection()`).
    Changed,
    /// Çift tık ya da Enter: odaktaki/tıklanan satır.
    Activated(usize),
    /// Anahtarlı satır çevrildi (seçimi etkilemez).
    Toggled(usize, bool),
}

/// Satır yoğunluğu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListDensity {
    /// Tek satır `space(14)`, alt metinli `space(18)`; dolgu `space(4)`.
    #[default]
    Regular,
    /// Tek satır `space(10)`, alt metinli `space(14)`; dolgu `space(3)` (kenar çubuğu).
    Compact,
}

impl ListDensity {
    /// Satırın yatay iç dolgusu.
    pub fn padding(self, theme: &Theme) -> f32 {
        match self {
            Self::Regular => theme.space(4),
            Self::Compact => theme.space(3),
        }
    }

    fn row_height(self, theme: &Theme, two_lines: bool) -> f32 {
        match (self, two_lines) {
            (Self::Regular, false) => theme.space(14),
            (Self::Regular, true) => theme.space(18),
            (Self::Compact, false) => theme.space(10),
            (Self::Compact, true) => theme.space(14),
        }
    }

    /// Grup başlığının metin satırı (başlık metni bu bandın altına yaslanır).
    fn header_row(self, theme: &Theme) -> f32 {
        match self {
            Self::Regular => theme.space(7),
            Self::Compact => theme.space(6),
        }
    }

    /// İlk olmayan grubun üstündeki boşluk (öğeler arası `space(1)`'e ek).
    fn group_gap(self, theme: &Theme) -> f32 {
        match self {
            Self::Regular => theme.space(4),
            Self::Compact => theme.space(3),
        }
    }

    /// İkon şeridinde grup başlığının yerini alan ayırıcı bandı.
    fn rail_separator(self, theme: &Theme) -> f32 {
        match self {
            Self::Regular => theme.space(4),
            Self::Compact => theme.space(3),
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Şerit ilerlemesinde metinlerin opaklığı: yolun ilk yarısında söner (dar alanda üst üste
/// binmesin), dönüşte son yarıda belirir.
fn text_alpha(rail: f32) -> f32 {
    (1.0 - rail.clamp(0.0, 1.0) * 2.0).clamp(0.0, 1.0)
}

#[derive(Debug, Clone)]
enum Trailing {
    None,
    Text(Label),
    Pill(Pill),
    Toggle(Box<Toggle>),
    Icon(Box<IconView>),
}

#[derive(Debug, Clone)]
pub struct ListItem {
    core: Pressable,
    title: Label,
    subtitle: Option<Label>,
    leading: Option<IconView>,
    trailing: Trailing,
    selected: bool,
    density: ListDensity,
    /// İkon şeridi ilerlemesi (0 = tam satır, 1 = şerit); `List` her yerleşimde verir.
    rail: f32,
}

impl ListItem {
    pub fn new(theme: &Theme, title: impl Into<String>) -> Self {
        Self {
            core: Pressable::new(theme),
            title: Label::new(title, text_style(theme, TextRole::Body).with_max_lines(1)),
            subtitle: None,
            leading: None,
            trailing: Trailing::None,
            selected: false,
            density: ListDensity::Regular,
            rail: 0.0,
        }
    }

    pub fn with_subtitle(mut self, theme: &Theme, text: impl Into<String>) -> Self {
        self.subtitle = Some(Label::new(text, text_style(theme, TextRole::Caption).with_max_lines(1)));
        self
    }

    pub fn with_leading(mut self, theme: &Theme, icon: Icon) -> Self {
        self.leading = Some(IconView::new(theme, icon));
        self
    }

    pub fn with_trailing(mut self, theme: &Theme, trailing: ListTrailing) -> Self {
        self.set_trailing(theme, trailing);
        self
    }

    pub fn set_trailing(&mut self, theme: &Theme, trailing: ListTrailing) {
        self.trailing = match trailing {
            ListTrailing::None => Trailing::None,
            ListTrailing::Text(t) => Trailing::Text(Label::new(t, text_style(theme, TextRole::Caption).with_max_lines(1))),
            ListTrailing::Pill(t) => {
                let mut p = Pill::new(theme, t).with_max_width(Some(Self::pill_max_text(theme)));
                p.set_enabled(self.core.enabled);
                Trailing::Pill(p)
            }
            ListTrailing::Toggle(on) => Trailing::Toggle(Box::new(Toggle::new(theme, on))),
            ListTrailing::Icon(i) => Trailing::Icon(Box::new(IconView::with_role(theme, i, TextRole::IconSmall))),
            ListTrailing::Chevron => Trailing::Icon(Box::new(IconView::with_role(theme, Icon::named(IconName::ChevronRight), TextRole::IconSmall))),
        };
        self.core.damage.mark();
    }

    /// Öndeki ikon (ör. doku yüklenince değiştirilir).
    pub fn set_leading(&mut self, theme: &Theme, icon: Option<Icon>) {
        self.leading = icon.map(|i| IconView::new(theme, i));
        self.core.damage.mark();
    }

    pub fn title(&self) -> &str {
        self.title.text()
    }

    pub fn set_title(&mut self, text: &str) {
        if self.title.set_text(text) {
            self.core.damage.mark();
        }
    }

    pub fn set_subtitle(&mut self, theme: &Theme, text: Option<&str>) {
        match (text, &mut self.subtitle) {
            (Some(t), Some(l)) => {
                l.set_text(t);
            }
            (Some(t), None) => self.subtitle = Some(Label::new(t, text_style(theme, TextRole::Caption).with_max_lines(1))),
            (None, _) => self.subtitle = None,
        }
        self.core.damage.mark();
    }

    /// Sağdaki değer metni (yalnızca `Text` ve `Pill` sağ uçları için).
    pub fn set_value(&mut self, text: &str) {
        let changed = match &mut self.trailing {
            Trailing::Text(l) => l.set_text(text),
            Trailing::Pill(p) => p.set_text(text),
            _ => false,
        };
        if changed {
            self.core.damage.mark();
        }
    }

    /// Anahtarın dış durumu (yalnızca `Toggle` sağ ucu için).
    pub fn set_toggle(&mut self, on: bool, now: Instant) {
        if let Trailing::Toggle(t) = &mut self.trailing {
            t.set_on(on, now);
        }
    }

    pub fn set_selected(&mut self, selected: bool) {
        if self.selected != selected {
            self.selected = selected;
            self.core.damage.mark();
        }
    }

    pub fn is_selected(&self) -> bool {
        self.selected
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        self.core.set_enabled(enabled, now);
        match &mut self.trailing {
            Trailing::Toggle(t) => {
                t.set_enabled(enabled, now);
            }
            Trailing::Pill(p) => p.set_enabled(enabled),
            _ => {}
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    /// Yoğunluk (`List` kendi yoğunluğunu her yerleşimde öğelerine verir; tek başına
    /// kullanılan satır için). Değişirse sonraki `set_rect` metinleri yeniden sığdırır.
    pub fn set_density(&mut self, density: ListDensity) {
        if self.density != density {
            self.density = density;
            // Boyut aynı kalsa da dolgu değişti: sonraki yerleşim metin genişliklerini yeniler.
            self.core.rect = Rect::ZERO;
            self.core.damage.mark();
        }
    }

    fn set_rail(&mut self, rail: f32) {
        if self.rail != rail {
            self.rail = rail;
            self.core.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.title.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
        if let Some(s) = &mut self.subtitle {
            s.set_style(text_style(theme, TextRole::Caption).with_max_lines(1));
        }
        if let Some(l) = &mut self.leading {
            l.set_theme(theme, now);
        }
        match &mut self.trailing {
            Trailing::Text(l) => {
                l.set_style(text_style(theme, TextRole::Caption).with_max_lines(1));
            }
            Trailing::Pill(p) => {
                p.set_theme(theme, now);
                p.set_max_width(Some(Self::pill_max_text(theme)));
            }
            Trailing::Toggle(t) => t.set_theme(theme, now),
            Trailing::Icon(i) => i.set_theme(theme, now),
            Trailing::None => {}
        }
    }

    /// Yoğunluğa göre: `Regular` tek satır `space(14)`, alt metinli `space(18)`; `Compact`
    /// `space(10)` / `space(14)`.
    pub fn height(&self, theme: &Theme) -> f32 {
        self.density.row_height(theme, self.subtitle.is_some())
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    /// Yerleşim; metinler kalan genişliğe göre `…` ile kısalır.
    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        let moved_only = self.core.rect.size() == rect.size();
        if !self.core.set_rect(rect) || moved_only {
            if let Trailing::Toggle(t) = &mut self.trailing {
                t.set_rect(Self::toggle_rect(theme, self.density, rect));
            }
            return;
        }
        let (tx, right) = self.text_span(theme, rect);
        let w = (right - tx).max(0.0);
        self.title.set_max_width(Some(w));
        if let Some(s) = &mut self.subtitle {
            s.set_max_width(Some(w));
        }
        if let Trailing::Toggle(t) = &mut self.trailing {
            t.set_rect(Self::toggle_rect(theme, self.density, rect));
        }
    }

    fn toggle_rect(theme: &Theme, density: ListDensity, rect: Rect) -> Rect {
        let s = Size::new(theme.space(13), theme.space(8));
        Rect::new(rect.right() - density.padding(theme) - s.width, rect.center().y - s.height * 0.5, s.width, s.height)
    }

    /// Metin sütununun x aralığı.
    fn text_span(&self, theme: &Theme, rect: Rect) -> (f32, f32) {
        let pad = self.density.padding(theme);
        let mut x = rect.x + pad;
        if let Some(l) = &self.leading {
            x += l.size().width + pad;
        }
        let trailing_w = match &self.trailing {
            Trailing::None => 0.0,
            Trailing::Text(_) => theme.space(24),
            // Ölçülmüş hap genişliği (ilk çizimden önce en geniş hâli); `draw` ölçünce metin
            // sütunu yeniden sığdırılır.
            Trailing::Pill(p) => match p.rect().width {
                w if w > 0.0 => w + pad,
                _ => theme.space(24),
            },
            Trailing::Toggle(_) => theme.space(13) + pad,
            Trailing::Icon(i) => i.size().width + pad,
        };
        (x, rect.right() - pad - trailing_w)
    }

    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<ListEvent> {
        let r = self.core.pointer(event, now);
        if let Trailing::Toggle(t) = &mut self.trailing {
            // Anahtar satırın hover'ını izler; tıklamayı satır yapar.
            let inside = self.core.rect.contains(event.position);
            t.set_visual_state(inside && event.kind != PointerKind::Leave, false, now);
        }
        self.after(r, now)
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<ListEvent> {
        let r = self.core.key(event, now);
        self.after(r, now)
    }

    fn after(&mut self, r: Response, now: Instant) -> Response<ListEvent> {
        if r.event.is_none() {
            return r.discard();
        }
        if let Trailing::Toggle(t) = &mut self.trailing {
            let on = !t.is_on();
            t.set_on(on, now);
            return Response::event(ListEvent::Toggled(on));
        }
        Response::event(ListEvent::Clicked)
    }

    /// İmleç: tıklanabilir satır `Pointer`, devre dışı `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.core.rect;
        let radii = Corners::uniform(theme.shape.radius_medium);
        let bg = if self.selected { tonal_container(theme) } else { nuxut_core::Color::TRANSPARENT };
        if self.selected {
            ui.painter.rect(rect, &RectStyle::filled(ui.fade(bg), radii));
        }
        let accent = accent_on(theme, bg);
        self.core.interaction.draw_state_layer_under(ui, rect, radii, c.on_surface, bg, &[c.on_surface, c.on_surface_variant, accent]);
        // Odak halkası satırın içinde: kaydırma alanının kırpmasına ve komşu satıra taşmaz
        // (tam genişlik satırda dışa doğru halka kenarlarda kesiliyordu).
        let inner = FOCUS_RING * 2.0;
        let ring_radii = Corners::uniform((theme.shape.radius_medium - inner).max(0.0));
        self.core.draw_focus_on(ui, rect.expand(-inner), ring_radii, &[bg]);
        let enabled = self.core.enabled;
        let dim = |col: nuxut_core::Color| if enabled { col } else { disabled_content(theme, col, &[bg]) };
        let pad = self.density.padding(theme);
        let mid = rect.center().y;
        if let Some(icon) = &mut self.leading {
            let w = icon.size().width;
            let color = if self.selected { accent } else { c.on_surface_variant };
            // Şeritte ikon satırın ortasına kayar (yay hafif taşabilir; kırpılmaz).
            let x = lerp(rect.x + pad + w * 0.5, rect.center().x, self.rail);
            icon.draw_centered(ui.painter, Point::new(x, mid), ui.fade(dim(color)));
        }
        let alpha = text_alpha(self.rail);
        if alpha > 0.0 {
            ui.with_alpha(alpha, |ui| self.draw_text(ui, rect, pad, dim));
        }
        self.core.finish(ui, self.core.extent());
    }

    fn draw_text(&mut self, ui: &mut Ui, rect: Rect, pad: f32, dim: impl Fn(nuxut_core::Color) -> nuxut_core::Color) {
        let theme = ui.theme;
        let c = &theme.colors;
        let mid = rect.center().y;
        if let Trailing::Pill(p) = &mut self.trailing {
            // Hap ölçülür ve sağa yaslanır; genişliği değiştiyse metin sütunu yeniden sığar
            // (Label aynı genişlikte yeniden şekillenmez).
            let s = p.size(ui.painter.text_system(), theme);
            p.place_inline(ui.painter.text_system(), theme, rect.right() - pad - s.width, mid);
            p.set_on_tonal(self.selected);
            let (tx, right) = self.text_span(theme, rect);
            let w = (right - tx).max(0.0);
            self.title.set_max_width(Some(w));
            if let Some(s) = &mut self.subtitle {
                s.set_max_width(Some(w));
            }
        }
        let (tx, _) = self.text_span(theme, rect);
        let ts = self.title.measure_in(ui.painter);
        let gap = theme.space(1) * 0.5;
        match &mut self.subtitle {
            Some(sub) => {
                let ss = sub.measure_in(ui.painter);
                let top = mid - (ts.height + gap + ss.height) * 0.5;
                self.title.draw(ui.painter, Point::new(tx, top), ui.fade(dim(c.on_surface)));
                sub.draw(ui.painter, Point::new(tx, top + ts.height + gap), ui.fade(dim(c.on_surface_variant)));
            }
            None => self.title.draw(ui.painter, Point::new(tx, mid - ts.height * 0.5), ui.fade(dim(c.on_surface))),
        }
        match &mut self.trailing {
            Trailing::None => {}
            Trailing::Text(l) => {
                let m = l.measure_in(ui.painter);
                l.draw(ui.painter, Point::new(rect.right() - pad - m.width, mid - m.height * 0.5), ui.fade(dim(c.on_surface_variant)));
            }
            Trailing::Icon(i) => {
                let w = i.size().width;
                i.draw_centered(ui.painter, Point::new(rect.right() - pad - w * 0.5, mid), ui.fade(dim(c.on_surface_variant)));
            }
            Trailing::Pill(p) => p.draw(ui),
            Trailing::Toggle(t) => t.draw(ui),
        }
    }

    /// Sağ uç hapının metin sınırı (`space(30)`; taşan `…`).
    fn pill_max_text(theme: &Theme) -> f32 {
        theme.space(30)
    }
}

/// Öğeler arasındaki grup başlığı (öğe değil).
#[derive(Debug, Clone)]
struct Group {
    /// Grubun ilk öğesinin indisi.
    start: usize,
    label: Label,
    /// Son yerleşimde başlık bandının üstü (içerik koordinatı) ve yüksekliği; öğe sayısının
    /// dışında kalan grup çizilmez (`height == 0`).
    top: f32,
    height: f32,
}

/// İpucunun kaynağı (yalnızca şerit kipinde).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TipSource {
    None,
    Pointer(usize),
    Focus(usize),
}

/// Dikey, kaydırılabilir, sanallaştırılmış liste.
#[derive(Debug, Clone)]
pub struct List {
    items: Vec<ListItem>,
    groups: Vec<Group>,
    density: ListDensity,
    /// 0 = tam satır, 1 = ikon şeridi (`spatial` yayı).
    rail: Animated<f32>,
    /// Son yerleşimin şerit ilerlemesi (değişince yeniden yerleşim).
    laid_rail: f32,
    tip: Tooltip,
    tip_source: TipSource,
    /// Tıklanan satır: imleç ayrılana kadar ipucu yeniden açılmaz.
    tip_block: Option<usize>,
    scroll: ScrollView,
    rect: Rect,
    starts: Vec<f32>,
    total: f32,
    focus: Option<usize>,
    /// Seçim kipi (`with_selection`): `pointer_select`/`key_select` ile tıklama ve oklar seçer.
    selection: Option<Selection>,
    /// Son tık: öğe ve an (çift tık).
    last_click: Option<(usize, Instant)>,
    /// Son yerleştirilen (görünen) satırlar.
    visible: std::ops::Range<usize>,
    /// Grup başlıkları/ayırıcılar için (öğeler kendi hasarını bildirir).
    damage: DamageState,
}

impl List {
    pub fn new(theme: &Theme) -> Self {
        Self {
            items: Vec::new(),
            groups: Vec::new(),
            density: ListDensity::Regular,
            rail: Animated::new(0.0, theme.motion.spatial),
            laid_rail: 0.0,
            tip: Tooltip::new(theme),
            tip_source: TipSource::None,
            tip_block: None,
            scroll: ScrollView::new(theme),
            rect: Rect::ZERO,
            starts: Vec::new(),
            total: 0.0,
            focus: None,
            selection: None,
            last_click: None,
            visible: 0..0,
            damage: DamageState::new(),
        }
    }

    pub fn with_items(mut self, items: Vec<ListItem>) -> Self {
        self.items = items;
        self.starts.clear();
        self
    }

    pub fn with_groups<S: AsRef<str>>(mut self, theme: &Theme, groups: &[(usize, S)]) -> Self {
        self.set_groups(theme, groups);
        self
    }

    pub fn with_density(mut self, density: ListDensity) -> Self {
        self.set_density(density);
        self
    }

    /// Animasyonsuz başlangıç kipi.
    pub fn with_rail(mut self, rail: bool) -> Self {
        self.rail.jump_to(if rail { 1.0 } else { 0.0 });
        self.starts.clear();
        self
    }

    pub fn items(&self) -> &[ListItem] {
        &self.items
    }

    /// Öğeleri değiştirdikten sonra `set_rect` (yükseklikler yeniden hesaplanır). Grup
    /// indisleri kaydırılmaz: öğe ekleyip çıkaran gerekirse `set_groups`'u yeniden çağırır.
    pub fn items_mut(&mut self) -> &mut Vec<ListItem> {
        self.starts.clear();
        &mut self.items
    }

    /// Grup başlıkları: `(grubun ilk öğesinin indisi, başlık)`. Sırasız verilebilir; aynı
    /// indiste ilk başlık kalır. Öğe sayısını aşan indisli başlık çizilmez. Boş dilim grupları
    /// kaldırır.
    pub fn set_groups<S: AsRef<str>>(&mut self, theme: &Theme, groups: &[(usize, S)]) {
        let style = text_style(theme, TextRole::Caption).with_max_lines(1);
        let mut out: Vec<Group> = groups
            .iter()
            .map(|(start, title)| Group { start: *start, label: Label::new(title.as_ref(), style.clone()), top: 0.0, height: 0.0 })
            .collect();
        out.sort_by_key(|g| g.start);
        out.dedup_by_key(|g| g.start);
        self.groups = out;
        self.starts.clear();
        self.damage.mark();
    }

    /// Grubun başlığı (`start` = grubun ilk öğesi).
    pub fn group_title(&self, start: usize) -> Option<&str> {
        self.groups.iter().find(|g| g.start == start).map(|g| g.label.text())
    }

    pub fn density(&self) -> ListDensity {
        self.density
    }

    /// Yoğunluk; sonraki yerleşimde (ya da çizimde) uygulanır.
    pub fn set_density(&mut self, density: ListDensity) {
        if self.density != density {
            self.density = density;
            self.starts.clear();
            self.damage.mark();
        }
    }

    /// İkon şeridi kipine (ya da geri) `spatial` yayıyla geçer; yarıda tersine çevrilebilir.
    /// Geçiş sürerken çağıran her kare genişliği [`Self::animated_width`] ile verip
    /// `set_rect` çağırmalı (liste kendi içini her karede yeniden yerleştirir).
    pub fn set_rail(&mut self, rail: bool, now: Instant) {
        let target = if rail { 1.0 } else { 0.0 };
        if self.rail.target() != target {
            self.rail.set_target(target, now);
            self.damage.mark();
            if !rail {
                self.hide_tip(now);
            }
        }
    }

    /// Hedef kip şerit mi (geçiş sürüyor olabilir).
    pub fn is_rail(&self) -> bool {
        self.rail.target() >= 0.5
    }

    /// Geçiş ilerlemesi, `[0, 1]`'e kırpılmış (0 = tam satır, 1 = şerit).
    pub fn rail_progress(&self, now: Instant) -> f32 {
        self.rail.value(now).clamp(0.0, 1.0)
    }

    pub fn is_rail_animating(&mut self, now: Instant) -> bool {
        !self.rail.is_settled(now)
    }

    /// Şerit genişliği: `space(16)` (64 px).
    pub fn rail_width(theme: &Theme) -> f32 {
        theme.space(16)
    }

    /// Geçişin bu anki genişliği: tam kipteki genişlik `full` ile şerit genişliği arası.
    pub fn animated_width(&self, theme: &Theme, full: f32, now: Instant) -> f32 {
        lerp(full, Self::rail_width(theme), self.rail_progress(now)).round()
    }

    /// Klavye odağındaki öğe.
    pub fn focused(&self) -> Option<usize> {
        self.focus
    }

    /// Klavye odağı listeden çıktı: halka kalkar, odak ipucu kapanır.
    pub fn blur(&mut self, now: Instant) {
        if let Some(f) = self.focus.take()
            && let Some(item) = self.items.get_mut(f)
        {
            item.set_focus_visible(false);
        }
        if matches!(self.tip_source, TipSource::Focus(_)) {
            self.hide_tip(now);
        }
    }

    /// Bekleyen ipucunun belirme anı (uygulama zamanlayıcısı için; çizim de `Ui::wake_at` ile
    /// bildirir).
    pub fn tooltip_deadline(&self) -> Option<Instant> {
        self.tip.deadline()
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.scroll.set_theme(theme, now);
        self.rail.set_motion(theme.motion.spatial, now);
        self.tip.set_theme(theme, now);
        let style = text_style(theme, TextRole::Caption).with_max_lines(1);
        for g in &mut self.groups {
            g.label.set_style(style.clone());
        }
        for i in &mut self.items {
            i.set_theme(theme, now);
        }
        self.starts.clear();
        self.damage.mark();
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn content_height(&self) -> f32 {
        self.total
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        let t = self.rail_progress(now);
        if self.rect != rect || self.laid_rail != t {
            self.damage.mark();
        }
        self.rect = rect;
        self.starts.clear();
        self.laid_rail = t;
        let d = self.density;
        let gap = theme.space(1);
        let mut y = 0.0;
        let mut g = 0;
        for g in &mut self.groups {
            g.height = 0.0;
        }
        for (i, item) in self.items.iter_mut().enumerate() {
            item.set_density(d);
            while g < self.groups.len() && self.groups[g].start <= i {
                if self.groups[g].start == i {
                    let first = i == 0;
                    let full = d.header_row(theme) + if first { 0.0 } else { d.group_gap(theme) };
                    let rail = if first { 0.0 } else { d.rail_separator(theme) };
                    let h = lerp(full, rail, t);
                    self.groups[g].top = y;
                    self.groups[g].height = h;
                    y += h;
                }
                g += 1;
            }
            self.starts.push(y);
            y += item.height(theme) + gap;
        }
        self.total = (y - gap).max(0.0);
        let pad = d.padding(theme);
        for g in &mut self.groups {
            g.label.set_max_width(Some((rect.width - pad * 2.0).max(0.0)));
        }
        self.scroll.set_geometry(rect, self.total, now);
    }

    /// Görünen öğeleri geçerli kaydırma konumuna yerleştirir.
    pub(crate) fn place_visible(&mut self, theme: &Theme, now: Instant) -> std::ops::Range<usize> {
        let range = self.scroll.visible_range(&self.starts, now);
        let origin = self.scroll.content_origin(now);
        // İkon kayması yayın taşmasını korur (kırpılmamış değer); metin opaklığı kırpılır.
        let rail = self.rail.value(now);
        for i in range.clone() {
            let h = self.items[i].height(theme);
            let r = Rect::new(origin.x, origin.y + self.starts[i], self.rect.width, h);
            self.items[i].set_rect(theme, r);
            self.items[i].set_rail(rail);
        }
        self.visible = range.clone();
        range
    }

    fn hide_tip(&mut self, now: Instant) {
        self.tip.hide(now);
        self.tip_source = TipSource::None;
    }

    /// Şerit kipinde işaretçinin altındaki satırın ipucu. `true`: görünüş/bekleme değişti.
    fn pointer_tip(&mut self, event: &PointerEvent, hovered: Option<usize>, now: Instant) -> bool {
        let before = (self.tip.is_visible(), self.tip.deadline());
        if !self.is_rail() {
            if self.tip_source != TipSource::None || before != (false, None) {
                self.hide_tip(now);
            }
            return before != (self.tip.is_visible(), self.tip.deadline());
        }
        if self.tip_block.is_some() && self.tip_block != hovered {
            self.tip_block = None;
        }
        match event.kind {
            PointerKind::Press(_) => {
                self.tip_block = hovered;
                self.hide_tip(now);
            }
            PointerKind::Scroll { .. } => self.hide_tip(now),
            _ => match hovered {
                Some(i) if self.tip_block != Some(i) => {
                    let item = &self.items[i];
                    let _ = self.tip.hover(item.title.text(), item.rect(), MenuSide::Right, Rect::ZERO, now);
                    self.tip_source = TipSource::Pointer(i);
                }
                Some(_) => {}
                None => {
                    if matches!(self.tip_source, TipSource::Pointer(_)) {
                        self.tip.unhover(now);
                        self.tip_source = TipSource::None;
                    }
                }
            },
        }
        before != (self.tip.is_visible(), self.tip.deadline())
    }

    /// Olay `(öğe indisi, olay)` döner.
    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<(usize, ListEvent)> {
        self.ensure_layout(theme, now);
        let mut out = self.scroll.pointer(theme, event, now).discard();
        if matches!(event.kind, PointerKind::Scroll { .. }) || self.scroll.is_panning() {
            // Kaydırma sırasında altta kalan öğelerin hover'ı güncellenir.
            out.repaint = true;
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

    fn pointer_content(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<(usize, ListEvent)> {
        let mut out = Response::none();
        let range = self.place_visible(theme, now);
        let inside = self.rect.contains(event.position);
        let hovered = (inside && event.kind != PointerKind::Leave)
            .then(|| range.clone().find(|&i| self.items[i].rect().contains(event.position)))
            .flatten();
        out.repaint |= self.pointer_tip(event, hovered, now);
        for i in range {
            // Görünüm dışında kalan (kırpılmış) kısma tıklama öğeye gitmez.
            let ev = if inside { *event } else { event.with_kind(event.position, PointerKind::Leave) };
            let r = self.items[i].pointer(&ev, now);
            out.repaint |= r.repaint;
            if let Some(e) = r.event {
                if let Some(old) = self.focus.take()
                    && let Some(item) = self.items.get_mut(old)
                {
                    item.set_focus_visible(false);
                }
                return Response::event((i, e));
            }
        }
        out
    }

    /// Yukarı/Aşağı odak, Enter/Space etkinleştirir. Grup başlıkları öğe olmadığından
    /// gezinme onları atlar; grubun ilk öğesine gelince başlığı da görünür kılınır.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<(usize, ListEvent)> {
        if event.state == KeyState::Released || self.items.is_empty() {
            return Response::none();
        }
        self.ensure_layout(theme, now);
        let n = self.items.len();
        let next = match (event.key, self.focus) {
            (Key::Down, Some(f)) => Some((f + 1).min(n - 1)),
            (Key::Up, Some(f)) => Some(f.saturating_sub(1)),
            (Key::Down | Key::Up, None) => Some(0),
            (Key::Home, _) => Some(0),
            (Key::End, _) => Some(n - 1),
            (Key::Enter | Key::Space, Some(f)) => {
                return self.items[f].key(event, now).map(|e| (f, e));
            }
            _ => return Response::none(),
        };
        if let Some(old) = self.focus {
            self.items[old].set_focus_visible(false);
        }
        self.focus = next;
        if let Some(f) = next {
            self.items[f].set_focus_visible(true);
            let top = self.starts.get(f).copied().unwrap_or(0.0);
            let header = self.groups.iter().find(|g| g.start == f && g.height > 0.0).map_or(top, |g| g.top);
            self.scroll.ensure_visible(header, top + self.items[f].height(theme), now);
            if self.is_rail() {
                // Çizim görünür olunca bağlar (kaydırma sürüyorsa yer değiştirerek izler).
                self.tip_source = TipSource::Focus(f);
                self.tip_block = None;
            }
        }
        Response::repaint()
    }

    // ── Seçim kipi (çoklu seçim; dosya seçici liste görünümü) ───────────────────────────

    /// Seçim kipini açar: `pointer_select`/`key_select` tıklama ve okları [`Selection`]
    /// kurallarıyla seçime çevirir (Ctrl/Shift tık, Shift/Ctrl ok, Ctrl+Space, Ctrl+A); satırların
    /// `set_selected` görünüşü seçimden yönetilir. `multiple == false`: tekli seçim.
    pub fn with_selection(mut self, multiple: bool) -> Self {
        let mut sel = Selection::new(multiple);
        sel.reset(self.items.len());
        self.selection = Some(sel);
        self
    }

    /// Seçim kipindeyse seçim (öğe sayısı değiştiyse bir sonraki girdide sıfırlanır).
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// Olaysız: yalnız `i` seçili, odak orada (seçim kipinde).
    pub fn select_only(&mut self, i: usize) {
        self.ensure_selection();
        if let Some(sel) = &mut self.selection {
            sel.select_only(i);
        }
        self.sync_selection();
    }

    fn ensure_selection(&mut self) {
        let n = self.items.len();
        if let Some(sel) = &mut self.selection
            && sel.len() != n
        {
            sel.reset(n);
            self.last_click = None;
        }
    }

    /// Satırların seçili görünüşünü seçimle eşler.
    fn sync_selection(&mut self) {
        let Some(sel) = &self.selection else { return };
        for (i, item) in self.items.iter_mut().enumerate() {
            item.set_selected(sel.is_selected(i));
        }
    }

    /// Seçim kipinde işaretçi: tık seçer (`modifiers`: `App::modifiers_changed`'den), çift tık
    /// `Activated`, anahtarlı satır `Toggled`. Seçim kipi kapalıysa `pointer` gibi davranır
    /// (tık `Activated`).
    pub fn pointer_select(&mut self, theme: &Theme, event: &PointerEvent, modifiers: Modifiers, now: Instant) -> Response<ListSelectEvent> {
        self.ensure_selection();
        let r = self.pointer(theme, event, now);
        let repaint = r.repaint;
        match r.event {
            Some((i, ListEvent::Toggled(on))) => Response::event(ListSelectEvent::Toggled(i, on)),
            Some((i, ListEvent::Clicked)) => {
                let Some(sel) = &mut self.selection else { return Response::event(ListSelectEvent::Activated(i)) };
                let double = self.last_click.is_some_and(|(j, t)| j == i && now.saturating_duration_since(t) < DOUBLE_CLICK);
                if double && !modifiers.ctrl && !modifiers.shift {
                    self.last_click = None;
                    return Response::event(ListSelectEvent::Activated(i));
                }
                self.last_click = Some((i, now));
                let changed = sel.click(i, modifiers);
                self.focus = Some(i);
                self.sync_selection();
                if changed { Response::event(ListSelectEvent::Changed) } else { Response::repaint() }
            }
            None => Response::none().with_repaint(repaint),
        }
    }

    /// Seçim kipinde klavye: Yukarı/Aşağı/Home/End odağı taşır ve seçer (Shift aralık, Ctrl yalnız
    /// odak), Space odaktakini seçer (Ctrl+Space ekler/çıkarır), Ctrl+A hepsi, Enter `Activated`.
    /// Anahtarlı satırda Space `Toggled`. Seçim kipi kapalıysa `key` gibi.
    pub fn key_select(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<ListSelectEvent> {
        if event.state == KeyState::Released || self.items.is_empty() {
            return Response::none();
        }
        self.ensure_selection();
        let m = event.modifiers;
        let toggle_row = |l: &Self, f: usize| matches!(l.items[f].trailing, Trailing::Toggle(_));
        match (event.key, self.focus) {
            (Key::Enter, Some(f)) => return Response::event(ListSelectEvent::Activated(f)),
            (Key::Char('a' | 'A'), _) if m.ctrl => {
                let changed = self.selection.as_mut().is_some_and(Selection::select_all);
                self.sync_selection();
                return if changed { Response::event(ListSelectEvent::Changed) } else { Response::none() };
            }
            (Key::Space, Some(f)) if !toggle_row(self, f) && self.selection.is_some() => {
                let sel = self.selection.as_mut().expect("az önce denetlendi");
                let changed = if m.ctrl {
                    let _ = sel.navigate(f, Modifiers { ctrl: true, ..Modifiers::default() });
                    sel.toggle_cursor()
                } else {
                    sel.select_only(f)
                };
                self.sync_selection();
                return if changed { Response::event(ListSelectEvent::Changed) } else { Response::repaint() };
            }
            _ => {}
        }
        let r = self.key(theme, event, now);
        match r.event {
            Some((i, ListEvent::Toggled(on))) => return Response::event(ListSelectEvent::Toggled(i, on)),
            Some((i, ListEvent::Clicked)) => return Response::event(ListSelectEvent::Activated(i)),
            None => {}
        }
        let moved = matches!(event.key, Key::Up | Key::Down | Key::Home | Key::End);
        if let (true, Some(f), Some(sel)) = (moved, self.focus, self.selection.as_mut()) {
            let changed = sel.navigate(f, m);
            self.sync_selection();
            if changed {
                return Response::event(ListSelectEvent::Changed);
            }
        }
        r.discard()
    }

    /// `items_mut`/`set_groups`/`set_density` sonrası ya da şerit geçişinde yerleşim eskiyse
    /// yeniden hesaplar.
    fn ensure_layout(&mut self, theme: &Theme, now: Instant) {
        if self.starts.len() != self.items.len() || self.laid_rail != self.rail_progress(now) {
            let r = self.rect;
            self.set_rect(theme, r, now);
        }
    }

    /// İmleç: kaydırma çubuğu `Default`, görünen satırlar kendi imleci, boş alan ve grup
    /// başlıkları `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) && !self.scroll.is_dragging_bar() {
            return None;
        }
        if let Some(c) = self.scroll.cursor(p) {
            return Some(c);
        }
        // Satırların son çizimdeki yerleri görünen içerikle aynı.
        let c = self.items.get(self.visible.clone()).and_then(|items| items.iter().find_map(|i| i.cursor(p)));
        Some(c.unwrap_or(CursorShape::Default))
    }

    /// İpucunu bu karenin yerleşimine bağlar: odak ipucu satır görünürken beklemesiz,
    /// işaretçi ipucu görünürken satırı izler (şerit geçişi, kaydırma).
    fn sync_tip(&mut self, now: Instant) {
        match self.tip_source {
            TipSource::Focus(i) if self.is_rail() && self.visible.contains(&i) => {
                let item = &self.items[i];
                self.tip.show(item.title.text(), item.rect(), MenuSide::Right, Rect::ZERO, now);
            }
            TipSource::Focus(_) => self.tip.hide(now),
            TipSource::Pointer(i) if self.tip.is_visible() => {
                if self.visible.contains(&i) {
                    let item = &self.items[i];
                    self.tip.show(item.title.text(), item.rect(), MenuSide::Right, Rect::ZERO, now);
                } else {
                    self.hide_tip(now);
                }
            }
            _ => {}
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let now = ui.now;
        let animating = !self.rail.is_settled(now);
        self.ensure_layout(theme, now);
        let range = self.place_visible(theme, now);
        self.sync_tip(now);
        let t = self.rail_progress(now);
        let pad = self.density.padding(theme);
        let width = self.rect.width;
        let items = &mut self.items;
        let groups = &mut self.groups;
        self.scroll.draw(ui, |ui, origin, span| {
            for g in groups.iter_mut() {
                if g.height <= 0.0 || g.top >= span.end || g.top + g.height <= span.start {
                    continue;
                }
                let band = Rect::new(origin.x, origin.y + g.top, width, g.height);
                draw_header(ui, g, band, pad, t);
            }
            for i in range {
                items[i].draw(ui);
            }
        });
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.rect, animating);
        self.tip.draw(ui);
    }
}

/// Grup başlığı: tam kipte metin (bandın altına yaslı), şeritte ince ayırıcı; geçişte biri
/// sönerken öteki belirir.
fn draw_header(ui: &mut Ui, g: &mut Group, band: Rect, pad: f32, rail: f32) {
    let theme = ui.theme;
    let text = text_alpha(rail);
    if text > 0.0 {
        let m = g.label.measure_in(ui.painter);
        let y = band.bottom() - theme.space(1) - m.height;
        let color = ui.fade(theme.colors.on_surface_variant.multiply_alpha(text));
        g.label.draw(ui.painter, Point::new(band.x + pad, y), color);
    }
    // Liste başındaki grubun şeritte ayırıcısı yok (bandı sıfıra iner).
    let line = ((rail - 0.5) * 2.0).clamp(0.0, 1.0);
    if line > 0.0 && g.start > 0 {
        ui.with_alpha(line, |ui| Separator::horizontal(theme.space(3)).draw(ui, band));
    }
}

// Erişilebilirlik (ADR-0011).
impl ListItem {
    /// Anahtarlı satır `Role::Switch` (+ açık/kapalı), diğerleri `Role::Button` (satır
    /// etkinleşir). Ad başlıktan; açıklama alt metin ve sağdaki değer/rozet metni. Seçili satır
    /// `selected`. Ek ad gerekmez.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash) -> AccessNode<'t> {
        let role = if matches!(self.trailing, Trailing::Toggle(_)) { Role::Switch } else { Role::Button };
        let mut desc = String::new();
        let mut add = |s: &str| {
            if !s.is_empty() {
                if !desc.is_empty() {
                    desc.push_str(", ");
                }
                desc.push_str(s);
            }
        };
        if let Some(sub) = &self.subtitle {
            add(sub.text());
        }
        match &self.trailing {
            Trailing::Text(l) => add(l.text()),
            Trailing::Pill(p) => add(p.text()),
            _ => {}
        }
        let mut node = tree
            .push(key, role)
            .label(self.title.text())
            .description(&desc)
            .bounds(self.core.rect)
            .enabled(self.core.enabled)
            .clickable();
        if let Trailing::Toggle(t) = &self.trailing {
            node = node.toggled(t.is_on());
        }
        if self.selected { node.selected(true) } else { node }
    }
}

impl List {
    /// `Role::List` kabı; çocukları görünen grup başlıkları (`Role::Heading`) ve görünen satırlar
    /// (+ görünmese de odaktaki satır). Satır anahtarı `item_key(indis)`: öğenin kalıcı kimliği
    /// olmalı (ayar anahtarı, cihaz adresi…). Odaktaki satır ağacın odağı olur. Kabın adı
    /// (sayfa başlığı) çağıranın: `.label(..)`.
    pub fn access<'t, K: Hash>(&self, tree: &'t mut AccessTree, key: impl Hash, item_key: impl Fn(usize) -> K) -> AccessNode<'t> {
        // Seçim kipinde ListBox (çoklu: multiselectable) + ListBoxOption; değilse List.
        let role = if self.selection.is_some() { Role::ListBox } else { Role::List };
        let mut node = tree.push(key, role).bounds(self.rect);
        if self.selection.as_ref().is_some_and(Selection::is_multiple) {
            node.node_mut().set_multiselectable();
        }
        let id = node.id();
        let selecting = self.selection.is_some();
        let n = self.items.len();
        tree.within(id, |tree| {
            let mut group = self.groups.iter().filter(|g| g.height > 0.0).peekable();
            let emit = |tree: &mut AccessTree, i: usize| {
                let focused = self.focus == Some(i);
                let node = self.items[i].access(tree, item_key(i)).position(i + 1, n).focused_if(focused);
                if selecting && !matches!(self.items[i].trailing, Trailing::Toggle(_)) {
                    let mut node = node.selected(self.items[i].is_selected());
                    node.node_mut().set_role(Role::ListBoxOption);
                }
            };
            for i in self.visible.clone().filter(|&i| i < n) {
                while let Some(g) = group.next_if(|g| g.start <= i) {
                    if g.start >= self.visible.start {
                        tree.push(("heading", g.start), Role::Heading).label(g.label.text());
                    }
                }
                emit(tree, i);
            }
            if let Some(f) = self.focus.filter(|f| *f < n && !self.visible.contains(f)) {
                emit(tree, f);
            }
        });
        tree.get(id).expect("az önce eklendi")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;
    use crate::input::Button;
    use nuxut_render::{Recorder, TextSystem};
    use std::time::Duration;

    fn list(n: usize) -> (List, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let items = (0..n)
            .map(|i| {
                let it = ListItem::new(&theme, format!("Öğe {i}"));
                if i % 2 == 0 { it.with_trailing(&theme, ListTrailing::Toggle(false)) } else { it.with_subtitle(&theme, "alt") }
            })
            .collect();
        let mut l = List::new(&theme).with_items(items);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 320.0, 300.0), t0);
        (l, theme, t0)
    }

    /// Kenar çubuğu biçimi: 3 grup, tek satırlı ikonlu öğeler.
    fn nav(theme: &Theme, density: ListDensity) -> List {
        let items = (0..9).map(|i| ListItem::new(theme, format!("Sayfa {i}")).with_leading(theme, Icon::named(IconName::Tune))).collect();
        List::new(theme).with_items(items).with_groups(theme, &[(5, "Sistem"), (0, "Kişisel"), (2, "Masaüstü")]).with_density(density)
    }

    fn frame(list: &mut List, theme: &Theme, text: &mut TextSystem, now: Instant) -> (bool, Option<Instant>) {
        let mut rec = Recorder::new();
        let mut out = (false, None);
        rec.record(text, (400, 600), 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            list.draw(&mut ui);
            out = (ui.wants_frame(), ui.wake_deadline());
        });
        out
    }

    #[test]
    fn glyph_leading_uses_icon_slot_and_no_leading_has_none() {
        let theme = Theme::default();
        let r = Rect::new(0.0, 0.0, 320.0, 48.0);
        let icon = ListItem::new(&theme, "a").with_leading(&theme, Icon::named(IconName::Tune));
        let glyph = ListItem::new(&theme, "a").with_leading(&theme, Icon::glyph("😀"));
        let plain = ListItem::new(&theme, "a");
        assert_eq!(glyph.text_span(&theme, r), icon.text_span(&theme, r), "glif ikonla aynı yuvada");
        let pad = plain.density.padding(&theme);
        assert_eq!(plain.text_span(&theme, r).0, r.x + pad, "ikonsuz satırda yuva yok");
    }

    #[test]
    fn click_toggles_row_switch() {
        let (mut l, theme, t0) = list(50);
        let _ = l.pointer(&theme, &press(100.0, 20.0), t0);
        let r = l.pointer(&theme, &release(100.0, 20.0), t0);
        assert_eq!(r.event, Some((0, ListEvent::Toggled(true))));
        // İkinci satır (alt metinli) tıklama verir.
        let y = theme.space(14) + theme.space(1) + 10.0;
        let _ = l.pointer(&theme, &press(100.0, y), t0);
        assert_eq!(l.pointer(&theme, &release(100.0, y), t0).event, Some((1, ListEvent::Clicked)));
    }

    fn touch(kind: PointerKind, x: f32, y: f32) -> PointerEvent {
        PointerEvent::touch(Point::new(x, y), kind)
    }

    /// Runtime öykünmesinin dokunuş dizisi: Enter, Motion, Press … Release, Leave.
    fn touch_down(l: &mut List, theme: &Theme, x: f32, y: f32, t: Instant) {
        for k in [PointerKind::Enter, PointerKind::Motion, PointerKind::Press(Button::Left)] {
            assert_eq!(l.pointer(theme, &touch(k, x, y), t).event, None);
        }
    }

    #[test]
    fn touch_tap_activates_row_without_delay() {
        let (mut l, theme, t0) = list(50);
        touch_down(&mut l, &theme, 100.0, 20.0, t0);
        let r = l.pointer(&theme, &touch(PointerKind::Release(Button::Left), 100.0, 21.0), t0 + Duration::from_millis(30));
        assert_eq!(r.event, Some((0, ListEvent::Toggled(true))));
        let _ = l.pointer(&theme, &touch(PointerKind::Leave, 100.0, 21.0), t0);
    }

    #[test]
    fn touch_drag_scrolls_and_never_activates() {
        let (mut l, theme, t0) = list(50);
        touch_down(&mut l, &theme, 100.0, 200.0, t0);
        let mut t = t0;
        for i in 1..=10 {
            t += Duration::from_millis(16);
            assert_eq!(l.pointer(&theme, &touch(PointerKind::Motion, 100.0, 200.0 - 15.0 * i as f32), t).event, None);
        }
        assert!(l.scroll.offset(t) > 100.0, "içerik parmağı izledi: {}", l.scroll.offset(t));
        // Parmak başladığı satıra dönüp kalksa da tıklama yok.
        let _ = l.pointer(&theme, &touch(PointerKind::Motion, 100.0, 200.0), t);
        let r = l.pointer(&theme, &touch(PointerKind::Release(Button::Left), 100.0, 200.0), t);
        assert_eq!(r.event, None);
        let _ = l.pointer(&theme, &touch(PointerKind::Leave, 100.0, 200.0), t);
        // Hiçbir satır kurulu kalmadı: sonraki bırakma eylem üretmez.
        assert_eq!(l.pointer(&theme, &release(100.0, 200.0), t).event, None);
    }

    #[test]
    fn only_visible_items_are_placed() {
        let (mut l, theme, t0) = list(1000);
        let range = l.place_visible(&theme, t0);
        assert!(range.len() < 10, "{range:?}");
        assert!(l.content_height() > 50_000.0);
    }

    /// Seçim kipinde dosya listesi: anahtarsız satırlar.
    fn files(n: usize, multiple: bool) -> (List, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let items = (0..n).map(|i| ListItem::new(&theme, format!("dosya-{i}"))).collect();
        let mut l = List::new(&theme).with_items(items).with_selection(multiple);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 320.0, 700.0), t0);
        // Satır dikdörtgenleri ve görünen aralık (çizimde de kurulur).
        let _ = l.place_visible(&theme, t0);
        (l, theme, t0)
    }

    fn row_y(l: &List, i: usize) -> f32 {
        l.items[i].rect().center().y
    }

    fn sel_click(l: &mut List, theme: &Theme, i: usize, m: Modifiers, t: Instant) -> Option<ListSelectEvent> {
        let y = row_y(l, i);
        let _ = l.pointer_select(theme, &press(100.0, y), m, t);
        l.pointer_select(theme, &release(100.0, y), m, t).event
    }

    fn selected(l: &List) -> Vec<usize> {
        l.selection().unwrap().iter().collect()
    }

    #[test]
    fn selection_mode_pointer() {
        let (mut l, theme, t0) = files(10, true);
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        let shift = Modifiers { shift: true, ..Modifiers::default() };
        assert_eq!(sel_click(&mut l, &theme, 1, Modifiers::default(), t0), Some(ListSelectEvent::Changed));
        assert_eq!(sel_click(&mut l, &theme, 3, ctrl, t0), Some(ListSelectEvent::Changed));
        assert_eq!(sel_click(&mut l, &theme, 5, shift, t0), Some(ListSelectEvent::Changed));
        assert_eq!(selected(&l), [3, 4, 5]);
        assert!(l.items()[4].is_selected() && !l.items()[1].is_selected(), "görünüş seçimle eşli");
        // Çift tık etkinleştirir.
        let t1 = t0 + std::time::Duration::from_secs(2);
        let _ = sel_click(&mut l, &theme, 7, Modifiers::default(), t1);
        let t2 = t1 + std::time::Duration::from_millis(150);
        assert_eq!(sel_click(&mut l, &theme, 7, Modifiers::default(), t2), Some(ListSelectEvent::Activated(7)));
        // Öğe sayısı değişince seçim sıfırlanır.
        l.items_mut().truncate(4);
        let _ = l.pointer_select(&theme, &motion(100.0, 5.0), Modifiers::default(), t2);
        assert!(l.selection().unwrap().is_empty());
    }

    #[test]
    fn selection_mode_keyboard() {
        let (mut l, theme, t0) = files(10, true);
        let shift = Modifiers { shift: true, ..Modifiers::default() };
        let ctrl = Modifiers { ctrl: true, ..Modifiers::default() };
        assert_eq!(l.key_select(&theme, &key(Key::Down), t0).event, Some(ListSelectEvent::Changed));
        assert_eq!(selected(&l), [0]);
        let _ = l.key_select(&theme, &key_mod(Key::Down, shift), t0);
        let _ = l.key_select(&theme, &key_mod(Key::Down, shift), t0);
        assert_eq!(selected(&l), [0, 1, 2]);
        // Ctrl+ok yalnız odak; Ctrl+Space ekler.
        let _ = l.key_select(&theme, &key_mod(Key::Down, ctrl), t0);
        let _ = l.key_select(&theme, &key_mod(Key::Down, ctrl), t0);
        assert_eq!(selected(&l), [0, 1, 2]);
        assert_eq!(l.key_select(&theme, &key_mod(Key::Space, ctrl), t0).event, Some(ListSelectEvent::Changed));
        assert_eq!(selected(&l), [0, 1, 2, 4]);
        assert_eq!(l.key_select(&theme, &key(Key::Enter), t0).event, Some(ListSelectEvent::Activated(4)));
        assert_eq!(l.key_select(&theme, &key_mod(Key::Char('a'), ctrl), t0).event, Some(ListSelectEvent::Changed));
        assert_eq!(l.selection().unwrap().count(), 10);
        assert_eq!(l.key_select(&theme, &key(Key::Space), t0).event, Some(ListSelectEvent::Changed));
        assert_eq!(selected(&l), [4]);
        // Tekli kipte Shift yalnız odağı seçer.
        let (mut s, theme, t0) = files(5, false);
        let _ = s.key_select(&theme, &key(Key::Down), t0);
        let _ = s.key_select(&theme, &key_mod(Key::Down, shift), t0);
        assert_eq!(selected(&s), [1]);
    }

    #[test]
    fn selection_mode_access_is_listbox() {
        use accesskit_consumer::Tree;
        let (mut l, _theme, _t0) = files(4, true);
        l.select_only(2);
        let mut t = AccessTree::new();
        let names = ["a", "b", "c", "d"];
        l.access(&mut t, "files", |i| names[i]);
        let tree = Tree::new(t.finish(), true);
        let root = tree.state().root();
        let lb = root.children().next().unwrap();
        assert_eq!(lb.role(), Role::ListBox);
        assert!(lb.is_multiselectable());
        let opts: Vec<_> = lb.children().map(|c| (c.role(), c.is_selected())).collect();
        assert_eq!(opts[2], (Role::ListBoxOption, Some(true)));
        assert_eq!(opts[0], (Role::ListBoxOption, Some(false)));
    }

    #[test]
    fn keyboard_focus_scrolls_into_view() {
        let (mut l, theme, t0) = list(100);
        let _ = l.key(&theme, &key(Key::End), t0);
        assert_eq!(l.focus, Some(99));
        assert!(l.scroll.max_offset() > 0.0);
        let r = l.key(&theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some((99, ListEvent::Clicked)));
    }

    #[test]
    fn groups_sit_between_items_and_keep_flat_indices() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Regular);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 240.0, 600.0), t0);
        let d = ListDensity::Regular;
        let gap = theme.space(1);
        // İlk grup listenin başında: üstünde boşluk yok, yalnızca başlık bandı.
        assert_eq!(l.starts[0], d.header_row(&theme));
        // Sonraki grubun bandı = grup arası boşluk + başlık satırı.
        let row = theme.space(14);
        assert_eq!(l.starts[2], l.starts[1] + row + gap + d.group_gap(&theme) + d.header_row(&theme));
        // Gruplar sıralandı; indisler düz öğe indisi.
        assert_eq!(l.group_title(0), Some("Kişisel"));
        assert_eq!(l.group_title(5), Some("Sistem"));
        assert_eq!(l.items().len(), 9);
        // Başlık bandına tıklama hiçbir öğeye gitmez.
        let hy = l.groups[1].top + 4.0;
        let _ = l.pointer(&theme, &press(100.0, hy), t0);
        assert_eq!(l.pointer(&theme, &release(100.0, hy), t0).event, None);
        // Başlığın hemen altındaki öğe kendi indisini verir.
        let y = l.starts[2] + 4.0;
        let _ = l.pointer(&theme, &press(100.0, y), t0);
        assert_eq!(l.pointer(&theme, &release(100.0, y), t0).event, Some((2, ListEvent::Clicked)));
    }

    #[test]
    fn keyboard_skips_headers() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Compact);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 240.0, 600.0), t0);
        let mut seen = Vec::new();
        for _ in 0..9 {
            let _ = l.key(&theme, &key(Key::Down), t0);
            seen.push(l.focused().unwrap());
        }
        assert_eq!(seen, (0..9).collect::<Vec<_>>(), "başlıklar odak almaz");
        assert_eq!(l.key(&theme, &key(Key::Enter), t0).event, Some((8, ListEvent::Clicked)));
    }

    #[test]
    fn group_indices_past_end_are_ignored() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Regular).with_groups(&theme, &[(0, "A"), (0, "B"), (40, "Yok")]);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 240.0, 600.0), t0);
        assert_eq!(l.group_title(0), Some("A"), "aynı indiste ilk başlık kalır");
        assert_eq!(l.groups.last().unwrap().height, 0.0);
        let last = l.starts[8] + theme.space(14);
        assert_eq!(l.content_height(), last);
    }

    #[test]
    fn compact_rows_are_space_10() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Compact);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 240.0, 600.0), t0);
        l.place_visible(&theme, t0);
        assert_eq!(l.items()[0].rect().height, theme.space(10));
        assert_eq!(l.starts[1] - l.starts[0], theme.space(10) + theme.space(1));
    }

    #[test]
    fn rail_collapses_headers_to_separators_and_settles_idle() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Compact);
        let full = 240.0;
        l.set_rect(&theme, Rect::new(0.0, 0.0, full, 600.0), t0);
        let full_height = l.content_height();
        l.set_rail(true, t0);
        assert!(l.is_rail());
        // Geçiş sürerken kare istenir.
        let mid = t0 + Duration::from_millis(16);
        let w = l.animated_width(&theme, full, mid);
        assert!(w < full && w > List::rail_width(&theme));
        l.set_rect(&theme, Rect::new(0.0, 0.0, w, 600.0), mid);
        assert!(frame(&mut l, &theme, &mut text, mid).0);
        // Oturunca: şerit genişliği, ilk grubun bandı yok, sonrakiler ayırıcı bandı.
        let end = t0 + Duration::from_secs(5);
        assert_eq!(l.animated_width(&theme, full, end), List::rail_width(&theme));
        l.set_rect(&theme, Rect::new(0.0, 0.0, List::rail_width(&theme), 600.0), end);
        assert_eq!(l.starts[0], 0.0);
        assert_eq!(l.groups[1].height, ListDensity::Compact.rail_separator(&theme));
        assert!(l.content_height() < full_height);
        frame(&mut l, &theme, &mut text, end);
        // Boşta sıfır kare.
        assert_eq!(frame(&mut l, &theme, &mut text, end), (false, None));
        // İkon satırın ortasında (kırpılmamış hedef 1).
        assert_eq!(l.items[0].rail, 1.0);
    }

    #[test]
    fn rail_tooltip_on_hover_and_focus() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Compact).with_rail(true);
        let w = List::rail_width(&theme);
        l.set_rect(&theme, Rect::new(0.0, 0.0, w, 600.0), t0);
        frame(&mut l, &theme, &mut text, t0);
        // Üzerine gelince gecikmeli: bekleme anı bildirilir, kare istenmez.
        let y = l.starts[3] + 4.0;
        let r = l.pointer(&theme, &motion(w * 0.5, y), t0);
        assert!(r.repaint);
        let at = l.tooltip_deadline().expect("bekleme başladı");
        let (wants, wake) = frame(&mut l, &theme, &mut text, t0);
        assert_eq!(wake, Some(at));
        assert!(!wants || l.items[3].core.is_animating(t0), "yalnızca hover katmanı canlanır");
        // Zamanı gelince başlık ipucu olarak görünür.
        frame(&mut l, &theme, &mut text, at);
        assert!(l.tip.is_visible());
        assert_eq!(l.tip.text(), "Sayfa 3");
        // Ayrılınca kapanır.
        let _ = l.pointer(&theme, &motion(w * 0.5, 2000.0), at);
        assert!(!l.tip.is_visible());
        // Klavye odağı beklemesiz gösterir.
        let _ = l.key(&theme, &key(Key::Down), at);
        frame(&mut l, &theme, &mut text, at);
        assert!(l.tip.is_visible());
        assert_eq!(l.tip.text(), "Sayfa 0");
        l.blur(at);
        assert!(!l.tip.is_visible());
        // Tam kipe dönünce ipucu yok.
        let _ = l.key(&theme, &key(Key::Down), at);
        l.set_rail(false, at);
        frame(&mut l, &theme, &mut text, at);
        assert!(!l.tip.is_visible());
    }

    #[test]
    fn clicked_row_does_not_reopen_tip_until_pointer_leaves() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Compact).with_rail(true);
        let w = List::rail_width(&theme);
        l.set_rect(&theme, Rect::new(0.0, 0.0, w, 600.0), t0);
        frame(&mut l, &theme, &mut text, t0);
        let (x, y) = (w * 0.5, l.starts[1] + 4.0);
        let _ = l.pointer(&theme, &motion(x, y), t0);
        let at = l.tooltip_deadline().unwrap();
        frame(&mut l, &theme, &mut text, at);
        assert!(l.tip.is_visible());
        // Tıklama ipucunu kapatır ve aynı satırda yeniden açmaz.
        let _ = l.pointer(&theme, &press(x, y), at);
        assert_eq!(l.pointer(&theme, &release(x, y), at).event, Some((1, ListEvent::Clicked)));
        assert!(!l.tip.is_visible());
        let later = at + Duration::from_secs(3);
        let _ = l.pointer(&theme, &motion(x, y + 1.0), later);
        assert_eq!(l.tooltip_deadline(), None);
        frame(&mut l, &theme, &mut text, later);
        assert!(!l.tip.is_visible(), "tıklanan satır imleç ayrılana kadar ipucu açmaz");
        // Başka satıra geçip dönünce yeniden açılır.
        let other = l.starts[3] + 4.0;
        let _ = l.pointer(&theme, &motion(x, other), later);
        let _ = l.pointer(&theme, &motion(x, y), later);
        let back = l.tooltip_deadline().unwrap_or(later);
        frame(&mut l, &theme, &mut text, back);
        assert!(l.tip.is_visible());
        assert_eq!(l.tip.text(), "Sayfa 1");
    }

    #[test]
    fn scroll_closes_rail_tip_and_focus_tip_stays_idle() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Compact).with_rail(true);
        let w = List::rail_width(&theme);
        // Kısa görünüm: kaydırılabilir.
        l.set_rect(&theme, Rect::new(0.0, 0.0, w, 120.0), t0);
        frame(&mut l, &theme, &mut text, t0);
        let _ = l.key(&theme, &key(Key::Down), t0);
        frame(&mut l, &theme, &mut text, t0);
        assert!(l.tip.is_visible());
        // Görünür odak ipucu oturunca kare istemez (her karedeki `show` değişiklik değil).
        let settled = t0 + Duration::from_secs(2);
        frame(&mut l, &theme, &mut text, settled);
        assert_eq!(frame(&mut l, &theme, &mut text, settled), (false, None));
        // İşaretçi ipucu kaydırmada kapanır.
        let y = l.starts[0] + 4.0;
        let _ = l.pointer(&theme, &motion(w * 0.5, y), settled);
        let at = l.tooltip_deadline().unwrap_or(settled);
        frame(&mut l, &theme, &mut text, at);
        assert!(l.tip.is_visible());
        let _ = l.pointer(&theme, &PointerEvent::new(Point::new(w * 0.5, y), PointerKind::Scroll { delta: Point::new(0.0, 40.0), steps: Point::new(0.0, 1.0) }), at);
        assert!(!l.tip.is_visible());
        assert_eq!(l.tooltip_deadline(), None);
    }

    #[test]
    fn full_mode_has_no_tooltip() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut l = nav(&theme, ListDensity::Regular);
        l.set_rect(&theme, Rect::new(0.0, 0.0, 240.0, 600.0), t0);
        let _ = l.pointer(&theme, &motion(100.0, l.starts[1] + 4.0), t0);
        assert_eq!(l.tooltip_deadline(), None);
    }

    /// Hap sağ ucu: sağa yaslı, dikey ortalı; başlık sütunu hapın soluna kadar sığar; düz metin
    /// sağ ucu eski davranışında (sabit `space(24)` yuva); `set_value` hapın metnini değiştirir.
    #[test]
    fn pill_trailing_is_right_aligned_and_title_fits_left_of_it() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut text = TextSystem::new();
        let rect = Rect::new(0.0, 0.0, 320.0, theme.space(14));
        let mut item = ListItem::new(&theme, "Uzun bir başlık ki hapın altına girmesin diye kısalmalı").with_trailing(&theme, ListTrailing::Pill("Yeni".into()));
        item.set_rect(&theme, rect);
        let mut rec = Recorder::new();
        let mut draw = |item: &mut ListItem, text: &mut TextSystem| {
            rec.record(text, (400, 100), 1.0, |p| {
                let mut ui = Ui::new(p, &theme, t0);
                item.draw(&mut ui);
                assert!(!ui.wants_frame(), "boşta kare yok");
            });
        };
        draw(&mut item, &mut text);
        let Trailing::Pill(p) = &item.trailing else { panic!("hap") };
        let pr = p.rect();
        let pad = ListDensity::Regular.padding(&theme);
        assert_eq!(pr.right(), rect.right() - pad);
        assert!((pr.center().y - rect.center().y).abs() <= 0.5);
        let (tx, right) = item.text_span(&theme, rect);
        assert_eq!(right, pr.x - pad, "başlık hapın solunda biter");
        assert_eq!(item.title.max_width(), Some(right - tx));
        item.set_value("Beta");
        let Trailing::Pill(p) = &item.trailing else { panic!("hap") };
        assert_eq!(p.text(), "Beta");
        // Satır devre dışıyken hap da soluk; sonradan konan hap satırın hâlini alır.
        item.set_enabled(false, t0);
        let Trailing::Pill(p) = &item.trailing else { panic!("hap") };
        assert!(!p.is_enabled());
        item.set_trailing(&theme, ListTrailing::Pill("Yeni".into()));
        let Trailing::Pill(p) = &item.trailing else { panic!("hap") };
        assert!(!p.is_enabled());
        item.set_enabled(true, t0);
        let Trailing::Pill(p) = &item.trailing else { panic!("hap") };
        assert!(p.is_enabled());
        crate::redact::assert_redacted(&ListTrailing::Pill(crate::redact::SECRET.into()));
        // Düz metin sağ ucu değişmedi.
        let plain = ListItem::new(&theme, "a").with_trailing(&theme, ListTrailing::Text("%64".into()));
        let (_, right) = plain.text_span(&theme, rect);
        assert_eq!(right, rect.right() - pad - theme.space(24));
    }
}
