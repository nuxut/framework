/// Yay parametreleri. Fiziksel kütle/sertlik yerine algısal iki değer kullanılır:
///
/// - `response`: sönümsüz salınım periyodu (saniye). Küçük = hızlı.
/// - `damping`: sönüm oranı ζ. `1.0` kritik (taşmaz), `< 1.0` taşar ve geri oturur
///   (puding), `> 1.0` ağır ve yavaş oturur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring {
    pub response: f32,
    pub damping: f32,
}

impl Spring {
    /// Anında oturan yay (hareket kapalıyken kullanılır).
    pub const INSTANT: Self = Self { response: 0.0, damping: 1.0 };

    pub const fn new(response: f32, damping: f32) -> Self {
        Self { response, damping }
    }

    pub fn is_instant(&self) -> bool {
        self.response <= f32::EPSILON
    }

    /// Global hız çarpanı uygulanmış kopya (`speed = 2.0` iki kat hızlı).
    pub fn scaled(&self, speed: f32) -> Self {
        if speed <= f32::EPSILON {
            return Self::INSTANT;
        }
        Self { response: self.response / speed, ..*self }
    }

    /// `t` saniye sonra, başlangıç sapması `x0` (konum − hedef) ve hızı `v0` olan yayın
    /// sapma ve hızı. Kapalı form çözüm.
    pub fn evaluate(&self, x0: f64, v0: f64, t: f64) -> SpringState {
        if self.is_instant() || t < 0.0 {
            return if self.is_instant() { SpringState::REST } else { SpringState { x: x0, v: v0 } };
        }
        let w0 = std::f64::consts::TAU / f64::from(self.response);
        let zeta = f64::from(self.damping.max(0.0));

        if (zeta - 1.0).abs() < 1e-4 {
            // Kritik sönüm.
            let b = v0 + w0 * x0;
            let e = (-w0 * t).exp();
            SpringState { x: (x0 + b * t) * e, v: (v0 - w0 * b * t) * e }
        } else if zeta < 1.0 {
            // Az sönümlü: taşar ve salınarak oturur.
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let e = (-zeta * w0 * t).exp();
            let (s, c) = (wd * t).sin_cos();
            let x = e * (x0 * c + (v0 + zeta * w0 * x0) / wd * s);
            let v = e * (v0 * c - (zeta * w0 * v0 + w0 * w0 * x0) / wd * s);
            SpringState { x, v }
        } else {
            // Aşırı sönümlü.
            let root = (zeta * zeta - 1.0).sqrt();
            let r1 = -w0 * (zeta - root);
            let r2 = -w0 * (zeta + root);
            let c2 = (v0 - r1 * x0) / (r2 - r1);
            let c1 = x0 - c2;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            SpringState { x: c1 * e1 + c2 * e2, v: c1 * r1 * e1 + c2 * r2 * e2 }
        }
    }
}

/// Hedefe göre sapma (`x`) ve hız (`v`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringState {
    pub x: f64,
    pub v: f64,
}

impl SpringState {
    pub const REST: Self = Self { x: 0.0, v: 0.0 };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settles(spring: Spring) {
        let s = spring.evaluate(100.0, 0.0, 5.0);
        assert!(s.x.abs() < 1e-3 && s.v.abs() < 1e-2, "{spring:?} oturmadı: {s:?}");
    }

    #[test]
    fn all_regimes_settle() {
        settles(Spring::new(0.4, 0.6));
        settles(Spring::new(0.4, 1.0));
        settles(Spring::new(0.4, 1.8));
    }

    #[test]
    fn underdamped_overshoots_critical_does_not() {
        let mut min_under = f64::MAX;
        let mut min_crit = f64::MAX;
        for i in 0..400 {
            let t = f64::from(i) / 200.0;
            min_under = min_under.min(Spring::new(0.4, 0.6).evaluate(100.0, 0.0, t).x);
            min_crit = min_crit.min(Spring::new(0.4, 1.0).evaluate(100.0, 0.0, t).x);
        }
        assert!(min_under < -1.0, "az sönümlü yay taşmalı");
        assert!(min_crit > -1e-6, "kritik yay taşmamalı");
    }

    #[test]
    fn velocity_matches_numeric_derivative() {
        for spring in [Spring::new(0.5, 0.5), Spring::new(0.5, 1.0), Spring::new(0.5, 1.5)] {
            let (x0, v0, t, h) = (40.0, -300.0, 0.13, 1e-5);
            let a = spring.evaluate(x0, v0, t - h).x;
            let b = spring.evaluate(x0, v0, t + h).x;
            let numeric = (b - a) / (2.0 * h);
            let analytic = spring.evaluate(x0, v0, t).v;
            assert!((numeric - analytic).abs() < 1e-3, "{spring:?}: {numeric} vs {analytic}");
        }
    }

    #[test]
    fn initial_conditions_hold() {
        let s = Spring::new(0.5, 0.7).evaluate(12.0, 34.0, 0.0);
        assert!((s.x - 12.0).abs() < 1e-9 && (s.v - 34.0).abs() < 1e-9);
    }
}
