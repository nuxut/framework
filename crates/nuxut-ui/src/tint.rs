//! Değer göstergelerinin rengi: [`Tint`] (tema rolü ya da sabit renk) ve [`Threshold`]
//! (değer eşiği aşınca başka renge kesilebilir geçiş). `Progress`, `Gauge` ve `Sparkline`
//! ortak kullanır.
//!
//! Renk her karede temadan çözülür (`Tint::Role`), böylece tema geçişini ve palet değişimini
//! kendiliğinden izler. Eşik geçişi iki rengin karışım oranıdır (`motion.effects` yayı); renk
//! bileşenleri canlandırılmaz — tema renkleri kendi geçişinde akarken çift gecikme olmaz.

use std::time::Instant;

use nuxut_core::Color;
use nuxut_motion::Animated;
use nuxut_theme::{ColorRole, Theme};

use crate::style::THRESHOLD_HYSTERESIS;

/// Göstergenin rengi: tema rolü (önerilen; temayla değişir) ya da çağıranın verdiği renk
/// (ör. takvim rengi, kullanıcı seçimi — koda gömülü renk değil).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    Role(ColorRole),
    Color(Color),
}

impl Tint {
    pub const PRIMARY: Self = Self::Role(ColorRole::Primary);
    pub const ERROR: Self = Self::Role(ColorRole::Error);

    /// Geçerli temada renk. Yüksek karşıtlıkta özel renk (`Tint::Color`) bir gösterge dolgusu
    /// olarak yüzeylere ≥ 3:1 itilir (`style::legible`); roller paletten zaten eşiktedir.
    pub fn resolve(self, theme: &Theme) -> Color {
        match self {
            Self::Role(r) => theme.colors.get(r),
            Self::Color(c) => crate::style::legible(theme, c, &crate::style::surfaces(theme), nuxut_theme::contrast::NON_TEXT_MIN),
        }
    }
}

impl Default for Tint {
    fn default() -> Self {
        Self::PRIMARY
    }
}

impl From<ColorRole> for Tint {
    fn from(r: ColorRole) -> Self {
        Self::Role(r)
    }
}

impl From<Color> for Tint {
    fn from(c: Color) -> Self {
        Self::Color(c)
    }
}

/// Eşik: değer `level`'e ulaşınca renk `tint`'e (varsayılan `error`) kesilebilir geçişle döner;
/// `level − hysteresis` altına inince geri döner. `level` bileşenin değer biriminde
/// (`Progress`/`Gauge`: 0..=1; `Sparkline`: itilen örneklerin birimi).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threshold {
    pub level: f32,
    pub tint: Tint,
    pub hysteresis: f32,
}

impl Threshold {
    /// `error` renginde, [`THRESHOLD_HYSTERESIS`] geri dönüş paylı eşik. Pay 0..=1 ölçeğindedir
    /// (`Progress`/`Gauge`); başka birimde değer veren bileşende (`Sparkline` °C, %, MB/s)
    /// [`for_range`](Self::for_range) / [`percent`](Self::percent) ya da
    /// [`with_hysteresis`](Self::with_hysteresis) kullanın — yoksa pay fiilen sıfırdır.
    pub fn new(level: f32) -> Self {
        Self { level, tint: Tint::ERROR, hysteresis: THRESHOLD_HYSTERESIS }
    }

    /// Değer birimi `0..=full_scale` olan bileşen için eşik: geri dönüş payı tam ölçeğin
    /// [`THRESHOLD_HYSTERESIS`] oranı (ör. `for_range(85.0, 100.0)` → 85'te açılır, 82'nin altında
    /// kapanır). `level` değer biriminde. Sonlu olmayan/negatif ölçek → pay 0.
    pub fn for_range(level: f32, full_scale: f32) -> Self {
        Self::new(level).with_hysteresis(THRESHOLD_HYSTERESIS * full_scale)
    }

    /// Yüzde biriminde (`0..=100`) değer için eşik: `for_range(level, 100.0)` (3 puan pay).
    pub fn percent(level: f32) -> Self {
        Self::for_range(level, 100.0)
    }

    pub fn with_tint(mut self, tint: impl Into<Tint>) -> Self {
        self.tint = tint.into();
        self
    }

    /// Geri dönüş payı (değer biriminde; negatif/sonlu olmayan → 0).
    pub fn with_hysteresis(mut self, h: f32) -> Self {
        self.hysteresis = if h.is_finite() { h.max(0.0) } else { 0.0 };
        self
    }

    /// Önceki durum `on` iken `value` için yeni durum (geri dönüş paylı). Sonlu olmayan
    /// değer durumu değiştirmez.
    pub fn crossed(&self, value: f32, on: bool) -> bool {
        if !value.is_finite() || !self.level.is_finite() {
            return on;
        }
        if on { value >= self.level - self.hysteresis } else { value >= self.level }
    }
}

/// Taban rengi + isteğe bağlı eşik rengi arasında karışım (bileşen içi).
#[derive(Debug, Clone)]
pub(crate) struct ToneState {
    base: Tint,
    threshold: Option<Threshold>,
    alert: bool,
    /// Son eşiğin rengi: eşik kaldırılınca geçiş bu renkten söner.
    alert_tint: Tint,
    /// 0 = taban, 1 = eşik rengi (`motion.effects`).
    mix: Animated<f32>,
}

impl ToneState {
    pub fn new(theme: &Theme) -> Self {
        Self {
            base: Tint::default(),
            threshold: None,
            alert: false,
            alert_tint: Tint::ERROR,
            mix: Animated::new(0.0, theme.motion.effects),
        }
    }

    /// Değiştiyse `true`.
    pub fn set_base(&mut self, tint: Tint) -> bool {
        let changed = self.base != tint;
        self.base = tint;
        changed
    }

    pub fn base(&self) -> Tint {
        self.base
    }

    /// Eşiği değiştirir ve `value` ile durumu yeniden değerlendirir. Görünüş değiştiyse `true`.
    pub fn set_threshold(&mut self, threshold: Option<Threshold>, value: f32, now: Instant) -> bool {
        let changed = self.threshold != threshold;
        self.threshold = threshold;
        if let Some(t) = threshold {
            self.alert_tint = t.tint;
        }
        self.update(value, now) || changed
    }

    /// Yeni değer (hedef; canlanan değil — renk veri gelince karar verir). Eşik durumu
    /// değiştiyse `true`.
    pub fn update(&mut self, value: f32, now: Instant) -> bool {
        let on = self.threshold.is_some_and(|t| t.crossed(value, self.alert));
        if on == self.alert {
            return false;
        }
        self.alert = on;
        self.mix.set_target(if on { 1.0 } else { 0.0 }, now);
        true
    }

    pub fn is_alert(&self) -> bool {
        self.alert
    }

    /// Animasyonsuz son duruma (ilk yerleşim, galeri).
    pub fn settle(&mut self) {
        self.mix.jump_to(self.mix.target());
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.mix.set_motion(theme.motion.effects, now);
    }

    pub fn color(&self, theme: &Theme, now: Instant) -> Color {
        let base = self.base.resolve(theme);
        let t = self.mix.value(now).clamp(0.0, 1.0);
        if t > 0.0 { base.lerp(&self.alert_tint.resolve(theme), t) } else { base }
    }

    /// Karışım hâlâ değişiyor mu (kare istenmeli).
    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.mix.is_settled(now)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn tint_resolves_from_theme() {
        let theme = Theme::default();
        assert_eq!(Tint::default().resolve(&theme), theme.colors.primary);
        assert_eq!(Tint::from(ColorRole::Error).resolve(&theme), theme.colors.error);
        let c = theme.colors.surface_bright;
        assert_eq!(Tint::from(c).resolve(&theme), c);
    }

    #[test]
    fn threshold_has_hysteresis_and_ignores_nan() {
        let t = Threshold::new(0.9).with_hysteresis(0.05);
        assert!(!t.crossed(0.89, false));
        assert!(t.crossed(0.9, false));
        assert!(t.crossed(0.86, true), "geri dönüş payı içinde açık kalır");
        assert!(!t.crossed(0.84, true));
        assert!(t.crossed(f32::NAN, true), "NaN durumu değiştirmez");
        assert!(!t.crossed(f32::INFINITY, false));
        assert_eq!(Threshold::new(0.5).with_hysteresis(f32::NAN).hysteresis, 0.0);
        assert_eq!(Threshold::new(0.5).with_hysteresis(-1.0).hysteresis, 0.0);
    }

    #[test]
    fn threshold_hysteresis_scales_with_unit() {
        // Yüzde: 3 puan pay.
        let p = Threshold::percent(90.0);
        assert_eq!((p.level, p.tint), (90.0, Tint::ERROR));
        assert!((p.hysteresis - 3.0).abs() < 1e-4);
        assert!(p.crossed(90.0, false) && !p.crossed(89.9, false));
        assert!(p.crossed(87.5, true), "pay içinde açık kalır");
        assert!(!p.crossed(86.9, true));
        // Keyfî ölçek (°C, 0..=120): pay 3.6.
        let c = Threshold::for_range(85.0, 120.0);
        assert!((c.hysteresis - 3.6).abs() < 1e-4);
        assert!(c.crossed(81.5, true) && !c.crossed(81.3, true));
        // 0..=1 ölçeği new ile aynı; bozuk ölçek pay 0.
        assert_eq!(Threshold::for_range(0.9, 1.0), Threshold::new(0.9));
        assert_eq!(Threshold::for_range(0.9, f32::NAN).hysteresis, 0.0);
        assert_eq!(Threshold::for_range(0.9, -5.0).hysteresis, 0.0);
    }

    #[test]
    fn tone_crossfades_to_threshold_color() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut tone = ToneState::new(&theme);
        assert!(!tone.set_threshold(None, 0.95, t0), "eşiksiz değişiklik yok");
        assert!(tone.set_threshold(Some(Threshold::new(0.9)), 0.95, t0));
        assert!(tone.is_alert());
        // Geçişin ortasında iki rengin arasında, sonunda error.
        let mid = tone.color(&theme, t0 + Duration::from_millis(60));
        assert_ne!(mid, theme.colors.primary);
        assert_ne!(mid, theme.colors.error);
        assert!(tone.is_animating(t0 + Duration::from_millis(60)));
        let end = t0 + Duration::from_secs(3);
        assert!(!tone.is_animating(end));
        assert_eq!(tone.color(&theme, end), theme.colors.error);
        // Aynı durumda yeni değer: değişiklik yok (kare istenmez).
        assert!(!tone.update(0.99, end));
        // Kesilebilir: geri dönüş yarı yolda başlar.
        assert!(tone.update(0.1, end));
        let back = end + Duration::from_millis(30);
        assert!(tone.update(0.95, back));
        assert!(tone.is_animating(back));
    }
}
