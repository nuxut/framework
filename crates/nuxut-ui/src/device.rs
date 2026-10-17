//! Wi-Fi / Bluetooth cihaz satırı ([`DeviceRow`]): sinyal gücü (4 kademe) ya da pil göstergesi,
//! "bağlanıyor" durumunda Spinner, şifre isteyen ağda satır içi parola alanı (satır yüksekliği
//! yayla açılır), bağlı/kayıtlı/kullanılamaz durumları. Metinler (durum yazısı) çağıranındır
//! (yerelleştirme), bileşen yalnızca biçim ve davranışı verir.

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;
use zeroize::Zeroizing;

use crate::button::{Button, ButtonVariant, IconButton, IconButtonVariant};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::label::Label;
use crate::pressable::Pressable;
use crate::progress::Spinner;
use crate::style::{DISABLED_CONTENT, accent_on, clip_text, faded_text, tonal_container};
use crate::text_input::{TextInput, TextInputEvent};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    /// `strength` sinyal yüzdesi (0..=100, NetworkManager `Strength`); `secured` şifreli ağ.
    Wifi { strength: u8, secured: bool },
    /// `battery` yüzde (bilinmiyorsa `None`).
    Bluetooth { battery: Option<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceState {
    Available,
    /// Daha önce bağlanılmış (şifre kayıtlı).
    Saved,
    Connecting,
    Connected,
    Unavailable,
}

#[derive(PartialEq, Eq)]
pub enum DeviceEvent {
    Connect,
    Disconnect,
    Forget,
    /// Satır içi parola ile bağlan; metin düşerken sıfırlanır.
    Password(Zeroizing<String>),
}

/// Parola hiçbir zaman yazılmaz (yalnızca uzunluk).
impl std::fmt::Debug for DeviceEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect => f.write_str("Connect"),
            Self::Disconnect => f.write_str("Disconnect"),
            Self::Forget => f.write_str("Forget"),
            Self::Password(p) => f.debug_tuple("Password").field(&crate::redact::Len(p)).finish(),
        }
    }
}



/// Türün varsayılan ikonu: Wi-Fi'de sinyal kademesi, Bluetooth'ta genel Bluetooth ikonu.
fn default_icon(kind: DeviceKind) -> Icon {
    match kind {
        DeviceKind::Wifi { strength, .. } => Icon::named(IconName::wifi(strength)),
        DeviceKind::Bluetooth { .. } => Icon::named(IconName::Bluetooth),
    }
}

fn battery_icon(kind: DeviceKind) -> Icon {
    match kind {
        DeviceKind::Bluetooth { battery: Some(b) } => Icon::named(IconName::battery(b)),
        _ => Icon::named(IconName::BatteryUnknown),
    }
}

#[derive(Debug)]
pub struct DeviceRow {
    core: Pressable,
    kind: DeviceKind,
    state: DeviceState,
    icon: IconView,
    /// Çağıranın verdiği ikon (`None`: türün varsayılanı, bkz. [`DeviceRow::set_icon`]).
    custom_icon: Option<Icon>,
    lock: IconView,
    /// Bluetooth pil kademesi (yalnızca yüzde biliniyorsa çizilir).
    battery: IconView,
    name: Label,
    status: Label,
    spinner: Spinner,
    disconnect: IconButton,
    forget: IconButton,
    password: TextInput,
    join: Button,
    expanded: bool,
    /// 0 = kapalı, 1 = parola alanı açık (satır yüksekliği).
    open: Animated<f32>,
    rect: Rect,
}

impl DeviceRow {
    pub fn new(theme: &Theme, name: &str, kind: DeviceKind, state: DeviceState) -> Self {
        let icon = default_icon(kind);
        let mut core = Pressable::new(theme);
        if state == DeviceState::Unavailable {
            core.set_enabled(false, Instant::now());
        }
        Self {
            core,
            kind,
            state,
            icon: IconView::new(theme, icon).filled(state == DeviceState::Connected),
            custom_icon: None,
            lock: IconView::with_role(theme, Icon::named(IconName::Lock), TextRole::IconSmall),
            battery: IconView::with_role(theme, battery_icon(kind), TextRole::IconSmall),
            name: Label::new(clip_text(name, 128), text_style(theme, TextRole::Body).with_max_lines(1)),
            status: Label::new("", text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1)),
            spinner: Spinner::new(theme),
            disconnect: IconButton::new(theme, Icon::named(IconName::LinkOff), IconButtonVariant::Standard),
            forget: IconButton::new(theme, Icon::named(IconName::Delete), IconButtonVariant::Standard),
            password: TextInput::new(theme).password(true).with_placeholder("••••••••"),
            join: Button::new(theme, ButtonVariant::Filled).with_icon(theme, Icon::named(IconName::ArrowForward)),
            expanded: false,
            open: Animated::new(0.0, theme.motion.spatial),
            rect: Rect::ZERO,
        }
    }

    pub fn state(&self) -> DeviceState {
        self.state
    }

    pub fn kind(&self) -> DeviceKind {
        self.kind
    }

    /// Satır ikonunu türün varsayılanı yerine `icon` yapar (ör. Bluetooth cihaz türü:
    /// `IconName::Phone`, `Headphones`, `Watch`, `Gamepad`, `Tablet`, `Keyboard`, `Mouse`).
    pub fn with_icon(mut self, icon: Icon) -> Self {
        self.set_icon(Some(icon));
        self
    }

    /// Satır ikonunu değiştirir; `None` türün varsayılanına döner (Wi-Fi: sinyal kademesi,
    /// `set_kind` ile izlenir; Bluetooth: `IconName::Bluetooth`). Özel ikon verilmişken
    /// `set_kind` ikonu değiştirmez. Bağlıyken dolu çizim aynen sürer. Değiştiyse `true`.
    pub fn set_icon(&mut self, icon: Option<Icon>) -> bool {
        if self.custom_icon == icon {
            return false;
        }
        let shown = icon.clone().unwrap_or_else(|| default_icon(self.kind));
        self.custom_icon = icon;
        let changed = self.icon.set_icon(shown);
        if changed {
            self.core.damage.mark();
        }
        changed
    }

    /// Çizilen satır ikonu (özel ya da türün varsayılanı).
    pub fn icon(&self) -> &Icon {
        self.icon.icon()
    }

    /// Durum yazısı (çağıranın dilinde): "Bağlı", "Bağlanıyor…", "%80" …
    pub fn set_status_text(&mut self, text: &str) {
        if self.status.set_text(clip_text(text, 128)) {
            self.core.damage.mark();
        }
    }

    /// Placeholder (çağıranın dilinde, ör. "Parola").
    pub fn set_password_placeholder(&mut self, text: &str) {
        self.password.set_placeholder(text);
    }

    pub fn set_state(&mut self, state: DeviceState, now: Instant) {
        if self.state != state {
            self.state = state;
            self.icon.set_filled(state == DeviceState::Connected, now);
            if !self.secured_new() {
                self.collapse(now);
            }
            self.core.set_enabled(state != DeviceState::Unavailable, now);
            self.core.damage.mark();
        }
    }

    pub fn set_kind(&mut self, kind: DeviceKind, now: Instant) {
        if self.kind != kind {
            self.kind = kind;
            if !self.secured_new() {
                self.collapse(now);
            }
            self.battery.set_icon(battery_icon(kind));
            if self.custom_icon.is_none() {
                self.icon.set_icon(default_icon(kind));
            }
            self.core.damage.mark();
        }
    }

    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    /// Parola alanını kapatır ve tamponu sıfırlar. Çağıran, başka bir cihaz seçildiğinde,
    /// bağlantı hata verdiğinde ya da panel kapandığında çağırır (satır kendisi Esc, gönderim,
    /// durum değişimi ve şifresiz ağa dönüşte çağırır).
    pub fn cancel_password(&mut self, now: Instant) {
        self.collapse(now);
    }

    fn collapse(&mut self, now: Instant) {
        // Kapalı görünse de tampon koşulsuz silinir (ön düzenleme dahil).
        self.password.clear();
        self.password.set_focused(false, now);
        if self.expanded {
            self.expanded = false;
            self.open.set_target(0.0, now);
            self.core.damage.mark();
        }
    }

    fn expand(&mut self, now: Instant) {
        if !self.expanded {
            self.expanded = true;
            self.open.set_target(1.0, now);
            self.password.set_focused(true, now);
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        for i in [&mut self.icon, &mut self.lock, &mut self.battery] {
            i.set_theme(theme, now);
        }
        self.name.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
        self.status.set_style(text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1));
        self.spinner.set_theme(theme, now);
        self.disconnect.set_theme(theme, now);
        self.forget.set_theme(theme, now);
        self.password.set_theme(theme, now);
        self.join.set_theme(theme, now);
        self.open.set_motion(theme.motion.spatial, now);
    }

    fn base_h(theme: &Theme) -> f32 {
        theme.space(14)
    }

    fn extra_h(theme: &Theme) -> f32 {
        theme.space(15)
    }

    /// Bu karedeki yükseklik (parola alanı açılırken yayla büyür).
    pub fn height(&self, theme: &Theme, now: Instant) -> f32 {
        Self::base_h(theme) + Self::extra_h(theme) * self.open.value(now).max(0.0)
    }

    pub fn target_height(&self, theme: &Theme) -> f32 {
        Self::base_h(theme) + Self::extra_h(theme) * self.open.target()
    }

    /// Yerleşim (genişlik sabit, yükseklik `height(now)`; liste her karede yeniden dizebilir).
    pub fn set_rect(&mut self, text: &mut TextSystem, theme: &Theme, rect: Rect) {
        self.rect = rect;
        let head = Rect::new(rect.x, rect.y, rect.width, Self::base_h(theme));
        self.core.set_rect(head);
        let s = theme.space(10);
        let y = head.center().y - s * 0.5;
        self.forget.set_rect(Rect::new(head.right() - theme.space(2) - s, y, s, s));
        self.disconnect.set_rect(Rect::new(head.right() - theme.space(3) - s * 2.0, y, s, s));
        self.spinner.set_rect(Rect::new(head.right() - theme.space(3) - s, y, s, s));
        let fy = head.bottom() + theme.space(1);
        let jw = self.join.preferred_size(text, theme).width;
        let pad = theme.space(4);
        self.password.set_rect(Rect::new(rect.x + pad, fy, rect.width - pad * 2.0 - jw - theme.space(2), theme.space(12)));
        self.join.set_rect(Rect::new(rect.right() - pad - jw, fy + theme.space(1), jw, theme.space(10)));
        let tw = rect.width - theme.space(4) - text_style(theme, TextRole::Icon).size - theme.space(3) - theme.space(24);
        self.name.set_max_width(Some(tw.max(0.0)));
        self.status.set_max_width(Some(tw.max(0.0)));
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn secured_new(&self) -> bool {
        matches!(self.kind, DeviceKind::Wifi { secured: true, .. }) && self.state == DeviceState::Available
    }

    fn activate(&mut self, now: Instant) -> Response<DeviceEvent> {
        match self.state {
            DeviceState::Available if self.secured_new() => {
                if self.expanded { self.collapse(now) } else { self.expand(now) }
                Response::repaint()
            }
            DeviceState::Available | DeviceState::Saved => Response::event(DeviceEvent::Connect),
            _ => Response::none(),
        }
    }

    fn submit(&mut self, now: Instant) -> Response<DeviceEvent> {
        if self.password.is_empty() {
            return Response::none();
        }
        let secret = self.password.with_secret(|s| Zeroizing::new(s.to_owned()));
        self.collapse(now);
        Response::event(DeviceEvent::Password(secret))
    }

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<DeviceEvent> {
        let connected = self.state == DeviceState::Connected;
        if connected && self.disconnect.pointer(event, now).event.is_some() {
            return Response::event(DeviceEvent::Disconnect);
        }
        if matches!(self.state, DeviceState::Saved | DeviceState::Connected) && self.forget.pointer(event, now).event.is_some() {
            return Response::event(DeviceEvent::Forget);
        }
        if self.expanded {
            let _ = self.password.pointer(text, theme, event, now);
            if self.join.pointer(event, now).event.is_some() {
                return self.submit(now);
            }
        }
        let on_button = (connected && self.disconnect.rect().contains(event.position))
            || (matches!(self.state, DeviceState::Saved | DeviceState::Connected) && self.forget.rect().contains(event.position));
        if on_button {
            return Response::repaint();
        }
        let r = self.core.pointer(event, now);
        if r.event.is_some() {
            return self.activate(now);
        }
        r.discard()
    }

    /// Satır odaktayken Enter/Space etkinleştirir; parola açıkken tuşlar alana gider, Enter
    /// bağlanır, Esc kapatır.
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<DeviceEvent> {
        if self.expanded {
            if event.state != KeyState::Released && event.key == Key::Escape {
                self.collapse(now);
                return Response::repaint();
            }
            return match self.password.key(event, now).event {
                Some(TextInputEvent::Submit) => self.submit(now),
                _ => Response::repaint(),
            };
        }
        let r = self.core.key(event, now);
        if r.event.is_some() { self.activate(now) } else { r.discard() }
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.expanded
            && let Some(c) = self.password.cursor(p).or_else(|| self.join.cursor(p))
        {
            return Some(c);
        }
        let connected = self.state == DeviceState::Connected;
        if connected && let Some(c) = self.disconnect.cursor(p) {
            return Some(c);
        }
        if matches!(self.state, DeviceState::Saved | DeviceState::Connected)
            && let Some(c) = self.forget.cursor(p)
        {
            return Some(c);
        }
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let o = self.open.value(now).clamp(0.0, 1.0);
        let full = Rect::new(self.rect.x, self.rect.y, self.rect.width, self.height(theme, now));
        let radii = Corners::uniform(theme.shape.radius_medium);
        let connected = self.state == DeviceState::Connected;
        let row_bg = if connected { tonal_container(theme) } else { c.surface_bright };
        if connected || o > 0.0 {
            ui.painter.rect(full, &RectStyle::filled(ui.fade(row_bg), radii));
        }
        let head = self.core.rect;
        let row_fill = if connected || o > 0.0 { row_bg } else { nuxut_core::Color::TRANSPARENT };
        let accent = accent_on(theme, row_fill);
        self.core.interaction.draw_state_layer_under(ui, head, radii, c.on_surface, row_fill, &[c.on_surface, c.on_surface_variant, accent]);
        self.core.draw_focus(ui, head, radii);
        // Kullanılamayan cihaz: soluk ama okunur (yüksek karşıtlıkta ≥ 4.5; `style::faded_text`).
        let dim = |col: nuxut_core::Color, on: bool| if on { col } else { faded_text(theme, col, DISABLED_CONTENT, &[row_fill]) };
        let avail = self.state != DeviceState::Unavailable;
        let pad = theme.space(4);
        let iw = self.icon.size().width;
        let mid = head.center().y;
        let icol = if connected { accent } else { c.on_surface_variant };
        let mut animating = self.icon.animate(now) | !self.open.is_settled(now);
        self.icon.draw_centered(ui.painter, Point::new(head.x + pad + iw * 0.5, mid), ui.fade(dim(icol, avail)));
        let tx = head.x + pad + iw + theme.space(3);
        let ns = self.name.measure_in(ui.painter);
        let ss = self.status.measure_in(ui.painter);
        let top = mid - (ns.height + if ss.width > 0.0 { ss.height } else { 0.0 }) * 0.5;
        self.name.draw(ui.painter, Point::new(tx, top), ui.fade(dim(c.on_surface, avail)));
        if matches!(self.kind, DeviceKind::Wifi { secured: true, .. }) {
            let lw = self.lock.size().width;
            self.lock.draw_centered(ui.painter, Point::new(tx + ns.width + theme.space(1) + lw * 0.5, top + ns.height * 0.5), ui.fade(dim(c.on_surface_variant, avail)));
        }
        // Durum yazısı; Bluetooth'ta pil yüzdesi yanında pil ikonu.
        if ss.width > 0.0 {
            self.status.draw(ui.painter, Point::new(tx, top + ns.height), ui.fade(dim(c.on_surface_variant, avail)));
        }
        if let DeviceKind::Bluetooth { battery: Some(_) } = self.kind {
            let bi = &mut self.battery;
            let bx = tx + ss.width + theme.space(2) + bi.size().width * 0.5;
            bi.draw_centered(ui.painter, Point::new(bx, top + ns.height + ss.height * 0.5), ui.fade(c.on_surface_variant));
        }
        match self.state {
            DeviceState::Connecting => {
                self.spinner.draw(ui);
                animating = true;
            }
            DeviceState::Connected => {
                self.disconnect.draw(ui);
                self.forget.draw(ui);
            }
            DeviceState::Saved => self.forget.draw(ui),
            _ => {}
        }
        if o > 0.01 {
            ui.painter.push_clip(full);
            ui.with_alpha(o, |ui| {
                self.password.draw(ui);
                self.join.draw(ui);
            });
            ui.painter.pop_clip();
        }
        if animating {
            ui.request_frame();
            ui.damage(full.expand(theme.space(1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    fn row(kind: DeviceKind, state: DeviceState) -> (DeviceRow, Theme, TextSystem, Instant) {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut r = DeviceRow::new(&theme, "ornek-ag", kind, state);
        r.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 360.0, 56.0));
        (r, theme, text, t0)
    }

    fn click(r: &mut DeviceRow, text: &mut TextSystem, theme: &Theme, p: Point, t: Instant) -> Option<DeviceEvent> {
        let _ = r.pointer(text, theme, &motion(p.x, p.y), t);
        let _ = r.pointer(text, theme, &press(p.x, p.y), t);
        r.pointer(text, theme, &release(p.x, p.y), t).event
    }

    #[test]
    fn secured_network_expands_password_and_submits() {
        let (mut r, theme, mut text, t0) = row(DeviceKind::Wifi { strength: 70, secured: true }, DeviceState::Available);
        assert!(click(&mut r, &mut text, &theme, Point::new(40.0, 20.0), t0).is_none());
        assert!(r.is_expanded());
        let h0 = r.height(&theme, t0);
        assert!(r.height(&theme, t0 + Duration::from_secs(3)) > h0, "yükseklik yayla açılır");
        for ch in "parola1".chars() {
            let _ = r.key(&typed(ch), t0);
        }
        let ev = r.key(&key(Key::Enter), t0).event;
        assert_eq!(ev, Some(DeviceEvent::Password(Zeroizing::new("parola1".to_owned()))));
        assert!(!r.is_expanded() && r.password.is_empty(), "alan kapanır ve temizlenir");
    }

    #[test]
    fn open_and_saved_connect_connected_disconnects() {
        let (mut r, theme, mut text, t0) = row(DeviceKind::Wifi { strength: 20, secured: false }, DeviceState::Available);
        assert_eq!(click(&mut r, &mut text, &theme, Point::new(40.0, 20.0), t0), Some(DeviceEvent::Connect));
        r.set_state(DeviceState::Connected, t0);
        let d = r.disconnect.rect().center();
        assert_eq!(click(&mut r, &mut text, &theme, d, t0), Some(DeviceEvent::Disconnect));
        let f = r.forget.rect().center();
        assert_eq!(click(&mut r, &mut text, &theme, f, t0), Some(DeviceEvent::Forget));
        assert_eq!(r.cursor(f), Some(CursorShape::Pointer));
        r.set_state(DeviceState::Unavailable, t0);
        assert!(click(&mut r, &mut text, &theme, Point::new(40.0, 20.0), t0).is_none());
    }

    #[test]
    fn icons() {
        let (mut r, _, _, _) = row(DeviceKind::Wifi { strength: 90, secured: false }, DeviceState::Available);
        assert_eq!(r.icon.icon(), &Icon::named(IconName::Wifi4));
        r.set_kind(DeviceKind::Wifi { strength: 10, secured: false }, Instant::now());
        assert_eq!(r.icon.icon(), &Icon::named(IconName::Wifi1));
    }

    #[test]
    fn custom_icon_overrides_kind_default() {
        let t0 = Instant::now();
        let theme = Theme::default();
        // Varsayılan: Bluetooth ikonu (eski davranış).
        let (mut r, ..) = row(DeviceKind::Bluetooth { battery: Some(60) }, DeviceState::Connected);
        assert_eq!(r.icon(), &Icon::named(IconName::Bluetooth));
        // Kurucuda özel ikon.
        let r2 = DeviceRow::new(&theme, "saat", DeviceKind::Bluetooth { battery: None }, DeviceState::Available)
            .with_icon(Icon::named(IconName::Watch));
        assert_eq!(r2.icon(), &Icon::named(IconName::Watch));
        // Değiştir; aynı ikon değişiklik değildir.
        assert!(r.set_icon(Some(Icon::named(IconName::Phone))));
        assert!(!r.set_icon(Some(Icon::named(IconName::Phone))));
        assert_eq!(r.icon(), &Icon::named(IconName::Phone));
        // Tür değişimi (pil) özel ikonu bozmaz.
        r.set_kind(DeviceKind::Bluetooth { battery: Some(10) }, t0);
        assert_eq!(r.icon(), &Icon::named(IconName::Phone));
        // None: türün varsayılanına döner.
        assert!(r.set_icon(None));
        assert_eq!(r.icon(), &Icon::named(IconName::Bluetooth));
        // Wi-Fi'de özel ikon sinyal kademesini ezer; kaldırılınca güncel kademe gelir.
        let (mut w, ..) = row(DeviceKind::Wifi { strength: 90, secured: false }, DeviceState::Available);
        w.set_icon(Some(Icon::named(IconName::Ethernet)));
        w.set_kind(DeviceKind::Wifi { strength: 10, secured: false }, t0);
        assert_eq!(w.icon(), &Icon::named(IconName::Ethernet));
        w.set_icon(None);
        assert_eq!(w.icon(), &Icon::named(IconName::Wifi1));
    }

    #[test]
    fn password_buffer_is_wiped_on_every_close() {
        let zeroed = |r: &DeviceRow| r.password.raw_secret_bytes().iter().all(|b| *b == 0);
        let typed_row = || {
            let (mut r, theme, mut text, t0) = row(DeviceKind::Wifi { strength: 70, secured: true }, DeviceState::Available);
            let _ = click(&mut r, &mut text, &theme, Point::new(40.0, 20.0), t0);
            for ch in "parola1".chars() {
                let _ = r.key(&typed(ch), t0);
            }
            let _ = r.password.ime_preedit("ön", None);
            assert!(!zeroed(&r));
            (r, t0)
        };
        // Esc
        let (mut r, t0) = typed_row();
        let _ = r.key(&key(Key::Escape), t0);
        assert!(!r.is_expanded() && zeroed(&r), "Esc");
        // Çağıranın iptali (başka cihaz, hata, panel kapandı)
        let (mut r, t0) = typed_row();
        r.cancel_password(t0);
        assert!(zeroed(&r), "cancel_password");
        // Durum değişimi
        let (mut r, t0) = typed_row();
        r.set_state(DeviceState::Saved, t0);
        assert!(!r.is_expanded() && zeroed(&r), "durum");
        // Ağ şifresiz oldu
        let (mut r, t0) = typed_row();
        r.set_kind(DeviceKind::Wifi { strength: 70, secured: false }, t0);
        assert!(!r.is_expanded() && zeroed(&r), "tür");
        // Gönderim: olay parolayı Zeroizing ile bir kez taşır, tampon sıfır
        let (mut r, t0) = typed_row();
        let ev = r.key(&key(Key::Enter), t0).event;
        assert!(matches!(ev, Some(DeviceEvent::Password(_))));
        assert!(zeroed(&r), "gönderim");
    }

    #[test]
    fn debug_is_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let mut r = DeviceRow::new(&theme, SECRET, DeviceKind::Wifi { strength: 70, secured: true }, DeviceState::Available);
        r.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 360.0, 56.0));
        r.set_status_text(SECRET);
        let _ = click(&mut r, &mut text, &theme, Point::new(40.0, 20.0), t0);
        for ch in "gizli-parola".chars() {
            let _ = r.key(&typed(ch), t0);
        }
        assert_redacted(&r);
        assert_redacted(&DeviceEvent::Password(Zeroizing::new(SECRET.to_owned())));
    }
}
