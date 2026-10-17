//! Locale: ortamdan seçim, yerelleştirilmiş tarih/saat biçimleme ve sayı ayırıcıları.
//!
//! - **Seçim (POSIX):** zaman için `LC_ALL` > `LC_TIME` > `LANG`, sayı için `LC_ALL` >
//!   `LC_NUMERIC` > `LANG`; ilk boş olmayan değişken karar verir (sonrakilere bakılmaz).
//!   `C`/`POSIX`, tanınmayan ya da geçersiz değer → `en_US` yedeği.
//! - **Veri:** ad tabloları, `%x`/`%X`/`%c` kalıpları ve `LC_NUMERIC` (ondalık/gruplama) glibc
//!   locale verisinden (`pure-rust-locales`, chrono `unstable-locales` de bunu kullanır); sistem
//!   locale'inin kurulu olması gerekmez.
//! - **Biçimleme hiç panik etmez:** kalıp önce ayrıştırılır (geçersiz → [`FormatError`]), sonra
//!   `write!` ile yazılır (chrono'nun `to_string` yolu hatada panik eder). Kalıp kullanıcı
//!   config'inden gelebilir; uzunluğu [`MAX_PATTERN_LEN`] ile sınırlı.

//! - **Ek API (chrono türü istemez):** [`available`] (gömülü locale listesi),
//!   [`Locale::language_name`] / [`Locale::display_name`], [`Locale::format_number`],
//!   [`Locale::format_ymd`] / [`Locale::format_month_year`] / [`Locale::format_hm`] /
//!   [`Locale::format_system_time`] ([`DateStyle`], [`DateTimeStyle`]).

#![forbid(unsafe_code)]

mod date;
mod names;
mod number;

pub use date::{DateStyle, DateTimeStyle};
pub use names::available;
pub use number::{MAX_FRACTION_DIGITS, MAX_INTEGER_DIGITS, NumberOptions, NumberSymbols, PercentPattern};

use std::fmt::{self, Write as _};

use chrono::format::{DelayedFormat, Item, StrftimeItems};
use chrono::{DateTime, NaiveDate, NaiveTime, TimeZone};
pub use chrono::Weekday;

/// Biçim kalıbının en fazla uzunluğu (bayt).
pub const MAX_PATTERN_LEN: usize = 256;
/// Ortam değişkeninden okunan locale adının en fazla uzunluğu.
const MAX_NAME_LEN: usize = 64;

/// Biçimleme hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// Kalıp geçersiz ya da çok uzun.
    Pattern,
    /// Değer kalıbı karşılamıyor (ör. saat dilimi olmayan değerde `%Z`).
    Value,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pattern => "geçersiz tarih/saat kalıbı",
            Self::Value => "değer kalıba uymuyor",
        })
    }
}

impl std::error::Error for FormatError {}

/// Ad uzunluğu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Length {
    /// "Oca", "Pzt"
    Short,
    /// "Ocak", "Pazartesi"
    Long,
}

/// Zaman locale'i. `Clone` ucuz değildir (ad dizgesi); bir kez kurulup paylaşılır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    inner: chrono::Locale,
    /// Kanonik ad: `tr_TR`, `sr_RS@latin` (kod kümesi atılmış).
    name: String,
    /// 12/24 saat geçersiz kılması (`framework.toml [locale] hour_cycle`); `None` = locale verisi.
    twelve_hour: Option<bool>,
}

impl Default for Locale {
    fn default() -> Self {
        Self::en_us()
    }
}

impl Locale {
    /// Yedek: `en_US`.
    pub fn en_us() -> Self {
        Self { inner: chrono::Locale::en_US, name: "en_US".to_owned(), twelve_hour: None }
    }

    /// Ortamdan (`LC_ALL` > `LC_TIME` > `LANG`); bulunamazsa `en_US`.
    pub fn from_env() -> Self {
        let var = |k| std::env::var(k).ok();
        Self::from_vars(var("LC_ALL").as_deref(), var("LC_TIME").as_deref(), var("LANG").as_deref())
    }

    /// Sayı locale'i ortamdan (`LC_ALL` > `LC_NUMERIC` > `LANG`); bulunamazsa `en_US`.
    /// Ayırıcılar için: [`decimal_separator`](Self::decimal_separator).
    pub fn numeric_from_env() -> Self {
        let var = |k| std::env::var(k).ok();
        Self::from_vars(var("LC_ALL").as_deref(), var("LC_NUMERIC").as_deref(), var("LANG").as_deref())
    }

    /// [`from_env`](Self::from_env) / [`numeric_from_env`](Self::numeric_from_env)'in saf hâli
    /// (testler; ortadaki değer kategori değişkeni): ilk boş olmayan değer karar verir.
    pub fn from_vars(lc_all: Option<&str>, lc_category: Option<&str>, lang: Option<&str>) -> Self {
        [lc_all, lc_category, lang]
            .into_iter()
            .flatten()
            .find(|v| !v.is_empty())
            .and_then(Self::parse)
            .unwrap_or_else(Self::en_us)
    }

    /// `tr_TR.UTF-8`, `de_DE@euro`, `sr_RS.UTF-8@latin`, `tr` gibi bir değeri çözer. `C`,
    /// `POSIX`, geçersiz karakterli ya da veride olmayan ad → `None`.
    pub fn parse(value: &str) -> Option<Self> {
        if value.is_empty()
            || value.len() > MAX_NAME_LEN
            || !value.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'@' | b'-'))
        {
            return None;
        }
        let (base, modifier) = match value.split_once('@') {
            Some((b, m)) => (b, Some(m)),
            None => (value, None),
        };
        // Kod kümesi (`.UTF-8`) atılır; ad tabloları UTF-8'dir.
        let base = base.split('.').next().unwrap_or_default();
        if base.is_empty() || base == "C" || base == "POSIX" {
            return None;
        }
        let (lang, region) = match base.split_once('_') {
            Some((l, r)) => (l.to_ascii_lowercase(), Some(r.to_ascii_uppercase())),
            None => (base.to_ascii_lowercase(), None),
        };
        // Yalnızca dil (`tr`, `en`, `eo`): önce aynı adlı bölge (`tr_TR`), sonra yaygın bölge
        // (`en` → `en_US`), sonra bölgesiz ad (`eo`), en son listede o dilin ilk adı.
        let candidates: Vec<String> = match region {
            Some(r) => vec![format!("{lang}_{r}")],
            None => [
                Some(format!("{lang}_{}", lang.to_ascii_uppercase())),
                names::likely_region(&lang).map(str::to_owned),
                Some(lang.clone()),
                names::first_with_language(&lang).map(str::to_owned),
            ]
            .into_iter()
            .flatten()
            .collect(),
        };
        let lookup = |s: &str| chrono::Locale::try_from(s).ok();
        let modifier = modifier.filter(|m| !m.is_empty()).map(str::to_ascii_lowercase);
        for plain in candidates {
            if let Some(m) = &modifier {
                // glibc adı: `sr_RS@latin`, `ca_ES@valencia` (yoksa değiştiricisiz ada düşülür).
                let name = format!("{plain}@{m}");
                if let Some(inner) = lookup(&name) {
                    return Some(Self { inner, name, twelve_hour: None });
                }
            }
            if let Some(inner) = lookup(&plain) {
                return Some(Self { inner, name: plain, twelve_hour: None });
            }
        }
        None
    }

    /// BCP 47 dil etiketinden (`tr`, `pt-BR`, `sr-Latn-RS`, `zh-Hant`; `_` de kabul) locale.
    /// Yazı alt etiketi glibc değiştiricisine çevrilir (`Latn` → `@latin`, `Cyrl` →
    /// `@cyrillic`, `Deva` → `@devanagari`); `zh-Hant` bölgesizse `zh_TW`. Bölgesiz etiket
    /// [`parse`](Self::parse) kuralıyla bölge bulur (`en` → `en_US`). Çözülemezse `None`.
    pub fn from_language_tag(tag: &str) -> Option<Self> {
        if tag.is_empty() || tag.len() > MAX_NAME_LEN || !tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')) {
            return None;
        }
        let mut parts = tag.split(['-', '_']);
        let lang = parts.next().filter(|l| (2..=3).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_alphabetic()))?;
        let (mut script, mut region) = (None, None);
        for p in parts {
            match p.len() {
                4 if script.is_none() && region.is_none() && p.bytes().all(|b| b.is_ascii_alphabetic()) => script = Some(p.to_ascii_lowercase()),
                2 if region.is_none() && p.bytes().all(|b| b.is_ascii_alphabetic()) => region = Some(p.to_ascii_uppercase()),
                // Sayısal bölge (419), varyant ve uzantılar glibc adlarında yok: yok sayılır.
                _ => {}
            }
        }
        let lang = lang.to_ascii_lowercase();
        if region.is_none() && lang == "zh" && script.as_deref() == Some("hant") {
            region = Some("TW".to_owned());
        }
        let modifier = match script.as_deref() {
            Some("latn") => Some("latin"),
            Some("cyrl") => Some("cyrillic"),
            Some("deva") => Some("devanagari"),
            _ => None,
        };
        let mut name = lang;
        if let Some(r) = region {
            name = format!("{name}_{r}");
        }
        if let Some(m) = modifier {
            name = format!("{name}@{m}");
        }
        Self::parse(&name)
    }

    /// Kanonik ad (`tr_TR`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Dil kodu (`tr`).
    pub fn language(&self) -> &str {
        self.name.split('_').next().unwrap_or_default()
    }

    /// Bölge kodu (`TR`).
    pub fn region(&self) -> &str {
        self.name.split(['_', '@']).nth(1).unwrap_or_default()
    }

    /// chrono'nun locale'i (kendi biçimlemesini yapan çağıran için).
    pub fn chrono(&self) -> chrono::Locale {
        self.inner
    }

    /// Haftanın ilk günü (CLDR `firstDay` bölgeleri; varsayılan Pazartesi).
    pub fn first_weekday(&self) -> Weekday {
        first_weekday_for_region(self.region())
    }

    /// 12/24 saat geçersiz kılması: `Some(true)` = 12 saat ("1:05 PM"), `Some(false)` = 24 saat
    /// ("13:05"), `None` = locale verisi (varsayılan). [`uses_12_hour`](Self::uses_12_hour),
    /// [`format_hm`](Self::format_hm) ve `format_system_time(.., Time | DateTime)` buna uyar;
    /// açık strftime kalıbı (`format_time(t, "%X")`, `%I`/`%H`) uymaz — çağıranın kalıbı kazanır.
    pub fn with_12_hour(mut self, twelve_hour: Option<bool>) -> Self {
        self.twelve_hour = twelve_hour;
        self
    }

    /// [`with_12_hour`](Self::with_12_hour) ile verilen geçersiz kılma (`None` = locale verisi).
    pub fn twelve_hour_override(&self) -> Option<bool> {
        self.twelve_hour
    }

    /// Saat:dakika 12 saatlik mi: geçersiz kılma ([`with_12_hour`](Self::with_12_hour)) varsa o,
    /// yoksa locale'in saat kalıbı (`%X`, glibc `T_FMT`: `en_US` → `%r`).
    pub fn uses_12_hour(&self) -> bool {
        self.twelve_hour.unwrap_or_else(|| self.data_uses_12_hour())
    }

    /// Locale verisinin saat kalıbı (`%X`) 12 saatlik mi.
    fn data_uses_12_hour(&self) -> bool {
        let one_pm = NaiveTime::from_hms_opt(13, 0, 0).unwrap_or_default();
        self.format_time(one_pm, "%X").is_ok_and(|s| !s.contains("13"))
    }

    /// Gün dilimi adları `[öğleden önce, öğleden sonra]` (glibc `AM_STR`/`PM_STR`, `%p`); locale'de
    /// boşsa (24 saatlik locale'lerin çoğu) `["AM", "PM"]`.
    pub fn day_periods(&self) -> [String; 2] {
        [(1, "AM"), (13, "PM")].map(|(h, fallback)| {
            NaiveTime::from_hms_opt(h, 0, 0)
                .and_then(|t| self.format_time(t, "%p").ok())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| fallback.to_owned())
        })
    }

    /// Ay adı; `month` 1–12 (dışı boş dizge).
    pub fn month_name(&self, month: u32, len: Length) -> String {
        let pattern = if len == Length::Long { "%B" } else { "%b" };
        NaiveDate::from_ymd_opt(2001, month, 1)
            .and_then(|d| self.format_date(d, pattern).ok())
            .unwrap_or_default()
    }

    /// Gün adı.
    pub fn weekday_name(&self, day: Weekday, len: Length) -> String {
        let pattern = if len == Length::Long { "%A" } else { "%a" };
        // 1 Ocak 2001 Pazartesi.
        NaiveDate::from_ymd_opt(2001, 1, 1 + day.num_days_from_monday())
            .and_then(|d| self.format_date(d, pattern).ok())
            .unwrap_or_default()
    }

    /// Tarih (`%x`, `%d %B %Y`…).
    pub fn format_date(&self, date: NaiveDate, pattern: &str) -> Result<String, FormatError> {
        let items = self.items(pattern)?;
        render(date.format_localized_with_items(items.iter(), self.inner))
    }

    /// Saat (`%X`, `%H:%M`…). Tarih öğesi (`%d`) içeren kalıp → [`FormatError::Value`].
    pub fn format_time(&self, time: NaiveTime, pattern: &str) -> Result<String, FormatError> {
        let items = self.items(pattern)?;
        render(DelayedFormat::new_with_locale(None, Some(time), items.iter(), self.inner))
    }

    /// Saat dilimli tarih-saat (`%c`, `%A %H:%M %Z`…).
    pub fn format_datetime<Tz: TimeZone>(&self, dt: &DateTime<Tz>, pattern: &str) -> Result<String, FormatError>
    where
        Tz::Offset: fmt::Display,
    {
        let items = self.items(pattern)?;
        render(dt.format_localized_with_items(items.iter(), self.inner))
    }

    /// Ondalık ayırıcı (`LC_NUMERIC` `decimal_point`: tr_TR → `,`, en_US → `.`).
    pub fn decimal_separator(&self) -> &'static str {
        pure_rust_locales::locale_match!(self.inner => LC_NUMERIC::DECIMAL_POINT)
    }

    /// Basamak gruplama ayırıcısı (`thousands_sep`: tr_TR → `.`, fr_FR → U+202F); gruplama
    /// yapmayan locale'de boş.
    pub fn grouping_separator(&self) -> &'static str {
        pure_rust_locales::locale_match!(self.inner => LC_NUMERIC::THOUSANDS_SEP)
    }

    /// Grup boyları sağdan sola (`grouping`: `[3, 3]` → 1.234.567; en_IN `[3, 2]` → 12,34,567);
    /// son eleman tekrarlanır, `-1` ya da boş = gruplama yok.
    pub fn grouping(&self) -> &'static [i64] {
        pure_rust_locales::locale_match!(self.inner => LC_NUMERIC::GROUPING)
    }

    /// Yüzde kalıbı (CLDR; [`PercentPattern::for_locale`]): tr_TR `%50`, en_US `50%`, de_DE `50 %`.
    pub fn percent_pattern(&self) -> PercentPattern {
        PercentPattern::for_locale(self.language(), self.region())
    }

    /// Oranı (`0.5` = yarı) yüzde olarak tam `fraction_digits` kesir basamağıyla biçimler:
    /// tr_TR `0.5, 0` → "%50", en_US → "50%", de_DE → "50 %" (U+00A0). Panik yok.
    pub fn format_percent(&self, ratio: f64, fraction_digits: u8) -> String {
        let mut options = NumberOptions::fixed(usize::from(fraction_digits));
        options.percent = true;
        self.number_symbols().format(ratio, &options)
    }

    /// Sayı simgeleri (ondalık, gruplama ayırıcısı, grup boyları, yüzde kalıbı) tek değerde.
    pub fn number_symbols(&self) -> NumberSymbols {
        NumberSymbols {
            decimal: self.decimal_separator(),
            group: self.grouping_separator(),
            grouping: self.grouping(),
            percent: self.percent_pattern(),
        }
    }

    /// Sayıyı tam `fraction_digits` kesir basamağıyla (yuvarlanır, en çok
    /// [`MAX_FRACTION_DIGITS`]) ve gruplamayla biçimler: tr_TR `1234567.891, 2` →
    /// "1.234.567,89", en_IN `1234567.0, 0` → "12,34,567". NaN → "NaN", sonsuz → "∞"/"-∞";
    /// panik yok.
    pub fn format_number(&self, value: f64, fraction_digits: u8) -> String {
        self.number_symbols().format(value, &NumberOptions::fixed(usize::from(fraction_digits)))
    }

    /// Seçenekli sayı biçimi ([`NumberOptions`]; Fluent `NUMBER` ile aynı uygulama).
    pub fn format_number_with(&self, value: f64, options: &NumberOptions) -> String {
        self.number_symbols().format(value, options)
    }

    /// Kalıbı bu locale'le ayrıştırır (`%x`/`%X`/`%c` locale kalıbına açılır).
    fn items<'a>(&self, pattern: &'a str) -> Result<Vec<Item<'a>>, FormatError> {
        if pattern.len() > MAX_PATTERN_LEN {
            return Err(FormatError::Pattern);
        }
        StrftimeItems::new_with_locale(pattern, self.inner).parse().map_err(|_| FormatError::Pattern)
    }
}

/// `Display` hatasını panik yerine `Value` olarak döndürür.
fn render(f: impl fmt::Display) -> Result<String, FormatError> {
    let mut out = String::new();
    write!(out, "{f}").map_err(|_| FormatError::Value)?;
    Ok(out)
}

/// CLDR `firstDay`: Pazar ve Cumartesi ile başlayan bölgeler; diğerleri Pazartesi.
fn first_weekday_for_region(region: &str) -> Weekday {
    const SUNDAY: &[&str] = &[
        "AG", "AS", "BD", "BR", "BS", "BT", "BW", "BZ", "CA", "CO", "DM", "DO", "ET", "GT", "GU", "HK", "HN", "ID", "IL", "IN", "JM",
        "JP", "KE", "KH", "KR", "LA", "MH", "MM", "MO", "MT", "MX", "MZ", "NI", "NP", "PA", "PE", "PH", "PK", "PR", "PT", "PY", "SA",
        "SG", "SV", "TH", "TT", "TW", "UM", "US", "VE", "VI", "WS", "YE", "ZA", "ZW",
    ];
    const SATURDAY: &[&str] = &["AE", "AF", "BH", "DJ", "DZ", "EG", "IQ", "IR", "JO", "KW", "LY", "OM", "QA", "SD", "SY"];
    if SUNDAY.contains(&region) {
        Weekday::Sun
    } else if SATURDAY.contains(&region) {
        Weekday::Sat
    } else {
        Weekday::Mon
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn loc(s: &str) -> Locale {
        Locale::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn env_precedence_is_posix() {
        let pick = |a, t, l| Locale::from_vars(a, t, l).name().to_owned();
        assert_eq!(pick(Some("de_DE.UTF-8"), Some("tr_TR.UTF-8"), Some("fr_FR.UTF-8")), "de_DE");
        assert_eq!(pick(Some(""), Some("tr_TR.UTF-8"), Some("fr_FR.UTF-8")), "tr_TR");
        assert_eq!(pick(None, None, Some("fr_FR.UTF-8")), "fr_FR");
        // İlk boş olmayan karar verir: LC_ALL=C → en_US yedeği, LANG'a düşülmez.
        assert_eq!(pick(Some("C"), Some("tr_TR.UTF-8"), None), "en_US");
        assert_eq!(pick(None, Some("POSIX"), Some("tr_TR.UTF-8")), "en_US");
        assert_eq!(pick(None, None, None), "en_US");
        assert_eq!(pick(None, None, Some("xx_YY.UTF-8")), "en_US");
    }

    #[test]
    fn parses_codesets_modifiers_and_bare_languages() {
        assert_eq!(loc("tr_TR.UTF-8").name(), "tr_TR");
        assert_eq!(loc("tr").name(), "tr_TR");
        assert_eq!(loc("de_DE@euro").name(), "de_DE@euro");
        // Veride olmayan değiştirici: değiştiricisiz ada düşülür.
        assert_eq!(loc("tr_TR@bilinmez").name(), "tr_TR");
        let sr = loc("sr_RS.UTF-8@latin");
        assert_eq!((sr.name(), sr.language(), sr.region()), ("sr_RS@latin", "sr", "RS"));
        assert_eq!(sr.month_name(1, Length::Long).to_lowercase(), "januar");
        for bad in ["", "C", "C.UTF-8", "POSIX", "tr_TR;rm", "../../etc", &"a".repeat(100), "xx_YY"] {
            assert!(Locale::parse(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn names_come_from_locale_data() {
        let tr = loc("tr_TR");
        assert_eq!(tr.month_name(10, Length::Long), "Ekim");
        assert_eq!(tr.month_name(2, Length::Short), "Şub");
        assert_eq!(tr.weekday_name(Weekday::Mon, Length::Long), "Pazartesi");
        assert_eq!(tr.weekday_name(Weekday::Sun, Length::Short), "Paz");
        assert_eq!(Locale::en_us().month_name(10, Length::Long), "October");
        assert_eq!(tr.month_name(13, Length::Long), "");
    }

    #[test]
    fn hour_cycle_and_first_weekday() {
        assert!(Locale::en_us().uses_12_hour());
        assert!(!loc("tr_TR").uses_12_hour());
        assert!(!loc("en_GB").uses_12_hour());
        assert!(!Locale::en_us().with_12_hour(Some(false)).uses_12_hour());
        assert!(loc("tr_TR").with_12_hour(Some(true)).uses_12_hour());
        assert!(Locale::en_us().with_12_hour(None).uses_12_hour());
        assert_eq!(loc("tr_TR").with_12_hour(Some(true)).twelve_hour_override(), Some(true));
        assert_eq!(Locale::en_us().first_weekday(), Weekday::Sun);
        assert_eq!(loc("tr_TR").first_weekday(), Weekday::Mon);
        assert_eq!(loc("ar_EG").first_weekday(), Weekday::Sat);
    }

    #[test]
    fn numeric_separators_from_locale_data() {
        let sep = |s: &str| {
            let l = loc(s);
            (l.decimal_separator(), l.grouping_separator(), l.grouping())
        };
        assert_eq!(sep("tr_TR.UTF-8"), (",", ".", &[3, 3][..]));
        assert_eq!(sep("de_DE"), (",", ".", &[3, 3][..]));
        assert_eq!(sep("fr_FR"), (",", "\u{202f}", &[3][..]));
        assert_eq!(sep("en_IN"), (".", ",", &[3, 2][..]));
        let en = Locale::en_us();
        assert_eq!((en.decimal_separator(), en.grouping_separator()), (".", ","));
        // Sayı kategorisi: LC_ALL > LC_NUMERIC > LANG; C → en_US yedeği.
        let pick = |a, n, l| Locale::from_vars(a, n, l).decimal_separator();
        assert_eq!(pick(None, Some("tr_TR.UTF-8"), Some("en_US.UTF-8")), ",");
        assert_eq!(pick(Some("en_US.UTF-8"), Some("tr_TR.UTF-8"), None), ".");
        assert_eq!(pick(None, None, Some("de_DE.UTF-8")), ",");
        assert_eq!(pick(None, Some("C"), Some("tr_TR.UTF-8")), ".");
    }

    #[test]
    fn day_periods_come_from_locale_data() {
        assert_eq!(Locale::en_us().day_periods(), ["AM", "PM"]);
        // tr_TR glibc verisinde ÖÖ/ÖS; Korece 오전/오후.
        assert_eq!(loc("tr_TR").day_periods(), ["ÖÖ", "ÖS"]);
        assert_eq!(loc("ko_KR").day_periods(), ["오전", "오후"]);
        // Boş AM/PM (de_DE) → yedek.
        assert_eq!(loc("de_DE").day_periods(), ["AM", "PM"]);
    }

    #[test]
    fn bare_languages_and_language_tags() {
        let name = |s: &str| Locale::parse(s).map(|l| l.name().to_owned());
        // Aynı adlı bölge önce (eski davranış korunur), sonra yaygın bölge, bölgesiz ad.
        assert_eq!(name("pt").as_deref(), Some("pt_PT"));
        assert_eq!(name("en").as_deref(), Some("en_US"));
        assert_eq!(name("ja").as_deref(), Some("ja_JP"));
        assert_eq!(name("zh").as_deref(), Some("zh_CN"));
        assert_eq!(name("eo").as_deref(), Some("eo"));
        assert_eq!(name("sr@latin").as_deref(), Some("sr_RS@latin"));
        assert_eq!(name("gez").as_deref(), Some("gez_ER"));
        assert_eq!(name("posix"), None);
        let tag = |s: &str| Locale::from_language_tag(s).map(|l| l.name().to_owned());
        assert_eq!(tag("tr").as_deref(), Some("tr_TR"));
        assert_eq!(tag("en").as_deref(), Some("en_US"));
        assert_eq!(tag("pt-BR").as_deref(), Some("pt_BR"));
        assert_eq!(tag("en_IN").as_deref(), Some("en_IN"));
        assert_eq!(tag("sr-Latn-RS").as_deref(), Some("sr_RS@latin"));
        assert_eq!(tag("sr-Latn").as_deref(), Some("sr_RS@latin"));
        assert_eq!(tag("zh-Hant").as_deref(), Some("zh_TW"));
        assert_eq!(tag("es-419").as_deref(), Some("es_ES"));
        for bad in ["", "x", "toolong-XX", "tr TR", "../tr", "C", &"a".repeat(100), "qps"] {
            assert_eq!(tag(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn number_formatting() {
        let n = |s: &str, v: f64, f: u8| loc(s).format_number(v, f);
        assert_eq!(n("tr_TR", 1234567.891, 2), "1.234.567,89");
        assert_eq!(n("en_US", 1234567.891, 2), "1,234,567.89");
        assert_eq!(n("de_DE", -1234.5, 1), "-1.234,5");
        assert_eq!(n("fr_FR", 1234567.0, 0), "1\u{202f}234\u{202f}567");
        assert_eq!(n("en_IN", 1234567.0, 0), "12,34,567");
        assert_eq!(n("ja_JP", 1234.5, 3), "1,234.500");
        assert_eq!(n("tr_TR", f64::NAN, 2), "NaN");
        assert_eq!(n("tr_TR", f64::NEG_INFINITY, 2), "-∞");
        assert_eq!(n("tr_TR", -0.001, 2), "0,00");
        let huge = n("en_US", 1e300, u8::MAX);
        assert!(huge.len() < 500 && huge.ends_with(&"0".repeat(MAX_FRACTION_DIGITS)), "{huge}");
        let o = NumberOptions { percent: true, ..NumberOptions::default() };
        assert_eq!(loc("tr_TR").format_number_with(0.256, &o), "%25,6");
        assert_eq!(loc("en_US").format_number_with(0.256, &o), "25.6%");
        assert_eq!(loc("tr_TR").format_percent(0.5, 0), "%50");
        assert_eq!(loc("tr_TR").format_percent(-0.05, 0), "-%5");
        assert_eq!(loc("en_US").format_percent(0.5, 0), "50%");
        assert_eq!(loc("de_DE").format_percent(0.5, 0), "50\u{a0}%");
        assert_eq!(loc("de_CH").format_percent(0.5, 0), "50%");
        assert_eq!(loc("fr_FR").format_percent(0.125, 1), "12,5\u{202f}%");
        assert_eq!(loc("eu_ES").format_percent(0.5, 0), "%\u{a0}50");
        assert_eq!(loc("ja_JP").format_percent(0.5, 0), "50%");
        assert_eq!(loc("tr_TR").percent_pattern().apply("50"), "%50");
        assert_eq!(loc("tr_TR").percent_pattern().apply("-5"), "-%5");
        assert_eq!(loc("en_US").percent_pattern().apply("50"), "50%");
        assert_eq!(loc("tr_TR").format_percent(f64::NAN, 0), "NaN");
        let tr = NumberSymbols { decimal: ",", group: ".", grouping: &[3, 3], percent: PercentPattern { prefix: true, gap: "" } };
        assert_eq!(loc("tr_TR").number_symbols(), tr);
    }

    #[test]
    fn formatting_never_panics() {
        let tr = loc("tr_TR");
        let d = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        assert_eq!(tr.format_date(d, "%-d %B %Y, %A").unwrap(), "4 Ekim 2026, Pazar");
        assert_eq!(tr.format_date(d, "%x").unwrap(), "04-10-2026");
        let t = NaiveTime::from_hms_opt(13, 5, 0).unwrap();
        assert_eq!(tr.format_time(t, "%H:%M").unwrap(), "13:05");
        assert!(Locale::en_us().format_time(t, "%X").unwrap().contains("PM"));
        // Geçersiz kalıp, eksik değer, aşırı uzun kalıp: hata, panik yok.
        assert_eq!(tr.format_date(d, "%Q"), Err(FormatError::Pattern));
        assert_eq!(tr.format_time(t, "%d"), Err(FormatError::Value));
        assert_eq!(tr.format_date(d, "%Z"), Err(FormatError::Value));
        assert_eq!(tr.format_date(d, &"%Y".repeat(200)), Err(FormatError::Pattern));
        let dt = FixedOffset::east_opt(3 * 3600).unwrap().with_ymd_and_hms(2026, 10, 4, 9, 30, 0).unwrap();
        assert_eq!(tr.format_datetime(&dt, "%A %H:%M %:z").unwrap(), "Pazar 09:30 +03:00");
    }
}
