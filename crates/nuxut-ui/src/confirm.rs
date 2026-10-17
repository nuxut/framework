//! Yerinde onay düğmesi ([`ConfirmButton`]): geri alınamayan eylem (kapat, sil, sıfırla) için
//! iki adımlı onay — soru metni ya da diyalog yok.
//!
//! İlk basış (tıklama, Enter/Space) düğmeyi "onay" durumuna geçirir: içerik onay ikonuna/etiketine,
//! renk onay değişkenine (varsayılan [`ButtonVariant::Destructive`] → `error` token'ı) döner ve
//! ikonun çevresinde tepeden saat yönünde geri sayım halkası ilerler (`theme.timing.confirm_window`,
//! varsayılan 4 sn; [`ConfirmButton::with_window`] ile değişir). Süre içinde ikinci basış onaylar
//! ([`ConfirmEvent::Confirmed`]). Süre dolunca, Escape'te, düğme dışına basışta ya da
//! [`ConfirmButton::cancel`] (kapsayıcı kapandı) ile vazgeçilir. Kurulduktan sonraki
//! `theme.timing.confirm_guard` içinde gelen ikinci basış yok sayılır (yanlışlıkla çift tık).
//!
//! Stil, yerleşim, odak halkası, imleç ve basılma hissi [`Button`]'ın kendisidir; bileşen yalnızca
//! içeriği takas eder ve halkayı çizer. Halka `motion.effects` yayıyla belirir/çekilir
//! (kesilebilir). Hareket açıkken halka sürerken her kare çizilir; hareket kapalıyken halka
//! saniyelik adımlarla azalır (her adım için yalnızca `ui.wake_at`). Boşta kare istenmez.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::{Duration, Instant};

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::TextSystem;
use nuxut_theme::Theme;

use crate::button::{Button, ButtonVariant};
use crate::icon::Icon;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::style::{COUNTDOWN_RING, COUNTDOWN_TRACK};
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// [`ConfirmButton`]'ın girdi sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmEvent {
    /// İlk basış: onay bekleniyor (içerik değişti; çağıran yerleşimi yenileyebilir).
    Armed,
    /// Süre içinde ikinci basış: eylemi uygula.
    Confirmed,
    /// Escape, dışarı basış ya da süre dolması (girdi sırasında fark edildiyse).
    Cancelled,
}

/// Basışın durum makinesindeki sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Press {
    Armed,
    Confirmed,
    /// Koruma süresi içinde: yok sayıldı.
    Ignored,
}

/// Saf durum makinesi: ne zamandan beri onay bekleniyor.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Arm {
    since: Option<Instant>,
    window: Duration,
    guard: Duration,
}

impl Arm {
    fn new(window: Duration, guard: Duration) -> Self {
        Self { since: None, window, guard }
    }

    fn is_armed(&self, now: Instant) -> bool {
        self.since.is_some_and(|t| now.saturating_duration_since(t) < self.window)
    }

    fn press(&mut self, now: Instant) -> Press {
        match self.since {
            Some(t) if self.is_armed(now) => {
                if now.saturating_duration_since(t) < self.guard {
                    return Press::Ignored;
                }
                self.since = None;
                Press::Confirmed
            }
            _ => {
                self.since = Some(now);
                Press::Armed
            }
        }
    }

    fn cancel(&mut self) -> bool {
        self.since.take().is_some()
    }

    /// Süresi dolduysa unutur; değiştiyse `true`.
    fn expire(&mut self, now: Instant) -> bool {
        if self.since.is_some() && !self.is_armed(now) {
            self.since = None;
            return true;
        }
        false
    }

    fn deadline(&self) -> Option<Instant> {
        self.since.map(|t| t + self.window)
    }

    /// Kalan süre oranı (1 → 0); bekleyen yoksa 0.
    fn remaining(&self, now: Instant) -> f32 {
        match self.since {
            Some(t) if !self.window.is_zero() => {
                (1.0 - now.saturating_duration_since(t).as_secs_f32() / self.window.as_secs_f32()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// Hareket kapalıyken: kalan süre saniyelik adımlarla (4 sn → 4/4, 3/4, 2/4, 1/4) ve sonraki
    /// adımın anı.
    fn stepped(&self, now: Instant) -> (f32, Option<Instant>) {
        let Some(t) = self.since else { return (0.0, None) };
        // Adım sayısı = süre saniyeye yuvarlanmış (en az 1); tamsayı aritmetiği (kayan nokta
        // kayması uyanma anını kaçırmasın).
        let steps = u32::try_from((self.window.as_millis() + 500) / 1000).unwrap_or(u32::MAX).max(1);
        let step = self.window / steps;
        if step.is_zero() {
            return (0.0, None);
        }
        let elapsed = now.saturating_duration_since(t).as_nanos();
        let i = u32::try_from(elapsed / step.as_nanos()).unwrap_or(u32::MAX).min(steps);
        let shown = (steps - i) as f32 / steps as f32;
        let next = (i < steps).then(|| t + step * (i + 1));
        (shown, next)
    }
}

/// İki adımlı yerinde onay düğmesi (bkz. modül belgesi).
#[derive(Debug, Clone)]
pub struct ConfirmButton {
    /// Görünen düğme (konum, durum, odak burada; içerik takas edilir).
    button: Button,
    /// Görünmeyen durumun içeriği (ölçü ve takas için).
    other: Button,
    /// `button` şu an onay içeriğini mi taşıyor.
    showing_armed: bool,
    arm: Arm,
    /// `with_window` ile verilen süre (yoksa `theme.timing.confirm_window`).
    window_override: Option<Duration>,
    /// Halkanın varlığı 0 → 1 (`motion.effects`): belirirken süpürür, vazgeçince çekilir.
    presence: Animated<f32>,
    /// Halkanın son gösterilen kalan oranı (çekilirken donar).
    ring_fraction: f32,
    /// İki durumun ikon boyut rolü (vars. `IconSmall`; geri sayım halkası bununla ölçeklenir).
    icon_role: TextRole,
    /// Son çizilen (oran, varlık): değişince hasar.
    drawn: (f32, f32),
}

impl ConfirmButton {
    /// `variant`/`icon`: boştaki görünüş (etiket için [`with_label`](Self::with_label)).
    /// `armed_label`: onay beklerken etiket ("Kapat · tekrar bas"); ikon aynı kalır
    /// ([`with_armed_icon`](Self::with_armed_icon)), renk [`ButtonVariant::Destructive`]
    /// ([`with_armed_variant`](Self::with_armed_variant)).
    pub fn new(theme: &Theme, variant: ButtonVariant, icon: Icon, armed_label: impl Into<String>) -> Self {
        let button = Button::new(theme, variant).with_icon(theme, icon.clone());
        let other = Button::new(theme, ButtonVariant::Destructive).with_icon(theme, icon).with_label(theme, armed_label);
        Self {
            button,
            other,
            showing_armed: false,
            arm: Arm::new(theme.timing.confirm_window, theme.timing.confirm_guard),
            window_override: None,
            presence: Animated::new(0.0, theme.motion.effects),
            ring_fraction: 1.0,
            icon_role: TextRole::IconSmall,
            drawn: (0.0, 0.0),
        }
    }

    fn idle_mut(&mut self) -> &mut Button {
        if self.showing_armed { &mut self.other } else { &mut self.button }
    }

    fn armed_mut(&mut self) -> &mut Button {
        if self.showing_armed { &mut self.button } else { &mut self.other }
    }

    /// Boştaki etiket (yoksa yalnızca ikon).
    pub fn with_label(mut self, theme: &Theme, text: impl Into<String>) -> Self {
        let text = text.into();
        self.idle_mut().rebuild(theme, |b| b.with_label(theme, text));
        self
    }

    /// Onay beklerken ikon (varsayılan: boştakiyle aynı).
    pub fn with_armed_icon(mut self, theme: &Theme, icon: Icon) -> Self {
        let role = self.icon_role;
        self.armed_mut().rebuild(theme, |b| b.with_icon(theme, icon).with_icon_role(theme, role));
        self
    }

    /// İkonun boyut rolü, iki durum için (varsayılan [`TextRole::IconSmall`]). Etiketsiz
    /// (yalnız ikon) boş hâl + [`TextRole::Icon`] ya da [`TextRole::IconLarge`] = [`IconButton`]
    /// görünümü (`space(10)` kare, aynı değişken renkleri; `Tonal` ↔ `IconButtonVariant::Tonal`,
    /// `Filled` ↔ `Filled`). Geri sayım halkasının yarıçapı ve kalınlığı ikon boyuyla ölçeklenir.
    ///
    /// [`IconButton`]: crate::IconButton
    pub fn with_icon_role(mut self, theme: &Theme, role: TextRole) -> Self {
        self.icon_role = role;
        self.button.set_icon_role(theme, role);
        self.other.set_icon_role(theme, role);
        self
    }

    /// Onay beklerken renk değişkeni (varsayılan `Destructive`).
    pub fn with_armed_variant(mut self, variant: ButtonVariant) -> Self {
        self.armed_mut().set_variant(variant);
        self
    }

    /// Bekleme süresi (varsayılan `theme.timing.confirm_window`).
    pub fn with_window(mut self, window: Duration) -> Self {
        self.window_override = Some(window);
        self.arm.window = window;
        self
    }

    /// Boştaki etiketi değiştirir.
    pub fn set_label(&mut self, text: &str) -> bool {
        self.idle_mut().set_label(text)
    }

    /// Onay etiketini değiştirir.
    pub fn set_armed_label(&mut self, text: &str) -> bool {
        self.armed_mut().set_label(text)
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.button.set_theme(theme, now);
        self.other.set_theme(theme, now);
        self.arm.window = self.window_override.unwrap_or(theme.timing.confirm_window);
        self.arm.guard = theme.timing.confirm_guard;
        self.presence.set_motion(theme.motion.effects, now);
    }

    /// Onay bekleniyor mu (süresi dolmamış).
    pub fn is_armed(&self, now: Instant) -> bool {
        self.arm.is_armed(now)
    }

    /// Bekleme bitiş anı (bekleyen yoksa `None`).
    pub fn deadline(&self) -> Option<Instant> {
        self.arm.deadline()
    }

    /// Kalan süre oranı 1 → 0 (bekleyen yoksa 0).
    pub fn remaining(&self, now: Instant) -> f32 {
        self.arm.remaining(now)
    }

    fn show(&mut self, armed: bool, now: Instant) {
        if self.showing_armed != armed {
            self.button.swap_content(&mut self.other);
            self.showing_armed = armed;
        }
        self.presence.set_target(if armed { 1.0 } else { 0.0 }, now);
        if armed {
            self.ring_fraction = 1.0;
        }
    }

    /// Programatik vazgeçme (kapsayıcı kapandı, bağlam değişti). Bekleyen varsa `true`.
    pub fn cancel(&mut self, now: Instant) -> bool {
        let was = self.arm.cancel();
        if was {
            self.show(false, now);
        }
        was
    }

    /// Süre dolduysa vazgeçer; değiştiyse `true` (içerik boşa döndü, yerleşim yenilenebilir).
    /// `draw` da çağırır; yerleşimi `preferred_size`'a göre yapan çağıran önce bunu çağırır.
    pub fn expire(&mut self, now: Instant) -> bool {
        let expired = self.arm.expire(now);
        if expired {
            self.show(false, now);
        }
        expired
    }

    pub fn is_enabled(&self) -> bool {
        self.button.is_enabled()
    }

    /// Devre dışı kalınca bekleyen onay düşer.
    pub fn set_enabled(&mut self, enabled: bool, now: Instant) -> bool {
        if !enabled {
            self.cancel(now);
        }
        self.button.set_enabled(enabled, now)
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.button.set_focus_visible(on)
    }

    /// Geçerli durumun tercih edilen boyutu (`Button` ile aynı kurallar).
    pub fn preferred_size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        self.button.preferred_size(text, theme)
    }

    /// İki durumun büyüğü: yerleşimi sabit tutmak isteyen çağıran için (onayda zıplama yok).
    pub fn max_size(&mut self, text: &mut TextSystem, theme: &Theme) -> Size {
        let a = self.button.preferred_size(text, theme);
        let b = self.other.preferred_size(text, theme);
        Size::new(a.width.max(b.width), a.height.max(b.height))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.button.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.button.rect()
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.button.cursor(p)
    }

    fn activate(&mut self, now: Instant) -> Response<ConfirmEvent> {
        match self.arm.press(now) {
            Press::Armed => {
                self.show(true, now);
                Response::event(ConfirmEvent::Armed)
            }
            Press::Confirmed => {
                self.show(false, now);
                Response::event(ConfirmEvent::Confirmed)
            }
            Press::Ignored => Response::repaint(),
        }
    }

    /// İşaretçi. Onay beklerken düğme dışına basış vazgeçirir ([`ConfirmEvent::Cancelled`]); o
    /// basışın altındaki öğeye de gidip gitmeyeceği çağıranın kararıdır. Bu yüzden beklerken
    /// yüzeydeki tüm basışlar bu bileşene de verilmelidir.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<ConfirmEvent> {
        let expired = self.expire(now);
        if self.arm.is_armed(now)
            && matches!(event.kind, PointerKind::Press(_))
            && !self.button.rect().contains(event.position)
        {
            let _ = self.button.pointer(event, now);
            self.cancel(now);
            return Response::event(ConfirmEvent::Cancelled);
        }
        let r = self.button.pointer(event, now);
        if r.event.is_some() {
            return self.activate(now);
        }
        if expired {
            return Response::event(ConfirmEvent::Cancelled);
        }
        Response { repaint: r.repaint, event: None }
    }

    /// Odaktayken Enter/Space basar; onay beklerken Escape vazgeçirir (tüketilir: `Cancelled`;
    /// beklemiyorsa Escape tüketilmez → çağıran kendi işini yapar).
    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<ConfirmEvent> {
        let expired = self.expire(now);
        if event.key == Key::Escape && event.state == KeyState::Pressed && self.cancel(now) {
            return Response::event(ConfirmEvent::Cancelled);
        }
        if self.button.key(event, now).event.is_some() {
            return self.activate(now);
        }
        if expired { Response::event(ConfirmEvent::Cancelled) } else { Response::none() }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        self.expire(now);
        self.button.draw(ui);

        let armed = self.arm.is_armed(now);
        let animate = theme.motion.enabled;
        if armed {
            self.ring_fraction = if animate {
                self.arm.remaining(now)
            } else {
                let (shown, next) = self.arm.stepped(now);
                if let Some(at) = next {
                    ui.wake_at(at);
                }
                shown
            };
            if let Some(at) = self.arm.deadline() {
                ui.wake_at(at);
            }
        }
        let presence = self.presence.value(now).clamp(0.0, 1.0);
        let settling = !self.presence.is_settled(now);
        let Some((center, icon_w)) = self.button.icon_center(ui.painter, theme) else { return };
        // Halka ikonu sarar (yarıçap ikonla büyür, ikon payı sabit space(1)); kalınlık ikon
        // boyuyla ölçeklenir: varsayılan (IconSmall) boyda COUNTDOWN_RING.
        let base = text_style(theme, TextRole::IconSmall).size;
        let k = if base > 0.0 { (icon_w / base).max(1.0) } else { 1.0 };
        let ring = COUNTDOWN_RING * k;
        // Düğmenin içinde kalır (büyük ikonda halka kenara taşmasın).
        let radius = (icon_w * 0.5 + theme.space(1)).min(self.button.rect().height * 0.5 - ring).max(icon_w * 0.5);
        let bounds = Rect::new(center.x - radius, center.y - radius, radius * 2.0, radius * 2.0).expand(ring);
        let state = (self.ring_fraction, presence);
        if state != self.drawn {
            ui.damage(bounds);
            self.drawn = state;
        }
        if (armed && animate) || settling {
            ui.damage(bounds);
            ui.request_frame();
        }
        if presence <= 0.0 {
            return;
        }
        let color = self.button.content_color(theme).multiply_alpha(presence);
        ui.painter.arc(center, radius, 0.0, TAU, ring, ui.fade(color.multiply_alpha(COUNTDOWN_TRACK)));
        let sweep = TAU * self.ring_fraction * presence;
        if sweep > 0.0 {
            ui.painter.arc(center, radius, -FRAC_PI_2, sweep, ring, ui.fade(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use nuxut_render::{DrawKind, Recorder};

    use super::*;
    use crate::icon_names::IconName;
    use crate::input::test_util::*;

    const RECT: Rect = Rect::new(10.0, 10.0, 160.0, 40.0);

    fn button(theme: &Theme) -> ConfirmButton {
        let mut b = ConfirmButton::new(theme, ButtonVariant::Tonal, Icon::named(IconName::Power), "Kapat · tekrar bas")
            .with_label(theme, "Kapat");
        b.set_rect(RECT);
        b
    }

    fn click(b: &mut ConfirmButton, at: Instant) -> Option<ConfirmEvent> {
        let _ = b.pointer(&press(20.0, 20.0), at);
        b.pointer(&release(20.0, 20.0), at).event
    }

    /// Bir kare çizer: (kare istendi mi, uyanma anı, halka çizildi mi).
    fn frame(b: &mut ConfirmButton, theme: &Theme, now: Instant) -> (bool, Option<Instant>, bool) {
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        let mut out = (false, None);
        rec.record(&mut text, (200, 80), 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            b.draw(&mut ui);
            out = (ui.wants_frame(), ui.wake_deadline());
        });
        let ring = rec.draws().iter().any(|d| d.kind == DrawKind::Stroke);
        (out.0, out.1, ring)
    }

    #[test]
    fn second_click_confirms_after_guard() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme);
        assert_eq!(click(&mut b, t), Some(ConfirmEvent::Armed));
        assert!(b.is_armed(t));
        assert_eq!(b.deadline(), Some(t + theme.timing.confirm_window));
        let later = t + theme.timing.confirm_guard + Duration::from_millis(100);
        assert_eq!(click(&mut b, later), Some(ConfirmEvent::Confirmed));
        assert!(!b.is_armed(later), "onaydan sonra boşta");
        assert_eq!(b.deadline(), None);
    }

    #[test]
    fn accidental_double_click_does_not_confirm() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme);
        assert_eq!(click(&mut b, t), Some(ConfirmEvent::Armed));
        // Çift tıkın ikinci yarısı (koruma süresi içinde): yok sayılır, onay sürer.
        assert_eq!(click(&mut b, t + Duration::from_millis(150)), None);
        assert!(b.is_armed(t + Duration::from_millis(150)));
        // Tuş tekrarı da aynı.
        assert_eq!(b.key(&key(Key::Enter), t + Duration::from_millis(200)).event, None);
        assert_eq!(click(&mut b, t + theme.timing.confirm_guard), Some(ConfirmEvent::Confirmed));
    }

    #[test]
    fn timeout_cancels_and_next_click_rearms() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme);
        click(&mut b, t);
        let w = theme.timing.confirm_window;
        assert!(!b.expire(t + w - Duration::from_millis(1)));
        assert!(b.is_armed(t + w - Duration::from_millis(1)));
        assert!(!b.is_armed(t + w), "süre dolunca beklemez");
        // Süre dolduktan sonraki tıklama onaylamaz, yeniden kurar.
        assert_eq!(click(&mut b, t + w), Some(ConfirmEvent::Armed));
        assert!(b.expire(t + w * 2));
        assert!(!b.expire(t + w * 2), "bir kez");
        // Girdi sırasında fark edilen süre dolması `Cancelled` bildirir.
        click(&mut b, t);
        assert_eq!(b.pointer(&motion(20.0, 20.0), t + w).event, Some(ConfirmEvent::Cancelled));
    }

    #[test]
    fn custom_window() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme).with_window(Duration::from_secs(2));
        click(&mut b, t);
        assert_eq!(b.deadline(), Some(t + Duration::from_secs(2)));
        assert!((b.remaining(t + Duration::from_secs(1)) - 0.5).abs() < 1e-3);
        b.set_theme(&theme, t);
        assert_eq!(b.deadline(), Some(t + Duration::from_secs(2)), "tema değişimi özel süreyi korur");
    }

    #[test]
    fn escape_outside_press_and_cancel_api() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme);
        // Beklemiyorken Escape tüketilmez.
        assert_eq!(b.key(&key(Key::Escape), t), Response::none());
        click(&mut b, t);
        assert_eq!(b.key(&key(Key::Escape), t).event, Some(ConfirmEvent::Cancelled));
        assert!(!b.is_armed(t));

        click(&mut b, t);
        assert_eq!(b.pointer(&motion(400.0, 400.0), t).event, None, "dışarıda gezinmek vazgeçirmez");
        assert_eq!(b.pointer(&press(400.0, 400.0), t).event, Some(ConfirmEvent::Cancelled));
        assert!(!b.is_armed(t));

        click(&mut b, t);
        assert!(b.cancel(t));
        assert!(!b.cancel(t));
        click(&mut b, t);
        assert!(b.set_enabled(false, t));
        assert!(!b.is_armed(t), "devre dışı kalınca düşer");
    }

    #[test]
    fn keyboard_arms_and_confirms() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme);
        assert_eq!(b.key(&key(Key::Enter), t).event, Some(ConfirmEvent::Armed));
        let later = t + theme.timing.confirm_guard;
        assert_eq!(b.key(&key(Key::Space), later).event, Some(ConfirmEvent::Confirmed));
    }

    #[test]
    fn content_swaps_and_sizes() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut text = TextSystem::new();
        let mut b = button(&theme);
        let idle = b.preferred_size(&mut text, &theme);
        click(&mut b, t);
        let armed = b.preferred_size(&mut text, &theme);
        assert!(armed.width > idle.width, "onay etiketi daha uzun");
        assert_eq!(b.button.variant(), ButtonVariant::Destructive);
        assert_eq!(b.max_size(&mut text, &theme).width, armed.width);
        b.cancel(t);
        assert_eq!(b.button.variant(), ButtonVariant::Tonal);
        assert_eq!(b.preferred_size(&mut text, &theme), idle);
        assert_eq!(b.max_size(&mut text, &theme).width, armed.width);
        assert_eq!(b.rect(), RECT, "konum takastan etkilenmez");
    }

    #[test]
    fn frames_only_while_ring_runs() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut b = button(&theme);
        // Boşta: kare yok, uyanma yok, halka yok.
        let (wants, wake, ring) = frame(&mut b, &theme, t);
        assert!(!wants && wake.is_none() && !ring);
        click(&mut b, t);
        let mid = t + theme.timing.confirm_window / 2;
        let (wants, wake, ring) = frame(&mut b, &theme, mid);
        assert!(wants && ring, "halka akarken kare");
        assert_eq!(wake, b.deadline());
        assert!((b.ring_fraction - 0.5).abs() < 1e-2);
        // Vazgeçince halka yayla çekilir, oturunca kare yok.
        b.cancel(mid);
        let (wants, ..) = frame(&mut b, &theme, mid + Duration::from_millis(16));
        assert!(wants, "çekilme sürüyor");
        let (wants, wake, ring) = frame(&mut b, &theme, mid + Duration::from_secs(5));
        assert!(!wants && wake.is_none() && !ring);
    }

    #[test]
    fn reduced_motion_steps_without_frames() {
        let (theme, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        let t = Instant::now();
        let mut b = button(&theme);
        click(&mut b, t);
        let (wants, wake, ring) = frame(&mut b, &theme, t);
        assert!(!wants, "hareket kapalı: kare istenmez");
        assert!(ring, "halka anlamlı: görünür");
        assert_eq!(b.ring_fraction, 1.0);
        assert_eq!(wake, Some(t + Duration::from_secs(1)), "sonraki saniye adımı");
        let (wants, wake, _) = frame(&mut b, &theme, t + Duration::from_millis(2500));
        assert!(!wants);
        assert_eq!(b.ring_fraction, 0.5, "4 sn'de 3. saniye: 2/4");
        assert_eq!(wake, Some(t + Duration::from_secs(3)));
        let (wants, wake, ring) = frame(&mut b, &theme, t + Duration::from_secs(4));
        assert!(!wants && wake.is_none() && !ring, "süre doldu: boşta");
    }

    #[test]
    fn arm_state_machine() {
        let t = Instant::now();
        let mut a = Arm::new(Duration::from_secs(4), Duration::from_millis(400));
        assert_eq!(a.remaining(t), 0.0);
        assert_eq!(a.stepped(t), (0.0, None));
        assert_eq!(a.press(t), Press::Armed);
        assert_eq!(a.press(t + Duration::from_millis(399)), Press::Ignored);
        assert_eq!(a.press(t + Duration::from_millis(400)), Press::Confirmed);
        assert_eq!(a.press(t), Press::Armed);
        assert_eq!(a.stepped(t + Duration::from_millis(3999)), (0.25, Some(t + Duration::from_secs(4))));
        assert_eq!(a.stepped(t + Duration::from_secs(9)).1, None);
        // Kesirli süre: en az bir adım.
        let mut s = Arm::new(Duration::from_millis(300), Duration::ZERO);
        s.press(t);
        assert_eq!(s.stepped(t), (1.0, Some(t + Duration::from_millis(300))));
    }

    /// Halka çizimlerinin (iz, kalan) sınır kutuları.
    fn ring_bounds(b: &mut ConfirmButton, theme: &Theme, now: Instant) -> Vec<Rect> {
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        rec.record(&mut text, (200, 80), 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            b.draw(&mut ui);
        });
        rec.draws().into_iter().filter(|d| d.kind == DrawKind::Stroke).map(|d| d.rect).collect()
    }

    #[test]
    fn icon_role_gives_icon_button_look_and_scales_ring() {
        let theme = Theme::default();
        let t = Instant::now();
        let mut text = TextSystem::new();
        let icon = || Icon::named(IconName::Power);
        let mut small = ConfirmButton::new(&theme, ButtonVariant::Tonal, icon(), "Kapat · tekrar bas");
        let mut large =
            ConfirmButton::new(&theme, ButtonVariant::Tonal, icon(), "Kapat · tekrar bas").with_icon_role(&theme, TextRole::IconLarge);
        // Etiketsiz boş hâl: IconButton ile aynı kare (space(10)).
        let ib = crate::IconButton::new(&theme, icon(), crate::IconButtonVariant::Tonal).preferred_size(&theme);
        assert_eq!(large.preferred_size(&mut text, &theme), ib);
        assert_eq!(small.preferred_size(&mut text, &theme), ib);
        // Onayda içerik (etiket) gelir: genişlik değişir; max_size zıplamasız yerleşim için.
        let max = large.max_size(&mut text, &theme);
        assert!(max.width > ib.width);
        let rect = Rect::new(10.0, 10.0, max.width, max.height);
        small.set_rect(rect);
        large.set_rect(rect);
        click(&mut small, t);
        click(&mut large, t);
        // Armed sonrası ikon rolü korunur (içerik takası iki durumda da aynı rol).
        let later = t + Duration::from_secs(1);
        let s = ring_bounds(&mut small, &theme, later);
        let l = ring_bounds(&mut large, &theme, later);
        assert_eq!((s.len(), l.len()), (2, 2), "iz + kalan");
        assert!(l[0].width > s[0].width, "halka ikonla büyür: {:?} / {:?}", l[0], s[0]);
        assert!(l[0].height <= rect.height + 2.0, "halka düğmenin içinde (AA payı hariç): {:?}", l[0]);
        // with_armed_icon sonradan çağrılsa da rol kalır.
        let mut b = ConfirmButton::new(&theme, ButtonVariant::Tonal, icon(), "x")
            .with_icon_role(&theme, TextRole::Icon)
            .with_armed_icon(&theme, Icon::named(IconName::Close));
        assert_eq!(b.preferred_size(&mut text, &theme), ib);
        b.set_rect(Rect::new(0.0, 0.0, 40.0, 40.0));
        click(&mut b, t);
        assert_eq!(b.button.icon_role(), Some(TextRole::Icon));
    }
}
