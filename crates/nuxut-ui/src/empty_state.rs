//! Boş durum ([`EmptyState`]): görsel (maskot Fiyu, ikon ya da hiçbiri) + başlık + gövde metni
//! (+ isteğe bağlı eylem düğmesi), alanın içinde yatay ve dikey ortalı tek sütun.
//!
//! Yerleşim kuralı (`docs/design/mascot.md` "Boş durum yerleşimi"; shell `widgets/empty_state.rs`
//! geometrisi buraya taşındı, talep 2026-10-13):
//! - Sütun: görsel, `space(4)` aralık, başlık (`Title`, ortalı, en fazla
//!   [`EMPTY_TITLE_LINES`] satır), `space(1)`, gövde (`Body`, `on_surface_variant`, ortalı, en
//!   fazla [`EMPTY_BODY_LINES`] satır), `space(4)`, düğme. Metin sarar; taşan `…`. Metin
//!   genişliği alanın içi (kenarlardan `space(4)`) ile `space(90)`'dan dar olanı.
//! - Maskot alan darsa küçülür (tercih [`EMPTY_MASCOT_SIZE`], en az `MASCOT_MIN_SIZE`), o da
//!   sığmazsa düşer: yalnız metin, kendi başına ortalı (aralık da düşer, boşluk kalmaz). İkon
//!   ölçeklenmez: sığmazsa düşer.
//! - `framework.toml [appearance] mascot = false`: maskot yer kaplamaz (aynı düşme kuralı;
//!   varsayılan ikona geçilmez — gizleme maskotun yerini kapatır). Çağıran
//!   [`EmptyState::with_fallback_icon`] verdiyse maskotun yerine o ikon çizilir ("gizliyken eski
//!   görünüm"; ikon kipinin kuralları: ölçeklenmez, sığmazsa düşer). Karar 2026-10-14.
//! - Sütun alandan uzunsa üstü alanın iç üstüne yaslanır (başlık kesilmez, taşma altta).
//!
//! Akış: `new` → `with_*` → `set_rect` → `draw` (yerleşim çizimde, tema/ölçek değişimini
//! kendiliğinden izler; girdi yerleşimden önce gelirse [`EmptyState::layout`] çağrılabilir).
//! Canlanma yok, boşta kare istenmez (maskot dokusu GPU'ya inene kadar hariç).

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_render::{TextAlign, TextStyle, TextSystem};
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};

use crate::button::{Button, ButtonVariant};
use crate::icon::{Icon, IconView};
use crate::input::{CursorShape, KeyEvent, PointerEvent};
use crate::label::Label;
#[cfg(feature = "brand")]
use crate::mascot::{MASCOT_MAX_SIZE, MASCOT_MIN_SIZE, Mascot, Mood};
use crate::style::clip_text;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Boş durum, sonuç yok ve hata yerleşimlerinde maskotun tercih edilen kenarı (mascot.md: 96).
pub const EMPTY_MASCOT_SIZE: f32 = 96.0;
/// Başlık ve gövdenin en uzun hâli (bayt; dış girdi şekillendiriciyi yormasın).
pub const MAX_EMPTY_TEXT: usize = 1024;
/// Başlığın en fazla satırı.
pub const EMPTY_TITLE_LINES: usize = 2;
/// Gövdenin en fazla satırı.
pub const EMPTY_BODY_LINES: usize = 4;

/// Boş durumun ölçüleri (yerleşim girdisi; saf geometri [`empty_layout`] için).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmptyMetrics {
    /// Görselin tercih edilen kenarı; `0` = görsel yok (ya da maskot gizli).
    pub art: f32,
    /// Görselin en küçük kenarı (maskot `MASCOT_MIN_SIZE`; ikon = kendi boyu, ölçeklenmez).
    pub art_min: f32,
    /// Görsel ile başlık arası.
    pub art_gap: f32,
    pub title: Size,
    /// Başlık–gövde arası (gövde yoksa kullanılmaz).
    pub body_gap: f32,
    /// Gövde; yoksa sıfır.
    pub body: Size,
    /// Metin–düğme arası (düğme yoksa kullanılmaz).
    pub action_gap: f32,
    /// Düğme; yoksa sıfır.
    pub action: Size,
}

/// [`empty_layout`] sonucu. Görsel yoksa ya da düştüyse `art == None`; metinler `text_width`
/// genişliğinde ortalı kutulara yerleşir (sol üstleri), düğme sütunun ortasında.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmptyLayout {
    pub art: Option<Rect>,
    pub title: Point,
    pub body: Point,
    pub action: Rect,
    /// Sütunun kapladığı dikdörtgen (görsel + metin + düğme).
    pub column: Rect,
}

fn has(s: Size) -> bool {
    s.width > 0.0 && s.height > 0.0
}

/// Metin bloğunun (başlık + gövde + düğme, aralarıyla) yüksekliği.
fn text_block(m: &EmptyMetrics) -> f32 {
    let mut h = m.title.height;
    if has(m.body) {
        h += m.body_gap + m.body.height;
    }
    if has(m.action) {
        h += m.action_gap + m.action.height;
    }
    h
}

/// Görselin `inner` alana sığan kenarı: tercih, alan yetmiyorsa küçültülmüş; en küçük kenar da
/// sığmıyorsa `None` (görsel çizilmez, aralığı da düşer).
pub fn fit_art(m: &EmptyMetrics, inner: Size) -> Option<f32> {
    if m.art.is_nan() || m.art <= 0.0 {
        return None;
    }
    let room = (inner.height - m.art_gap - text_block(m)).min(inner.width);
    let side = m.art.min(room);
    (side >= m.art_min && side > 0.0).then_some(side)
}

/// `inner` (dolgu düşülmüş alan) içinde ortalı sütun. Sütun alandan uzunsa üstü `inner`'in
/// üstüne yaslanır.
pub fn empty_layout(inner: Rect, m: &EmptyMetrics) -> EmptyLayout {
    let art = fit_art(m, inner.size());
    let art_h = art.map_or(0.0, |s| s + m.art_gap);
    let total = art_h + text_block(m);
    let cx = inner.center().x;
    let top = (inner.center().y - total * 0.5).max(inner.y);
    let art_rect = art.map(|s| Rect::new(cx - s * 0.5, top, s, s));
    let title = Point::new(cx - m.title.width * 0.5, top + art_h);
    let mut y = title.y + m.title.height;
    let body = if has(m.body) {
        y += m.body_gap;
        let p = Point::new(cx - m.body.width * 0.5, y);
        y += m.body.height;
        p
    } else {
        Point::new(cx, y)
    };
    let action = if has(m.action) {
        y += m.action_gap;
        let r = Rect::new(cx - m.action.width * 0.5, y, m.action.width, m.action.height);
        y += m.action.height;
        r
    } else {
        Rect::new(cx, y, 0.0, 0.0)
    };
    let width = [art.unwrap_or(0.0), m.title.width, m.body.width, m.action.width].into_iter().fold(0.0, f32::max);
    EmptyLayout { art: art_rect, title, body, action, column: Rect::new(cx - width * 0.5, top, width, y - top) }
}

#[derive(Debug, Clone)]
enum Art {
    None,
    Icon(Box<IconView>),
    #[cfg(feature = "brand")]
    Mascot(Box<Mascot>),
}

/// Boş durum bileşeni (ayrıntı: modül belgesi).
#[derive(Debug, Clone)]
pub struct EmptyState {
    art: Art,
    /// Maskot gizliyken (`[appearance] mascot = false`) onun yerine çizilen ikon; yalnız maskot
    /// kipinde anlamlı.
    fallback: Option<Box<IconView>>,
    /// Maskotun tercih edilen kenarı.
    art_size: f32,
    title: Label,
    body: Option<Label>,
    action: Option<Button>,
    rect: Rect,
    layout: Option<EmptyLayout>,
    damage: DamageState,
}

fn title_style(theme: &Theme) -> TextStyle {
    text_style(theme, TextRole::Title).with_align(TextAlign::Center).with_max_lines(EMPTY_TITLE_LINES)
}

fn body_style(theme: &Theme) -> TextStyle {
    text_style(theme, TextRole::Body).with_align(TextAlign::Center).with_max_lines(EMPTY_BODY_LINES)
}

impl EmptyState {
    /// Görselsiz, yalnız başlıklı boş durum.
    pub fn new(theme: &Theme, title: impl Into<String>) -> Self {
        let title: String = title.into();
        Self {
            art: Art::None,
            fallback: None,
            art_size: EMPTY_MASCOT_SIZE,
            title: Label::new(clip_text(&title, MAX_EMPTY_TEXT), title_style(theme)),
            body: None,
            action: None,
            rect: Rect::ZERO,
            layout: None,
            damage: DamageState::new(),
        }
    }

    /// Maskot Fiyu, ruh hâli `mood` (yerleşim kuralı: boş → `Empty`, sonuç yok → `NoResults`,
    /// hata → `Error`).
    #[cfg(feature = "brand")]
    pub fn with_mascot(mut self, theme: &Theme, mood: Mood) -> Self {
        self.art = Art::Mascot(Box::new(Mascot::new(theme, mood, self.art_size)));
        self
    }

    /// Maskotun tercih edilen kenarı (Hoş geldin/Hakkında 128–160); alan darsa yine küçülür.
    pub fn with_mascot_size(mut self, size: f32) -> Self {
        if size.is_finite() && size > 0.0 {
            self.art_size = size;
        }
        self
    }

    /// Maskot yerine ikon (`IconLarge`, `on_surface_variant`); maskotun uygun olmadığı yerler.
    pub fn with_icon(mut self, theme: &Theme, icon: Icon) -> Self {
        self.art = Art::Icon(Box::new(IconView::with_role(theme, icon, TextRole::IconLarge)));
        self
    }

    /// Maskot gizliyken (`[appearance] mascot = false`) maskotun yerine çizilecek ikon (ikon
    /// kipiyle aynı: `IconLarge`, `on_surface_variant`, ölçeklenmez, sığmazsa düşer). Maskot
    /// görünürken çizilmez; maskotsuz/ikonlu kipte etkisizdir. Verilmezse gizleme maskotun yerini
    /// kapatır (karar 2026-10-14, `docs/design/mascot.md`).
    pub fn with_fallback_icon(mut self, theme: &Theme, icon: Icon) -> Self {
        self.set_fallback_icon(theme, Some(icon));
        self
    }

    /// Yedek ikonu kurar, değiştirir ya da (`None`) kaldırır.
    pub fn set_fallback_icon(&mut self, theme: &Theme, icon: Option<Icon>) {
        match (icon, &mut self.fallback) {
            (Some(icon), Some(f)) => {
                if !f.set_icon(icon) {
                    return;
                }
            }
            (Some(icon), None) => self.fallback = Some(Box::new(IconView::with_role(theme, icon, TextRole::IconLarge))),
            (None, None) => return,
            (None, Some(_)) => self.fallback = None,
        }
        self.invalidate();
    }

    /// Kurulu yedek ikon.
    pub fn fallback_icon(&self) -> Option<&Icon> {
        self.fallback.as_deref().map(IconView::icon)
    }

    pub fn with_body(mut self, theme: &Theme, text: impl Into<String>) -> Self {
        self.set_body(theme, Some(&text.into()));
        self
    }

    /// Eylem düğmesi (`Tonal`); tıklama [`EmptyState::pointer`]/[`EmptyState::key`] olayıdır.
    pub fn with_action(mut self, theme: &Theme, label: impl Into<String>) -> Self {
        self.action = Some(Button::new(theme, ButtonVariant::Tonal).with_label(theme, label));
        self
    }

    pub fn title(&self) -> &str {
        self.title.text()
    }

    pub fn set_title(&mut self, text: &str) {
        if self.title.set_text(clip_text(text, MAX_EMPTY_TEXT)) {
            self.invalidate();
        }
    }

    pub fn body(&self) -> Option<&str> {
        self.body.as_ref().map(Label::text)
    }

    /// Gövde metni; `None` ya da boş metin gövdeyi kaldırır.
    pub fn set_body(&mut self, theme: &Theme, text: Option<&str>) {
        let text = text.filter(|t| !t.is_empty()).map(|t| clip_text(t, MAX_EMPTY_TEXT));
        match (text, &mut self.body) {
            (Some(t), Some(l)) => {
                if !l.set_text(t) {
                    return;
                }
            }
            (Some(t), None) => self.body = Some(Label::new(t, body_style(theme))),
            (None, None) => return,
            (None, Some(_)) => self.body = None,
        }
        self.invalidate();
    }

    /// Maskotun ruh hâli (maskotlu değilse etkisiz).
    #[cfg(feature = "brand")]
    pub fn set_mood(&mut self, mood: Mood) {
        if let Art::Mascot(m) = &mut self.art {
            m.set_mood(mood);
        }
    }

    /// Maskotun ruh hâli; maskotlu değilse `None`. `[appearance] mascot = false` iken de
    /// kurulan ruh hâlini döner (gizleme çizimi kapatır, seçimi silmez).
    #[cfg(feature = "brand")]
    pub fn mood(&self) -> Option<Mood> {
        match &self.art {
            Art::Mascot(m) => Some(m.mood()),
            _ => None,
        }
    }

    /// İkonu değiştirir (ikonlu değilse etkisiz).
    pub fn set_icon(&mut self, icon: Icon) {
        if let Art::Icon(i) = &mut self.art
            && i.set_icon(icon)
        {
            self.invalidate();
        }
    }

    /// Eylem düğmesi (etiket, varyant, etkinlik çağıranda).
    pub fn action_mut(&mut self) -> Option<&mut Button> {
        self.action.as_mut()
    }

    /// Eylem düğmesini kaldırır ya da etiketiyle kurar.
    pub fn set_action(&mut self, theme: &Theme, label: Option<&str>) {
        match (label, &mut self.action) {
            (Some(l), Some(b)) => {
                if !b.set_label(l) {
                    return;
                }
            }
            (Some(l), None) => self.action = Some(Button::new(theme, ButtonVariant::Tonal).with_label(theme, l)),
            (None, None) => return,
            (None, Some(_)) => self.action = None,
        }
        self.invalidate();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.title.set_style(title_style(theme));
        if let Some(b) = &mut self.body {
            b.set_style(body_style(theme));
        }
        match &mut self.art {
            Art::None => {}
            Art::Icon(i) => i.set_theme(theme, now),
            #[cfg(feature = "brand")]
            Art::Mascot(m) => m.set_theme(theme, now),
        }
        if let Some(f) = &mut self.fallback {
            f.set_theme(theme, now);
        }
        if let Some(b) = &mut self.action {
            b.set_theme(theme, now);
        }
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.layout = None;
        self.damage.mark();
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Kaplanacak alan; içerik bunun ortasına yerleşir.
    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.invalidate();
        }
    }

    /// Kenarlardan iç boşluk (`space(4)`).
    fn padding(theme: &Theme) -> f32 {
        theme.space(4)
    }

    /// Metin sütununun genişliği: `avail` ile `space(90)`'dan dar olanı.
    fn text_width(theme: &Theme, avail: f32) -> f32 {
        avail.min(theme.space(90)).max(0.0).floor()
    }

    /// Görselin tercih edilen ve en küçük kenarı (gizli maskot: yedek ikonun kenarı ya da `0`).
    /// İkon ölçeklenmez.
    #[cfg_attr(not(feature = "brand"), allow(unused_variables))]
    fn art_metrics(&self, theme: &Theme) -> (f32, f32) {
        let icon_side = |i: &IconView| {
            let s = i.size();
            let side = s.width.max(s.height);
            (side, side)
        };
        match &self.art {
            Art::None => (0.0, 0.0),
            Art::Icon(i) => icon_side(i),
            #[cfg(feature = "brand")]
            Art::Mascot(_) if !theme.appearance.mascot => self.fallback.as_deref().map_or((0.0, 0.0), icon_side),
            #[cfg(feature = "brand")]
            Art::Mascot(_) => (self.art_size.clamp(MASCOT_MIN_SIZE, MASCOT_MAX_SIZE), MASCOT_MIN_SIZE),
        }
    }

    /// `width` genişlikte metin ve düğme ölçüleri.
    fn metrics(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> EmptyMetrics {
        let (art, art_min) = self.art_metrics(theme);
        self.title.set_max_width(Some(width));
        let title = self.title.measure(text);
        let body = match &mut self.body {
            Some(b) => {
                b.set_max_width(Some(width));
                b.measure(text)
            }
            None => Size::ZERO,
        };
        let action = match &mut self.action {
            Some(b) => {
                // Dar alanda etiket kısalır; düğme sütunu taşmaz.
                b.set_max_width(theme, Some(width));
                let s = b.preferred_size(text, theme);
                Size::new(s.width.min(width), s.height)
            }
            None => Size::ZERO,
        };
        EmptyMetrics {
            art,
            art_min,
            art_gap: theme.space(4),
            title,
            body_gap: theme.space(1),
            body,
            action_gap: theme.space(4),
            action,
        }
    }

    /// Doğal boyut, en fazla `max_width` genişlikte (dolgu dahil; görsel tercih edilen kenarında).
    /// Kabı içeriğe göre boyutlayan çağıran için; ortalama yine `set_rect` alanında yapılır.
    pub fn measure(&mut self, text: &mut TextSystem, theme: &Theme, max_width: f32) -> Size {
        let pad = Self::padding(theme);
        let width = Self::text_width(theme, max_width - pad * 2.0);
        let m = self.metrics(text, theme, width);
        let art = if m.art > 0.0 { m.art.min(width) + m.art_gap } else { 0.0 };
        let w = [m.art.min(width), m.title.width, m.body.width, m.action.width].into_iter().fold(0.0, f32::max);
        // Ölçüm yerleşimi bozmasın: genişlik değişmiş olabilir.
        self.layout = None;
        Size::new((w + pad * 2.0).ceil(), (art + text_block(&m) + pad * 2.0).ceil())
    }

    /// Yerleşimi hesaplar ve alt bileşenlere uygular (önbellekli; `draw` de çağırır). Görselin
    /// düşüp düşmediği sonuçtan okunur (`art == None`).
    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme) -> EmptyLayout {
        let pad = Self::padding(theme);
        let inner = self.rect.expand(-pad);
        let inner = Rect::new(inner.x, inner.y, inner.width.max(0.0), inner.height.max(0.0));
        let width = Self::text_width(theme, inner.width);
        let m = self.metrics(text, theme, width);
        let l = empty_layout(inner, &m);
        // İkon çizimde `art` merkezine çizilir; maskot kendi kutusunu ve kenarını tutar.
        #[cfg(feature = "brand")]
        if let (Some(art), Art::Mascot(mascot), true) = (l.art, &mut self.art, theme.appearance.mascot) {
            mascot.set_size(art.width);
            mascot.set_rect(art);
        }
        if let Some(b) = &mut self.action {
            b.set_rect(l.action);
        }
        if self.layout != Some(l) {
            self.layout = Some(l);
            self.damage.mark();
        }
        l
    }

    /// Son yerleşim (henüz çizilmediyse `None`).
    pub fn last_layout(&self) -> Option<EmptyLayout> {
        self.layout
    }

    /// Düğme tıklandı: `event = Some(())`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        match &mut self.action {
            Some(b) => b.pointer(event, now),
            None => Response::none(),
        }
    }

    /// Düğme odaktayken Enter/Space.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response {
        match &mut self.action {
            Some(b) => b.key(event, now),
            None => Response::none(),
        }
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.action.as_mut().is_some_and(|b| b.set_focus_visible(on))
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.action.as_ref().and_then(|b| b.cursor(p))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        if self.rect.is_empty() {
            self.damage.report_hidden(ui);
            return;
        }
        let l = self.layout(ui.painter.text_system(), theme);
        let c = &theme.colors;
        if let Some(art) = l.art {
            match &mut self.art {
                Art::None => {}
                Art::Icon(i) => i.draw_centered(ui.painter, art.center(), ui.fade(c.on_surface_variant)),
                #[cfg(feature = "brand")]
                Art::Mascot(m) if theme.appearance.mascot => m.draw(ui),
                // Gizli maskot: yerleşim yalnız yedek ikon varsa `art` verir.
                #[cfg(feature = "brand")]
                Art::Mascot(_) => {
                    if let Some(f) = &mut self.fallback {
                        f.draw_centered(ui.painter, art.center(), ui.fade(c.on_surface_variant));
                    }
                }
            }
        }
        self.title.draw(ui.painter, l.title, ui.fade(c.on_surface));
        if let Some(b) = &mut self.body {
            b.draw(ui.painter, l.body, ui.fade(c.on_surface_variant));
        }
        if let Some(b) = &mut self.action {
            b.draw(ui);
        }
        // Görsel düşünce/gizlenince eski yeri de silinsin: alanın tamamı (yalnız değişimde).
        self.damage.report(ui, self.rect, false);
    }
}

// Erişilebilirlik (ADR-0011).
impl EmptyState {
    /// `Role::Group`: ad başlık, açıklama gövde; eylem düğmesi varsa çocuk `Role::Button`
    /// (`"action"` anahtarı). Maskot ve ikon süstür, ağaçta yok.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash) -> AccessNode<'t> {
        let id = tree
            .push(key, Role::Group)
            .label(self.title.text())
            .description(self.body.as_ref().map_or("", |b| b.text()))
            .bounds(self.rect)
            .id();
        if let Some(a) = &self.action {
            tree.within(id, |tree| {
                a.access(tree, "action");
            });
        }
        tree.get(id).expect("az önce eklendi")
    }

    /// AT isteği eylem düğmesine `Click` mi (tıklamayla aynı: çağıran eylemi yapar).
    /// `state`: `access`'in döndürdüğü kimlik.
    pub fn is_access_action(&self, state: NodeId, request: AccessRequest) -> bool {
        self.action.as_ref().is_some_and(Button::is_enabled)
            && request.action == AccessAction::Click
            && request.node == child_id(state, "action")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon_names::IconName;
    use crate::input::test_util::*;
    use nuxut_render::{DrawKind, Recorder};

    fn metrics(art: f32, art_min: f32) -> EmptyMetrics {
        EmptyMetrics {
            art,
            art_min,
            art_gap: 12.0,
            title: Size::new(120.0, 20.0),
            body_gap: 0.0,
            body: Size::ZERO,
            action_gap: 0.0,
            action: Size::ZERO,
        }
    }

    // --- Saf geometri (shell `widgets/empty_state.rs` testlerinin karşılığı) ------------------

    #[test]
    fn column_centers_art_and_label() {
        let area = Rect::new(100.0, 50.0, 400.0, 300.0);
        let l = empty_layout(area, &metrics(96.0, 48.0));
        // Toplam 128: üst 50 + (300 - 128) / 2 = 136.
        assert_eq!(l.art, Some(Rect::new(252.0, 136.0, 96.0, 96.0)));
        assert_eq!(l.title, Point::new(240.0, 136.0 + 96.0 + 12.0));
        assert_eq!(l.column, Rect::new(240.0, 136.0, 120.0, 128.0));
    }

    #[test]
    fn hidden_art_leaves_no_gap() {
        let l = empty_layout(Rect::new(0.0, 0.0, 400.0, 300.0), &metrics(0.0, 48.0));
        assert_eq!(l.title, Point::new(140.0, 140.0), "metin tam ortada; görsel/aralık payı yok");
        assert_eq!(l.art, None);
    }

    #[test]
    fn mascot_shrinks_then_drops() {
        let fit = |h: f32| fit_art(&metrics(96.0, 48.0), Size::new(400.0, h));
        assert_eq!(fit(400.0), Some(96.0));
        assert_eq!(fit(100.0), Some(68.0));
        assert_eq!(fit(80.0), Some(48.0));
        assert_eq!(fit(79.0), None, "48'in altında yüz okunmaz");
        // Dar alan da küçültür.
        assert_eq!(fit_art(&metrics(96.0, 48.0), Size::new(60.0, 400.0)), Some(60.0));
        // İkon ölçeklenmez: sığmazsa düşer.
        assert_eq!(fit_art(&metrics(40.0, 40.0), Size::new(400.0, 71.0)), None);
        assert_eq!(fit_art(&metrics(40.0, 40.0), Size::new(400.0, 72.0)), Some(40.0));
    }

    #[test]
    fn body_and_action_stack_below_title() {
        let mut m = metrics(96.0, 48.0);
        m.body_gap = 4.0;
        m.body = Size::new(200.0, 40.0);
        m.action_gap = 16.0;
        m.action = Size::new(100.0, 40.0);
        let l = empty_layout(Rect::new(0.0, 0.0, 400.0, 600.0), &m);
        let art = l.art.unwrap();
        assert_eq!(l.title.y, art.bottom() + 12.0);
        assert_eq!(l.body, Point::new(100.0, l.title.y + 20.0 + 4.0));
        assert_eq!(l.action, Rect::new(150.0, l.body.y + 40.0 + 16.0, 100.0, 40.0));
        let total = 96.0 + 12.0 + 20.0 + 4.0 + 40.0 + 16.0 + 40.0;
        assert_eq!(l.column.height, total);
        assert!((l.column.center().y - 300.0).abs() < 1e-3);
    }

    #[test]
    fn overflowing_column_pins_to_top() {
        let mut m = metrics(0.0, 48.0);
        m.title = Size::new(100.0, 200.0);
        let l = empty_layout(Rect::new(0.0, 10.0, 300.0, 100.0), &m);
        assert_eq!(l.title.y, 10.0, "başlık yukarıdan kesilmez");
    }

    // --- Bileşen -------------------------------------------------------------------------------

    fn draw(e: &mut EmptyState, theme: &Theme, text: &mut TextSystem) -> (Vec<DrawKind>, bool) {
        let mut rec = Recorder::new();
        let mut wants = false;
        rec.record(text, (800, 800), 1.0, |p| {
            let mut ui = Ui::new(p, theme, Instant::now());
            e.draw(&mut ui);
            wants = ui.wants_frame();
        });
        (rec.draws().into_iter().map(|d| d.kind).collect(), wants)
    }

    /// Dar alanda uzun eylem etiketi `…` ile kısalır: düğme sütunun içinde kalır, etiket düğmenin
    /// içinde; geniş alanda tam etiket geri gelir. Erişilebilirlik adı her zaman tam metin.
    #[test]
    fn long_action_label_shortens_in_narrow_column() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let long = "Ağ ayarlarını aç ve bağlantıyı yeniden dene lütfen";
        let mut e = EmptyState::new(&theme, "Bağlantı yok").with_action(&theme, long);
        let full = e.action_mut().unwrap().preferred_size(&mut text, &theme).width;
        let narrow = 200.0;
        e.set_rect(Rect::new(0.0, 0.0, narrow, 400.0));
        let l = e.layout(&mut text, &theme);
        let inner = e.rect().expand(-EmptyState::padding(&theme));
        assert!(full > inner.width, "deneme etiketi sığmayacak kadar uzun olmalı ({full})");
        assert!(l.action.x >= inner.x && l.action.right() <= inner.right(), "düğme taşmamalı: {:?} / {inner:?}", l.action);
        let b = e.action_mut().unwrap();
        assert_eq!(b.rect(), l.action);
        let label = b.label_mut().unwrap();
        let lw = label.measure(&mut text);
        assert!(lw.width <= l.action.width, "etiket düğmeye sığmalı: {lw:?}");
        assert!(lw.height < theme.space(10), "tek satır");
        assert_eq!(label.text(), long, "metnin kendisi kırpılmaz (erişilebilirlik adı)");
        let (kinds, wants) = draw(&mut e, &theme, &mut text);
        assert!(!kinds.is_empty() && !wants);
        // Genişleyince etiket metin sütununa (`space(90)`) dek açılır.
        e.set_rect(Rect::new(0.0, 0.0, 1200.0, 400.0));
        let l = e.layout(&mut text, &theme);
        assert_eq!(l.action.width, full.min(theme.space(90)));
    }

    #[cfg(feature = "brand")]
    #[test]
    fn mood_getter_reports_mascot_only() {
        let theme = Theme::default();
        let mut e = EmptyState::new(&theme, "Sonuç yok").with_mascot(&theme, Mood::NoResults);
        assert_eq!(e.mood(), Some(Mood::NoResults));
        e.set_mood(Mood::Error);
        assert_eq!(e.mood(), Some(Mood::Error));
        // Gizleme seçimi silmez.
        let mut cfg = nuxut_theme::FrameworkConfig::default();
        cfg.appearance.mascot = false;
        let hidden = Theme::from_config(&cfg).0;
        e.set_theme(&hidden, Instant::now());
        assert_eq!(e.mood(), Some(Mood::Error));
        assert_eq!(EmptyState::new(&theme, "x").mood(), None);
        assert_eq!(EmptyState::new(&theme, "x").with_icon(&theme, Icon::named(IconName::Search)).mood(), None);
    }

    #[cfg(feature = "brand")]
    #[test]
    fn mascot_shrinks_drops_and_hides_with_text_centered() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut e = EmptyState::new(&theme, "Bildirim yok").with_mascot(&theme, Mood::Empty).with_body(&theme, "Yeni bildirimler burada görünür.");
        e.set_rect(Rect::new(0.0, 0.0, 400.0, 500.0));
        let big = e.layout(&mut text, &theme);
        assert_eq!(big.art.map(|r| r.width), Some(EMPTY_MASCOT_SIZE));
        // Kısa alan: küçülür.
        let pad = theme.space(4);
        let text_h = big.column.bottom() - big.title.y;
        e.set_rect(Rect::new(0.0, 0.0, 400.0, pad * 2.0 + theme.space(4) + text_h + 60.0));
        let mid = e.layout(&mut text, &theme);
        let side = mid.art.unwrap().width;
        assert!((side - 60.0).abs() < 0.5, "{side}");
        // Daha kısa: düşer, metin ortada.
        let h = pad * 2.0 + text_h + 30.0;
        e.set_rect(Rect::new(0.0, 0.0, 400.0, h));
        let small = e.layout(&mut text, &theme);
        assert_eq!(small.art, None);
        assert!((small.column.center().y - h * 0.5).abs() < 0.5);
        // Gizleme: büyük alanda da yer kaplamaz, metin ortada.
        let mut hidden = theme.clone();
        hidden.appearance.mascot = false;
        e.set_rect(Rect::new(0.0, 0.0, 400.0, 500.0));
        e.set_theme(&hidden, Instant::now());
        let l = e.layout(&mut text, &hidden);
        assert_eq!(l.art, None);
        assert!((l.column.center().y - 250.0).abs() < 0.5);
        let (kinds, _) = draw(&mut e, &hidden, &mut text);
        assert!(!kinds.contains(&DrawKind::Image), "gizli maskot çizilmez");
        // Geri açılınca maskot döner.
        e.set_theme(&theme, Instant::now());
        assert!(e.layout(&mut text, &theme).art.is_some());
        let (kinds, _) = draw(&mut e, &theme, &mut text);
        assert!(kinds.contains(&DrawKind::Image));
    }

    #[cfg(feature = "brand")]
    fn hidden_theme(theme: &Theme) -> Theme {
        let mut hidden = theme.clone();
        hidden.appearance.mascot = false;
        hidden
    }

    #[cfg(feature = "brand")]
    fn icon_draws(kinds: &[DrawKind]) -> usize {
        kinds.iter().filter(|k| matches!(k, DrawKind::Glyphs)).count()
    }

    #[cfg(feature = "brand")]
    #[test]
    fn hidden_mascot_with_fallback_draws_icon_like_icon_mode() {
        let theme = Theme::default();
        let hidden = hidden_theme(&theme);
        let mut text = TextSystem::new();
        let area = Rect::new(0.0, 0.0, 400.0, 400.0);
        let mk = || EmptyState::new(&theme, "Bildirim yok").with_body(&theme, "Sessiz kip açık.");
        let mut e = mk().with_mascot(&theme, Mood::Empty).with_fallback_icon(&theme, Icon::named(IconName::Search));
        assert_eq!(e.fallback_icon(), Some(&Icon::named(IconName::Search)));
        e.set_rect(area);
        e.set_theme(&hidden, Instant::now());
        let (kinds, wants) = draw(&mut e, &hidden, &mut text);
        assert!(!kinds.contains(&DrawKind::Image), "gizli maskot çizilmez");
        assert!(!wants, "boşta kare yok");
        let fallback = e.last_layout().unwrap();
        // İkonlu kiple birebir aynı yerleşim ve çizim.
        let mut icon = mk().with_icon(&hidden, Icon::named(IconName::Search));
        icon.set_rect(area);
        let (icon_kinds, _) = draw(&mut icon, &hidden, &mut text);
        assert_eq!(Some(fallback), icon.last_layout());
        assert_eq!(fallback.art.map(|r| r.width), Some(text_style(&theme, TextRole::IconLarge).size));
        assert_eq!(kinds, icon_kinds);
        // Görünür maskot: maskot çizilir, yedek ikon çizilmez.
        e.set_theme(&theme, Instant::now());
        let (kinds, _) = draw(&mut e, &theme, &mut text);
        assert!(kinds.contains(&DrawKind::Image));
        assert_eq!(e.last_layout().unwrap().art.map(|r| r.width), Some(EMPTY_MASCOT_SIZE));
        let mut plain = mk().with_mascot(&theme, Mood::Empty);
        plain.set_rect(area);
        let (plain_kinds, _) = draw(&mut plain, &theme, &mut text);
        assert_eq!(icon_draws(&kinds), icon_draws(&plain_kinds), "maskot görünürken ek ikon yok");
        // Canlı geri geçiş ve kaldırma: yedeksiz gizli = eski davranış (yer kapanır).
        e.set_theme(&hidden, Instant::now());
        assert_eq!(e.layout(&mut text, &hidden), fallback);
        e.set_fallback_icon(&hidden, None);
        assert!(e.damage.is_dirty());
        assert_eq!(e.fallback_icon(), None);
        let l = e.layout(&mut text, &hidden);
        assert_eq!(l.art, None);
        let mut old = mk().with_mascot(&theme, Mood::Empty);
        old.set_rect(area);
        old.set_theme(&hidden, Instant::now());
        assert_eq!(old.layout(&mut text, &hidden), l);
    }

    #[cfg(feature = "brand")]
    #[test]
    fn fallback_icon_does_not_scale_and_drops() {
        let theme = Theme::default();
        let hidden = hidden_theme(&theme);
        let mut text = TextSystem::new();
        let mut e = EmptyState::new(&hidden, "Boş").with_mascot(&hidden, Mood::Empty).with_fallback_icon(&hidden, Icon::named(IconName::Search));
        e.set_rect(Rect::new(0.0, 0.0, 400.0, 400.0));
        let big = e.layout(&mut text, &hidden);
        let side = big.art.unwrap().width;
        let pad = theme.space(4);
        let title_h = big.column.bottom() - big.title.y;
        // İkon tam sığar.
        e.set_rect(Rect::new(0.0, 0.0, 400.0, pad * 2.0 + theme.space(4) + title_h + side));
        assert_eq!(e.layout(&mut text, &hidden).art.map(|r| r.width), Some(side));
        // Bir piksel eksik: küçülmez, düşer.
        e.set_rect(Rect::new(0.0, 0.0, 400.0, pad * 2.0 + theme.space(4) + title_h + side - 1.0));
        assert_eq!(e.layout(&mut text, &hidden).art, None);
    }

    #[test]
    fn fallback_is_inert_outside_mascot_mode() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut hidden = theme.clone();
        hidden.appearance.mascot = false;
        let mut plain = EmptyState::new(&theme, "Boş");
        let mut with = EmptyState::new(&theme, "Boş").with_fallback_icon(&theme, Icon::named(IconName::Search));
        for e in [&mut plain, &mut with] {
            e.set_rect(Rect::new(0.0, 0.0, 400.0, 400.0));
        }
        assert_eq!(plain.layout(&mut text, &hidden), with.layout(&mut text, &hidden));
        assert_eq!(with.last_layout().unwrap().art, None);
    }

    #[test]
    fn long_text_wraps_within_line_limits_and_byte_cap() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let long = "uzun ".repeat(400);
        let mut e = EmptyState::new(&theme, long.clone()).with_body(&theme, long.clone());
        assert!(e.title().len() <= MAX_EMPTY_TEXT && e.body().unwrap().len() <= MAX_EMPTY_TEXT);
        e.set_rect(Rect::new(0.0, 0.0, 300.0, 600.0));
        let _ = e.layout(&mut text, &theme);
        let w = 300.0 - theme.space(4) * 2.0;
        assert_eq!(e.title.max_width(), Some(w.floor()));
        let title = e.title.layout(&mut text);
        assert!(title.line_count() <= EMPTY_TITLE_LINES);
        let body = e.body.as_mut().unwrap().layout(&mut text);
        assert!(body.line_count() <= EMPTY_BODY_LINES);
        // Geniş alanda metin sütunu space(90) ile sınırlı.
        e.set_rect(Rect::new(0.0, 0.0, 2000.0, 600.0));
        let _ = e.layout(&mut text, &theme);
        assert_eq!(e.title.max_width(), Some(theme.space(90)));
        // Gövde kaldırılabilir.
        e.set_body(&theme, None);
        assert_eq!(e.body(), None);
        e.set_body(&theme, Some(""));
        assert_eq!(e.body(), None);
    }

    #[test]
    fn icon_variant_draws_without_idle_frames() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut e = EmptyState::new(&theme, "Sonuç yok").with_icon(&theme, Icon::named(IconName::Search));
        e.set_rect(Rect::new(0.0, 0.0, 400.0, 400.0));
        let (_, wants) = draw(&mut e, &theme, &mut text);
        assert!(!wants, "boşta kare yok");
        let l = e.last_layout().unwrap();
        let side = text_style(&theme, TextRole::IconLarge).size;
        assert_eq!(l.art.map(|r| r.width), Some(side));
        // Yerleşim aynı kaldıkça ikinci çizim yeniden yerleşim hasarı üretmez.
        assert!(!e.damage.is_dirty());
        // Boş alan: hiçbir şey çizilmez.
        e.set_rect(Rect::ZERO);
        let (kinds, _) = draw(&mut e, &theme, &mut text);
        assert!(kinds.is_empty());
    }

    #[test]
    fn action_click_is_reported() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut e = EmptyState::new(&theme, "Bağlantı yok").with_action(&theme, "Yeniden dene");
        e.set_rect(Rect::new(0.0, 0.0, 400.0, 400.0));
        let l = e.layout(&mut text, &theme);
        let c = l.action.center();
        assert!(l.action.width > 0.0 && l.action.y > l.title.y);
        assert_eq!(e.cursor(c), Some(CursorShape::Pointer));
        let _ = e.pointer(&press(c.x, c.y), t0);
        assert_eq!(e.pointer(&release(c.x, c.y), t0).event, Some(()));
        e.set_action(&theme, None);
        assert!(e.action_mut().is_none());
        assert_eq!(e.pointer(&press(c.x, c.y), t0).event, None);
    }

    #[test]
    fn theme_change_relayouts() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut e = EmptyState::new(&theme, "Pano boş").with_body(&theme, "Kopyaladıkların burada.");
        e.set_rect(Rect::new(0.0, 0.0, 400.0, 400.0));
        let a = e.layout(&mut text, &theme);
        let mut big = theme.clone();
        big.typography.size *= 1.5;
        e.set_theme(&big, Instant::now());
        assert!(e.damage.is_dirty());
        let b = e.layout(&mut text, &big);
        assert!(b.column.height > a.column.height, "{} > {}", b.column.height, a.column.height);
    }

    #[test]
    fn measure_includes_padding() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let mut e = EmptyState::new(&theme, "Boş");
        let s = e.measure(&mut text, &theme, 300.0);
        assert!(s.width <= 300.0 && s.height >= theme.space(8));
    }
}
