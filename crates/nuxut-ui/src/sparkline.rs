//! Küçük geçmiş grafiği: [`SampleHistory`] (sabit kapasiteli halka tampon) ve [`Sparkline`]
//! (çizgi + isteğe bağlı tabana doğru sönen dolgu).
//!
//! Yeni örnek sağdan girer; grafik bir örnek aralığı sola **kayar** (`motion.spatial_fast`,
//! kesilebilir: hızlı gelen örneklerde kayma hızını koruyarak birikir). Ölçek kendiliğinden
//! değişiyorsa ([`SparkScale::Auto`], [`SparkScale::AutoMax`]) eksen aralığı da aynı yayla akar,
//! çizgi sıçramaz. Renk [`Tint`]'tir; isteğe bağlı [`Threshold`] son örnek eşiği aşınca tüm
//! çizgiyi eşik rengine `motion.effects` ile geçirir.
//!
//! Sözleşme: örnek gelmezse kare yok; hareket bitince kare istenmez; hasar yalnızca kendi
//! dikdörtgeni (çizim ona kırpılır). Sonlu olmayan örnek (NaN, ±∞) **boşluktur**: çizgi orada
//! kesilir, dolgu atlanır, ölçek ve eşik onu yok sayar. Çizim karede bellek ayırmaz (nokta
//! tamponu yeniden kullanılır).

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_theme::Theme;

use crate::style::{SPARKLINE_FILL, SPARKLINE_STROKE};
use crate::tint::{Threshold, Tint, ToneState};
use crate::ui::{DamageState, Ui};

/// [`SampleHistory`] / [`Sparkline`] kapasite üst sınırı (dış girdi sınırı).
pub const MAX_SPARK_SAMPLES: usize = 1024;

/// Kayma birikimi sınırı (örnek aralığı): örnekler kaymadan çok hızlı gelirse (toplu itme)
/// fazlası animasyonsuz atlanır; grafik hiçbir zaman verinin gerisinde sürüklenmez.
const MAX_SCROLL_LAG: f32 = 2.0;

/// Sabit kapasiteli halka tampon: dolunca en eski örnek düşer. Bir kez ayrılır, büyümez.
/// Sonlu olmayan örnekler NaN olarak saklanır (boşluk).
#[derive(Debug, Clone)]
pub struct SampleHistory {
    buf: Vec<f32>,
    /// Sonraki yazılacak yer (tampon doluyken en eski örnek).
    head: usize,
    capacity: usize,
}

impl SampleHistory {
    /// `capacity` 1..=[`MAX_SPARK_SAMPLES`] aralığına kırpılır.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, MAX_SPARK_SAMPLES);
        Self { buf: Vec::with_capacity(capacity), head: 0, capacity }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.buf.len() == self.capacity
    }

    /// Örneği ekler; tampon doluysa düşen en eski örneği döner.
    pub fn push(&mut self, value: f32) -> Option<f32> {
        let value = if value.is_finite() { value } else { f32::NAN };
        if self.buf.len() < self.capacity {
            self.buf.push(value);
            return None;
        }
        let old = std::mem::replace(&mut self.buf[self.head], value);
        self.head = (self.head + 1) % self.capacity;
        Some(old)
    }

    /// `i`. örnek, 0 = en eski.
    pub fn get(&self, i: usize) -> Option<f32> {
        (i < self.buf.len()).then(|| self.buf[(self.head + i) % self.buf.len()])
    }

    /// En yeni örnek.
    pub fn latest(&self) -> Option<f32> {
        self.len().checked_sub(1).and_then(|i| self.get(i))
    }

    /// Eskiden yeniye.
    pub fn iter(&self) -> impl Iterator<Item = f32> + '_ {
        let (newer, older) = self.buf.split_at(self.head.min(self.buf.len()));
        older.iter().chain(newer).copied()
    }

    /// Sonlu örneklerin (en küçük, en büyük); yoksa `None`.
    pub fn range(&self) -> Option<(f32, f32)> {
        self.buf.iter().filter(|v| v.is_finite()).fold(None, |acc, &v| match acc {
            None => Some((v, v)),
            Some((lo, hi)) => Some((lo.min(v), hi.max(v))),
        })
    }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.head = 0;
    }
}

/// Dikey eksen. Değerler aralığa kırpılır (aşan örnek kenarda düz çizilir).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SparkScale {
    /// Sabit aralık (ör. yüzde: 0..100). Geçersizse (max ≤ min, sonlu değil) 0..1.
    Fixed { min: f32, max: f32 },
    /// Alt sınır sabit, üst sınır görünen örneklerin en büyüğü ama en az `min_max` (ağ hızı:
    /// `min = 0`, boşta çizgi tabana yapışık kalsın diye `min_max` > 0).
    AutoMax { min: f32, min_max: f32 },
    /// İki sınır da görünen örneklerden; aralık en az `min_span` (ortadan genişler; sıcaklık).
    Auto { min_span: f32 },
}

impl SparkScale {
    /// 0..1 (varsayılan).
    pub const UNIT: Self = Self::Fixed { min: 0.0, max: 1.0 };

    /// Görünen örnekler için (alt, üst); her zaman sonlu ve üst > alt.
    pub fn range(&self, data: Option<(f32, f32)>) -> (f32, f32) {
        let finite = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let (lo, hi) = match *self {
            Self::Fixed { min, max } => (min, max),
            Self::AutoMax { min, min_max } => {
                let min = finite(min, 0.0);
                (min, data.map_or(min_max, |(_, hi)| hi.max(min_max)))
            }
            Self::Auto { min_span } => {
                let span = finite(min_span, 0.0).max(0.0);
                match data {
                    Some((lo, hi)) if hi - lo >= span => (lo, hi),
                    Some((lo, hi)) => {
                        let mid = (lo + hi) * 0.5;
                        (mid - span * 0.5, mid + span * 0.5)
                    }
                    // Veri yok: tabandan en az bir birim (sıfır aralık eksen sıçratırdı).
                    None => (0.0, span.max(1.0)),
                }
            }
        };
        if lo.is_finite() && hi.is_finite() && hi > lo {
            (lo, hi)
        } else if lo.is_finite() && hi.is_finite() && hi == lo {
            (lo - 0.5, hi + 0.5)
        } else {
            (0.0, 1.0)
        }
    }
}

impl Default for SparkScale {
    fn default() -> Self {
        Self::UNIT
    }
}

/// Küçük geçmiş grafiği (bkz. modül belgesi).
#[derive(Debug, Clone)]
pub struct Sparkline {
    history: SampleHistory,
    /// Tampon doluyken son düşen örnek: kayma sürerken soldan çıkarken çizilir.
    evicted: Option<f32>,
    scale: SparkScale,
    /// Eksen aralığı (x = alt, y = üst), `spatial_fast`.
    range: Animated<Point>,
    /// Toplam kayma (örnek aralığı); hedef her itmede +1. Görünen kayma = hedef − değer.
    scroll: Animated<f32>,
    tone: ToneState,
    fill: bool,
    rect: Rect,
    damage: DamageState,
    /// Çizim tamponu (karede ayırma yok).
    points: Vec<Point>,
}

impl Sparkline {
    /// `capacity`: görünen örnek sayısı (1..=[`MAX_SPARK_SAMPLES`]; ör. 60 = 1 Hz'de bir dakika).
    pub fn new(theme: &Theme, capacity: usize) -> Self {
        let history = SampleHistory::new(capacity);
        let scale = SparkScale::default();
        let (lo, hi) = scale.range(None);
        Self {
            points: Vec::with_capacity(history.capacity() + 1),
            history,
            evicted: None,
            scale,
            range: Animated::new(Point::new(lo, hi), theme.motion.spatial_fast),
            scroll: Animated::new(0.0, theme.motion.spatial_fast),
            tone: ToneState::new(theme),
            fill: true,
            rect: Rect::ZERO,
            damage: DamageState::new(),
        }
    }

    pub fn with_scale(mut self, scale: SparkScale) -> Self {
        self.scale = scale;
        let (lo, hi) = self.target_range();
        self.range.jump_to(Point::new(lo, hi));
        self
    }

    /// Çizgi altındaki dolgu (varsayılan açık).
    pub fn with_fill(mut self, fill: bool) -> Self {
        self.fill = fill;
        self
    }

    /// Renk (varsayılan `primary`).
    pub fn with_tint(mut self, tint: impl Into<Tint>) -> Self {
        self.tone.set_base(tint.into());
        self
    }

    pub fn with_threshold(mut self, threshold: Threshold) -> Self {
        let v = self.latest_value();
        self.tone.set_threshold(Some(threshold), v, Instant::now());
        self.tone.settle();
        self
    }

    pub fn history(&self) -> &SampleHistory {
        &self.history
    }

    pub fn scale(&self) -> SparkScale {
        self.scale
    }

    pub fn set_scale(&mut self, scale: SparkScale, now: Instant) {
        if self.scale != scale {
            self.scale = scale;
            self.retarget_range(now);
            self.damage.mark();
        }
    }

    pub fn set_fill(&mut self, fill: bool) {
        if self.fill != fill {
            self.fill = fill;
            self.damage.mark();
        }
    }

    pub fn set_tint(&mut self, tint: impl Into<Tint>) {
        if self.tone.set_base(tint.into()) {
            self.damage.mark();
        }
    }

    pub fn set_threshold(&mut self, threshold: Option<Threshold>, now: Instant) {
        let v = self.latest_value();
        if self.tone.set_threshold(threshold, v, now) {
            self.damage.mark();
        }
    }

    /// Son örnek eşiği aşmış mı (eşik rengi hedefte).
    pub fn is_alert(&self) -> bool {
        self.tone.is_alert()
    }

    fn latest_value(&self) -> f32 {
        self.history.latest().unwrap_or(f32::NAN)
    }

    fn target_range(&self) -> (f32, f32) {
        self.scale.range(self.history.range())
    }

    fn retarget_range(&mut self, now: Instant) {
        let (lo, hi) = self.target_range();
        self.range.set_target(Point::new(lo, hi), now);
    }

    /// Yeni örnek (sağdan girer, grafik kayar). Sonlu olmayan değer boşluktur.
    pub fn push(&mut self, value: f32, now: Instant) {
        let had_data = self.history.range().is_some();
        let evicted = self.history.push(value);
        if evicted.is_some() {
            self.evicted = evicted;
        }
        // Kayma: hedef bir aralık ileri; konum ve hız korunur (kesilebilir). Birikim sınırı aşılırsa atla.
        let target = self.scroll.target() + 1.0;
        if target - self.scroll.value(now) > MAX_SCROLL_LAG {
            self.scroll.jump_to(target);
        } else {
            self.scroll.set_target(target, now);
        }
        let (lo, hi) = self.target_range();
        if had_data {
            self.range.set_target(Point::new(lo, hi), now);
        } else {
            // İlk veri: eksen boştan akmaz, doğrudan oturur.
            self.range.jump_to(Point::new(lo, hi));
        }
        self.tone.update(self.latest_value(), now);
        self.damage.mark();
    }

    /// Tüm geçmişi animasyonsuz değiştirir (panel açılışında saklı geçmiş); kapasiteyi aşan
    /// baştaki örnekler düşer.
    pub fn set_samples(&mut self, samples: impl IntoIterator<Item = f32>) {
        self.history.clear();
        for v in samples {
            self.history.push(v);
        }
        self.evicted = None;
        self.scroll.jump_to(0.0);
        let (lo, hi) = self.target_range();
        self.range.jump_to(Point::new(lo, hi));
        let v = self.latest_value();
        self.tone.update(v, Instant::now());
        self.tone.settle();
        self.damage.mark();
    }

    /// Geçmişi boşaltır (animasyonsuz).
    pub fn clear(&mut self) {
        self.set_samples(std::iter::empty());
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.range.set_motion(theme.motion.spatial_fast, now);
        self.scroll.set_motion(theme.motion.spatial_fast, now);
        self.tone.set_theme(theme, now);
        self.damage.mark();
    }

    /// Yükseklik `space(10)`.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(10))
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

    /// Kayma, eksen ya da renk hâlâ canlanıyor mu.
    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.scroll.is_settled(now) || !self.range.is_settled(now) || self.tone.is_animating(now)
    }

    /// Görünen kayma (örnek aralığı; 0 = yerinde).
    fn scroll_offset(&self, now: Instant) -> f32 {
        (self.scroll.target() - self.scroll.value(now)).clamp(-1.0, MAX_SCROLL_LAG)
    }

    /// Örneklerin çizim noktaları (`points`); x sağa hizalı, y eksen aralığına göre. Sonlu
    /// olmayan örnek y = NaN (boşluk).
    fn layout_points(&mut self, now: Instant) {
        self.points.clear();
        let r = self.rect;
        let cap = self.history.capacity();
        let step = if cap > 1 { r.width / (cap - 1) as f32 } else { 0.0 };
        let offset = self.scroll_offset(now);
        let range = self.range.value(now);
        let (lo, span) = (range.x, (range.y - range.x).max(f32::EPSILON));
        let inset = SPARKLINE_STROKE * 0.5;
        let (top, bottom) = (r.y + inset, r.bottom() - inset);
        let h = (bottom - top).max(0.0);
        let y = |v: f32| if v.is_finite() { bottom - ((v - lo) / span).clamp(0.0, 1.0) * h } else { f32::NAN };
        let len = self.history.len();
        // En yeni örnek sağ kenarda (kayma sürerken bir aralık sağında, kırpmanın dışından girer).
        let x_of = |i: f32| r.right() - (len as f32 - 1.0 - i) * step + offset * step;
        if offset > 0.0
            && self.history.is_full()
            && let Some(e) = self.evicted
        {
            self.points.push(Point::new(x_of(-1.0), y(e)));
        }
        for (i, v) in self.history.iter().enumerate() {
            self.points.push(Point::new(x_of(i as f32), y(v)));
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let r = self.rect;
        if self.scroll.is_settled(now) && self.scroll.target() != 0.0 {
            // Oturdu: sayaç sıfırlanır (görünen kayma zaten 0; f32 hassasiyeti uzun oturumda korunur).
            self.scroll.jump_to(0.0);
            self.evicted = None;
        }
        let animating = self.is_animating(now);
        if animating {
            ui.request_frame();
        }
        if r.width <= 0.0 || r.height <= 0.0 {
            self.damage.report_hidden(ui);
            return;
        }
        if !self.history.is_empty() {
            self.layout_points(now);
            let color = ui.fade(self.tone.color(theme, now));
            ui.painter.push_clip(r);
            if self.fill {
                ui.painter.area(&self.points, r.bottom(), color.multiply_alpha(SPARKLINE_FILL), 0.0);
            }
            // Boşluklarla bölünmüş sürekli parçalar.
            for run in self.points.split(|p| !p.y.is_finite()).filter(|run| !run.is_empty()) {
                ui.painter.polyline(run, SPARKLINE_STROKE, color);
            }
            ui.painter.pop_clip();
        }
        self.damage.report(ui, r, animating);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use nuxut_render::{DrawKind, Recorder, TextSystem};

    use super::*;

    #[test]
    fn ring_buffer_keeps_newest_in_order() {
        let mut h = SampleHistory::new(3);
        assert!(h.is_empty());
        assert_eq!(h.latest(), None);
        assert_eq!(h.push(1.0), None);
        assert_eq!(h.push(2.0), None);
        assert_eq!(h.push(3.0), None);
        assert!(h.is_full());
        assert_eq!(h.push(4.0), Some(1.0), "en eski düşer");
        assert_eq!(h.push(5.0), Some(2.0));
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![3.0, 4.0, 5.0]);
        assert_eq!(h.iter().count(), 3);
        assert_eq!((h.get(0), h.get(2), h.get(3)), (Some(3.0), Some(5.0), None));
        assert_eq!(h.latest(), Some(5.0));
        assert_eq!(h.range(), Some((3.0, 5.0)));
        // Kapasite tampondan büyümez.
        for i in 0..100 {
            h.push(i as f32);
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.iter().collect::<Vec<_>>(), vec![97.0, 98.0, 99.0]);
        h.clear();
        assert!(h.is_empty());
        assert_eq!(SampleHistory::new(0).capacity(), 1);
        assert_eq!(SampleHistory::new(usize::MAX).capacity(), MAX_SPARK_SAMPLES);
    }

    #[test]
    fn non_finite_samples_are_gaps() {
        let mut h = SampleHistory::new(4);
        h.push(f32::NAN);
        assert_eq!(h.range(), None, "yalnız boşluk: aralık yok");
        h.push(f32::INFINITY);
        h.push(0.5);
        h.push(f32::NEG_INFINITY);
        assert!(h.iter().filter(|v| v.is_nan()).count() == 3, "±∞ NaN olarak saklanır");
        assert_eq!(h.range(), Some((0.5, 0.5)));
    }

    #[test]
    fn scales_are_finite_and_ordered() {
        let data = Some((10.0, 30.0));
        assert_eq!(SparkScale::UNIT.range(data), (0.0, 1.0));
        assert_eq!(SparkScale::Fixed { min: 0.0, max: 100.0 }.range(data), (0.0, 100.0));
        assert_eq!(SparkScale::Fixed { min: 5.0, max: 1.0 }.range(data), (0.0, 1.0), "ters aralık");
        assert_eq!(SparkScale::Fixed { min: f32::NAN, max: 1.0 }.range(data), (0.0, 1.0));
        assert_eq!(SparkScale::AutoMax { min: 0.0, min_max: 1.0 }.range(data), (0.0, 30.0));
        assert_eq!(SparkScale::AutoMax { min: 0.0, min_max: 50.0 }.range(data), (0.0, 50.0));
        assert_eq!(SparkScale::AutoMax { min: 0.0, min_max: 1.0 }.range(None), (0.0, 1.0));
        assert_eq!(SparkScale::Auto { min_span: 0.0 }.range(data), (10.0, 30.0));
        assert_eq!(SparkScale::Auto { min_span: 40.0 }.range(data), (0.0, 40.0), "ortadan genişler");
        assert_eq!(SparkScale::Auto { min_span: 0.0 }.range(Some((7.0, 7.0))), (6.5, 7.5), "tek değer");
        assert_eq!(SparkScale::Auto { min_span: f32::NAN }.range(None), (0.0, 1.0));
        assert_eq!(SparkScale::AutoMax { min: 0.0, min_max: f32::INFINITY }.range(data), (0.0, 1.0));
    }

    const RECT: Rect = Rect::new(10.0, 10.0, 118.0, 40.0);

    /// Bir kare: (kare istendi mi, hasar bildirildi mi, çizgi/alan sayısı).
    fn frame(s: &mut Sparkline, theme: &Theme, now: Instant) -> (bool, usize, usize) {
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        let mut wants = false;
        rec.record(&mut text, (160, 80), 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            s.draw(&mut ui);
            wants = ui.wants_frame();
        });
        let d = rec.draws();
        let strokes = d.iter().filter(|d| d.kind == DrawKind::Stroke).count();
        let areas = d.iter().filter(|d| d.kind == DrawKind::Area).count();
        for d in &d {
            assert!(d.rect.x >= -2.0 && d.rect.width.is_finite(), "{d:?}");
            assert!(d.clip.is_some(), "çizim kendi dikdörtgenine kırpılı");
        }
        (wants, strokes, areas)
    }

    fn line(theme: &Theme) -> Sparkline {
        let mut s = Sparkline::new(theme, 60);
        s.set_rect(RECT);
        s
    }

    #[test]
    fn no_samples_no_frames() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = line(&theme);
        let (wants, strokes, areas) = frame(&mut s, &theme, t0);
        assert!(!wants, "örneksiz kare istenmez");
        assert_eq!((strokes, areas), (0, 0));
        assert!(!frame(&mut s, &theme, t0 + Duration::from_secs(5)).0);
    }

    #[test]
    fn push_slides_then_settles_without_frames() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = line(&theme);
        s.set_samples((0..60).map(|i| (i % 7) as f32 / 7.0));
        assert!(!frame(&mut s, &theme, t0).0, "toplu geçmiş animasyonsuz");
        s.push(0.9, t0);
        let (wants, strokes, areas) = frame(&mut s, &theme, t0 + Duration::from_millis(16));
        assert!(wants, "kayma sürerken kare");
        // Kayarken düşen örnek de çizilir: 61 nokta → 60 parça.
        assert_eq!(strokes, 60);
        assert_eq!(areas, 60);
        assert!(s.is_animating(t0 + Duration::from_millis(16)));
        let late = t0 + Duration::from_secs(3);
        assert!(!frame(&mut s, &theme, late).0, "oturunca kare yok");
        assert!(!frame(&mut s, &theme, late + Duration::from_secs(10)).0, "örnek gelmezse kare yok");
        assert_eq!(frame(&mut s, &theme, late).1, 59, "yerinde: 60 nokta");
    }

    #[test]
    fn fast_pushes_accumulate_but_stay_bounded() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = line(&theme);
        s.set_samples([0.5; 60]);
        for i in 0..20 {
            s.push(0.5, t0 + Duration::from_millis(i * 5));
        }
        let now = t0 + Duration::from_millis(100);
        let off = s.scroll_offset(now);
        assert!((0.0..=MAX_SCROLL_LAG).contains(&off), "kayma sınırlı: {off}");
    }

    #[test]
    fn instant_motion_never_requests_frames() {
        let (theme, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        let t0 = Instant::now();
        let mut s = line(&theme).with_scale(SparkScale::Auto { min_span: 0.0 });
        s.push(1.0, t0);
        s.push(5.0, t0);
        assert!(!frame(&mut s, &theme, t0).0);
    }

    #[test]
    fn nan_breaks_line_and_is_safe() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = line(&theme).with_scale(SparkScale::Auto { min_span: 0.0 });
        s.set_samples([0.1, 0.2, f32::NAN, 0.4, 0.5, f32::INFINITY, f32::NEG_INFINITY]);
        let (wants, strokes, areas) = frame(&mut s, &theme, t0);
        assert!(!wants);
        // Parçalar: [0.1, 0.2] → 1, [0.4, 0.5] → 1. Alan da boşlukta atlanır.
        assert_eq!(strokes, 2);
        assert_eq!(areas, 2);
        // Yalnız boşluk: çizim yok, panik yok.
        s.set_samples([f32::NAN; 3]);
        assert_eq!(frame(&mut s, &theme, t0).1, 0);
        s.push(f32::NAN, t0);
        let _ = frame(&mut s, &theme, t0 + Duration::from_millis(10));
    }

    #[test]
    fn threshold_switches_color_with_transition() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = line(&theme).with_threshold(Threshold::new(0.9));
        s.set_samples([0.2; 10]);
        assert!(!s.is_alert());
        s.push(0.95, t0);
        assert!(s.is_alert());
        let mid = t0 + Duration::from_millis(50);
        assert!(frame(&mut s, &theme, mid).0);
        let c = s.tone.color(&theme, mid);
        assert_ne!(c, theme.colors.primary);
        assert_eq!(s.tone.color(&theme, t0 + Duration::from_secs(3)), theme.colors.error);
        // Eşik altında ama geri dönüş payında: kırmızı kalır.
        s.push(0.88, t0 + Duration::from_secs(3));
        assert!(s.is_alert());
        s.push(0.5, t0 + Duration::from_secs(4));
        assert!(!s.is_alert());
        // NaN durumu değiştirmez.
        s.push(0.95, t0 + Duration::from_secs(5));
        s.push(f32::NAN, t0 + Duration::from_secs(6));
        assert!(s.is_alert());
    }

    #[test]
    fn auto_range_flows_instead_of_jumping() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut s = line(&theme).with_scale(SparkScale::AutoMax { min: 0.0, min_max: 1.0 });
        s.set_samples([1.0; 10]);
        assert_eq!(s.range.value(t0), Point::new(0.0, 1.0));
        s.push(10.0, t0);
        let mid = s.range.value(t0 + Duration::from_millis(40)).y;
        assert!(mid > 1.0 && mid < 10.0, "eksen akar: {mid}");
        assert_eq!(s.range.target(), Point::new(0.0, 10.0));
    }
}
