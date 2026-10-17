//! Kullanılabilir arayüz dilleri (ayarlar "Dil" seçimi) ve gösterim adları.
//!
//! Kaynak: kayıtlı [`Source`]'ların gömülü dilleri ∪ kullanıcı çeviri dizinindeki diller
//! (`<kök>/<domain>/<dil>.ftl`; [`crate::user`] ile aynı güvenlik kuralları: dizin taraması
//! sınırlı, geçersiz ad atlanır, dosya içeriği okunmaz). Sözde dil (`qps`) listelenmez.

use std::path::Path;
use std::str::FromStr;

use nuxut_locale::Locale;
use unic_langid::LanguageIdentifier;

use crate::{Source, user};

/// Kaynakların dilleri, ilk görülme sırasıyla, tekrarsız (katalog dil eşlemesi bu sırayı
/// kullanır).
pub(crate) fn collect(sources: &[Source], user_dir: Option<&Path>) -> Vec<LanguageIdentifier> {
    let mut out: Vec<LanguageIdentifier> = Vec::new();
    let mut add = |tag: &str| {
        if let Ok(id) = LanguageIdentifier::from_str(tag)
            && !out.contains(&id)
        {
            out.push(id);
        }
    };
    for src in sources {
        src.languages().for_each(&mut add);
        if let Some(dir) = user_dir {
            user::languages(&dir.join(src.domain)).iter().for_each(|l| add(l));
        }
    }
    out
}

/// [`available_languages`]'ın saf hâli: verilen kaynaklar ve kullanıcı kökü. Kararlı sıra
/// (BCP 47 etiketine göre), tekrarsız; dil alt etiketi boş olan (`und`) atlanır.
pub fn available_languages_in(sources: &[Source], user_dir: Option<&Path>) -> Vec<LanguageIdentifier> {
    let mut out: Vec<LanguageIdentifier> =
        collect(sources, user_dir).into_iter().filter(|id| !id.language.is_empty()).collect();
    out.sort_by_cached_key(ToString::to_string);
    out
}

/// Seçilebilir arayüz dilleri: kayıtlı kaynakların ([`crate::register`] / [`crate::init`])
/// gömülü dilleri ∪ kullanıcı dizinindeki ([`crate::default_user_dir`]) diller. Gösterim adı
/// için [`language_label`].
pub fn available_languages() -> Vec<LanguageIdentifier> {
    available_languages_in(&crate::registered(), crate::default_user_dir().as_deref())
}

/// Dil etiketinin biçim locale'i (`tr` → `tr_TR`, `en` → `en_US`, `pt-BR` → `pt_BR`,
/// `sr-Latn` → `sr_RS@latin`); gömülü veride karşılığı yoksa `None`.
pub fn language_locale(id: &LanguageIdentifier) -> Option<Locale> {
    Locale::from_language_tag(&id.to_string())
}

/// Dilin kendi dilindeki gösterim adı: yalnızca dil → [`Locale::language_name`] ("Türkçe",
/// "English"); bölge ya da yazı belirtilmişse [`Locale::display_name`] ("Português
/// (Brasil)"). Karşılığı olmayan etiket olduğu gibi döner.
pub fn language_label(id: &LanguageIdentifier) -> String {
    match language_locale(id) {
        Some(l) if id.region.is_some() || id.script.is_some() => l.display_name(),
        Some(l) => l.language_name(),
        None => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI: Source = Source { domain: "ui", embedded: &[("en", ""), ("tr", "")] };
    const APP: Source = Source { domain: "app", embedded: &[("de", ""), ("en", ""), ("pt-BR", ""), ("und", "")] };

    fn ids(v: &[LanguageIdentifier]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn embedded_languages_sorted_unique() {
        assert_eq!(ids(&available_languages_in(&[UI, APP], None)), ["de", "en", "pt-BR", "tr"]);
        assert!(available_languages_in(&[], None).is_empty());
    }

    #[test]
    fn user_dir_languages_join_safely() {
        let root = std::env::temp_dir().join(format!("nuxut-i18n-langs-{}", std::process::id()));
        let dir = root.join("ui");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["ja.ftl", "tr.ftl", "bozuk ad.ftl", "notes.txt", "-.ftl"] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("ja.ftl"), dir.join("ko.ftl")).unwrap();
        let got = ids(&available_languages_in(&[UI], Some(&root)));
        std::fs::remove_dir_all(&root).unwrap();
        // Geçersiz adlar ve symlink (yüklemede O_NOFOLLOW ile reddedilir) atlanır; tekrar yok.
        assert_eq!(got, ["en", "ja", "tr"]);
    }

    #[test]
    fn labels_in_own_language() {
        let label = |t: &str| language_label(&t.parse().unwrap());
        assert_eq!(label("tr"), "Türkçe");
        assert_eq!(label("en"), "English");
        assert_eq!(label("de"), "Deutsch");
        assert_eq!(label("ja"), "日本語");
        assert_eq!(label("pt-BR"), "Português (Brasil)");
        assert_eq!(label("en-US"), "English (United States)");
        assert_eq!(label("sr-Latn"), "Srpski (Srbija)");
        assert_eq!(label("qps"), "qps");
        assert_eq!(language_locale(&"tr".parse().unwrap()).map(|l| l.name().to_owned()).as_deref(), Some("tr_TR"));
    }
}
