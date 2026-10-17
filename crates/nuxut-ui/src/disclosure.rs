//! Açılır satır (aşamalı açıklama, "Disclosure"/"Expander"): başlık + isteğe bağlı açıklama,
//! kapalıyken özet metni ("11 renk · 2'si elle ayarlı"), isteğe bağlı nokta rozeti (içeride
//! varsayılandan farklı değer var) ve açılınca yayla dönen chevron. İçerik yüksekliği
//! `motion.spatial_fast` yayıyla, kesilebilir biçimde açılır/kapanır.
//!
//! İçerik çağıranındır: çağıran içeriğin tam (açık) yüksekliğini
//! [`set_content_height`](Disclosure::set_content_height) ile bildirir, bileşen açılma oranını
//! ([`progress`](Disclosure::progress)), içeriğin yerleşim dikdörtgenini
//! ([`content_rect`](Disclosure::content_rect)) ve görünen (kırpma) dikdörtgenini
//! ([`content_clip`](Disclosure::content_clip)) verir; [`draw_content`](Disclosure::draw_content)
//! kırpma + solma ile çağıranın çizimini sarar.
//!
//! Başlık satırı tıklanabilir (Pressable): tık, odaktayken Enter/Space (aç/kapa), Sağ (aç),
//! Sol (kapa); odak halkası satırın içine çizilir (kart kenarında kırpılmasın). Arama sonucu
//! için [`flash`](Disclosure::flash) / [`flash_content`](Disclosure::flash_content): vurgu
//! `timing.flash_hold` boyunca tam görünür, sonra `motion.effects` ile söner (bekleme süresi
//! kare değil [`Ui::wake_at`] ister). Boşta kare istenmez.

use std::f32::consts::PI;
use std::time::Instant;

use nuxut_core::{Color, Corners, Insets, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::label::Label;
use crate::pressable::Pressable;
use crate::style::{FLASH_TONE, FOCUS_RING, HAIRLINE, clip_text, disabled_content, row_height, state_layer_under};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// Başlık/açıklama/özet metninin en uzun hâli (bayt).
pub const MAX_DISCLOSURE_TEXT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureEvent {
    /// Kullanıcı açtı (`true`) ya da kapattı (`false`); bileşen durumu zaten güncelledi.
    Toggled(bool),
}

/// Vurgunun yeri.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FlashTarget {
    Header,
    /// İçerik koordinatında (içeriğin sol-üstü = 0,0) bir dikdörtgen.
    Content(Rect),
}

/// Ölçüme bağlı başlık yerleşimi (son `header_height`/`draw`'da).
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeaderLayout {
    width: f32,
    height: f32,
    /// Özet başlıkla aynı satırda, chevron'un solunda (sığmıyorsa altta).
    summary_beside: bool,
}

#[derive(Debug, Clone)]
pub struct Disclosure {
    /// Başlık satırı (rect = başlık dikdörtgeni).
    core: Pressable,
    title: Label,
    description: Option<Label>,
    summary: Label,
    badge: bool,
    /// Rozet ölçeği 0..1 (`spatial_fast`).
    badge_scale: Animated<f32>,
    open: bool,
    /// 0 = kapalı, 1 = açık (`spatial_fast`; az sönümlü yayda hafif taşabilir).
    expand: Animated<f32>,
    content_height: f32,
    flash: Animated<f32>,
    flash_target: FlashTarget,
    /// Vurgunun tam opaklıkta kalacağı son an; geçince sönmeye başlar.
    flash_until: Option<Instant>,
    /// Tüm blok (başlık + görünen içerik).
    rect: Rect,
    layout: Option<HeaderLayout>,
}

impl Disclosure {
    pub fn new(theme: &Theme, title: &str) -> Self {
        Self {
            core: Pressable::new(theme),
            title: Label::new(clip_text(title, MAX_DISCLOSURE_TEXT), title_style(theme)),
            description: None,
            summary: Label::new("", caption_style(theme)),
            badge: false,
            badge_scale: Animated::new(0.0, theme.motion.spatial_fast),
            open: false,
            expand: Animated::new(0.0, theme.motion.spatial_fast),
            content_height: 0.0,
            flash: Animated::new(0.0, theme.motion.effects),
            flash_target: FlashTarget::Header,
            flash_until: None,
            rect: Rect::ZERO,
            layout: None,
        }
    }

    /// Başlığın altında kısa açıklama (`Caption`, `on_surface_variant`).
    pub fn with_description(mut self, theme: &Theme, description: &str) -> Self {
        self.set_description(theme, Some(description));
        self
    }

    pub fn title(&self) -> &str {
        self.title.text()
    }

    pub fn set_title(&mut self, title: &str) {
        if self.title.set_text(clip_text(title, MAX_DISCLOSURE_TEXT)) {
            self.relayout();
        }
    }

    pub fn set_description(&mut self, theme: &Theme, description: Option<&str>) {
        match (description.filter(|d| !d.is_empty()), &mut self.description) {
            (None, None) => return,
            (None, d @ Some(_)) => *d = None,
            (Some(s), Some(l)) => {
                if !l.set_text(clip_text(s, MAX_DISCLOSURE_TEXT)) {
                    return;
                }
            }
            (Some(s), d @ None) => *d = Some(Label::new(clip_text(s, MAX_DISCLOSURE_TEXT), caption_style(theme))),
        }
        self.relayout();
    }

    /// Kapalıyken gösterilen özet (boş = yok). Açılırken söner.
    pub fn set_summary(&mut self, summary: &str) {
        if self.summary.set_text(clip_text(summary, MAX_DISCLOSURE_TEXT)) {
            self.relayout();
        }
    }

    pub fn summary(&self) -> &str {
        self.summary.text()
    }

    /// Nokta rozeti (içeride varsayılandan farklı değer var); başlığın hemen sağında,
    /// `spatial_fast` ölçek yayıyla belirir/kaybolur.
    pub fn set_badge(&mut self, on: bool, now: Instant) {
        if self.badge != on {
            self.badge = on;
            self.badge_scale.set_target(if on { 1.0 } else { 0.0 }, now);
            self.relayout();
        }
    }

    pub fn has_badge(&self) -> bool {
        self.badge
    }

    /// Tema değişimi; `TextSystem` gerekmez (ölçüm bir sonraki `header_height`/`draw`'da).
    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.title.set_style(title_style(theme));
        if let Some(d) = &mut self.description {
            d.set_style(caption_style(theme));
        }
        self.summary.set_style(caption_style(theme));
        self.badge_scale.set_motion(theme.motion.spatial_fast, now);
        self.expand.set_motion(theme.motion.spatial_fast, now);
        self.flash.set_motion(theme.motion.effects, now);
        self.relayout();
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) {
        self.core.set_enabled(enabled, now);
    }

    pub fn is_enabled(&self) -> bool {
        self.core.enabled
    }

    /// Klavye odak halkası (başlık satırının içinde).
    pub fn set_focus_visible(&mut self, on: bool) {
        self.core.set_focus_visible(on);
    }

    // --- Açık/kapalı ----------------------------------------------------------------------

    /// Kullanıcıya göre açık (kapanış animasyonu sayılmaz).
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Açar/kapatır. `animate = false` anında (ör. arama sonucuna gitmeden önce kaydırma
    /// hedefi son hâle göre hesaplansın; oturumdan geri yükleme). Hareket kapalıysa her
    /// zaman anında. Durum değiştiyse `true`. Olay üretmez (çağıranın kendi isteği).
    pub fn set_open(&mut self, open: bool, animate: bool, now: Instant) -> bool {
        let target = if open { 1.0 } else { 0.0 };
        let changed = self.open != open;
        self.open = open;
        if animate {
            self.expand.set_target(target, now);
        } else {
            self.expand.jump_to(target);
        }
        self.core.damage.mark();
        changed
    }

    fn toggle(&mut self, now: Instant) -> Response<DisclosureEvent> {
        let open = !self.open;
        self.set_open(open, true, now);
        Response::event(DisclosureEvent::Toggled(open))
    }

    /// Açılma oranı: 0 = kapalı, 1 = açık. Yay az sönümlüyse geçişte 1'i hafifçe aşabilir
    /// (yükseklik "puding"); solma için [`reveal`](Self::reveal).
    pub fn progress(&self, now: Instant) -> f32 {
        self.expand.value(now).max(0.0)
    }

    /// İçerik opaklığı için 0..=1'e kırpılmış açılma oranı.
    pub fn reveal(&self, now: Instant) -> f32 {
        self.expand.value(now).clamp(0.0, 1.0)
    }

    /// Açılma/kapanma, rozet, vurgu sönmesi ya da hover geçişi sürüyor mu (kare gerekir).
    /// Vurgunun tam opaklıktaki bekleyişi sayılmaz: o süre [`Ui::wake_at`] ile beklenir.
    pub fn is_animating(&mut self, now: Instant) -> bool {
        let flash_fading = self.flash_until.is_none() && !self.flash.is_settled(now);
        !self.expand.is_settled(now) || !self.badge_scale.is_settled(now) || flash_fading || self.core.is_animating(now)
    }

    // --- Yerleşim -------------------------------------------------------------------------

    /// İçeriğin tam (açıkken) yüksekliği; çağıran bildirir (içerik değişince yeniden).
    pub fn set_content_height(&mut self, height: f32) {
        let h = if height.is_finite() { height.max(0.0) } else { 0.0 };
        if h != self.content_height {
            self.content_height = h;
            self.core.damage.mark();
        }
    }

    pub fn content_height(&self) -> f32 {
        self.content_height
    }

    /// Başlık satırının yüksekliği (`width` genişliğinde): en az [`row_height`] (standart satır), iç boşluk
    /// yatay `space(4)` dikey `space(3)`; özet sığarsa başlıkla aynı satırda, sığmazsa altta.
    pub fn header_height(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> f32 {
        self.ensure_layout(text, theme, width).height
    }

    /// Bloğun bu karedeki yüksekliği: başlık + içerik × açılma oranı. Önce
    /// [`header_height`](Self::header_height) (ya da bir çizim) ölçmüş olmalı.
    pub fn height(&self, now: Instant) -> f32 {
        let head = self.layout.map_or(0.0, |l| l.height);
        let e = self.progress(now);
        if e <= 0.001 { head } else { head + self.content_height * e }
    }

    /// Tüm blok: başlık üstte, `header_height` yüksekliğinde; içerik altında.
    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.core.damage.mark();
        }
        let head = Rect::new(rect.x, rect.y, rect.width, self.layout.map_or(rect.height, |l| l.height.min(rect.height)));
        self.core.set_rect(head);
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Başlık satırı (tıklanan bölge).
    pub fn header_rect(&self) -> Rect {
        self.core.rect
    }

    /// İçeriğin yerleşim dikdörtgeni (tam yükseklik, başlığın hemen altı). Satırlar buna
    /// göre yerleştirilir; açılırken kırpma [`content_clip`](Self::content_clip) ile.
    pub fn content_rect(&self) -> Rect {
        let h = self.core.rect;
        Rect::new(h.x, h.bottom(), h.width, self.content_height)
    }

    /// İçeriğin bu karede görünen bölümü (kırpma dikdörtgeni). Kapalıyken boş.
    pub fn content_clip(&self, now: Instant) -> Rect {
        let h = self.core.rect;
        Rect::new(h.x, h.bottom(), h.width, self.content_height * self.progress(now))
    }

    /// Ölçüm gerekiyorsa yapar (genişlik değişti, metin/tema/rozet değişti).
    fn ensure_layout(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> HeaderLayout {
        if let Some(l) = self.layout
            && l.width == width
        {
            return l;
        }
        let l = self.measure(text, theme, width);
        self.layout = Some(l);
        self.core.damage.mark();
        l
    }

    fn relayout(&mut self) {
        self.layout = None;
        self.core.damage.mark();
    }

    fn measure(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> HeaderLayout {
        let (px, py) = padding(theme);
        let gap = theme.space(2);
        let side = chevron_size(theme) + gap;
        let text_w = (width - px * 2.0 - side).max(theme.space(20));
        let badge_w = if self.badge { badge_size(theme) + gap } else { 0.0 };
        let has_summary = !self.summary.text().is_empty();

        // Özet başlıkla yan yana sığıyor mu (doğal genişlikleriyle)?
        self.summary.set_max_width(None);
        let sw = if has_summary { self.summary.measure(text).width } else { 0.0 };
        self.title.set_max_width(None);
        let tw = self.title.measure(text).width;
        let beside = has_summary && tw + badge_w + theme.space(4) + sw <= text_w;

        let column = if beside { text_w - sw - theme.space(4) } else { text_w };
        self.title.set_max_width(Some((column - badge_w).max(0.0)));
        let mut h = self.title.measure(text).height;
        if let Some(d) = &mut self.description {
            d.set_max_width(Some(column));
            h += theme.space(1) + d.measure(text).height;
        }
        if has_summary && !beside {
            self.summary.set_max_width(Some(text_w));
            h += theme.space(1) + self.summary.measure(text).height;
        }
        HeaderLayout { width, height: (h + py * 2.0).max(row_height(theme)), summary_beside: beside }
    }

    // --- Girdi ----------------------------------------------------------------------------

    /// Başlık satırına tık açar/kapatır. İçerikteki olaylar çağıranın.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<DisclosureEvent> {
        let r = self.core.pointer(event, now);
        if r.event.is_some() {
            return self.toggle(now);
        }
        r.discard()
    }

    /// Başlık odaktayken: Enter/Space aç/kapa, Sağ aç, Sol kapa.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<DisclosureEvent> {
        if !self.core.enabled || event.state != KeyState::Pressed {
            return Response::none();
        }
        match event.key {
            Key::Enter | Key::Space => {
                self.core.pulse(now);
                self.toggle(now)
            }
            Key::Right if !self.open => self.toggle(now),
            Key::Left if self.open => self.toggle(now),
            _ => Response::none(),
        }
    }

    /// Başlık satırında `Pointer` (devre dışı `Default`); dışarıda `None` (içerik çağıranın).
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    // --- Vurgu ----------------------------------------------------------------------------

    /// Başlık satırını parlatır (ör. arama sonucu açılır satırın kendisi).
    pub fn flash(&mut self, now: Instant) {
        self.start_flash(FlashTarget::Header, now);
    }

    /// İçerikteki bir satırı parlatır; `rect` içerik koordinatında (içeriğin sol-üstü 0,0).
    /// Kapalıysa önce [`set_open`](Self::set_open) çağrılmalı (vurgu kırpılır).
    pub fn flash_content(&mut self, rect: Rect, now: Instant) {
        self.start_flash(FlashTarget::Content(rect), now);
    }

    fn start_flash(&mut self, target: FlashTarget, now: Instant) {
        self.flash_target = target;
        self.flash.jump_to(1.0);
        self.flash_until = Some(now);
        self.core.damage.mark();
    }

    // --- Çizim ----------------------------------------------------------------------------

    /// Başlık satırı ve vurgu. İçerik için ardından [`draw_content`](Self::draw_content)
    /// (ya da `content_clip` ile kendi çizimin).
    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let head = self.core.rect;
        let layout = self.ensure_layout(ui.painter.text_system(), theme, head.width);
        // Ölçüm başlık yüksekliğini değiştirdiyse başlık dikdörtgeni yenilenir (set_rect sonra
        // gelirse yine düzelir).
        if layout.height != head.height && layout.height <= self.rect.height + 0.5 {
            self.core.set_rect(Rect::new(head.x, head.y, head.width, layout.height));
        }
        let head = self.core.rect;
        let e = self.progress(now);
        let radii = theme.shape.small();

        // Vurgu süresi: tam opaklık `flash_hold` boyunca (zamanlayıcı, kare değil), sonra söner.
        if let Some(start) = self.flash_until {
            let until = start + theme.timing.flash_hold;
            if now >= until {
                self.flash_until = None;
                self.flash.set_target(0.0, now);
            } else {
                ui.wake_at(until);
            }
        }
        let flash = self.flash.value(now).clamp(0.0, 1.0);
        if flash > 0.01 {
            let (r, clip) = match self.flash_target {
                FlashTarget::Header => (head, None),
                FlashTarget::Content(r) => (r.translate(Point::new(head.x, head.bottom())), Some(self.content_clip(now))),
            };
            if let Some(clip) = clip {
                ui.painter.push_clip(clip);
            }
            let flash = state_layer_under(theme, c.primary, FLASH_TONE * flash, &[], &[c.on_surface, c.on_surface_variant]);
            ui.painter.rect(r, &RectStyle::filled(ui.fade(flash), radii));
            if clip.is_some() {
                ui.painter.pop_clip();
            }
        }

        self.core.interaction.draw_state_layer_under(ui, head, radii, c.on_surface, Color::TRANSPARENT, &[c.on_surface, c.on_surface_variant]);

        let content = |col: Color| if self.core.enabled { col } else { disabled_content(theme, col, &[]) };
        let (px, py) = padding(theme);
        let x = head.x + px;
        let title_h = self.title.measure_in(ui.painter).height;
        let desc_h = self.description.as_mut().map_or(0.0, |d| d.measure_in(ui.painter).height + theme.space(1));
        let has_summary = !self.summary.text().is_empty();
        let sum = if has_summary { self.summary.measure_in(ui.painter) } else { nuxut_core::Size::ZERO };
        let below_h = if has_summary && !layout.summary_beside { sum.height + theme.space(1) } else { 0.0 };
        let mut y = head.y + ((head.height - title_h - desc_h - below_h) * 0.5).max(py);
        let title_at = Point::new(x, y);
        self.title.draw(ui.painter, title_at, ui.fade(content(c.on_surface)));

        // Rozet: başlığın ilk satırının sağında, dikey ortada.
        let k = self.badge_scale.value(now).max(0.0);
        if k > 0.01 {
            let d = badge_size(theme) * k;
            let tw = self.title.measure_in(ui.painter).width;
            let ts = title_style(theme);
            let line_h = (ts.size * ts.line_height).min(title_h);
            let cx = title_at.x + tw + theme.space(2) + badge_size(theme) * 0.5;
            let cy = title_at.y + line_h * 0.5;
            let dot = Rect::new(cx - d * 0.5, cy - d * 0.5, d, d);
            ui.painter.rect(dot, &RectStyle::filled(ui.fade(content(c.primary)), Corners::uniform(d * 0.5)));
        }
        y += title_h;
        if let Some(d) = &mut self.description {
            d.draw(ui.painter, Point::new(x, y + theme.space(1)), ui.fade(content(c.on_surface_variant)));
            y += desc_h;
        }

        // Chevron: aşağı bakan ok, açılma oranıyla 180° döner (aynı yay; taşma dahil).
        let cs = chevron_size(theme);
        let center = Point::new(head.right() - px - cs * 0.5, head.center().y);
        let chevron_color = ui.fade(content(c.on_surface_variant));
        draw_chevron(ui, center, cs, PI * e, chevron_color);

        // Özet: kapalıyken; açılırken söner.
        let summary_alpha = 1.0 - self.reveal(now);
        if has_summary && summary_alpha > 0.01 {
            let at = if layout.summary_beside {
                Point::new(center.x - cs * 0.5 - theme.space(2) - sum.width, head.center().y - sum.height * 0.5)
            } else {
                Point::new(x, y + theme.space(1))
            };
            let color = ui.fade(content(c.on_surface_variant).multiply_alpha(summary_alpha));
            self.summary.draw(ui.painter, at, color);
        }

        // Odak halkası satırın içinde (kart kenarında kırpılmaz).
        let ring = head.inset(Insets::uniform(FOCUS_RING * 2.0));
        self.core.draw_focus(ui, ring, radii);

        let animating = self.is_animating(now);
        if animating {
            ui.request_frame();
        }
        // Başlık + görünen içerik (içerik değişimi de bu bölgede; eski ∪ yeni bildirilir).
        let extent = Rect::new(head.x, head.y, head.width, head.height + self.content_height * e);
        self.core.damage.report(ui, extent, animating);
    }

    /// İçeriği çizer: görünen bölüme kırpar, açılma oranıyla soldurur; `f` içeriğin tam
    /// yerleşim dikdörtgenini alır. Kapalıyken çağrılmaz.
    pub fn draw_content(&self, ui: &mut Ui, f: impl FnOnce(&mut Ui, Rect)) {
        let now = ui.now;
        let clip = self.content_clip(now);
        if clip.height <= 0.5 || clip.width <= 0.5 {
            return;
        }
        let alpha = self.reveal(now);
        let rect = self.content_rect();
        ui.painter.push_clip(clip);
        ui.with_alpha(alpha, |ui| f(ui, rect));
        ui.painter.pop_clip();
    }
}

fn title_style(theme: &Theme) -> nuxut_render::TextStyle {
    text_style(theme, TextRole::Body).with_max_lines(2)
}

fn caption_style(theme: &Theme) -> nuxut_render::TextStyle {
    text_style(theme, TextRole::Caption).with_max_lines(2)
}

/// Satır kalıbı iç boşluğu (settings-ux §5.1): yatay `space(4)`, dikey `space(3)`.
fn padding(theme: &Theme) -> (f32, f32) {
    (theme.space(4), theme.space(3))
}

/// Chevron kutusu: küçük ikon rolünün boyu.
fn chevron_size(theme: &Theme) -> f32 {
    text_style(theme, TextRole::IconSmall).size
}

fn badge_size(theme: &Theme) -> f32 {
    theme.space(2)
}

/// Çizgisel chevron (`angle` = 0 aşağı bakar); ikon fontundaki glif döndürülemediği için.
fn draw_chevron(ui: &mut Ui, center: Point, size: f32, angle: f32, color: Color) {
    let a = size * 0.25;
    let (s, c) = angle.sin_cos();
    let rot = |x: f32, y: f32| Point::new(center.x + x * c - y * s, center.y + x * s + y * c);
    let pts = [rot(-a, -a * 0.5), rot(0.0, a * 0.5), rot(a, -a * 0.5)];
    let width = (size / 12.0).max(HAIRLINE * 1.5);
    ui.painter.polyline(&pts, width, color);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    fn setup(theme: &Theme) -> (Disclosure, TextSystem) {
        let mut text = TextSystem::new();
        let mut d = Disclosure::new(theme, "Renkleri tek tek ayarla").with_description(theme, "Uzman ayarı");
        d.set_summary("11 renk");
        d.set_content_height(200.0);
        let h = d.header_height(&mut text, theme, 480.0);
        d.set_rect(Rect::new(0.0, 0.0, 480.0, h));
        (d, text)
    }

    fn settle(d: &mut Disclosure, from: Instant) -> Instant {
        let mut t = from;
        for _ in 0..600 {
            t += Duration::from_millis(16);
            if !d.is_animating(t) {
                return t;
            }
        }
        panic!("animasyon bitmedi");
    }

    #[test]
    fn header_is_at_least_row_minimum() {
        let theme = Theme::default();
        let (d, _) = setup(&theme);
        assert!(d.header_rect().height >= row_height(&theme));
        assert_eq!(d.height(Instant::now()), d.header_rect().height);
    }

    #[test]
    fn click_toggles_with_spring_and_reports_event() {
        let theme = Theme::default();
        let (mut d, _) = setup(&theme);
        let t0 = Instant::now();
        let c = d.header_rect().center();
        let _ = d.pointer(&press(c.x, c.y), t0);
        let r = d.pointer(&release(c.x, c.y), t0);
        assert_eq!(r.event, Some(DisclosureEvent::Toggled(true)));
        assert!(d.is_open());
        let head = d.header_rect().height;
        let mid = d.height(t0 + Duration::from_millis(40));
        assert!(mid > head && mid < head + 200.0, "yayla açılıyor: {mid}");
        let t1 = settle(&mut d, t0);
        assert!((d.height(t1) - (head + 200.0)).abs() < 0.5);
        assert_eq!(d.content_clip(t1).height.round(), 200.0);
        // Kesilebilir: açılırken kapatınca o anki konumdan geri döner.
        let _ = d.pointer(&press(c.x, c.y), t1);
        let r = d.pointer(&release(c.x, c.y), t1);
        assert_eq!(r.event, Some(DisclosureEvent::Toggled(false)));
        let t2 = settle(&mut d, t1);
        assert_eq!(d.height(t2), head);
        assert!(!d.is_animating(t2), "boşta kare yok");
    }

    #[test]
    fn keyboard_enter_space_right_left() {
        let theme = Theme::default();
        let (mut d, _) = setup(&theme);
        let t0 = Instant::now();
        assert_eq!(d.key(&key(Key::Left), t0).event, None, "kapalıyken Sol etkisiz");
        assert_eq!(d.key(&key(Key::Right), t0).event, Some(DisclosureEvent::Toggled(true)));
        assert_eq!(d.key(&key(Key::Right), t0).event, None);
        assert_eq!(d.key(&key(Key::Left), t0).event, Some(DisclosureEvent::Toggled(false)));
        assert_eq!(d.key(&key(Key::Enter), t0).event, Some(DisclosureEvent::Toggled(true)));
        assert_eq!(d.key(&key(Key::Space), t0).event, Some(DisclosureEvent::Toggled(false)));
        d.set_enabled(false, t0);
        assert_eq!(d.key(&key(Key::Enter), t0).event, None);
    }

    #[test]
    fn set_open_without_animation_is_instant() {
        let theme = Theme::default();
        let (mut d, _) = setup(&theme);
        let t0 = Instant::now();
        assert!(d.set_open(true, false, t0));
        assert!(!d.set_open(true, false, t0));
        assert_eq!(d.progress(t0), 1.0);
        assert!(!d.is_animating(t0));
    }

    #[test]
    fn motion_disabled_is_instant() {
        let (theme, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        let (mut d, _) = setup(&theme);
        let t0 = Instant::now();
        let _ = d.key(&key(Key::Enter), t0);
        assert_eq!(d.progress(t0), 1.0);
    }

    #[test]
    fn summary_moves_below_when_narrow() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        // Açıklamalı: alta inen özet satırı en az yüksekliği (row_height) aşar.
        let mut d = Disclosure::new(&theme, "Renkleri tek tek ayarla").with_description(&theme, "Duvar kağıdından üretilen renkleri ezer");
        d.set_summary("11 renk · 2'si elle ayarlı");
        let wide = d.header_height(&mut text, &theme, 900.0);
        assert!(d.layout.unwrap().summary_beside);
        let narrow = d.header_height(&mut text, &theme, 260.0);
        assert!(!d.layout.unwrap().summary_beside);
        assert!(narrow > wide);
    }

    #[test]
    fn set_theme_needs_no_text_system_and_remeasures_lazily() {
        let theme = Theme::default();
        let (mut d, mut text) = setup(&theme);
        d.set_theme(&theme, Instant::now());
        assert!(d.layout.is_none());
        let _ = d.header_height(&mut text, &theme, 480.0);
        assert!(d.layout.is_some());
    }

    #[test]
    fn flash_holds_then_fades_and_goes_idle() {
        use nuxut_render::Recorder;
        let theme = Theme::default();
        let (mut d, mut text) = setup(&theme);
        let mut rec = Recorder::new();
        let t0 = Instant::now();
        d.flash(t0);
        let frame = |d: &mut Disclosure, text: &mut TextSystem, rec: &mut Recorder, now: Instant| {
            let mut out = (false, None);
            rec.record(text, (480, 400), 1.0, |p| {
                let mut ui = Ui::new(p, &theme, now);
                d.draw(&mut ui);
                out = (ui.wants_frame(), ui.wake_deadline());
            });
            out
        };
        // Bekleme süresince kare yok, zamanlayıcı var.
        let (wants, wake) = frame(&mut d, &mut text, &mut rec, t0);
        assert!(!wants, "beklerken kare istenmez");
        assert_eq!(wake, Some(t0 + theme.timing.flash_hold));
        // Süre dolunca söner (kare ister), sonra boşta.
        let t1 = t0 + theme.timing.flash_hold;
        let (wants, _) = frame(&mut d, &mut text, &mut rec, t1);
        assert!(wants);
        let mut t = t1;
        for _ in 0..200 {
            t += Duration::from_millis(16);
            let (wants, wake) = frame(&mut d, &mut text, &mut rec, t);
            if !wants {
                assert_eq!(wake, None);
                return;
            }
        }
        panic!("vurgu sönmedi");
    }

    #[test]
    fn badge_appears_with_spring() {
        let theme = Theme::default();
        let (mut d, _) = setup(&theme);
        let t0 = Instant::now();
        d.set_badge(true, t0);
        assert!(d.has_badge());
        assert!(d.is_animating(t0));
        let t1 = settle(&mut d, t0);
        assert_eq!(d.badge_scale.value(t1), 1.0);
    }
}
