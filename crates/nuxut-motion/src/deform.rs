//! Hıza bağlı esneme: hareket eden şekil hareket yönünde uzar, dik yönde incelir (alan
//! korunur), durunca az sönümlü yayla salınarak oturur — "puding" hissinin ikinci yarısı.

use std::time::Instant;

use nuxut_core::Point;

use crate::animated::{Animatable, Animated};
use crate::spring::Spring;

/// Simetrik 2x2 esneme matrisi `[[xx, xy], [xy, yy]]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    pub xx: f32,
    pub xy: f32,
    pub yy: f32,
}

impl Stretch {
    pub const IDENTITY: Self = Self { xx: 1.0, xy: 0.0, yy: 1.0 };

    /// `velocity` yönünde `factor` kadar uzayan, dik yönde `1/factor` kadar incelen matris
    /// (`R · diag(f, 1/f) · Rᵀ`, determinant 1).
    pub fn along(velocity: Point, factor: f32) -> Self {
        let speed = velocity.x.hypot(velocity.y);
        if speed <= f32::EPSILON || factor <= 0.0 {
            return Self::IDENTITY;
        }
        let (c, s) = (velocity.x / speed, velocity.y / speed);
        let inv = 1.0 / factor;
        Self {
            xx: factor * c * c + inv * s * s,
            xy: (factor - inv) * c * s,
            yy: factor * s * s + inv * c * c,
        }
    }

    /// Satır öncelikli `[a, b, c, d]` (render'ın `BlobShape::deform` biçimi).
    pub fn to_array(&self) -> [f32; 4] {
        [self.xx, self.xy, self.xy, self.yy]
    }

    pub fn determinant(&self) -> f32 {
        self.xx * self.yy - self.xy * self.xy
    }
}

impl Animatable for Stretch {
    const LEN: usize = 3;
    fn to_components(&self) -> [f32; 4] {
        [self.xx, self.xy, self.yy, 0.0]
    }
    fn from_components(c: [f32; 4]) -> Self {
        Self { xx: c[0], xy: c[1], yy: c[2] }
    }
}

/// Hız vektöründen esneme matrisi üreten, yayla yumuşatılmış durum.
///
/// Her karede şeklin hızıyla [`update`](Self::update) çağrılır; [`matrix`](Self::matrix)
/// çizimde kullanılır. Yay analitik olduğundan sonuç kare hızından bağımsızdır.
#[derive(Debug, Clone)]
pub struct Deform {
    anim: Animated<Stretch>,
    intensity: f32,
}

/// Bu hızın (mantıksal px/s) altı "duruyor" sayılır.
const MIN_SPEED: f32 = 5.0;
/// Hız başına esneme (şiddet 1.0'da): 1000 px/s ≈ %80 uzama, aşağıdaki sınıra takılır.
const STRETCH_PER_SPEED: f32 = 0.0008;
/// Şiddet 1.0'daki en büyük uzama oranı.
const MAX_STRETCH: f32 = 0.6;
/// Hedef bu kadar az değiştiyse yay yeniden hedeflenmez (gereksiz yeniden başlatma yok).
const RETARGET_EPSILON: f32 = 1e-3;

impl Deform {
    /// Az sönümlü, tek bir salınımda oturan varsayılan yay.
    pub const DEFAULT_SPRING: Spring = Spring::new(0.44, 0.57);

    /// `intensity` 0 = kapalı; 0.6 tipik.
    pub fn new(spring: Spring, intensity: f32) -> Self {
        Self { anim: Animated::new(Stretch::IDENTITY, spring), intensity: sanitize(intensity) }
    }

    pub fn set_params(&mut self, spring: Spring, intensity: f32, now: Instant) {
        self.anim.set_spring(spring, now);
        self.intensity = sanitize(intensity);
    }

    /// Hıza göre hedef esnemeyi verir.
    pub fn target_for(&self, velocity: Point) -> Stretch {
        let speed = velocity.x.hypot(velocity.y);
        if self.intensity <= 0.0 || !speed.is_finite() || speed < MIN_SPEED {
            return Stretch::IDENTITY;
        }
        let extra = (speed * STRETCH_PER_SPEED * self.intensity).min(MAX_STRETCH * self.intensity);
        Stretch::along(velocity, 1.0 + extra)
    }

    /// Şeklin bu karedeki hızı (mantıksal px/s).
    pub fn update(&mut self, velocity: Point, now: Instant) {
        let target = self.target_for(velocity);
        let cur = self.anim.target();
        let delta = (target.xx - cur.xx).abs() + (target.xy - cur.xy).abs() + (target.yy - cur.yy).abs();
        if delta > RETARGET_EPSILON || (target == Stretch::IDENTITY && cur != Stretch::IDENTITY) {
            self.anim.set_target(target, now);
        }
    }

    pub fn stretch(&self, now: Instant) -> Stretch {
        self.anim.value(now)
    }

    pub fn matrix(&self, now: Instant) -> [f32; 4] {
        self.stretch(now).to_array()
    }

    /// Esneme bitti ve hedef kimlik mi? (kare istemeyi bırakmak için)
    pub fn is_settled(&mut self, now: Instant) -> bool {
        self.anim.target() == Stretch::IDENTITY && self.anim.is_settled(now)
    }
}

fn sanitize(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn stretch_preserves_area_and_aligns_with_velocity() {
        for v in [Point::new(300.0, 0.0), Point::new(0.0, -800.0), Point::new(120.0, 90.0)] {
            let m = Stretch::along(v, 1.3);
            assert!((m.determinant() - 1.0).abs() < 1e-5, "{m:?}");
            // Hız yönündeki birim vektör 1.3 kat uzamalı.
            let n = v.x.hypot(v.y);
            let (ux, uy) = (v.x / n, v.y / n);
            let (mx, my) = (m.xx * ux + m.xy * uy, m.xy * ux + m.yy * uy);
            assert!((mx.hypot(my) - 1.3).abs() < 1e-5);
        }
    }

    #[test]
    fn slow_or_disabled_is_identity() {
        let d = Deform::new(Deform::DEFAULT_SPRING, 0.6);
        assert_eq!(d.target_for(Point::new(2.0, 0.0)), Stretch::IDENTITY);
        let off = Deform::new(Deform::DEFAULT_SPRING, 0.0);
        assert_eq!(off.target_for(Point::new(2000.0, 0.0)), Stretch::IDENTITY);
        assert_eq!(d.target_for(Point::new(f32::NAN, 0.0)), Stretch::IDENTITY);
    }

    #[test]
    fn stretch_is_bounded() {
        let d = Deform::new(Deform::DEFAULT_SPRING, 1.0);
        let m = d.target_for(Point::new(1e6, 0.0));
        assert!((m.xx - 1.6).abs() < 1e-4);
    }

    #[test]
    fn wobbles_then_settles_after_stop() {
        let t0 = Instant::now();
        let mut d = Deform::new(Deform::DEFAULT_SPRING, 0.6);
        d.update(Point::new(0.0, 900.0), t0);
        let t1 = t0 + Duration::from_millis(200);
        assert!(d.stretch(t1).yy > 1.1, "hareket yönünde uzamalı");
        d.update(Point::ZERO, t1);
        // Durduktan sonra kimliğin ötesine geçer (basılma) — puding.
        let min_yy = (1..60)
            .map(|i| d.stretch(t1 + Duration::from_millis(i * 10)).yy)
            .fold(f32::MAX, f32::min);
        assert!(min_yy < 0.99, "az sönümlü yay taşmalı: {min_yy}");
        let later = t1 + Duration::from_secs(4);
        assert!(d.is_settled(later));
        assert_eq!(d.matrix(later), [1.0, 0.0, 0.0, 1.0]);
    }
}
