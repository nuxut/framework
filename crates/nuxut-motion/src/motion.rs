//! Hareket stili: yay ya da süre + eğri. [`Animated`](crate::Animated) ikisini de çalıştırır;
//! uygulama hangisinin seçildiğini bilmez (tema verir).

use crate::bezier::CubicBezier;
use crate::spring::{Spring, SpringState};

/// Süre + zamanlama eğrisi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve {
    pub easing: CubicBezier,
    /// Saniye.
    pub duration: f32,
}

impl Curve {
    pub const fn new(easing: CubicBezier, duration: f32) -> Self {
        Self { easing, duration }
    }

    pub fn is_instant(&self) -> bool {
        self.duration.is_nan() || self.duration <= 1e-4
    }

    /// Başlangıç sapması `x0` (konum − hedef) ve hızı `v0` olan geçişin `t` saniyedeki
    /// sapma ve hızı.
    ///
    /// Değer `x0 · (1 − ease(p))` eğrisini izler. Kesilen bir hareketin hızı sıçramasın diye
    /// başlangıçtaki hız farkı, sonda kendiliğinden sönen bir düzeltme terimiyle
    /// (`p·(1−p)²` biçimli) karşılanır: `t = 0`'da hız tam `v0`, `t = T`'de değer ve hız 0.
    pub fn evaluate(&self, x0: f64, v0: f64, t: f64) -> SpringState {
        if self.is_instant() {
            return SpringState::REST;
        }
        let duration = f64::from(self.duration);
        if t >= duration {
            return SpringState::REST;
        }
        let p = (t / duration).max(0.0);
        let pf = p as f32;
        let e = f64::from(self.easing.ease(pf));
        let slope = f64::from(self.easing.slope(pf));
        let slope0 = f64::from(self.easing.slope(0.0));
        // Eğrinin kendi başlangıç hızı −x0·slope0/T; kalan fark düzeltme terimine.
        let correction = v0 + x0 * slope0 / duration;
        let h = p * (1.0 - p) * (1.0 - p);
        let dh = (1.0 - p) * (1.0 - 3.0 * p);
        SpringState {
            x: x0 * (1.0 - e) + correction * duration * h,
            v: -x0 * slope / duration + correction * dh,
        }
    }
}

/// Bir değerin hedefe nasıl gideceği.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    /// Kesilebilir, hızı koruyan yay (varsayılan; puding hissi).
    Spring(Spring),
    /// Sabit süreli eğri. Kesilince o anki değer ve hızdan yeni bir geçiş başlar.
    Curve(Curve),
}

impl Motion {
    pub const INSTANT: Self = Self::Spring(Spring::INSTANT);

    pub fn is_instant(&self) -> bool {
        match self {
            Self::Spring(s) => s.is_instant(),
            Self::Curve(c) => c.is_instant(),
        }
    }

    /// Global hız çarpanı (`speed = 2.0` iki kat hızlı).
    pub fn scaled(&self, speed: f32) -> Self {
        if speed.is_nan() || speed <= f32::EPSILON {
            return Self::INSTANT;
        }
        match *self {
            Self::Spring(s) => Self::Spring(s.scaled(speed)),
            Self::Curve(c) => Self::Curve(Curve { duration: c.duration / speed, ..c }),
        }
    }

    pub fn evaluate(&self, x0: f64, v0: f64, t: f64) -> SpringState {
        match self {
            Self::Spring(s) => s.evaluate(x0, v0, t),
            Self::Curve(c) => c.evaluate(x0, v0, t),
        }
    }
}

impl From<Spring> for Motion {
    fn from(s: Spring) -> Self {
        Self::Spring(s)
    }
}

impl From<Curve> for Motion {
    fn from(c: Curve) -> Self {
        Self::Curve(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_hits_target_at_duration() {
        let c = Curve::new(CubicBezier::STANDARD, 0.3);
        let s = c.evaluate(100.0, 0.0, 0.0);
        assert!((s.x - 100.0).abs() < 1e-6);
        assert_eq!(c.evaluate(100.0, 0.0, 0.301), SpringState::REST);
        let mid = c.evaluate(100.0, 0.0, 0.15);
        assert!(mid.x > 0.0 && mid.x < 100.0);
    }

    #[test]
    fn interrupted_curve_keeps_velocity() {
        for easing in [CubicBezier::STANDARD, CubicBezier::EASE, CubicBezier::LINEAR, CubicBezier::EASE_IN_OUT] {
            let c = Curve::new(easing, 0.4);
            let s = c.evaluate(-50.0, 420.0, 0.0);
            assert!((s.v - 420.0).abs() < 1e-3, "{easing:?}: {}", s.v);
        }
    }

    #[test]
    fn curve_velocity_matches_numeric_derivative() {
        let c = Curve::new(CubicBezier::EASE, 0.5);
        let (x0, v0, t, h) = (80.0, -200.0, 0.21, 1e-4);
        let numeric = (c.evaluate(x0, v0, t + h).x - c.evaluate(x0, v0, t - h).x) / (2.0 * h);
        let analytic = c.evaluate(x0, v0, t).v;
        assert!((numeric - analytic).abs() < 0.5, "{numeric} vs {analytic}");
    }

    #[test]
    fn speed_scales_duration() {
        let m = Motion::Curve(Curve::new(CubicBezier::EASE, 0.4)).scaled(2.0);
        assert_eq!(m, Motion::Curve(Curve::new(CubicBezier::EASE, 0.2)));
        assert!(Motion::Curve(Curve::new(CubicBezier::EASE, 0.4)).scaled(0.0).is_instant());
    }
}
