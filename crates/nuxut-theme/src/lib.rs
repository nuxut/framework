//! Nuxut tasarım token'ları.
//!
//! `~/.config/nuxut/framework.toml` → [`FrameworkConfig`] (ham şema) → doğrulama/sınırlama →
//! [`Theme`] (bileşenlerin okuduğu token'lar). Bileşenler asla sabit renk/yarıçap/süre
//! içermez; her şey buradan gelir.

#![forbid(unsafe_code)]

mod animated;
pub mod color_theme;
pub mod contrast;
mod palette;
mod schema;

pub use animated::AnimatedTheme;
pub use color_theme::{ColorTheme, ColorThemeId, recolor_svg};
pub use contrast::{ContrastMode, contrast_ratio};

use std::path::Path;
use std::time::Duration;


use nuxut_core::{Color, Corners};
use nuxut_motion::{CubicBezier, Curve, Deform, Spring};

pub use nuxut_config::{ConfigError, Loaded};
pub use palette::{
    Choice, ChoiceValue, ColorMode, ColorRole, ColorSource, ColorsConfig, GeneratedPalette, MAX_MATERIAL_ROLES, PALETTE_FILE, Palette, PaletteMeta, PalettePatch, SUCCESS_SEED, ThemeMode,
    palette_path,
    resolve_palette,
};
pub use schema::{
    AppearanceConfig, CURSOR_SIZE_DEFAULT, CURSOR_SIZE_MAX, CURSOR_SIZE_MIN, DesktopConfig, MAX_THEME_NAME, QtBackend, valid_theme_name,
    BlobConfig, FontRoleConfig, FrameworkConfig, HourCycle, HourCycleSetting, IconSet, LocaleConfig, LocaleSetting, MotionConfig, MotionPreset, MotionStyle, Outlines, PresetValues, RoleConfig, RoleValues,
    ShadowConfig,
    ShapeConfig, SpringConfig,
    TypographyConfig,
};

/// Config dosyasının adı (`<config_dir>/framework.toml`).
pub const CONFIG_NAME: &str = "framework";

/// Boşluk ızgarasının birimi (mantıksal piksel).
pub const SPACING_GRID: f32 = 4.0;

/// State layer opaklıkları (DESIGN.md: hover %8, basılı %12).
pub const STATE_HOVER: f32 = 0.08;
pub const STATE_PRESSED: f32 = 0.12;

/// Zamanlama token'ları (DESIGN.md "Zamanlama"). Hareket değildir: `motion.enabled`/`speed`
/// bunları etkilemez (hareket kapalıyken de ipucu hemen fırlamaz).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    /// Fare üzerine geldikten sonra ipucunun belirmesine kadar bekleme.
    pub hover_delay: Duration,
    /// İpucu kapandıktan sonraki "ısınmış" pencere: bu süre içinde başka öğeye gelinirse
    /// ipucu beklemeden belirir.
    pub hover_warm: Duration,
    /// Yerinde onayda (`ConfirmButton`) ikinci basış için bekleme; dolunca vazgeçilir.
    pub confirm_window: Duration,
    /// Onay kurulduktan sonra ikinci basışın yok sayıldığı kısa süre: ilk basışın hemen
    /// ardından gelen yanlışlıkla çift tık (ya da tuş tekrarı) onaylamaz.
    pub confirm_guard: Duration,
    /// "Satırı parlat" vurgusunun (ör. arama sonucuna gidilince) tam opaklıkta kaldığı süre;
    /// ardından `motion.effects` ile söner. Hareket kapalıyken de vurgu bu süre görünür.
    pub flash_hold: Duration,
}

impl Timing {
    pub const DEFAULT: Self = Self {
        hover_delay: Duration::from_millis(500),
        hover_warm: Duration::from_millis(800),
        confirm_window: Duration::from_secs(4),
        confirm_guard: Duration::from_millis(400),
        flash_hold: Duration::from_millis(900),
    };
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub radius_small: f32,
    pub radius_medium: f32,
    pub radius_large: f32,
    pub outlines: Outlines,
}

/// `outlines = "subtle"` iken `colors.outline` opaklığı (ton zaten ince).
pub const OUTLINE_SUBTLE_ALPHA: f32 = 1.0;

/// `outlines = "full"` iken kenar `on_surface_variant` bu opaklıkta: yüzeylere karşı ≥ 3:1
/// (WCAG 1.4.11). `colors.outline` koyu, ince bir ton olduğu için "full" ona dayanmaz.
pub const OUTLINE_FULL_ALPHA: f32 = 0.7;

impl Shape {
    pub fn small(&self) -> Corners {
        Corners::uniform(self.radius_small)
    }
    pub fn medium(&self) -> Corners {
        Corners::uniform(self.radius_medium)
    }
    pub fn large(&self) -> Corners {
        Corners::uniform(self.radius_large)
    }
}

/// Gölge token'ı. `enabled = false` ise [`Theme::shadow`] `None` verir.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub enabled: bool,
    pub blur: f32,
    pub offset_y: f32,
    /// `colors.shadow` × `opacity`, düz (straight) alpha.
    pub color: Color,
}

/// Hareket token'ları: `enabled`, `speed` ve rol stili (yay ya da eğri) uygulanmış
/// halleri. Bileşen onları olduğu gibi `Animated::new(v, theme.motion.spatial)` ile kullanır;
/// hangi stilin seçildiğini bilmez.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    pub enabled: bool,
    pub speed: f32,
    /// Konum/boyut — varsayılan az sönümlü yay (puding).
    pub spatial: nuxut_motion::Motion,
    pub spatial_fast: nuxut_motion::Motion,
    /// Opaklık/renk — taşmaz.
    pub effects: nuxut_motion::Motion,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blob {
    /// Birleşim filetosu yarıçapı (mantıksal piksel).
    pub smoothing: f32,
    /// Hızdan gelen esneme şiddeti (0 = kapalı). Hareket kapalıysa 0.
    pub deform: f32,
    /// Esnemenin oturma yayı (`motion.speed` uygulanmış).
    pub deform_spring: Spring,
}

/// Doğrulanmış tipografi (ADR-0006). Bileşenler aileyi yalnızca rol üzerinden alır.
#[derive(Debug, Clone, PartialEq)]
pub struct Typography {
    /// Taban boy (mantıksal px).
    pub size: f32,
    pub icon_set: IconSet,
    /// Arayüz metni.
    pub ui: FontRole,
    /// Büyük metin (saat, başlık).
    pub display: FontRole,
    /// Uzun okuma metni.
    pub document: FontRole,
    /// Eş aralıklı.
    pub mono: FontRole,
    /// İkon fontu; `family` boşsa ikon setinin fontu ([`Typography::icon_family`]).
    pub icon: FontRole,
}

/// Bir tipografi rolü: aile, ağırlık (100–1000), boyut çarpanı.
#[derive(Debug, Clone, PartialEq)]
pub struct FontRole {
    pub family: String,
    pub weight: u16,
    pub scale: f32,
}

/// `material` setinin fontu.
pub const MATERIAL_SYMBOLS_FAMILY: &str = "Material Symbols Rounded";

/// `[typography] size` varsayılanı (mantıksal px) ve metin ölçeğinin referansı: bu boyda
/// [`Typography::text_scale`] 1.0'dır (docs/design/accessibility.md §3).
pub const TEXT_SIZE_REFERENCE: f32 = 13.0;

impl Typography {
    /// Metin ölçeği: taban boyun referansa oranı (`size / 13`). Dış hedeflere (GTK
    /// `text-scaling-factor`, Qt) aktarım `nuxut_palette::text_scale`'dedir. Doğrulanmış boy
    /// 4..=128 px olduğundan sonlu ve pozitiftir.
    pub fn text_scale(&self) -> f32 {
        self.size / TEXT_SIZE_REFERENCE
    }

    /// İkonların çizileceği aile: `icon.family` ya da setin fontu (`nerd` → `mono` ailesi,
    /// varsayılanı Nerd Font).
    pub fn icon_family(&self) -> &str {
        if !self.icon.family.is_empty() {
            return &self.icon.family;
        }
        match self.icon_set {
            IconSet::Material => MATERIAL_SYMBOLS_FAMILY,
            IconSet::Nerd => &self.mono.family,
        }
    }
}

/// Rol varsayılanları (DESIGN.md `[typography]`).
const ROLE_DEFAULTS: [(&str, &str, u16); 5] = [
    ("ui", "Inter", 500),
    ("display", "Inter", 600),
    ("document", "Source Serif 4", 400),
    ("mono", "JetBrainsMono Nerd Font", 400),
    ("icon", "", 400),
];

/// Doğrulanmış `[desktop]` seçimleri. Tema adı `None` = önerilen (seçimi tüketici — shell — yapar).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop {
    pub unified: bool,
    pub icon_theme: Option<String>,
    pub cursor_theme: Option<String>,
    /// [`CURSOR_SIZE_MIN`]..=[`CURSOR_SIZE_MAX`].
    pub cursor_size: u32,
    pub qt_backend: QtBackend,
}

impl Default for Desktop {
    fn default() -> Self {
        Self { unified: true, icon_theme: None, cursor_theme: None, cursor_size: CURSOR_SIZE_DEFAULT, qt_backend: QtBackend::Auto }
    }
}

/// `[appearance]` seçimleri (doğrulanmış; şu an yalnızca bool'lar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Appearance {
    /// Maskot (Fiyu) görünür mü. `false` iken `nuxut_ui::Mascot` çizmez, ölçüsü sıfırdır.
    pub mascot: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { mascot: true }
    }
}

/// Bileşenlerin okuduğu, doğrulanmış tasarım token'ları.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Çözülmüş kip (`[colors] mode`, `auto` çözülmüş; bkz. [`ColorsConfig::resolve_mode`]);
    /// `colors` bu kipin paletidir.
    pub mode: ThemeMode,
    pub colors: Palette,
    /// Seçili Nuxut renk teması (`[colors] theme`); renkler duvar kağıdından geliyorsa
    /// (`source = "wallpaper"`) `None`. Yerleşik duvar kağıtları `Some` iken bu temaya
    /// yeniden renklendirilir ([`ColorTheme::recolor`]).
    pub color_theme: Option<ColorThemeId>,
    /// Karşıtlık kipi (`[colors] contrast`). `High` iken `colors` zaten yüksek karşıtlıklıdır
    /// ([`Palette::high_contrast`]), `shape.outlines` en az `subtle`'dır ve [`Self::outline`]
    /// tam opaktır. Bileşenler ek eşik isterse (ör. kendi karışım renkleri) buna bakar.
    pub contrast: ContrastMode,
    pub shape: Shape,
    pub shadow: Shadow,
    pub motion: Motion,
    pub blob: Blob,
    pub typography: Typography,
    pub timing: Timing,
    /// Diğer uygulamaların görünümü (`[desktop]`, doğrulanmış).
    pub desktop: Desktop,
    /// `[appearance]`: maskot görünürlüğü. Görsel token sayılır: değişince `theme_changed`
    /// gelir (bileşenler canlı gizlenir/gösterilir).
    pub appearance: Appearance,
    /// `[locale]` (ADR-0010): arayüz dili ve biçimler. Görsel token değildir; runtime bu alan
    /// değişince `App::locale_changed` çağırır (yalnızca bu değiştiyse `theme_changed` gelmez).
    pub locale: LocaleConfig,
}

impl Default for Theme {
    fn default() -> Self {
        Self::from_config(&FrameworkConfig::default()).0
    }
}

impl Theme {
    /// Azaltılmış hareket (docs/design/accessibility.md §2): Ayarlar → Hareket "Kapalı"
    /// (`[motion] preset = "instant"`) ya da `[motion] enabled = false`. `true` iken her hareket
    /// rolü anında (`Motion::INSTANT`), blob esnemesi kapalı; bileşenler sonsuz döngüleri
    /// (spinner) durağan çizer, kaydırma ataleti yoktur. Tek kaynak: shell portal arka ucu
    /// `org.freedesktop.appearance reduced-motion`'ı ve look `gtk-enable-animations`'ı bundan
    /// türetir.
    pub fn reduced_motion(&self) -> bool {
        !self.motion.enabled
    }

    /// framework.toml **var ama** okunamadı/ayrıştırılamadı (sözdizimi, yanlış tür, çok büyük,
    /// G/Ç) iken kullanılacak tema: varsayılan + `desktop.unified = false`. Bozuk dosya
    /// kullanıcının niyetini (ör. `unified = false`) okutmaz; varsayılan açık birleşik görünüm
    /// dış hedefleri (GTK/Qt/dconf/imleç) ezmesin diye kapalı sayılır. Dosya ya da anahtar
    /// **yoksa** varsayılan (`Theme::default`, açık) geçerlidir. Runtime açılışta yükleme hatasında
    /// bunu kurar; çalışırken gelen hata son geçerli temayı korur.
    pub fn broken_config() -> Self {
        let mut theme = Self::default();
        theme.desktop.unified = false;
        theme
    }

    /// Ham config'i doğrular, sınır dışı değerleri sınırlar. İkinci değer kullanıcıya
    /// gösterilecek uyarılardır. Üretilmiş palet katmanı yoktur (bkz. [`Self::from_layers`]).
    pub fn from_config(cfg: &FrameworkConfig) -> (Self, Vec<String>) {
        Self::from_layers(cfg, None)
    }

    /// Üretilmiş palet dosyasıyla (önerilen yol): kip [`ColorsConfig::resolve_mode`] ile
    /// çözülür (`auto`'da paletin kayıtlı kipi, [`GeneratedPalette::recorded_mode`]), sonra o
    /// kipin rolleri ([`GeneratedPalette::colors_for`]) katman olur. `generated` yalnızca
    /// `colors.source = "wallpaper"` iken kullanılır.
    pub fn from_generated(cfg: &FrameworkConfig, generated: Option<&GeneratedPalette>) -> (Self, Vec<String>) {
        let generated = generated.filter(|_| cfg.colors.source == ColorSource::Wallpaper);
        let mode = cfg.colors.resolve_mode(generated.and_then(GeneratedPalette::recorded_mode));
        let mut warnings = Vec::new();
        let patch = generated.map(|g| {
            let (patch, other_mode) = g.colors_for(mode);
            if other_mode {
                warnings.push(format!("üretilmiş palette {} kipi yok; mevcut kip kullanıldı", mode.name()));
            }
            patch
        });
        let (theme, mut more) = Self::build(cfg, mode, patch);
        warnings.append(&mut more);
        (theme, warnings)
    }

    /// Renk katmanlarıyla: kipin varsayılanı ← `generated` (çözülmüş kipin rolleri; yalnızca
    /// `colors.source = "wallpaper"` iken) ← framework.toml'daki açık renkler. Palet dosyasının
    /// kaydı elde yoksa: `auto` kipte paletin kipi `generated.surface` parlaklığından çıkarılır.
    /// Dosya varsa [`Self::from_generated`] tercih edilir.
    pub fn from_layers(cfg: &FrameworkConfig, generated: Option<&PalettePatch>) -> (Self, Vec<String>) {
        let mode = cfg.colors.resolve_mode(generated.and_then(|p| p.surface).map(ThemeMode::of_surface));
        Self::build(cfg, mode, generated)
    }

    fn build(cfg: &FrameworkConfig, mode: ThemeMode, generated: Option<&PalettePatch>) -> (Self, Vec<String>) {
        let mut v = Validator::default();
        let d = FrameworkConfig::default();
        let colors = resolve_palette(&cfg.colors, mode, generated);
        if cfg.colors.mode.invalid {
            v.warnings.push("colors.mode geçersiz (auto | dark | light); 'auto' kullanıldı".into());
        }
        if cfg.colors.theme.invalid {
            let names: Vec<_> = ColorThemeId::ALL.iter().map(|t| t.name()).collect();
            v.warnings.push(format!("colors.theme geçersiz ({}); 'blue' kullanıldı", names.join(" | ")));
        }
        let contrast = cfg.colors.contrast.value;
        if cfg.colors.contrast.invalid {
            v.warnings.push("colors.contrast geçersiz (standard | high); 'standard' kullanıldı".into());
        }
        let desktop = v.desktop(&cfg.desktop);
        let locale = v.locale(&cfg.locale);

        let shape = Shape {
            radius_small: v.range("shape.radius_small", cfg.shape.radius_small, d.shape.radius_small, 0.0, 256.0),
            radius_medium: v.range("shape.radius_medium", cfg.shape.radius_medium, d.shape.radius_medium, 0.0, 256.0),
            radius_large: v.range("shape.radius_large", cfg.shape.radius_large, d.shape.radius_large, 0.0, 256.0),
            // Yüksek karşıtlıkta bileşen sınırı her zaman çizilir (WCAG 1.4.11): `none` → `subtle`.
            outlines: if contrast.is_high() && cfg.shape.outlines == Outlines::None { Outlines::Subtle } else { cfg.shape.outlines },
        };

        let opacity = v.range("shadow.opacity", cfg.shadow.opacity, d.shadow.opacity, 0.0, 1.0);
        let shadow = Shadow {
            enabled: cfg.shadow.enabled,
            blur: v.range("shadow.blur", cfg.shadow.blur, d.shadow.blur, 0.0, 128.0),
            offset_y: v.range("shadow.offset_y", cfg.shadow.offset_y, d.shadow.offset_y, -64.0, 64.0),
            color: colors.shadow.multiply_alpha(opacity),
        };

        // Hareket: ön ayar varsayılanları verir; dosyada yazılan her anahtar üstüne yazar.
        if cfg.motion.unknown_preset {
            v.warnings.push(format!("motion.preset bilinmiyor; '{}' kullanıldı", MotionPreset::default().name()));
        }
        let preset = cfg.motion.preset.values();
        let speed = v.range("motion.speed", cfg.motion.speed, d.motion.speed, 0.05, 20.0);
        let enabled = cfg.motion.enabled && preset.enabled;
        let global_style = cfg.motion.style.unwrap_or(preset.style);
        let mut role = |name: &str, r: &RoleConfig, def: &RoleValues| -> nuxut_motion::Motion {
            if !enabled {
                return nuxut_motion::Motion::INSTANT;
            }
            let motion: nuxut_motion::Motion = match r.style.unwrap_or(global_style) {
                MotionStyle::Spring => {
                    let response = r.response.map_or(def.response, |x| {
                        v.range(&format!("motion.{name}.response"), x, def.response, 0.01, 5.0)
                    });
                    let damping = r.damping.map_or(def.damping, |x| {
                        v.range(&format!("motion.{name}.damping"), x, def.damping, 0.05, 4.0)
                    });
                    Spring::new(response, damping).into()
                }
                MotionStyle::Curve => {
                    let easing = CubicBezier::parse(r.easing.as_deref().unwrap_or(def.easing)).unwrap_or_else(|e| {
                        v.warnings.push(format!("motion.{name}.easing: {e}; '{}' kullanıldı", def.easing));
                        CubicBezier::parse(def.easing).unwrap_or(CubicBezier::STANDARD)
                    });
                    let ms = r.duration_ms.map_or(def.duration_ms, |x| {
                        v.range(&format!("motion.{name}.duration_ms"), x, def.duration_ms, 0.0, 5000.0)
                    });
                    Curve::new(easing, ms / 1000.0).into()
                }
            };
            motion.scaled(speed)
        };
        let motion = Motion {
            enabled,
            speed,
            spatial: role("spatial", &cfg.motion.spatial, &preset.spatial),
            spatial_fast: role("spatial_fast", &cfg.motion.spatial_fast, &preset.spatial_fast),
            effects: role("effects", &cfg.motion.effects, &preset.effects),
        };

        let deform = cfg.blob.deform.map_or(preset.deform, |x| v.range("blob.deform", x, preset.deform, 0.0, 2.0));
        let blob = Blob {
            smoothing: v.range("blob.smoothing", cfg.blob.smoothing, d.blob.smoothing, 0.0, 128.0),
            deform: if enabled { deform } else { 0.0 },
            deform_spring: if enabled { Deform::DEFAULT_SPRING.scaled(speed) } else { Spring::INSTANT },
        };

        let typography = v.typography(&cfg.typography, &d.typography);

        let appearance = Appearance { mascot: cfg.appearance.mascot };
        let color_theme = (cfg.colors.source == ColorSource::Static).then_some(cfg.colors.theme.value);
        let theme = Self { mode, colors, color_theme, contrast, shape, shadow, motion, blob, typography, timing: Timing::DEFAULT, desktop, appearance, locale };
        (theme, v.warnings)
    }

    /// `[locale]` dışındaki her token aynı mı (dil/biçim değişimi görsel değişim değildir).
    pub fn same_visuals(&self, other: &Self) -> bool {
        let Self { mode, colors, color_theme, contrast, shape, shadow, motion, blob, typography, timing, desktop, appearance, locale: _ } = self;
        *mode == other.mode
            && *contrast == other.contrast
            && *colors == other.colors
            && *color_theme == other.color_theme
            && *shape == other.shape
            && *shadow == other.shadow
            && *motion == other.motion
            && *blob == other.blob
            && *typography == other.typography
            && *timing == other.timing
            && *desktop == other.desktop
            && *appearance == other.appearance
    }

    /// `<config_dir>/framework.toml`'u (ve `source = "wallpaper"` ise
    /// `<state_dir>/palette.toml`'u) yükler. framework.toml yoksa varsayılan tema; bozuksa
    /// hata (çağıran son geçerli temayı korur). Bozuk/eksik palet dosyası hata değildir:
    /// uyarı verilir ve palet katmanı atlanır.
    pub fn load() -> Result<Loaded<Self>, ConfigError> {
        let palette = palette_path();
        match nuxut_config::config_path(CONFIG_NAME) {
            Some(path) => Self::load_layers(&path, palette.as_deref()),
            None => Ok(Loaded { value: Self::default(), warnings: Vec::new(), from_file: false }),
        }
    }

    /// Yalnızca framework.toml (palet katmanı yok).
    pub fn load_from(path: &Path) -> Result<Loaded<Self>, ConfigError> {
        Self::load_layers(path, None)
    }

    /// framework.toml + (opsiyonel) üretilmiş palet dosyası.
    /// Durumsuz: bozuk palet dosyasında palet katmanı atlanır. Uzun ömürlü süreçler (runtime)
    /// son geçerli paleti koruyan [`ThemeLoader`]'ı kullanır.
    pub fn load_layers(config: &Path, palette: Option<&Path>) -> Result<Loaded<Self>, ConfigError> {
        Self::load_layers_keeping(config, palette, &mut None)
    }

    fn load_layers_keeping(
        config: &Path,
        palette: Option<&Path>,
        last_valid: &mut Option<GeneratedPalette>,
    ) -> Result<Loaded<Self>, ConfigError> {
        let loaded: Loaded<FrameworkConfig> = nuxut_config::load(config)?;
        let mut all = loaded.warnings;
        let generated = match palette {
            Some(p) if loaded.value.colors.source == ColorSource::Wallpaper => match GeneratedPalette::load(p) {
                Ok(g) if g.from_file => {
                    all.extend(g.warnings.into_iter().map(|w| format!("{}: {w}", p.display())));
                    let mut kept = g.value;
                    kept.material.clear();
                    *last_valid = Some(kept);
                    last_valid.as_ref()
                }
                Ok(_) => {
                    // Dosya silindi: palet katmanı kalkar.
                    all.push(format!("colors.source = \"wallpaper\" ama {} yok", p.display()));
                    *last_valid = None;
                    None
                }
                Err(e) => {
                    // Bozuk dosya: son geçerli palet korunur (değişmez 7).
                    let kept = if last_valid.is_some() { "son geçerli palet korundu" } else { "palet katmanı atlandı" }; // i18n: tanı üretilmiş palet okuma uyarısının parçası
                    all.push(format!("üretilmiş palet okunamadı, {kept}: {e}")); // i18n: tanı üretilmiş palet okuma uyarısı
                    last_valid.as_ref()
                }
            },
            _ => None,
        };
        let (theme, mut warnings) = Self::from_generated(&loaded.value, generated);
        all.append(&mut warnings);
        if !all.is_empty() {
            // Değişmez 9: metinler (değer, yol) yalnızca `Loaded::warnings` ile UI'a gider; loga
            // sayı ve anahtar adları.
            tracing::warn!(count = all.len(), keys = ?warning_keys(&all), "framework.toml uyarıları");
        }
        Ok(Loaded { value: theme, warnings: all, from_file: loaded.from_file })
    }

    /// Metinden (testler ve gömülü temalar için).
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), String> {
        let (cfg, mut warnings): (FrameworkConfig, _) = nuxut_config::parse_str(text)?;
        let (theme, mut more) = Self::from_config(&cfg);
        warnings.append(&mut more);
        Ok((theme, warnings))
    }

    /// Bileşen kenar çizgisinin rengi; `outlines = "none"` iken `None` (çizme). Bileşenler
    /// kenarı YALNIZCA bu yardımcıyla çizer. Klavye odak halkası bundan bağımsızdır.
    pub fn outline(&self) -> Option<Color> {
        match self.shape.outlines {
            Outlines::None => None,
            Outlines::Subtle => Some(self.colors.outline.multiply_alpha(OUTLINE_SUBTLE_ALPHA)),
            Outlines::Full => Some(self.colors.on_surface_variant.multiply_alpha(if self.contrast.is_high() { 1.0 } else { OUTLINE_FULL_ALPHA })),
        }
    }

    /// Boşluk ızgarası: `n` × 4 mantıksal piksel (DESIGN.md: 4'ün katları).
    pub fn space(&self, n: u32) -> f32 {
        n as f32 * SPACING_GRID
    }

    /// İki tema arası renk geçişi; renk dışındaki token'lar `to`'dan alınır.
    pub fn lerp_colors(&self, to: &Self, t: f32) -> Self {
        Self { colors: self.colors.lerp(&to.colors, t), ..to.clone() }
    }
}

/// Tema yükleyicisi: framework.toml + üretilmiş palet. Bozuk `palette.toml` geldiğinde son
/// geçerli paleti korur (silinirse palet katmanı kalkar). Runtime her değişiklikte `load` çağırır.
#[derive(Debug, Clone, Default)]
pub struct ThemeLoader {
    config: Option<std::path::PathBuf>,
    palette: Option<std::path::PathBuf>,
    last_palette: Option<GeneratedPalette>,
}

impl ThemeLoader {
    /// Varsayılan yollar: `<config_dir>/framework.toml`, `<state_dir>/palette.toml`.
    pub fn new() -> Self {
        Self::with_paths(nuxut_config::config_path(CONFIG_NAME), palette_path())
    }

    pub fn with_paths(config: Option<std::path::PathBuf>, palette: Option<std::path::PathBuf>) -> Self {
        Self { config, palette, last_palette: None }
    }

    pub fn load(&mut self) -> Result<Loaded<Theme>, ConfigError> {
        match &self.config {
            Some(c) => Theme::load_layers_keeping(c, self.palette.as_deref(), &mut self.last_palette),
            None => Ok(Loaded { value: Theme::default(), warnings: Vec::new(), from_file: false }),
        }
    }
}

#[derive(Default)]
struct Validator {
    warnings: Vec<String>,
}

impl Validator {
    fn range(&mut self, key: &str, value: f32, default: f32, min: f32, max: f32) -> f32 {
        if !value.is_finite() {
            self.warnings.push(format!("{key} = {value} geçersiz; varsayılan {default} kullanıldı"));
            return default;
        }
        let clamped = value.clamp(min, max);
        if clamped != value {
            self.warnings.push(format!("{key} = {value} aralık dışı [{min}, {max}]; {clamped} kullanıldı"));
        }
        clamped
    }

    fn desktop(&mut self, cfg: &DesktopConfig) -> Desktop {
        let mut theme = |key: &str, value: &str| -> Option<String> {
            if value.is_empty() {
                return None;
            }
            if valid_theme_name(value) {
                return Some(value.to_owned());
            }
            // Değer (kullanıcı metni) uyarıya girmez; yalnızca anahtar.
            self.warnings.push(format!("desktop.{key} geçersiz ad; önerilen tema kullanıldı"));
            None
        };
        let icon_theme = theme("icon_theme", &cfg.icon_theme);
        let cursor_theme = theme("cursor_theme", &cfg.cursor_theme);
        let cursor_size = cfg.cursor_size.clamp(CURSOR_SIZE_MIN, CURSOR_SIZE_MAX);
        if cursor_size != cfg.cursor_size {
            self.warnings.push(format!(
                "desktop.cursor_size = {} aralık dışı [{CURSOR_SIZE_MIN}, {CURSOR_SIZE_MAX}]; {cursor_size} kullanıldı",
                cfg.cursor_size
            ));
        }
        if cfg.qt_backend.invalid {
            self.warnings.push("desktop.qt_backend geçersiz (auto | kde | qt6ct); 'auto' kullanıldı".into());
        }
        Desktop { unified: cfg.unified, icon_theme, cursor_theme, cursor_size, qt_backend: cfg.qt_backend.value }
    }

    /// `[locale]`: geçersiz değer şemada `auto`'ya dönmüştür; burada yalnızca uyarı (değer yok).
    fn locale(&mut self, cfg: &LocaleConfig) -> LocaleConfig {
        for (key, setting) in [("language", &cfg.language), ("formats", &cfg.formats)] {
            if setting.invalid {
                self.warnings.push(format!("locale.{key} geçersiz; 'auto' kullanıldı"));
            }
        }
        if cfg.hour_cycle.invalid {
            self.warnings.push("locale.hour_cycle geçersiz (auto | h23 | h12); 'auto' kullanıldı".into());
        }
        cfg.clone()
    }

    fn weight(&mut self, key: &str, value: u16) -> u16 {
        let clamped = value.clamp(100, 1000);
        if clamped != value {
            self.warnings.push(format!("{key} = {value} aralık dışı [100, 1000]; {clamped} kullanıldı"));
        }
        clamped
    }

    /// `[typography]`: roller, eski şemanın eşlenmesi (uyarıyla), sınırlar.
    fn typography(&mut self, cfg: &TypographyConfig, d: &TypographyConfig) -> Typography {
        let size = self.range("typography.size", cfg.size, d.size, 4.0, 128.0);
        let mut ui = cfg.ui.clone();
        if let Some(family) = &cfg.family {
            self.warnings.push("typography.family eskidi; typography.ui.family kullanın".into());
            ui.family = ui.family.or_else(|| Some(family.clone()));
        }
        if let Some(weight) = cfg.weight {
            self.warnings.push("typography.weight eskidi; typography.ui.weight kullanın".into());
            ui.weight = ui.weight.or(Some(weight));
        }
        let configs = [&ui, &cfg.display, &cfg.document, &cfg.mono, &cfg.icon];
        let [ui, display, document, mono, icon] = std::array::from_fn(|i| {
            let (name, family, weight) = ROLE_DEFAULTS[i];
            self.font_role(name, configs[i], family, weight)
        });
        Typography { size, icon_set: cfg.icon_set, ui, display, document, mono, icon }
    }

    fn font_role(&mut self, name: &str, cfg: &FontRoleConfig, family: &str, weight: u16) -> FontRole {
        if cfg.legacy {
            self.warnings.push(format!("typography.{name} düz dize olarak eskidi; typography.{name}.family kullanın"));
        }
        let family = match &cfg.family {
            // İkon rolünde boş = ikon setinin fontu.
            Some(f) if name == "icon" && f.trim().is_empty() => String::new(),
            Some(f) => self.family(&format!("typography.{name}.family"), f, family),
            None => family.to_owned(),
        };
        FontRole {
            family,
            weight: cfg.weight.map_or(weight, |w| self.weight(&format!("typography.{name}.weight"), w)),
            scale: cfg.scale.map_or(1.0, |s| self.range(&format!("typography.{name}.scale"), s, 1.0, 0.5, 4.0)),
        }
    }

    fn family(&mut self, key: &str, value: &str, default: &str) -> String {
        let value = value.trim();
        if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
            self.warnings.push(format!("{key} geçersiz; '{default}' kullanıldı"));
            return default.to_owned();
        }
        value.to_owned()
    }
}

/// Uyarı metinlerinin başındaki config anahtarları (`typography.ui.family` gibi; noktalı,
/// `[a-z0-9_.]`). Değer ya da yol taşımaz: metin başında yalnızca koddaki sabit anahtar adı
/// durur; anahtarla başlamayan uyarı (ör. palet okunamadı) atlanır.
fn warning_keys(warnings: &[String]) -> Vec<&str> {
    let key_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.';
    warnings
        .iter()
        .filter_map(|w| {
            let key = &w[..w.find(|c| !key_char(c)).unwrap_or(w.len())];
            key.contains('.').then_some(key.trim_end_matches('.'))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_log_keys_carry_no_values() {
        let secret = "GizliAile";
        let toml = format!(
            "[typography.ui]\nfamily = \"\"\n[typography]\nsize = 9999\n[motion]\npreset = \"{secret}\"\n[unknownsection]\nx = \"{secret}\"\n"
        );
        let (_, warnings) = Theme::parse(&toml).unwrap();
        assert!(warnings.len() >= 3, "{warnings:?}");
        let keys = warning_keys(&warnings);
        let logged = format!("{keys:?}");
        assert!(!logged.contains(secret) && !logged.contains("9999"), "{logged}");
        assert!(logged.contains("typography.size"), "{logged}");
        // Yol ya da serbest metinle başlayan uyarılar atlanır.
        let other = ["/home/u/.config/nuxut/x.toml yok".to_owned(), "üretilmiş palet okunamadı".to_owned()];
        assert!(warning_keys(&other).is_empty());
    }

    fn sp(response: f32, damping: f32) -> nuxut_motion::Motion {
        Spring::new(response, damping).into()
    }

    fn spring_of(m: nuxut_motion::Motion) -> Spring {
        match m {
            nuxut_motion::Motion::Spring(s) => s,
            other => panic!("yay bekleniyordu: {other:?}"),
        }
    }

    #[test]
    fn defaults_match_design_doc() {
        let t = Theme::default();
        assert_eq!(t.colors.surface.to_hex(), "#12141a");
        assert_eq!(t.colors.primary.to_hex(), "#9ec5ff");
        assert_eq!(t.shape.radius_large, 22.0);
        assert!(t.shadow.enabled);
        assert_eq!(t.shadow.blur, 16.0);
        assert!((t.shadow.color.a - 0.42).abs() < 1e-6);
        // Varsayılan ön ayar: calm.
        assert_eq!(t.motion.spatial, sp(0.38, 0.87));
        assert_eq!(t.motion.spatial_fast, sp(0.26, 0.92));
        assert_eq!(t.motion.effects, sp(0.22, 1.0));
        assert!((t.blob.deform - 0.2).abs() < 1e-6);
        assert_eq!(t.blob.smoothing, 20.0);
        assert_eq!(t.typography.ui, FontRole { family: "Inter".into(), weight: 500, scale: 1.0 });
        assert_eq!(t.typography.document.family, "Source Serif 4");
        assert_eq!(t.typography.mono.family, "JetBrainsMono Nerd Font");
        assert_eq!(t.typography.icon_set, IconSet::Material);
        assert_eq!(t.typography.icon_family(), MATERIAL_SYMBOLS_FAMILY);
    }

    #[test]
    fn empty_file_is_default() {
        let (t, w) = Theme::parse("").unwrap();
        assert_eq!(t, Theme::default());
        assert!(w.is_empty());
    }

    #[test]
    fn partial_spring_section_keeps_its_own_defaults() {
        let (t, _) = Theme::parse("[motion.spatial_fast]\ndamping = 0.9\n").unwrap();
        assert_eq!(t.motion.spatial_fast, sp(0.26, 0.9));
        assert_eq!(t.motion.spatial, sp(0.38, 0.87));
    }

    #[test]
    fn invalid_values_are_clamped_with_warnings() {
        let text = "[shape]\nradius_small = -4.0\n[motion.spatial]\ndamping = 0.0\n[typography.ui]\nfamily = \"\"\nweight = 5\nscale = 9.0\n";
        let (t, w) = Theme::parse(text).unwrap();
        assert_eq!(t.shape.radius_small, 0.0);
        assert_eq!(spring_of(t.motion.spatial).damping, 0.05);
        assert_eq!(t.typography.ui, FontRole { family: "Inter".into(), weight: 100, scale: 4.0 });
        assert_eq!(w.len(), 5, "{w:?}");
    }

    /// Eski şema (`family`, `weight`, düz dize `mono`/`icon`) yeni rollere uyarıyla eşlenir;
    /// açık yeni anahtar eskisinden önce gelir.
    #[test]
    fn legacy_typography_maps_to_roles() {
        let text = "[typography]\nfamily = \"Cantarell\"\nweight = 400\nmono = \"Iosevka\"\nicon = \"Symbols X\"\n";
        let (t, w) = Theme::parse(text).unwrap();
        assert_eq!(t.typography.ui, FontRole { family: "Cantarell".into(), weight: 400, scale: 1.0 });
        assert_eq!(t.typography.mono.family, "Iosevka");
        assert_eq!(t.typography.mono.weight, 400);
        assert_eq!(t.typography.icon_family(), "Symbols X");
        assert_eq!(t.typography.display.family, "Inter");
        assert_eq!(w.len(), 4, "{w:?}");

        let (t, _) = Theme::parse("[typography]\nfamily = \"Eski\"\n[typography.ui]\nfamily = \"Yeni\"\n").unwrap();
        assert_eq!(t.typography.ui.family, "Yeni");
    }

    /// Kısmi rol tablosu rolün kendi varsayılanlarını korur; ikon ailesi boşsa setin fontu.
    #[test]
    fn partial_roles_and_icon_set() {
        let text = "[typography]\nicon_set = \"nerd\"\n[typography.display]\nscale = 1.5\n[typography.mono]\nfamily = \"Hack Nerd Font\"\n";
        let (t, w) = Theme::parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.typography.display, FontRole { family: "Inter".into(), weight: 600, scale: 1.5 });
        assert_eq!(t.typography.icon_set, IconSet::Nerd);
        assert_eq!(t.typography.icon_family(), "Hack Nerd Font");
        // Yeni şema geri yazılınca eski anahtar oluşmaz.
        let out = toml::to_string(&FrameworkConfig::default()).unwrap();
        assert!(!out.contains("family ="), "{out}");
    }

    #[test]
    fn nan_falls_back_to_default() {
        let (t, w) = Theme::parse("[blob]\nsmoothing = nan\n").unwrap();
        assert_eq!(t.blob.smoothing, 20.0);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn motion_disabled_makes_everything_instant() {
        let (t, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        assert!(t.motion.spatial.is_instant());
        assert!(t.motion.effects.is_instant());
        assert_eq!(t.blob.deform, 0.0);
    }

    /// Ön ayar varsayılanları verir; açık anahtar yalnızca kendisini ezer.
    #[test]
    fn motion_presets_and_overrides() {
        let (t, w) = Theme::parse("[motion]\npreset = \"lively\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.motion.spatial, sp(0.42, 0.70));
        assert_eq!(t.motion.spatial_fast, sp(0.28, 0.82));
        assert!((t.blob.deform - 0.6).abs() < 1e-6);

        let (t, _) = Theme::parse("[motion]\npreset = \"snappy\"\n").unwrap();
        assert_eq!(t.motion.spatial, sp(0.30, 1.0));
        assert_eq!(t.blob.deform, 0.0);

        let (t, _) = Theme::parse("[motion]\npreset = \"instant\"\n").unwrap();
        assert!(!t.motion.enabled && t.motion.spatial.is_instant() && t.blob.deform == 0.0);

        // Rolün tek anahtarı ezilir, diğeri ön ayardan; blob.deform açıkça yazılırsa o.
        let (t, _) = Theme::parse("[motion]\npreset = \"lively\"\n[motion.spatial]\ndamping = 0.95\n[blob]\ndeform = 0.1\n").unwrap();
        assert_eq!(t.motion.spatial, sp(0.42, 0.95));
        assert_eq!(t.motion.spatial_fast, sp(0.28, 0.82));
        assert!((t.blob.deform - 0.1).abs() < 1e-6);

        // Bilinmeyen ön ayar dosyayı bozmaz: uyarı (değer yansıtılmaz) + calm.
        let (t, w) = Theme::parse("[motion]\npreset = \"zıpzıp\"\n").unwrap();
        assert_eq!(t.motion.spatial, sp(0.38, 0.87));
        assert_eq!(w.len(), 1);
        assert!(!w[0].contains("zıpzıp"), "{w:?}");
    }

    #[test]
    fn motion_config_customization_and_serialization() {
        assert_eq!(FrameworkConfig::default().motion.preset, MotionPreset::Calm);
        assert!(!MotionConfig::for_preset(MotionPreset::Lively).is_customized());
        let cfg: FrameworkConfig = toml::from_str("[motion.effects]\nresponse = 0.3\n").unwrap();
        assert!(cfg.motion.is_customized());
        // Geri yazımda ön ayar rolleri dosyaya kopyalanmaz (yalnızca açık anahtarlar).
        let out = toml::to_string(&FrameworkConfig::default()).unwrap();
        assert!(out.contains("preset = \"calm\""), "{out}");
        assert!(!out.contains("response") && !out.contains("deform"), "{out}");
        let back: FrameworkConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.motion, cfg.motion);
        for p in MotionPreset::ALL {
            assert_eq!(MotionPreset::from_name(p.name()), Some(p));
        }
    }

    #[test]
    fn speed_scales_response() {
        let (t, _) = Theme::parse("[motion]\nspeed = 2.0\n").unwrap();
        let s = spring_of(t.motion.spatial);
        assert!((s.response - 0.19).abs() < 1e-6);
        assert_eq!(s.damping, 0.87);
    }

    #[test]
    fn curve_style_global_and_per_role() {
        let text = "[motion]\nstyle = \"curve\"\nspeed = 2.0\n[motion.spatial]\neasing = \"cubic-bezier(0.3, 0, 0, 1)\"\nduration_ms = 600\n[motion.effects]\nstyle = \"spring\"\n";
        let (t, w) = Theme::parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.motion.spatial, nuxut_motion::Motion::Curve(Curve::new(CubicBezier::new(0.3, 0.0, 0.0, 1.0), 0.3)));
        // Rol varsayılan eğrisi + global hız.
        assert_eq!(t.motion.spatial_fast, nuxut_motion::Motion::Curve(Curve::new(CubicBezier::DECELERATE, 0.175)));
        // Rol düzeyinde yay.
        assert_eq!(t.motion.effects, sp(0.11, 1.0));
    }

    #[test]
    fn bad_easing_falls_back_with_warning() {
        let (t, w) = Theme::parse("[motion.spatial]\nstyle = \"curve\"\neasing = \"bouncy\"\n").unwrap();
        assert_eq!(t.motion.spatial, nuxut_motion::Motion::Curve(Curve::new(CubicBezier::STANDARD, 0.5)));
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(Theme::parse("[motion]\nstyle = \"wobbly\"\n").is_err(), "bilinmeyen stil parse hatası");
    }

    #[test]
    fn unknown_keys_and_bad_colors() {
        let (_, w) = Theme::parse("[colors]\nsurfac = \"#000\"\n").unwrap();
        assert!(w.iter().any(|w| w.contains("colors.surfac")), "{w:?}");
        assert!(Theme::parse("[colors]\nsurface = \"blue\"\n").is_err());
    }

    #[test]
    fn load_layers_reads_palette_only_for_wallpaper_source() {
        let dir = std::env::temp_dir().join(format!("nuxut-theme-layers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("framework.toml");
        let pal = dir.join(PALETTE_FILE);
        let red = Color::from_hex("#ff0000").unwrap();
        GeneratedPalette { colors: PalettePatch { primary: Some(red), ..Default::default() }, ..Default::default() }
            .save(&pal)
            .unwrap();

        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\non_surface = \"#ffffff\"\n").unwrap();
        let t = Theme::load_layers(&cfg, Some(&pal)).unwrap().value;
        assert_eq!(t.colors.primary, red);
        assert_eq!(t.colors.on_surface, Color::WHITE);

        std::fs::write(&cfg, "[colors]\non_surface = \"#ffffff\"\n").unwrap();
        let t = Theme::load_layers(&cfg, Some(&pal)).unwrap().value;
        assert_eq!(t.colors.primary, Palette::default().primary, "statik kaynakta palet okunmaz");

        // Bozuk palet: hata değil; durumsuz yüklemede katman atlanır...
        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\n").unwrap();
        let mut loader = ThemeLoader::with_paths(Some(cfg.clone()), Some(pal.clone()));
        assert_eq!(loader.load().unwrap().value.colors.primary, red);
        std::fs::write(&pal, "[colors]\nprimary = \"mavi\"\n").unwrap();
        let loaded = Theme::load_layers(&cfg, Some(&pal)).unwrap();
        assert_eq!(loaded.value.colors.primary, Palette::default().primary);
        assert!(loaded.warnings.iter().any(|w| w.contains("palet")), "{:?}", loaded.warnings);
        // ...ThemeLoader ise son geçerli paleti korur.
        let loaded = loader.load().unwrap();
        assert_eq!(loaded.value.colors.primary, red);
        assert!(loaded.warnings.iter().any(|w| w.contains("korundu")), "{:?}", loaded.warnings);
        // Dosya silinince katman kalkar.
        std::fs::remove_file(&pal).unwrap();
        assert_eq!(loader.load().unwrap().value.colors.primary, Palette::default().primary);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn error_roles_default_and_override() {
        let t = Theme::default();
        assert_eq!(t.colors.error.to_hex(), "#ffb4ab");
        assert_eq!(t.colors.on_error.to_hex(), "#690005");
        let (o, w) = Theme::parse("[colors]\nerror = \"#ff0000\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(o.colors.error.to_hex(), "#ff0000");
    }

    #[test]
    fn outlines_default_none_and_modes() {
        let t = Theme::default();
        assert_eq!(t.shape.outlines, Outlines::None);
        assert_eq!(t.outline(), None);
        let (s, _) = Theme::parse("[shape]\noutlines = \"subtle\"\n").unwrap();
        assert!((s.outline().unwrap().a - OUTLINE_SUBTLE_ALPHA).abs() < 1e-6);
        let (f, _) = Theme::parse("[shape]\noutlines = \"full\"\n").unwrap();
        // "full" kenarı yüzeylere karşı WCAG 1.4.11 sınırını (3:1) geçmeli.
        let lum = |c: Color| {
            let ch = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
            0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
        };
        let over = |fg: Color, bg: Color| bg.lerp(&fg.with_alpha(1.0), fg.a);
        for bg in [f.colors.surface, f.colors.surface_container] {
            let edge = over(f.outline().unwrap(), bg);
            let (a, b) = (lum(edge) + 0.05, lum(bg) + 0.05);
            let ratio = a.max(b) / a.min(b);
            assert!(ratio >= 3.0, "full kenar kontrastı {ratio:.2} < 3 ({bg:?})");
        }
        assert!(Theme::parse("[shape]\noutlines = \"kalın\"\n").is_err());
    }

    #[test]
    fn color_lerp_endpoints() {
        let a = Theme::default();
        let (b, _) = Theme::parse("[colors]\nsurface = \"#ffffff\"\n").unwrap();
        assert_eq!(a.lerp_colors(&b, 1.0).colors.surface, b.colors.surface);
        assert_eq!(a.lerp_colors(&b, 0.0).colors.surface, a.colors.surface);
    }

    #[test]
    fn color_mode_selects_static_palette() {
        // Anahtar yok → auto → (statik varsayılanın yüzeyi koyu) koyu.
        let (t, w) = Theme::parse("").unwrap();
        assert!(w.is_empty());
        assert_eq!(FrameworkConfig::default().colors.mode.value, ColorMode::Auto);
        assert_eq!(t.mode, ThemeMode::Dark);
        assert_eq!(t.colors, Palette::default());
        assert_eq!(Theme::parse("[colors]\nmode = \"auto\"\n").unwrap(), (t.clone(), vec![]));
        let (t, w) = Theme::parse("[colors]\nmode = \"light\"\nprimary = \"#ff0000\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.mode, ThemeMode::Light);
        assert_eq!(t.colors.surface, Palette::light().surface);
        // Açıkça yazılmış renk kipten bağımsız üstüne gelir.
        assert_eq!(t.colors.primary, Color::from_hex("#ff0000").unwrap());
        // Açık palet gerçekten açık: zemin parlak, metin koyu.
        let lum = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        assert!(lum(Palette::light().surface) > 0.9 && lum(Palette::light().on_surface) < 0.2);
        // Geçersiz kip ve yanlış tür dosyayı bozmaz: uyarı (değer yansıtılmaz) + auto.
        for bad in ["\"Light\"", "3", "\"gizli-değer\""] {
            let (t, w) = Theme::parse(&format!("[colors]\nmode = {bad}\n")).unwrap();
            assert_eq!(t.mode, ThemeMode::Dark);
            assert_eq!(w.len(), 1, "{w:?}");
            assert!(w[0].starts_with("colors.mode") && w[0].contains("'auto'") && !w[0].contains("gizli"), "{w:?}");
            // auto olarak çözülür: açık yüzey açık kip verir.
            let (t, _) = Theme::parse(&format!("[colors]\nmode = {bad}\nsurface = \"#ffffff\"\n")).unwrap();
            assert_eq!(t.mode, ThemeMode::Light);
        }
    }

    #[test]
    fn auto_mode_follows_static_surface() {
        // Açıkça yazılmış yüzey auto'da kipi belirler; açık renkler yine üstüne gelir.
        let (t, w) = Theme::parse("[colors]\nsurface = \"#ffffff\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.mode, ThemeMode::Light);
        assert_eq!(t.colors.surface, Color::WHITE);
        assert_eq!(t.colors.on_surface, Palette::light().on_surface);
        let (t, _) = Theme::parse("[colors]\nsurface = \"#202020\"\n").unwrap();
        assert_eq!(t.mode, ThemeMode::Dark);
        // Sabit kip yüzeye bakmaz.
        let (t, _) = Theme::parse("[colors]\nmode = \"dark\"\nsurface = \"#ffffff\"\n").unwrap();
        assert_eq!((t.mode, t.colors.on_surface), (ThemeMode::Dark, Palette::default().on_surface));
        // Statik kaynakta üretilmiş paletin kipi yok sayılır.
        let g = GeneratedPalette { meta: PaletteMeta { mode: Some("light".into()), ..Default::default() }, ..Default::default() };
        assert_eq!(Theme::from_generated(&FrameworkConfig::default(), Some(&g)).0.mode, ThemeMode::Dark);
        // Eşik: algısal orta gri civarı.
        assert_eq!(ThemeMode::of_surface(Color::from_hex("#808080").unwrap()), ThemeMode::Light);
        assert_eq!(ThemeMode::of_surface(Color::from_hex("#707070").unwrap()), ThemeMode::Dark);
        assert_eq!(ColorMode::Auto.fixed(), None);
        assert_eq!(ColorMode::from(ThemeMode::Light).fixed(), Some(ThemeMode::Light));
    }

    #[test]
    fn auto_mode_follows_wallpaper_palette() {
        let dir = tempfile_dir("auto");
        let cfg = dir.join("framework.toml");
        let pal = dir.join(PALETTE_FILE);
        let (red, blue) = (Color::from_hex("#ff0000").unwrap(), Color::from_hex("#0000ff").unwrap());
        let patch = |c| PalettePatch { primary: Some(c), ..Default::default() };
        let generated = |mode: &str| GeneratedPalette {
            meta: PaletteMeta { mode: Some(mode.into()), ..Default::default() },
            colors: patch(red),
            dark: Some(patch(red)),
            light: Some(patch(blue)),
            ..Default::default()
        };
        generated("light").save(&pal).unwrap();
        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\n").unwrap();
        let loaded = Theme::load_layers(&cfg, Some(&pal)).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.value.mode, ThemeMode::Light);
        assert_eq!(loaded.value.colors.primary, blue);
        assert_eq!(loaded.value.colors.surface, Palette::light().surface);
        // Kayıtlı kip, açıkça yazılmış yüzeyden önce gelir.
        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\nsurface = \"#101010\"\n").unwrap();
        assert_eq!(Theme::load_layers(&cfg, Some(&pal)).unwrap().value.mode, ThemeMode::Light);
        generated("dark").save(&pal).unwrap();
        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\n").unwrap();
        let loaded = Theme::load_layers(&cfg, Some(&pal)).unwrap();
        assert_eq!((loaded.value.mode, loaded.value.colors.primary), (ThemeMode::Dark, red));
        // Palet dosyası yok: yüzeye düşer.
        std::fs::remove_file(&pal).unwrap();
        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\nsurface = \"#fafafa\"\n").unwrap();
        assert_eq!(Theme::load_layers(&cfg, Some(&pal)).unwrap().value.mode, ThemeMode::Light);
        // Kayıt elde yokken (yalnızca yama): auto, yamanın yüzeyinden.
        let cfg: FrameworkConfig = toml::from_str("[colors]\nsource = \"wallpaper\"\n").unwrap();
        let light = PalettePatch::from(Palette::light());
        assert_eq!(Theme::from_layers(&cfg, Some(&light)).0.mode, ThemeMode::Light);
        assert_eq!(Theme::from_layers(&cfg, None).0.mode, ThemeMode::Dark);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn locale_section_is_schema_and_not_visual() {
        let (t, w) = Theme::parse("[locale]\nlanguage = \"de\"\nformats = \"tr_TR.UTF-8\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(t.locale.language.value(), Some("de"));
        assert_eq!(t.locale.formats.value(), Some("tr_TR.UTF-8"));
        assert!(t.same_visuals(&Theme::default()));
        assert_ne!(t, Theme::default());
        let (t, w) = Theme::parse("[locale]\nlanguage = \"gizli değer!\"\nformats = 3\nhour_cycle = \"gizli\"\n").unwrap();
        assert!(t.locale.language.is_auto() && t.locale.formats.is_auto());
        assert_eq!(t.locale.hour_cycle.cycle, HourCycle::Auto);
        assert_eq!(w.len(), 3, "{w:?}");
        let (t, w) = Theme::parse("[locale]\nhour_cycle = \"h12\"\n").unwrap();
        assert!(w.is_empty() && t.locale.hour_cycle.cycle == HourCycle::H12, "{w:?}");
        assert!(t.same_visuals(&Theme::default()), "hour_cycle görsel değil");
        assert!(w.iter().all(|w| !w.contains("gizli")), "{w:?}");
        let (t, _) = Theme::parse("[shape]\nradius_small = 3.0\n").unwrap();
        assert!(!t.same_visuals(&Theme::default()));
    }

    #[test]
    fn loader_distinguishes_missing_and_broken_file() {
        let dir = std::env::temp_dir().join(format!("nuxut-theme-broken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("framework.toml");
        let _ = std::fs::remove_file(&path);
        // Yok: varsayılan, birleşik görünüm açık.
        let loaded = ThemeLoader::with_paths(Some(path.clone()), None).load().unwrap();
        assert!(!loaded.from_file && loaded.value.desktop.unified);
        // Yanlış tür: hata (çağıran Theme::broken_config'e düşer → kapalı).
        std::fs::write(&path, "[desktop]\nunified = 0\n").unwrap();
        assert!(ThemeLoader::with_paths(Some(path.clone()), None).load().is_err());
        std::fs::write(&path, "[desktop]\nunified = false\n").unwrap();
        assert!(!ThemeLoader::with_paths(Some(path.clone()), None).load().unwrap().value.desktop.unified);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn appearance_mascot_defaults_on_and_is_visual() {
        assert!(Theme::default().appearance.mascot);
        let (t, w) = Theme::parse("[appearance]\nmascot = false\n").unwrap();
        assert!(w.is_empty(), "bilinmeyen anahtar uyarısı olmamalı: {w:?}");
        assert!(!t.appearance.mascot);
        assert!(!t.same_visuals(&Theme::default()), "gizleme theme_changed tetiklemeli");
        assert!(Theme::parse("[appearance]\nmascot = \"no\"\n").is_err());
        // Varsayılan döküm diğer tablolar gibi bölümü taşır (istek, 2026-10-14).
        let text = toml::to_string(&FrameworkConfig::default()).unwrap();
        assert!(text.contains("[appearance]\nmascot = true"), "{text}");
        let cfg = FrameworkConfig { appearance: AppearanceConfig { mascot: false }, ..FrameworkConfig::default() };
        assert!(toml::to_string(&cfg).unwrap().contains("[appearance]\nmascot = false"));
        // Bölümsüz eski dosya uyarısız yüklenir, maskot açık.
        let (old, w) = Theme::parse("[colors]\nmode = \"dark\"\ncontrast = \"standard\"\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert!(old.appearance.mascot);
        // Döküm geri okunur (gidiş-dönüş).
        let back: FrameworkConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.appearance, AppearanceConfig::default());
    }

    #[test]
    fn desktop_section_is_schema_not_unknown() {
        let text = "[colors]\nmode = \"dark\"\n[desktop]\nunified = true\nicon_theme = \"Papirus-Dark\"\ncursor_theme = \"\"\ncursor_size = 32\nqt_backend = \"qt6ct\"\n";
        let (t, w) = Theme::parse(text).unwrap();
        assert!(w.is_empty(), "bilinmeyen anahtar uyarısı olmamalı: {w:?}");
        assert_eq!(
            t.desktop,
            Desktop { unified: true, icon_theme: Some("Papirus-Dark".into()), cursor_theme: None, cursor_size: 32, qt_backend: QtBackend::Qt6ct }
        );
        assert_eq!(Theme::default().desktop, Desktop::default());
        // Birleşik görünüm varsayılan açık: anahtar ya da [desktop] yoksa true; açıkça false kapatır.
        assert!(Desktop::default().unified && DesktopConfig::default().unified);
        assert!(Theme::parse("[colors]\nmode = \"dark\"\n").unwrap().0.desktop.unified);
        assert!(Theme::parse("[desktop]\ncursor_size = 32\n").unwrap().0.desktop.unified);
        assert!(!Theme::parse("[desktop]\nunified = false\n").unwrap().0.desktop.unified);
        // Bozuk dosya (sözdizimi ya da yanlış tür) hata verir; çağıranın düştüğü tema kapalı.
        assert!(Theme::parse("[desktop]\nunified = \"no\"\n").is_err());
        assert!(Theme::parse("[desktop\nunified = false\n").is_err());
        let broken = Theme::broken_config();
        assert!(!broken.desktop.unified);
        assert_eq!(Theme { desktop: Desktop::default(), ..broken }, Theme::default());
        // Eski ad, sınır dışı boy, güvensiz ad, bilinmeyen arka uç.
        let (t, _) = Theme::parse("[desktop]\nqt_backend = \"qtct\"\n").unwrap();
        assert_eq!(t.desktop.qt_backend, QtBackend::Qt6ct);
        let text = "[desktop]\ncursor_size = 500\nicon_theme = \"../etc\"\ncursor_theme = \"a\\nb\"\nqt_backend = \"kvantum\"\n";
        let (t, w) = Theme::parse(text).unwrap();
        assert_eq!(t.desktop.cursor_size, CURSOR_SIZE_MAX);
        assert_eq!((t.desktop.icon_theme, t.desktop.cursor_theme), (None, None));
        assert_eq!(t.desktop.qt_backend, QtBackend::Auto);
        assert_eq!(w.len(), 4, "{w:?}");
        assert!(w.iter().all(|w| !w.contains("etc") && !w.contains("kvantum")), "{w:?}");
        let (t, _) = Theme::parse("[desktop]\ncursor_size = 1\n").unwrap();
        assert_eq!(t.desktop.cursor_size, CURSOR_SIZE_MIN);
        for ok in ["Papirus-Dark", "Bibata-Modern-Classic", "breeze_cursors", "Vimix Cursors", "a+b.c"] {
            assert!(valid_theme_name(ok), "{ok}");
        }
        for bad in ["", ".x", "a/b", "a\"b", "a'b", "a=b", &"x".repeat(MAX_THEME_NAME + 1)] {
            assert!(!valid_theme_name(bad), "{bad}");
        }
    }

    #[test]
    fn schema_export_carries_mode_and_desktop() {
        // Ayarlar varsayılan belgeyi bu serileştirmeden kurar.
        let out = toml::to_string(&FrameworkConfig::default()).unwrap();
        assert!(out.contains("mode = \"auto\""), "{out}");
        assert!(out.contains("[desktop]") && out.contains("qt_backend = \"auto\"") && out.contains("cursor_size = 24"), "{out}");
        let cfg: FrameworkConfig =
            toml::from_str("[colors]\nmode = \"light\"\n[desktop]\nunified = true\nqt_backend = \"kde\"\n").unwrap();
        let back: FrameworkConfig = toml::from_str(&toml::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back, cfg);
        assert_eq!(back.colors.mode.value, ColorMode::Light);
    }

    #[test]
    fn high_contrast_mode_from_config() {
        use crate::contrast::{NON_TEXT_MIN, TEXT_MIN};
        // Varsayılan standart: anahtar yokken tema eskisiyle aynı, şemada "standard" yazılır.
        let (std_theme, _) = Theme::parse("").unwrap();
        assert_eq!(std_theme.contrast, ContrastMode::Standard);
        assert!(toml::to_string(&FrameworkConfig::default()).unwrap().contains("contrast = \"standard\""));
        for mode in ["dark", "light"] {
            let (t, w) = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap();
            assert!(w.is_empty(), "{w:?}");
            assert_eq!(t.contrast, ContrastMode::High);
            let p = t.colors;
            assert_eq!(p, p.high_contrast(), "idempotent");
            for s in [p.surface, p.surface_container, p.surface_bright] {
                for fg in [p.on_surface, p.on_surface_variant, p.primary, p.error] {
                    assert!(contrast_ratio(fg, s) >= TEXT_MIN, "{mode} {fg:?} / {s:?}");
                }
                assert!(contrast_ratio(p.outline, s) >= NON_TEXT_MIN, "{mode} outline");
                assert!(contrast_ratio(t.outline().unwrap(), s) >= NON_TEXT_MIN);
            }
            assert!(contrast_ratio(p.on_primary, p.primary) >= TEXT_MIN);
            assert!(contrast_ratio(p.on_error, p.error) >= TEXT_MIN);
            // Kenar her zaman çizilir; yüzeyler değişmez (kimlik).
            assert_eq!(t.shape.outlines, Outlines::Subtle);
            let base = Palette::for_mode(t.mode);
            assert_eq!((p.surface, p.surface_container, p.surface_bright), (base.surface, base.surface_container, base.surface_bright));
            assert!(!t.same_visuals(&Theme::parse(&format!("[colors]\nmode = \"{mode}\"\n")).unwrap().0));
        }
        // full kenar yüksek karşıtlıkta tam opak; açık `full` korunur.
        let (t, _) = Theme::parse("[colors]\ncontrast = \"high\"\n[shape]\noutlines = \"full\"\n").unwrap();
        assert_eq!(t.shape.outlines, Outlines::Full);
        assert_eq!(t.outline(), Some(t.colors.on_surface_variant));
        // Açıkça yazılmış düşük karşıtlıklı renk de eşiğe itilir.
        let (t, _) = Theme::parse("[colors]\ncontrast = \"high\"\non_surface_variant = \"#303340\"\n").unwrap();
        assert!(contrast_ratio(t.colors.on_surface_variant, t.colors.surface_bright) >= TEXT_MIN);
        // Geçersiz değer: uyarı + standart.
        let (t, w) = Theme::parse("[colors]\ncontrast = \"ultra\"\n").unwrap();
        assert_eq!(t.contrast, ContrastMode::Standard);
        assert_eq!(w.len(), 1, "{w:?}");
    }

    #[test]
    fn wallpaper_palette_follows_mode() {
        let dir = tempfile_dir("mode");
        let cfg = dir.join("framework.toml");
        let pal = dir.join(PALETTE_FILE);
        let (red, blue) = (Color::from_hex("#ff0000").unwrap(), Color::from_hex("#0000ff").unwrap());
        let patch = |c| PalettePatch { primary: Some(c), ..Default::default() };
        GeneratedPalette {
            meta: PaletteMeta { mode: Some("dark".into()), ..Default::default() },
            colors: patch(red),
            dark: Some(patch(red)),
            light: Some(patch(blue)),
            ..Default::default()
        }
        .save(&pal)
        .unwrap();
        std::fs::write(&cfg, "[colors]\nsource = \"wallpaper\"\nmode = \"light\"\n").unwrap();
        let loaded = Theme::load_layers(&cfg, Some(&pal)).unwrap();
        assert_eq!(loaded.value.colors.primary, blue);
        assert_eq!(loaded.value.colors.surface, Palette::light().surface);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        // Eski dosya (kip bölümü yok, başka kipte üretilmiş): `colors` + uyarı.
        GeneratedPalette { meta: PaletteMeta { mode: Some("dark".into()), ..Default::default() }, colors: patch(red), ..Default::default() }
            .save(&pal)
            .unwrap();
        let loaded = Theme::load_layers(&cfg, Some(&pal)).unwrap();
        assert_eq!(loaded.value.colors.primary, red);
        assert_eq!(loaded.warnings.len(), 1, "{:?}", loaded.warnings);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn tempfile_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nuxut-theme-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
