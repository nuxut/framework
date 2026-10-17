//! chrono türü istemeyen tarih/saat biçimleri: kısa (`%x`), uzun ("4 Ekim 2026"), haftanın
//! günüyle uzun, "ay yıl" başlığı, saat:dakika ve `SystemTime` (yerel saat dilimi).
//!
//! **Sıra veriden:** gün/ay/yıl sırası locale'in `%x` kalıbından (glibc `d_fmt`) türetilir
//! (G-A-Y / A-G-Y / Y-A-G). ISO kalıbı (`%Y-%m-%d`: sv_SE, en_CA, hu_HU, lt_LT) bir dil sırası
//! değil, makine biçimidir: uzun biçimde İngilizce A-G-Y, diğerleri G-A-Y sayılır.
//! **Noktalama küçük bir tablodan** (glibc'de uzun tarih kalıbı yok; `date_fmt` dağınık ve
//! saat içerir):
//! - Doğu Asya (ja/zh/yue/cmn/lzh/hak/nan, ko): sayısal ay, "2026年10月4日" / "2026년 10월 4일";
//!   başlık "2026年10月" / "2026년 10월".
//! - hu: "2026. október 4." (glibc `d_fmt` ISO olduğundan sıra dilden); lv "2026. gada 4.
//!   oktobris"; ca "4 d’octubre de 2026" (glibc ca `mon` edatı içerir).
//! - Gün sonrası nokta (sıra sayısı: de/da/nb/nn/fi/cs/sk/sl/hr/bs/sr/is/fo/et/lb/nds/hsb/dsb):
//!   "4. Oktober 2026".
//! - es/pt/gl/ast/an: "4 de octubre de 2026".
//! - A-G-Y: "October 4, 2026".
//! - Haftanın günü: tr/az/Doğu Asya sonda ("4 Ekim 2026 Pazar"), hu sonda virgülle,
//!   fr/it/nl/ca başta virgülsüz ("dimanche 4 octobre 2026"), diğerleri "Sunday, October 4, 2026".
//!
//! Ay adı tarih içinde `%B` (glibc `mon`; ru/pl/cs/el/uk/lt'de -in hâli: "4 октября"), başlıkta
//! bağımsız ad (`alt_mon`; yoksa `mon`: "Октябрь 2026"). Bilinen sınır: ek isteyen diller
//! (fi "lokakuuta", ru "г.", lt "m./d.") yalın biçimde kalır.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};

use crate::{FormatError, Length, Locale};

/// Tarih biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum DateStyle {
    /// Locale'in kısa kalıbı (`%x`): "04-10-2026", "10/04/2026", "04.10.2026".
    Short,
    /// Uzun: "4 Ekim 2026", "October 4, 2026", "4. Oktober 2026", "2026年10月4日".
    #[default]
    Long,
    /// Haftanın günüyle uzun: "4 Ekim 2026 Pazar", "Sunday, October 4, 2026".
    Full,
}

/// [`Locale::format_system_time`] biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DateTimeStyle {
    /// Yalnızca tarih.
    Date(DateStyle),
    /// Yalnızca saat:dakika ([`Locale::format_hm`]).
    Time,
    /// Tarih + boşluk + saat:dakika.
    DateTime(DateStyle),
}

/// `%x`'ten türetilen gün/ay/yıl sırası.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Dmy,
    Mdy,
    Ymd,
}

/// Doğu Asya yazımı (sayısal ay + birim karakterleri).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EastAsian {
    Han,
    Hangul,
}

/// Yıl sınırı: `SystemTime` → yerel tarih bu aralık dışında [`FormatError::Value`].
const MAX_ABS_YEAR: i32 = 9999;

impl Locale {
    /// Yıl/ay/gün'ü biçimler. Geçersiz tarih (ay 13, gün 0, 31 Şubat) → [`FormatError::Value`].
    pub fn format_ymd(&self, year: i32, month: u32, day: u32, style: DateStyle) -> Result<String, FormatError> {
        let date = NaiveDate::from_ymd_opt(year, month, day).ok_or(FormatError::Value)?;
        self.date_with_style(date, style)
    }

    /// "Ay yıl" başlığı: "Ekim 2026", "October 2026", "Октябрь 2026", "2026年10月",
    /// "2026년 10월". Ay 1–12 dışı → [`FormatError::Value`].
    pub fn format_month_year(&self, year: i32, month: u32) -> Result<String, FormatError> {
        if !(1..=12).contains(&month) {
            return Err(FormatError::Value);
        }
        Ok(self.compose_month_year(&self.standalone_month_name(month), year))
    }

    /// Hazır bir ay metnini (bağımsız ad, ya da "Eki – Kas" gibi aralık) locale sırasıyla
    /// yılla birleştirir: "Ekim 2026" / "2026年10月" / "2026. október".
    pub fn compose_month_year(&self, month: &str, year: i32) -> String {
        match self.east_asian() {
            Some(EastAsian::Han) => format!("{year}年{month}"),
            Some(EastAsian::Hangul) => format!("{year}년 {month}"),
            None if self.language() == "hu" => format!("{year}. {month}"),
            None if self.language() == "lv" => format!("{year}. gada {month}"),
            None if self.order() == Order::Ymd => format!("{year} {month}"),
            None => format!("{month} {year}"),
        }
    }

    /// Bağımsız (yalın) ay adı — başlık ve takvim için: glibc `alt_mon`, yoksa `mon`
    /// (ru "Октябрь"; tarih içi `%B` "октября"). Doğu Asya'da sayısal ("10月", "10월").
    /// `month` 1–12 dışı boş dizge.
    pub fn standalone_month_name(&self, month: u32) -> String {
        if !(1..=12).contains(&month) {
            return String::new();
        }
        match self.east_asian() {
            Some(EastAsian::Han) => return format!("{month}月"),
            Some(EastAsian::Hangul) => return format!("{month}월"),
            None => {}
        }
        let i = (month - 1) as usize;
        let alt: Option<&[&str]> = pure_rust_locales::locale_match!(self.inner => LC_TIME::ALT_MON);
        let mon: &[&str] = pure_rust_locales::locale_match!(self.inner => LC_TIME::MON);
        alt.and_then(|a| a.get(i)).or_else(|| mon.get(i)).map(|s| (*s).to_owned()).unwrap_or_default()
    }

    /// Saat:dakika, locale'in saat kalıbından saniye atılarak (24 saat `%X` = glibc `t_fmt`,
    /// 12 saat `t_fmt_ampm`; saat başı sıfırsız): "13:05", "1:05 PM", "13시 05분",
    /// fi "13.05". 12/24 saat seçimi [`uses_12_hour`](Self::uses_12_hour)'dan (geçersiz kılma:
    /// [`with_12_hour`](Self::with_12_hour)); locale'in gün dilimi adı yoksa (de_DE gibi 24
    /// saatlik locale'e 12 saat zorlanınca) "AM"/"PM" ([`day_periods`](Self::day_periods)).
    /// Saat > 23 ya da dakika > 59 → [`FormatError::Value`].
    pub fn format_hm(&self, hour: u32, minute: u32) -> Result<String, FormatError> {
        let time = NaiveTime::from_hms_opt(hour, minute, 0).ok_or(FormatError::Value)?;
        let twelve = self.uses_12_hour();
        let periods = |pattern: &str| -> String {
            if !twelve || !pattern.contains("%p") || self.format_time(time, "%p").is_ok_and(|p| !p.trim().is_empty()) {
                return pattern.to_owned();
            }
            let [am, pm] = self.day_periods();
            pattern.replace("%p", &(if hour < 12 { am } else { pm }).replace('%', "%%"))
        };
        // Veri kalıbı biçimlenemezse (beklenmez) güvenli yedek.
        self.format_time(time, &periods(&self.hm_pattern()))
            .or_else(|_| self.format_time(time, &periods(if twelve { "%-I:%M %p" } else { "%H:%M" })))
            .map(|s| s.trim().to_owned())
    }

    /// `SystemTime`'ı yerel saat diliminde (`TZ` / `/etc/localtime`) biçimler. Temsil
    /// edilemeyen ya da ±9999 yılı dışındaki an → [`FormatError::Value`] (panik yok).
    pub fn format_system_time(&self, time: SystemTime, style: DateTimeStyle) -> Result<String, FormatError> {
        let local = local_datetime(time).ok_or(FormatError::Value)?;
        let hm = || self.format_hm(local.hour(), local.minute());
        match style {
            DateTimeStyle::Date(s) => self.date_with_style(local.date(), s),
            DateTimeStyle::Time => hm(),
            DateTimeStyle::DateTime(s) => Ok(format!("{} {}", self.date_with_style(local.date(), s)?, hm()?)),
        }
    }

    fn date_with_style(&self, date: NaiveDate, style: DateStyle) -> Result<String, FormatError> {
        match style {
            DateStyle::Short => self.format_date(date, "%x"),
            DateStyle::Long => Ok(self.long_date(date)),
            DateStyle::Full => {
                let long = self.long_date(date);
                let wd = self.weekday_name(date.weekday(), Length::Long);
                Ok(match (self.east_asian(), self.language()) {
                    (Some(_), _) | (None, "tr" | "az") => format!("{long} {wd}"),
                    (None, "hu") => format!("{long}, {wd}"),
                    (None, "fr" | "it" | "nl" | "ca") => format!("{wd} {long}"),
                    _ => format!("{wd}, {long}"),
                })
            }
        }
    }

    fn long_date(&self, date: NaiveDate) -> String {
        let (y, m, d) = (date.year(), date.month(), date.day());
        match self.east_asian() {
            Some(EastAsian::Han) => return format!("{y}年{m}月{d}日"),
            Some(EastAsian::Hangul) => return format!("{y}년 {m}월 {d}일"),
            None => {}
        }
        let month = self.month_name(m, Length::Long);
        let lang = self.language();
        // Sırası ya da eki `%x`'ten çıkmayan diller.
        match lang {
            "hu" => return format!("{y}. {month} {d}."),
            "lv" => return format!("{y}. gada {d}. {month}"),
            // glibc ca `mon` edatı içerir ("d’octubre").
            "ca" => return format!("{d} {month} de {y}"),
            _ => {}
        }
        match self.order() {
            Order::Mdy => format!("{month} {d}, {y}"),
            Order::Ymd => format!("{y} {month} {d}"),
            Order::Dmy => match lang {
                "es" | "pt" | "gl" | "ast" | "an" => format!("{d} de {month} de {y}"),
                "de" | "da" | "nb" | "nn" | "fi" | "cs" | "sk" | "sl" | "hr" | "bs" | "sr" | "is" | "fo" | "et" | "lb"
                | "nds" | "hsb" | "dsb" => format!("{d}. {month} {y}"),
                _ => format!("{d} {month} {y}"),
            },
        }
    }

    fn east_asian(&self) -> Option<EastAsian> {
        match self.language() {
            "ko" => Some(EastAsian::Hangul),
            "ja" | "zh" | "yue" | "cmn" | "lzh" | "hak" => Some(EastAsian::Han),
            // nan_TW@latin Latin yazılı (Pe̍h-ōe-jī).
            "nan" if self.modifier().is_none() => Some(EastAsian::Han),
            _ => None,
        }
    }

    /// `%x` (glibc `d_fmt`) kalıbındaki gün/ay/yıl sırası.
    fn order(&self) -> Order {
        let d_fmt: &str = pure_rust_locales::locale_match!(self.inner => LC_TIME::D_FMT);
        match order_of(d_fmt) {
            // ISO makine biçimi: dil sırası değil.
            Some(Order::Ymd) if d_fmt.contains('-') => {
                if self.language() == "en" { Order::Mdy } else { Order::Dmy }
            }
            Some(o) => o,
            None => Order::Dmy,
        }
    }

    /// Saniyesiz saat kalıbı (locale verisinden).
    fn hm_pattern(&self) -> String {
        let twelve = self.uses_12_hour();
        let raw: &str = if twelve {
            pure_rust_locales::locale_match!(self.inner => LC_TIME::T_FMT_AMPM)
        } else {
            pure_rust_locales::locale_match!(self.inner => LC_TIME::T_FMT)
        };
        let fallback = if twelve { "%-I:%M %p" } else { "%H:%M" };
        let expanded = raw.replace("%T", "%H:%M:%S").replace("%r", "%I:%M:%S %p").replace("%R", "%H:%M");
        // Saat dilimi (anp_IN `%I:%M:%S %p %Z`) saat:dakikanın parçası değil; değersiz `%Z` hata verir.
        let expanded = expanded.replace(" %Z", "").replace(" %z", "").replace("%Z", "").replace("%z", "");
        let pattern = strip_seconds(expanded.trim());
        let pattern = pattern.replace("%I", "%-I").replace("%l", "%-I");
        // Saat ya da dakika içermeyen (bozuk/boş) veri ya da istenen döngüye uymayan kalıp
        // (`hour_cycle = "h23"` iken en_US `T_FMT` = `%r` 12 saatlik) → güvenli yedek.
        let has = |set: &[&str]| set.iter().any(|t| pattern.contains(t));
        let cycle_ok = if twelve { has(&["%-I"]) && !has(&["%H", "%k"]) } else { has(&["%H", "%k"]) && !has(&["%-I", "%p"]) };
        if cycle_ok && pattern.contains("%M") && pattern.len() <= crate::MAX_PATTERN_LEN {
            pattern
        } else {
            fallback.to_owned()
        }
    }
}

/// Kalıptaki gün (`%d %e`), ay (`%m %b %B %h`), yıl (`%Y %y %C %G %g`) sırası; `%D` = A/G/Y,
/// `%F` = Y-A-G. Bayrak/değiştiriciler (`-_0^#EO`) atlanır.
fn order_of(pattern: &str) -> Option<Order> {
    #[derive(PartialEq, Clone, Copy)]
    enum F {
        D,
        M,
        Y,
    }
    let mut seen: Vec<F> = Vec::with_capacity(3);
    let mut push = |f: F| {
        if !seen.contains(&f) {
            seen.push(f);
        }
    };
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            continue;
        }
        let conv = chars.by_ref().find(|c| !matches!(c, '-' | '_' | '0' | '^' | '#' | 'E' | 'O'));
        match conv {
            Some('d' | 'e') => push(F::D),
            Some('m' | 'b' | 'B' | 'h') => push(F::M),
            Some('Y' | 'y' | 'C' | 'G' | 'g') => push(F::Y),
            Some('D') => [F::M, F::D, F::Y].into_iter().for_each(&mut push),
            Some('F') => [F::Y, F::M, F::D].into_iter().for_each(&mut push),
            _ => {}
        }
    }
    match seen.as_slice() {
        [F::D, F::M, F::Y] => Some(Order::Dmy),
        [F::M, F::D, F::Y] => Some(Order::Mdy),
        [F::Y, F::M, F::D] => Some(Order::Ymd),
        _ => None,
    }
}

/// `%S`'i, önündeki noktalamayı (`:` `.`) ya da boşluğu ve ardından gelen birim ekini (`초`,
/// `秒`) atar: "%H:%M:%S" → "%H:%M", "%I:%M:%S %p" → "%I:%M %p", "%H시 %M분 %S초" →
/// "%H시 %M분".
fn strip_seconds(pattern: &str) -> String {
    let Some(i) = pattern.find("%S") else { return pattern.to_owned() };
    let (before, after) = (&pattern[..i], &pattern[i + 2..]);
    let suffix_end = after.find(|c: char| c == '%' || c.is_whitespace()).unwrap_or(after.len());
    let after = &after[suffix_end..];
    let trimmed = before.trim_end_matches(|c: char| c.is_ascii_punctuation() && c != '%');
    let before = if trimmed.len() == before.len() { before.trim_end() } else { trimmed };
    // Önceki öğe `%` ile bitiyorsa (ör. "%%S") dokunma.
    if before.ends_with('%') {
        return pattern.to_owned();
    }
    format!("{before}{after}")
}

/// `SystemTime` → yerel tarih-saat; temsil edilemezse ya da yıl ±9999 dışındaysa `None`.
fn local_datetime(time: SystemTime) -> Option<NaiveDateTime> {
    let (secs, nanos) = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => (i64::try_from(d.as_secs()).ok()?, d.subsec_nanos()),
        Err(e) => {
            let d = e.duration();
            let secs = i64::try_from(d.as_secs()).ok()?;
            match d.subsec_nanos() {
                0 => (secs.checked_neg()?, 0),
                n => (secs.checked_neg()?.checked_sub(1)?, 1_000_000_000 - n),
            }
        }
    };
    let utc = DateTime::<Utc>::from_timestamp(secs, nanos)?;
    // Uç değerde yerel ofset taşmasın: önce UTC yılını sınırla.
    if utc.year().abs() > MAX_ABS_YEAR {
        return None;
    }
    let local = utc.with_timezone(&chrono::Local).naive_local();
    (local.year().abs() <= MAX_ABS_YEAR).then_some(local)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn loc(s: &str) -> Locale {
        Locale::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    fn long(s: &str) -> String {
        loc(s).format_ymd(2026, 10, 4, DateStyle::Long).unwrap()
    }

    #[test]
    fn order_from_d_fmt() {
        assert!(order_of("%d-%m-%Y") == Some(Order::Dmy));
        assert!(order_of("%m/%d/%y") == Some(Order::Mdy));
        assert!(order_of("%Y年%m月%d日") == Some(Order::Ymd));
        assert!(order_of("%D") == Some(Order::Mdy));
        assert!(order_of("%F") == Some(Order::Ymd));
        assert!(order_of("%e.%m.%Y") == Some(Order::Dmy));
        assert!(order_of("%-d.%-m.%Y") == Some(Order::Dmy));
        assert!(order_of("%H:%M").is_none() && order_of("").is_none() && order_of("%").is_none());
        assert!(loc("en_US").order() == Order::Mdy);
        assert!(loc("en_CA").order() == Order::Mdy, "ISO → İngilizce A-G-Y");
        assert!(loc("sv_SE").order() == Order::Dmy, "ISO → G-A-Y");
        assert!(loc("ja_JP").order() == Order::Ymd);
    }

    #[test]
    fn long_dates() {
        assert_eq!(long("tr_TR"), "4 Ekim 2026");
        assert_eq!(long("en_US"), "October 4, 2026");
        assert_eq!(long("en_GB"), "4 October 2026");
        assert_eq!(long("en_IN"), "4 October 2026");
        assert_eq!(long("de_DE"), "4. Oktober 2026");
        assert_eq!(long("fr_FR"), "4 octobre 2026");
        assert_eq!(long("es_ES"), "4 de octubre de 2026");
        assert_eq!(long("ru_RU"), "4 октября 2026");
        assert_eq!(long("ja_JP"), "2026年10月4日");
        assert_eq!(long("zh_CN"), "2026年10月4日");
        assert_eq!(long("ko_KR"), "2026년 10월 4일");
        assert_eq!(long("hu_HU"), "2026. október 4.");
        assert_eq!(long("sv_SE"), "4 oktober 2026");
    }

    #[test]
    fn short_and_full_dates() {
        let f = |s: &str, st| loc(s).format_ymd(2026, 10, 4, st).unwrap();
        assert_eq!(f("tr_TR", DateStyle::Short), "04-10-2026");
        assert_eq!(f("en_US", DateStyle::Short), "10/04/2026");
        assert_eq!(f("de_DE", DateStyle::Short), "04.10.2026");
        assert_eq!(f("tr_TR", DateStyle::Full), "4 Ekim 2026 Pazar");
        assert_eq!(f("en_US", DateStyle::Full), "Sunday, October 4, 2026");
        assert_eq!(f("de_DE", DateStyle::Full), "Sonntag, 4. Oktober 2026");
        assert_eq!(f("fr_FR", DateStyle::Full), "dimanche 4 octobre 2026");
        assert_eq!(f("ja_JP", DateStyle::Full), "2026年10月4日 日曜日");
        assert_eq!(f("ko_KR", DateStyle::Full), "2026년 10월 4일 일요일");
    }

    #[test]
    fn month_year_titles() {
        let my = |s: &str| loc(s).format_month_year(2026, 10).unwrap();
        assert_eq!(my("tr_TR"), "Ekim 2026");
        assert_eq!(my("en_US"), "October 2026");
        assert_eq!(my("de_DE"), "Oktober 2026");
        assert_eq!(my("ru_RU"), "Октябрь 2026");
        assert_eq!(my("ja_JP"), "2026年10月");
        assert_eq!(my("zh_CN"), "2026年10月");
        assert_eq!(my("ko_KR"), "2026년 10월");
        assert_eq!(my("hu_HU"), "2026. október");
        assert_eq!(loc("tr_TR").compose_month_year("Eki – Kas", 2026), "Eki – Kas 2026");
        assert_eq!(loc("ja_JP").compose_month_year("10月 – 11月", 2026), "2026年10月 – 11月");
        assert_eq!(loc("pl_PL").standalone_month_name(10), "październik");
        assert_eq!(loc("pl_PL").month_name(10, Length::Long), "października");
    }

    #[test]
    fn hour_minute() {
        let hm = |s: &str| loc(s).format_hm(13, 5).unwrap();
        assert_eq!(hm("tr_TR"), "13:05");
        assert_eq!(hm("en_US"), "1:05 PM");
        assert_eq!(hm("de_DE"), "13:05");
        assert_eq!(hm("ko_KR"), "13시 05분"); // glibc ko_KR t_fmt 24 saatlik
        assert_eq!(hm("fi_FI"), "13.05");
        assert_eq!(strip_seconds("%H:%M:%S"), "%H:%M");
        assert_eq!(strip_seconds("%I:%M:%S %p"), "%I:%M %p");
        assert_eq!(strip_seconds("%H시 %M분 %S초"), "%H시 %M분");
        assert_eq!(strip_seconds("%H:%M"), "%H:%M");
        assert_eq!(strip_seconds("%%S"), "%%S");
        // Her gömülü locale saat:dakika üretebilir.
        for name in crate::available() {
            let s = loc(name).format_hm(0, 0);
            assert!(s.as_ref().is_ok_and(|s| !s.is_empty()), "{name}: {s:?}");
        }
    }

    #[test]
    fn hour_cycle_override() {
        let hm = |s: &str, o: Option<bool>, h: u32| loc(s).with_12_hour(o).format_hm(h, 5).unwrap();
        // auto: dilden.
        assert_eq!(hm("tr_TR", None, 13), "13:05");
        assert_eq!(hm("en_US", None, 13), "1:05 PM");
        // h23 her iki dilde.
        assert_eq!(hm("tr_TR", Some(false), 13), "13:05");
        assert_eq!(hm("en_US", Some(false), 13), "13:05");
        assert_eq!(hm("en_US", Some(false), 9), "09:05");
        // h12 her iki dilde (tr gün dilimi glibc'den: ÖÖ/ÖS).
        assert_eq!(hm("en_US", Some(true), 13), "1:05 PM");
        assert_eq!(hm("en_US", Some(true), 0), "12:05 AM");
        let tr12 = hm("tr_TR", Some(true), 13);
        assert!(tr12.starts_with("1:05") && !tr12.contains("13"), "{tr12}");
        let [_, pm] = loc("tr_TR").day_periods();
        assert!(tr12.ends_with(pm.as_str()), "{tr12} / {pm}");
        // Gün dilimi adı olmayan locale'e 12 saat: AM/PM yedeği, sondaki boşluk yok.
        assert_eq!(hm("de_DE", Some(true), 13), "1:05 PM");
        assert_eq!(hm("de_DE", Some(true), 1), "1:05 AM");
        // Her gömülü locale her iki döngüde biçimlenir ve döngüye uyar.
        for name in crate::available() {
            let l = loc(name);
            let h24 = l.clone().with_12_hour(Some(false)).format_hm(13, 7).unwrap();
            assert!(h24.contains("13"), "{name}: {h24}");
            let h12 = l.with_12_hour(Some(true)).format_hm(13, 7).unwrap();
            assert!(!h12.contains("13") && !h12.is_empty(), "{name}: {h12}");
        }
    }

    #[test]
    fn hostile_inputs() {
        let tr = loc("tr_TR");
        for (y, m, d) in [(2026, 13, 1), (2026, 0, 1), (2026, 10, 0), (2026, 2, 30), (2026, 10, 32), (i32::MAX, 1, 1), (i32::MIN, 1, 1)] {
            for st in [DateStyle::Short, DateStyle::Long, DateStyle::Full] {
                assert_eq!(tr.format_ymd(y, m, d, st), Err(FormatError::Value), "{y}-{m}-{d}");
            }
        }
        assert_eq!(tr.format_month_year(2026, 13), Err(FormatError::Value));
        assert_eq!(tr.format_month_year(2026, 0), Err(FormatError::Value));
        assert_eq!(tr.standalone_month_name(13), "");
        assert_eq!(tr.format_hm(24, 0), Err(FormatError::Value));
        assert_eq!(tr.format_hm(0, 60), Err(FormatError::Value));
        let styles = [DateTimeStyle::Time, DateTimeStyle::Date(DateStyle::Full), DateTimeStyle::DateTime(DateStyle::Short)];
        let far = [
            UNIX_EPOCH.checked_add(Duration::from_secs(u64::MAX / 4)),
            UNIX_EPOCH.checked_add(Duration::from_secs(400 * 1_000_000 * 31_556_952)),
            UNIX_EPOCH.checked_sub(Duration::from_secs(20_000 * 31_556_952)),
        ];
        for t in far.into_iter().flatten() {
            for st in styles {
                assert_eq!(tr.format_system_time(t, st), Err(FormatError::Value));
            }
        }
    }

    #[test]
    fn system_time() {
        let tr = loc("tr_TR");
        // Saat dilimi ortama bağlı: yalnızca biçim denetlenir.
        let t = UNIX_EPOCH + Duration::from_secs(1_791_115_200); // 2026-10-04 12:00 UTC
        let date = tr.format_system_time(t, DateTimeStyle::Date(DateStyle::Long)).unwrap();
        assert!(date.ends_with("Ekim 2026"), "{date}");
        let time = tr.format_system_time(t, DateTimeStyle::Time).unwrap();
        assert_eq!(time.len(), 5, "{time}");
        let both = tr.format_system_time(t, DateTimeStyle::DateTime(DateStyle::Short)).unwrap();
        assert!(both.contains("-10-2026 "), "{both}");
        // Epoch öncesi (kesirli saniye dahil) biçimlenir.
        let before = UNIX_EPOCH - Duration::from_millis(1500);
        assert!(tr.format_system_time(before, DateTimeStyle::Date(DateStyle::Short)).is_ok());
    }
}
