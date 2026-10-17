//! Nuxut renk temaları: adlandırılmış, elle ayarlanmış kaynak paletler (`[colors] theme`).
//!
//! Bir tema beş tonal paletten oluşur (Material 3 Dynamic Color'ın yapı taşları): birincil,
//! ikincil, üçüncül, nötr ve nötr-varyant. Her palet bir HCT ton çizgisidir (hue + kroma; ton
//! 0…100). Rollerin hangi tonu kullanacağını Material 3 tonal-spot kuralları belirler; tema yalnızca
//! **hangi renk ailesinden** ve **ne kadar doygun** olduğunu seçer. Böylece her tema her kipte
//! (koyu/açık) ve karşıtlık seviyesinde aynı ton mimarisini paylaşır: metin/zemin karşıtlığı
//! temadan bağımsız olarak tonlarla güvenceye alınır (testler her tema × kip için denetler).
//!
//! Duvar kağıdından üretilen paletten (`source = "wallpaper"`, nuxut-palette) bağımsızdır:
//! `source = "static"` iken seçili tema temel paleti verir. Tasarım ve tema karakterleri:
//! `docs/design/color-themes.md`.
//!
//! Yerleşik duvar kağıtları Nuxut mavisiyle çizilmiştir; [`ColorTheme::recolor`] onların
//! renklerini seçili temaya taşır (tonu koruyarak), [`recolor_svg`] bunu SVG metnine uygular.

use material_colors::color::Argb;
use material_colors::dynamic_color::{DynamicScheme, Variant};
use material_colors::hct::Hct;
use material_colors::palette::TonalPalette;

use nuxut_core::Color;

use crate::palette::{ChoiceValue, Palette, ThemeMode};

/// Yerleşik renk teması kimliği (`framework.toml [colors] theme`). Adlar kalıcıdır (dosyaya
/// yazılır); görünen adlar uygulamaların katalogundadır (`color-theme-<ad>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ColorThemeId {
    /// Nuxut mavisi: varsayılan, serin mavi.
    #[default]
    Blue,
    /// Nötr: sıcak taş/kum grisi, zeytin dokunuşu.
    Neutral,
    /// Adaçayı: yumuşak, griye çalan yeşil.
    Sage,
    /// Gül kurusu: tozlu pembe, sıcak gri zemin.
    Rose,
    /// Kehribar: bal sarısı vurgu, kahveye çalan koyu zemin.
    Amber,
    /// Grafit: neredeyse renksiz, serin gri.
    Graphite,
}

impl ColorThemeId {
    /// Seçicideki sıra.
    pub const ALL: [Self; 6] = [Self::Blue, Self::Neutral, Self::Sage, Self::Rose, Self::Amber, Self::Graphite];

    pub fn name(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Neutral => "neutral",
            Self::Sage => "sage",
            Self::Rose => "rose",
            Self::Amber => "amber",
            Self::Graphite => "graphite",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.name() == s.trim())
    }

    /// Temanın tanımı.
    pub fn theme(self) -> &'static ColorTheme {
        match self {
            Self::Blue => &BLUE,
            Self::Neutral => &NEUTRAL,
            Self::Sage => &SAGE,
            Self::Rose => &ROSE,
            Self::Amber => &AMBER,
            Self::Graphite => &GRAPHITE,
        }
    }
}

impl ChoiceValue for ColorThemeId {
    fn from_name(s: &str) -> Option<Self> {
        ColorThemeId::from_name(s)
    }
    fn name(self) -> &'static str {
        ColorThemeId::name(self)
    }
}

/// Bir tonal paletin rengi: hue (derece) ve HCT kroması.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hue {
    /// Tohumun hue'suna göre mi (`true`: `degrees` bir kaydırmadır) yoksa mutlak mı.
    pub relative: bool,
    pub degrees: f64,
}

/// Tonal palet tanımı.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tonal {
    pub hue: Hue,
    pub chroma: f64,
}

const fn rel(degrees: f64, chroma: f64) -> Tonal {
    Tonal { hue: Hue { relative: true, degrees }, chroma }
}

const fn abs(degrees: f64, chroma: f64) -> Tonal {
    Tonal { hue: Hue { relative: false, degrees }, chroma }
}

/// Elle ayarlanmış renk teması.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorTheme {
    pub id: ColorThemeId,
    /// Tohum (anahtar) renk: göreli hue'ların kökü; `success`/`warning` aileleri ona
    /// uyumlandırılır. Dışa aktarımda `source_color`.
    pub seed: Color,
    pub primary: Tonal,
    pub secondary: Tonal,
    pub tertiary: Tonal,
    pub neutral: Tonal,
    pub neutral_variant: Tonal,
    /// Yerleşik duvar kağıtlarının yeniden boyanmasında ek kroma çarpanı ([`Self::recolor`]):
    /// Nuxut mavisinin koyu lacivert zeminleri sıcak hue'larda aynı kromada çamurlu kahveye döner;
    /// sakin temalar zemini griye yaklaştırır. Göz kararı, ekran görüntüleriyle ayarlandı.
    pub wallpaper_chroma: f64,
    /// Koyu kipte türetilmiş token'ların yerine geçen elle seçilmiş palet. Yalnızca Nuxut mavisi
    /// (tarihî varsayılan; `Palette::default` bayt bayt korunur).
    dark_tokens: Option<Palette>,
}

const fn hex(rgb: u32) -> Color {
    // renk-matematiği: tema tanımlarının tohumları (tek kaynak bu dosya, nuxut-theme)
    Color::from_rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}

/// Nuxut mavisi: Material 3 tonal-spot (tohum `#9ec5ff`). Koyu kipte tarihî elle ayarlanmış
/// varsayılan, açık kipte tohumun tonal-spot şeması (`Palette::light`).
pub static BLUE: ColorTheme = ColorTheme {
    id: ColorThemeId::Blue,
    seed: hex(0x9ec5ff),
    primary: rel(0.0, 36.0),
    secondary: rel(0.0, 16.0),
    tertiary: rel(60.0, 24.0),
    neutral: rel(0.0, 6.0),
    neutral_variant: rel(0.0, 8.0),
    wallpaper_chroma: 1.0,
    dark_tokens: Some(Palette::DARK),
};

/// Nötr: sıcak taş grisi zemin, kum-kil vurgusu, zeytin üçüncül. Doygunluk düşük; renk
/// zeminden çok vurguda ve yalnızca ima düzeyinde.
pub static NEUTRAL: ColorTheme = ColorTheme {
    id: ColorThemeId::Neutral,
    seed: hex(0xc2a68a),
    primary: rel(0.0, 22.0),
    secondary: rel(0.0, 9.0),
    tertiary: abs(112.0, 16.0),
    neutral: abs(70.0, 4.0),
    neutral_variant: abs(70.0, 7.0),
    wallpaper_chroma: 0.55,
    dark_tokens: None,
};

/// Adaçayı: griye çalan yeşil vurgu, sisli mavi üçüncül.
pub static SAGE: ColorTheme = ColorTheme {
    id: ColorThemeId::Sage,
    seed: hex(0x9dbb98),
    primary: rel(0.0, 26.0),
    secondary: rel(0.0, 11.0),
    tertiary: rel(75.0, 18.0),
    neutral: rel(0.0, 4.0),
    neutral_variant: rel(0.0, 7.0),
    wallpaper_chroma: 0.9,
    dark_tokens: None,
};

/// Gül kurusu: tozlu pembe vurgu, altın üçüncül, sıcak gri zemin.
pub static ROSE: ColorTheme = ColorTheme {
    id: ColorThemeId::Rose,
    seed: hex(0xcf9a9f),
    primary: rel(0.0, 30.0),
    secondary: rel(0.0, 13.0),
    tertiary: rel(55.0, 20.0),
    neutral: rel(0.0, 4.0),
    neutral_variant: rel(0.0, 7.0),
    wallpaper_chroma: 0.7,
    dark_tokens: None,
};

/// Kehribar: bal sarısı vurgu, kiremit üçüncül, kahveye çalan zemin.
pub static AMBER: ColorTheme = ColorTheme {
    id: ColorThemeId::Amber,
    seed: hex(0xe2a85c),
    primary: rel(0.0, 40.0),
    secondary: rel(-5.0, 15.0),
    tertiary: rel(-40.0, 26.0),
    neutral: rel(-5.0, 5.0),
    neutral_variant: rel(-5.0, 8.0),
    wallpaper_chroma: 0.85,
    dark_tokens: None,
};

/// Grafit: neredeyse renksiz; vurgu yalnızca ton farkıyla, belli belirsiz serin.
pub static GRAPHITE: ColorTheme = ColorTheme {
    id: ColorThemeId::Graphite,
    seed: hex(0xa0a6b0),
    primary: rel(0.0, 6.0),
    secondary: rel(0.0, 3.0),
    tertiary: rel(0.0, 8.0),
    neutral: rel(0.0, 1.5),
    neutral_variant: rel(0.0, 2.5),
    wallpaper_chroma: 1.0,
    dark_tokens: None,
};

pub(crate) fn to_argb(c: Color) -> Argb {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Argb::new(255, b(c.r), b(c.g), b(c.b))
}

pub(crate) fn from_argb(c: Argb) -> Color {
    Color::from_rgba8(c.red, c.green, c.blue, 255)
}

fn wrap(deg: f64) -> f64 {
    deg.rem_euclid(360.0)
}

impl ColorTheme {
    fn seed_hue(&self) -> f64 {
        Hct::new(to_argb(self.seed)).get_hue()
    }

    fn hue_of(&self, t: Tonal) -> f64 {
        if t.hue.relative { wrap(self.seed_hue() + t.hue.degrees) } else { wrap(t.hue.degrees) }
    }

    fn tonal(&self, t: Tonal) -> TonalPalette {
        TonalPalette::of(self.hue_of(t), t.chroma)
    }

    /// Kipin Material 3 dinamik şeması (tonal-spot ton kuralları, bu temanın paletleri).
    /// `contrast` -1.0 … 1.0 (Material karşıtlık seviyesi; sonlu değilse 0). Tam rol kümesi
    /// için nuxut-palette `Palette::from_color_theme` bunu kullanır.
    pub fn dynamic_scheme(&self, dark: bool, contrast: f64) -> DynamicScheme {
        let contrast = if contrast.is_finite() { contrast.clamp(-1.0, 1.0) } else { 0.0 };
        DynamicScheme::new(
            to_argb(self.seed),
            None,
            Variant::TonalSpot,
            dark,
            Some(contrast),
            self.tonal(self.primary),
            self.tonal(self.secondary),
            self.tonal(self.tertiary),
            self.tonal(self.neutral),
            self.tonal(self.neutral_variant),
            None,
        )
    }

    /// Kipin 11 tema token'ı (karşıtlık kipi uygulanmamış; bkz. [`Palette::high_contrast`]).
    /// Eşleme nuxut-palette `THEME_ROLES` ile aynı: `surface_bright` ← `surface_container_high`,
    /// `outline` ← `outline_variant`.
    pub fn palette(&self, mode: ThemeMode) -> Palette {
        if mode == ThemeMode::Dark
            && let Some(p) = self.dark_tokens
        {
            return p;
        }
        let s = self.dynamic_scheme(mode == ThemeMode::Dark, 0.0);
        Palette {
            surface: from_argb(s.surface()),
            surface_container: from_argb(s.surface_container()),
            surface_bright: from_argb(s.surface_container_high()),
            on_surface: from_argb(s.on_surface()),
            on_surface_variant: from_argb(s.on_surface_variant()),
            primary: from_argb(s.primary()),
            on_primary: from_argb(s.on_primary()),
            error: from_argb(s.error()),
            on_error: from_argb(s.on_error()),
            outline: from_argb(s.outline_variant()),
            shadow: from_argb(s.shadow()),
        }
    }

    /// `reference` temasıyla çizilmiş bir rengi bu temaya taşır (yerleşik duvar kağıtları,
    /// [`BLUE`] ile tasarlanmış görseller). Ton (algısal parlaklık) **aynen korunur**: görselin
    /// ışık düzeni ve üstündeki arayüzle karşıtlığı değişmez. Hue ve kroma, rengin ne kadar
    /// "vurgu" olduğuna göre iki eşleme arasında yumuşakça karışır:
    /// - nötr eşleme (kroma ≤ kaynak nötr-varyant kroması): hue nötr paletlerin farkı kadar
    ///   döner, kroma nötr kroma oranı × [`Self::wallpaper_chroma`] ile ölçeklenir (koyu zemin
    ///   gradyanları);
    /// - vurgu eşlemesi (kroma ≥ kaynak birincil kroması): hue birincil paletlerin farkı kadar
    ///   döner (vurgular arası hue aralığı korunur: auroranın mor-camgöbeği yelpazesi yeni
    ///   temada da yelpaze kalır), kroma birincil kroma oranıyla ölçeklenir.
    ///
    /// `self == reference` iken renk dokunulmadan döner. Saydamlık korunur.
    pub fn recolor(&self, reference: &ColorTheme, c: Color) -> Color {
        if self.id == reference.id {
            return c;
        }
        let hct = Hct::new(to_argb(c));
        let (h, ch, tone) = (hct.get_hue(), hct.get_chroma(), hct.get_tone());
        let lo = reference.neutral_variant.chroma;
        let hi = reference.primary.chroma.max(lo + 1.0);
        let x = ((ch - lo) / (hi - lo)).clamp(0.0, 1.0);
        let w = x * x * (3.0 - 2.0 * x);
        let shift = |t: fn(&ColorTheme) -> Tonal| {
            let d = self.hue_of(t(self)) - reference.hue_of(t(reference));
            // En kısa yay.
            (d + 540.0).rem_euclid(360.0) - 180.0
        };
        let ratio = |t: fn(&ColorTheme) -> Tonal| t(self).chroma / t(reference).chroma.max(0.5);
        let (dn, da) = (shift(|t| t.neutral), shift(|t| t.primary));
        let hue = wrap(h + dn + (da - dn) * w);
        let k = ratio(|t| t.neutral) + (ratio(|t| t.primary) - ratio(|t| t.neutral)) * w;
        // Sakinleştirme yalnızca zemin tarafında: vurgular (w → 1) kendi kromasını korur.
        let k = k * (self.wallpaper_chroma + (1.0 - self.wallpaper_chroma) * w);
        let out = from_argb(Hct::from(hue, ch * k, tone).into());
        Color { a: c.a, ..out }
    }
}

/// SVG metnindeki renk özniteliklerinin (`fill`, `stroke`, `stop-color`, `flood-color`,
/// `lighting-color`, `color`) `#rrggbb` / `#rgb` değerlerini `map`'ten geçirir. Yalnızca bu
/// özniteliklerin tırnaklı değerleri değişir (`href="#abc"` gibi kimlik başvuruları ve `url(#…)`
/// dokunulmaz); başka biçimler (`rgb()`, adlı renkler, `style` içi) olduğu gibi kalır. Güvenilir
/// gömülü kaynaklar içindir (yerleşik duvar kağıtları); ayrıştırıcı değildir.
pub fn recolor_svg(svg: &[u8], map: impl Fn(Color) -> Color) -> Vec<u8> {
    const ATTRS: [&[u8]; 6] = [b"fill", b"stroke", b"stop-color", b"flood-color", b"lighting-color", b"color"];
    let mut out = Vec::with_capacity(svg.len());
    let mut i = 0;
    while i < svg.len() {
        // `="#` → önceki sözcük bir renk özniteliği mi?
        if svg[i] == b'=' && matches!(svg.get(i + 1), Some(b'"' | b'\'')) && svg.get(i + 2) == Some(&b'#') {
            let quote = svg[i + 1];
            let name_start = svg[..i].iter().rposition(|b| !(b.is_ascii_alphanumeric() || *b == b'-')).map_or(0, |p| p + 1);
            let name = &svg[name_start..i];
            let digits = svg[i + 3..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
            let closed = svg.get(i + 3 + digits) == Some(&quote);
            if ATTRS.contains(&name) && closed && (digits == 6 || digits == 3) {
                let text = std::str::from_utf8(&svg[i + 2..i + 3 + digits]).unwrap_or_default();
                if let Ok(c) = Color::from_hex(text) {
                    out.extend_from_slice(&svg[i..i + 2]);
                    out.extend_from_slice(map(c).to_hex().as_bytes());
                    i += 3 + digits;
                    continue;
                }
            }
        }
        out.push(svg[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contrast::{NON_TEXT_MIN, TEXT_MIN, contrast_ratio};

    fn hct(c: Color) -> Hct {
        Hct::new(to_argb(c))
    }

    #[test]
    fn names_roundtrip_and_unique() {
        for id in ColorThemeId::ALL {
            assert_eq!(ColorThemeId::from_name(id.name()), Some(id));
            assert_eq!(id.theme().id, id);
        }
        let mut names: Vec<_> = ColorThemeId::ALL.iter().map(|t| t.name()).collect();
        names.dedup();
        assert_eq!(names.len(), ColorThemeId::ALL.len());
        assert_eq!(ColorThemeId::from_name("mor"), None);
        assert_eq!(ColorThemeId::default(), ColorThemeId::Blue);
    }

    #[test]
    fn blue_keeps_historic_static_palettes() {
        assert_eq!(BLUE.palette(ThemeMode::Dark), Palette::default());
        assert_eq!(BLUE.palette(ThemeMode::Light), Palette::light());
    }

    /// Kontrast denetimi (docs/design/color-themes.md §Kontrast): her tema, her kip, iki
    /// karşıtlık seviyesi. Metin ≥ 4.5:1, grafik (vurgu, hata) ≥ 3:1; yüksek karşıtlıkta
    /// ayrıca vurgu ≥ 4.5 ve kenar ≥ 3.
    #[test]
    fn every_theme_meets_contrast_in_every_mode() {
        for id in ColorThemeId::ALL {
            for mode in ThemeMode::ALL {
                for high in [false, true] {
                    let base = id.theme().palette(mode);
                    let p = if high { base.high_contrast() } else { base };
                    let tag = format!("{} {} {}", id.name(), mode.name(), if high { "yüksek" } else { "standart" });
                    assert_eq!(ThemeMode::of_surface(p.surface), mode, "{tag}: yüzey kipi");
                    for bg in [p.surface, p.surface_container, p.surface_bright] {
                        for (what, fg, min) in [
                            ("on_surface", p.on_surface, TEXT_MIN),
                            ("on_surface_variant", p.on_surface_variant, TEXT_MIN),
                            ("primary", p.primary, if high { TEXT_MIN } else { NON_TEXT_MIN }),
                            ("error", p.error, if high { TEXT_MIN } else { NON_TEXT_MIN }),
                        ] {
                            let r = contrast_ratio(fg, bg);
                            assert!(r >= min, "{tag}: {what} {:.2} < {min}", r);
                        }
                        if high {
                            assert!(contrast_ratio(p.outline, bg) >= NON_TEXT_MIN, "{tag}: outline");
                        }
                    }
                    assert!(contrast_ratio(p.on_primary, p.primary) >= TEXT_MIN, "{tag}: on_primary");
                    assert!(contrast_ratio(p.on_error, p.error) >= TEXT_MIN, "{tag}: on_error");
                    // Ton hiyerarşisi: kap yüzeyleri ayrışır (tonal ayrım, kenar çizgisi yok).
                    let lum = crate::contrast::relative_luminance;
                    let order = [lum(p.surface), lum(p.surface_container), lum(p.surface_bright)];
                    if mode == ThemeMode::Dark {
                        assert!(order[0] < order[1] && order[1] < order[2], "{tag}: koyu yüzey sırası");
                    } else {
                        assert!(order[0] > order[1] && order[1] > order[2], "{tag}: açık yüzey sırası");
                    }
                }
            }
        }
    }

    #[test]
    fn themes_are_distinct() {
        // Her tema çifti birincil renkte görünür biçimde ayrışır (hue ya da kroma).
        for (i, a) in ColorThemeId::ALL.iter().enumerate() {
            for b in &ColorThemeId::ALL[i + 1..] {
                let (pa, pb) = (hct(a.theme().palette(ThemeMode::Dark).primary), hct(b.theme().palette(ThemeMode::Dark).primary));
                let dh = ((pa.get_hue() - pb.get_hue() + 540.0).rem_euclid(360.0) - 180.0).abs();
                let dc = (pa.get_chroma() - pb.get_chroma()).abs();
                assert!(dh > 20.0 || dc > 12.0, "{} ~ {}", a.name(), b.name());
            }
        }
    }

    #[test]
    fn recolor_keeps_tone_and_alpha() {
        assert_eq!(BLUE.recolor(&BLUE, hex(0x4a7bd8)), hex(0x4a7bd8));
        for id in ColorThemeId::ALL {
            for c in [hex(0x0f121a), hex(0x16213a), hex(0x9ec5ff), hex(0x4a7bd8), hex(0x7f8df0), hex(0xf5f7fc)] {
                let out = id.theme().recolor(&BLUE, c.multiply_alpha(0.5));
                assert!((out.a - 0.5).abs() < 1e-6);
                let d = (hct(out).get_tone() - hct(c).get_tone()).abs();
                assert!(d < 1.0, "{}: ton {d}", id.name());
            }
        }
        // Vurgu yeni temanın birincil hue'suna gider; grafit renksizleşir.
        let accent = ROSE.recolor(&BLUE, hex(0x9ec5ff));
        let want = hct(ROSE.palette(ThemeMode::Dark).primary).get_hue();
        let dh = ((hct(accent).get_hue() - want + 540.0).rem_euclid(360.0) - 180.0).abs();
        assert!(dh < 8.0, "gül vurgu hue farkı {dh}");
        assert!(hct(GRAPHITE.recolor(&BLUE, hex(0x4a7bd8))).get_chroma() < 12.0);
    }

    #[test]
    fn svg_recolor_touches_only_color_attributes() {
        let svg = br##"<svg><linearGradient id="abc"><stop stop-color="#9ec5ff" stop-opacity=".2"/></linearGradient><use href="#abc" fill="#fff"/><rect fill="url(#abc)" stroke='#123456' data-x="#9ec5ff"/></svg>"##;
        let out = recolor_svg(svg, |_| hex(0x102030));
        let out = String::from_utf8(out).unwrap();
        assert_eq!(
            out,
            r##"<svg><linearGradient id="abc"><stop stop-color="#102030" stop-opacity=".2"/></linearGradient><use href="#abc" fill="#102030"/><rect fill="url(#abc)" stroke='#102030' data-x="#9ec5ff"/></svg>"##
        );
        // Kimlik eşlemesi bayt bayt aynı (küçük harf altılı yazımda).
        let same = br##"<path fill="#0f121a"/>"##;
        assert_eq!(recolor_svg(same, |c| c), same);
    }
}
