//! Sayı biçimi (glibc `LC_NUMERIC`: ondalık ayırıcı, gruplama ayırıcısı ve grup boyları).
//!
//! Tek uygulama: `nuxut-i18n`'in Fluent `NUMBER` biçimleyicisi de buradaki
//! [`NumberSymbols::format`]'ı çağırır. Hiç panik etmez; NaN/sonsuz/çok büyük değerler ve
//! aşırı seçenekler sınırlı çıktıyla biçimlenir.

/// Bir locale'in sayı simgeleri (glibc verisi, `'static`; `Copy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberSymbols {
    /// Ondalık ayırıcı (`decimal_point`).
    pub decimal: &'static str,
    /// Gruplama ayırıcısı (`thousands_sep`); boş = gruplama yok.
    pub group: &'static str,
    /// Sağdan sola grup boyları (`grouping`); son eleman tekrarlanır, `≤ 0` / `≥ 127` ya da boş
    /// = sonrası gruplanmaz.
    pub grouping: &'static [i64],
    /// Yüzde işaretinin yeri ve boşluğu (CLDR; glibc verisinde yok).
    pub percent: PercentPattern,
}

impl NumberSymbols {
    /// Locale'siz düz biçim: `.` ondalık, gruplama yok.
    pub const PLAIN: Self = Self { decimal: ".", group: "", grouping: &[], percent: PercentPattern::SUFFIX };

    /// `value`'yu biçimler. NaN → `NaN`, sonsuz → `∞` / `-∞`; kesir en çok
    /// [`MAX_FRACTION_DIGITS`], tamsayı dolgusu en çok [`MAX_INTEGER_DIGITS`].
    pub fn format(&self, value: f64, options: &NumberOptions) -> String {
        let mut value = value;
        if options.percent {
            value *= 100.0;
            // × 100 kayan nokta artığı bırakır (0.29 → 28.999999999999996). En kısa gösterimde
            // (kesir sınırı yok) 15 anlamlı basamağa yuvarlanır: f64'ün kesin taşıdığı basamak
            // sayısı; artık görünmez, gerçek kesirler (0.1234 → 12.34) korunur.
            if options.max_fraction_digits.is_none() && value.is_finite() {
                value = format!("{value:.14e}").parse().unwrap_or(value);
            }
        }
        if value.is_nan() {
            return "NaN".into();
        }
        if value.is_infinite() {
            return if value < 0.0 { "-∞".into() } else { "∞".into() };
        }
        let min = options.min_fraction_digits.min(MAX_FRACTION_DIGITS);
        let digits = match options.max_fraction_digits {
            Some(max) => format!("{:.*}", max.clamp(min, MAX_FRACTION_DIGITS), value.abs()),
            // `{}` en kısa gidiş-dönüş gösterimi; üs gösterimi kullanmaz (1e300 → 301 basamak).
            None => format!("{}", value.abs()),
        };
        let (int, frac) = digits.split_once('.').unwrap_or((&digits, ""));
        // Sondaki sıfırlar en aza kadar atılır, sonra en aza dek doldurulur.
        let mut frac = frac.trim_end_matches('0').to_owned();
        while frac.len() < min {
            frac.push('0');
        }
        let pad = options.min_integer_digits.min(MAX_INTEGER_DIGITS);
        let int = format!("{int:0>pad$}");
        let int = if options.grouping { self.group(&int) } else { int };
        // Yuvarlanınca sıfır kalan negatif değer işaretsiz (`-0` yazılmaz).
        let negative =
            value < 0.0 && (int.bytes().any(|b| b.is_ascii_digit() && b != b'0') || frac.bytes().any(|b| b != b'0'));
        let mut out = String::with_capacity(int.len() + frac.len() + 8);
        if negative {
            out.push('-');
        }
        let percent = options.percent.then_some(self.percent);
        if let Some(p) = percent.filter(|p| p.prefix) {
            out.push('%');
            out.push_str(p.gap);
        }
        out.push_str(&int);
        if !frac.is_empty() {
            out.push_str(self.decimal);
            out.push_str(&frac);
        }
        if let Some(p) = percent.filter(|p| !p.prefix) {
            out.push_str(p.gap);
            out.push('%');
        }
        out
    }

    /// Tamsayı basamaklarını gruplar (glibc `grouping`: sağdan sola, son eleman tekrarlanır;
    /// `≤ 0` ya da `≥ 127` (CHAR_MAX) = sonrası gruplanmaz).
    fn group(&self, int: &str) -> String {
        if self.group.is_empty() || self.grouping.is_empty() {
            return int.to_owned();
        }
        // Basamaklar ASCII: bayt dilimleme güvenli.
        let mut parts: Vec<&str> = Vec::new();
        let mut end = int.len();
        let mut i = 0usize;
        while end > 0 {
            let size = self.grouping.get(i).or(self.grouping.last()).copied().unwrap_or(-1);
            let Ok(size) = usize::try_from(size) else {
                parts.push(&int[..end]);
                break;
            };
            if size == 0 || size >= 127 || size >= end {
                parts.push(&int[..end]);
                break;
            }
            parts.push(&int[end - size..end]);
            end -= size;
            i += 1;
        }
        parts.reverse();
        parts.join(self.group)
    }
}

/// Yüzde kalıbı: `%` sayının önünde mi arkasında mı, arada hangi boşluk var (CLDR
/// `percentFormats`: tr `%50`, en `50%`, de `50 %` (U+00A0), fr `50 %` (U+202F), eu `% 50`).
/// Eksi işareti her zaman en başta: tr `-%5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PercentPattern {
    /// `%` sayının önünde.
    pub prefix: bool,
    /// `%` ile sayı arasındaki boşluk (`""`, U+00A0 ya da U+202F).
    pub gap: &'static str,
}

impl PercentPattern {
    /// `50%` (CLDR kök kalıbı; tabloda olmayan her dil).
    pub const SUFFIX: Self = Self { prefix: false, gap: "" };

    /// Dil (ve bölge) için kalıp. Tablo CLDR'den elle alınmış küçük bir alt kümedir; bilinmeyen
    /// dil → [`PercentPattern::SUFFIX`].
    pub fn for_locale(language: &str, region: &str) -> Self {
        const NBSP: &str = "\u{a0}";
        const NNBSP: &str = "\u{202f}";
        let suffix_gap = |gap| Self { prefix: false, gap };
        match (language, region) {
            // İsviçre'de Almanca ve Fransızca boşluksuz.
            ("de" | "fr", "CH") => Self::SUFFIX,
            ("tr", _) => Self { prefix: true, gap: "" },
            ("eu", _) => Self { prefix: true, gap: NBSP },
            ("fr", _) => suffix_gap(NNBSP),
            (
                "de" | "es" | "ca" | "gl" | "sv" | "nb" | "nn" | "no" | "da" | "fi" | "cs" | "sk" | "ru" | "ro"
                | "lt" | "sl" | "hr" | "be",
                _,
            ) => suffix_gap(NBSP),
            _ => Self::SUFFIX,
        }
    }

    /// Önceden biçimlenmiş sayı metnine `%` ekler (ör. kaydırıcı etiketinde [`NumberSymbols::format`]
    /// çıktısı ya da tamsayı): tr `"50"` → `"%50"`, `"-5"` → `"-%5"`; en `"50"` → `"50%"`.
    pub fn apply(self, number: &str) -> String {
        let mut out = String::with_capacity(number.len() + 4);
        if self.prefix {
            let (sign, rest) = match number.strip_prefix('-') {
                Some(rest) => ("-", rest),
                None => ("", number),
            };
            out.push_str(sign);
            out.push('%');
            out.push_str(self.gap);
            out.push_str(rest);
        } else {
            out.push_str(number);
            out.push_str(self.gap);
            out.push('%');
        }
        out
    }
}

impl Default for PercentPattern {
    fn default() -> Self {
        Self::SUFFIX
    }
}

impl Default for NumberSymbols {
    fn default() -> Self {
        Self::PLAIN
    }
}

/// En çok kesir basamağı (aşırı seçenek bellek/işlem patlatmasın).
pub const MAX_FRACTION_DIGITS: usize = 20;
/// En çok tamsayı dolgusu.
pub const MAX_INTEGER_DIGITS: usize = 64;

/// Sayı biçimi seçenekleri (ECMA-402 `Intl.NumberFormat` alt kümesi; Fluent `NUMBER`).
/// `#[non_exhaustive]`: `NumberOptions::default()` ile kurup alanları atayın.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct NumberOptions {
    /// En az kesir basamağı (sıfırla doldurulur).
    pub min_fraction_digits: usize,
    /// En çok kesir basamağı (yuvarlanır); `None` = değerin en kısa gösterimi.
    pub max_fraction_digits: Option<usize>,
    /// En az tamsayı basamağı (soldan sıfırla doldurulur).
    pub min_integer_digits: usize,
    /// Basamak gruplama.
    pub grouping: bool,
    /// Yüzde: değer × 100 ve locale kalıbıyla `%` ([`PercentPattern`]: tr `%50`, en `50%`).
    pub percent: bool,
}

impl Default for NumberOptions {
    fn default() -> Self {
        Self { min_fraction_digits: 0, max_fraction_digits: None, min_integer_digits: 1, grouping: true, percent: false }
    }
}

impl NumberOptions {
    /// Tam `digits` kesir basamağı (yuvarlanır, sıfırla doldurulur), gruplamalı.
    pub fn fixed(digits: usize) -> Self {
        Self { min_fraction_digits: digits, max_fraction_digits: Some(digits), ..Self::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TR: NumberSymbols =
        NumberSymbols { decimal: ",", group: ".", grouping: &[3, 3], percent: PercentPattern { prefix: true, gap: "" } };
    const EN: NumberSymbols = NumberSymbols { decimal: ".", group: ",", grouping: &[3, 3], percent: PercentPattern::SUFFIX };

    fn opts(f: impl FnOnce(&mut NumberOptions)) -> NumberOptions {
        let mut o = NumberOptions::default();
        f(&mut o);
        o
    }

    #[test]
    fn separators_and_options() {
        let d = NumberOptions::default();
        assert_eq!(TR.format(1234567.5, &d), "1.234.567,5");
        assert_eq!(EN.format(1234567.5, &d), "1,234,567.5");
        assert_eq!(TR.format(-0.25, &d), "-0,25");
        assert_eq!(TR.format(12.0, &opts(|o| o.min_fraction_digits = 2)), "12,00");
        assert_eq!(EN.format(4.56789, &opts(|o| o.max_fraction_digits = Some(2))), "4.57");
        assert_eq!(EN.format(1234.0, &opts(|o| o.grouping = false)), "1234");
        assert_eq!(EN.format(0.5, &opts(|o| o.percent = true)), "50%");
        assert_eq!(TR.format(0.5, &opts(|o| o.percent = true)), "%50");
        let o = opts(|o| {
            o.percent = true;
            o.max_fraction_digits = Some(1);
        });
        assert_eq!(TR.format(-0.255, &o), "-%25,5");
        // Yüzde çarpımı artığı en kısa gösterimde görünmez; gerçek kesir korunur.
        let pct = opts(|o| o.percent = true);
        assert_eq!(TR.format(0.29, &pct), "%29");
        assert_eq!(EN.format(0.07, &pct), "7%");
        assert_eq!(EN.format(0.1234, &pct), "12.34%");
        assert_eq!(EN.format(-0.57, &pct), "-57%");
        assert_eq!(EN.format(f64::MAX, &pct), "∞");
        // Kesir sınırı verilince yuvarlama zaten sınırla yapılır.
        let wide = EN.format(0.29, &opts(|o| {
            o.percent = true;
            o.max_fraction_digits = Some(20);
        }));
        assert!(wide.starts_with("28.99999"), "{wide}");
        assert_eq!(EN.format(-0.0001, &opts(|o| o.max_fraction_digits = Some(2))), "0");
        assert_eq!(EN.format(7.0, &opts(|o| o.min_integer_digits = 3)), "007");
        assert_eq!(EN.format(2.5, &NumberOptions::fixed(2)), "2.50");
    }

    #[test]
    fn hostile_values_are_bounded() {
        let d = NumberOptions::default();
        assert_eq!(EN.format(f64::NAN, &d), "NaN");
        assert_eq!(EN.format(f64::INFINITY, &d), "∞");
        assert_eq!(EN.format(f64::NEG_INFINITY, &d), "-∞");
        let huge = EN.format(1e300, &NumberOptions::fixed(2));
        assert!(huge.starts_with("1,000,000") && huge.ends_with(".00"), "{huge}");
        assert!(huge.len() < 500);
        assert_eq!(EN.format(-f64::MAX, &d).chars().next(), Some('-'));
        assert_eq!(EN.format(f64::MIN_POSITIVE, &opts(|o| o.max_fraction_digits = Some(3))), "0");
        let big = EN.format(
            1.0,
            &opts(|o| {
                o.min_fraction_digits = usize::MAX;
                o.min_integer_digits = usize::MAX;
                o.max_fraction_digits = Some(usize::MAX);
            }),
        );
        assert!(big.len() < 200, "{}", big.len());
    }

    #[test]
    fn indian_and_unusual_grouping() {
        let s = |grouping| NumberSymbols { decimal: ".", group: ",", grouping, percent: PercentPattern::SUFFIX };
        assert_eq!(s(&[3, 2]).group("1234567"), "12,34,567");
        assert_eq!(s(&[3, 2]).group("123"), "123");
        assert_eq!(s(&[3, -1]).group("1234567"), "1234,567");
        assert_eq!(s(&[3, 127]).group("1234567"), "1234,567");
        assert_eq!(s(&[0]).group("1234567"), "1234567");
        assert_eq!(s(&[]).group("1234567"), "1234567");
        assert_eq!(NumberSymbols::PLAIN.format(1234567.0, &NumberOptions::default()), "1234567");
    }
}
