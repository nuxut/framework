//! İşçi profilleri (ARCHITECTURE "Duvar kağıdı yalıtımı"). rlimit'ler girdi okunmadan önce
//! kurulduğu için profil istek başına değil, **işçi süreci başına**dır: ebeveyn işçiyi
//! `--profile=<ad>` ile başlatır, işçi kilitlenirken profilin sınırlarını uygular ve el
//! sıkışmada beyan eder; ebeveyn beyanı kendi profiliyle karşılaştırır (uyuşmazsa
//! `Unavailable`). Her profil ayrı bir `Sandbox`'tır (havuz, kova, susturma ayrı): tepsi ikonu
//! fırtınası duvar kağıdını hız sınırına sokamaz, ele geçirilmiş tepsi ikonu duvar kağıdı
//! işçisinin geniş sınırlarını elde edemez.

use nuxut_image::Limits;

/// İşçi profili.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum WorkerProfile {
    /// Tepsi/bildirim/medya ikonları ve küçük görseller (dar sınırlar).
    #[default]
    Icon,
    /// Kullanıcı dosyaları: duvar kağıdı, kilit avatarı, içe aktarılan durağan görsel.
    Wallpaper,
}

/// Bir profilin süreç ve istek sınırları (işçi ve ebeveyn aynı tabloyu kullanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileCaps {
    /// Girdi baytı üst sınırı (istek başlığındaki `payload_len`).
    pub max_input_bytes: u32,
    /// İsteğin sınırları bunları aşamaz (kenar, kaynak piksel, çözücü bütçesi).
    pub limits: Limits,
    /// Yanıt (çıktı) piksel üst sınırı.
    pub max_out_pixels: u64,
    /// `RLIMIT_AS` (bayt).
    pub address_space: u64,
    /// `RLIMIT_CPU` yumuşak (SIGXCPU) / sert (SIGKILL) saniye.
    pub cpu_soft_secs: u64,
    pub cpu_hard_secs: u64,
    /// İşçinin kendine uyguladığı `nice` (0 = değişmez).
    pub nice: i32,
    /// `/proc/self/oom_score_adj` (`None` = değişmez): bellek sıkışınca çekirdek önce işçiyi
    /// öldürsün, shell'i değil.
    pub oom_score_adj: Option<i32>,
}

impl WorkerProfile {
    pub const ALL: [Self; 2] = [Self::Icon, Self::Wallpaper];

    /// El sıkışmadaki kod.
    pub const fn code(self) -> u8 {
        match self {
            Self::Icon => 1,
            Self::Wallpaper => 2,
        }
    }

    pub const fn from_code(c: u8) -> Option<Self> {
        match c {
            1 => Some(Self::Icon),
            2 => Some(Self::Wallpaper),
            _ => None,
        }
    }

    /// İşçinin komut satırı argümanı.
    pub const fn arg(self) -> &'static str {
        match self {
            Self::Icon => "--profile=icon",
            Self::Wallpaper => "--profile=wallpaper",
        }
    }

    pub fn from_arg(a: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.arg() == a)
    }

    pub const fn caps(self) -> ProfileCaps {
        match self {
            Self::Icon => ProfileCaps {
                max_input_bytes: 8 << 20,
                limits: Limits::ICON,
                max_out_pixels: 4096 * 4096,
                address_space: 768 << 20,
                cpu_soft_secs: 4,
                cpu_hard_secs: 5,
                nice: 0,
                oom_score_adj: None,
            },
            Self::Wallpaper => ProfileCaps {
                max_input_bytes: 256 << 20,
                limits: Limits::WALLPAPER,
                max_out_pixels: 8192 * 8192,
                address_space: 2560 << 20,
                cpu_soft_secs: 10,
                cpu_hard_secs: 12,
                nice: 10,
                oom_score_adj: Some(1000),
            },
        }
    }
}

impl ProfileCaps {
    /// İsteğin sınırlarını profilin tavanına indirger (asla yükseltmez).
    pub fn clamp(&self, l: &Limits) -> Limits {
        Limits {
            max_file_bytes: l.max_file_bytes.min(u64::from(self.max_input_bytes)),
            max_width: l.max_width.min(self.limits.max_width),
            max_height: l.max_height.min(self.limits.max_height),
            max_pixels: l.max_pixels.min(self.limits.max_pixels),
            max_decode_bytes: l.max_decode_bytes.min(self.limits.max_decode_bytes),
            ..*l
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_args_roundtrip() {
        for p in WorkerProfile::ALL {
            assert_eq!(WorkerProfile::from_code(p.code()), Some(p));
            assert_eq!(WorkerProfile::from_arg(p.arg()), Some(p));
        }
        assert_eq!(WorkerProfile::from_code(0), None);
        assert_eq!(WorkerProfile::from_arg("--profile=root"), None);
    }

    #[test]
    fn icon_profile_is_not_loosened_and_clamp_never_raises() {
        let icon = WorkerProfile::Icon.caps();
        assert_eq!((icon.address_space, icon.cpu_soft_secs, icon.max_input_bytes), (768 << 20, 4, 8 << 20));
        let wp = WorkerProfile::Wallpaper.caps();
        let wide = Limits { max_pixels: u64::MAX, max_decode_bytes: u64::MAX, max_width: u32::MAX, ..Limits::DEFAULT };
        let c = icon.clamp(&wide);
        assert_eq!((c.max_width, c.max_pixels, c.max_decode_bytes, c.max_file_bytes), (4096, 16_000_000, 128_000_000, 8 << 20));
        let c = wp.clamp(&Limits::ICON);
        assert_eq!(c, Limits::ICON, "dar istek dar kalır");
        // Bütçe + girdi adres alanına sığar (ikili ve ayırıcı payı 768 MiB).
        assert!(wp.limits.max_decode_bytes + u64::from(wp.max_input_bytes) + (768 << 20) <= wp.address_space);
    }
}
