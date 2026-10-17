//! Kaynak renkten Material You şeması (`material-colors`, MCU portu) ve Nuxut rol eşlemesi.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use material_colors::blend::harmonize;
use material_colors::color::Argb as Rgb;
use material_colors::dynamic_color::Variant as McuVariant;
use material_colors::hct::Hct;
use nuxut_core::Color;
use nuxut_theme::ContrastMode;

use crate::roles::Scheme;

/// Şema varyantı (Material 3 Dynamic Color).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    #[default]
    TonalSpot,
    Vibrant,
    Expressive,
    Neutral,
    Fidelity,
    Content,
    Monochrome,
    Rainbow,
    FruitSalad,
}

impl Variant {
    pub const ALL: [Self; 9] = [
        Self::TonalSpot,
        Self::Vibrant,
        Self::Expressive,
        Self::Neutral,
        Self::Fidelity,
        Self::Content,
        Self::Monochrome,
        Self::Rainbow,
        Self::FruitSalad,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::TonalSpot => "tonal-spot",
            Self::Vibrant => "vibrant",
            Self::Expressive => "expressive",
            Self::Neutral => "neutral",
            Self::Fidelity => "fidelity",
            Self::Content => "content",
            Self::Monochrome => "monochrome",
            Self::Rainbow => "rainbow",
            Self::FruitSalad => "fruit-salad",
        }
    }

    pub(crate) fn mcu(self) -> McuVariant {
        match self {
            Self::TonalSpot => McuVariant::TonalSpot,
            Self::Vibrant => McuVariant::Vibrant,
            Self::Expressive => McuVariant::Expressive,
            Self::Neutral => McuVariant::Neutral,
            Self::Fidelity => McuVariant::Fidelity,
            Self::Content => McuVariant::Content,
            Self::Monochrome => McuVariant::Monochrome,
            Self::Rainbow => McuVariant::Rainbow,
            Self::FruitSalad => McuVariant::FruitSalad,
        }
    }
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Variant {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let norm = s.trim().to_ascii_lowercase().replace('_', "-");
        let norm = norm.strip_prefix("scheme-").unwrap_or(&norm);
        Self::ALL.into_iter().find(|v| v.name() == norm).ok_or_else(|| {
            let names: Vec<_> = Self::ALL.iter().map(|v| v.name()).collect();
            format!("bilinmeyen şema '{s}' (geçerli: {})", names.join(", "))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dark,
    Light,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dark" => Ok(Self::Dark),
            "light" => Ok(Self::Light),
            _ => Err(format!("bilinmeyen kip '{s}' (dark|light)")),
        }
    }
}

// Yalnızca çözülmüş kiple (`ThemeMode`) iki yönlü eşleme. `nuxut_theme::ColorMode` `auto`
// taşır; ondan `Mode`'a dönüşüm bilerek yok (önce `ColorsConfig::resolve_mode`).
impl From<nuxut_theme::ThemeMode> for Mode {
    fn from(m: nuxut_theme::ThemeMode) -> Self {
        match m {
            nuxut_theme::ThemeMode::Dark => Self::Dark,
            nuxut_theme::ThemeMode::Light => Self::Light,
        }
    }
}

impl From<Mode> for nuxut_theme::ThemeMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Dark => Self::Dark,
            Mode::Light => Self::Light,
        }
    }
}

impl From<Mode> for nuxut_theme::ColorMode {
    fn from(m: Mode) -> Self {
        nuxut_theme::ThemeMode::from(m).into()
    }
}

/// Bir kipteki tüm renk rolleri: Material 3 rolleri + `success`/`warning` aileleri
/// (snake_case, bkz. [`crate::Role`]), `source_color` ve `term0`…`term15`. Adlandırılmış alanlı
/// tam rol kümesi [`Roles::scheme`].
#[derive(Debug, Clone, PartialEq)]
pub struct Roles {
    scheme: Scheme,
    map: BTreeMap<&'static str, Color>,
}

impl Roles {
    pub fn get(&self, role: &str) -> Option<Color> {
        self.map.get(role).copied()
    }

    /// Ad sırasıyla (alfabetik) tüm girdiler.
    pub fn iter(&self) -> impl Iterator<Item = (&'static str, Color)> + '_ {
        self.map.iter().map(|(k, v)| (*k, *v))
    }

    /// Bu kipin adlandırılmış alanlı tam Material rol kümesi.
    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// `framework.toml [colors]` rolleri (DESIGN.md) ile Material rolleri arasındaki eşleme.
/// Anahtar Nuxut rolü, değer Material rolü.
pub const THEME_ROLES: [(&str, &str); 11] = [
    ("surface", "surface"),
    ("surface_container", "surface_container"),
    ("surface_bright", "surface_container_high"),
    ("on_surface", "on_surface"),
    ("on_surface_variant", "on_surface_variant"),
    ("primary", "primary"),
    ("on_primary", "on_primary"),
    ("error", "error"),
    ("on_error", "on_error"),
    ("outline", "outline_variant"),
    ("shadow", "shadow"),
];

/// Üretilmiş palet: iki kip de hesaplanır (şablonlar `dark`/`light`'a ayrı erişebilir).
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub source: Color,
    pub variant: Variant,
    pub mode: Mode,
    /// -1.0 (düşük) … 0.0 (standart) … 1.0 (yüksek).
    pub contrast: f64,
    /// Karşıtlık kipi (`framework.toml [colors] contrast`); `High` iken iki kip de
    /// [`Scheme::high_contrast`]'tan geçer.
    pub contrast_mode: ContrastMode,
    pub dark: Roles,
    pub light: Roles,
}

impl Palette {
    pub fn generate(source: Color, variant: Variant, mode: Mode, contrast: f64) -> Self {
        Self::generate_with(source, variant, mode, contrast, ContrastMode::Standard)
    }

    /// [`Self::generate`] + karşıtlık kipi (bkz. [`Scheme::generate_with`]). Terminal renkleri
    /// yüksek karşıtlıklı rollerden türetilir.
    pub fn generate_with(source: Color, variant: Variant, mode: Mode, contrast: f64, contrast_mode: ContrastMode) -> Self {
        let contrast = if contrast.is_finite() { contrast.clamp(-1.0, 1.0) } else { 0.0 };
        let rgb = to_rgb(source);
        Self {
            source,
            variant,
            mode,
            contrast,
            contrast_mode,
            dark: roles(rgb, variant, true, contrast, contrast_mode),
            light: roles(rgb, variant, false, contrast, contrast_mode),
        }
    }

    /// Nuxut renk temasından iki kipin tam rol kümesi (`[colors] source = "static"`; dış
    /// uygulamalara dışa aktarım). `source` temanın tohumu, `variant` tonal-spot (temanın ton
    /// kuralları). Karşıtlık kipi `High` iken iki kip de [`Scheme::high_contrast`]'tan geçer.
    pub fn from_color_theme(theme: &nuxut_theme::ColorTheme, mode: Mode, contrast_mode: ContrastMode) -> Self {
        let rgb = to_rgb(theme.seed);
        let build = |m: Mode| {
            let scheme = Scheme::from_color_theme(theme, m, 0.0);
            let scheme = if contrast_mode.is_high() { scheme.high_contrast() } else { scheme };
            let mut map: BTreeMap<&'static str, Color> = scheme.iter_snake().collect();
            map.insert("source_color", theme.seed);
            terminal(&mut map, rgb, m == Mode::Dark);
            Roles { scheme, map }
        };
        Self {
            source: theme.seed,
            variant: Variant::TonalSpot,
            mode,
            contrast: 0.0,
            contrast_mode,
            dark: build(Mode::Dark),
            light: build(Mode::Light),
        }
    }

    /// Seçili kipin rolleri.
    pub fn active(&self) -> &Roles {
        match self.mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }

    pub fn roles(&self, mode: Mode) -> &Roles {
        match mode {
            Mode::Dark => &self.dark,
            Mode::Light => &self.light,
        }
    }

    /// `mode` kipinin adlandırılmış alanlı tam rol kümesi.
    pub fn scheme(&self, mode: Mode) -> &Scheme {
        self.roles(mode).scheme()
    }

    /// Seçili kipte `framework.toml [colors]` rolleri, `THEME_ROLES` sırasıyla.
    pub fn theme_colors(&self) -> Vec<(&'static str, Color)> {
        self.theme_colors_for(self.mode)
    }

    /// `mode` kipinde `framework.toml [colors]` rolleri, `THEME_ROLES` sırasıyla.
    pub fn theme_colors_for(&self, mode: Mode) -> Vec<(&'static str, Color)> {
        let r = self.roles(mode);
        let fallback = nuxut_theme::Palette::default();
        THEME_ROLES.iter().map(|(ours, m3)| (*ours, r.get(m3).unwrap_or_else(|| default_role(&fallback, ours)))).collect()
    }
}

pub(crate) fn to_rgb(c: Color) -> Rgb {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Rgb::new(255, b(c.r), b(c.g), b(c.b))
}

/// Material rolü üretilemezse (olmamalı) tema varsayılanındaki karşılık: koda gömülü renk yok.
fn default_role(d: &nuxut_theme::Palette, role: &str) -> Color {
    match role {
        "surface" => d.surface,
        "surface_container" => d.surface_container,
        "surface_bright" => d.surface_bright,
        "on_surface" => d.on_surface,
        "on_surface_variant" => d.on_surface_variant,
        "primary" => d.primary,
        "on_primary" => d.on_primary,
        "error" => d.error,
        "on_error" => d.on_error,
        "outline" => d.outline,
        _ => d.shadow,
    }
}

pub(crate) fn from_rgb(c: Rgb) -> Color {
    Color::from_rgba8(c.red, c.green, c.blue, 255)
}

fn roles(source: Rgb, variant: Variant, dark: bool, contrast: f64, level: ContrastMode) -> Roles {
    let scheme = Scheme::from_rgb_source(source, variant, dark, contrast);
    let scheme = if level.is_high() { scheme.high_contrast() } else { scheme };
    let mut m: BTreeMap<&'static str, Color> = scheme.iter_snake().collect();
    m.insert("source_color", from_rgb(source));
    terminal(&mut m, source, dark);
    Roles { scheme, map: m }
}

/// 16 ANSI rengi: standart tonlar kaynak renge uyumlandırılır (`harmonize`), kipine göre
/// okunur tona çekilir. 0/7/8/15 nötr yüzey rollerinden gelir.
fn terminal(m: &mut BTreeMap<&'static str, Color>, source: Rgb, dark: bool) {
    const NAMES: [&str; 16] = [
        "term0", "term1", "term2", "term3", "term4", "term5", "term6", "term7", "term8", "term9", "term10", "term11",
        "term12", "term13", "term14", "term15",
    ];
    // kırmızı, yeşil, sarı, mavi, eflatun, camgöbeği
    const BASE: [u32; 6] = [0xE5_3935, 0x43_A047, 0xFD_D835, 0x1E_88E5, 0x8E_24AA, 0x00_ACC1];
    let (normal, bright) = if dark { (72.0, 82.0) } else { (42.0, 32.0) };
    let get = |m: &BTreeMap<&str, Color>, k: &str| m[k];
    let (c0, c7, c8, c15) = if dark {
        (get(m, "surface_container_high"), get(m, "on_surface_variant"), get(m, "outline"), get(m, "on_surface"))
    } else {
        (get(m, "on_surface"), get(m, "outline"), get(m, "on_surface_variant"), get(m, "surface_container_high"))
    };
    m.insert(NAMES[0], c0);
    m.insert(NAMES[7], c7);
    m.insert(NAMES[8], c8);
    m.insert(NAMES[15], c15);
    for (i, base) in BASE.iter().enumerate() {
        let h = Hct::new(harmonize(Rgb::from_u32(*base), source));
        for (slot, tone) in [(i + 1, normal), (i + 9, bright)] {
            let mut c = h;
            c.set_chroma(h.get_chroma().max(48.0));
            c.set_tone(tone);
            m.insert(NAMES[slot], from_rgb(c.into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variants_and_modes() {
        assert_eq!("tonal-spot".parse::<Variant>(), Ok(Variant::TonalSpot));
        assert_eq!("scheme-fruit_salad".parse::<Variant>(), Ok(Variant::FruitSalad));
        assert!("nope".parse::<Variant>().is_err());
        assert_eq!("LIGHT".parse::<Mode>(), Ok(Mode::Light));
    }

    #[test]
    fn every_theme_role_has_a_default() {
        let d = nuxut_theme::Palette::default();
        let shadow_named: Vec<_> = THEME_ROLES.iter().filter(|(r, _)| default_role(&d, r) == d.shadow).map(|(r, _)| *r).collect();
        assert_eq!(shadow_named, ["shadow"], "her rol kendi varsayılanına eşlenmeli");
    }

    #[test]
    fn has_all_roles_both_modes() {
        let p = Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0);
        assert_eq!(p.dark.len(), p.light.len());
        assert!(p.dark.len() >= 66);
        for (_, m3) in THEME_ROLES {
            assert!(p.dark.get(m3).is_some(), "{m3}");
        }
        // Koyu kipte yüzey koyu, metin açık; açık kipte tersi.
        let lum = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        assert!(lum(p.dark.get("surface").unwrap()) < 0.1);
        assert!(lum(p.dark.get("on_surface").unwrap()) > 0.7);
        assert!(lum(p.light.get("surface").unwrap()) > 0.9);
    }
}
