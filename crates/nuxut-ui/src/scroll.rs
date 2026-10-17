//! Dikey kaydırma görünümü: kinetik kaydırma, kenarda yumuşak geri sekme, sanallaştırma.
//!
//! - **Tekerlek** (çentik): hedef konum çentik başına `space(12)` ilerler; konum kritik
//!   sönümlü bir yayla akar ve hedefi **asla aşmaz** (tema `spatial_fast` az sönümlü olsa da:
//!   tekerlekte taşma, içerik "önce ters gidiyor" gibi okunuyordu). Hızlı çevirmede hız
//!   birikir ama hedefe doğru bileşeni `ω·mesafe` ile sınırlanır (kritik yayda sıfır geçişi
//!   olmaz); yön değişince eski hız atılır ve yeni hedef o anki konumdan sayılır (anında
//!   döner). Kenarda hedef sınıra kırpılır: lastik ya da geri sekme yok, düz durur.
//! - **Dokunmatik yüzey** (piksel delta): içerik parmağı doğrudan izler; kenarın ötesinde
//!   lastik direnç (taşma arttıkça azalan kazanç). Hareket bitince (`scroll_end` ya da kısa
//!   sessizlik) hız varsa üstel yavaşlayan fırlatma (analitik), sınır aşılırsa o hızla sınıra
//!   yaylanır; taşmışsa geri sekip oturur. Hareket kapalıyken (`Theme::reduced_motion`) fırlatma
//!   yok: bırakılınca olduğu yerde durur, taşma anında sınıra oturur (parmağı izlemek doğrudan
//!   işlemdir, hareket sayılmaz).
//! - **Sanallaştırma:** [`ScrollView::visible_range`] / [`visible_range_uniform`] yalnızca
//!   görünen öğelerin aralığını verir; çağıran yalnızca onları çizer.
//! - **Kaydırma çubuğu:** ince, `on_surface_variant`; imleç üstündeyken ya da hareket
//!   sürerken görünür (zamanlayıcısız: boşta iş yok), sürüklenebilir.

use std::ops::Range;
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Point, Rect};
use nuxut_motion::{Animated, Spring};
use nuxut_render::RectStyle;
use nuxut_theme::Theme;

use crate::input::{Button, CursorShape, PointerEvent, PointerKind, TOUCH_SLOP, TOUCH_TAP_DELAY};
use crate::ui::{DamageState, Response, Ui};

/// Fırlatma yavaşlamasının zaman sabiti (saniye, `motion.speed` ile ölçeklenir). Hız her
/// `TAU` saniyede `1/e`'ye iner; toplam yol `v·TAU`.
const FLING_TAU: f64 = 0.325;
/// Bunun altındaki hız (px/s) fırlatma başlatmaz.
const FLING_MIN_SPEED: f32 = 60.0;
/// Dokunmatik yüzey hareketinin bittiği sayılan sessizlik — yalnızca `ScrollEnd` gelmezse yedek.
const GESTURE_IDLE: Duration = Duration::from_millis(60);
/// Lastik direnç: taşma bu ölçeğe (görünüm yüksekliği oranı) asimptotik yaklaşır.
const RUBBER_LIMIT: f32 = 0.18;
/// Tekerlek yayının periyodu (saniye, `motion.speed` ile ölçeklenir; sönüm her zaman kritik).
/// Bir çentik (~48 px) ≈ 0.25 s'de yarım pikselin altına oturur.
const WHEEL_RESPONSE: f32 = 0.26;
/// Kaydırma göstergesinin `on_surface_variant` opaklığı.
const SCROLL_THUMB_TONE: f32 = 0.6;

/// Parmak bu süreden uzun kımıldamadan durup kalkarsa fırlatma yok (bırakma hızı sıfır).
const TOUCH_STILL: Duration = Duration::from_millis(80);

/// Dokunmatik ekran dizisinin durumu (`PointerSource::Touch`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum TouchGesture {
    None,
    /// Basma çocuklardan bekletiliyor: kaydırma mı dokunuş mu henüz belli değil.
    Pending { press: PointerEvent, at: Instant },
    /// Parmak içeriği kaydırıyor; çocuklar dizinin geri kalanını görmez.
    Panning { last_y: f32 },
    /// Basma çocuklara iletildi; dizi sonuna dek olaylar doğrudan geçer.
    Delivered,
}

/// [`ScrollView::forward`]: çocuklara iletilecek olaylar (en çok üç; ayırmasız).
#[derive(Debug, Clone, Copy)]
pub struct Forward {
    events: [PointerEvent; 3],
    len: u8,
    next: u8,
}

impl Forward {
    const fn empty() -> Self {
        let e = PointerEvent::new(Point::new(0.0, 0.0), PointerKind::Leave);
        Self { events: [e; 3], len: 0, next: 0 }
    }

    fn one(event: PointerEvent) -> Self {
        let mut f = Self::empty();
        f.push(event);
        f
    }

    fn push(&mut self, event: PointerEvent) {
        if (self.len as usize) < self.events.len() {
            self.events[self.len as usize] = event;
            self.len += 1;
        }
    }
}

impl Iterator for Forward {
    type Item = PointerEvent;

    fn next(&mut self) -> Option<PointerEvent> {
        (self.next < self.len).then(|| {
            self.next += 1;
            self.events[self.next as usize - 1]
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    /// Yay (`anim`) sürüyor ya da duruyor.
    Spring,
    /// Tekerlek: `from`'dan `v0` hızla `target`'a kritik sönümlü, aşmasız (analitik).
    Wheel { start: Instant, from: f32, v0: f32, target: f32 },
    /// Dokunmatik yüzey: konum doğrudan `offset`.
    /// `held`: dokunmatik ekranda parmak hâlâ basılı (sessizlikte kendiliğinden bitmez).
    Touch { last: Instant, velocity: f32, held: bool },
    /// Üstel yavaşlama.
    Fling { start: Instant, x0: f32, v0: f32 },
    /// Kaydırma çubuğu sürükleniyor (imleç y'si ile tutamaç arasındaki fark).
    Drag { grab: f32 },
}

#[derive(Debug, Clone)]
pub struct ScrollView {
    viewport: Rect,
    content_height: f32,
    /// Yay modunda konum.
    anim: Animated<f32>,
    /// Dokunmatik/fırlatma modunda konum.
    offset: f32,
    motion: Motion,
    tau: f64,
    /// Tekerlek yayı (kritik sönüm; hareket kapalıysa anında).
    wheel_spring: Spring,
    /// Fırlatma (atalet) açık mı; hareket kapalıyken (`Theme::reduced_motion`) bırakılan dokunmatik
    /// hareket olduğu yerde durur, taşma anında geri oturur.
    inertia: bool,
    hovered: bool,
    bar_alpha: Animated<f32>,
    damage: DamageState,
    /// Son çizimdeki kaydırma çubuğu bölgesi (imleç için).
    last_bar: Option<Rect>,
    /// Dokunmatik ekran dizisi ve son `pointer` çağrısının çocuklara ilettiği olaylar.
    gesture: TouchGesture,
    forward: Option<Forward>,
}

impl ScrollView {
    pub fn new(theme: &Theme) -> Self {
        Self {
            viewport: Rect::ZERO,
            content_height: 0.0,
            anim: Animated::new(0.0, theme.motion.spatial_fast),
            offset: 0.0,
            motion: Motion::Spring,
            tau: FLING_TAU / f64::from(theme.motion.speed.max(0.05)),
            wheel_spring: wheel_spring(theme),
            inertia: !theme.reduced_motion(),
            hovered: false,
            bar_alpha: Animated::new(0.0, theme.motion.effects),
            damage: DamageState::new(),
            last_bar: None,
            gesture: TouchGesture::None,
            forward: None,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.anim.set_motion(theme.motion.spatial_fast, now);
        self.bar_alpha.set_motion(theme.motion.effects, now);
        self.tau = FLING_TAU / f64::from(theme.motion.speed.max(0.05));
        self.wheel_spring = wheel_spring(theme);
        self.inertia = !theme.reduced_motion();
        self.damage.mark();
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    /// Görünüm alanı ve içerik yüksekliği. İçerik kısalırsa konum sınıra yaylanır.
    pub fn set_geometry(&mut self, viewport: Rect, content_height: f32, now: Instant) {
        if self.viewport == viewport && self.content_height == content_height {
            return;
        }
        self.viewport = viewport;
        self.content_height = content_height.max(0.0);
        self.damage.mark();
        let max = self.max_offset();
        match self.motion {
            Motion::Spring if self.anim.target() > max => self.anim.set_target(max, now),
            Motion::Wheel { target, .. } if target > max => {
                let (from, v) = (self.offset(now), self.velocity(now));
                self.enter_wheel(from, v, max, now);
            }
            _ => {}
        }
    }

    pub fn max_offset(&self) -> f32 {
        (self.content_height - self.viewport.height).max(0.0)
    }

    /// Geçerli kaydırma konumu (taşma dahil; negatif = üstte taşma).
    pub fn offset(&self, now: Instant) -> f32 {
        match self.motion {
            Motion::Spring | Motion::Drag { .. } => self.anim.value(now),
            Motion::Touch { .. } => self.offset,
            Motion::Wheel { start, from, v0, target } => self.wheel_at(start, from, v0, target, now).0,
            Motion::Fling { start, x0, v0 } => fling_at(x0, v0, self.tau, now.saturating_duration_since(start).as_secs_f64()).0,
        }
    }

    /// Tekerlek hareketinin `now` anındaki (konum, hız).
    fn wheel_at(&self, start: Instant, from: f32, v0: f32, target: f32, now: Instant) -> (f32, f32) {
        let t = now.saturating_duration_since(start).as_secs_f64();
        let s = self.wheel_spring.evaluate(f64::from(from - target), f64::from(v0), t);
        (target + s.x as f32, s.v as f32)
    }

    /// İçeriğin sol-üst köşesinin yüzey koordinatı (içerik `y`'si buna eklenir).
    pub fn content_origin(&self, now: Instant) -> Point {
        Point::new(self.viewport.x, self.viewport.y - self.offset(now))
    }

    /// İçerik koordinatına çevrilmiş olay (çocuklar içerik koordinatında yerleşiyorsa).
    pub fn to_content(&self, event: &PointerEvent, now: Instant) -> PointerEvent {
        let o = self.content_origin(now);
        event.translated(o)
    }

    /// Animasyonla bir konuma gider (ör. seçili öğeyi görünür kılmak için).
    pub fn scroll_to(&mut self, offset: f32, now: Instant) {
        let current = self.offset(now);
        let velocity = self.velocity(now);
        self.enter_spring(current, velocity, offset.clamp(0.0, self.max_offset()), now);
    }

    /// `[top, bottom)` içerik aralığını görünür kılar (gerekirse en az kaydırmayla).
    /// Görünüm henüz yerleşmemişse (yükseklik 0) hiçbir şey yapmaz.
    pub fn ensure_visible(&mut self, top: f32, bottom: f32, now: Instant) {
        if let Some(o) = self.reveal_offset(top, bottom) {
            self.scroll_to(o, now);
        }
    }

    /// [`ensure_visible`](Self::ensure_visible) gibi, ama animasyonsuz (ilk yerleşimde).
    pub fn ensure_visible_now(&mut self, top: f32, bottom: f32, now: Instant) {
        if let Some(o) = self.reveal_offset(top, bottom) {
            let o = o.clamp(0.0, self.max_offset());
            self.enter_spring(o, 0.0, o, now);
        }
    }

    /// `[top, bottom)`'ı gösteren en yakın konum; zaten görünürse ya da görünüm boşsa `None`.
    fn reveal_offset(&self, top: f32, bottom: f32) -> Option<f32> {
        let h = self.viewport.height;
        if h <= 0.0 {
            return None;
        }
        let cur = self.target_offset();
        if top < cur {
            Some(top)
        } else if bottom > cur + h {
            Some(bottom - h)
        } else {
            None
        }
    }

    /// Hareketin varacağı konum (süren yay/tekerlek/fırlatmanın hedefi).
    pub(crate) fn target_offset(&self) -> f32 {
        match self.motion {
            Motion::Spring | Motion::Drag { .. } => self.anim.target(),
            Motion::Touch { .. } => self.offset,
            Motion::Wheel { target, .. } => target,
            Motion::Fling { x0, v0, .. } => x0 + v0 * self.tau as f32,
        }
    }

    fn velocity(&self, now: Instant) -> f32 {
        match self.motion {
            Motion::Spring | Motion::Drag { .. } => self.anim.velocity(now),
            Motion::Touch { velocity, .. } => velocity,
            Motion::Wheel { start, from, v0, target } => self.wheel_at(start, from, v0, target, now).1,
            Motion::Fling { start, x0, v0 } => fling_at(x0, v0, self.tau, now.saturating_duration_since(start).as_secs_f64()).1,
        }
    }

    /// Tekerlek moduna geçer: `from`'dan `target`'a, aşmasız. Hız yalnızca hedefe doğru
    /// bileşeniyle ve en çok `ω·mesafe` kadar taşınır: kritik yayda `x(t) = (x0 + b·t)·e^(−ωt)`,
    /// `b = v0 + ω·x0`; bu sınırla `b` `x0` ile aynı işaretli kalır, sıfır geçişi (aşma) olmaz ve
    /// hareket tekdüze olur.
    fn enter_wheel(&mut self, from: f32, velocity: f32, target: f32, now: Instant) {
        let dist = target - from;
        let v0 = if self.wheel_spring.is_instant() || velocity * dist <= 0.0 {
            0.0
        } else {
            let w0 = std::f32::consts::TAU / self.wheel_spring.response;
            velocity.signum() * velocity.abs().min(w0 * dist.abs())
        };
        self.motion = Motion::Wheel { start: now, from, v0, target };
        self.damage.mark();
    }

    /// Yay moduna geçer: `from` konumundan `velocity` hızla `target`'a.
    fn enter_spring(&mut self, from: f32, velocity: f32, target: f32, now: Instant) {
        self.anim.jump_to(from);
        self.anim.set_target(target, now);
        // `jump_to` hızı sıfırlar; gelen hızı itme olarak geri ver.
        if velocity != 0.0 {
            self.anim.kick(velocity, now);
        }
        self.motion = Motion::Spring;
        self.damage.mark();
    }

    /// Dokunmatik yüzey hareketi bitti (`PointerKind::ScrollEnd` = `axis_stop`; `pointer` bunu
    /// kendisi bağlar). Compositor vermezse kısa sessizlikten sonra kendiliğinden biter.
    pub fn scroll_end(&mut self, now: Instant) {
        if let Motion::Touch { velocity, .. } = self.motion {
            self.release(velocity, now);
        }
    }

    fn release(&mut self, velocity: f32, now: Instant) {
        let x = self.offset;
        let max = self.max_offset();
        if x < 0.0 || x > max {
            self.enter_spring(x, velocity, x.clamp(0.0, max), now);
        } else if self.inertia && velocity.abs() >= FLING_MIN_SPEED {
            self.motion = Motion::Fling { start: now, x0: x, v0: velocity };
        } else {
            self.enter_spring(x, 0.0, x, now);
        }
        self.damage.mark();
    }

    /// Kare başı bakımı: hareketin bitip bitmediği, fırlatmanın sınıra çarpması.
    fn advance(&mut self, now: Instant) {
        match self.motion {
            Motion::Touch { last, velocity, held: false } if now.saturating_duration_since(last) >= GESTURE_IDLE => {
                self.release(velocity, now);
            }
            Motion::Wheel { start, from, v0, target } => {
                let (x, v) = self.wheel_at(start, from, v0, target, now);
                if (x - target).abs() < 0.25 && v.abs() < 1.0 {
                    self.enter_spring(target, 0.0, target, now);
                }
            }
            Motion::Fling { start, x0, v0 } => {
                let t = now.saturating_duration_since(start).as_secs_f64();
                let (x, v) = fling_at(x0, v0, self.tau, t);
                let max = self.max_offset();
                if x < 0.0 || x > max {
                    // Sınıra çarptı: kalan hızla sınıra yaylanır (taşıp geri oturur).
                    self.enter_spring(x.clamp(0.0, max), v, x.clamp(0.0, max), now);
                } else if v.abs() < 1.0 {
                    self.enter_spring(x, 0.0, x, now);
                }
            }
            _ => {}
        }
    }

    fn is_moving(&mut self, now: Instant) -> bool {
        match self.motion {
            Motion::Spring | Motion::Drag { .. } => !self.anim.is_settled(now),
            Motion::Touch { .. } | Motion::Fling { .. } | Motion::Wheel { .. } => true,
        }
    }

    fn bar_rects(&self, theme: &Theme, now: Instant) -> Option<(Rect, Rect)> {
        let max = self.max_offset();
        if max <= 0.0 {
            return None;
        }
        let v = self.viewport;
        let w = theme.space(1);
        let inset = theme.space(1);
        let track = Rect::new(v.right() - w - inset, v.y + inset, w, v.height - inset * 2.0);
        let ratio = (v.height / self.content_height).clamp(0.0, 1.0);
        let min_len = theme.space(8);
        let len = (track.height * ratio).max(min_len).min(track.height);
        let off = self.offset(now);
        // Taşmada çubuk kısalır (iOS gibi), kaybolmaz.
        let over = if off < 0.0 { -off } else if off > max { off - max } else { 0.0 };
        let len = (len - over).max(w * 2.0);
        let pos = (off.clamp(0.0, max) / max) * (track.height - len);
        Some((track, Rect::new(track.x, track.y + pos, w, len)))
    }

    /// İşaretçi. Kaydırma olaylarını tüketir (`event = Some(())` = konum değişti).
    ///
    /// Dokunmatik ekranda (`PointerSource::Touch`) görünüm içindeki basma çocuklardan bekletilir:
    /// parmak [`TOUCH_SLOP`]'u dikey aşarsa içerik parmağı izler (kenarda lastik direnç, bırakınca
    /// fırlatma) ve çocuk basmayı hiç görmez; yatay aşarsa, [`TOUCH_TAP_DELAY`] dolarsa ya da parmak
    /// kalkarsa basma çocuğa iletilir. Fırlatma sürerken dokunuş içeriği durdurur, eylem üretmez.
    /// Çocuklara ne iletileceğini [`forward`](Self::forward) verir.
    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response {
        self.forward = None;
        if event.is_touch() {
            return self.touch_pointer(event, now);
        }
        let inside = self.viewport.contains(event.position);
        let mut out = Response::none();
        if inside != self.hovered && !matches!(self.motion, Motion::Drag { .. }) {
            self.hovered = inside;
            self.bar_alpha.set_target(if inside { 1.0 } else { 0.0 }, now);
            self.damage.mark();
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Leave if self.hovered => {
                self.hovered = false;
                self.bar_alpha.set_target(0.0, now);
                out.repaint = true;
            }
            PointerKind::ScrollEnd if matches!(self.motion, Motion::Touch { .. }) => {
                self.scroll_end(now);
                return Response::repaint();
            }
            PointerKind::Scroll { delta, steps } if inside => {
                if steps.y != 0.0 {
                    self.wheel(theme, steps.y, now);
                } else if delta.y != 0.0 {
                    self.touch(delta.y, false, now);
                } else {
                    return out;
                }
                self.damage.mark();
                return Response::event(());
            }
            PointerKind::Press(Button::Left) => {
                if let Some((track, thumb)) = self.bar_rects(theme, now) {
                    let hit = track.expand(theme.space(1));
                    if hit.contains(event.position) {
                        let grab = if thumb.contains(event.position) {
                            event.position.y - thumb.y
                        } else {
                            thumb.height * 0.5
                        };
                        let cur = self.offset(now);
                        self.anim.jump_to(cur);
                        self.motion = Motion::Drag { grab };
                        self.drag_to(theme, event.position.y, now);
                        return Response::event(());
                    }
                }
            }
            PointerKind::Motion if self.is_dragging_bar() => {
                self.drag_to(theme, event.position.y, now);
                return Response::event(());
            }
            PointerKind::Release(Button::Left) if self.is_dragging_bar() => {
                self.motion = Motion::Spring;
                self.hovered = inside;
                self.bar_alpha.set_target(if inside { 1.0 } else { 0.0 }, now);
                self.damage.mark();
                return Response::repaint();
            }
            _ => {}
        }
        out
    }

    /// Kaydırma çubuğu sürükleniyor mu (çocuklara girdi verilmemeli).
    pub fn is_dragging_bar(&self) -> bool {
        matches!(self.motion, Motion::Drag { .. })
    }

    /// `pointer`'dan sonra: aynı olay için çocuklara (içerik) iletilecek olaylar, sırayla.
    /// Fare olayı olduğu gibi geçer (çubuk sürüklenirken hiçbiri); dokunmada bekletilen basma
    /// burada yeniden üretilir (`Motion`, `Press`, ardından güncel olay). Kapsayıcılar çocuklara
    /// `event` yerine bunları verir:
    /// `for ev in scroll.forward(event) { child.pointer(&ev, now) … }`.
    pub fn forward(&mut self, event: &PointerEvent) -> Forward {
        match self.forward.take() {
            Some(f) => f,
            None if self.is_dragging_bar() => Forward::empty(),
            None => Forward::one(*event),
        }
    }

    /// Dokunmatik dizi parmağı izleyerek içeriği kaydırıyor mu.
    pub fn is_panning(&self) -> bool {
        matches!(self.gesture, TouchGesture::Panning { .. })
    }

    fn touch_pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        let mut fwd = Forward::empty();
        let mut out = Response::none();
        let pos = event.position;
        match (self.gesture, event.kind) {
            (_, PointerKind::Press(Button::Left)) => {
                if !self.viewport.contains(pos) {
                    self.gesture = TouchGesture::Delivered;
                    fwd.push(*event);
                } else if self.is_moving(now) && self.velocity(now).abs() >= FLING_MIN_SPEED {
                    // Akan içeriği yakala: durur, dokunuş eylem üretmez.
                    self.begin_pan(now);
                    self.gesture = TouchGesture::Panning { last_y: pos.y };
                    out = Response::repaint();
                } else {
                    self.gesture = TouchGesture::Pending { press: *event, at: now };
                }
            }
            (TouchGesture::Pending { press, at }, PointerKind::Motion) => {
                let d = pos - press.position;
                if d.x.hypot(d.y) > TOUCH_SLOP {
                    if d.y.abs() >= d.x.abs() && self.max_offset() > 0.0 {
                        self.begin_pan(now);
                        // Eşik payı düşülür: içerik sıçramadan parmağı izlemeye başlar.
                        let last_y = press.position.y + TOUCH_SLOP * d.y.signum();
                        self.gesture = TouchGesture::Panning { last_y };
                        self.pan_to(pos.y, now);
                        out = Response::event(());
                    } else {
                        Self::deliver_press(&mut fwd, press);
                        fwd.push(*event);
                        self.gesture = TouchGesture::Delivered;
                    }
                } else if now.saturating_duration_since(at) >= TOUCH_TAP_DELAY {
                    Self::deliver_press(&mut fwd, press);
                    if pos != press.position {
                        fwd.push(*event);
                    }
                    self.gesture = TouchGesture::Delivered;
                }
            }
            (TouchGesture::Pending { press, .. }, PointerKind::Release(Button::Left)) => {
                // Hızlı dokunuş: beklemeden basma + bırakma.
                Self::deliver_press(&mut fwd, press);
                fwd.push(*event);
                self.gesture = TouchGesture::None;
            }
            (TouchGesture::Panning { .. }, PointerKind::Motion) => {
                self.pan_to(pos.y, now);
                out = Response::event(());
            }
            (TouchGesture::Panning { .. }, PointerKind::Release(_) | PointerKind::Leave) => {
                if let Motion::Touch { last, velocity, .. } = self.motion {
                    let still = now.saturating_duration_since(last) > TOUCH_STILL;
                    self.release(if still { 0.0 } else { velocity }, now);
                }
                self.gesture = TouchGesture::None;
                out = Response::repaint();
            }
            (TouchGesture::Pending { .. }, PointerKind::Leave) => {
                // İptal: çocuk basmayı hiç görmedi.
                self.gesture = TouchGesture::None;
            }
            (TouchGesture::Delivered, PointerKind::Release(_) | PointerKind::Leave) => {
                fwd.push(*event);
                self.gesture = TouchGesture::None;
            }
            (TouchGesture::Delivered, _) => fwd.push(*event),
            // Basma öncesi Enter/Motion: dokunmada hover yok, çocuklar basmayla birlikte görür.
            (TouchGesture::None, PointerKind::Leave) => fwd.push(*event),
            _ => {}
        }
        self.forward = Some(fwd);
        if out.repaint || out.event.is_some() {
            self.damage.mark();
        }
        out
    }

    fn deliver_press(fwd: &mut Forward, press: PointerEvent) {
        fwd.push(press.with_kind(press.position, PointerKind::Motion));
        fwd.push(press);
    }

    /// Parmak içeriği tutar: o anki konumda durur (taşma korunur).
    fn begin_pan(&mut self, now: Instant) {
        self.offset = self.offset(now);
        self.motion = Motion::Touch { last: now, velocity: 0.0, held: true };
        self.damage.mark();
    }

    fn pan_to(&mut self, y: f32, now: Instant) {
        let TouchGesture::Panning { last_y } = self.gesture else { return };
        // Parmak aşağı → içerik aşağı → konum azalır.
        self.touch(last_y - y, true, now);
        self.gesture = TouchGesture::Panning { last_y: y };
        self.damage.mark();
    }

    fn drag_to(&mut self, theme: &Theme, y: f32, now: Instant) {
        let Motion::Drag { grab } = self.motion else { return };
        let Some((track, thumb)) = self.bar_rects(theme, now) else { return };
        let travel = (track.height - thumb.height).max(1.0);
        let t = ((y - grab - track.y) / travel).clamp(0.0, 1.0);
        self.anim.jump_to(t * self.max_offset());
        self.damage.mark();
    }

    fn wheel(&mut self, theme: &Theme, steps: f32, now: Instant) {
        let max = self.max_offset();
        let cur = self.offset(now);
        // Aynı yönde art arda çentikler bekleyen hedefe eklenir; yön değişince o anki konumdan
        // sayılır (yoksa yeni hedef hâlâ eski yönde kalabilir ve içerik bir an ters gider).
        let base = match self.motion {
            Motion::Wheel { target, .. } if (target - cur) * steps > 0.0 => target,
            _ => cur,
        };
        let target = (base + steps * theme.space(12)).clamp(0.0, max);
        let velocity = self.velocity(now);
        self.enter_wheel(cur, velocity, target, now);
    }

    fn touch(&mut self, dy: f32, held: bool, now: Instant) {
        let cur = self.offset(now);
        let (last_t, last_v) = match self.motion {
            Motion::Touch { last, velocity, .. } => (Some(last), velocity),
            _ => (None, 0.0),
        };
        let max = self.max_offset();
        let limit = self.viewport.height * RUBBER_LIMIT;
        // Taşma bölgesinde kazanç taşma arttıkça azalır (asimptotik lastik).
        let over = if cur < 0.0 { -cur } else if cur > max { cur - max } else { 0.0 };
        let outward = (cur <= 0.0 && dy < 0.0) || (cur >= max && dy > 0.0);
        let gain = if outward && limit > 0.0 { (1.0 - over / limit).clamp(0.0, 1.0).powi(2) } else { 1.0 };
        self.offset = cur + dy * gain;
        let velocity = match last_t {
            Some(t) => {
                let dt = now.saturating_duration_since(t).as_secs_f32().max(1.0 / 240.0);
                // Üstel ortalama: tek olay sıçramasını yumuşatır.
                last_v * 0.4 + (dy / dt) * 0.6
            }
            None => 0.0,
        };
        self.motion = Motion::Touch { last: now, velocity, held };
    }

    /// İmleç: kaydırma çubuğu (ve sürüklemesi) `Default`; içerik için çağıran çocuğa sorar
    /// (`to_content`).
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.is_dragging_bar() || self.last_bar.is_some_and(|b| b.contains(p)) {
            Some(CursorShape::Default)
        } else {
            None
        }
    }

    /// Görünür içerik aralığı (`[üst, alt)`, içerik koordinatı).
    pub fn visible_span(&self, now: Instant) -> Range<f32> {
        let o = self.offset(now);
        o..o + self.viewport.height
    }

    /// Öğe başlangıçları `starts` (artan, içerik koordinatı) için görünen
    /// öğelerin indis aralığı (ikili arama).
    pub fn visible_range(&self, starts: &[f32], now: Instant) -> Range<usize> {
        let span = self.visible_span(now);
        let first = starts.partition_point(|&s| s <= span.start).saturating_sub(1);
        let last = starts.partition_point(|&s| s < span.end);
        first.min(starts.len())..last.min(starts.len())
    }

    /// Çizim: içeriği görünüme kırpar, `f(ui, content_origin, visible_span)` ile çizdirir,
    /// kaydırma çubuğunu üstüne çizer.
    pub fn draw(&mut self, ui: &mut Ui, f: impl FnOnce(&mut Ui, Point, Range<f32>)) {
        let now = ui.now;
        self.advance(now);
        let theme = ui.theme;
        let origin = self.content_origin(now);
        let span = self.visible_span(now);
        ui.painter.push_clip(self.viewport);
        f(ui, origin, span);
        ui.painter.pop_clip();

        let moving = self.is_moving(now);
        // Hareket sürerken çubuk tam görünür; durunca (imleç dışarıdaysa) söner.
        if moving {
            if self.bar_alpha.target() < 1.0 {
                self.bar_alpha.jump_to(1.0);
            }
        } else if !self.hovered && self.bar_alpha.target() > 0.0 {
            self.bar_alpha.set_target(0.0, now);
        }
        let alpha = self.bar_alpha.value(now).clamp(0.0, 1.0);
        if alpha > 0.0
            && let Some((_, thumb)) = self.bar_rects(theme, now)
        {
            // Kaydırma göstergesi metin dışı işaret: yüksek karşıtlıkta ≥ 3:1 (`style::faded_mark`).
            let color = ui.fade(crate::style::faded_mark(theme, theme.colors.on_surface_variant, SCROLL_THUMB_TONE, &[]).multiply_alpha(alpha));
            ui.painter.rect(thumb, &RectStyle::filled(color, Corners::uniform(thumb.width * 0.5)));
        }
        self.last_bar = self.bar_rects(theme, now).map(|(track, _)| track.expand(theme.space(1)));
        let animating = moving | !self.bar_alpha.is_settled(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.viewport, animating);
    }
}

/// Düzgün yükseklikli öğeler için görünen indis aralığı.
pub fn visible_range_uniform(span: Range<f32>, item_height: f32, count: usize) -> Range<usize> {
    if item_height <= 0.0 || count == 0 {
        return 0..0;
    }
    let first = (span.start / item_height).floor().max(0.0) as usize;
    let last = (span.end / item_height).ceil().max(0.0) as usize;
    first.min(count)..last.min(count)
}

/// Tekerlek yayı: kritik sönüm, `motion.speed` ile ölçekli; hareket kapalıysa anında.
fn wheel_spring(theme: &Theme) -> Spring {
    if theme.motion.enabled && !theme.motion.spatial_fast.is_instant() {
        Spring::new(WHEEL_RESPONSE, 1.0).scaled(theme.motion.speed)
    } else {
        Spring::INSTANT
    }
}

/// Üstel yavaşlama: `t` anında (konum, hız).
fn fling_at(x0: f32, v0: f32, tau: f64, t: f64) -> (f32, f32) {
    let e = (-t / tau).exp();
    let x = f64::from(x0) + f64::from(v0) * tau * (1.0 - e);
    (x as f32, (f64::from(v0) * e) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> (ScrollView, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = ScrollView::new(&theme);
        s.set_geometry(Rect::new(0.0, 0.0, 300.0, 400.0), 2000.0, t0);
        (s, theme, t0)
    }

    fn scroll(x: f32, y: f32, delta: f32, steps: f32) -> PointerEvent {
        PointerEvent::new(Point::new(x, y), PointerKind::Scroll { delta: Point::new(0.0, delta), steps: Point::new(0.0, steps) })
    }

    fn run(s: &mut ScrollView, from: Instant, secs: f32) -> Instant {
        let mut t = from;
        for _ in 0..(secs * 120.0) as u32 {
            t += Duration::from_micros(8333);
            s.advance(t);
        }
        t
    }

    #[test]
    fn ensure_visible_is_noop_without_viewport() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = ScrollView::new(&theme);
        s.set_geometry(Rect::new(0.0, 0.0, 300.0, 0.0), 2000.0, t0);
        s.ensure_visible(400.0, 456.0, t0);
        s.ensure_visible_now(400.0, 456.0, t0);
        assert_eq!(s.offset(t0 + std::time::Duration::from_secs(3)), 0.0, "sıfır yükseklik: kaydırma yok");
        // Yerleşince animasyonsuz sürüm aynı anda varır.
        s.set_geometry(Rect::new(0.0, 0.0, 300.0, 400.0), 2000.0, t0);
        s.ensure_visible_now(400.0, 456.0, t0);
        assert_eq!(s.offset(t0), 56.0);
    }

    #[test]
    fn wheel_scrolls_smoothly_and_clamps() {
        let (mut s, theme, t0) = view();
        assert_eq!(s.pointer(&theme, &scroll(10.0, 10.0, 0.0, 2.0), t0).event, Some(()));
        assert_eq!(s.offset(t0), 0.0, "yay: anında atlamaz");
        let t1 = run(&mut s, t0, 2.0);
        assert!((s.offset(t1) - theme.space(24)).abs() < 0.5);
        // Üst kenarda yukarı: sınırda kalır.
        let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, -50.0), t1);
        let t2 = run(&mut s, t1, 3.0);
        assert!(s.offset(t2).abs() < 0.5);
    }

    /// `secs` boyunca 1 ms adımla örnekler (bakım dahil); her örnek `(t, konum)`.
    fn samples(s: &mut ScrollView, from: Instant, secs: f32) -> Vec<(Instant, f32)> {
        let mut t = from;
        let mut out = Vec::new();
        for _ in 0..(secs * 1000.0) as u32 {
            t += Duration::from_millis(1);
            s.advance(t);
            out.push((t, s.offset(t)));
        }
        out
    }

    #[test]
    fn wheel_never_overshoots_and_is_monotonic() {
        let (mut s, theme, t0) = view();
        let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, 1.0), t0);
        let target = theme.space(12);
        let mut prev = 0.0;
        for (_, x) in samples(&mut s, t0, 1.0) {
            assert!(x <= target + 1e-3, "tek çentik hedefi aştı: {x} > {target}");
            assert!(x >= prev - 1e-3, "geri gitti: {x} < {prev}");
            prev = x;
        }
        assert!((prev - target).abs() < 0.5, "oturmadı: {prev}");
        assert_eq!(s.motion, Motion::Spring, "oturunca yay modunda (boşta kare yok)");
    }

    #[test]
    fn rapid_wheel_accumulates_without_overshoot() {
        let (mut s, theme, t0) = view();
        let mut t = t0;
        let mut prev = 0.0;
        // 10 ms arayla 8 çentik: hız birikir, hiçbir an son hedefi aşmaz ve geri gitmez.
        for _ in 0..8 {
            let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, 1.0), t);
            for (tt, x) in samples(&mut s, t, 0.01) {
                assert!(x >= prev - 1e-3, "geri gitti: {x} < {prev}");
                prev = x;
                t = tt;
            }
        }
        let target = 8.0 * theme.space(12);
        for (_, x) in samples(&mut s, t, 1.5) {
            assert!(x <= target + 1e-3, "aştı: {x} > {target}");
            assert!(x >= prev - 1e-3, "geri gitti: {x} < {prev}");
            prev = x;
        }
        assert!((prev - target).abs() < 0.5);
    }

    #[test]
    fn wheel_stops_flat_at_edges() {
        let (mut s, theme, t0) = view();
        // Üst kenar: yukarı tekerlek hiç taşmaz.
        let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, -3.0), t0);
        for (_, x) in samples(&mut s, t0, 0.5) {
            assert_eq!(x, 0.0, "üst kenarda kıpırdamamalı");
        }
        // Alt kenara hızlıca: sınırın ötesine bir an bile geçmez, düz durur.
        let mut t = t0 + Duration::from_millis(500);
        let max = s.max_offset();
        for _ in 0..60 {
            let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, 2.0), t);
            for (tt, x) in samples(&mut s, t, 0.008) {
                assert!(x <= max + 1e-3, "alt kenarı aştı: {x} > {max}");
                t = tt;
            }
        }
        for (_, x) in samples(&mut s, t, 1.0) {
            assert!(x <= max + 1e-3, "alt kenarı aştı: {x} > {max}");
        }
        assert!((s.offset(t + Duration::from_secs(1)) - max).abs() < 0.5);
    }

    #[test]
    fn wheel_reverses_immediately() {
        let (mut s, theme, t0) = view();
        let mut t = t0;
        for _ in 0..4 {
            let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, 1.0), t);
            t += Duration::from_millis(15);
        }
        t += Duration::from_millis(40);
        s.advance(t);
        assert!(s.velocity(t) > 0.0, "aşağı hızla gidiyor olmalı");
        let at = s.offset(t);
        assert!(at > theme.space(12), "ters çentiğe yer olmalı: {at}");
        let _ = s.pointer(&theme, &scroll(10.0, 10.0, 0.0, -1.0), t);
        assert!(s.velocity(t) <= 0.0, "yön değişimi anında: eski hız atılmalı");
        let mut prev = at;
        for (_, x) in samples(&mut s, t, 1.0) {
            assert!(x <= prev + 1e-3, "ters çentikten sonra hâlâ aşağı gitti: {x} > {prev}");
            prev = x;
        }
        assert!((prev - (at - theme.space(12))).abs() < 0.5, "o anki konumdan bir çentik yukarı: {prev}");
    }

    #[test]
    fn wheel_is_instant_without_motion() {
        let (t_on, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        let t0 = Instant::now();
        let mut s = ScrollView::new(&t_on);
        s.set_geometry(Rect::new(0.0, 0.0, 300.0, 400.0), 2000.0, t0);
        let _ = s.pointer(&t_on, &scroll(10.0, 10.0, 0.0, 1.0), t0);
        assert_eq!(s.offset(t0), t_on.space(12));
    }

    #[test]
    fn touch_fling_decelerates_and_stops_in_bounds() {
        let (mut s, theme, t0) = view();
        let mut t = t0;
        for _ in 0..6 {
            t += Duration::from_millis(10);
            let _ = s.pointer(&theme, &scroll(10.0, 10.0, 20.0, 0.0), t);
        }
        let at_release = s.offset(t);
        assert!((at_release - 120.0).abs() < 1e-3);
        s.scroll_end(t);
        assert!(matches!(s.motion, Motion::Fling { .. }));
        let t2 = run(&mut s, t, 4.0);
        let end = s.offset(t2);
        assert!(end > at_release + 100.0, "fırlatma devam etmeli: {end}");
        assert!(end <= s.max_offset() + 0.5);
    }

    #[test]
    fn scroll_end_event_starts_fling_immediately() {
        let (mut s, theme, t0) = view();
        let mut t = t0;
        for _ in 0..5 {
            t += Duration::from_millis(10);
            let _ = s.pointer(&theme, &scroll(10.0, 10.0, 25.0, 0.0), t);
        }
        let r = s.pointer(&theme, &PointerEvent::new(Point::new(10.0, 10.0), PointerKind::ScrollEnd), t);
        assert!(r.repaint);
        assert!(matches!(s.motion, Motion::Fling { .. }), "60 ms beklemeden fırlatma");
    }

    #[test]
    fn rubber_band_and_bounce_back() {
        let (mut s, theme, t0) = view();
        let mut t = t0;
        for _ in 0..30 {
            t += Duration::from_millis(10);
            let _ = s.pointer(&theme, &scroll(10.0, 10.0, -40.0, 0.0), t);
        }
        let over = -s.offset(t);
        assert!(over > 0.0, "üstte taşma olmalı");
        assert!(over < 400.0 * RUBBER_LIMIT, "lastik sınırı aşılmamalı: {over}");
        // Sessizlik → geri sekme.
        let t2 = run(&mut s, t, 3.0);
        assert!(s.offset(t2).abs() < 0.5);
    }

    #[test]
    fn virtualization_ranges() {
        let (mut s, _theme, t0) = view();
        s.anim.jump_to(450.0);
        let starts: Vec<f32> = (0..40).map(|i| i as f32 * 50.0).collect();
        assert_eq!(s.visible_range(&starts, t0), 9..17);
        assert_eq!(visible_range_uniform(450.0..850.0, 50.0, 40), 9..17);
        assert_eq!(visible_range_uniform(0.0..100.0, 50.0, 1), 0..1);
    }

    #[test]
    fn scrollbar_drag() {
        let (mut s, theme, t0) = view();
        let (track, thumb) = s.bar_rects(&theme, t0).unwrap();
        let p = Point::new(thumb.center().x, thumb.center().y);
        let _ = s.pointer(&theme, &PointerEvent::new(p, PointerKind::Press(Button::Left)), t0);
        assert!(s.is_dragging_bar());
        let _ = s.pointer(&theme, &PointerEvent::new(Point::new(p.x, track.bottom() + 100.0), PointerKind::Motion), t0);
        assert_eq!(s.offset(t0), s.max_offset());
    }

    fn t(kind: PointerKind, y: f32) -> PointerEvent {
        PointerEvent::touch(Point::new(100.0, y), kind)
    }

    fn kinds(f: Forward) -> Vec<PointerKind> {
        f.map(|e| e.kind).collect()
    }

    const PRESS: PointerKind = PointerKind::Press(Button::Left);
    const RELEASE: PointerKind = PointerKind::Release(Button::Left);

    /// Öykünme dizisinin başı; çocuklara hiçbir şey geçmez (dokunmada hover yok, basma bekletilir).
    fn down(s: &mut ScrollView, theme: &Theme, y: f32, now: Instant) {
        for k in [PointerKind::Enter, PointerKind::Motion, PRESS] {
            let ev = t(k, y);
            let _ = s.pointer(theme, &ev, now);
            assert!(kinds(s.forward(&ev)).is_empty(), "{k:?}");
        }
    }

    #[test]
    fn mouse_events_pass_through() {
        let (mut s, theme, t0) = view();
        let ev = PointerEvent::new(Point::new(10.0, 10.0), PRESS);
        let _ = s.pointer(&theme, &ev, t0);
        assert_eq!(kinds(s.forward(&ev)), [PRESS]);
    }

    #[test]
    fn touch_tap_forwards_press_and_release_on_lift() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 100.0, t0);
        let up = t(RELEASE, 103.0);
        let _ = s.pointer(&theme, &up, t0 + Duration::from_millis(40));
        let f: Vec<_> = s.forward(&up).collect();
        assert_eq!(f.iter().map(|e| e.kind).collect::<Vec<_>>(), [PointerKind::Motion, PRESS, RELEASE]);
        assert_eq!(f[1].position.y, 100.0, "basma ilk konumda");
        assert!(f.iter().all(PointerEvent::is_touch));
        assert_eq!(s.offset(t0), 0.0);
        let leave = t(PointerKind::Leave, 103.0);
        let _ = s.pointer(&theme, &leave, t0);
        assert_eq!(kinds(s.forward(&leave)), [PointerKind::Leave]);
    }

    #[test]
    fn touch_hold_forwards_press_after_tap_delay() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 100.0, t0);
        let early = t(PointerKind::Motion, 101.0);
        let _ = s.pointer(&theme, &early, t0 + Duration::from_millis(50));
        assert!(kinds(s.forward(&early)).is_empty());
        // Runtime'ın bekleme tiki (aynı konumda Motion).
        let tick = t(PointerKind::Motion, 101.0);
        let _ = s.pointer(&theme, &tick, t0 + TOUCH_TAP_DELAY);
        assert_eq!(kinds(s.forward(&tick)), [PointerKind::Motion, PRESS, PointerKind::Motion]);
        // Artık sürükleme çocuğundur: içerik kaymaz.
        let drag = t(PointerKind::Motion, 40.0);
        let _ = s.pointer(&theme, &drag, t0 + Duration::from_millis(200));
        assert_eq!(kinds(s.forward(&drag)), [PointerKind::Motion]);
        assert_eq!(s.offset(t0 + Duration::from_millis(200)), 0.0);
    }

    #[test]
    fn horizontal_touch_drag_goes_to_child() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 100.0, t0);
        let ev = PointerEvent::touch(Point::new(130.0, 102.0), PointerKind::Motion);
        let _ = s.pointer(&theme, &ev, t0 + Duration::from_millis(20));
        assert_eq!(kinds(s.forward(&ev)), [PointerKind::Motion, PRESS, PointerKind::Motion]);
        assert!(!s.is_panning());
    }

    #[test]
    fn vertical_touch_drag_pans_without_jump_and_flings() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 300.0, t0);
        let mut now = t0;
        let mut y = 300.0;
        for _ in 0..10 {
            now += Duration::from_millis(10);
            y -= 20.0;
            let ev = t(PointerKind::Motion, y);
            let r = s.pointer(&theme, &ev, now);
            assert!(r.event.is_some());
            assert!(kinds(s.forward(&ev)).is_empty(), "kaydırırken çocuk bir şey görmez");
        }
        // Parmak 200 px yukarı: eşik payı düşülmüş, içerik parmağı izler.
        assert!((s.offset(now) - (200.0 - TOUCH_SLOP)).abs() < 0.5, "{}", s.offset(now));
        // Parmak basılıyken sessizlik bırakma sayılmaz.
        s.advance(now + Duration::from_millis(70));
        assert!(s.is_panning() && matches!(s.motion, Motion::Touch { held: true, .. }));
        let up = t(RELEASE, y);
        let _ = s.pointer(&theme, &up, now + Duration::from_millis(5));
        assert!(kinds(s.forward(&up)).is_empty());
        assert!(matches!(s.motion, Motion::Fling { .. }), "{:?}", s.motion);
        let end = run(&mut s, now, 3.0);
        assert!(s.offset(end) > 300.0, "fırlatma devam etti: {}", s.offset(end));
    }

    #[test]
    fn touch_lift_after_pause_does_not_fling() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 300.0, t0);
        let _ = s.pointer(&theme, &t(PointerKind::Motion, 250.0), t0 + Duration::from_millis(10));
        let _ = s.pointer(&theme, &t(PointerKind::Motion, 200.0), t0 + Duration::from_millis(20));
        let x = s.offset(t0);
        let _ = s.pointer(&theme, &t(RELEASE, 200.0), t0 + Duration::from_millis(300));
        let end = run(&mut s, t0 + Duration::from_millis(300), 1.0);
        assert!((s.offset(end) - x).abs() < 0.5);
    }

    #[test]
    fn touch_pull_past_top_resists_and_springs_back() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 50.0, t0);
        let mut now = t0;
        for i in 1..=20 {
            now += Duration::from_millis(16);
            let _ = s.pointer(&theme, &t(PointerKind::Motion, 50.0 + 20.0 * i as f32), now);
        }
        let over = -s.offset(now);
        assert!(over > 0.0 && over < 400.0 * RUBBER_LIMIT, "lastik: {over}");
        let _ = s.pointer(&theme, &t(RELEASE, 450.0), now);
        let end = run(&mut s, now, 2.0);
        assert!(s.offset(end).abs() < 0.5);
    }

    #[test]
    fn touch_during_fling_catches_without_child_press() {
        let (mut s, theme, t0) = view();
        s.motion = Motion::Fling { start: t0, x0: 100.0, v0: 3000.0 };
        let now = t0 + Duration::from_millis(50);
        for k in [PointerKind::Enter, PointerKind::Motion, PRESS] {
            let ev = t(k, 200.0);
            let _ = s.pointer(&theme, &ev, now);
            assert!(kinds(s.forward(&ev)).is_empty());
        }
        let held = s.offset(now);
        let up = t(RELEASE, 200.0);
        let _ = s.pointer(&theme, &up, now + Duration::from_millis(200));
        assert!(kinds(s.forward(&up)).is_empty(), "yakalama dokunuşu eylem üretmez");
        let end = run(&mut s, now + Duration::from_millis(200), 1.0);
        assert!((s.offset(end) - held).abs() < 0.5, "durdu: {} vs {held}", s.offset(end));
    }

    #[test]
    fn touch_cancel_before_delivery_forwards_nothing_but_leave() {
        let (mut s, theme, t0) = view();
        down(&mut s, &theme, 100.0, t0);
        let leave = t(PointerKind::Leave, 100.0);
        let _ = s.pointer(&theme, &leave, t0);
        assert!(kinds(s.forward(&leave)).is_empty());
    }

    #[test]
    fn short_content_never_pans() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = ScrollView::new(&theme);
        s.set_geometry(Rect::new(0.0, 0.0, 300.0, 400.0), 200.0, t0);
        down(&mut s, &theme, 100.0, t0);
        let ev = t(PointerKind::Motion, 160.0);
        let _ = s.pointer(&theme, &ev, t0);
        assert_eq!(kinds(s.forward(&ev)), [PointerKind::Motion, PRESS, PointerKind::Motion]);
    }
}
