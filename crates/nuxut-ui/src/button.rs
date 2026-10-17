//! Düğmeler: [`Button`] (metin / ikon / ikon+metin; filled, tonal, outlined, text) ve
//! [`IconButton`] (yuvarlak, isteğe bağlı seçili durumu).
//!
//! Biçim hareketi: dinlenirken hap (tam yuvarlak), basılınca köşeler `radius_small`'a doğru
//! `spatial_fast` yayıyla sıkışır, bırakınca hafifçe taşıp geri yuvarlanır.

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_render::{Border, Painter, RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::access::{AccessNode, AccessTree, Role};
use crate::icon::{Icon, IconView};
use crate::input::{CursorShape, KeyEvent, PointerEvent};
use crate::label::Label;
use crate::pressable::Pressable;
use crate::style::{HAIRLINE, disabled_container, disabled_content, pill, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonVariant {
    /// `primary` dolgu, `on_primary` içerik — birincil eylem.
    #[default]
    Filled,
    /// `primary` ile boyanmış kap — ikincil eylem.
    Tonal,
    /// Kenar kipi açıkken (`outlines` subtle/full) kenarlıklı ve dolgusuz; varsayılan tonal dilde
    /// (`outlines = "none"`) `surface_bright` dolgulu, `primary` içerikli.
    Outlined,
    /// Yalnızca `primary` metin.
    Text,
    /// Yıkıcı eylem (sil, sıfırla): `error` dolgu, `on_error` içerik.
    Destructive,
}

/// Değişkene göre (kap, içerik, state layer içerik rengi).
pub(crate) fn variant_colors(theme: &Theme, variant: ButtonVariant, enabled: bool) -> (Color, Color, Color) {
    let c = &theme.colors;
    if !enabled {
        let filled = matches!(variant, ButtonVariant::Filled | ButtonVariant::Tonal | ButtonVariant::Destructive)
            || (variant == ButtonVariant::Outlined && theme.outline().is_none());
        let container = if filled { disabled_container(theme) } else { Color::TRANSPARENT };
        return (container, disabled_content(theme, c.on_surface, &[container]), c.on_surface);
    }
    match variant {
        ButtonVariant::Filled => (c.primary, c.on_primary, c.on_primary),
        ButtonVariant::Tonal => (tonal_container(theme), c.on_surface, c.on_surface),
        ButtonVariant::Outlined => match theme.outline() {
            Some(_) => (Color::TRANSPARENT, c.primary, c.primary),
            None => (c.surface_bright, c.primary, c.primary),
        },
        ButtonVariant::Text => (Color::TRANSPARENT, c.primary, c.primary),
        ButtonVariant::Destructive => (c.error, c.on_error, c.on_error),
    }
}

/// Sıkışma oranına göre köşe yarıçapı: hap → `radius_small`.
fn squished_radii(theme: &Theme, rect: Rect, squish: f32) -> Corners {
    let rest = pill(rect).top_left;
    let pressed = theme.shape.radius_small.min(rest);
    Corners::uniform((rest + (pressed - rest) * squish).max(0.0))
}

/// Düğme etiketi tek satırdır: genişlik sınırı ([`Button::set_max_width`]) verilince `…` ile
/// kısalır (sınırsızken etkisiz).
fn label_style(theme: &Theme) -> nuxut_render::TextStyle {
    text_style(theme, TextRole::Label).with_max_lines(1)
}

/// Metin ve/veya ikonlu düğme.
#[derive(Debug, Clone)]
pub struct Button {
    core: Pressable,
    variant: ButtonVariant,
    icon: Option<IconView>,
    label: Option<Label>,
}

impl Button {
    pub fn new(theme: &Theme, variant: ButtonVariant) -> Self {
        Self { core: Pressable::new(theme), variant, icon: None, label: None }
    }

    pub fn with_label(mut self, theme: &Theme, text: impl Into<String>) -> Self {
        self.label = Some(Label::new(text, label_style(theme)));
        self
    }

    pub fn with_icon(mut self, theme: &Theme, icon: Icon) -> Self {
        self.icon = Some(IconView::with_role(theme, icon, TextRole::IconSmall));
        self
    }

    /// İkonun boyut rolü (varsayılan [`TextRole::IconSmall`]; ikon düğmesi görünümü için
    /// [`TextRole::Icon`] / [`TextRole::IconLarge`]). İkon yoksa etkisiz.
    pub fn with_icon_role(mut self, theme: &Theme, role: TextRole) -> Self {
        self.set_icon_role(theme, role);
        self
    }

    /// Değiştiyse `true` (yerleşim yenilenmeli: tercih edilen genişlik değişir).
    pub fn set_icon_role(&mut self, theme: &Theme, role: TextRole) -> bool {
        let changed = self.icon.as_mut().is_some_and(|i| i.set_role(theme, role));
        if changed {
            self.core.damage.mark();
        }
        changed
    }

    /// İkonun boyut rolü (ikon yoksa `None`).
    pub fn icon_role(&self) -> Option<TextRole> {
        self.icon.as_ref().map(IconView::role)
    }

    /// İkon ile metin arası: küçük ikonda `space(2)`, daha büyük ikon rollerinde `space(3)`
    /// (büyük ikon dar boşlukta metne yapışık görünür; onay halkasına da yer kalır).
    fn icon_gap(&self, theme: &Theme) -> f32 {
        match self.icon_role() {
            Some(TextRole::IconSmall) | None => theme.space(2),
            Some(_) => theme.space(3),
        }
    }

    pub fn variant(&self) -> ButtonVariant {
        self.variant
    }

    pub fn set_variant(&mut self, variant: ButtonVariant) {
        if self.variant != variant {
            self.variant = variant;
            self.core.damage.mark();
        }
    }

    pub fn set_label(&mut self, text: &str) -> bool {
        let changed = self.label.as_mut().is_some_and(|l| l.set_text(text));
        if changed {
            self.core.damage.mark();
        }
        changed
    }

    pub fn is_enabled(&self) -> bool {
        self.core.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) -> bool {
        self.core.set_enabled(enabled, now)
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    /// Görünüş durumunu doğrudan verir (galeri, ekran görüntüsü testleri).
    pub fn set_visual_state(&mut self, hovered: bool, pressed: bool, now: Instant) {
        self.core.set_visual_state(hovered, pressed, now);
    }

    pub fn is_hovered(&self) -> bool {
        self.core.interaction.is_hovered()
    }

    pub fn is_pressed(&self) -> bool {
        self.core.interaction.is_pressed()
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        if let Some(l) = &mut self.label {
            l.set_style(label_style(theme));
        }
        if let Some(i) = &mut self.icon {
            i.set_theme(theme, now);
        }
    }

    fn padding(&self, theme: &Theme) -> (f32, f32) {
        // (sol, sağ). İkonlu düğmede ikon tarafı daha dar (görsel denge).
        let side = if self.variant == ButtonVariant::Text { theme.space(3) } else { theme.space(6) };
        match (&self.icon, &self.label) {
            (Some(_), Some(_)) => (side - theme.space(2), side),
            (Some(_), None) => (theme.space(2), theme.space(2)),
            _ => (side, side),
        }
    }

    /// Düğmenin en geniş hâli (iç boşluk ve ikon dahil): etiket sığmazsa `…` ile kısalır, böylece
    /// [`preferred_size`](Self::preferred_size) bu genişliği aşmaz (en dar hâl yine kare: yükseklik).
    /// Erişilebilirlik adı tam metindir. `None`: sınırsız. Değiştiyse `true`.
    pub fn set_max_width(&mut self, theme: &Theme, width: Option<f32>) -> bool {
        let (l, r) = self.padding(theme);
        let icon = self.icon.as_ref().map_or(0.0, |i| i.size().width + self.icon_gap(theme));
        let label_width = width.map(|w| (w - l - r - icon).floor().max(0.0));
        let changed = self.label.as_mut().is_some_and(|label| label.set_max_width(label_width));
        if changed {
            self.core.damage.mark();
        }
        changed
    }

    #[cfg(test)]
    pub(crate) fn label_mut(&mut self) -> Option<&mut Label> {
        self.label.as_mut()
    }

    /// Tercih edilen boyut (yükseklik `space(10)`).
    pub fn preferred_size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        let height = theme.space(10);
        let (l, r) = self.padding(theme);
        let gap = self.icon_gap(theme);
        let mut w = l + r;
        if let Some(i) = &mut self.icon {
            w += i.measure(text).width;
        }
        if let Some(label) = &mut self.label {
            w += label.measure(text).width;
            if self.icon.is_some() {
                w += gap;
            }
        }
        Size::new(w.max(height).ceil(), height)
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    /// Tıklamada `event = Some(())`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        self.core.pointer(event, now)
    }

    /// Odaktayken Enter/Space.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response {
        self.core.key(event, now)
    }

    /// İmleç (DESIGN.md "İmleç").
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let rect = self.core.rect;
        let (container, content, layer) = variant_colors(theme, self.variant, self.core.enabled);
        let radii = squished_radii(theme, rect, self.core.squish(now));
        ui.painter.rect(rect, &RectStyle::filled(ui.fade(container), radii));
        if self.variant == ButtonVariant::Outlined
            && let Some(outline) = theme.outline()
        {
            let border = if self.core.enabled { outline } else { content };
            let style = RectStyle {
                fill: Color::TRANSPARENT,
                radii,
                border: Some(Border { width: HAIRLINE, color: ui.fade(border) }),
            };
            ui.painter.rect(rect, &style);
        }
        self.core.interaction.draw_state_layer(ui, rect, radii, layer, container);
        self.core.draw_focus(ui, rect, radii);

        // İçerik: ortalanmış [ikon][boşluk][metin].
        let content = ui.fade(content);
        let (mut x, label_size) = self.content_start(ui.painter, theme);
        let icon_w = self.icon.as_ref().map_or(0.0, |i| i.size().width);
        let gap = self.icon_gap(theme);
        if let Some(icon) = &mut self.icon {
            icon.draw_centered(ui.painter, Point::new(x + icon_w * 0.5, rect.center().y), content);
            x += icon_w + gap;
        }
        if let Some(label) = &mut self.label {
            label.draw(ui.painter, Point::new(x, rect.center().y - label_size.height * 0.5), content);
        }
        self.core.finish(ui, self.core.extent());
    }

    /// Ortalanmış içeriğin ([ikon][`icon_gap`][metin]) sol kenarı ve metin boyutu.
    fn content_start(&mut self, painter: &mut Painter, theme: &Theme) -> (f32, Size) {
        let gap = self.icon_gap(theme);
        let icon_w = self.icon.as_ref().map_or(0.0, |i| i.size().width);
        let label_size = self.label.as_mut().map_or(Size::ZERO, |l| l.measure_in(painter));
        let total = icon_w + label_size.width + if icon_w > 0.0 && label_size.width > 0.0 { gap } else { 0.0 };
        (self.core.rect.center().x - total * 0.5, label_size)
    }

    /// İkonun çizildiği merkez ve kutu kenarı (ikon yoksa `None`); `draw` ile aynı yerleşim.
    pub(crate) fn icon_center(&mut self, painter: &mut Painter, theme: &Theme) -> Option<(Point, f32)> {
        let icon_w = self.icon.as_ref()?.size().width;
        let (x, _) = self.content_start(painter, theme);
        Some((Point::new(x + icon_w * 0.5, self.core.rect.center().y), icon_w))
    }

    /// Geçerli değişken ve etkinlikteki içerik rengi (solma uygulanmamış).
    pub(crate) fn content_color(&self, theme: &Theme) -> Color {
        variant_colors(theme, self.variant, self.core.enabled).1
    }

    /// İçeriği (değişken, ikon, metin) `other` ile takas eder; konum, hover/basılı, odak ve
    /// etkinlik bu düğmede kalır (yerinde içerik değişimi: `ConfirmButton`).
    pub(crate) fn swap_content(&mut self, other: &mut Button) {
        std::mem::swap(&mut self.variant, &mut other.variant);
        std::mem::swap(&mut self.icon, &mut other.icon);
        std::mem::swap(&mut self.label, &mut other.label);
        self.core.damage.mark();
    }

    /// İçeriğini yeniden kurmak için (oluşturucu zinciri dışında `with_*` kullanımı).
    pub(crate) fn rebuild(&mut self, theme: &Theme, f: impl FnOnce(Button) -> Button) {
        let placeholder = Button::new(theme, self.variant);
        let old = std::mem::replace(self, placeholder);
        *self = f(old);
        self.core.damage.mark();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IconButtonVariant {
    /// Kapsız; yalnızca hover/basılı katmanı.
    #[default]
    Standard,
    Filled,
    Tonal,
}

/// Yuvarlak ikon düğmesi. `set_selected` ile aç/kapa düğmesi olur (seçiliyken dolu).
#[derive(Debug, Clone)]
pub struct IconButton {
    core: Pressable,
    variant: IconButtonVariant,
    icon: IconView,
    selected: Option<bool>,
}

impl IconButton {
    pub fn new(theme: &Theme, icon: Icon, variant: IconButtonVariant) -> Self {
        Self { core: Pressable::new(theme), variant, icon: IconView::new(theme, icon), selected: None }
    }

    /// Aç/kapa davranışı: tıklama seçili durumu çevirir.
    /// Seçiliyken ikon dolu (FILL ekseni), değişimde dolgu geçişli.
    pub fn toggleable(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self.icon = self.icon.filled(selected);
        self
    }

    pub fn selected(&self) -> Option<bool> {
        self.selected
    }

    /// Dış durum (ör. Bluetooth açıldı); ikon dolgusu geçişli değişir.
    pub fn set_selected(&mut self, selected: bool, now: Instant) {
        if self.selected != Some(selected) {
            self.selected = Some(selected);
            self.icon.set_filled(selected, now);
            self.core.damage.mark();
        }
    }

    pub fn set_icon(&mut self, icon: Icon) {
        if self.icon.set_icon(icon) {
            self.core.damage.mark();
        }
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) -> bool {
        self.core.set_enabled(enabled, now)
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    /// Görünüş durumunu doğrudan verir (galeri, ekran görüntüsü testleri).
    pub fn set_visual_state(&mut self, hovered: bool, pressed: bool, now: Instant) {
        self.core.set_visual_state(hovered, pressed, now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.icon.set_theme(theme, now);
    }

    /// Kare: `space(10)`.
    pub fn preferred_size(&self, theme: &Theme) -> Size {
        Size::new(theme.space(10), theme.space(10))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    /// Tıklamada `event = Some(seçili_mi)` (aç/kapa değilse hep `false`).
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<bool> {
        let r = self.core.pointer(event, now);
        self.after_activate(r, now)
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<bool> {
        let r = self.core.key(event, now);
        self.after_activate(r, now)
    }

    fn after_activate(&mut self, r: Response, now: Instant) -> Response<bool> {
        if r.event.is_some()
            && let Some(sel) = &mut self.selected
        {
            *sel = !*sel;
            self.icon.set_filled(*sel, now);
            self.core.damage.mark();
        }
        let selected = self.selected.unwrap_or(false);
        r.map(|()| selected)
    }

    /// İmleç (DESIGN.md "İmleç").
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.core.rect;
        let enabled = self.core.enabled;
        let (container, content) = if !enabled {
            let container = if self.variant == IconButtonVariant::Standard {
                Color::TRANSPARENT
            } else {
                disabled_container(theme)
            };
            (container, disabled_content(theme, c.on_surface, &[container]))
        } else {
            match (self.variant, self.selected) {
                (IconButtonVariant::Filled, Some(false)) => (c.surface_bright, c.primary),
                (IconButtonVariant::Filled, _) | (IconButtonVariant::Tonal, Some(true)) => (c.primary, c.on_primary),
                (IconButtonVariant::Tonal, _) => (tonal_container(theme), c.on_surface),
                (IconButtonVariant::Standard, Some(true)) => (Color::TRANSPARENT, c.primary),
                (IconButtonVariant::Standard, Some(false)) => (Color::TRANSPARENT, c.on_surface_variant),
                (IconButtonVariant::Standard, None) => (Color::TRANSPARENT, c.on_surface),
            }
        };
        let radii = squished_radii(theme, rect, self.core.squish(now));
        ui.painter.rect(rect, &RectStyle::filled(ui.fade(container), radii));
        let layer = if container == c.primary { c.on_primary } else { c.on_surface };
        self.core.interaction.draw_state_layer_under(ui, rect, radii, layer, container, &[content]);
        self.core.draw_focus(ui, rect, radii);
        if self.icon.animate(now) {
            ui.request_frame();
            self.core.damage.mark();
        }
        self.icon.draw_centered(ui.painter, rect.center(), ui.fade(content));
        self.core.finish(ui, self.core.extent());
    }
}

// Erişilebilirlik (ADR-0011).
impl Button {
    /// `Role::Button`: ad görünen etiketten; yalnız ikonluysa çağıran `.label(&t!(..))` verir.
    /// AT `Click`'i çağıranın tıklama yoluna gider (bileşende durum yok).
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash) -> AccessNode<'t> {
        let node = tree.push(key, Role::Button).bounds(self.core.rect).enabled(self.core.enabled).clickable();
        match &self.label {
            Some(label) => node.label(label.text()),
            None => node,
        }
    }
}

impl IconButton {
    /// `Role::Button` (seçilebilirse aç/kapa durumuyla). Ad görünmez: çağıran `.label(&t!(..))`
    /// vermek ZORUNDA (`AccessTree::unlabeled` testi).
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash) -> AccessNode<'t> {
        let node = tree.push(key, Role::Button).bounds(self.core.rect).enabled(self.core.enabled).clickable();
        match self.selected {
            Some(on) => node.toggled(on),
            None => node,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_names::IconName;
    use crate::input::test_util::*;

    #[test]
    fn click_requires_press_and_release_inside() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut b = Button::new(&theme, ButtonVariant::Filled);
        b.set_rect(Rect::new(0.0, 0.0, 100.0, 40.0));
        assert!(b.pointer(&motion(10.0, 10.0), t0).repaint);
        assert!(b.pointer(&press(10.0, 10.0), t0).event.is_none());
        assert!(b.is_pressed());
        assert_eq!(b.pointer(&release(12.0, 10.0), t0).event, Some(()));
        // Dışarıda bırakılırsa tıklama yok.
        let _ = b.pointer(&press(10.0, 10.0), t0);
        assert!(b.pointer(&release(200.0, 10.0), t0).event.is_none());
        // Dışarıda basıp içeride bırakmak da tıklama değil.
        let _ = b.pointer(&press(200.0, 10.0), t0);
        assert!(b.pointer(&release(10.0, 10.0), t0).event.is_none());
    }

    #[test]
    fn disabled_ignores_input() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut b = Button::new(&theme, ButtonVariant::Tonal);
        b.set_rect(Rect::new(0.0, 0.0, 100.0, 40.0));
        b.set_enabled(false, t0);
        let _ = b.pointer(&press(10.0, 10.0), t0);
        assert!(b.pointer(&release(10.0, 10.0), t0).event.is_none());
    }

    #[test]
    fn outlined_is_tonal_without_outlines() {
        let none = Theme::default();
        let (c, ..) = variant_colors(&none, ButtonVariant::Outlined, true);
        assert_eq!(c, none.colors.surface_bright, "kenarsız dilde tonal dolgu");
        let (full, _) = Theme::parse("[shape]\noutlines = \"full\"\n").unwrap();
        let (c, ..) = variant_colors(&full, ButtonVariant::Outlined, true);
        assert_eq!(c.a, 0.0, "kenar kipinde dolgusuz");
    }

    #[test]
    fn press_squishes_radius_toward_small() {
        let theme = Theme::default();
        let r = Rect::new(0.0, 0.0, 120.0, 40.0);
        assert_eq!(squished_radii(&theme, r, 0.0).top_left, 20.0);
        assert_eq!(squished_radii(&theme, r, 1.0).top_left, theme.shape.radius_small);
    }

    #[test]
    fn icon_button_toggles() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut b = IconButton::new(&theme, Icon::named(IconName::Wifi), IconButtonVariant::Standard).toggleable(false);
        b.set_rect(Rect::new(0.0, 0.0, 40.0, 40.0));
        let _ = b.pointer(&press(5.0, 5.0), t0);
        assert_eq!(b.pointer(&release(5.0, 5.0), t0).event, Some(true));
        assert_eq!(b.key(&key(crate::input::Key::Enter), t0).event, Some(false));
    }

    #[test]
    fn icon_role_resizes_icon_and_widens_gap() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let make = || Button::new(&theme, ButtonVariant::Tonal).with_icon(&theme, Icon::named(IconName::Power)).with_label(&theme, "Kapat");
        let mut small = make();
        let mut large = make();
        assert_eq!(small.icon_role(), Some(TextRole::IconSmall), "varsayılan küçük");
        assert!(large.set_icon_role(&theme, TextRole::IconLarge));
        assert!(!large.set_icon_role(&theme, TextRole::IconLarge), "aynı rol: değişiklik yok");
        let iw = |b: &Button| b.icon.as_ref().unwrap().size().width;
        assert!(iw(&large) > iw(&small));
        // Genişlik farkı = ikon farkı + boşluk farkı (space(3) − space(2)); yükseklik aynı.
        let (ws, wl) = (small.preferred_size(&mut text, &theme), large.preferred_size(&mut text, &theme));
        assert_eq!(ws.height, wl.height);
        let expected = iw(&large) - iw(&small) + theme.space(3) - theme.space(2);
        assert!((wl.width - ws.width - expected).abs() <= 1.0, "{ws:?} {wl:?}");
        // İkonsuz düğmede rol yok, etkisiz.
        let mut plain = Button::new(&theme, ButtonVariant::Text).with_label(&theme, "x");
        assert!(!plain.set_icon_role(&theme, TextRole::IconLarge));
        assert_eq!(plain.icon_role(), None);
    }
}
