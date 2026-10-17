//! Tema önizleme şeritleri: [`CursorPreview`] (bir imleç temasının birkaç şekli yan yana) ve
//! [`IconThemePreview`] (bir ikon temasından temsilci ikonlar). Ayarlar'daki tema kartlarında
//! kullanılır; bileşen görsel çözmez — dokuları uygulama verir (nuxut-ui `nuxut-image` bilmez).
//!
//! **Görünüm (DESIGN tonal dil):** tek bir yuvarlatılmış "kuyu" (`surface` zemin — kart başka
//! tonda değilse [`with_background`](ThemePreview::with_background) ile değişir —, kart
//! `surface_container` üzerinde içe gömülü görünür, kenar çizgisi yok), içinde eşit aralıklı
//! yuvalar. Yüklenirken ve temada olmayan şekilde yuva yerinde küçük, soluk bir nokta durur
//! (statik — parıltı animasyonu yok). Görsel gelince `motion.effects` ile belirir ve
//! `spatial_fast` ile 0.92 → 1 büyür. İmleçler kendi piksel boyutunda (doku ÷ ölçek) çizilir,
//! ikonlar sabit boyutta (`Contain`).
//!
//! **Tembel yükleme:** bileşen ilk kez çizildiğinde görünen yuvaların aralığını
//! [`take_load_request`](ThemePreview::take_load_request) bir kez verir; genişlik büyüyüp yeni
//! yuva görünürse yalnızca yenileri istenir. Görünmeyen kart hiçbir şey yüklemez.
//!
//! **Boşta sıfır iş:** animasyonlu imleç (ör. `wait`) yalnızca işaretçi şeridin üzerindeyken
//! oynar ve bir sonraki kare zamanına [`Ui::wake_at`] ister (sürekli kare istemez); işaretçi
//! ayrılınca ilk kareye döner. Belirme geçişleri bitince kare istenmez.

use std::marker::PhantomData;
use std::ops::Range;
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{ImageFit, ImageStyle, RectStyle, Texture};
use nuxut_theme::{ColorRole, Theme};

use crate::input::{CursorShape, PointerEvent, PointerKind};
use crate::ui::{DamageState, Response, Ui};

/// En fazla yuva (veri şeridi taşırmasın).
pub const MAX_PREVIEW_SLOTS: usize = 16;
/// Belirme büyümesinin başlangıç ölçeği.
const APPEAR_FROM: f32 = 0.92;
/// Yer tutucu noktanın opaklığı (`on_surface_variant` üzerine).
const PLACEHOLDER_ALPHA: f32 = 0.18;
/// Yer tutucu noktanın çapı, öğe boyutuna oran.
const PLACEHOLDER_RATIO: f32 = 0.22;

/// Bir önizleme karesi (animasyonlu imleçte birden çok).
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewFrame {
    pub texture: Texture,
    /// Karenin süresi (tek karede anlamsız).
    pub delay: Duration,
}

impl PreviewFrame {
    pub fn still(texture: Texture) -> Self {
        Self { texture, delay: Duration::ZERO }
    }
}

/// Bir yuvanın durumu.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PreviewSlot {
    /// Henüz yüklenmedi (yer tutucu).
    #[default]
    Loading,
    /// Kareler (en az bir); boş vektör `Missing` sayılır.
    Ready(Vec<PreviewFrame>),
    /// Temada yok ya da çözülemedi (soluk yer tutucu kalır).
    Missing,
}

/// Önizleme türü (tür parametresi; davranış farkları buradadır).
pub trait PreviewKind {
    /// Öğenin varsayılan mantıksal boyutu.
    fn item_size(theme: &Theme) -> f32;
    /// `true`: doku kendi piksel boyutunda (÷ ölçek) çizilir (imleç); `false`: öğe boyutuna
    /// sığdırılır (ikon).
    const NATIVE_SIZE: bool;
}

/// İmleç önizlemesi türü.
#[derive(Debug, Clone, Copy)]
pub enum Cursors {}

/// İkon teması önizlemesi türü.
#[derive(Debug, Clone, Copy)]
pub enum Icons {}

impl PreviewKind for Cursors {
    /// 32 mantıksal piksel (`space(8)`): önizlemede şekil ayrıntısı seçilebilsin (gerçek
    /// boyut kullanıcının seçtiği imleç boyutudur; `with_item_size` ile verilebilir).
    fn item_size(theme: &Theme) -> f32 {
        theme.space(8)
    }
    const NATIVE_SIZE: bool = true;
}

impl PreviewKind for Icons {
    /// Uygulama ikonu boyutu: 32 mantıksal piksel (`space(8)`).
    fn item_size(theme: &Theme) -> f32 {
        theme.space(8)
    }
    const NATIVE_SIZE: bool = false;
}

/// Bir imleç temasının şekilleri yan yana.
pub type CursorPreview = ThemePreview<Cursors>;
/// Bir ikon temasından temsilci ikonlar.
pub type IconThemePreview = ThemePreview<Icons>;

#[derive(Debug, Clone)]
pub struct ThemePreview<K: PreviewKind> {
    slots: Vec<PreviewSlot>,
    appear: Vec<Animated<f32>>,
    item: f32,
    /// Kuyu zemini (varsayılan `surface`).
    background: ColorRole,
    rect: Rect,
    /// Yükleme istenmiş yuva sayısı (`0..requested`).
    requested: usize,
    /// Son çizimde görünen yuva sayısı.
    visible: usize,
    hover_since: Option<Instant>,
    damage: DamageState,
    _kind: PhantomData<K>,
}

impl<K: PreviewKind> ThemePreview<K> {
    /// `slots` yuva (en çok [`MAX_PREVIEW_SLOTS`]), hepsi `Loading`.
    pub fn new(theme: &Theme, slots: usize) -> Self {
        let n = slots.min(MAX_PREVIEW_SLOTS);
        Self {
            slots: vec![PreviewSlot::Loading; n],
            appear: (0..n).map(|_| Animated::new(0.0, theme.motion.effects)).collect(),
            item: K::item_size(theme),
            background: ColorRole::Surface,
            rect: Rect::ZERO,
            requested: 0,
            visible: 0,
            hover_since: None,
            damage: DamageState::new(),
            _kind: PhantomData,
        }
    }

    /// Öğe boyutu (mantıksal piksel; varsayılan [`PreviewKind::item_size`]). İmleçte bu,
    /// yüklenecek nominal boyuttur: [`load_px`](Self::load_px).
    pub fn with_item_size(mut self, logical: f32) -> Self {
        self.item = logical.max(1.0);
        self
    }

    /// Kuyu zemin rolü (varsayılan [`ColorRole::Surface`]). Kart `surface_container` değilse
    /// (ör. `surface` üzerinde liste satırı) kuyunun gömülü görünmesi için bir ton farklısı verilir.
    pub fn with_background(mut self, role: ColorRole) -> Self {
        self.background = role;
        self
    }

    pub fn background(&self) -> ColorRole {
        self.background
    }

    pub fn item_size(&self) -> f32 {
        self.item
    }

    /// Dokuların yükleneceği fiziksel piksel boyutu (`item × scale`, yuvarlanmış).
    pub fn load_px(&self, scale: f32) -> u32 {
        (self.item * scale.max(1.0)).round().max(1.0) as u32
    }

    /// Kuyunun iç boşluğu ve öğeler arası en küçük aralık.
    fn metrics(theme: &Theme) -> (f32, f32) {
        (theme.space(3), theme.space(3))
    }

    /// Tüm yuvaların sığdığı boyut.
    pub fn preferred_size(&self, theme: &Theme) -> Size {
        let (pad, gap) = Self::metrics(theme);
        let n = self.slots.len().max(1) as f32;
        Size::new(pad * 2.0 + n * self.item + (n - 1.0) * gap, self.preferred_height(theme))
    }

    pub fn preferred_height(&self, theme: &Theme) -> f32 {
        Self::metrics(theme).0 * 2.0 + self.item
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn slot(&self, i: usize) -> Option<&PreviewSlot> {
        self.slots.get(i)
    }

    /// Yuvayı günceller (sınır dışı indeks yok sayılır). `Ready` olan yuva belirme geçişiyle
    /// görünür; aynı değer tekrar verilirse iş yok.
    pub fn set_slot(&mut self, i: usize, slot: PreviewSlot, now: Instant) {
        let slot = match slot {
            PreviewSlot::Ready(f) if f.is_empty() => PreviewSlot::Missing,
            s => s,
        };
        let Some(cur) = self.slots.get_mut(i) else { return };
        if *cur == slot {
            return;
        }
        let ready = matches!(slot, PreviewSlot::Ready(_));
        *cur = slot;
        self.appear[i].set_target(if ready { 1.0 } else { 0.0 }, now);
        self.damage.mark();
    }

    /// Yuva sayısını değiştirir ve hepsini `Loading`'e döndürür (ör. kart başka temayı
    /// gösteriyor); bir sonraki çizimde görünenler yeniden istenir.
    pub fn reset(&mut self, theme: &Theme, slots: usize) {
        let n = slots.min(MAX_PREVIEW_SLOTS);
        self.slots = vec![PreviewSlot::Loading; n];
        self.appear = (0..n).map(|_| Animated::new(0.0, theme.motion.effects)).collect();
        self.requested = 0;
        self.visible = 0;
        self.damage.mark();
    }

    /// Yüklenmesi gereken yeni görünür yuvalar (bir kez verilir). Çizimden sonra çağrılır;
    /// henüz çizilmemiş ya da sıfır genişlikteki şerit hiçbir şey istemez.
    pub fn take_load_request(&mut self) -> Option<Range<usize>> {
        (self.visible > self.requested).then(|| {
            let r = self.requested..self.visible;
            self.requested = self.visible;
            r
        })
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for a in &mut self.appear {
            a.set_motion(theme.motion.effects, now);
        }
        self.damage.mark();
    }

    /// İşaretçi: üzerindeyken animasyonlu imleçler oynar. Olay üretmez.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response {
        let inside = self.rect.contains(event.position) && !matches!(event.kind, PointerKind::Leave);
        if inside == self.hover_since.is_some() {
            return Response::none();
        }
        self.hover_since = inside.then_some(now);
        let animated = self.slots.iter().any(|s| matches!(s, PreviewSlot::Ready(f) if f.len() > 1));
        if animated {
            self.damage.mark();
        }
        Response::none().with_repaint(animated)
    }

    /// Şerit tıklanabilir değil (kart seçimi kartın işidir): görüş yok.
    pub fn cursor(&self, _p: Point) -> Option<CursorShape> {
        None
    }

    /// Genişliğe sığan yuva sayısı.
    fn fit_count(&self, theme: &Theme) -> usize {
        let (pad, gap) = Self::metrics(theme);
        let avail = self.rect.width - pad * 2.0 + gap;
        if avail <= 0.0 {
            return 0;
        }
        ((avail / (self.item + gap)).floor() as usize).min(self.slots.len())
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let now = ui.now;
        let r = self.rect;
        if r.is_empty() {
            self.visible = 0;
            self.damage.report_hidden(ui);
            return;
        }
        let (pad, _) = Self::metrics(theme);
        let well_radius = (theme.shape.radius_medium - theme.space(1)).max(theme.shape.radius_small);
        ui.painter.rect(r, &RectStyle::filled(ui.fade(theme.colors.get(self.background)), Corners::uniform(well_radius)));
        let n = self.fit_count(theme);
        self.visible = n;
        let scale = ui.painter.scale().max(1.0);
        let mut animating = false;
        let mut next_wake: Option<Instant> = None;
        if n > 0 {
            // Eşit dağılım: yuva merkezleri iç alanı n eşit parçaya böler.
            let inner_w = r.width - pad * 2.0;
            let step = inner_w / n as f32;
            let cy = r.y + r.height * 0.5;
            for i in 0..n {
                let cx = r.x + pad + step * (i as f32 + 0.5);
                let k = self.appear[i].value(now).clamp(0.0, 1.0);
                animating |= !self.appear[i].is_settled(now);
                // Yer tutucu: görsel belirdikçe söner.
                if k < 1.0 {
                    let d = (self.item * PLACEHOLDER_RATIO).max(theme.space(1));
                    let dot = Rect::new(cx - d * 0.5, cy - d * 0.5, d, d);
                    let c = theme.colors.on_surface_variant.multiply_alpha(PLACEHOLDER_ALPHA * (1.0 - k));
                    ui.painter.rect(dot, &RectStyle::filled(ui.fade(c), Corners::uniform(d * 0.5)));
                }
                let PreviewSlot::Ready(frames) = &self.slots[i] else { continue };
                let (frame, wake) = current_frame(frames, self.hover_since, now);
                if let Some(w) = wake {
                    next_wake = Some(next_wake.map_or(w, |n| n.min(w)));
                }
                let tex = &frame.texture;
                if !tex.is_ready() {
                    animating = true;
                    continue;
                }
                let (tw, th) = tex.size();
                let (w, h) = if K::NATIVE_SIZE {
                    // Kendi boyutu, yuvaya sığacak kadar küçültülmüş.
                    let (w, h) = (tw as f32 / scale, th as f32 / scale);
                    let fit = (step.min(r.height - pad) / w.max(h)).min(1.0);
                    (w * fit, h * fit)
                } else {
                    (self.item, self.item)
                };
                let s = APPEAR_FROM + (1.0 - APPEAR_FROM) * k;
                let dst = Rect::new(cx - w * s * 0.5, cy - h * s * 0.5, w * s, h * s);
                let style = ImageStyle::new(ImageFit::Contain).with_opacity(ui.alpha() * k);
                ui.painter.image(dst, tex, &style);
            }
        }
        if let Some(w) = next_wake {
            ui.wake_at(w);
        }
        if animating {
            ui.request_frame();
        }
        // Animasyon karesi değişecekse bölge o an yeniden çizilir (wake_at ile uyanılır).
        if next_wake.is_some() {
            self.damage.mark();
        }
        self.damage.report(ui, r, animating);
    }
}

/// Animasyonun o anki karesi ve bir sonraki kare zamanı (yalnızca işaretçi üzerindeyken).
fn current_frame(frames: &[PreviewFrame], hover_since: Option<Instant>, now: Instant) -> (&PreviewFrame, Option<Instant>) {
    let Some(since) = hover_since.filter(|_| frames.len() > 1) else { return (&frames[0], None) };
    let (i, wake) = frame_at(frames.iter().map(|f| f.delay), since, now);
    (&frames[i.min(frames.len() - 1)], Some(wake))
}

/// Kare süreleri (her biri en az 10 ms) döngüsünde `now` anındaki kare ve onun bittiği an.
fn frame_at(delays: impl Iterator<Item = Duration> + Clone, since: Instant, now: Instant) -> (usize, Instant) {
    let floor = Duration::from_millis(10);
    let delays = delays.map(move |d| d.max(floor));
    let total = delays.clone().sum::<Duration>().as_nanos().max(1);
    let elapsed = now.saturating_duration_since(since).as_nanos();
    let cycle_start = elapsed / total * total;
    let mut boundary = cycle_start;
    for (i, d) in delays.enumerate() {
        boundary += d.as_nanos();
        if elapsed < boundary {
            return (i, since + Duration::from_nanos(u64::try_from(boundary).unwrap_or(u64::MAX)));
        }
    }
    (0, now + floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_request_follows_visibility() {
        let theme = Theme::default();
        let mut p = CursorPreview::new(&theme, 9);
        assert_eq!(p.take_load_request(), None, "çizilmeden istek yok");
        p.rect = Rect::new(0.0, 0.0, 200.0, 48.0);
        p.visible = p.fit_count(&theme);
        let first = p.take_load_request().unwrap();
        assert_eq!(first.start, 0);
        assert!(first.end > 0 && first.end < 9, "dar şerit: yalnızca görünenler ({first:?})");
        assert_eq!(p.take_load_request(), None, "bir kez");
        p.rect.width = 2000.0;
        p.visible = p.fit_count(&theme);
        assert_eq!(p.take_load_request(), Some(first.end..9), "yalnızca yeni görünenler");
        p.reset(&theme, 4);
        p.visible = p.fit_count(&theme);
        assert_eq!(p.take_load_request(), Some(0..4));
    }

    #[test]
    fn fits_preferred_size_exactly() {
        let theme = Theme::default();
        let mut p = IconThemePreview::new(&theme, 8);
        let s = p.preferred_size(&theme);
        p.rect = Rect::new(0.0, 0.0, s.width, s.height);
        assert_eq!(p.fit_count(&theme), 8);
        p.rect.width = s.width - 1.0;
        assert_eq!(p.fit_count(&theme), 7);
        p.rect.width = 0.0;
        assert_eq!(p.fit_count(&theme), 0);
        assert_eq!(p.load_px(1.5), 48);
        assert!(CursorPreview::new(&theme, 100).len() == MAX_PREVIEW_SLOTS);
    }

    #[test]
    fn background_role_defaults_to_surface() {
        let theme = Theme::default();
        assert_eq!(CursorPreview::new(&theme, 3).background(), ColorRole::Surface);
        let mut p = IconThemePreview::new(&theme, 3).with_background(ColorRole::SurfaceBright);
        assert_eq!(p.background(), ColorRole::SurfaceBright);
        // Yuvaları sıfırlamak zemini değiştirmez.
        p.reset(&theme, 5);
        assert_eq!(p.background(), ColorRole::SurfaceBright);
    }

    #[test]
    fn empty_ready_is_missing() {
        let theme = Theme::default();
        let mut p = CursorPreview::new(&theme, 2);
        let now = Instant::now();
        p.set_slot(0, PreviewSlot::Ready(Vec::new()), now);
        p.set_slot(5, PreviewSlot::Missing, now);
        assert_eq!(p.slot(0), Some(&PreviewSlot::Missing));
    }

    #[test]
    fn frame_timing_cycles_and_wakes_at_boundaries() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let d = [ms(100), ms(50), ms(0)];
        let at = |e: u64| frame_at(d.iter().copied(), t0, t0 + ms(e));
        assert_eq!(at(0), (0, t0 + ms(100)));
        assert_eq!(at(120), (1, t0 + ms(150)));
        assert_eq!(at(155), (2, t0 + ms(160)), "0 ms kare en az 10 ms");
        assert_eq!(at(160 + 30), (0, t0 + ms(260)), "döngü başa sarar");
    }

    #[test]
    fn hover_only_changes_state_on_edges() {
        let theme = Theme::default();
        let mut p = CursorPreview::new(&theme, 2);
        p.set_rect(Rect::new(0.0, 0.0, 200.0, 48.0));
        let now = Instant::now();
        let at = |x| PointerEvent::new(Point::new(x, 10.0), PointerKind::Motion);
        // Animasyonlu yuva yokken kare istenmez.
        assert!(!p.pointer(&at(10.0), now).repaint);
        assert!(p.hover_since.is_some());
        assert!(!p.pointer(&at(20.0), now).repaint);
        let _ = p.pointer(&PointerEvent::new(Point::new(20.0, 10.0), PointerKind::Leave), now);
        assert!(p.hover_since.is_none());
    }
}
