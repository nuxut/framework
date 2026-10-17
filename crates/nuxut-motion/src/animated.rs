use std::time::Instant;

use nuxut_core::{Color, Corners, Insets, Point, Rect};

use crate::motion::Motion;

/// Yay ile canlandırılabilen değer: en fazla 4 bağımsız bileşen.
pub trait Animatable: Copy + PartialEq {
    const LEN: usize;
    /// Bu sapmanın altında hareket görünmez; değer hedefe yapıştırılır (hız eşiği bunun
    /// [`REST_VELOCITY_RATIO`] katı, birim/saniye). Varsayılan renk/opaklık/oran ölçeğinde
    /// (0..1; 1/255'in yarısı). Konum/boyut türleri mantıksal piksel eşiği verir.
    const REST: f64 = REST_DISPLACEMENT;
    fn to_components(&self) -> [f32; 4];
    fn from_components(c: [f32; 4]) -> Self;
}

impl Animatable for f32 {
    const LEN: usize = 1;
    fn to_components(&self) -> [f32; 4] {
        [*self, 0.0, 0.0, 0.0]
    }
    fn from_components(c: [f32; 4]) -> Self {
        c[0]
    }
}

impl Animatable for Point {
    const LEN: usize = 2;
    const REST: f64 = GEOMETRY_REST;
    fn to_components(&self) -> [f32; 4] {
        [self.x, self.y, 0.0, 0.0]
    }
    fn from_components(c: [f32; 4]) -> Self {
        Point::new(c[0], c[1])
    }
}

impl Animatable for Rect {
    const LEN: usize = 4;
    const REST: f64 = GEOMETRY_REST;
    fn to_components(&self) -> [f32; 4] {
        [self.x, self.y, self.width, self.height]
    }
    fn from_components(c: [f32; 4]) -> Self {
        Rect::new(c[0], c[1], c[2], c[3])
    }
}

impl Animatable for Insets {
    const LEN: usize = 4;
    const REST: f64 = GEOMETRY_REST;
    fn to_components(&self) -> [f32; 4] {
        [self.top, self.right, self.bottom, self.left]
    }
    fn from_components(c: [f32; 4]) -> Self {
        Insets::new(c[0], c[1], c[2], c[3])
    }
}

impl Animatable for Corners {
    const LEN: usize = 4;
    const REST: f64 = GEOMETRY_REST;
    fn to_components(&self) -> [f32; 4] {
        self.to_array()
    }
    fn from_components(c: [f32; 4]) -> Self {
        Corners::new(c[0], c[1], c[2], c[3])
    }
}

impl Animatable for Color {
    const LEN: usize = 4;
    fn to_components(&self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }
    fn from_components(c: [f32; 4]) -> Self {
        Color::rgba(c[0], c[1], c[2], c[3])
    }
}

/// Bir hedefe yay ile giden değer.
///
/// Zaman dışarıdan verilir (`now`), çünkü bir karedeki tüm animasyonlar aynı kare saatini
/// görmelidir. Animasyon bitince [`is_settled`](Self::is_settled) `true` döner; çağıran
/// taraf o zaman kare istemeyi bırakır (boşta sıfır CPU).
#[derive(Debug, Clone)]
pub struct Animated<T: Animatable> {
    target: T,
    motion: Motion,
    start: Instant,
    x0: [f64; 4],
    v0: [f64; 4],
    settled: bool,
}

/// Varsayılan dinlenme eşiği: 0..1 ölçekli değerler (renk bileşeni, opaklık, oran) için 1/255'in
/// yarısı — bunun altındaki fark 8 bitlik çıktıda görünmez.
const REST_DISPLACEMENT: f64 = 0.002;
/// Konum/boyut türleri (mantıksal piksel): 1/20 px. Yayın kuyruğunda bu sapmanın altındaki
/// hareket yalnızca kenar yumuşatmasını < %5 oynatır; eskiden 0.002 px'e dek kare çiziliyordu ve
/// varsayılan `spatial` yayında (0.38 s, ζ 0.87) ~700 px'lik bir panel geçişi ~0.89 s yerine
/// ~0.7 s'de oturur (geçiş başına ~11 kare; ölçüm: shell docs/research/perf-2026-10.md).
const GEOMETRY_REST: f64 = 0.05;
/// Hız eşiği = sapma eşiği × bu oran (birim/saniye). Oturmaya yakın yayda |v| ≈ ω·|x|; oran
/// küçük kalırsa hız koşulu sapma eşiğini etkisizleştirir.
const REST_VELOCITY_RATIO: f64 = 10.0;

impl<T: Animatable> Animated<T> {
    /// `motion`: [`Spring`](crate::Spring), [`Curve`](crate::Curve) ya da tema token'ı
    /// ([`Motion`]).
    pub fn new(value: T, motion: impl Into<Motion>) -> Self {
        Self {
            target: value,
            motion: motion.into(),
            start: Instant::now(),
            x0: [0.0; 4],
            v0: [0.0; 4],
            settled: true,
        }
    }

    pub fn target(&self) -> T {
        self.target
    }

    pub fn motion(&self) -> Motion {
        self.motion
    }

    /// [`motion`](Self::motion) ile aynı (geriye uyumluluk; artık yay olmayabilir).
    pub fn spring(&self) -> Motion {
        self.motion
    }

    /// Hareket ayarı değişir (ör. config hot-reload); devam eden hareket mevcut konum ve
    /// hızdan yeni ayarla sürer.
    pub fn set_motion(&mut self, motion: impl Into<Motion>, now: Instant) {
        let motion = motion.into();
        if motion != self.motion {
            self.rebase(now);
            self.motion = motion;
            if motion.is_instant() {
                self.jump_to(self.target);
            }
        }
    }

    /// [`set_motion`](Self::set_motion) ile aynı (geriye uyumluluk).
    pub fn set_spring(&mut self, motion: impl Into<Motion>, now: Instant) {
        self.set_motion(motion, now);
    }

    /// Yeni hedef. Mevcut konum ve hız korunur: kesintisiz, momentumlu geçiş.
    pub fn set_target(&mut self, target: T, now: Instant) {
        if target == self.target {
            return;
        }
        let current = self.components_at(now);
        let velocity = self.velocity_components(now);
        let t = target.to_components();
        for i in 0..T::LEN {
            self.x0[i] = current[i] - f64::from(t[i]);
            self.v0[i] = velocity[i];
        }
        self.target = target;
        self.start = now;
        self.settled = self.motion.is_instant();
    }

    /// Animasyonsuz atlama.
    pub fn jump_to(&mut self, value: T) {
        self.target = value;
        self.x0 = [0.0; 4];
        self.v0 = [0.0; 4];
        self.settled = true;
    }

    /// Hedefe dokunmadan anlık bir itme ekler (ör. tıklamada küçük bir "zıplama").
    pub fn kick(&mut self, velocity: T, now: Instant) {
        self.rebase(now);
        let k = velocity.to_components();
        for (v, k) in self.v0.iter_mut().zip(k).take(T::LEN) {
            *v += f64::from(k);
        }
        self.settled = self.motion.is_instant();
    }

    pub fn value(&self, now: Instant) -> T {
        let c = self.components_at(now);
        T::from_components([c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32])
    }

    pub fn velocity(&self, now: Instant) -> T {
        let v = self.velocity_components(now);
        T::from_components([v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32])
    }

    /// Hareket bitmiş mi? Bitmişse bir sonraki çağrıda dahi değer tam hedeftir.
    pub fn is_settled(&mut self, now: Instant) -> bool {
        if self.settled {
            return true;
        }
        let t = self.elapsed(now);
        let resting = (0..T::LEN).all(|i| {
            let s = self.motion.evaluate(self.x0[i], self.v0[i], t);
            s.x.abs() < T::REST && s.v.abs() < T::REST * REST_VELOCITY_RATIO
        });
        if resting {
            self.jump_to(self.target);
        }
        resting
    }

    fn rebase(&mut self, now: Instant) {
        let current = self.components_at(now);
        let velocity = self.velocity_components(now);
        let t = self.target.to_components();
        for i in 0..T::LEN {
            self.x0[i] = current[i] - f64::from(t[i]);
            self.v0[i] = velocity[i];
        }
        self.start = now;
    }

    fn elapsed(&self, now: Instant) -> f64 {
        now.saturating_duration_since(self.start).as_secs_f64()
    }

    fn components_at(&self, now: Instant) -> [f64; 4] {
        let t = self.target.to_components();
        let mut out = [0.0; 4];
        if self.settled {
            for i in 0..T::LEN {
                out[i] = f64::from(t[i]);
            }
            return out;
        }
        let elapsed = self.elapsed(now);
        for i in 0..T::LEN {
            out[i] = f64::from(t[i]) + self.motion.evaluate(self.x0[i], self.v0[i], elapsed).x;
        }
        out
    }

    fn velocity_components(&self, now: Instant) -> [f64; 4] {
        let mut out = [0.0; 4];
        if self.settled {
            return out;
        }
        let elapsed = self.elapsed(now);
        for (i, o) in out.iter_mut().enumerate().take(T::LEN) {
            *o = self.motion.evaluate(self.x0[i], self.v0[i], elapsed).v;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{CubicBezier, Curve, Spring};

    #[test]
    fn curve_reaches_target_exactly_at_duration() {
        let t0 = Instant::now();
        let mut a = Animated::new(0.0f32, Curve::new(CubicBezier::EASE, 0.25));
        a.set_target(100.0, t0);
        let mid = a.value(t0 + Duration::from_millis(125));
        assert!(mid > 0.0 && mid < 100.0);
        // Eğri hedefi aşmaz.
        let max = (0..30).map(|i| a.value(t0 + Duration::from_millis(i * 10))).fold(0.0, f32::max);
        assert!(max <= 100.0 + 1e-3);
        let end = t0 + Duration::from_millis(250);
        assert!(a.is_settled(end));
        assert_eq!(a.value(end), 100.0);
    }

    #[test]
    fn interrupted_curve_has_no_jump() {
        let t0 = Instant::now();
        let mut a = Animated::new(0.0f32, Curve::new(CubicBezier::STANDARD, 0.4));
        a.set_target(100.0, t0);
        let mid = t0 + Duration::from_millis(120);
        let (p, v) = (a.value(mid), a.velocity(mid));
        a.set_target(-30.0, mid);
        assert!((a.value(mid) - p).abs() < 1e-3, "konum sıçramamalı");
        assert!((a.velocity(mid) - v).abs() < 0.5, "hız korunmalı: {v} → {}", a.velocity(mid));
        assert!(a.is_settled(mid + Duration::from_millis(400)));
    }

    #[test]
    fn switching_motion_style_keeps_position() {
        let t0 = Instant::now();
        let mut a = Animated::new(0.0f32, Spring::new(0.4, 0.7));
        a.set_target(100.0, t0);
        let mid = t0 + Duration::from_millis(100);
        let p = a.value(mid);
        a.set_motion(Curve::new(CubicBezier::EASE, 0.3), mid);
        assert!((a.value(mid) - p).abs() < 1e-3);
        assert!(a.is_settled(mid + Duration::from_millis(300)));
    }

    #[test]
    fn reaches_target_and_settles() {
        let t0 = Instant::now();
        let mut a = Animated::new(0.0f32, Spring::new(0.3, 0.8));
        a.set_target(100.0, t0);
        assert!(!a.is_settled(t0));
        let later = t0 + Duration::from_secs(3);
        assert!(a.is_settled(later));
        assert_eq!(a.value(later), 100.0);
    }

    #[test]
    fn retarget_preserves_position_and_velocity() {
        let t0 = Instant::now();
        let mut a = Animated::new(0.0f32, Spring::new(0.4, 0.7));
        a.set_target(100.0, t0);
        let mid = t0 + Duration::from_millis(80);
        let (p, v) = (a.value(mid), a.velocity(mid));
        a.set_target(-50.0, mid);
        assert!((a.value(mid) - p).abs() < 1e-3, "konum sıçramamalı");
        assert!((a.velocity(mid) - v).abs() < 1e-2, "hız korunmalı");
    }

    #[test]
    fn geometry_rests_at_sub_pixel_not_at_colour_precision() {
        // Varsayılan `spatial` yayı, ~700 px'lik panel geçişi.
        let spring = Spring::new(0.38, 0.87);
        let t0 = Instant::now();
        let mut rect = Animated::new(Rect::new(0.0, 0.0, 0.0, 0.0), spring);
        rect.set_target(Rect::new(0.0, 0.0, 700.0, 700.0), t0);
        let mut alpha = Animated::new(0.0f32, spring);
        alpha.set_target(1.0, t0);
        let settle = |f: &mut dyn FnMut(Instant) -> bool| {
            (1..400).map(|i| t0 + Duration::from_millis(i * 5)).find(|&t| f(t)).expect("oturmalı") - t0
        };
        let geometry = settle(&mut |t| rect.is_settled(t));
        assert!(geometry < Duration::from_millis(760), "{geometry:?}");
        assert!(geometry > Duration::from_millis(500), "{geometry:?}");
        // 0..1 ölçeği renk hassasiyetinde kalır (1/255 altı).
        let alpha_at = settle(&mut |t| alpha.is_settled(t));
        let mut probe = Animated::new(0.0f32, spring);
        probe.set_target(1.0, t0);
        let before = alpha_at - Duration::from_millis(5);
        assert!((probe.value(t0 + before) - 1.0).abs() < 1.0 / 255.0);
    }

    #[test]
    fn instant_spring_jumps() {
        let t0 = Instant::now();
        let mut a = Animated::new(Rect::ZERO, Spring::INSTANT);
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        a.set_target(r, t0);
        assert!(a.is_settled(t0));
        assert_eq!(a.value(t0), r);
    }
}
