//! `framework.toml` şeması. Alan adları ve varsayılanlar `docs/DESIGN.md` ile birebir aynıdır;
//! her bölüm `#[serde(default)]` olduğundan kısmi dosyalar geçerlidir.

use serde::{Deserialize, Serialize};

pub use nuxut_config::locale::{HourCycle, HourCycleSetting, LocaleConfig, LocaleSetting};

use crate::palette::{Choice, ChoiceValue, ColorsConfig};

#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct FrameworkConfig {
    pub colors: ColorsConfig,
    pub shape: ShapeConfig,
    pub shadow: ShadowConfig,
    pub motion: MotionConfig,
    pub blob: BlobConfig,
    pub typography: TypographyConfig,
    pub desktop: DesktopConfig,
    /// `[locale]`: arayüz dili ve biçimler (ADR-0010; tip `nuxut-config`'te).
    pub locale: LocaleConfig,
    /// `[appearance]`: maskot görünürlüğü vb. Diğer tablolar gibi varsayılan dökümde yer alır
    /// (`mascot = true`); kullanıcı dosyasına kendiliğinden yazılmaz — bölümsüz eski dosya aynen
    /// yüklenir (istek, 2026-10-14).
    pub appearance: AppearanceConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShapeConfig {
    pub radius_small: f32,
    pub radius_medium: f32,
    pub radius_large: f32,
    /// Kenar çizgileri: ayrım varsayılan olarak ton ve boşlukla yapılır (DESIGN "Kenarlar ve
    /// ayrım").
    pub outlines: Outlines,
}

impl Default for ShapeConfig {
    fn default() -> Self {
        Self { radius_small: 8.0, radius_medium: 14.0, radius_large: 22.0, outlines: Outlines::None }
    }
}

/// Bileşen kenar çizgisi kipi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outlines {
    /// Çizgi yok; ayrım yüzey tonu, boşluk ve yükseltiyle (varsayılan).
    #[default]
    None,
    /// `outline` rengi düşük opaklıkta.
    Subtle,
    /// Tam kontrastlı çizgi (yüksek kontrast / erişilebilirlik tercihi).
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct ShadowConfig {
    pub enabled: bool,
    pub blur: f32,
    pub offset_y: f32,
    pub opacity: f32,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self { enabled: true, blur: 16.0, offset_y: 2.0, opacity: 0.42 }
    }
}

/// Hareket stili: yay (kesilebilir, hızı korur) ya da süre + eğri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionStyle {
    #[default]
    Spring,
    Curve,
}

/// Hareket ön ayarı: rollerin, stilin ve blob esnemesinin VARSAYILANLARINI belirler. Dosyada
/// açıkça yazılan her anahtar (rol alanı, `style`, `blob.deform`) ön ayarın üstüne yazar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionPreset {
    /// Sakin: hafif sönümlü, taşma ≈ %0.5, belirsiz esneme (varsayılan).
    #[default]
    Calm,
    /// Canlı "puding": belirgin taşma ve esneme.
    Lively,
    /// Çevik: kritik sönümlü, kısa, esnemesiz.
    Snappy,
    /// Hareketsiz: her şey anında (`motion.enabled = false` ile eşdeğer).
    Instant,
}

/// Bir rolün tam değerleri (ön ayardan).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoleValues {
    pub response: f32,
    pub damping: f32,
    pub easing: &'static str,
    pub duration_ms: f32,
}

/// Bir ön ayarın tüm değerleri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresetValues {
    pub enabled: bool,
    pub style: MotionStyle,
    pub spatial: RoleValues,
    pub spatial_fast: RoleValues,
    pub effects: RoleValues,
    /// `blob.deform` varsayılanı.
    pub deform: f32,
}

const fn rv(response: f32, damping: f32, easing: &'static str, duration_ms: f32) -> RoleValues {
    RoleValues { response, damping, easing, duration_ms }
}

impl MotionPreset {
    pub const ALL: [Self; 4] = [Self::Calm, Self::Lively, Self::Snappy, Self::Instant];

    /// Dosyadaki ad (`"calm"`…).
    pub fn name(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Lively => "lively",
            Self::Snappy => "snappy",
            Self::Instant => "instant",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name.trim())
    }

    /// Ön ayarın değerleri (tek kaynak; DESIGN.md "Hareket ön ayarları" tablosu).
    pub fn values(self) -> PresetValues {
        // Eğri stili seçilirse süre/eğri ön ayardan bağımsızdır.
        let curves = (("emphasized", 500.0), ("emphasized-decelerate", 350.0), ("ease", 200.0));
        let p = |s: (f32, f32), f: (f32, f32), e: (f32, f32), deform: f32, enabled: bool| PresetValues {
            enabled,
            style: MotionStyle::Spring,
            spatial: rv(s.0, s.1, curves.0.0, curves.0.1),
            spatial_fast: rv(f.0, f.1, curves.1.0, curves.1.1),
            effects: rv(e.0, e.1, curves.2.0, curves.2.1),
            deform,
        };
        match self {
            Self::Calm => p((0.38, 0.87), (0.26, 0.92), (0.22, 1.0), 0.2, true),
            Self::Lively => p((0.42, 0.70), (0.28, 0.82), (0.22, 1.0), 0.6, true),
            Self::Snappy => p((0.30, 1.0), (0.22, 1.0), (0.22, 1.0), 0.0, true),
            Self::Instant => p((0.38, 0.87), (0.26, 0.92), (0.22, 1.0), 0.0, false),
        }
    }
}

/// Bir hareket rolünün (`spatial`, `spatial_fast`, `effects`) dosyada AÇIKÇA yazılan
/// alanları; `None` = ön ayarın değeri. Yay alanları (`response`, `damping`) yalnızca
/// `spring`, eğri alanları (`easing`, `duration_ms`) yalnızca `curve` stilinde kullanılır.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct RoleConfig {
    /// `None` = `[motion] style`'ı izle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<MotionStyle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub damping: Option<f32>,
    /// Ad ya da `cubic-bezier(x1, y1, x2, y2)` (bkz. `CubicBezier::parse`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub easing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f32>,
}

impl RoleConfig {
    /// Hiçbir alan yazılmamış (tamamen ön ayardan).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Eski ad (yalnızca yay alanları varken).
pub type SpringConfig = RoleConfig;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(into = "MotionPatch")]
pub struct MotionConfig {
    pub preset: MotionPreset,
    /// `false` = her şey anında (ön ayardan bağımsız ana anahtar).
    pub enabled: bool,
    pub speed: f32,
    /// Rollerin varsayılan stili; `None` = ön ayarınki.
    pub style: Option<MotionStyle>,
    pub spatial: RoleConfig,
    pub spatial_fast: RoleConfig,
    pub effects: RoleConfig,
    /// Dosyada tanınmayan ön ayar adı vardı (doğrulama uyarısı için; değer saklanmaz).
    pub unknown_preset: bool,
}

impl MotionConfig {
    /// Yalnızca ön ayar; hiçbir rol ezilmemiş.
    pub fn for_preset(preset: MotionPreset) -> Self {
        Self { preset, ..Self::default() }
    }

    /// Ön ayarın üstüne yazılmış bir rol alanı ya da stil var mı ("Özelleştirilmiş").
    /// `enabled`/`speed` ön ayar değildir, sayılmaz.
    pub fn is_customized(&self) -> bool {
        self.style.is_some() || !(self.spatial.is_empty() && self.spatial_fast.is_empty() && self.effects.is_empty())
    }
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            preset: MotionPreset::Calm,
            enabled: true,
            speed: 1.0,
            style: None,
            spatial: RoleConfig::default(),
            spatial_fast: RoleConfig::default(),
            effects: RoleConfig::default(),
            unknown_preset: false,
        }
    }
}

impl<'de> Deserialize<'de> for MotionConfig {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        Ok(MotionPatch::deserialize(de)?.into())
    }
}

/// Dosyadaki `[motion]`: tanınmayan ön ayar adı dosyayı bozmaz (uyarı + `calm`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct MotionPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<MotionStyle>,
    #[serde(skip_serializing_if = "RoleConfig::is_empty")]
    spatial: RoleConfig,
    #[serde(skip_serializing_if = "RoleConfig::is_empty")]
    spatial_fast: RoleConfig,
    #[serde(skip_serializing_if = "RoleConfig::is_empty")]
    effects: RoleConfig,
}

impl From<MotionPatch> for MotionConfig {
    fn from(p: MotionPatch) -> Self {
        let d = Self::default();
        let preset = p.preset.as_deref().map(MotionPreset::from_name);
        Self {
            preset: preset.flatten().unwrap_or(d.preset),
            unknown_preset: matches!(preset, Some(None)),
            enabled: p.enabled.unwrap_or(d.enabled),
            speed: p.speed.unwrap_or(d.speed),
            style: p.style,
            spatial: p.spatial,
            spatial_fast: p.spatial_fast,
            effects: p.effects,
        }
    }
}

impl From<MotionConfig> for MotionPatch {
    fn from(c: MotionConfig) -> Self {
        Self {
            preset: Some(c.preset.name().to_owned()),
            enabled: Some(c.enabled),
            speed: Some(c.speed),
            style: c.style,
            spatial: c.spatial,
            spatial_fast: c.spatial_fast,
            effects: c.effects,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct BlobConfig {
    pub smoothing: f32,
    /// `None` = hareket ön ayarınınki.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deform: Option<f32>,
}

impl Default for BlobConfig {
    fn default() -> Self {
        Self { smoothing: 20.0, deform: None }
    }
}

/// `[typography]` (ADR-0006): rol başına aile + ağırlık + ölçek. Rolde verilmeyen alan rolün
/// varsayılanını alır ([`crate::Theme::from_config`]).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct TypographyConfig {
    /// Taban boy (mantıksal px); tip ölçeği buradan türetilir.
    pub size: f32,
    pub icon_set: IconSet,
    pub ui: FontRoleConfig,
    pub display: FontRoleConfig,
    pub document: FontRoleConfig,
    pub mono: FontRoleConfig,
    pub icon: FontRoleConfig,
    /// Eski şema: `ui.family`'ye uyarıyla eşlenir.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    /// Eski şema: `ui.weight`'e uyarıyla eşlenir.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
}

impl Default for TypographyConfig {
    fn default() -> Self {
        Self {
            size: crate::TEXT_SIZE_REFERENCE,
            icon_set: IconSet::Material,
            ui: FontRoleConfig::default(),
            display: FontRoleConfig::default(),
            document: FontRoleConfig::default(),
            mono: FontRoleConfig::default(),
            icon: FontRoleConfig::default(),
            family: None,
            weight: None,
        }
    }
}

/// Bir tipografi rolü. `None` alanlar rolün varsayılanıdır. Eski şemadaki düz dize
/// (`mono = "…"`) aile olarak okunur ve `legacy` işaretlenir.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct FontRoleConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip)]
    pub legacy: bool,
}

impl<'de> Deserialize<'de> for FontRoleConfig {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Table {
            family: Option<String>,
            weight: Option<u16>,
            scale: Option<f32>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Family(String),
            Table(Table),
        }
        Ok(match Raw::deserialize(de)? {
            Raw::Family(family) => Self { family: Some(family), legacy: true, ..Self::default() },
            Raw::Table(t) => Self { family: t.family, weight: t.weight, scale: t.scale, legacy: false },
        })
    }
}

/// İkon seti: semantik ikon adını glife çeviren katman (ADR-0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IconSet {
    /// Material Symbols Rounded ligatürleri; `FILL` ekseniyle dolgu animasyonu.
    #[default]
    Material,
    /// Nerd Font (`nf-md-*`) kod noktaları; dolgu = çizgi/dolu glif çifti.
    Nerd,
}

/// Qt uygulamalarının tema yolu (shell ADR-0004 §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QtBackend {
    /// plasma-integration kuruluysa `kde`, değilse qt6ct.
    #[default]
    Auto,
    /// `QT_QPA_PLATFORMTHEME=kde` (canlı; birincil yol).
    Kde,
    /// `QT_QPA_PLATFORMTHEME=qt6ct` (Qt5: qt5ct; yeniden başlatma ister).
    Qt6ct,
}

impl QtBackend {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Kde, Self::Qt6ct];
}

impl ChoiceValue for QtBackend {
    /// `qtct` eski ad olarak kabul edilir.
    fn from_name(s: &str) -> Option<Self> {
        match s {
            "auto" => Some(Self::Auto),
            "kde" => Some(Self::Kde),
            "qt6ct" | "qtct" => Some(Self::Qt6ct),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Kde => "kde",
            Self::Qt6ct => "qt6ct",
        }
    }
}

/// `[desktop]`: diğer uygulamaların görünümü (birleşik görünüm, shell ADR-0004 §12-3, §17-1).
/// Bu ham şemadır; doğrulanmış hali [`crate::Desktop`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct DesktopConfig {
    /// Birleşik görünüm (varsayılan `true`); `false` iken hiçbir dış hedef yazılmaz.
    pub unified: bool,
    /// İkon teması dizin adı; `""` = önerilen.
    pub icon_theme: String,
    /// İmleç teması dizin adı; `""` = önerilen.
    pub cursor_theme: String,
    /// İmleç boyu (mantıksal piksel), [`CURSOR_SIZE_MIN`]..=[`CURSOR_SIZE_MAX`].
    pub cursor_size: u32,
    pub qt_backend: Choice<QtBackend>,
}

/// İmleç boyu sınırları ve varsayılanı.
pub const CURSOR_SIZE_MIN: u32 = 16;
pub const CURSOR_SIZE_MAX: u32 = 96;
pub const CURSOR_SIZE_DEFAULT: u32 = 24;
/// Tema adının en uzun hali (bayt).
pub const MAX_THEME_NAME: usize = 128;

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            unified: true,
            icon_theme: String::new(),
            cursor_theme: String::new(),
            cursor_size: CURSOR_SIZE_DEFAULT,
            qt_backend: Choice::new(QtBackend::Auto),
        }
    }
}

/// Tema dizin adı geçerli mi: ≤ [`MAX_THEME_NAME`] bayt, `.` ile başlamaz, yalnızca harf,
/// rakam ve ` -_.+` (dış dosyalara — ini, dconf, ortam — güvenle yazılır).
pub fn valid_theme_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_THEME_NAME
        && !s.starts_with('.')
        && s.chars().all(|c| c.is_alphanumeric() || " -_.+".contains(c))
}

/// `[appearance]`: arayüz kişiliği tercihleri. Varsayılan döküm (`FrameworkConfig::default()`
/// serileştirmesi) bölümü `mascot = true` ile taşır; bölüm/anahtar yoksa varsayılan geçerlidir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Maskot (Fiyu) boş durum/hata/karşılama yüzeylerinde görünür (varsayılan `true`).
    /// `false` iken `nuxut_ui::Mascot` hiçbir şey çizmez ve yer kaplamaz (docs/design/mascot.md;
    /// karar: kullanıcı, 2026-10-13).
    pub mascot: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self { mascot: true }
    }
}

impl AppearanceConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
