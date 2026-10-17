//! Tam Material 3 renk rolü kümesi: tek kipin kararlı, adlandırılmış alanlı yapısı ([`Scheme`])
//! ve rol tanımlayıcısı ([`Role`]).
//!
//! Şablonlar ve dış üreticiler (GTK/Qt/KDE) rolleri [`Scheme::iter`] ile ad → renk olarak
//! tüketir. Her rolün iki adı vardır:
//! - [`Role::snake`]: `primary_container` — matugen şablon adları (`{{colors.<ad>…}}`) ve
//!   `palette.toml [material]` anahtarları;
//! - [`Role::camel`]: `primaryContainer` — Material Theme Builder / MCU JSON adları.
//!
//! Rol kümesi ve sırası (`Role::ALL`) kararlıdır: yeni rol yalnızca sona eklenir, ad
//! değişmez. Değerler `material-colors` `DynamicScheme`'inden gelir (tema katmanının
//! `palette.toml` ile aldığı renklerle bayt bayt aynı). Material'de olmayan `success` ve
//! `warning` aileleri sabit bir tohum renginin kaynağa **uyumlandırılmasıyla** (MCU
//! `harmonize`: ton farkının yarısı, en çok 15°) türetilir — matugen'in "custom color"
//! yöntemiyle aynı tonlar (açık 40/100/90/10, koyu 80/20/30/90).

use material_colors::blend::harmonize;
use material_colors::color::Argb as Rgb;
use material_colors::dynamic_color::DynamicScheme;
use material_colors::palette::CorePalette;
use nuxut_core::Color;

use nuxut_theme::ContrastMode;

use crate::scheme::{Mode, Variant, from_rgb, to_rgb};

/// `success` ailesinin tohumu (Material yeşil 600); kaynağa uyumlandırılır. Tek kaynak
/// [`nuxut_theme::SUCCESS_SEED`] (temanın türetilmiş `Palette::success`'i de onu kullanır).
pub const SUCCESS_SEED: Color = nuxut_theme::SUCCESS_SEED;
/// `warning` ailesinin tohumu (Material kehribar 700); kaynağa uyumlandırılır.
// renk-matematiği: çıktı rengi değil, harmonize girdisi (hue/kroma kaynağı; tonları MCU verir)
pub const WARNING_SEED: Color = Color::from_rgba8(0xff, 0xa0, 0x00, 0xff);

macro_rules! define_roles {
    ($( $(#[$doc:meta])* $field:ident, $variant:ident, $camel:literal; )*) => {
        /// Material 3 renk rolü (+ Nuxut `success`/`warning`). Sıra [`Role::ALL`]'dadır.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Role {
            $( $(#[$doc])* $variant, )*
        }

        impl Role {
            /// Tüm roller, kararlı sırada.
            pub const ALL: &'static [Role] = &[$( Role::$variant, )*];

            /// snake_case ad (matugen uyumlu): `on_primary_container`.
            pub const fn snake(self) -> &'static str {
                match self { $( Role::$variant => stringify!($field), )* }
            }

            /// camelCase ad (Material Theme Builder / MCU uyumlu): `onPrimaryContainer`.
            pub const fn camel(self) -> &'static str {
                match self { $( Role::$variant => $camel, )* }
            }
        }

        /// Tek kipin tüm renk rolleri. Alanlar [`Role`] ile birebir; belge rolün kullanımını
        /// anlatır. Renkler opaktır (α = 1).
        #[derive(Debug, Clone, Copy, PartialEq)]
        pub struct Scheme {
            $( $(#[$doc])* pub $field: Color, )*
        }

        impl Scheme {
            /// Rolün rengi.
            pub const fn get(&self, role: Role) -> Color {
                match role { $( Role::$variant => self.$field, )* }
            }

            /// `role`'ün rengini değiştirir.
            pub fn set(&mut self, role: Role, color: Color) {
                match role { $( Role::$variant => self.$field = color, )* }
            }

            fn from_dynamic(s: &DynamicScheme, extra: &Extra) -> Self {
                define_roles!(@build s, extra; $( $field ),*)
            }
        }
    };
    (@build $s:ident, $e:ident; $( $field:ident ),*) => {
        Self { $( $field: define_roles!(@value $s, $e, $field), )* }
    };
    (@value $s:ident, $e:ident, success) => { $e.success.color };
    (@value $s:ident, $e:ident, on_success) => { $e.success.on_color };
    (@value $s:ident, $e:ident, success_container) => { $e.success.container };
    (@value $s:ident, $e:ident, on_success_container) => { $e.success.on_container };
    (@value $s:ident, $e:ident, warning) => { $e.warning.color };
    (@value $s:ident, $e:ident, on_warning) => { $e.warning.on_color };
    (@value $s:ident, $e:ident, warning_container) => { $e.warning.container };
    (@value $s:ident, $e:ident, on_warning_container) => { $e.warning.on_container };
    (@value $s:ident, $e:ident, $field:ident) => { from_rgb($s.$field()) };
}

define_roles! {
    /// Vurgu: dolu düğme, etkin durum, seçim, bağlantı.
    primary, Primary, "primary";
    /// `primary` üstündeki metin/ikon.
    on_primary, OnPrimary, "onPrimary";
    /// Daha az vurgulu dolgu (seçili satır, etkin olmayan pencerede seçim).
    primary_container, PrimaryContainer, "primaryContainer";
    on_primary_container, OnPrimaryContainer, "onPrimaryContainer";
    /// İkincil vurgu (filtre çipi, tonlu düğme).
    secondary, Secondary, "secondary";
    on_secondary, OnSecondary, "onSecondary";
    secondary_container, SecondaryContainer, "secondaryContainer";
    on_secondary_container, OnSecondaryContainer, "onSecondaryContainer";
    /// Üçüncül vurgu (ziyaret edilmiş bağlantı, karşıt vurgu).
    tertiary, Tertiary, "tertiary";
    on_tertiary, OnTertiary, "onTertiary";
    tertiary_container, TertiaryContainer, "tertiaryContainer";
    on_tertiary_container, OnTertiaryContainer, "onTertiaryContainer";
    /// Hata / yıkıcı eylem.
    error, Error, "error";
    on_error, OnError, "onError";
    error_container, ErrorContainer, "errorContainer";
    on_error_container, OnErrorContainer, "onErrorContainer";
    /// Pencere zemini (Material 3'te `surface` ile aynı ton).
    background, Background, "background";
    on_background, OnBackground, "onBackground";
    /// Ana yüzey (pencere zemini).
    surface, Surface, "surface";
    /// En koyu yüzey tonu (her iki kipte de `surface`'ten koyu).
    surface_dim, SurfaceDim, "surfaceDim";
    /// En açık yüzey tonu.
    surface_bright, SurfaceBright, "surfaceBright";
    surface_container_lowest, SurfaceContainerLowest, "surfaceContainerLowest";
    /// İçerik alanı, kenar çubuğu.
    surface_container_low, SurfaceContainerLow, "surfaceContainerLow";
    /// Kart, başlık çubuğu.
    surface_container, SurfaceContainer, "surfaceContainer";
    /// Açılır pencere, menü, iletişim kutusu, düğme.
    surface_container_high, SurfaceContainerHigh, "surfaceContainerHigh";
    surface_container_highest, SurfaceContainerHighest, "surfaceContainerHighest";
    /// Yüzey üstündeki metin.
    on_surface, OnSurface, "onSurface";
    surface_variant, SurfaceVariant, "surfaceVariant";
    /// İkincil/pasif metin.
    on_surface_variant, OnSurfaceVariant, "onSurfaceVariant";
    /// İpucu balonu, snackbar zemini.
    inverse_surface, InverseSurface, "inverseSurface";
    inverse_on_surface, InverseOnSurface, "inverseOnSurface";
    /// `inverse_surface` üstündeki vurgu.
    inverse_primary, InversePrimary, "inversePrimary";
    /// Belirgin kenar (odak halkası dışı çerçeve).
    outline, Outline, "outline";
    /// Ayırıcı, ince kenar.
    outline_variant, OutlineVariant, "outlineVariant";
    /// Gölge rengi (opak; saydamlığı tüketici verir).
    shadow, Shadow, "shadow";
    /// Kip arka planı karartma rengi (opak; saydamlığı tüketici verir).
    scrim, Scrim, "scrim";
    surface_tint, SurfaceTint, "surfaceTint";
    /// Kipten bağımsız sabit tonlar (`*_fixed`: iki kipte aynı).
    primary_fixed, PrimaryFixed, "primaryFixed";
    primary_fixed_dim, PrimaryFixedDim, "primaryFixedDim";
    on_primary_fixed, OnPrimaryFixed, "onPrimaryFixed";
    on_primary_fixed_variant, OnPrimaryFixedVariant, "onPrimaryFixedVariant";
    secondary_fixed, SecondaryFixed, "secondaryFixed";
    secondary_fixed_dim, SecondaryFixedDim, "secondaryFixedDim";
    on_secondary_fixed, OnSecondaryFixed, "onSecondaryFixed";
    on_secondary_fixed_variant, OnSecondaryFixedVariant, "onSecondaryFixedVariant";
    tertiary_fixed, TertiaryFixed, "tertiaryFixed";
    tertiary_fixed_dim, TertiaryFixedDim, "tertiaryFixedDim";
    on_tertiary_fixed, OnTertiaryFixed, "onTertiaryFixed";
    on_tertiary_fixed_variant, OnTertiaryFixedVariant, "onTertiaryFixedVariant";
    /// Başarı (Nuxut; [`SUCCESS_SEED`] kaynağa uyumlandırılır).
    success, Success, "success";
    on_success, OnSuccess, "onSuccess";
    success_container, SuccessContainer, "successContainer";
    on_success_container, OnSuccessContainer, "onSuccessContainer";
    /// Uyarı (Nuxut; [`WARNING_SEED`] kaynağa uyumlandırılır).
    warning, Warning, "warning";
    on_warning, OnWarning, "onWarning";
    warning_container, WarningContainer, "warningContainer";
    on_warning_container, OnWarningContainer, "onWarningContainer";
}

impl Role {
    /// snake_case, camelCase ya da kebab-case addan rol (`on_primary`, `onPrimary`,
    /// `on-primary`). Bilinmeyen adda `None`.
    pub fn from_name(name: &str) -> Option<Self> {
        let snake = name.replace('-', "_");
        Self::ALL.iter().copied().find(|r| r.snake() == snake || r.camel() == name)
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.snake())
    }
}

/// Yüzey rolleri: metin ve metin dışı ön planların üstünde durduğu zeminler.
pub const SURFACE_ROLES: [Role; 10] = [
    Role::Background,
    Role::Surface,
    Role::SurfaceDim,
    Role::SurfaceBright,
    Role::SurfaceContainerLowest,
    Role::SurfaceContainerLow,
    Role::SurfaceContainer,
    Role::SurfaceContainerHigh,
    Role::SurfaceContainerHighest,
    Role::SurfaceVariant,
];

/// `on_X` / `X` metin çiftleri (dolgu ve kap aileleri).
pub const ON_PAIRS: [(Role, Role); 12] = [
    (Role::OnPrimary, Role::Primary),
    (Role::OnPrimaryContainer, Role::PrimaryContainer),
    (Role::OnSecondary, Role::Secondary),
    (Role::OnSecondaryContainer, Role::SecondaryContainer),
    (Role::OnTertiary, Role::Tertiary),
    (Role::OnTertiaryContainer, Role::TertiaryContainer),
    (Role::OnError, Role::Error),
    (Role::OnErrorContainer, Role::ErrorContainer),
    (Role::OnSuccess, Role::Success),
    (Role::OnSuccessContainer, Role::SuccessContainer),
    (Role::OnWarning, Role::Warning),
    (Role::OnWarningContainer, Role::WarningContainer),
];

/// Sabit tonlar: (`X_fixed`, `X_fixed_dim`, ikisinin de üstündeki metin rolleri).
const FIXED: [(Role, Role, [Role; 2]); 3] = [
    (Role::PrimaryFixed, Role::PrimaryFixedDim, [Role::OnPrimaryFixed, Role::OnPrimaryFixedVariant]),
    (Role::SecondaryFixed, Role::SecondaryFixedDim, [Role::OnSecondaryFixed, Role::OnSecondaryFixedVariant]),
    (Role::TertiaryFixed, Role::TertiaryFixedDim, [Role::OnTertiaryFixed, Role::OnTertiaryFixedVariant]),
];

/// Bir ailenin dört rolü (`X`, `on_X`, `X_container`, `on_X_container`).
#[derive(Debug, Clone, Copy)]
struct Group {
    color: Color,
    on_color: Color,
    container: Color,
    on_container: Color,
}

struct Extra {
    success: Group,
    warning: Group,
}

/// Tohumu kaynağa uyumlandırıp (hue en çok 15° döner) MCU özel renk tonlarını verir.
fn harmonized_group(seed: Color, source: Rgb, dark: bool) -> Group {
    let value = harmonize(to_rgb(seed), source);
    let tones = CorePalette::of(value).primary;
    let t = |n| from_rgb(tones.tone(n));
    if dark {
        Group { color: t(80), on_color: t(20), container: t(30), on_container: t(90) }
    } else {
        Group { color: t(40), on_color: t(100), container: t(90), on_container: t(10) }
    }
}

impl Scheme {
    /// Kaynak renkten tek kipin tüm rolleri. `contrast` -1.0 … 1.0 (sonlu değilse 0);
    /// `success`/`warning` aileleri kontrasttan etkilenmez (sabit tonlar).
    pub fn generate(source: Color, variant: Variant, mode: Mode, contrast: f64) -> Self {
        let contrast = if contrast.is_finite() { contrast.clamp(-1.0, 1.0) } else { 0.0 };
        Self::from_rgb_source(to_rgb(source), variant, mode == Mode::Dark, contrast)
    }

    pub(crate) fn from_rgb_source(source: Rgb, variant: Variant, dark: bool, contrast: f64) -> Self {
        let s = DynamicScheme::by_variant(source, &variant.mcu(), dark, Some(contrast));
        let extra = Extra {
            success: harmonized_group(SUCCESS_SEED, source, dark),
            warning: harmonized_group(WARNING_SEED, source, dark),
        };
        Self::from_dynamic(&s, &extra)
    }

    /// Nuxut renk temasından (`[colors] theme`) tek kipin tüm rolleri: temanın beş tonal paleti,
    /// tonal-spot ton kuralları; `success`/`warning` temanın tohumuna uyumlandırılır. Temanın 11
    /// token'ı ([`nuxut_theme::ColorTheme::palette`]; Nuxut mavisi koyuda tarihî elle değerler)
    /// karşılık gelen rollere yazılır: Nuxut'un çizdiği ile dış uygulamaların gördüğü aynı.
    pub fn from_color_theme(theme: &nuxut_theme::ColorTheme, mode: Mode, contrast: f64) -> Self {
        let dark = mode == Mode::Dark;
        let source = to_rgb(theme.seed);
        let extra = Extra {
            success: harmonized_group(SUCCESS_SEED, source, dark),
            warning: harmonized_group(WARNING_SEED, source, dark),
        };
        let mut s = Self::from_dynamic(&theme.dynamic_scheme(dark, contrast), &extra);
        if contrast == 0.0 {
            let tokens = theme.palette(mode.into());
            for (ours, m3) in crate::THEME_ROLES {
                if let (Some(role), Some(color)) = (Role::from_name(m3), nuxut_theme::ColorRole::from_name(ours).map(|r| tokens.get(r))) {
                    s.set(role, color);
                }
            }
            s.surface_tint = s.primary;
        }
        s
    }

    /// [`Self::generate`] + karşıtlık kipi: `High` iken sonuç [`Self::high_contrast`]'tan geçer;
    /// `Standard` `generate` ile bayt bayt aynıdır.
    pub fn generate_with(source: Color, variant: Variant, mode: Mode, contrast: f64, level: ContrastMode) -> Self {
        let s = Self::generate(source, variant, mode, contrast);
        if level.is_high() { s.high_contrast() } else { s }
    }

    /// Yüksek karşıtlık karşılığı (docs/design/accessibility.md §1; tema katmanındaki
    /// [`nuxut_theme::Palette::high_contrast`] ile aynı yöntem): yüzey, kap dolguları, gölge ve
    /// perde aynı kalır; ön planlar HCT tonuyla zeminlerinden uzağa itilir (hue/kroma korunur).
    /// Garanti (her kaynak, varyant ve kipte; testle korunur), `S` = tüm yüzey rolleri
    /// (`background`, `surface*`, `surface_container*`, `surface_variant`):
    /// - `on_surface`, `on_background`, `on_surface_variant` / S ≥ 4.5 (hedef 7; `surface_variant`
    ///   hariç — o dolgu rolüne hedef uygulanmaz, ikincil metin birincile eşitlenmesin);
    /// - `primary`, `secondary`, `tertiary`, `error`, `success`, `warning` / S ≥ 4.5 (metin,
    ///   ikon, odak halkası olarak kullanılır);
    /// - her `on_X` / `X` ve `on_X_container` / `X_container` ≥ 4.5 (hedef 7);
    /// - `inverse_on_surface`, `inverse_primary` / `inverse_surface` ≥ 4.5;
    /// - `on_X_fixed`, `on_X_fixed_variant` / `X_fixed`, `X_fixed_dim` ≥ 4.5;
    /// - `outline` / S ≥ 4.5 (belirgin kenar), `outline_variant` / S ≥ 3 (ayırıcı, WCAG 1.4.11).
    ///
    /// `surface_tint` `primary`'yi izler. İdempotenttir.
    pub fn high_contrast(&self) -> Self {
        use nuxut_theme::contrast::{NON_TEXT_MIN, TEXT_MIN, TEXT_TARGET, ensure_contrast, ensure_pair};
        use Role as R;
        let mut s = *self;
        let surfaces: Vec<Color> = SURFACE_ROLES.iter().map(|r| s.get(*r)).collect();
        // Metin: zeminlere hedef 7; `surface_variant` (koyuda T30, dolgu rolü) yalnızca 4.5 ister —
        // ona da 7 istemek ikincil metni birincile eşitler, metin hiyerarşisi kaybolur.
        let text_bgs: Vec<Color> =
            SURFACE_ROLES.iter().filter(|r| **r != R::SurfaceVariant).map(|r| s.get(*r)).collect();
        for r in [R::OnSurface, R::OnBackground, R::OnSurfaceVariant] {
            let c = ensure_contrast(s.get(r), &text_bgs, TEXT_TARGET);
            s.set(r, ensure_contrast(c, &surfaces, TEXT_MIN));
        }
        for r in [R::Primary, R::Secondary, R::Tertiary, R::Error, R::Success, R::Warning] {
            s.set(r, ensure_contrast(s.get(r), &surfaces, TEXT_MIN));
        }
        for (on, bg) in ON_PAIRS {
            let (f, b) = ensure_pair(s.get(on), s.get(bg), TEXT_TARGET, TEXT_MIN);
            s.set(on, f);
            s.set(bg, b);
        }
        (s.inverse_on_surface, s.inverse_surface) = ensure_pair(s.inverse_on_surface, s.inverse_surface, TEXT_TARGET, TEXT_MIN);
        s.inverse_primary = ensure_contrast(s.inverse_primary, &[s.inverse_surface], TEXT_MIN);
        for (fixed, dim, ons) in FIXED {
            let bgs = [s.get(fixed), s.get(dim)];
            for on in ons {
                s.set(on, ensure_contrast(s.get(on), &bgs, TEXT_TARGET));
            }
        }
        s.outline = ensure_contrast(s.outline, &surfaces, TEXT_MIN);
        s.outline_variant = ensure_contrast(s.outline_variant, &surfaces, NON_TEXT_MIN);
        s.surface_tint = s.primary;
        s
    }

    /// Tüm roller (rol, renk) olarak, [`Role::ALL`] sırasıyla.
    pub fn iter(&self) -> impl Iterator<Item = (Role, Color)> + '_ {
        Role::ALL.iter().map(move |r| (*r, self.get(*r)))
    }

    /// snake_case ad → renk (matugen adları), [`Role::ALL`] sırasıyla.
    pub fn iter_snake(&self) -> impl Iterator<Item = (&'static str, Color)> + '_ {
        self.iter().map(|(r, c)| (r.snake(), c))
    }

    /// camelCase ad → renk (Material Theme Builder adları), [`Role::ALL`] sırasıyla.
    pub fn iter_camel(&self) -> impl Iterator<Item = (&'static str, Color)> + '_ {
        self.iter().map(|(r, c)| (r.camel(), c))
    }

    /// Ada göre rol rengi (snake/camel/kebab; bkz. [`Role::from_name`]).
    pub fn by_name(&self, name: &str) -> Option<Color> {
        Role::from_name(name).map(|r| self.get(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_camel(snake: &str) -> String {
        let mut out = String::new();
        let mut up = false;
        for ch in snake.chars() {
            if ch == '_' {
                up = true;
            } else if up {
                out.extend(ch.to_uppercase());
                up = false;
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn names_are_consistent_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for r in Role::ALL {
            assert_eq!(r.camel(), to_camel(r.snake()), "{r:?}");
            assert!(seen.insert(r.snake()), "yinelenen ad {}", r.snake());
            assert_eq!(Role::from_name(r.snake()), Some(*r));
            assert_eq!(Role::from_name(r.camel()), Some(*r));
            assert_eq!(Role::from_name(&r.snake().replace('_', "-")), Some(*r));
        }
        assert_eq!(Role::ALL.len(), 57);
        assert_eq!(Role::from_name("nope"), None);
    }

    #[test]
    fn get_set_roundtrip() {
        let mut s = Scheme::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0);
        let c = Color::from_rgba8(1, 2, 3, 255);
        for r in Role::ALL {
            s.set(*r, c);
            assert_eq!(s.get(*r), c);
        }
    }

    #[test]
    fn success_and_warning_are_harmonized() {
        use material_colors::hct::Hct;
        let hue = |c: Color| Hct::new(to_rgb(c)).get_hue();
        let dist = |a: f64, b: f64| {
            let d = (a - b).rem_euclid(360.0);
            d.min(360.0 - d)
        };
        let seed_hue = hue(SUCCESS_SEED);
        // Kaynak yeşilden uzak (mavi): hue kaynağa doğru döner ama en çok 15°.
        let blue = Color::from_hex("#1e40ff").unwrap();
        let s = Scheme::generate(blue, Variant::TonalSpot, Mode::Light, 0.0);
        let moved = dist(hue(s.success), seed_hue);
        assert!(moved > 3.0 && moved <= 15.5, "{moved}");
        assert!(dist(hue(s.success), hue(blue)) < dist(seed_hue, hue(blue)));
        // Kaynak tohuma çok yakınsa neredeyse hiç dönmez.
        let s = Scheme::generate(SUCCESS_SEED, Variant::TonalSpot, Mode::Light, 0.0);
        assert!(dist(hue(s.success), seed_hue) < 2.0);
    }
}
