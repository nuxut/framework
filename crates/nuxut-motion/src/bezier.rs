/// [`CubicBezier::slope`]'un üst sınırı.
pub const MAX_SLOPE: f32 = 50.0;

/// CSS `cubic-bezier(x1, y1, x2, y2)` zamanlama eğrisi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezier {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezier {
    pub const LINEAR: Self = Self::new(0.0, 0.0, 1.0, 1.0);
    /// Material 3 "standard".
    pub const STANDARD: Self = Self::new(0.2, 0.0, 0.0, 1.0);
    /// Material 3 "emphasized decelerate" — içeri giren öğeler.
    pub const DECELERATE: Self = Self::new(0.05, 0.7, 0.1, 1.0);
    /// Material 3 "emphasized accelerate" — dışarı çıkan öğeler.
    pub const ACCELERATE: Self = Self::new(0.3, 0.0, 0.8, 0.15);
    /// CSS adlandırılmış eğrileri.
    pub const EASE: Self = Self::new(0.25, 0.1, 0.25, 1.0);
    pub const EASE_IN: Self = Self::new(0.42, 0.0, 1.0, 1.0);
    pub const EASE_OUT: Self = Self::new(0.0, 0.0, 0.58, 1.0);
    pub const EASE_IN_OUT: Self = Self::new(0.42, 0.0, 0.58, 1.0);

    /// Ad ya da `cubic-bezier(x1, y1, x2, y2)` biçiminden. Adlar: `linear`, `ease`,
    /// `ease-in`, `ease-out`, `ease-in-out`, `emphasized` (= standard),
    /// `emphasized-decelerate`, `emphasized-accelerate`. x değerleri [0, 1], y değerleri
    /// [-1, 2] dışında olamaz.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        let named = match s {
            "linear" => Some(Self::LINEAR),
            "ease" => Some(Self::EASE),
            "ease-in" => Some(Self::EASE_IN),
            "ease-out" => Some(Self::EASE_OUT),
            "ease-in-out" => Some(Self::EASE_IN_OUT),
            "emphasized" | "standard" => Some(Self::STANDARD),
            "emphasized-decelerate" => Some(Self::DECELERATE),
            "emphasized-accelerate" => Some(Self::ACCELERATE),
            _ => None,
        };
        if let Some(c) = named {
            return Ok(c);
        }
        let inner = s
            .strip_prefix("cubic-bezier(")
            .and_then(|r| r.strip_suffix(')'))
            .ok_or_else(|| format!("bilinmeyen eğri '{s}'"))?;
        let v: Vec<f32> = inner
            .split(',')
            .map(|p| p.trim().parse::<f32>().map_err(|_| format!("geçersiz sayı '{}' ({s})", p.trim())))
            .collect::<Result<_, _>>()?;
        let [x1, y1, x2, y2] = v[..] else {
            return Err(format!("cubic-bezier dört sayı ister ({s})"));
        };
        if ![x1, x2].iter().all(|x| (0.0..=1.0).contains(x)) {
            return Err(format!("cubic-bezier x değerleri [0, 1] içinde olmalı ({s})"));
        }
        if ![y1, y2].iter().all(|y| (-1.0..=2.0).contains(y)) {
            return Err(format!("cubic-bezier y değerleri [-1, 2] içinde olmalı ({s})"));
        }
        Ok(Self::new(x1, y1, x2, y2))
    }

    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// İlerleme `t ∈ [0, 1]` için eğri değeri.
    pub fn ease(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        if t == 0.0 || t == 1.0 {
            return t;
        }
        let s = self.solve_x(t);
        sample(self.y1, self.y2, s)
    }

    /// Eğrinin `t`'deki eğimi (dy/dx). Dikey başlangıçlı eğrilerde (x1 = 0, y1 > 0)
    /// [`MAX_SLOPE`]'a sınırlanır.
    pub fn slope(&self, t: f32) -> f32 {
        let s = self.solve_x(t.clamp(0.0, 1.0));
        let dx = sample_derivative(self.x1, self.x2, s);
        let dy = sample_derivative(self.y1, self.y2, s);
        if dx.abs() < 1e-4 {
            // Teğet dik ya da başlangıç/bitiş noktası: x1 = y1 = 0 gibi durumlarda eğim
            // ikinci mertebeden gelir; küçük bir adımla sayısal tahmin.
            let h = 1e-3;
            let (a, b) = ((t - h).clamp(0.0, 1.0), (t + h).clamp(0.0, 1.0));
            return ((self.ease(b) - self.ease(a)) / (b - a)).clamp(-MAX_SLOPE, MAX_SLOPE);
        }
        (dy / dx).clamp(-MAX_SLOPE, MAX_SLOPE)
    }

    fn solve_x(&self, x: f32) -> f32 {
        // Newton-Raphson, yakınsamazsa ikiye bölme.
        let mut s = x;
        for _ in 0..8 {
            let err = sample(self.x1, self.x2, s) - x;
            if err.abs() < 1e-6 {
                return s;
            }
            let d = sample_derivative(self.x1, self.x2, s);
            if d.abs() < 1e-6 {
                break;
            }
            s -= err / d;
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        s = x;
        for _ in 0..32 {
            let v = sample(self.x1, self.x2, s);
            if (v - x).abs() < 1e-6 {
                break;
            }
            if v < x {
                lo = s;
            } else {
                hi = s;
            }
            s = (lo + hi) * 0.5;
        }
        s
    }
}

fn sample(p1: f32, p2: f32, s: f32) -> f32 {
    let inv = 1.0 - s;
    3.0 * inv * inv * s * p1 + 3.0 * inv * s * s * p2 + s * s * s
}

fn sample_derivative(p1: f32, p2: f32, s: f32) -> f32 {
    let inv = 1.0 - s;
    3.0 * inv * inv * p1 + 6.0 * inv * s * (p2 - p1) + 3.0 * s * s * (1.0 - p2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_and_monotonic() {
        for curve in [CubicBezier::STANDARD, CubicBezier::DECELERATE, CubicBezier::ACCELERATE] {
            assert_eq!(curve.ease(0.0), 0.0);
            assert_eq!(curve.ease(1.0), 1.0);
            let mut prev = 0.0;
            for i in 1..=100 {
                let v = curve.ease(i as f32 / 100.0);
                assert!(v >= prev - 1e-4);
                prev = v;
            }
        }
    }

    #[test]
    fn linear_is_identity() {
        assert!((CubicBezier::LINEAR.ease(0.37) - 0.37).abs() < 1e-4);
    }
}
