//! `framework.toml [locale]` şeması (ADR-0010 §3). Hem tema (`FrameworkConfig`) hem
//! `nuxut-i18n` bu tipi okur; tek tanım burada.
//!
//! ```toml
//! [locale]
//! language = "auto"   # "auto" | BCP 47 etiketi ("tr", "en", "pt-BR")
//! formats  = "auto"   # "auto" | POSIX locale adı ("tr_TR", "en_US.UTF-8") — tarih/sayı biçimi
//! hour_cycle = "auto" # "auto" (biçim locale'inden) | "h23" | "h12" — saat:dakika 12/24 saat
//! ```
//!
//! Dosyayı bozmaz: yanlış tür, boş ya da sözdizimi geçersiz değer `auto`'ya döner ve
//! [`LocaleSetting::invalid`] / [`HourCycleSetting::invalid`] işaretlenir (tema doğrulayıcısı uyarı verir; değer loga yazılmaz).

use serde::{Deserialize, Serialize};

/// Bir ayarın kabul edilen en uzun hali (bayt).
pub const MAX_LOCALE_SETTING: usize = 64;

/// `[locale]` bölümü.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct LocaleConfig {
    /// Arayüz dili.
    pub language: LocaleSetting,
    /// Tarih/saat ve sayı biçimi (`nuxut-locale`); dil ile bağımsız.
    pub formats: LocaleSetting,
    /// Saat:dakika 12/24 saat seçimi; `auto` = `formats` locale'inin verisi. Uygulamanın açık
    /// strftime kalıbı (ör. shell `clock.format`) bunu geçersiz kılar.
    pub hour_cycle: HourCycleSetting,
}

/// `"auto"` ya da sözdizimi geçerli bir ad. Ayrıntılı çözüm (etiket gerçekten tanınıyor mu)
/// tüketicidedir: `nuxut-i18n` dil etiketini, `nuxut-locale` biçim adını çözer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocaleSetting {
    /// `None` = `auto`.
    value: Option<String>,
    /// Dosyada geçersiz bir değer vardı (doğrulama uyarısı için; değer saklanmaz).
    pub invalid: bool,
}

impl LocaleSetting {
    /// `auto`.
    pub const AUTO: Self = Self { value: None, invalid: false };

    /// Metinden: boş ya da `auto` → auto; sözdizimi geçersiz → auto + `invalid`.
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("auto") {
            return Self::AUTO;
        }
        if valid_setting(s) {
            Self { value: Some(s.to_owned()), invalid: false }
        } else {
            Self { value: None, invalid: true }
        }
    }

    /// `true` = ortamdan seç.
    pub fn is_auto(&self) -> bool {
        self.value.is_none()
    }

    /// Dosyadaki ad: `"auto"` ya da değer.
    pub fn as_str(&self) -> &str {
        self.value.as_deref().unwrap_or("auto")
    }

    /// Açık değer (`auto` ise `None`).
    pub fn value(&self) -> Option<&str> {
        self.value.as_deref()
    }
}

/// `[A-Za-z][A-Za-z0-9_.@-]*`, ≤ [`MAX_LOCALE_SETTING`] bayt: BCP 47 etiketi ve POSIX locale
/// adının ortak sözdizimi (dış ayrıştırıcılara güvenle verilir).
fn valid_setting(s: &str) -> bool {
    s.len() <= MAX_LOCALE_SETTING
        && s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'@' | b'-'))
}

impl Serialize for LocaleSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for LocaleSetting {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = toml::Value::deserialize(de)?;
        Ok(match raw.as_str() {
            Some(s) => Self::parse(s),
            None => Self { value: None, invalid: true },
        })
    }
}

/// `[locale] hour_cycle`: saat:dakika gösteriminin 12/24 saat seçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HourCycle {
    /// Biçim locale'inin verisinden (tr_TR → 24 saat, en_US → 12 saat).
    #[default]
    Auto,
    /// 24 saat ("13:05").
    H23,
    /// 12 saat ("1:05 PM").
    H12,
}

impl HourCycle {
    /// Dosyadaki ad: `"auto"`, `"h23"`, `"h12"`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::H23 => "h23",
            Self::H12 => "h12",
        }
    }

    /// Addan (büyük/küçük harf ve kenar boşluğu önemsiz); bilinmeyen → `None`.
    pub fn from_name(s: &str) -> Option<Self> {
        let s = s.trim();
        [Self::Auto, Self::H23, Self::H12].into_iter().find(|c| c.as_str().eq_ignore_ascii_case(s))
    }
}

/// `hour_cycle` alanı: değer + dosyada geçersiz değer vardı mı (yalnızca o anahtar `auto`'ya döner).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HourCycleSetting {
    pub cycle: HourCycle,
    /// Dosyada geçersiz bir değer vardı (doğrulama uyarısı için; değer saklanmaz).
    pub invalid: bool,
}

impl HourCycleSetting {
    /// `auto`.
    pub const AUTO: Self = Self { cycle: HourCycle::Auto, invalid: false };

    /// Metinden: boş → auto; bilinmeyen → auto + `invalid`.
    pub fn parse(s: &str) -> Self {
        if s.trim().is_empty() {
            return Self::AUTO;
        }
        match HourCycle::from_name(s) {
            Some(cycle) => Self { cycle, invalid: false },
            None => Self { cycle: HourCycle::Auto, invalid: true },
        }
    }
}

impl From<HourCycle> for HourCycleSetting {
    fn from(cycle: HourCycle) -> Self {
        Self { cycle, invalid: false }
    }
}

impl Serialize for HourCycleSetting {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.cycle.as_str())
    }
}

impl<'de> Deserialize<'de> for HourCycleSetting {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = toml::Value::deserialize(de)?;
        Ok(match raw.as_str() {
            Some(s) => Self::parse(s),
            None => Self { cycle: HourCycle::Auto, invalid: true },
        })
    }
}

/// Yalnızca `[locale]`'i okuyan kısmi `framework.toml` görünümü (diğer bölümler yok sayılır).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LocaleOnly {
    pub locale: LocaleConfig,
}

/// `<config_dir>/framework.toml`'daki `[locale]`; dosya yok/bozuk → varsayılan (`auto`).
/// Bozuk dosyanın uyarısını tema yükleyicisi verir; burada yalnızca hata türü loglanır.
pub fn load_locale_config() -> LocaleConfig {
    let Some(path) = crate::config_path("framework") else {
        return LocaleConfig::default();
    };
    match crate::load::<LocaleOnly>(&path) {
        Ok(loaded) => loaded.value.locale,
        Err(e) => {
            tracing::debug!(error = %e.log_summary(), "framework.toml [locale] okunamadı; auto");
            LocaleConfig::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auto_and_values() {
        let (c, _): (LocaleOnly, Vec<String>) =
            crate::parse_str("[locale]\nlanguage = \"pt-BR\"\nformats = \"tr_TR.UTF-8\"\n").unwrap();
        assert_eq!(c.locale.language.value(), Some("pt-BR"));
        assert_eq!(c.locale.formats.as_str(), "tr_TR.UTF-8");
        assert!(!c.locale.language.invalid);
        let (c, _): (LocaleOnly, Vec<String>) = crate::parse_str("[locale]\nlanguage = \" AUTO \"\n").unwrap();
        assert!(c.locale.language.is_auto() && !c.locale.language.invalid);
        assert!(c.locale.formats.is_auto());
    }

    #[test]
    fn invalid_values_fall_back_to_auto() {
        for bad in ["\"tr; rm -rf\"", "42", "\"../../etc\"", "[\"tr\"]", "\"ğ\""] {
            let (c, _): (LocaleOnly, Vec<String>) =
                crate::parse_str(&format!("[locale]\nlanguage = {bad}\n")).unwrap();
            assert!(c.locale.language.is_auto(), "{bad}");
            assert!(c.locale.language.invalid, "{bad}");
        }
        let long = "a".repeat(MAX_LOCALE_SETTING + 1);
        assert!(LocaleSetting::parse(&long).invalid);
    }

    #[test]
    fn serializes_as_name() {
        let c = LocaleConfig { language: LocaleSetting::parse("de"), formats: LocaleSetting::AUTO, ..Default::default() };
        let text = toml::to_string(&c).unwrap();
        assert!(text.contains("language = \"de\"") && text.contains("formats = \"auto\""), "{text}");
        assert!(text.contains("hour_cycle = \"auto\""), "{text}");
        let c = LocaleConfig { hour_cycle: HourCycle::H12.into(), ..Default::default() };
        assert!(toml::to_string(&c).unwrap().contains("hour_cycle = \"h12\""));
    }

    #[test]
    fn hour_cycle_values() {
        let cycle = |v: &str| {
            let (c, _): (LocaleOnly, Vec<String>) = crate::parse_str(&format!("[locale]\nhour_cycle = {v}\n")).unwrap();
            c.locale.hour_cycle
        };
        assert_eq!(cycle("\"auto\""), HourCycleSetting::AUTO);
        assert_eq!(cycle("\"h23\""), HourCycle::H23.into());
        assert_eq!(cycle("\" H12 \""), HourCycle::H12.into());
        let (c, _): (LocaleOnly, Vec<String>) = crate::parse_str("[locale]\nformats = \"tr_TR\"\n").unwrap();
        assert_eq!(c.locale.hour_cycle, HourCycleSetting::AUTO, "yazılmamış → auto");
        // Geçersiz değer yalnızca bu anahtarı düşürür: auto + invalid, diğer anahtarlar korunur.
        for bad in ["\"h24\"", "\"k\"", "12", "true", "[\"h12\"]"] {
            let (c, _): (LocaleOnly, Vec<String>) =
                crate::parse_str(&format!("[locale]\nformats = \"de_DE\"\nhour_cycle = {bad}\n")).unwrap();
            assert_eq!(c.locale.hour_cycle.cycle, HourCycle::Auto, "{bad}");
            assert!(c.locale.hour_cycle.invalid, "{bad}");
            assert_eq!(c.locale.formats.value(), Some("de_DE"), "{bad}");
        }
        assert_eq!(HourCycle::from_name("H23"), Some(HourCycle::H23));
        assert_eq!(HourCycle::from_name("h11"), None);
    }
}
