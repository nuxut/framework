//! Renk paleti ve katmanlaması.
//!
//! Tema renkleri üç katmandan çözülür (sonraki öncekinin üstüne yazar):
//! 1. derlenmiş varsayılanlar ([`Palette::default`]),
//! 2. üretilmiş palet `$XDG_STATE_HOME/nuxut/palette.toml` — yalnızca framework.toml'da
//!    `[colors] source = "wallpaper"` iken,
//! 3. framework.toml `[colors]` bölümünde **açıkça yazılmış** renkler.
//!
//! framework.toml asla otomatik yazılmaz; palet üreticisi (nuxut-media) yalnızca
//! [`GeneratedPalette::save`] ile state dosyasını yazar.

use std::io::Write;
use std::path::{Path, PathBuf};

use nuxut_config::{ConfigError, Loaded};
use nuxut_core::Color;
use serde::{Deserialize, Serialize};

use crate::color_theme::ColorThemeId;
use crate::contrast::ContrastMode;

const fn hex(rgb: u32) -> Color {
    Color::from_rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}

/// Olumlu durum (başarı) renginin tohumu (Material yeşil 600). Çıktı rengi değildir: hue/kroma
/// kaynağıdır, tonu ve uyumlandırmayı [`Palette::success`] (ve nuxut-palette `success` ailesi)
/// verir.
pub const SUCCESS_SEED: Color = hex(0x43a047);

/// [`Palette`] alanlarının adla seçimi: bileşene "hangi token" verilir, renk her karede temadan
/// okunur (tema geçişini ve palet değişimini kendiliğinden izler). Dize biçimi alan adıdır
/// (`"surface_container"`), config/IPC için.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRole {
    Surface,
    SurfaceContainer,
    SurfaceBright,
    OnSurface,
    OnSurfaceVariant,
    Primary,
    OnPrimary,
    Error,
    OnError,
    Outline,
    Shadow,
}

impl ColorRole {
    pub const ALL: [Self; 11] = [
        Self::Surface,
        Self::SurfaceContainer,
        Self::SurfaceBright,
        Self::OnSurface,
        Self::OnSurfaceVariant,
        Self::Primary,
        Self::OnPrimary,
        Self::Error,
        Self::OnError,
        Self::Outline,
        Self::Shadow,
    ];

    /// `[colors]` anahtarıyla aynı ad.
    pub fn name(self) -> &'static str {
        match self {
            Self::Surface => "surface",
            Self::SurfaceContainer => "surface_container",
            Self::SurfaceBright => "surface_bright",
            Self::OnSurface => "on_surface",
            Self::OnSurfaceVariant => "on_surface_variant",
            Self::Primary => "primary",
            Self::OnPrimary => "on_primary",
            Self::Error => "error",
            Self::OnError => "on_error",
            Self::Outline => "outline",
            Self::Shadow => "shadow",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim();
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
}

/// Çözülmüş renk token'ları (Material 3 adlandırması).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub surface: Color,
    pub surface_container: Color,
    pub surface_bright: Color,
    pub on_surface: Color,
    pub on_surface_variant: Color,
    pub primary: Color,
    pub on_primary: Color,
    /// Hata durumu (başarısız eylem, doğrulama).
    pub error: Color,
    pub on_error: Color,
    pub outline: Color,
    pub shadow: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self::DARK
    }
}

impl Palette {
    /// Statik koyu varsayılan (Nuxut mavisi koyu; elle ayarlanmış tarihî değerler).
    pub const DARK: Self = Self {
        surface: hex(0x12141a),
        surface_container: hex(0x1a1d25),
        surface_bright: hex(0x252935),
        on_surface: hex(0xe4e6ee),
        on_surface_variant: hex(0x9298a6),
        primary: hex(0x9ec5ff),
        on_primary: hex(0x0a1a33),
        error: hex(0xffb4ab),
        on_error: hex(0x690005),
        outline: hex(0x2c303b),
        shadow: hex(0x000000),
    };

    /// Rolün rengi.
    pub fn get(&self, role: ColorRole) -> Color {
        match role {
            ColorRole::Surface => self.surface,
            ColorRole::SurfaceContainer => self.surface_container,
            ColorRole::SurfaceBright => self.surface_bright,
            ColorRole::OnSurface => self.on_surface,
            ColorRole::OnSurfaceVariant => self.on_surface_variant,
            ColorRole::Primary => self.primary,
            ColorRole::OnPrimary => self.on_primary,
            ColorRole::Error => self.error,
            ColorRole::OnError => self.on_error,
            ColorRole::Outline => self.outline,
            ColorRole::Shadow => self.shadow,
        }
    }

    /// Statik varsayılan paletin açık kip karşılığı (`[colors] mode = "light"`). Değerler
    /// koyu varsayılanın `primary`'sinden Material 3 tonal-spot açık şemasıdır (nuxut-palette
    /// testi `static_light_palette_matches_derivation` eşliği denetler).
    pub fn light() -> Self {
        Self {
            surface: hex(0xf9f9ff),
            surface_container: hex(0xededf4),
            surface_bright: hex(0xe7e8ee),
            on_surface: hex(0x191c20),
            on_surface_variant: hex(0x43474e),
            primary: hex(0x3b608f),
            on_primary: hex(0xffffff),
            error: hex(0xba1a1a),
            on_error: hex(0xffffff),
            outline: hex(0xc3c6cf),
            shadow: hex(0x000000),
        }
    }

    /// Kipin statik varsayılan paleti (Nuxut mavisi; seçili renk teması için
    /// [`crate::ColorTheme::palette`]).
    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::default(),
            ThemeMode::Light => Self::light(),
        }
    }

    /// Olumlu durum rengi (başarılı eylem, "kaydedildi"; ör. `NoticeKind::Success` ikonu).
    /// Ayrı bir token değildir, paletten türetilir: [`SUCCESS_SEED`] `primary`'ye uyumlandırılır
    /// (MCU `harmonize`, nuxut-palette'in `success` ailesiyle aynı yöntem; orada kaynak duvar
    /// kâğıdı rengi, burada `primary`), kipe göre ton 80 (koyu) / 40 (açık) seçilir, sonra üç
    /// yüzeye (`surface`, `surface_container`, `surface_bright`) karşı en az 4.5:1'e itilir
    /// ([`crate::contrast::ensure_contrast`]; metin olarak da kullanılabilir). Yüksek karşıtlık
    /// paleti zaten aynı yüzeyleri taşıdığından garanti o kipte de geçerlidir. Saf hesap; sık
    /// çizen bileşen sonucu paletle önbelleğe alabilir.
    pub fn success(&self) -> Color {
        use crate::contrast::{TEXT_MIN, ensure_contrast};
        use material_colors::blend::harmonize;
        use material_colors::color::Argb;
        use material_colors::palette::CorePalette;
        let argb = |c: Color| {
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            Argb::new(255, b(c.r), b(c.g), b(c.b))
        };
        let dark = ThemeMode::of_surface(self.surface) == ThemeMode::Dark;
        let tone = CorePalette::of(harmonize(argb(SUCCESS_SEED), argb(self.primary))).primary.tone(if dark { 80 } else { 40 });
        let base = Color::from_rgba8(tone.red, tone.green, tone.blue, 255);
        ensure_contrast(base, &[self.surface, self.surface_container, self.surface_bright], TEXT_MIN)
    }

    /// Yüksek karşıtlık karşılığı (docs/design/accessibility.md §1): yüzeyler ve gölge aynı
    /// kalır; ön planlar HCT tonuyla zeminlerden uzağa itilir (hue/kroma korunur,
    /// [`crate::contrast::ensure_contrast`]). Garanti, üç yüzeyin (`surface`,
    /// `surface_container`, `surface_bright`) hepsine karşı:
    /// - `on_surface`, `on_surface_variant` ≥ 4.5 (hedef 7);
    /// - `primary`, `error` ≥ 4.5 (metin/bağlantı/odak halkası olarak da kullanılır);
    /// - `on_primary`/`primary`, `on_error`/`error` ≥ 4.5 (hedef 7);
    /// - `outline` ≥ 3 (bileşen sınırı, WCAG 1.4.11).
    ///
    /// İdempotenttir; eşiği zaten geçen rol bayt bayt aynı kalır.
    pub fn high_contrast(&self) -> Self {
        use crate::contrast::{NON_TEXT_MIN, TEXT_MIN, TEXT_TARGET, ensure_contrast, ensure_pair};
        let mut p = *self;
        let surfaces = [p.surface, p.surface_container, p.surface_bright];
        p.on_surface = ensure_contrast(p.on_surface, &surfaces, TEXT_TARGET);
        p.on_surface_variant = ensure_contrast(p.on_surface_variant, &surfaces, TEXT_TARGET);
        p.primary = ensure_contrast(p.primary, &surfaces, TEXT_MIN);
        p.error = ensure_contrast(p.error, &surfaces, TEXT_MIN);
        (p.on_primary, p.primary) = ensure_pair(p.on_primary, p.primary, TEXT_TARGET, TEXT_MIN);
        (p.on_error, p.error) = ensure_pair(p.on_error, p.error, TEXT_TARGET, TEXT_MIN);
        p.outline = ensure_contrast(p.outline, &surfaces, NON_TEXT_MIN);
        p
    }
}

/// Çözülmüş koyu/açık kip ([`Theme::mode`](crate::Theme::mode)). `auto` burada yoktur:
/// [`ColorsConfig::resolve_mode`] onu her zaman ikisinden birine çözer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Light];

    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// `dark` | `light` (palette.toml `meta.mode`); başka her şey `None`.
    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s.trim())
    }

    /// Yüzey renginin kipi: göreli parlaklığı (WCAG) siyah metinle beyaz metinden daha iyi
    /// kontrast veriyorsa açık, değilse koyu (eşik Y ≈ 0.18, algısal orta gri).
    pub fn of_surface(surface: Color) -> Self {
        let lin = |c: f32| {
            let c = c.clamp(0.0, 1.0);
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let y = 0.2126 * lin(surface.r) + 0.7152 * lin(surface.g) + 0.0722 * lin(surface.b);
        // (y + 0.05) / 0.05 > 1.05 / (y + 0.05)
        if (y + 0.05) * (y + 0.05) > 0.05 * 1.05 { Self::Light } else { Self::Dark }
    }
}

/// `framework.toml [colors] mode`: `auto` (varsayılan) | `dark` | `light`. Ham şema değeridir;
/// tema her zaman [`ThemeMode`]'a çözülmüş kipi kullanır ([`ColorsConfig::resolve_mode`]).
/// `Auto`'yu sessizce bir kipe eşleyen `From` bilerek yoktur; sabit kip için [`Self::fixed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorMode {
    /// Palet kaydındaki kip (duvar kağıdı) → yüzey parlaklığı → koyu.
    #[default]
    Auto,
    Dark,
    Light,
}

impl ColorMode {
    pub const ALL: [Self; 3] = [Self::Auto, Self::Dark, Self::Light];

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    /// Kullanıcının sabitlediği kip; `Auto` için `None`.
    pub fn fixed(self) -> Option<ThemeMode> {
        match self {
            Self::Auto => None,
            Self::Dark => Some(ThemeMode::Dark),
            Self::Light => Some(ThemeMode::Light),
        }
    }
}

impl From<ThemeMode> for ColorMode {
    fn from(m: ThemeMode) -> Self {
        match m {
            ThemeMode::Dark => Self::Dark,
            ThemeMode::Light => Self::Light,
        }
    }
}

impl ChoiceValue for ColorMode {
    fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
    fn name(self) -> &'static str {
        ColorMode::name(self)
    }
}

/// [`Choice`] ile okunabilen adlandırılmış seçenek.
pub trait ChoiceValue: Copy + Default {
    /// Dosyadaki addan değer (bilinmeyen adda `None`).
    fn from_name(s: &str) -> Option<Self>;
    /// Dosyaya yazılan ad.
    fn name(self) -> &'static str;
}

/// Dosyayı bozmayan seçenek alanı: bilinmeyen ad ya da yanlış tür varsayılana döner ve
/// `invalid` işaretlenir (tema doğrulayıcısı uyarı verir). Dosyaya yalnızca adı yazılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Choice<T> {
    pub value: T,
    pub invalid: bool,
}

impl<T> Choice<T> {
    pub const fn new(value: T) -> Self {
        Self { value, invalid: false }
    }
}

impl<T: ChoiceValue> Serialize for Choice<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.value.name())
    }
}

impl<'de, T: ChoiceValue> Deserialize<'de> for Choice<T> {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = toml::Value::deserialize(de)?;
        Ok(match raw.as_str().map(str::trim).and_then(T::from_name) {
            Some(value) => Self::new(value),
            None => Self { value: T::default(), invalid: true },
        })
    }
}

/// Alan listesi tek yerde: patch uygulama ve karışım aynı sırayı kullanır.
macro_rules! palette_fields {
    ($m:ident) => {
        $m!(surface, surface_container, surface_bright, on_surface, on_surface_variant, primary, on_primary, error, on_error, outline, shadow)
    };
}

impl Palette {
    /// Tema geçişi için renk karışımı (`t ∈ [0, 1]`).
    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        macro_rules! mix {
            ($($f:ident),*) => { Self { $($f: self.$f.lerp(&to.$f, t)),* } };
        }
        palette_fields!(mix)
    }

    /// Yazılmış renkleri bu paletin üstüne uygular.
    pub fn apply(&mut self, patch: &PalettePatch) {
        macro_rules! apply {
            ($($f:ident),*) => { $( if let Some(c) = patch.$f { self.$f = c; } )* };
        }
        palette_fields!(apply);
    }
}

/// Paletin kısmi hali: yalnızca yazılmış renkler `Some`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PalettePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_container: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_bright: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_surface: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_surface_variant: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_primary: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Color>,
}

impl From<Palette> for PalettePatch {
    fn from(p: Palette) -> Self {
        macro_rules! all {
            ($($f:ident),*) => { Self { $($f: Some(p.$f)),* } };
        }
        palette_fields!(all)
    }
}

/// Renklerin kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorSource {
    /// Yalnızca varsayılanlar + framework.toml.
    #[default]
    Static,
    /// Duvar kağıdından üretilmiş palet (`palette.toml`) varsayılanların üstüne gelir.
    Wallpaper,
}

/// framework.toml `[colors]` bölümü.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct ColorsConfig {
    pub source: ColorSource,
    /// Koyu/açık kip (`auto` varsayılan): statik varsayılan paleti ve duvar kağıdı paletinin
    /// hangi kipinin kullanılacağını seçer ([`Self::resolve_mode`]). Açıkça yazılmış renkler
    /// her iki kipte de üstüne gelir.
    pub mode: Choice<ColorMode>,
    /// Karşıtlık kipi (`standard` varsayılan | `high`): yüksekte çözülmüş palet
    /// [`Palette::high_contrast`]'tan geçer (açıkça yazılmış renkler dahil).
    pub contrast: Choice<ContrastMode>,
    /// Nuxut renk teması (`blue` varsayılan | `neutral` | `sage` | `rose` | `amber` |
    /// `graphite`; [`ColorThemeId`]): `source = "static"` iken temel paleti verir
    /// ([`crate::ColorTheme::palette`]). `source = "wallpaper"` iken yalnızca üretilmiş palette
    /// eksik kalan rolleri doldurur. Bilinmeyen ad: uyarı + `blue`.
    pub theme: Choice<ColorThemeId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_container: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_bright: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_surface: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_surface_variant: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_primary: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<Color>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Color>,
}

impl ColorsConfig {
    /// Açıkça yazılmış renkler.
    pub fn patch(&self) -> PalettePatch {
        macro_rules! all {
            ($($f:ident),*) => { PalettePatch { $($f: self.$f),* } };
        }
        palette_fields!(all)
    }

    /// Etkin kip. `dark`/`light` olduğu gibi; `auto` sırayla: `generated` (üretilmiş paletin
    /// kayıtlı kipi, bkz. [`GeneratedPalette::recorded_mode`]; yalnızca `source = "wallpaper"`
    /// iken) → statik temanın yüzey parlaklığı (açıkça yazılmış `surface` dahil;
    /// [`ThemeMode::of_surface`]) → koyu (statik varsayılanın yüzeyi koyudur).
    pub fn resolve_mode(&self, generated: Option<ThemeMode>) -> ThemeMode {
        if let Some(m) = self.mode.value.fixed() {
            return m;
        }
        if self.source == ColorSource::Wallpaper
            && let Some(m) = generated
        {
            return m;
        }
        self.surface.map_or(ThemeMode::Dark, ThemeMode::of_surface)
    }
}

/// Katmanları çözer: seçili renk temasının kip paleti ([`crate::ColorTheme::palette`]; varsayılan
/// Nuxut mavisi = [`Palette::for_mode`]) ← (source = wallpaper ise) üretilmiş ← açık renkler;
/// `contrast = "high"` ise sonuç [`Palette::high_contrast`]'tan geçer. `mode` çözülmüş kiptir
/// ([`ColorsConfig::resolve_mode`]).
pub fn resolve_palette(colors: &ColorsConfig, mode: ThemeMode, generated: Option<&PalettePatch>) -> Palette {
    let mut p = colors.theme.value.theme().palette(mode);
    if colors.source == ColorSource::Wallpaper
        && let Some(g) = generated
    {
        p.apply(g);
    }
    p.apply(&colors.patch());
    if colors.contrast.value.is_high() { p.high_contrast() } else { p }
}

/// Üretilmiş palet dosyasının adı (`<state_dir>/palette.toml`).
pub const PALETTE_FILE: &str = "palette.toml";

/// `$XDG_STATE_HOME/nuxut/palette.toml` (genelde `~/.local/state/nuxut/palette.toml`).
pub fn palette_path() -> Option<PathBuf> {
    Some(dirs::state_dir()?.join("nuxut").join(PALETTE_FILE))
}

/// `palette.toml` içeriği. Üretici (nuxut-palette) doldurur ve [`save`](Self::save) ile
/// yazar; tema katmanı yalnızca `colors`'ı okur, `material` uygulamalara açıktır.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct GeneratedPalette {
    pub meta: PaletteMeta,
    /// framework.toml `[colors]` ile aynı 11 rol, üretimdeki kipte (`meta.mode`).
    pub colors: PalettePatch,
    /// Aynı 11 rol koyu kipte (eski dosyalarda yok → `colors`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark: Option<PalettePatch>,
    /// Aynı 11 rol açık kipte (eski dosyalarda yok → `colors`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub light: Option<PalettePatch>,
    /// Seçili kipin tüm Material 3 rolleri (`primary_container`, `tertiary`…). En fazla
    /// [`MAX_MATERIAL_ROLES`] girdi; fazlası yüklemede atılır.
    pub material: std::collections::BTreeMap<String, Color>,
}

/// En fazla Material rolü (M3 ~50 rol tanımlar; pay bırakıldı).
pub const MAX_MATERIAL_ROLES: usize = 128;

/// Palet bilgisi (yalnızca bilgi; tema okumaz).
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct PaletteMeta {
    /// Tohum renk (`#rrggbb`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Şema türü (`tonal-spot`, `vibrant`…).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// `dark` | `light`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contrast: Option<f32>,
    /// Paletin üretildiği görsel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

impl GeneratedPalette {
    /// `mode` kipinin rolleri. Kip bölümü yoksa (eski dosya) `colors`; ikinci değer, `colors`'ın
    /// başka kipte üretildiği bilindiğinde `true` (çağıran uyarır).
    pub fn colors_for(&self, mode: ThemeMode) -> (&PalettePatch, bool) {
        let section = match mode {
            ThemeMode::Dark => self.dark.as_ref(),
            ThemeMode::Light => self.light.as_ref(),
        };
        match section {
            Some(p) => (p, false),
            None => (&self.colors, self.meta.mode.as_deref().is_some_and(|m| m != mode.name())),
        }
    }

    /// Paletin kayıtlı kipi (`auto` için): `meta.mode`; yoksa ya da tanınmıyorsa `colors`
    /// bölümündeki yüzeyin parlaklığı; o da yoksa `None`.
    pub fn recorded_mode(&self) -> Option<ThemeMode> {
        self.meta.mode.as_deref().and_then(ThemeMode::from_name).or_else(|| self.colors.surface.map(ThemeMode::of_surface))
    }

    /// Dosyayı yükler. Yoksa `from_file = false` ve boş palet.
    pub fn load(path: &Path) -> Result<Loaded<Self>, ConfigError> {
        let mut loaded: Loaded<Self> = nuxut_config::load(path)?;
        let m = &mut loaded.value.material;
        if m.len() > MAX_MATERIAL_ROLES {
            let extra: Vec<String> = m.keys().skip(MAX_MATERIAL_ROLES).cloned().collect();
            for k in &extra {
                m.remove(k);
            }
            loaded.warnings.push(format!("[material] {} rolü aşıyor; fazlası atıldı", MAX_MATERIAL_ROLES));
        }
        m.retain(|k, _| k.len() <= 64 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
        Ok(loaded)
    }

    /// Atomik yazım: aynı dizinde geçici dosya + `rename`. İzleyiciler hiçbir zaman yarım
    /// dosya görmez. Dizin yoksa oluşturulur.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().ok_or_else(|| std::io::Error::other("palet yolunun dizini yok"))?;
        std::fs::create_dir_all(dir)?;
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        let tmp = dir.join(format!(".{PALETTE_FILE}.{}.tmp", std::process::id()));
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(b"# nuxut tarafindan uretildi; elle duzenleme framework.toml'da yapilir.\n")?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Color {
        Color::from_hex(s).unwrap()
    }

    /// Türetilmiş başarı rengi: iki kipte, standart ve yüksek karşıtlıkta, üç yüzeye karşı
    /// ≥ 4.5 (metin; simge için gereken 3'ün de üstü); yeşil kalır, `error`'dan ayrışır;
    /// deterministik.
    #[test]
    fn derived_success_contrast_and_hue() {
        use crate::contrast::{TEXT_MIN, contrast_ratio};
        use material_colors::hct::Hct;
        let hue = |c: Color| {
            let b = |v: f32| (v * 255.0).round() as u8;
            Hct::new(material_colors::color::Argb::new(255, b(c.r), b(c.g), b(c.b))).get_hue()
        };
        // Turuncu birincil renkli bir duvar kâğıdı paleti de (uyumlandırma ≤ 15°).
        let orange = Palette { primary: c("#ffb870"), ..Palette::default() };
        for p in [Palette::default(), Palette::light(), orange] {
            for p in [p, p.high_contrast()] {
                let s = p.success();
                for bg in [p.surface, p.surface_container, p.surface_bright] {
                    assert!(contrast_ratio(s, bg) >= TEXT_MIN, "{s:?} / {bg:?}");
                }
                let h = hue(s);
                assert!((100.0..=175.0).contains(&h), "yeşil kalır: {h}");
                assert_ne!(s, p.error);
                assert_eq!(s, p.success());
            }
        }
        // Koyu kipte açık, açık kipte koyu ton.
        assert!(crate::contrast::relative_luminance(Palette::default().success()) > crate::contrast::relative_luminance(Palette::light().success()));
    }

    #[test]
    fn color_role_names_and_lookup() {
        // Her alanı farklı yapılmış palet: get her rolü kendi alanına eşler (karışma yok).
        let mut p = Palette::default();
        let fields: [&mut Color; 11] = [
            &mut p.surface,
            &mut p.surface_container,
            &mut p.surface_bright,
            &mut p.on_surface,
            &mut p.on_surface_variant,
            &mut p.primary,
            &mut p.on_primary,
            &mut p.error,
            &mut p.on_error,
            &mut p.outline,
            &mut p.shadow,
        ];
        for (i, f) in fields.into_iter().enumerate() {
            *f = Color::from_rgba8(i as u8, 0, 0, 255);
        }
        for (i, role) in ColorRole::ALL.into_iter().enumerate() {
            assert_eq!(p.get(role), Color::from_rgba8(i as u8, 0, 0, 255), "{}", role.name());
            assert_eq!(ColorRole::from_name(role.name()), Some(role));
        }
        assert_eq!(ColorRole::from_name(" primary "), Some(ColorRole::Primary));
        assert_eq!(ColorRole::from_name("tertiary"), None);
        // Adlar [colors] anahtarlarıyla aynı: her ad ColorsConfig'te tanınır.
        for role in ColorRole::ALL {
            let toml = format!("{} = \"#102030\"\n", role.name());
            let cfg: ColorsConfig = toml::from_str(&toml).unwrap();
            let p = resolve_palette(&cfg, ThemeMode::Dark, None);
            assert_eq!(p.get(role), c("#102030"), "{}", role.name());
        }
    }

    #[test]
    fn layering_order() {
        let generated = PalettePatch { primary: Some(c("#ff0000")), surface: Some(c("#111111")), ..Default::default() };
        let mut colors = ColorsConfig { surface: Some(c("#222222")), ..Default::default() };
        // Statik kaynakta üretilmiş palet yok sayılır.
        let p = resolve_palette(&colors, ThemeMode::Dark, Some(&generated));
        assert_eq!(p.primary, Palette::default().primary);
        assert_eq!(p.surface, c("#222222"));
        // Duvar kağıdı kaynağında üretilmiş palet gelir, açık renk yine kazanır.
        colors.source = ColorSource::Wallpaper;
        let p = resolve_palette(&colors, ThemeMode::Dark, Some(&generated));
        assert_eq!(p.primary, c("#ff0000"));
        assert_eq!(p.surface, c("#222222"));
        assert_eq!(p.outline, Palette::default().outline);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("nuxut-palette-test-{}", std::process::id()));
        let path = dir.join(PALETTE_FILE);
        let g = GeneratedPalette {
            meta: PaletteMeta {
                source: Some("#4a6fa5".into()),
                scheme: Some("tonal-spot".into()),
                mode: Some("dark".into()),
                contrast: Some(0.0),
                image: Some("/tmp/w.png".into()),
            },
            colors: Palette::default().into(),
            dark: None,
            light: Some(Palette::light().into()),
            material: [("tertiary".to_owned(), Color::WHITE)].into_iter().collect(),
        };
        g.save(&path).unwrap();
        let loaded = GeneratedPalette::load(&path).unwrap();
        assert!(loaded.from_file && loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.value, g);
        // Geçici dosya kalmamalı.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
