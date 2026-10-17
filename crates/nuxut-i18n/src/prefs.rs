//! Dil tercihleri: `framework.toml [locale]` + ortam (ADR-0010 §3 "Dil seçimi").
//!
//! - `language = "<etiket>"` → o dil. `"auto"` → `LANGUAGE` (iki nokta listesi) > `LC_ALL` >
//!   `LC_MESSAGES` > `LANG`; ilk boş olmayan karar verir; `C`/`POSIX`/boş/çözülemeyen → `en`.
//! - `NUXUT_LANG` (yalnızca geliştirme) her şeyin önündedir: `qps` sözde dil, başka bir
//!   etiket o dili zorlar.
//! - Biçimler (`formats`) dil ile bağımsız: `auto` → `LC_ALL` > `LC_NUMERIC` > `LANG`
//!   (`nuxut-locale`); yalnızca `NUMBER`/sayı biçimi buradan okunur.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use nuxut_config::locale::{HourCycle, LocaleConfig, LocaleSetting};
use nuxut_locale::Locale;
use unic_langid::LanguageIdentifier;

/// Ortam değişkenlerinin anlık görüntüsü (saf çözüm ve testler için).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocaleEnv {
    pub nuxut_lang: Option<String>,
    pub language: Option<String>,
    pub lc_all: Option<String>,
    pub lc_messages: Option<String>,
    pub lc_numeric: Option<String>,
    pub lang: Option<String>,
}

impl LocaleEnv {
    /// Süreç ortamından.
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var(k).ok();
        Self {
            nuxut_lang: var("NUXUT_LANG"),
            language: var("LANGUAGE"),
            lc_all: var("LC_ALL"),
            lc_messages: var("LC_MESSAGES"),
            lc_numeric: var("LC_NUMERIC"),
            lang: var("LANG"),
        }
    }
}

/// [`crate::Catalog::load`]'ın girdisi: istenen diller (öncelik sırasıyla), sözde dil,
/// sayı locale'i, kullanıcı çeviri dizini.
#[derive(Debug, Clone, PartialEq)]
pub struct LanguagePrefs {
    requested: Vec<LanguageIdentifier>,
    pseudo: bool,
    numeric: Locale,
    user_dir: Option<PathBuf>,
}

impl LanguagePrefs {
    /// `[locale]` + süreç ortamı + varsayılan kullanıcı dizini.
    pub fn from_config(cfg: &LocaleConfig) -> Self {
        Self::resolve(&cfg.language, &cfg.formats, &LocaleEnv::from_process()).with_user_dir(default_user_dir())
    }

    /// Saf çözüm (kullanıcı dizini yok; [`with_user_dir`](Self::with_user_dir) ile eklenir).
    pub fn resolve(language: &LocaleSetting, formats: &LocaleSetting, env: &LocaleEnv) -> Self {
        let nonempty = |v: &Option<String>| v.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let numeric = formats_from(formats, env);
        if let Some(dev) = nonempty(&env.nuxut_lang) {
            if dev == "qps" || dev.starts_with("qps-") {
                return Self { requested: vec![en()], pseudo: true, numeric, user_dir: None };
            }
            if let Some(id) = posix_to_langid(&dev) {
                return Self { requested: vec![id], pseudo: false, numeric, user_dir: None };
            }
        }
        let requested = match language.value() {
            Some(tag) => match posix_to_langid(tag) {
                Some(id) => vec![id],
                None => {
                    // Değer loga yazılmaz (değişmez 9).
                    tracing::warn!("locale.language çözülemedi; 'auto' kullanıldı");
                    from_env(env)
                }
            },
            None => from_env(env),
        };
        Self { requested, pseudo: false, numeric, user_dir: None }
    }

    /// Sabit diller (testler, araçlar): ortam okunmaz, sayı biçimi `en_US`, kullanıcı dizini
    /// yok. Çözülemeyen etiket atlanır.
    pub fn fixed(tags: &[&str]) -> Self {
        Self {
            requested: tags.iter().filter_map(|t| posix_to_langid(t)).collect(),
            pseudo: false,
            numeric: Locale::en_us(),
            user_dir: None,
        }
    }

    /// Kullanıcı çeviri kökü (`<kök>/<domain>/<dil>.ftl`); `None` = yok.
    pub fn with_user_dir(mut self, dir: Option<PathBuf>) -> Self {
        self.user_dir = dir;
        self
    }

    /// Sözde dil (aksan + %35 uzatma; yalnızca geliştirme).
    pub fn with_pseudo(mut self, pseudo: bool) -> Self {
        self.pseudo = pseudo;
        self
    }

    /// Sayı locale'i (`NUMBER` ve `{ $n }` ayırıcıları).
    pub fn with_numeric(mut self, numeric: Locale) -> Self {
        self.numeric = numeric;
        self
    }

    pub fn requested(&self) -> &[LanguageIdentifier] {
        &self.requested
    }

    pub fn is_pseudo(&self) -> bool {
        self.pseudo
    }

    pub fn numeric(&self) -> &Locale {
        &self.numeric
    }

    pub fn user_dir(&self) -> Option<&Path> {
        self.user_dir.as_deref()
    }
}

fn en() -> LanguageIdentifier {
    LanguageIdentifier::from_str("en").unwrap_or_default()
}

/// `LANGUAGE` > `LC_ALL` > `LC_MESSAGES` > `LANG`.
fn from_env(env: &LocaleEnv) -> Vec<LanguageIdentifier> {
    if let Some(list) = env.language.as_deref().filter(|s| !s.trim().is_empty()) {
        let langs: Vec<_> = list.split(':').filter_map(posix_to_langid).take(16).collect();
        if !langs.is_empty() {
            return langs;
        }
    }
    let first = [&env.lc_all, &env.lc_messages, &env.lang]
        .into_iter()
        .filter_map(|v| v.as_deref().map(str::trim))
        .find(|v| !v.is_empty());
    vec![first.and_then(posix_to_langid).unwrap_or_else(en)]
}

/// Biçim ayarından sayı locale'i: açık ad çözülemezse ortama düşülür.
fn formats_from(formats: &LocaleSetting, env: &LocaleEnv) -> Locale {
    let from_env = || Locale::from_vars(env.lc_all.as_deref(), env.lc_numeric.as_deref(), env.lang.as_deref());
    match formats.value() {
        Some(name) => Locale::parse(name).unwrap_or_else(from_env),
        None => from_env(),
    }
}

/// `[locale] formats` → tarih/sayı locale'i; `None` = `auto` (çağıran `Locale::from_env`
/// / `numeric_from_env` kullanır). Çözülemeyen ad da `None`.
pub fn formats_locale(formats: &LocaleSetting) -> Option<Locale> {
    formats.value().and_then(Locale::parse)
}

/// `[locale] hour_cycle` → [`Locale::with_12_hour`] değeri: `auto` → `None` (locale verisi),
/// `h12` → `Some(true)`, `h23` → `Some(false)`.
pub fn twelve_hour_override(cycle: HourCycle) -> Option<bool> {
    match cycle {
        HourCycle::Auto => None,
        HourCycle::H12 => Some(true),
        HourCycle::H23 => Some(false),
    }
}

/// `[locale]` → saat/tarih biçimleyen locale: `formats` (`auto` ya da çözülemeyen ad →
/// `Locale::from_env`, `LC_ALL` > `LC_TIME` > `LANG`) + `hour_cycle` geçersiz kılması.
/// `format_hm` / `format_system_time` / `uses_12_hour` bununla 12/24 saate uyar.
pub fn time_locale_for(cfg: &LocaleConfig) -> Locale {
    formats_locale(&cfg.formats)
        .unwrap_or_else(Locale::from_env)
        .with_12_hour(twelve_hour_override(cfg.hour_cycle.cycle))
}

/// POSIX locale adı ya da BCP 47 etiketi → dil kimliği: `tr_TR.UTF-8` → `tr-TR`,
/// `sr_RS@latin` → `sr-Latn-RS`, `pt-BR` → `pt-BR`. `C`, `POSIX`, boş ya da geçersiz → `None`.
pub fn posix_to_langid(s: &str) -> Option<LanguageIdentifier> {
    let s = s.trim();
    if s.is_empty() || s.len() > 64 {
        return None;
    }
    let (base, modifier) = match s.split_once('@') {
        Some((b, m)) => (b, Some(m)),
        None => (s, None),
    };
    let base = base.split('.').next().unwrap_or_default();
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    let mut id = LanguageIdentifier::from_str(&base.replace('_', "-")).ok()?;
    if id.language.is_empty() {
        return None;
    }
    let script = match modifier {
        Some("latin") => Some("Latn"),
        Some("cyrillic") => Some("Cyrl"),
        _ => None,
    };
    if let Some(script) = script.and_then(|s| s.parse().ok()) {
        id.script = Some(script);
    }
    Some(id)
}

/// `$XDG_DATA_HOME/nuxut/locales` (genelde `~/.local/share/nuxut/locales`).
pub fn default_user_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("nuxut").join("locales"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> LocaleEnv {
        let get = |k: &str| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned());
        LocaleEnv {
            nuxut_lang: get("NUXUT_LANG"),
            language: get("LANGUAGE"),
            lc_all: get("LC_ALL"),
            lc_messages: get("LC_MESSAGES"),
            lc_numeric: get("LC_NUMERIC"),
            lang: get("LANG"),
        }
    }

    fn req(language: &str, e: &LocaleEnv) -> Vec<String> {
        LanguagePrefs::resolve(&LocaleSetting::parse(language), &LocaleSetting::AUTO, e)
            .requested()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    #[test]
    fn env_priority() {
        let all = env(&[("LANGUAGE", "de:fr"), ("LC_ALL", "es_ES.UTF-8"), ("LC_MESSAGES", "it_IT"), ("LANG", "tr_TR.UTF-8")]);
        assert_eq!(req("auto", &all), ["de", "fr"]);
        assert_eq!(req("auto", &env(&[("LC_ALL", "es_ES.UTF-8"), ("LC_MESSAGES", "it_IT"), ("LANG", "tr_TR")])), ["es-ES"]);
        assert_eq!(req("auto", &env(&[("LC_MESSAGES", "it_IT"), ("LANG", "tr_TR")])), ["it-IT"]);
        assert_eq!(req("auto", &env(&[("LANG", "tr_TR.UTF-8")])), ["tr-TR"]);
        // Boş değişken atlanır; C/POSIX/boş → en.
        assert_eq!(req("auto", &env(&[("LANGUAGE", ""), ("LC_ALL", ""), ("LANG", "tr_TR")])), ["tr-TR"]);
        assert_eq!(req("auto", &env(&[("LC_ALL", "C"), ("LANG", "tr_TR")])), ["en"]);
        assert_eq!(req("auto", &env(&[("LANG", "POSIX")])), ["en"]);
        assert_eq!(req("auto", &env(&[])), ["en"]);
        // LANGUAGE'ta yalnızca geçersiz girdiler → sonraki değişken.
        assert_eq!(req("auto", &env(&[("LANGUAGE", "C:::"), ("LANG", "de_DE")])), ["de-DE"]);
    }

    #[test]
    fn explicit_setting_beats_env_and_dev_override_beats_all() {
        let e = env(&[("LANG", "tr_TR.UTF-8")]);
        assert_eq!(req("pt-BR", &e), ["pt-BR"]);
        assert_eq!(req("de_AT", &e), ["de-AT"]);
        let e = env(&[("LANG", "tr_TR.UTF-8"), ("NUXUT_LANG", "fr")]);
        assert_eq!(req("de", &e), ["fr"]);
        let p = LanguagePrefs::resolve(&LocaleSetting::parse("de"), &LocaleSetting::AUTO, &env(&[("NUXUT_LANG", "qps")]));
        assert!(p.is_pseudo());
    }

    #[test]
    fn posix_names() {
        let s = |v: &str| posix_to_langid(v).map(|l| l.to_string());
        assert_eq!(s("tr_TR.UTF-8").as_deref(), Some("tr-TR"));
        assert_eq!(s("sr_RS@latin").as_deref(), Some("sr-Latn-RS"));
        assert_eq!(s("de_DE@euro").as_deref(), Some("de-DE"));
        assert_eq!(s("C.UTF-8"), None);
        assert_eq!(s("POSIX"), None);
        assert_eq!(s(""), None);
        assert_eq!(s("!!"), None);
        assert_eq!(s("und"), None);
    }

    #[test]
    fn formats_are_independent() {
        let e = env(&[("LC_NUMERIC", "tr_TR.UTF-8"), ("LANG", "en_US.UTF-8")]);
        let p = LanguagePrefs::resolve(&LocaleSetting::AUTO, &LocaleSetting::AUTO, &e);
        assert_eq!(p.numeric().name(), "tr_TR");
        assert_eq!(p.requested()[0].to_string(), "en-US");
        let p = LanguagePrefs::resolve(&LocaleSetting::AUTO, &LocaleSetting::parse("de_DE"), &e);
        assert_eq!(p.numeric().name(), "de_DE");
        assert_eq!(formats_locale(&LocaleSetting::AUTO), None);
    }

    #[test]
    fn hour_cycle_applies_to_time_locale() {
        let cfg = |formats: &str, cycle: &str| LocaleConfig {
            formats: LocaleSetting::parse(formats),
            hour_cycle: nuxut_config::locale::HourCycleSetting::parse(cycle),
            ..Default::default()
        };
        let hm = |f: &str, c: &str| time_locale_for(&cfg(f, c)).format_hm(13, 5).unwrap();
        assert_eq!(hm("tr_TR", "auto"), "13:05");
        assert_eq!(hm("en_US", "auto"), "1:05 PM");
        assert_eq!(hm("tr_TR", "h23"), "13:05");
        assert_eq!(hm("en_US", "h23"), "13:05");
        assert_eq!(hm("en_US", "h12"), "1:05 PM");
        assert!(time_locale_for(&cfg("tr_TR", "h12")).uses_12_hour());
        // Geçersiz değer → auto (dilden).
        assert_eq!(hm("en_US", "h24"), "1:05 PM");
        assert_eq!(hm("tr_TR", "h24"), "13:05");
        assert_eq!(twelve_hour_override(HourCycle::Auto), None);
    }
}
