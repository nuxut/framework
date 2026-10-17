//! Çözme sınırları ve hedef boyut.

/// Dış girdiye (dosya, D-Bus pixmap) uygulanan sınırlar. Varsayılanlar 8K duvar kağıdına ve
/// uzun GIF'lere yeter, dekompresyon bombasını reddeder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Okunacak en büyük dosya.
    pub max_file_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    /// Genişlik × yükseklik (çözülmüş tek kare).
    pub max_pixels: u64,
    /// Çözücünün bellek bütçesi (bayt): çözülmüş tampon (kanal × bit derinliği) + 8 bit RGBA'ya
    /// dönüşüm kopyası, başlıktan ÇÖZMEDEN tahmin edilir; aşan görsel `TooLarge`. Aynı değer
    /// çözücünün kendi ayırma sınırıdır (`image::Limits::max_alloc`). JPEG XL ara düzlemleri
    /// (f32) piksel başına [`JXL_BYTES_PER_PIXEL`] sayılır.
    pub max_decode_bytes: u64,
    /// Animasyondaki en fazla kare.
    pub max_frames: usize,
    /// Animasyonun (küçültülmüş) karelerini bellekte tutma bütçesi. Aşılırsa kareler
    /// [`crate::AnimationStream`] ile arka planda akıtılır (bellekte ~3 kare).
    pub max_animation_bytes: u64,
    /// Animasyonun bellekte biriktirilerek çözülürken harcayabileceği iş: kare başına tam
    /// tuval (her kare tuvale birleştirilir). Aşılırsa kalan kareler akıtılır; çok kareli
    /// büyük tuval çözücüyü toptan meşgul edemez.
    pub max_animation_pixels: u64,
    /// Kaynağın güveni; sabitlerde `Untrusted`. Sınırlarla birlikte taşınır ki `..Limits::ICON`
    /// ile kurulan her istek güvenli varsayılanı alsın.
    pub trust: Trust,
}

/// JPEG XL çözümünün piksel başına tepe bellek tahmini (bayt): jxl-oxide'ın f32 kanal düzlemleri,
/// ara tamponlar ve 8 bit çıktı. Ölçüm (`nuxut-image-worker` `examples/wallpaper_bench.rs`): 8K
/// UHD (33.2 Mpx) işçi tepe RSS 879 MiB ≈ 27.7 B/px. Duvar kağıdı bütçesiyle (1.5 GiB) ≈ 57 Mpx.
pub const JXL_BYTES_PER_PIXEL: u64 = 28;

impl Limits {
    pub const DEFAULT: Self = Self {
        max_file_bytes: 256 << 20,
        max_width: 16_384,
        max_height: 16_384,
        max_pixels: 80_000_000,
        max_decode_bytes: 640_000_000,
        max_frames: 2_000,
        max_animation_bytes: 32 << 20,
        max_animation_pixels: 400_000_000,
        trust: Trust::Untrusted,
    };

    /// İkonlar ve bildirim görselleri için dar sınırlar.
    pub const ICON: Self = Self {
        max_file_bytes: 8 << 20,
        max_width: 4_096,
        max_height: 4_096,
        max_pixels: 16_000_000,
        max_decode_bytes: 128_000_000,
        max_frames: 256,
        max_animation_bytes: 8 << 20,
        max_animation_pixels: 32_000_000,
        trust: Trust::Untrusted,
    };

    /// Kullanıcının duvar kağıdı (yalıtılmış işçinin `Wallpaper` profili, ARCHITECTURE "Duvar
    /// kağıdı yalıtımı"): 256 MiB dosya, 16K kenar, 16K² kaynak, 1.5 GiB çözücü bütçesi (8 bit
    /// JPEG 16K² ≈ 805 MB geçer; 16 bit RGBA PNG 16K² ≈ 3 GiB `TooLarge`; JPEG XL ≈ 57 Mpx).
    /// Yalnızca ilk kare çözülür.
    pub const WALLPAPER: Self = Self {
        max_file_bytes: 256 << 20,
        max_width: 16_384,
        max_height: 16_384,
        max_pixels: 16_384 * 16_384,
        max_decode_bytes: 1536 << 20,
        max_frames: 1,
        max_animation_bytes: 32 << 20,
        max_animation_pixels: 400_000_000,
        trust: Trust::Untrusted,
    };

    pub const fn with_trust(mut self, trust: Trust) -> Self {
        self.trust = trust;
        self
    }

    /// Tek sınır kapısı: her biçim ve yol (başlık, çözüm sonrası, animasyon karesi, ham piksel)
    /// boyutu buradan geçirir.
    pub fn check_dimensions(&self, width: u32, height: u32) -> Result<(), crate::ImageError> {
        if width == 0 || height == 0 {
            return Err(crate::ImageError::Empty);
        }
        if width > self.max_width || height > self.max_height || u64::from(width) * u64::from(height) > self.max_pixels {
            return Err(crate::ImageError::TooLarge { width, height });
        }
        Ok(())
    }

    /// Bellek bütçesi kapısı: `estimate` bayt (başlıktan tahmin) [`max_decode_bytes`]'ı
    /// aşarsa `TooLarge`.
    ///
    /// [`max_decode_bytes`]: Self::max_decode_bytes
    pub fn check_budget(&self, (width, height): (u32, u32), estimate: u64) -> Result<(), crate::ImageError> {
        if estimate > self.max_decode_bytes {
            return Err(crate::ImageError::TooLarge { width, height });
        }
        Ok(())
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Hedef kutuya nasıl yerleşileceği. Çözme yalnızca **küçültür**, asla büyütmez; kırpma ve
/// ortalama çizimde yapılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// Kutuyu tamamen kaplar (taşan kısım çizimde kırpılır). Duvar kağıdı varsayılanı.
    #[default]
    Cover,
    /// Kutunun içine sığar (en-boy oranı korunur).
    Contain,
    /// Kutuya gerilir (en-boy oranı bozulur).
    Fill,
}

/// Fiziksel piksel hedef boyutu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetSize {
    pub width: u32,
    pub height: u32,
    pub fit: Fit,
    /// Yalnızca `Fit::Cover`: kutuyu kaplayan ölçekten sonra taşan eksen ORTADAN kırpılır, çıktı
    /// her iki eksende ≤ kutu olur (kırpma küçültmeden önce kaynak dikdörtgende yapılır; tek
    /// geçiş). Çizimdeki `ImageFit::Cover` ortalamasıyla aynı bölge. Diğer yerleşimlerde etkisiz.
    pub crop: bool,
}

impl TargetSize {
    pub const fn new(width: u32, height: u32, fit: Fit) -> Self {
        Self { width, height, fit, crop: false }
    }

    /// `Cover` + ortadan kırpma (duvar kağıdı): çıktı tam olarak ≤ kutu.
    pub const fn cover_crop(width: u32, height: u32) -> Self {
        Self { width, height, fit: Fit::Cover, crop: true }
    }

    /// Kırpma gerçekten uygulanıyor mu (`Cover` + `crop`).
    pub const fn crops(&self) -> bool {
        self.crop && matches!(self.fit, Fit::Cover)
    }
}

/// Kaynağın güveni — biçime göre değil, baytların nereden geldiğine göre. Güven belirtmeyen
/// her çağrı `Untrusted`'tır (güvenli varsayılan); `System` yalnızca açıkça verilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Trust {
    /// Sistem ikon temasından (ev dizini dışındaki arama dizinleri: `/usr/share/icons`,
    /// `/usr/share/pixmaps`) bizim seçtiğimiz adla bulunan dosya ve gömülü varlıklar. Kullanıcının
    /// duvar kağıdı artık `System` DEĞİLDİR: yalıtılmış işçide çözülür (`WallpaperDecoder`).
    System,
    /// Başka bir sürecin verdiği ad, yol ya da bayt (tepsi `IconName`/`IconThemePath`, dbusmenu
    /// `icon-name`/`icon-data`, bildirim `image-path`/`app_icon`/`image-data`, MPRIS `artUrl`).
    /// SVG süreç içinde hiç ayrıştırılmaz (ADR-0007 işçisi inene kadar `Unsupported`).
    #[default]
    Untrusted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Sınırlar ve kaynağın güveni ([`Limits::trust`]).
    pub limits: Limits,
    /// `None` = özgün boyut.
    pub target: Option<TargetSize>,
    /// `false`: animasyonlu görselde yalnızca ilk kare çözülür.
    pub animate: bool,
}

impl DecodeOptions {
    pub const fn trust(&self) -> Trust {
        self.limits.trust
    }

    pub const fn with_trust(mut self, trust: Trust) -> Self {
        self.limits.trust = trust;
        self
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self { limits: Limits::DEFAULT, target: None, animate: true }
    }
}
