//! Karşıtlık kipi (`[colors] contrast`) ve WCAG karşıtlık düzeltmesi (docs/design/accessibility.md §1).
//!
//! Yüksek karşıtlık kipi renkleri **yeniden seçmez**: her ön plan rengini HCT uzayında yalnızca
//! **tonuyla** (L*) zeminlerinden uzaklaştırır; hue ve kroma korunur (gamut dışına taşarsa
//! kroma MCU çözücüsüyle azalır). Böylece dinamik rengin kimliği kalır, tonlar uçlara itilir.
//! Eşiği zaten geçen renk bayt bayt aynı kalır (dönüşüm idempotenttir; standart çıktıya
//! dokunulmaz).

use material_colors::color::Argb;
use material_colors::hct::Hct;
use nuxut_core::Color;

use crate::palette::ChoiceValue;

/// WCAG 2.x AA, normal metin (1.4.3): metin ve metin üstü zemin çiftleri.
pub const TEXT_MIN: f32 = 4.5;
/// WCAG 2.x AA, metin dışı (1.4.11): ikon, kenar, odak halkası, bileşen sınırı.
pub const NON_TEXT_MIN: f32 = 3.0;
/// Yüksek karşıtlık kipinde birincil/ikincil metnin ve `on_*` rollerinin hedefi (WCAG AAA
/// 1.4.6). Uç tonda bile ulaşılamazsa [`TEXT_MIN`] yine garanti edilir.
pub const TEXT_TARGET: f32 = 7.0;

/// `framework.toml [colors] contrast`: `standard` (varsayılan) | `high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContrastMode {
    /// Türetilen palet olduğu gibi (DESIGN.md ≥ 3:1 kuralı).
    #[default]
    Standard,
    /// Metin ≥ 4.5:1 (hedef 7:1), metin dışı UI ≥ 3:1; kenarlar en az `subtle` ve tam opak.
    High,
}

impl ContrastMode {
    pub const ALL: [Self; 2] = [Self::Standard, Self::High];

    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::High => "high",
        }
    }

    pub fn is_high(self) -> bool {
        self == Self::High
    }
}

impl ChoiceValue for ContrastMode {
    fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
    fn name(self) -> &'static str {
        ContrastMode::name(self)
    }
}

/// WCAG göreli parlaklık (renk opak kabul edilir).
pub fn relative_luminance(c: Color) -> f32 {
    let lin = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG karşıtlık oranı (1..=21); renkler opak kabul edilir.
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (x, y) = (relative_luminance(a), relative_luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `fg`'nin `bgs` içindeki en kötü zemine oranı (boş liste: sonsuz).
pub fn worst_ratio(fg: Color, bgs: &[Color]) -> f32 {
    bgs.iter().map(|b| contrast_ratio(fg, *b)).fold(f32::INFINITY, f32::min)
}

fn to_argb(c: Color) -> Argb {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Argb::new(255, b(c.r), b(c.g), b(c.b))
}

/// `fg`'yi `bgs`'nin hepsine karşı en az `target` orana getirir: HCT tonu zeminlerden uzağa
/// (fg zeminlerin ortalamasından açıksa yukarı, değilse aşağı) **en az** gereken kadar itilir,
/// hue/kroma korunur, alfa korunur. Zaten `target`'ı geçen `fg` aynen döner. O yönde uç tonda
/// (siyah/beyaz) bile ulaşılamazsa uç döner (çağıran gerekirse zemini de iter, bkz.
/// [`ensure_pair`]).
pub fn ensure_contrast(fg: Color, bgs: &[Color], target: f32) -> Color {
    if worst_ratio(fg, bgs) >= target {
        return fg;
    }
    let hct = Hct::new(to_argb(fg));
    let at = |tone: f64| {
        let mut h = hct;
        h.set_tone(tone);
        let a: Argb = h.into();
        Color::from_rgba8(a.red, a.green, a.blue, 255).multiply_alpha(fg.a)
    };
    let t0 = hct.get_tone();
    let mean_bg = bgs.iter().map(|b| relative_luminance(*b)).sum::<f32>() / bgs.len().max(1) as f32;
    let extreme = if relative_luminance(fg) >= mean_bg { 100.0 } else { 0.0 };
    let end = at(extreme);
    if worst_ratio(end, bgs) < target {
        // Uçta da yetmiyor: en iyisi uç; ters yöne geçip rengi zeminin öbür yanına atlatmak
        // kimliği bozar (çiftlerde zemini ensure_pair iter).
        return if worst_ratio(end, bgs) > worst_ratio(fg, bgs) { end } else { fg };
    }
    // lo hedefi geçmez, hi geçer: değişmez. Ölçüm nicelenmiş (8 bit) renkle yapılır.
    let (mut lo, mut hi) = (t0, extreme);
    let mut hi_color = end;
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        let c = at(mid);
        if worst_ratio(c, bgs) >= target {
            hi = mid;
            hi_color = c;
        } else {
            lo = mid;
        }
    }
    hi_color
}

/// `fg`/`bg` çifti: önce `fg` `target`'a itilir; sonuç `min`'in altındaysa (zemin orta tonda)
/// `bg` de `fg`'den uzağa `min`'e itilir. Dönen değer (fg, bg).
pub fn ensure_pair(fg: Color, bg: Color, target: f32, min: f32) -> (Color, Color) {
    let fg = ensure_contrast(fg, &[bg], target);
    if contrast_ratio(fg, bg) >= min {
        return (fg, bg);
    }
    let bg = ensure_contrast(bg, &[fg], min);
    (ensure_contrast(fg, &[bg], target), bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Color {
        Color::from_hex(s).unwrap()
    }

    #[test]
    fn ratio_matches_wcag_reference() {
        assert!((contrast_ratio(Color::WHITE, c("#000000")) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(c("#777777"), Color::WHITE) - 4.48).abs() < 0.01);
        assert_eq!(worst_ratio(Color::WHITE, &[]), f32::INFINITY);
    }

    #[test]
    fn passing_colors_are_untouched() {
        let fg = c("#e4e6ee");
        assert_eq!(ensure_contrast(fg, &[c("#12141a")], TEXT_MIN), fg);
    }

    #[test]
    fn pushes_tone_keeps_hue_and_alpha() {
        let fg = c("#5a6aa0").multiply_alpha(0.5);
        let bg = c("#12141a");
        let out = ensure_contrast(fg, &[bg], TEXT_TARGET);
        assert!(contrast_ratio(out, bg) >= TEXT_TARGET);
        assert!((out.a - 0.5).abs() < 1e-6);
        let (h0, h1) = (Hct::new(to_argb(fg)), Hct::new(to_argb(out)));
        assert!(h1.get_tone() > h0.get_tone());
        let d = (h0.get_hue() - h1.get_hue()).rem_euclid(360.0);
        assert!(d.min(360.0 - d) < 4.0, "hue kaydı {d}");
        // En az değişim: hedefin çok üstüne çıkılmaz.
        assert!(contrast_ratio(out, bg) < TEXT_TARGET + 0.3);
    }

    #[test]
    fn mid_tone_background_is_pushed_in_pairs() {
        let (fg, bg) = ensure_pair(c("#ffffff"), c("#808080"), TEXT_TARGET, TEXT_MIN);
        assert!(contrast_ratio(fg, bg) >= TEXT_MIN, "{fg:?} {bg:?}");
    }

    #[test]
    fn choice_names() {
        for m in ContrastMode::ALL {
            assert_eq!(<ContrastMode as ChoiceValue>::from_name(m.name()), Some(m));
        }
        assert_eq!(ContrastMode::default(), ContrastMode::Standard);
    }
}
