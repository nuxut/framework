//! Bildirim kartı ([`NotificationCard`]) ve kartları yayla dizen [`NotificationStack`].
//!
//! Kart: uygulama ikonu + adı + zaman, başlık, gövde (çok satırlı, `…` ile kesilir), opsiyonel
//! görsel yuvası, eylem düğmeleri, kapatma düğmesi (hover'da belirir). Yatay sürükleme ile
//! kaydırarak kapatma: kart parmağı izler, uzaklaştıkça solar; eşik ya da hız aşılırsa
//! `spatial` yayıyla ekrandan uçar, aşılmazsa yayla yerine döner (puding).
//!
//! Görseller (uygulama ikonu dosyası, bildirim resmi): render tarafında görsel çizimi
//! gelene kadar kart yalnızca yerini ayırır — [`NotificationCard::image_rect`] /
//! [`NotificationCard::app_icon_rect`] dikdörtgenine çağıran kendisi çizer.
//!
//! Maskot yuvası ([`NotificationCard::with_mascot`], `brand` özelliği): yalnız Nuxut'un kendi
//! iç bildirimleri içindir ("güncelleme bitti" gibi; dış uygulama bildirimlerinde maskot yok —
//! `docs/design/mascot.md`). Maskot metin sütununun solunda, karosuz, [`NOTIFICATION_MASCOT_SIZE`]
//! kenarlı durur; kartın kaydırma/kapanma solmasıyla birlikte solar. `[appearance] mascot = false`
//! iken yuva hiç yoktur: metin sola yaslanır, kart yüksekliği yalnız metne göre ölçülür.

use std::fmt;
use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{ImageFit, ImageStyle, RectStyle, SpanStyle, TextLayout, TextStyle, TextSystem, Texture};
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, NodeId, Role, child_id};

use crate::button::{Button, ButtonVariant, IconButton, IconButtonVariant};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{Button as MouseButton, CursorShape, PointerEvent, PointerKind};
use crate::label::Label;
#[cfg(feature = "brand")]
use crate::mascot::{Mascot, Mood};
use crate::redact::{Count, Len, OptLen};
use crate::style::clip_text;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// En fazla gösterilen eylem düğmesi.
pub const MAX_NOTIFICATION_ACTIONS: usize = 3;

/// Bildirim kartındaki maskotun mantıksal kenarı (px). `docs/design/mascot.md` "Bildirim
/// yerleşimi": 64–96 aralığının alt ucu — 360–420 px kartta metin sütunu ≥ 240 px kalır, başlık +
/// iki satır gövdeyle aynı yükseklikte durur; `MASCOT_MIN_SIZE` (48) üstünde, yüz okunur.
pub const NOTIFICATION_MASCOT_SIZE: f32 = 64.0;
#[cfg(feature = "brand")]
const _: () = assert!(NOTIFICATION_MASCOT_SIZE >= crate::mascot::MASCOT_MIN_SIZE && NOTIFICATION_MASCOT_SIZE <= 96.0);

#[derive(Clone, PartialEq, Eq)]
pub enum NotificationEvent {
    /// Gövdeye tıklandı (varsayılan eylem).
    Activated,
    /// Eylem düğmesi (indis).
    Action(usize),
    /// Gövdedeki bağlantıya tıklandı (URL'yi açmak çağıranın işi; kart açmaz).
    Link(String),
    /// Kapatma düğmesi ya da kaydırma. Kart uçma animasyonunu sürdürür;
    /// [`NotificationCard::is_gone`] olunca kaldırılır.
    Dismissed,
}

/// Gövdenin bir parçası (freedesktop bildirim işaretlemesi: <b>, <i>, <u>, <a href>).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct BodySpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// Bağlantı hedefi; tıklanınca [`NotificationEvent::Link`].
    pub link: Option<String>,
}

impl BodySpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), ..Self::default() }
    }
}

/// Başlığın en uzun hâli (bayt).
pub const MAX_NOTIFICATION_TITLE: usize = 256;
/// Gövdenin toplam en uzun hâli (bayt).
pub const MAX_NOTIFICATION_BODY: usize = 4096;
/// Eylem etiketinin en uzun hâli (bayt).
pub const MAX_ACTION_LABEL: usize = 64;
/// Uygulama adı ve zaman metninin en uzun hâli (bayt).
pub const MAX_NOTIFICATION_META: usize = 128;

/// Kartın içeriği (D-Bus'tan gelen bildirimden doldurulur). Metinler dış girdidir;
/// [`NotificationCard::new`] onları yukarıdaki sınırlara keser.
#[derive(Clone, PartialEq, Default)]
pub struct NotificationContent {
    pub app_name: String,
    /// Yazı ikonu; dosya/pixmap ikon için `None` bırakıp `app_icon_rect`'e çizin.
    pub app_icon: Option<Icon>,
    /// "şimdi", "2 dk" …
    pub time: String,
    pub title: String,
    pub body: Vec<BodySpan>,
    /// Bildirim görseli (sağda kare küçük resim; `image-data` / `image-path`). Doku GPU'ya
    /// ulaşana kadar yerinde nötr bir yuva çizilir.
    pub image: Option<Texture>,
    pub actions: Vec<String>,
}

// Değişmez 9: bildirim metni, bağlantılar ve eylem etiketleri Debug'da yalnızca uzunluk/sayı.
impl fmt::Debug for NotificationEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Activated => f.write_str("Activated"),
            Self::Action(i) => f.debug_tuple("Action").field(i).finish(),
            Self::Link(url) => f.debug_tuple("Link").field(&Len(url)).finish(),
            Self::Dismissed => f.write_str("Dismissed"),
        }
    }
}

impl fmt::Debug for BodySpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BodySpan")
            .field("text", &Len(&self.text))
            .field("bold", &self.bold)
            .field("italic", &self.italic)
            .field("underline", &self.underline)
            .field("link", &OptLen(self.link.as_deref()))
            .finish()
    }
}

impl fmt::Debug for NotificationContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NotificationContent")
            .field("app_name", &Len(&self.app_name))
            .field("app_icon", &self.app_icon.is_some())
            .field("time", &Len(&self.time))
            .field("title", &Len(&self.title))
            .field("body", &self.body)
            .field("image", &self.image.is_some())
            .field("actions", &Count(self.actions.len()))
            .finish()
    }
}

impl fmt::Debug for RichBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RichBody")
            .field("spans", &Count(self.spans.len()))
            .field("links", &Count(self.links.len()))
            .field("origin", &self.origin)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Swipe {
    Idle,
    /// Basıldı, henüz eşiği geçmedi.
    Pending { start: Point },
    Dragging { start_x: f32, last_x: f32, last_t: Instant, velocity: f32 },
}

#[derive(Debug, Clone)]
pub struct NotificationCard {
    content: NotificationContent,
    rect: Rect,
    app_icon: Option<IconView>,
    app_name: Label,
    time: Label,
    title: Label,
    body: RichBody,
    expanded: bool,
    actions: Vec<Button>,
    close: IconButton,
    hovered: bool,
    close_alpha: Animated<f32>,
    /// Yatay kayma (kaydırarak kapatma).
    offset: Animated<f32>,
    swipe: Swipe,
    dismissed: bool,
    damage: DamageState,
    /// Görsel/ikon dokusu yükleniyordu (hazır olduğu karede kart yeniden boyanır).
    textures_pending: bool,
    /// Son çizimdeki yatay kayma (imleç isabeti için).
    last_offset: f32,
    /// İç bildirim maskotu (metnin solunda; gizleme `Theme::appearance.mascot`).
    #[cfg(feature = "brand")]
    mascot: Option<Box<Mascot>>,
}

/// Gövdenin kapalı/açık satır sınırı.
const BODY_LINES: usize = 3;
const BODY_LINES_EXPANDED: usize = 12;

/// Gövde metni `ui` ailesinde (`Body`), `Document` (serif, uzun okuma) DEĞİL: bildirim kısa,
/// arayüzün sesiyle okunur (karar 2026-10-14; `Body` ile `Document` aynı tip ölçeği
/// kademesinde, kart yüksekliği yalnız aile metrikleri kadar değişir).
fn body_text_style(theme: &Theme, lines: usize) -> nuxut_render::TextStyle {
    text_style(theme, TextRole::Body).with_max_lines(lines)
}

impl NotificationCard {
    pub fn new(theme: &Theme, mut content: NotificationContent) -> Self {
        clip_content(&mut content);
        let caption = text_style(theme, TextRole::Caption).with_max_lines(1);
        let actions = content
            .actions
            .iter()
            .take(MAX_NOTIFICATION_ACTIONS)
            .map(|a| Button::new(theme, ButtonVariant::Tonal).with_label(theme, a.clone()))
            .collect();
        Self {
            app_icon: content.app_icon.clone().map(|i| IconView::with_role(theme, i, TextRole::IconSmall)),
            app_name: Label::new(content.app_name.clone(), caption.clone()),
            time: Label::new(content.time.clone(), caption),
            title: Label::new(content.title.clone(), text_style(theme, TextRole::Label).with_max_lines(2)),
            body: RichBody::new(theme, &content.body, body_text_style(theme, BODY_LINES)),
            expanded: false,
            actions,
            close: IconButton::new(theme, Icon::named(IconName::Close), IconButtonVariant::Standard),
            hovered: false,
            close_alpha: Animated::new(0.0, theme.motion.effects),
            offset: Animated::new(0.0, theme.motion.spatial),
            swipe: Swipe::Idle,
            dismissed: false,
            textures_pending: false,
            last_offset: 0.0,
            #[cfg(feature = "brand")]
            mascot: None,
            content,
            rect: Rect::ZERO,
            damage: DamageState::new(),
        }
    }

    /// Metnin soluna maskot koyar (yalnız Nuxut'un iç bildirimleri; ör. güncelleme bitti →
    /// [`Mood::Celebrate`]). Karo yok; kenar [`NOTIFICATION_MASCOT_SIZE`].
    #[cfg(feature = "brand")]
    pub fn with_mascot(mut self, theme: &Theme, mood: Mood) -> Self {
        self.set_mascot(theme, Some(mood));
        self
    }

    /// Maskotu değiştirir ya da kaldırır (`None`). Yuva eklenir/kalkarsa yükseklik değişir:
    /// çağıran [`measure`](Self::measure) + [`set_rect`](Self::set_rect) ile yeniden dizer.
    #[cfg(feature = "brand")]
    pub fn set_mascot(&mut self, theme: &Theme, mood: Option<Mood>) {
        match (mood, &mut self.mascot) {
            (Some(mood), Some(m)) if m.mood() == mood => return,
            (Some(mood), Some(m)) => m.set_mood(mood),
            (Some(mood), None) => self.mascot = Some(Box::new(Mascot::new(theme, mood, NOTIFICATION_MASCOT_SIZE))),
            (None, None) => return,
            (None, Some(_)) => self.mascot = None,
        }
        // Metin sütunu kayar (bağlantı isabeti çizimden önce de doğru olsun).
        self.body.origin.x = self.rect.x + self.text_inset(theme);
        self.damage.mark();
    }

    /// Kartın maskotu (yoksa `None`).
    #[cfg(feature = "brand")]
    pub fn mascot(&self) -> Option<Mood> {
        self.mascot.as_ref().map(|m| m.mood())
    }

    /// Maskot yuvası bu temada yer kaplıyor mu (maskot var ve `[appearance] mascot` açık).
    #[cfg(feature = "brand")]
    fn mascot_shown(&self, theme: &Theme) -> bool {
        self.mascot.is_some() && theme.appearance.mascot
    }

    #[cfg(not(feature = "brand"))]
    fn mascot_shown(&self, _theme: &Theme) -> bool {
        false
    }

    /// Maskot sütununun genişliği (kenar + metne aralık); yuva yoksa 0.
    fn mascot_column(&self, theme: &Theme) -> f32 {
        if self.mascot_shown(theme) { NOTIFICATION_MASCOT_SIZE + theme.space(3) } else { 0.0 }
    }

    /// Metin sütununun sol kenarı (kart sol kenarına göre).
    fn text_inset(&self, theme: &Theme) -> f32 {
        Self::pad(theme) + self.mascot_column(theme)
    }

    /// Maskotun dinlenme konumundaki yeri (metin sütununun solunda, başlık satırının altında);
    /// maskot yoksa ya da gizliyse `None`.
    fn mascot_rest(&self, theme: &Theme) -> Option<Rect> {
        if !self.mascot_shown(theme) {
            return None;
        }
        let pad = Self::pad(theme);
        let top = self.rect.y + pad + theme.space(6) + theme.space(1);
        Some(Rect::new(self.rect.x + pad, top, NOTIFICATION_MASCOT_SIZE, NOTIFICATION_MASCOT_SIZE))
    }

    pub fn content(&self) -> &NotificationContent {
        &self.content
    }

    /// Zaman metni (dakikalık güncelleme).
    pub fn set_time(&mut self, time: &str) {
        if self.time.set_text(time) {
            self.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        let caption = text_style(theme, TextRole::Caption).with_max_lines(1);
        self.app_name.set_style(caption.clone());
        self.time.set_style(caption);
        self.title.set_style(text_style(theme, TextRole::Label).with_max_lines(2));
        self.body.set_theme(theme, self.body_style(theme));
        if let Some(i) = &mut self.app_icon {
            i.set_theme(theme, now);
        }
        for a in &mut self.actions {
            a.set_theme(theme, now);
        }
        self.close.set_theme(theme, now);
        #[cfg(feature = "brand")]
        if let Some(m) = &mut self.mascot {
            // `[appearance] mascot` canlı: yuva açılır/kapanır, metin sütunu kayar. Yükseklik
            // değişebilir; çağıran tema değişiminde zaten yeniden ölçer.
            m.set_theme(theme, now);
        }
        self.body.origin.x = self.rect.x + self.text_inset(theme);
        self.close_alpha.set_motion(theme.motion.effects, now);
        self.offset.set_motion(theme.motion.spatial, now);
        self.damage.mark();
    }

    fn body_style(&self, theme: &Theme) -> nuxut_render::TextStyle {
        body_text_style(theme, if self.expanded { BODY_LINES_EXPANDED } else { BODY_LINES })
    }

    /// Gövde satır sınırı (kapalı 3, açık 12). Değişince yükseklik değişir; yığın yeniden dizilmeli.
    pub fn set_expanded(&mut self, theme: &Theme, expanded: bool) {
        if self.expanded != expanded {
            self.expanded = expanded;
            self.body.set_style(self.body_style(theme));
            self.damage.mark();
        }
    }

    fn pad(theme: &Theme) -> f32 {
        theme.space(4)
    }

    fn image_side(theme: &Theme) -> f32 {
        theme.space(14)
    }

    /// Metin sütununun genişliği.
    fn text_width(&self, theme: &Theme, width: f32) -> f32 {
        let img = if self.content.image.is_some() { Self::image_side(theme) + theme.space(3) } else { 0.0 };
        (width - Self::pad(theme) * 2.0 - img - self.mascot_column(theme)).max(0.0)
    }

    /// Verilen genişlikte kartın yüksekliği (metin ölçümü dahil).
    pub fn measure(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> f32 {
        let pad = Self::pad(theme);
        let tw = self.text_width(theme, width);
        self.title.set_max_width(Some(tw));
        self.body.set_max_width(Some(tw));
        let header = theme.space(6);
        let mut h = pad + header + theme.space(1);
        let mut text_h = 0.0;
        if !self.content.title.is_empty() {
            text_h += self.title.measure(text).height;
        }
        if !self.content.body.is_empty() {
            text_h += theme.space(1) + self.body.measure(text).height;
        }
        if self.content.image.is_some() {
            text_h = text_h.max(Self::image_side(theme));
        }
        if self.mascot_shown(theme) {
            text_h = text_h.max(NOTIFICATION_MASCOT_SIZE);
        }
        h += text_h;
        if !self.actions.is_empty() {
            h += theme.space(3) + theme.space(10);
            for a in &mut self.actions {
                let _ = a.preferred_size(text, theme);
            }
        }
        (h + pad).ceil()
    }

    /// Yerleşim (dinlenme konumu; kaydırma kayması çizimde eklenir).
    pub fn set_rect(&mut self, text: &mut TextSystem, theme: &Theme, rect: Rect) {
        if self.rect == rect {
            return;
        }
        let moved = self.rect.size() == rect.size();
        self.rect = rect;
        self.damage.mark();
        if !moved {
            self.measure(text, theme, rect.width);
        }
        self.layout_children(text, theme);
    }

    fn layout_children(&mut self, text: &mut TextSystem, theme: &Theme) {
        let r = self.rect;
        let pad = Self::pad(theme);
        // Gövde kökeni (bağlantı isabeti çizimden önce de doğru olsun).
        let mut by = r.y + pad + theme.space(6) + theme.space(1);
        if !self.content.title.is_empty() {
            by += self.title.measure(text).height + theme.space(1);
        }
        self.body.origin = Point::new(r.x + self.text_inset(theme), by);
        let cs = theme.space(8);
        self.close.set_rect(Rect::new(r.right() - pad - cs + theme.space(2), r.y + pad - theme.space(1), cs, cs));
        if !self.actions.is_empty() {
            let h = theme.space(10);
            let y = r.bottom() - pad - h;
            let gap = theme.space(2);
            // Eylemler kalan genişliği eşit paylaşır.
            let n = self.actions.len() as f32;
            let w = ((r.width - pad * 2.0 - gap * (n - 1.0)) / n).max(0.0);
            for (i, a) in self.actions.iter_mut().enumerate() {
                let _ = a.preferred_size(text, theme);
                a.set_rect(Rect::new(r.x + pad + (w + gap) * i as f32, y, w, h));
            }
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Uygulama ikonunun bu karedeki yeri (kaydırma dahil; dosya/pixmap ikonu çağıran çizer).
    pub fn app_icon_rect(&self, theme: &Theme, now: Instant) -> Rect {
        let pad = Self::pad(theme);
        let s = text_style(theme, TextRole::IconSmall).size;
        let x = self.rect.x + pad + self.offset.value(now);
        Rect::new(x, self.rect.y + pad + (theme.space(6) - s) * 0.5, s, s)
    }

    /// Görselin bu karedeki yeri (sağda kare); görsel yoksa `None`.
    pub fn image_rect(&self, theme: &Theme, now: Instant) -> Option<Rect> {
        self.image_rest(theme).map(|r| r.translate(Point::new(self.offset.value(now), 0.0)))
    }

    /// Maskotun bu karedeki yeri (kaydırma dahil); maskot yoksa ya da gizliyse `None`.
    pub fn mascot_rect(&self, theme: &Theme, now: Instant) -> Option<Rect> {
        self.mascot_rest(theme).map(|r| r.translate(Point::new(self.offset.value(now), 0.0)))
    }

    fn image_rest(&self, theme: &Theme) -> Option<Rect> {
        self.content.image.as_ref()?;
        let pad = Self::pad(theme);
        let side = Self::image_side(theme);
        let top = self.rect.y + pad + theme.space(6) + theme.space(1);
        Some(Rect::new(self.rect.right() - pad - side, top, side, side))
    }

    /// Kaydırma animasyonu bitti ve kart kapandı: listeden kaldırılabilir.
    pub fn is_gone(&mut self, now: Instant) -> bool {
        self.dismissed && self.offset.is_settled(now)
    }

    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    /// Programatik kapatma (zaman aşımı, kapatma sinyali): sağa uçar.
    pub fn dismiss(&mut self, now: Instant) {
        if !self.dismissed {
            self.dismissed = true;
            self.offset.set_target(self.rect.width + Self::pad_out(self.rect), now);
            self.damage.mark();
        }
    }

    fn pad_out(rect: Rect) -> f32 {
        rect.width * 0.1
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<NotificationEvent> {
        if self.dismissed {
            return Response::none();
        }
        let shifted = event.translated(Point::new(self.offset.value(now), 0.0));
        let inside = self.rect.contains(shifted.position) && event.kind != PointerKind::Leave;
        let mut out = Response::none();
        if inside != self.hovered {
            self.hovered = inside;
            self.close_alpha.set_target(if inside { 1.0 } else { 0.0 }, now);
            self.damage.mark();
            out.repaint = true;
        }

        // Kaydırma hareketi çocuklardan önce: sürüklerken düğmeler basılı kalmaz.
        let slop = theme.space(2);
        match (self.swipe, event.kind) {
            (Swipe::Idle, PointerKind::Press(MouseButton::Left)) if inside => {
                self.swipe = Swipe::Pending { start: event.position };
            }
            (Swipe::Pending { start }, PointerKind::Motion) => {
                let dx = event.position.x - start.x;
                if dx.abs() > slop && dx.abs() > (event.position.y - start.y).abs() {
                    let base = self.offset.value(now);
                    self.offset.jump_to(base);
                    self.swipe = Swipe::Dragging { start_x: start.x - base, last_x: event.position.x, last_t: now, velocity: 0.0 };
                    // Çocukların basılı durumunu bırak.
                    let leave = event.with_kind(event.position, PointerKind::Leave);
                    let _ = self.close.pointer(&leave, now);
                    for a in &mut self.actions {
                        let _ = a.pointer(&leave, now);
                    }
                }
            }
            (Swipe::Dragging { start_x, last_x, last_t, velocity }, PointerKind::Motion) => {
                let dt = now.saturating_duration_since(last_t).as_secs_f32().max(1.0 / 240.0);
                let v = velocity * 0.4 + (event.position.x - last_x) / dt * 0.6;
                // Sola doğru dirençli (yalnızca sağa uçar; sol taraf lastik).
                let dx = event.position.x - start_x;
                let dx = if dx < 0.0 { -(-dx).sqrt() * theme.space(1) } else { dx };
                self.offset.jump_to(dx);
                self.swipe = Swipe::Dragging { start_x, last_x: event.position.x, last_t: now, velocity: v };
                self.damage.mark();
                return Response::repaint();
            }
            (Swipe::Dragging { velocity, .. }, PointerKind::Release(MouseButton::Left) | PointerKind::Leave) => {
                self.swipe = Swipe::Idle;
                let x = self.offset.value(now);
                let fling = velocity > theme.space(200);
                if x > self.rect.width * 0.35 || fling {
                    self.dismissed = true;
                    self.offset.set_target(self.rect.width + Self::pad_out(self.rect), now);
                    self.offset.kick(velocity.max(0.0), now);
                    self.damage.mark();
                    return Response::event(NotificationEvent::Dismissed);
                }
                self.offset.set_target(0.0, now);
                self.offset.kick(velocity, now);
                self.damage.mark();
                return Response::repaint();
            }
            (Swipe::Pending { .. }, PointerKind::Release(_) | PointerKind::Leave) => self.swipe = Swipe::Idle,
            _ => {}
        }
        if let Swipe::Dragging { .. } = self.swipe {
            return out;
        }

        let r = self.close.pointer(&shifted, now);
        out.repaint |= r.repaint;
        if r.event.is_some() {
            self.dismiss(now);
            return Response::event(NotificationEvent::Dismissed);
        }
        for (i, a) in self.actions.iter_mut().enumerate() {
            let r = a.pointer(&shifted, now);
            out.repaint |= r.repaint;
            if r.event.is_some() {
                return Response::event(NotificationEvent::Action(i));
            }
        }
        let on_child = self.close.rect().contains(shifted.position) || self.actions.iter().any(|a| a.rect().contains(shifted.position));
        if let PointerKind::Release(MouseButton::Left) = event.kind
            && inside
            && !on_child
            && self.swipe == Swipe::Idle
        {
            if let Some(url) = self.body.link_at(shifted.position) {
                return Response::event(NotificationEvent::Link(url.to_owned()));
            }
            return Response::event(NotificationEvent::Activated);
        }
        out
    }

    /// İmleç: kaydırırken `Grabbing`; gövde, bağlantı, eylemler ve kapatma `Pointer`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if let Swipe::Dragging { .. } = self.swipe {
            return Some(CursorShape::Grabbing);
        }
        if self.dismissed {
            return None;
        }
        let q = Point::new(p.x - self.last_offset, p.y);
        if !self.rect.contains(q) {
            return None;
        }
        let child = self.close.cursor(q).or_else(|| self.actions.iter().find_map(|a| a.cursor(q)));
        Some(child.unwrap_or(CursorShape::Pointer))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let dx = self.offset.value(now);
        self.last_offset = dx;
        let rest = self.rect;
        let r = rest.translate(Point::new(dx, 0.0));
        // Uzaklaştıkça solar.
        let fade = (1.0 - dx.abs() / (rest.width * 0.9).max(1.0)).clamp(0.0, 1.0);
        let radii = theme.shape.large();
        let pad = Self::pad(theme);
        let mut image_pending = false;
        #[cfg_attr(not(feature = "brand"), allow(unused_mut))]
        let mut mascot_pending = false;
        ui.with_alpha(fade, |ui| {
            ui.painter.rect(r, &RectStyle::filled(ui.fade(c.surface_container), radii));
            // Başlık satırı: ikon · uygulama · zaman ............ ×
            let header_h = theme.space(6);
            let hy = r.y + pad;
            let mut x = r.x + pad;
            let icon_s = text_style(theme, TextRole::IconSmall).size;
            if let Some(icon) = &mut self.app_icon {
                icon.draw_centered(ui.painter, Point::new(x + icon_s * 0.5, hy + header_h * 0.5), ui.fade(c.primary));
                x += icon_s + theme.space(2);
            } else if self.content.app_icon.is_none() {
                // Görsel ikon çağıranın; yer yine ayrılır.
                x += icon_s + theme.space(2);
            }
            let an = self.app_name.measure_in(ui.painter);
            self.app_name.draw(ui.painter, Point::new(x, hy + (header_h - an.height) * 0.5), ui.fade(c.on_surface_variant));
            x += an.width;
            if !self.content.time.is_empty() {
                let tm = self.time.measure_in(ui.painter);
                let sep = theme.space(2);
                let dot = Rect::new(x + sep - 1.5, hy + header_h * 0.5 - 1.5, 3.0, 3.0);
                ui.painter.rect(dot, &RectStyle::filled(ui.fade(c.on_surface_variant), nuxut_core::Corners::uniform(1.5)));
                self.time.draw(ui.painter, Point::new(x + sep * 2.0, hy + (header_h - tm.height) * 0.5), ui.fade(c.on_surface_variant));
            }
            let ca = self.close_alpha.value(now).clamp(0.0, 1.0);
            if ca > 0.0 {
                let cr = self.close.rect();
                self.close.set_rect(cr.translate(Point::new(dx, 0.0)));
                ui.with_alpha(ca, |ui| self.close.draw(ui));
                self.close.set_rect(cr);
            }

            // Metin sütunu.
            let tw = self.text_width(theme, rest.width);
            let inset = self.text_inset(theme);
            let mut y = hy + header_h + theme.space(1);
            if !self.content.title.is_empty() {
                self.title.set_max_width(Some(tw));
                let ts = self.title.measure_in(ui.painter);
                self.title.draw(ui.painter, Point::new(r.x + inset, y), ui.fade(c.on_surface));
                y += ts.height + theme.space(1);
            }
            if !self.content.body.is_empty() {
                self.body.set_max_width(Some(tw));
                let rest = Point::new(rest.x + inset, y);
                self.body.draw(ui.painter, Point::new(r.x + inset, y), rest, ui.fade(c.on_surface_variant));
            }
            // Maskot: karosuz, kartın solmasıyla (`ui.alpha`) birlikte solar.
            #[cfg(feature = "brand")]
            let at = self.mascot_rest(theme).map_or(Rect::ZERO, |mr| mr.translate(Point::new(dx, 0.0)));
            #[cfg(feature = "brand")]
            if let Some(m) = &mut self.mascot {
                m.set_rect(at);
                m.draw(ui);
                mascot_pending = m.is_pending();
            }
            if let Some(img) = self.image_rest(theme) {
                let img = img.translate(Point::new(dx, 0.0));
                // Nötr yuva her zaman (doku yüklenene kadar ve şeffaf görsellerin arkasında).
                ui.painter.rect(img, &RectStyle::filled(ui.fade(c.surface_bright), theme.shape.small()));
                if let Some(tex) = &self.content.image {
                    let style = ImageStyle::new(ImageFit::Cover).with_radii(theme.shape.small()).with_opacity(ui.alpha());
                    ui.painter.image(img, tex, &style);
                    image_pending = !tex.is_ready();
                }
            }
            for a in &mut self.actions {
                let ar = a.rect();
                a.set_rect(ar.translate(Point::new(dx, 0.0)));
                a.draw(ui);
                a.set_rect(ar);
            }
        });
        let pending = image_pending | mascot_pending | self.app_icon.as_ref().is_some_and(IconView::is_pending);
        if self.textures_pending && !pending {
            self.damage.mark();
        }
        self.textures_pending = pending;
        let animating = !self.offset.is_settled(now)
            | !self.close_alpha.is_settled(now)
            | matches!(self.swipe, Swipe::Dragging { .. })
            | pending;
        if animating {
            ui.request_frame();
        }
        let extent = rest.union(&r).expand(theme.space(1));
        self.damage.report(ui, extent, animating);
    }

    /// Ölçüm için hazır boyut (yığın yerleşimi).
    pub fn size(&self) -> Size {
        self.rect.size()
    }
}

/// Bildirim yığınındaki bir kartın bu karedeki yeri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StackSlot {
    /// Yığın kökenine göre dikey konum.
    pub y: f32,
    /// Giriş solması 0..1.
    pub alpha: f32,
}

#[derive(Debug, Clone)]
struct StackEntry<K> {
    key: K,
    y: Animated<f32>,
    alpha: Animated<f32>,
}

/// Kartları üstten alta (ya da alttan üste) dizen yerleşim yardımcısı. Kart eklenince yeni
/// kart yukarıdan süzülerek girer, diğerleri yayla aşağı kayar; kart kalkınca boşluk yayla
/// kapanır. Kartları çağıran tutar; yığın yalnızca konumları canlandırır.
#[derive(Debug, Clone)]
pub struct NotificationStack<K: Clone + PartialEq> {
    entries: Vec<StackEntry<K>>,
    spring: nuxut_motion::Motion,
    fade: nuxut_motion::Motion,
    /// `true`: ilk kart altta, yığın yukarı büyür.
    pub bottom_up: bool,
}

impl<K: Clone + PartialEq> NotificationStack<K> {
    pub fn new(theme: &Theme) -> Self {
        Self { entries: Vec::new(), spring: theme.motion.spatial, fade: theme.motion.effects, bottom_up: false }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.spring = theme.motion.spatial;
        self.fade = theme.motion.effects;
        for e in &mut self.entries {
            e.y.set_motion(self.spring, now);
            e.alpha.set_motion(self.fade, now);
        }
    }

    /// Sıralı `(anahtar, yükseklik)` listesiyle hedef konumları günceller. `gap` kartlar arası.
    /// Toplam yüksekliği döner (hedef; input bölgesi için).
    pub fn layout(&mut self, items: &[(K, f32)], gap: f32, now: Instant) -> f32 {
        self.entries.retain(|e| items.iter().any(|(k, _)| *k == e.key));
        let mut y = 0.0;
        let mut ordered = Vec::with_capacity(items.len());
        for (k, h) in items {
            let target = if self.bottom_up { -(y + h) } else { y };
            let entry = match self.entries.iter().position(|e| e.key == *k) {
                Some(i) => {
                    let mut e = self.entries.swap_remove(i);
                    e.y.set_target(target, now);
                    e
                }
                None => {
                    // Yeni kart: kendi yüksekliği kadar dışarıdan, saydam başlar.
                    let start = if self.bottom_up { target + h * 0.5 } else { target - h * 0.5 };
                    let mut ya = Animated::new(start, self.spring);
                    ya.set_target(target, now);
                    let mut a = Animated::new(0.0, self.fade);
                    a.set_target(1.0, now);
                    StackEntry { key: k.clone(), y: ya, alpha: a }
                }
            };
            ordered.push(entry);
            y += h + gap;
        }
        self.entries = ordered;
        (y - gap).max(0.0)
    }

    pub fn slot(&self, key: &K, now: Instant) -> Option<StackSlot> {
        self.entries
            .iter()
            .find(|e| e.key == *key)
            .map(|e| StackSlot { y: e.y.value(now), alpha: e.alpha.value(now).clamp(0.0, 1.0) })
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        self.entries.iter_mut().fold(false, |acc, e| acc | !e.y.is_settled(now) | !e.alpha.is_settled(now))
    }
}

/// Dış girdi metinlerini sınırlar.
fn clip_content(c: &mut NotificationContent) {
    let clip = |s: &mut String, max: usize| {
        let n = clip_text(s, max).len();
        s.truncate(n);
    };
    clip(&mut c.app_name, MAX_NOTIFICATION_META);
    clip(&mut c.time, MAX_NOTIFICATION_META);
    clip(&mut c.title, MAX_NOTIFICATION_TITLE);
    for a in &mut c.actions {
        clip(a, MAX_ACTION_LABEL);
    }
    let mut budget = MAX_NOTIFICATION_BODY;
    c.body.retain_mut(|span| {
        if budget == 0 {
            return false;
        }
        clip(&mut span.text, budget);
        budget -= span.text.len();
        if let Some(l) = &mut span.link {
            clip(l, MAX_NOTIFICATION_BODY);
        }
        !span.text.is_empty()
    });
}

/// Zengin gövde: parçalar + bağlantı hedefleri + önbellekli layout.
#[derive(Clone)]
struct RichBody {
    spans: Vec<(String, SpanStyle)>,
    links: Vec<String>,
    style: TextStyle,
    max_width: Option<f32>,
    layout: Option<TextLayout>,
    /// Son çizimin kökeni (dinlenme konumu; bağlantı isabeti için).
    origin: Point,
}

impl RichBody {
    fn new(theme: &Theme, spans: &[BodySpan], style: TextStyle) -> Self {
        let mut links = Vec::new();
        let spans = spans
            .iter()
            .map(|s| {
                let link = s.link.as_ref().map(|l| {
                    links.push(l.clone());
                    (links.len() - 1) as u32
                });
                let st = SpanStyle {
                    bold: s.bold,
                    italic: s.italic,
                    underline: s.underline || link.is_some(),
                    strikethrough: false,
                    color: link.map(|_| theme.colors.primary),
                    link,
                };
                (s.text.clone(), st)
            })
            .collect();
        Self { spans, links, style, max_width: None, layout: None, origin: Point::ZERO }
    }

    fn set_theme(&mut self, theme: &Theme, style: TextStyle) {
        for (_, st) in &mut self.spans {
            if st.link.is_some() {
                st.color = Some(theme.colors.primary);
            }
        }
        self.style = style;
        self.layout = None;
    }

    fn set_style(&mut self, style: TextStyle) {
        if self.style != style {
            self.style = style;
            self.layout = None;
        }
    }

    fn set_max_width(&mut self, w: Option<f32>) {
        if self.max_width != w {
            self.max_width = w;
            self.layout = None;
        }
    }

    fn layout(&mut self, text: &mut TextSystem) -> &TextLayout {
        if self.layout.is_none() {
            let spans: Vec<(&str, SpanStyle)> = self.spans.iter().map(|(t, s)| (t.as_str(), *s)).collect();
            self.layout = Some(text.layout_rich(&spans, &self.style, self.max_width));
        }
        self.layout.as_ref().expect("layout")
    }

    fn measure(&mut self, text: &mut TextSystem) -> Size {
        self.layout(text).size()
    }

    fn draw(&mut self, painter: &mut nuxut_render::Painter, origin: Point, rest_origin: Point, color: Color) {
        self.origin = rest_origin;
        self.layout(painter.text_system());
        if let Some(l) = &self.layout {
            painter.text(l, origin, color);
        }
    }

    /// Dinlenme koordinatındaki nokta → bağlantı hedefi.
    fn link_at(&self, p: Point) -> Option<&str> {
        let layout = self.layout.as_ref()?;
        let id = layout.link_at(p - self.origin)?;
        self.links.get(id as usize).map(String::as_str)
    }
}

// Erişilebilirlik (ADR-0011).
impl NotificationCard {
    /// `Role::Group`, tıklanabilir (varsayılan eylem): ad başlık (boşsa uygulama adı), açıklama
    /// uygulama adı, zaman ve gövdenin düz metni. Çocuklar: eylem düğmeleri (etiketleriyle) ve
    /// kapatma düğmesi (`close_label`, çağıranın dilinde). Maskot süstür, ağaçta yok. Açılır
    /// kartta çağıran `.live(Live::Polite)` ekler (Orca odaksız kartı duyursun).
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash, close_label: &str) -> AccessNode<'t> {
        let c = &self.content;
        let label = if c.title.is_empty() { c.app_name.as_str() } else { c.title.as_str() };
        let body: String = c.body.iter().map(|s| s.text.as_str()).collect();
        let desc: Vec<&str> =
            [c.app_name.as_str(), c.time.as_str(), body.as_str()].into_iter().filter(|s| !s.is_empty() && *s != label).collect();
        let id = tree
            .push(key, Role::Group)
            .label(label)
            .description(&desc.join(", "))
            .bounds(self.rect)
            .clickable()
            .id();
        tree.within(id, |tree| {
            for (i, a) in self.actions.iter().enumerate() {
                a.access(tree, ("action", i));
            }
            self.close.access(tree, "close").label(close_label);
        });
        tree.get(id).expect("az önce eklendi")
    }

    /// AT isteğini kart olayına çevirir: kartta `Click` → `Activated`, eylem düğmesinde →
    /// `Action(i)`, kapatmada → `Dismissed` (kart uçar, tıklamayla aynı). `card`: `access`'in
    /// döndürdüğü kimlik.
    pub fn access_action(&mut self, card: NodeId, request: AccessRequest, now: Instant) -> Option<NotificationEvent> {
        if request.action != AccessAction::Click || self.dismissed {
            return None;
        }
        if request.node == card {
            return Some(NotificationEvent::Activated);
        }
        if request.node == child_id(card, "close") {
            self.dismiss(now);
            return Some(NotificationEvent::Dismissed);
        }
        (0..self.actions.len()).find(|&i| request.node == child_id(card, ("action", i))).map(NotificationEvent::Action)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    fn card() -> (NotificationCard, TextSystem, Theme, Instant) {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut c = NotificationCard::new(
            &theme,
            NotificationContent {
                app_name: "Mesajlar".into(),
                app_icon: Some(Icon::named(IconName::Chat)),
                time: "şimdi".into(),
                title: "Ayşe".into(),
                body: vec![BodySpan::plain("Akşam yemeğine geliyor musun? ".repeat(20))],
                image: None,
                actions: vec!["Yanıtla".into(), "Okundu".into()],
            },
        );
        let h = c.measure(&mut text, &theme, 360.0);
        c.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 360.0, h));
        (c, text, theme, t0)
    }

    #[test]
    fn debug_is_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let mut text = TextSystem::new();
        let theme = Theme::default();
        let link = BodySpan { link: Some(format!("https://{SECRET}")), ..BodySpan::plain(SECRET) };
        let content = NotificationContent {
            app_name: SECRET.into(),
            time: SECRET.into(),
            title: SECRET.into(),
            body: vec![BodySpan::plain(SECRET), link],
            actions: vec![SECRET.into()],
            ..NotificationContent::default()
        };
        assert_redacted(&content);
        let mut c = NotificationCard::new(&theme, content);
        let h = c.measure(&mut text, &theme, 360.0);
        c.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 360.0, h));
        assert_redacted(&c);
        assert_redacted(&NotificationEvent::Link(SECRET.into()));
    }

    #[test]
    fn body_uses_ui_family_not_document() {
        // Belge ailesi ayırt edilebilir olsun (varsayılanda da farklı ama sabit kalmasın).
        let (theme, _) = Theme::parse("[typography.document]\nfamily = \"Belge Serif\"\n[typography.ui]\nfamily = \"Arayuz Sans\"\n").unwrap();
        let ui = text_style(&theme, TextRole::Body);
        let check = |c: &NotificationCard, theme: &Theme| {
            let s = &c.body.style;
            let ui = text_style(theme, TextRole::Body);
            assert_eq!((s.family.as_str(), s.fallback, s.size, s.weight), (ui.family.as_str(), ui.fallback, ui.size, ui.weight));
            assert_ne!(s.family, text_style(theme, TextRole::Document).family, "gövde Document rolünde değil");
        };
        let mut c = NotificationCard::new(&theme, NotificationContent { body: vec![BodySpan::plain("Merhaba")], ..NotificationContent::default() });
        check(&c, &theme);
        assert_eq!(c.body.style.family, "Arayuz Sans");
        c.set_expanded(&theme, true);
        check(&c, &theme);
        // Tema değişimi de ui ailesini korur.
        let (other, _) = Theme::parse("[typography.document]\nfamily = \"Belge 2\"\n[typography.ui]\nfamily = \"Arayuz 2\"\n").unwrap();
        c.set_theme(&other, Instant::now());
        check(&c, &other);
        assert_eq!(c.body.style.family, "Arayuz 2");
        assert_ne!(ui.family, c.body.style.family);
    }

    #[test]
    fn body_is_clamped_and_expands() {
        let (mut c, mut text, theme, _) = card();
        let collapsed = c.measure(&mut text, &theme, 360.0);
        c.set_expanded(&theme, true);
        let expanded = c.measure(&mut text, &theme, 360.0);
        assert!(expanded > collapsed);
    }

    #[test]
    fn swipe_past_threshold_dismisses() {
        let (mut c, _text, theme, t0) = card();
        let y = c.rect().y + 30.0;
        let _ = c.pointer(&theme, &press(50.0, y), t0);
        let mut t = t0;
        for i in 1..=10 {
            t += Duration::from_millis(16);
            let _ = c.pointer(&theme, &motion(50.0 + i as f32 * 20.0, y), t);
        }
        let r = c.pointer(&theme, &release(250.0, y), t);
        assert_eq!(r.event, Some(NotificationEvent::Dismissed));
        assert!(c.is_gone(t + Duration::from_secs(4)));
    }

    #[test]
    fn short_swipe_springs_back() {
        let (mut c, _text, theme, t0) = card();
        let y = c.rect().y + 30.0;
        let _ = c.pointer(&theme, &press(50.0, y), t0);
        let t = t0 + Duration::from_millis(500);
        let _ = c.pointer(&theme, &motion(80.0, y), t);
        let r = c.pointer(&theme, &release(80.0, y), t + Duration::from_millis(500));
        assert!(r.event.is_none());
        assert!(!c.is_dismissed());
        assert_eq!(c.offset.target(), 0.0);
    }

    #[test]
    fn click_activates_and_actions_fire() {
        let (mut c, _text, theme, t0) = card();
        let y = c.rect().y + 50.0;
        let _ = c.pointer(&theme, &press(100.0, y), t0);
        assert_eq!(c.pointer(&theme, &release(100.0, y), t0).event, Some(NotificationEvent::Activated));
        let a = c.actions[1].rect().center();
        let _ = c.pointer(&theme, &motion(a.x, a.y), t0);
        let _ = c.pointer(&theme, &press(a.x, a.y), t0);
        assert_eq!(c.pointer(&theme, &release(a.x, a.y), t0).event, Some(NotificationEvent::Action(1)));
    }

    #[test]
    fn stack_slides_on_insert_and_remove() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s: NotificationStack<u32> = NotificationStack::new(&theme);
        s.layout(&[(1, 100.0), (2, 80.0)], 8.0, t0);
        let t1 = t0 + Duration::from_secs(3);
        assert!(!s.is_animating(t1));
        assert_eq!(s.slot(&2, t1).unwrap().y, 108.0);
        // Üste yeni kart: diğerleri aşağı kayar (yayla).
        s.layout(&[(3, 60.0), (1, 100.0), (2, 80.0)], 8.0, t1);
        assert_eq!(s.slot(&1, t1).unwrap().y, 0.0, "konum sıçramaz");
        assert_eq!(s.slot(&3, t1).unwrap().alpha, 0.0);
        let t2 = t1 + Duration::from_secs(3);
        assert_eq!(s.slot(&1, t2).unwrap().y, 68.0);
        // Ortadaki kalkar: alttaki yukarı kayar.
        s.layout(&[(3, 60.0), (2, 80.0)], 8.0, t2);
        assert!(s.slot(&1, t2).is_none());
        assert_eq!(s.slot(&2, t2 + Duration::from_secs(3)).unwrap().y, 68.0);
    }
    #[test]
    fn link_click_and_limits() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut c = NotificationCard::new(
            &theme,
            NotificationContent {
                app_name: "x".repeat(1000),
                title: "Başlık".repeat(200),
                body: vec![
                    BodySpan { text: "https://nuxut.dev".into(), link: Some("https://nuxut.dev".into()), ..BodySpan::default() },
                    BodySpan::plain(" sonrası düz metin"),
                    BodySpan::plain("ç".repeat(5000)),
                ],
                actions: vec!["a".repeat(500)],
                ..NotificationContent::default()
            },
        );
        assert!(c.content().app_name.len() <= MAX_NOTIFICATION_META);
        assert!(c.content().title.len() <= MAX_NOTIFICATION_TITLE);
        assert!(c.content().body.iter().map(|s| s.text.len()).sum::<usize>() <= MAX_NOTIFICATION_BODY);
        assert!(c.content().actions[0].len() <= MAX_ACTION_LABEL);
        let h = c.measure(&mut text, &theme, 360.0);
        c.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 360.0, h));
        // Gövdenin ilk satırının başı bağlantı.
        let p = c.body.origin + Point::new(8.0, 6.0);
        let _ = c.pointer(&theme, &press(p.x, p.y), t0);
        let r = c.pointer(&theme, &release(p.x, p.y), t0);
        assert_eq!(r.event, Some(NotificationEvent::Link("https://nuxut.dev".into())));
    }

    #[test]
    fn fast_flick_dismisses_below_distance_threshold() {
        let (mut c, _text, theme, t0) = card();
        let y = c.rect().y + 30.0;
        let _ = c.pointer(&theme, &press(50.0, y), t0);
        let mut t = t0;
        for i in 1..=4 {
            t += Duration::from_millis(8);
            let _ = c.pointer(&theme, &motion(50.0 + i as f32 * 20.0, y), t);
        }
        assert!(c.offset.value(t) < c.rect().width * 0.35);
        assert_eq!(c.pointer(&theme, &release(130.0, y), t).event, Some(NotificationEvent::Dismissed));
    }

    // --- Görsel ve maskot yuvaları -----------------------------------------------------------

    use nuxut_render::{DrawKind, ImageData, Recorder, TextureOptions};

    /// Kısa iç bildirim (başlık + tek satır gövde; maskottan kısa).
    fn internal() -> NotificationContent {
        NotificationContent {
            app_name: "Nuxut".into(),
            app_icon: Some(Icon::named(IconName::Settings)),
            time: "şimdi".into(),
            title: "Nuxut güncellendi".into(),
            body: vec![BodySpan::plain("Sürüm 0.4 kuruldu.")],
            ..NotificationContent::default()
        }
    }

    fn placed(mut c: NotificationCard, text: &mut TextSystem, theme: &Theme) -> NotificationCard {
        let h = c.measure(text, theme, 360.0);
        c.set_rect(text, theme, Rect::new(0.0, 0.0, 360.0, h));
        c
    }

    /// Kartı bir kez çizer: (çizimler, görüntü opaklıkları).
    fn draw_card(c: &mut NotificationCard, theme: &Theme, text: &mut TextSystem, alpha: f32, now: Instant) -> (Vec<(DrawKind, Rect)>, Vec<f32>) {
        let mut rec = Recorder::new();
        rec.record(text, (800, 600), 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            ui.with_alpha(alpha, |ui| c.draw(ui));
        });
        (rec.draws().into_iter().map(|d| (d.kind, d.rect)).collect(), rec.image_opacities())
    }

    fn thumb() -> Texture {
        let mut rec = Recorder::new();
        let mut text = TextSystem::new();
        let mut tex = None;
        rec.record(&mut text, (8, 8), 1.0, |p| {
            let data = ImageData { width: 2, height: 2, pixels: std::sync::Arc::from(vec![255u8; 16]) };
            tex = Some(p.upload_texture(data, TextureOptions::default()));
        });
        tex.expect("doku")
    }

    #[test]
    fn image_slot_is_unchanged_without_mascot() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let content = NotificationContent { image: Some(thumb()), ..internal() };
        let mut c = placed(NotificationCard::new(&theme, content), &mut text, &theme);
        let pad = theme.space(4);
        let side = theme.space(14);
        // Sağda kare, başlık satırının altında; metin kart dolgusundan başlar.
        let want = Rect::new(360.0 - pad - side, pad + theme.space(6) + theme.space(1), side, side);
        assert_eq!(c.image_rect(&theme, t0), Some(want));
        assert_eq!(c.body.origin.x, pad);
        assert!(c.rect().height >= want.bottom() + pad);
        // Görsel yuvası: nötr karo + görüntü (iki çizim, ikisi de yuvada).
        let (draws, _) = draw_card(&mut c, &theme, &mut text, 1.0, t0);
        assert!(draws.contains(&(DrawKind::Image, want)));
        assert!(draws.contains(&(DrawKind::Quad, want)));
        assert_eq!(draws.iter().filter(|(k, _)| *k == DrawKind::Image).count(), 1);
    }

    #[cfg(feature = "brand")]
    #[test]
    fn mascot_sits_left_without_tile_and_image_slot_stays() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let pad = theme.space(4);
        let content = NotificationContent { image: Some(thumb()), ..internal() };
        let plain = placed(NotificationCard::new(&theme, content.clone()), &mut text, &theme);
        let mut c = placed(NotificationCard::new(&theme, content).with_mascot(&theme, Mood::Celebrate), &mut text, &theme);
        assert_eq!(c.mascot(), Some(Mood::Celebrate));
        // Görsel yuvası maskottan etkilenmez.
        assert_eq!(c.image_rect(&theme, t0), plain.image_rect(&theme, t0));
        let top = pad + theme.space(6) + theme.space(1);
        let m = Rect::new(pad, top, NOTIFICATION_MASCOT_SIZE, NOTIFICATION_MASCOT_SIZE);
        assert_eq!(c.mascot_rect(&theme, t0), Some(m));
        // Metin sütunu maskotun sağında; kart maskotu kapsar.
        assert_eq!(c.body.origin.x, pad + NOTIFICATION_MASCOT_SIZE + theme.space(3));
        assert_eq!(c.rect().height, (m.bottom() + pad).ceil());
        let (draws, _) = draw_card(&mut c, &theme, &mut text, 1.0, t0);
        assert!(draws.contains(&(DrawKind::Image, m)), "maskot yuvasında çizilir");
        // Karo yok: maskotla kesişen tek dörtgen kartın kendi zemini.
        let card = c.rect();
        let tiles = draws.iter().filter(|(k, r)| *k == DrawKind::Quad && *r != card && !r.intersect(&m).is_empty()).count();
        assert_eq!(tiles, 0, "maskotun arkasında karo olmamalı");
        // Kaldırma: yuva kapanır.
        c.set_mascot(&theme, None);
        assert_eq!(c.mascot(), None);
        assert_eq!(c.mascot_rect(&theme, t0), None);
        assert_eq!(c.measure(&mut text, &theme, 360.0), plain.rect().height);
    }

    #[cfg(feature = "brand")]
    #[test]
    fn hidden_mascot_leaves_no_slot_and_toggles_live() {
        let shown = Theme::default();
        let mut hidden = shown.clone();
        hidden.appearance.mascot = false;
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let pad = shown.space(4);
        let plain_h = placed(NotificationCard::new(&shown, internal()), &mut text, &shown).rect().height;
        let shown_h = placed(NotificationCard::new(&shown, internal()).with_mascot(&shown, Mood::Celebrate), &mut text, &shown).rect().height;
        assert!(shown_h > plain_h, "kısa metinde maskot kartı uzatır");

        // Gizliyken kurulan kart: yuva yok, metin sola yaslı, yükseklik maskotsuzla aynı.
        let mut c = placed(NotificationCard::new(&hidden, internal()).with_mascot(&hidden, Mood::Celebrate), &mut text, &hidden);
        assert_eq!(c.rect().height, plain_h);
        assert_eq!(c.mascot_rect(&hidden, t0), None);
        assert_eq!(c.body.origin.x, pad);
        let (draws, _) = draw_card(&mut c, &hidden, &mut text, 1.0, t0);
        assert!(!draws.iter().any(|(k, _)| *k == DrawKind::Image), "gizli maskot çizilmez");

        // Canlı açma: set_theme → yuva gelir, metin kayar, yeniden ölçüm maskotu kapsar.
        c.set_theme(&shown, t0);
        assert_eq!(c.body.origin.x, pad + NOTIFICATION_MASCOT_SIZE + shown.space(3));
        let h = c.measure(&mut text, &shown, 360.0);
        assert_eq!(h, shown_h);
        c.set_rect(&mut text, &shown, Rect::new(0.0, 0.0, 360.0, h));
        let (draws, _) = draw_card(&mut c, &shown, &mut text, 1.0, t0);
        assert_eq!(draws.iter().filter(|(k, _)| *k == DrawKind::Image).count(), 1);

        // Canlı kapatma: geri döner.
        c.set_theme(&hidden, t0);
        assert_eq!(c.body.origin.x, pad);
        assert_eq!(c.measure(&mut text, &hidden, 360.0), plain_h);
    }

    #[cfg(feature = "brand")]
    #[test]
    fn mascot_follows_swipe_and_fades_with_card() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut c = placed(NotificationCard::new(&theme, internal()).with_mascot(&theme, Mood::Celebrate), &mut text, &theme);
        let rest = c.mascot_rect(&theme, t0).expect("yuva");
        // Dinlenmede: yalnız dış (yığın) alfası.
        let (_, op) = draw_card(&mut c, &theme, &mut text, 0.5, t0);
        assert_eq!(op, [0.5]);
        // Kartın genişliğinin %45'i kadar kaymış: kart solması 1 − 0,45/0,9 = 0,5; dış alfa ile 0,25.
        let dx = 360.0 * 0.45;
        c.offset.jump_to(dx);
        let (draws, op) = draw_card(&mut c, &theme, &mut text, 0.5, t0);
        assert_eq!(op.len(), 1);
        assert!((op[0] - 0.25).abs() < 1e-4, "maskot kartla birlikte solmalı: {}", op[0]);
        let moved = rest.translate(Point::new(dx, 0.0));
        assert_eq!(c.mascot_rect(&theme, t0), Some(moved));
        assert!(draws.contains(&(DrawKind::Image, moved)), "maskot kaydırmayı izler");
    }
}
