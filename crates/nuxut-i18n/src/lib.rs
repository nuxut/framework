//! Nuxut uluslararasılaştırma (ADR-0010): Fluent katalogları.
//!
//! - **Kaynak:** her repo `locales/<dil>.ftl` dosyalarını [`embed!`] ile gömer ([`Source`]).
//! - **Katalog:** [`Catalog::load`] istenen dilleri mevcut dosyalara eşler, zinciri
//!   `istenen → en → tr` (tekrarsız) kurar, kullanıcı dizinindeki dosyaları gömülünün üstüne
//!   yazar. **Asla başarısız olmaz:** bozuk/eksik çeviri zincirde sonraki dile, en kötü
//!   anahtarın kendisine düşer (anahtar başına tek uyarı).
//! - **Süreç geneli:** [`set_current`]/[`current`] (okuma kilitsiz: iş parçacığı önbelleği +
//!   nesil sayacı) ve [`t!`] makrosu. [`init`]/[`configure`] kayıtlı kaynaklardan kurar;
//!   Wayland runtime'ı `framework.toml [locale]` değişince [`configure`] eder ve uygulamaya
//!   `App::locale_changed` gönderir.
//! - **Denetim:** [`check::catalog`] (katalog kapıları) ve [`check::source_guard`] (gömülü
//!   kullanıcı metni bekçisi); her repo tek `#[test]` ile çağırır.
//!
//! Çeviri düz metindir; hiçbir bileşen çevrilmiş metni biçimlendirme/bağlantı olarak
//! yorumlamaz. Biçimlenmiş çıktı en çok [`MAX_OUTPUT_BYTES`].

#![forbid(unsafe_code)]

pub mod check;
mod global;
mod languages;
mod number;
mod prefs;
mod pseudo;
mod user;
pub mod watch;

use std::borrow::Cow;
use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Mutex;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentError, FluentResource};
use unicode_segmentation::UnicodeSegmentation;

pub use fluent_bundle::{FluentArgs as Args, FluentValue};
pub use global::{configure, current, init, numeric_locale, register, registered, reload, set_current, set_register_hook, time_locale};
pub use languages::{available_languages, available_languages_in, language_label, language_locale};
pub use nuxut_config::locale::{HourCycle, HourCycleSetting, LocaleConfig, LocaleSetting};
pub use prefs::{LanguagePrefs, LocaleEnv, default_user_dir, formats_locale, posix_to_langid, time_locale_for, twelve_hour_override};
pub use unic_langid::LanguageIdentifier;
pub use user::{MAX_USER_FILE_BYTES, MAX_USER_MESSAGES};

#[doc(hidden)]
pub use nuxut_i18n_macros::__embed;

/// Biçimlenmiş bir metnin en büyük boyu (bayt); aşan çıktı grafem sınırında kırpılır.
pub const MAX_OUTPUT_BYTES: usize = 4096;

/// Uyarı tekilleştirme kümesinin sınırı (anahtar başına tek uyarı; küme sınırsız büyümez).
const MAX_WARNED: usize = 4096;

/// Bir domain'in (repo) gömülü katalogları: `(dil etiketi, .ftl içeriği)`. [`embed!`] üretir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// Kullanıcı dizinindeki alt dizin adı (`$XDG_DATA_HOME/nuxut/locales/<domain>/`).
    pub domain: &'static str,
    /// Dil etiketine göre sıralı.
    pub embedded: &'static [(&'static str, &'static str)],
}

impl Source {
    /// Gömülü dil etiketleri.
    pub fn languages(&self) -> impl Iterator<Item = &'static str> {
        self.embedded.iter().map(|(lang, _)| *lang)
    }
}

/// Çağıran crate'in `locales/*.ftl` dosyalarını gömer → [`Source`]. Dil listesi derleme
/// zamanında dizinden çıkarılır; içerik `include_str!` ile (değişince yeniden derlenir).
///
/// ```ignore
/// let src = nuxut::i18n::embed!("settings");            // <crate>/locales/*.ftl
/// let src = nuxut::i18n::embed!("ui", "../../locales"); // crate köküne göreli dizin
/// ```
///
/// Yeni bir `<dil>.ftl` eklemek mevcut dosyaları değiştirmediği için yeniden derlemeyi
/// tetiklemez; makroyu çağıran dosyaya dokunun ([`check::catalog_embedded`] yakalar).
#[macro_export]
macro_rules! embed {
    ($domain:literal $(,)?) => {
        $crate::__embed!($crate, $domain)
    };
    ($domain:literal, $dir:literal $(,)?) => {
        $crate::__embed!($crate, $domain, $dir)
    };
}

/// Süreç geneli katalogdan metin ([`current`]): `t!("shell-bar-clock")`,
/// `t!("shell-bar-battery-tooltip", percent = 80, time = s.as_str())`. Değer
/// [`FluentValue`]'ya dönüşen her şey (sayılar, `&str`, `String`). Sonuç `String`.
#[macro_export]
macro_rules! t {
    ($id:literal $(,)?) => {
        $crate::current().text($id).into_owned()
    };
    ($id:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::Args::new();
        $( args.set(::core::stringify!($name), $value); )+
        $crate::current().format($id, &args).into_owned()
    }};
}

/// Yazı yönü (zincirin ilk dilinden). Yeni API'ler Left/Right değil Start/End kullanır;
/// metin hizası buradan okunur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

/// Yüklü katalog: zincirdeki her dil için bir Fluent paketi. `Send + Sync`; `Arc` ile
/// paylaşılır, değişmez (dil değişince yenisi kurulur).
pub struct Catalog {
    chain: Vec<LanguageIdentifier>,
    bundles: Vec<FluentBundle<FluentResource>>,
    direction: Direction,
    pseudo: bool,
    symbols: number::Symbols,
    numeric: nuxut_locale::Locale,
    warned: Mutex<HashSet<String>>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog")
            .field("chain", &self.chain.iter().map(ToString::to_string).collect::<Vec<_>>())
            .field("direction", &self.direction)
            .field("pseudo", &self.pseudo)
            .finish_non_exhaustive()
    }
}

/// Zincirin zorunlu kuyruğu: önce `en`, en sona `tr` (kaynak dil, her zaman tam).
const FALLBACKS: [&str; 2] = ["en", "tr"];

impl Catalog {
    /// Kaynaklardan ve tercihlerden kurar. **Asla başarısız olmaz:** bozuk gömülü dosya
    /// (geliştirme hatası, katalog testi yakalar) ya da kullanıcı dosyası atlanır, uyarı
    /// loglanır (içerik ve yol yazılmaz).
    pub fn load(sources: &[Source], prefs: &LanguagePrefs) -> Self {
        // Mevcut diller: gömülü ∪ kullanıcı dizini.
        let available = languages::collect(sources, prefs.user_dir());
        let chain = resolve_chain(prefs.requested(), &available);
        let direction = match chain.first().map(|l| l.character_direction()) {
            Some(unic_langid::CharacterDirection::RTL) if !prefs.is_pseudo() => Direction::Rtl,
            _ => Direction::Ltr,
        };

        let bundles = chain
            .iter()
            .map(|lang| {
                let mut bundle = FluentBundle::new_concurrent(vec![lang.clone()]);
                // FSI/PDI yalnızca RTL'de: LTR'de görünmez karakter yok, ölçüm değişmez.
                bundle.set_use_isolating(direction == Direction::Rtl);
                bundle.set_formatter(Some(number::formatter));
                if prefs.is_pseudo() {
                    bundle.set_transform(Some(pseudo::transform));
                }
                // NUMBER(): seçenekleri birleştirir; biçim `number::formatter`'da (nuxut-locale).
                if bundle.add_builtins().is_err() {
                    tracing::debug!("fluent yerleşik işlevleri eklenemedi");
                }
                for src in sources {
                    add_embedded(&mut bundle, src, lang);
                    if let Some(dir) = prefs.user_dir() {
                        user::apply(&mut bundle, dir, src.domain, lang);
                    }
                }
                bundle
            })
            .collect();

        Self {
            chain,
            bundles,
            direction,
            pseudo: prefs.is_pseudo(),
            symbols: prefs.numeric().number_symbols(),
            numeric: prefs.numeric().clone(),
            warned: Mutex::new(HashSet::new()),
        }
    }

    /// Kaynaksız katalog: her istek anahtarın kendisini döndürür.
    pub fn empty() -> Self {
        Self::load(&[], &LanguagePrefs::fixed(&[]))
    }

    /// Değişkensiz metin.
    pub fn text(&self, id: &str) -> Cow<'_, str> {
        self.resolve(id, None, None).unwrap_or_else(|| self.missing(id))
    }

    /// `$değişkenli` metin.
    pub fn format(&self, id: &str, args: &Args<'_>) -> Cow<'_, str> {
        self.resolve(id, None, Some(args)).unwrap_or_else(|| self.missing(id))
    }

    /// Mesaj özniteliği (`.tooltip`, `.a11y`). Zincirin hiçbir dilinde yoksa `None`
    /// (+ tek uyarı).
    pub fn attr(&self, id: &str, attr: &str, args: Option<&Args<'_>>) -> Option<Cow<'_, str>> {
        let out = self.resolve(id, Some(attr), args);
        if out.is_none() {
            self.warn_once(&format!("{id}.{attr}"), "çeviri özniteliği hiçbir dilde yok");
        }
        out
    }

    /// Mesaj zincirin herhangi bir dilinde var mı.
    pub fn has(&self, id: &str) -> bool {
        self.bundles.iter().any(|b| b.has_message(id))
    }

    /// Çözülmüş zincir (istenen → en → tr).
    pub fn languages(&self) -> &[LanguageIdentifier] {
        &self.chain
    }

    /// Zincirin ilk dilinin yazı yönü.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// Sayı locale'i (`[locale] formats` ya da ortam): `NUMBER` / `{ $n }` bununla biçimlenir.
    /// Uygulamanın kendi biçimlediği sayılar (`format_percent`, `format_number`) da bunu kullanır.
    pub fn numeric(&self) -> &nuxut_locale::Locale {
        &self.numeric
    }

    /// Sözde dil (`NUXUT_LANG=qps`) etkin mi.
    pub fn is_pseudo(&self) -> bool {
        self.pseudo
    }

    /// Zincirde ilk bulunan dilden biçimler. Çözümleme hatası (eksik değişken, kırık
    /// referans) olan dil atlanır; hiçbiri temiz değilse ilk sonuç kullanılır.
    fn resolve(&self, id: &str, attr: Option<&str>, args: Option<&Args<'_>>) -> Option<Cow<'_, str>> {
        let mut first: Option<Cow<'_, str>> = None;
        number::scoped(self.symbols, || {
            for bundle in &self.bundles {
                let Some(msg) = bundle.get_message(id) else { continue };
                let pattern = match attr {
                    None => msg.value(),
                    Some(a) => msg.get_attribute(a).map(|a| a.value()),
                };
                let Some(pattern) = pattern else { continue };
                let mut errors: Vec<FluentError> = Vec::new();
                let out = bundle.format_pattern(pattern, args, &mut errors);
                if errors.is_empty() {
                    first = Some(out);
                    return;
                }
                self.warn_once(id, "çeviri çözümleme hatası; zincirde sonraki dil denendi");
                first.get_or_insert(out);
            }
        });
        first.map(|s| self.finish(s))
    }

    fn finish<'a>(&self, s: Cow<'a, str>) -> Cow<'a, str> {
        let s = if self.pseudo { Cow::Owned(pseudo::wrap(&s)) } else { s };
        clip(s)
    }

    fn missing(&self, id: &str) -> Cow<'_, str> {
        self.warn_once(id, "çeviri anahtarı hiçbir dilde yok; anahtar gösteriliyor");
        clip(Cow::Owned(id.to_owned()))
    }

    /// Anahtar başına tek uyarı. Anahtar koddaki sabittir (kullanıcı içeriği değil).
    fn warn_once(&self, key: &str, msg: &'static str) {
        let Ok(mut warned) = self.warned.lock() else { return };
        if warned.len() >= MAX_WARNED || warned.contains(key) {
            return;
        }
        warned.insert(key.to_owned());
        drop(warned);
        tracing::warn!(key, "{msg}");
    }
}

/// `istenen (mevcut dosyalara eşlenmiş) → en → tr`, tekrarsız.
fn resolve_chain(requested: &[LanguageIdentifier], available: &[LanguageIdentifier]) -> Vec<LanguageIdentifier> {
    let negotiated = fluent_langneg::negotiate_languages(
        requested,
        available,
        None,
        fluent_langneg::NegotiationStrategy::Filtering,
    );
    let mut chain: Vec<LanguageIdentifier> = Vec::new();
    let fallbacks = FALLBACKS.iter().filter_map(|t| LanguageIdentifier::from_str(t).ok());
    for lang in negotiated.into_iter().cloned().chain(fallbacks) {
        if !chain.contains(&lang) {
            chain.push(lang);
        }
    }
    chain
}

/// Gömülü `<lang>.ftl`'i pakete ekler. Ayrıştırma hatalı girdiler atlanır (geri kalan
/// kullanılır); tekrar eden kimlikte ilk kazanır.
fn add_embedded(bundle: &mut FluentBundle<FluentResource>, src: &Source, lang: &LanguageIdentifier) {
    let Some((_, text)) = src
        .embedded
        .iter()
        .find(|(tag, _)| LanguageIdentifier::from_str(tag).is_ok_and(|t| &t == lang))
    else {
        return;
    };
    let res = match FluentResource::try_new((*text).to_owned()) {
        Ok(res) => res,
        Err((res, errors)) => {
            tracing::warn!(domain = src.domain, %lang, count = errors.len(), "gömülü katalogda ayrıştırma hatası");
            res
        }
    };
    if let Err(errors) = bundle.add_resource(res) {
        tracing::warn!(domain = src.domain, %lang, count = errors.len(), "gömülü katalogda tekrar eden kimlik");
    }
}

/// En çok [`MAX_OUTPUT_BYTES`]; grafem sınırında kırpar.
fn clip(s: Cow<'_, str>) -> Cow<'_, str> {
    if s.len() <= MAX_OUTPUT_BYTES {
        return s;
    }
    let end = s
        .grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .take_while(|end| *end <= MAX_OUTPUT_BYTES)
        .last()
        .unwrap_or(0);
    Cow::Owned(s[..end].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn catalog_is_send_sync() {
        assert_send_sync::<Catalog>();
    }

    #[test]
    fn clip_respects_graphemes() {
        let s = "e\u{301}".repeat(MAX_OUTPUT_BYTES); // her grafem 3 bayt
        let out = clip(Cow::Owned(s));
        assert!(out.len() <= MAX_OUTPUT_BYTES);
        assert_eq!(out.len() % 3, 0);
        assert!(matches!(clip(Cow::Borrowed("kısa")), Cow::Borrowed(_)));
    }

    #[test]
    fn chain_has_no_duplicates() {
        let ids = |v: &[&str]| v.iter().map(|s| s.parse().unwrap()).collect::<Vec<LanguageIdentifier>>();
        let chain = resolve_chain(&ids(&["tr-TR", "en"]), &ids(&["en", "tr", "de"]));
        assert_eq!(chain, ids(&["tr", "en"]));
        let chain = resolve_chain(&ids(&["de-AT"]), &ids(&["en", "tr", "de"]));
        assert_eq!(chain, ids(&["de", "en", "tr"]));
        let chain = resolve_chain(&ids(&["ja"]), &ids(&["en", "tr"]));
        assert_eq!(chain, ids(&["en", "tr"]));
    }
}
