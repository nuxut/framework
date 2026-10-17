//! İçe aktarmanın kodlama bütçesi: kaynak süresi, kare sayısı ve duvar saati zaman aşımı.
//!
//! Dosya/tuval sınırları (`nuxut_image::Limits`) içindeki bir animasyon ya da video da çok
//! uzun olabilir (binlerce kare, saatler); kodlaması yalnızca iptalle sınırlı kalmasın diye:
//!
//! 1. **Önce ucuz yoklama**: animasyonlu görselde kap yapısından kesin kare sayısı ve süre
//!    (`nuxut_image::animation_info`, piksel çözülmez); videoda kabın süresi ve kare hızı
//!    (mpv, `vo=null`, çözmeden; kap kare hızı bildirmiyorsa birkaç kareden tahmin). Aşan
//!    kaynak kodlayıcıya hiç gitmez.
//! 2. **Akış sırasında**: kodlama yoklanırken kabın bildirdiği süre/kare tahmini aşarsa durdurulur.
//! 3. **mpv'nin kendi sayacı** (`end`, `frames`): üst veri yalan ya da eksik olsa da kodlanan
//!    iş bütçede kesilir (çıktı bütçe uzunluğunda kırpılmış döngü olur).
//! 4. **Duvar saati**: kodlama `encode_timeout`'u aşarsa mpv durdurulur, örneği eşzamanlı yok
//!    edilir (`mpv_terminate_destroy`, tüm iş parçacıkları biter) ve yarım çıktı silinir.

use std::time::Duration;

/// İçe aktarmanın kodlama bütçesi ([`crate::Importer::set_limits`]). Yalnızca **kodlanan**
/// kaynaklara uygulanır; olduğu gibi oynatılan video (bağlantı kaydı) iş üretmez.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportLimits {
    /// Kaynağın (bir tur) en uzun süresi. Eşit olan geçer.
    pub max_duration: Duration,
    /// Kaynaktaki en fazla kare. Eşit olan geçer.
    pub max_frames: u64,
    /// Tek kodlamanın duvar saati süresi; aşılırsa kodlama durdurulur ve çıktı silinir.
    pub encode_timeout: Duration,
}

impl ImportLimits {
    /// 10 dakika, 36 000 kare (10 dk × 60 fps), 2 saat kodlama.
    pub const DEFAULT: Self = Self {
        max_duration: Duration::from_secs(10 * 60),
        max_frames: 36_000,
        encode_timeout: Duration::from_secs(2 * 3600),
    };

    /// Bilinen süre/kare sayısını bütçeyle karşılaştırır (bilinmeyen `None` geçer).
    pub(crate) fn check(&self, duration: Option<Duration>, frames: Option<u64>) -> Result<(), BudgetExceeded> {
        if let Some(actual) = duration
            && actual > self.max_duration
        {
            return Err(BudgetExceeded::Duration { actual, limit: self.max_duration });
        }
        if let Some(actual) = frames
            && actual > self.max_frames
        {
            return Err(BudgetExceeded::Frames { actual, limit: self.max_frames });
        }
        Ok(())
    }

    /// Kabın bildirdiği süre (sn) ve kare hızından kontrol (videolar; tahmini kare sayısı
    /// yuvarlanır). Geçersiz/bilinmeyen değerler yok sayılır. Değerler güvenilmeyen kap üst
    /// verisinden gelir: `Duration`'a sığmayan süre `Duration::MAX` sayılır (panik yok), çarpım
    /// sonsuza taşarsa `as u64` doyar.
    pub(crate) fn check_estimate(&self, duration_s: Option<f64>, fps: Option<f64>) -> Result<(), BudgetExceeded> {
        let duration_s = duration_s.filter(|d| d.is_finite() && *d >= 0.0);
        let frames = duration_s.zip(fps.filter(|f| f.is_finite() && *f > 0.0)).map(|(d, f)| (d * f).round() as u64);
        let duration = duration_s.map(|d| Duration::try_from_secs_f64(d).unwrap_or(Duration::MAX));
        self.check(duration, frames)
    }
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Bütçe aşımı ([`crate::ImportError::Budget`]). İleti kullanıcıya gösterilebilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BudgetExceeded {
    #[error("kaynak çok uzun: {} (sınır {})", HumanDuration(*actual), HumanDuration(*limit))]
    Duration { actual: Duration, limit: Duration },
    #[error("kaynakta çok fazla kare: {actual} (sınır {limit})")]
    Frames { actual: u64, limit: u64 },
}

/// "1 sa 5 dk 3 sn", "960 ms".
pub(crate) struct HumanDuration(pub Duration);

impl std::fmt::Display for HumanDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secs = self.0.as_secs();
        if secs == 0 {
            return write!(f, "{} ms", self.0.as_millis());
        }
        let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
        let parts: Vec<String> = [(h, "sa"), (m, "dk"), (s, "sn")].iter().filter(|(v, _)| *v > 0).map(|(v, u)| format!("{v} {u}")).collect();
        f.write_str(&parts.join(" "))
    }
}

/// `run_mpv`'in akış bütçesi.
#[derive(Debug, Clone, Copy)]
pub(crate) struct StreamBudget {
    pub limits: ImportLimits,
    /// Akış sırasında kabın süre/kare hızı tahmini denetlensin mi (videolar). Animasyonlu
    /// görselde kapalı: kesin sayılar önceden bilinir, FFmpeg'in görsel süre tahmini kaba.
    pub estimate: bool,
}

impl StreamBudget {
    /// mpv'nin kendi kesme seçenekleri (`end` sn, `frames`). mpv'nin aralığı dışındaki (pratikte
    /// sınırsız) değerler verilmez.
    pub fn backstop(&self) -> Vec<(&'static str, String)> {
        let mut v = Vec::with_capacity(2);
        let secs = self.limits.max_duration.as_secs_f64();
        if secs < 1e9 {
            v.push(("end", format!("{secs:.3}")));
        }
        if self.limits.max_frames < i32::MAX as u64 {
            v.push(("frames", self.limits.max_frames.to_string()));
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_passes_and_excess_fails() {
        let l = ImportLimits { max_duration: Duration::from_secs(10), max_frames: 100, encode_timeout: Duration::from_secs(1) };
        assert_eq!(l.check(Some(Duration::from_secs(10)), Some(100)), Ok(()));
        assert_eq!(l.check(None, None), Ok(()));
        assert_eq!(
            l.check(Some(Duration::from_millis(10_001)), Some(1)),
            Err(BudgetExceeded::Duration { actual: Duration::from_millis(10_001), limit: Duration::from_secs(10) })
        );
        assert_eq!(l.check(Some(Duration::from_secs(1)), Some(101)), Err(BudgetExceeded::Frames { actual: 101, limit: 100 }));
        // Tahmin: 10 sn × 10 fps = 100 kare geçer; 10 sn × 10.1 fps = 101 aşar.
        assert_eq!(l.check_estimate(Some(10.0), Some(10.0)), Ok(()));
        assert!(matches!(l.check_estimate(Some(10.0), Some(10.1)), Err(BudgetExceeded::Frames { actual: 101, .. })));
        assert_eq!(l.check_estimate(Some(f64::NAN), Some(1e9)), Ok(()));
        assert!(matches!(l.check_estimate(Some(11.0), None), Err(BudgetExceeded::Duration { .. })));
    }

    /// Düşmanca kap üst verisi: panik yok, sınırsız değerler aşım sayılır.
    #[test]
    fn hostile_estimates_never_panic() {
        let l = ImportLimits::DEFAULT;
        let odd = [None, Some(0.0), Some(-0.0), Some(-1.0), Some(f64::NAN), Some(f64::INFINITY), Some(f64::NEG_INFINITY), Some(f64::MIN_POSITIVE), Some(1e300), Some(f64::MAX)];
        for d in odd {
            for f in odd {
                let _ = l.check_estimate(d, f);
            }
        }
        assert!(matches!(l.check_estimate(Some(f64::MAX), None), Err(BudgetExceeded::Duration { actual: Duration::MAX, .. })));
        assert!(matches!(l.check_estimate(Some(1e300), Some(1e300)), Err(BudgetExceeded::Duration { .. })));
        let tiny = ImportLimits { max_duration: Duration::MAX, ..l };
        assert!(matches!(tiny.check_estimate(Some(1e300), Some(1e300)), Err(BudgetExceeded::Frames { actual: u64::MAX, .. })));
        assert_eq!(l.check_estimate(Some(0.0), Some(f64::MAX)), Ok(()));
        // İletiler en uç değerlerde de biçimlenir.
        let _ = BudgetExceeded::Duration { actual: Duration::MAX, limit: Duration::ZERO }.to_string();
    }

    #[test]
    fn messages_are_readable() {
        let e = BudgetExceeded::Duration { actual: Duration::from_secs(3723), limit: Duration::from_secs(600) };
        assert_eq!(e.to_string(), "kaynak çok uzun: 1 sa 2 dk 3 sn (sınır 10 dk)");
        let e = BudgetExceeded::Duration { actual: Duration::from_millis(960), limit: Duration::from_millis(900) };
        assert_eq!(e.to_string(), "kaynak çok uzun: 960 ms (sınır 900 ms)");
        assert_eq!(BudgetExceeded::Frames { actual: 12, limit: 11 }.to_string(), "kaynakta çok fazla kare: 12 (sınır 11)");
    }
}
