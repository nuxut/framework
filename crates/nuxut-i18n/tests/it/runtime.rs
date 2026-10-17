//! Çalışma zamanı: zincir/geri düşüş, çoğul, NUMBER, RTL yalıtımı, sözde dil, kullanıcı
//! dizini (düşmanca dosyalar dahil), çıktı sınırı, başlangıç süresi.

use std::path::PathBuf;
use std::time::Instant;

use nuxut_i18n::{Args, Catalog, Direction, LanguagePrefs, MAX_OUTPUT_BYTES, Source};
use nuxut_locale::Locale;

fn fx() -> Source {
    nuxut_i18n::embed!("fx", "tests/fixtures/good")
}

fn load(langs: &[&str]) -> Catalog {
    Catalog::load(&[fx()], &LanguagePrefs::fixed(langs))
}

fn chain(c: &Catalog) -> Vec<String> {
    c.languages().iter().map(ToString::to_string).collect()
}

/// Test başına geçici dizin (süreç + ad; bitince silinir).
struct Tmp(PathBuf);

impl Tmp {
    fn new(name: &str) -> Self {
        let p = std::env::temp_dir().join(format!("nuxut-i18n-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("fx")).unwrap();
        Self(p)
    }
    fn file(&self, lang: &str) -> PathBuf {
        self.0.join("fx").join(format!("{lang}.ftl"))
    }
    fn catalog(&self, langs: &[&str]) -> Catalog {
        Catalog::load(&[fx()], &LanguagePrefs::fixed(langs).with_user_dir(Some(self.0.clone())))
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn embed_lists_languages_at_compile_time() {
    let src = fx();
    assert_eq!(src.domain, "fx");
    assert_eq!(src.languages().collect::<Vec<_>>(), ["ar", "de", "en", "tr"]);
}

#[test]
fn chain_and_fallback() {
    let de = load(&["de-DE"]);
    assert_eq!(chain(&de), ["de", "en", "tr"]);
    assert_eq!(de.text("fx-hello"), "Hallo");
    // de'de yok → en.
    assert_eq!(de.text("fx-partial"), "Only Turkish and English");
    // Hiçbir dilde yok → anahtar.
    assert_eq!(de.text("fx-nope"), "fx-nope");
    assert!(!de.has("fx-nope") && de.has("fx-partial"));

    let tr = load(&["tr-TR"]);
    assert_eq!(chain(&tr), ["tr", "en"]);
    assert_eq!(tr.text("fx-partial"), "Yalnızca Türkçe ve İngilizce");
    assert_eq!(tr.text("fx-about"), "Nuxut hakkında");

    // Mevcut olmayan dil ve boş istek → en → tr.
    assert_eq!(chain(&load(&["ja"])), ["en", "tr"]);
    assert_eq!(chain(&load(&[])), ["en", "tr"]);
    assert_eq!(load(&["ja"]).text("fx-hello"), "Hello");
    // Birden çok istenen dil sırayla.
    assert_eq!(chain(&load(&["fr", "de"])), ["de", "en", "tr"]);
}

#[test]
fn plurals_tr_en() {
    let mut a = Args::new();
    for (lang, n, want) in [("en", 1, "1 file"), ("en", 5, "5 files"), ("tr", 1, "1 dosya"), ("tr", 5, "5 dosya")] {
        a.set("count", n);
        assert_eq!(load(&[lang]).format("fx-files", &a), want, "{lang} {n}");
    }
}

#[test]
fn number_uses_locale_separators() {
    let mut a = Args::new();
    a.set("n", 1234.5);
    let prefs = LanguagePrefs::fixed(&["tr"]).with_numeric(Locale::parse("tr_TR").unwrap());
    assert_eq!(Catalog::load(&[fx()], &prefs).format("fx-number", &a), "Toplam 1.234,50");
    assert_eq!(load(&["en"]).format("fx-number", &a), "Total 1,234.50");
}

#[test]
fn rtl_isolation_only_in_rtl() {
    let mut a = Args::new();
    a.set("name", "Ada");
    let ar = load(&["ar"]);
    assert_eq!(ar.direction(), Direction::Rtl);
    let out = ar.format("fx-greet", &a);
    assert!(out.contains('\u{2068}') && out.contains('\u{2069}'), "{out:?}");
    let en = load(&["en"]);
    assert_eq!(en.direction(), Direction::Ltr);
    assert_eq!(en.format("fx-greet", &a), "Hello Ada");
}

#[test]
fn attributes() {
    let en = load(&["en"]);
    assert_eq!(en.attr("fx-button", "tooltip", None).as_deref(), Some("Save changes"));
    assert_eq!(en.attr("fx-button", "a11y", None), None);
    assert_eq!(en.attr("fx-nope", "tooltip", None), None);
    // de'de mesaj yok → en'in özniteliği.
    assert_eq!(load(&["de"]).attr("fx-button", "tooltip", None).as_deref(), Some("Save changes"));
}

#[test]
fn missing_variable_falls_through_but_never_panics() {
    let out = load(&["en"]).format("fx-greet", &Args::new()).into_owned();
    assert!(out.starts_with("Hello"), "{out}");
}

#[test]
fn pseudo_language() {
    let c = Catalog::load(&[fx()], &LanguagePrefs::fixed(&["en"]).with_pseudo(true));
    assert!(c.is_pseudo());
    let out = c.text("fx-button");
    assert!(out.starts_with("[Ŝåṽé") && out.ends_with("··]"), "{out}");
    let mut a = Args::new();
    a.set("name", "Ada");
    // Değişken aksanlanmaz.
    assert!(c.format("fx-greet", &a).contains("Ada"));
}

#[test]
fn user_file_overrides_embedded() {
    let t = Tmp::new("override");
    std::fs::write(t.file("en"), "fx-hello = Howdy\nthis is junk {{{\nfx-greet = Hi { $name }\n").unwrap();
    let c = t.catalog(&["en"]);
    assert_eq!(c.text("fx-hello"), "Howdy");
    let mut a = Args::new();
    a.set("name", "Ada");
    assert_eq!(c.format("fx-greet", &a), "Hi Ada");
    // Dokunulmayan anahtar gömülüden.
    assert_eq!(c.text("fx-partial"), "Only Turkish and English");
}

#[test]
fn user_dir_adds_a_new_language() {
    let t = Tmp::new("newlang");
    std::fs::write(t.file("fr"), "fx-hello = Bonjour\n").unwrap();
    let c = t.catalog(&["fr-FR"]);
    assert_eq!(chain(&c), ["fr", "en", "tr"]);
    assert_eq!(c.text("fx-hello"), "Bonjour");
    assert_eq!(c.text("fx-partial"), "Only Turkish and English");
}

fn assert_ignored(t: &Tmp) {
    let c = t.catalog(&["en"]);
    assert_eq!(c.text("fx-hello"), "Hello");
}

#[cfg(unix)]
#[test]
fn hostile_symlink_is_not_followed() {
    let t = Tmp::new("symlink");
    let target = t.0.join("target.ftl");
    std::fs::write(&target, "fx-hello = Pwned\n").unwrap();
    std::os::unix::fs::symlink(&target, t.file("en")).unwrap();
    assert_ignored(&t);
}

#[cfg(unix)]
#[test]
fn hostile_fifo_does_not_block() {
    use rustix::fs::{CWD, FileType, Mode, mknodat};
    let t = Tmp::new("fifo");
    mknodat(CWD, t.file("en"), FileType::Fifo, Mode::from_raw_mode(0o600), 0).unwrap();
    let start = Instant::now();
    assert_ignored(&t);
    assert!(start.elapsed().as_secs() < 2);
}

#[test]
fn hostile_directory_huge_and_binary_files() {
    let t = Tmp::new("dir");
    std::fs::create_dir(t.file("en")).unwrap();
    assert_ignored(&t);

    let t = Tmp::new("huge");
    let mut big = String::from("fx-hello = Big\n");
    big.push_str(&"# dolgu\n".repeat((nuxut_i18n::MAX_USER_FILE_BYTES as usize) / 8 + 1));
    std::fs::write(t.file("en"), big).unwrap();
    assert_ignored(&t);

    let t = Tmp::new("binary");
    std::fs::write(t.file("en"), b"fx-hello = \xff\xfe\n").unwrap();
    assert_ignored(&t);
}

#[test]
fn hostile_too_many_messages() {
    let t = Tmp::new("many");
    let mut s = String::from("fx-hello = Many\n");
    for i in 0..nuxut_i18n::MAX_USER_MESSAGES {
        s.push_str(&format!("fx-m{i} = x\n"));
    }
    std::fs::write(t.file("en"), s).unwrap();
    assert_ignored(&t);
}

#[test]
fn hostile_cycle_falls_back() {
    let t = Tmp::new("cycle");
    std::fs::write(t.file("en"), "fx-hello = { fx-a }\nfx-a = { fx-b }\nfx-b = { fx-hello }\n").unwrap();
    let c = t.catalog(&["en"]);
    // en'de döngü → çözümleme hatası → zincirde tr.
    assert_eq!(c.text("fx-hello"), "Merhaba");
}

#[test]
fn hostile_output_is_clipped() {
    let t = Tmp::new("long");
    let mut s = format!("fx-big = {}\n", "ab".repeat(3000));
    s.push_str("fx-hello =");
    for _ in 0..90 {
        s.push_str(" { fx-big }");
    }
    s.push('\n');
    std::fs::write(t.file("en"), s).unwrap();
    let c = t.catalog(&["en"]);
    let out = c.text("fx-hello");
    assert!(out.len() <= MAX_OUTPUT_BYTES && out.len() > 4000, "{}", out.len());
    // Kendi başına uzun mesaj da sınırlı.
    assert!(c.text("fx-big").len() <= MAX_OUTPUT_BYTES);
}

/// ADR bütçesi: 2000 mesaj < 5 ms (release). Debug'da yalnızca raporlanır.
#[test]
fn startup_time_2000_messages() {
    let mut tr = String::new();
    let mut en = String::new();
    for i in 0..2000 {
        tr.push_str(&format!("# Nerede: test. Değişkenler: $n (sayı).\nfx-msg-{i} = İleti {{ $n }} numara {i}\n"));
        en.push_str(&format!("# Nerede: test. Değişkenler: $n (sayı).\nfx-msg-{i} = Message {{ $n }} number {i}\n"));
    }
    let tr: &'static str = Box::leak(tr.into_boxed_str());
    let en: &'static str = Box::leak(en.into_boxed_str());
    let embedded: &'static [(&'static str, &'static str)] = Box::leak(vec![("en", en), ("tr", tr)].into_boxed_slice());
    let src = Source { domain: "fx", embedded };
    let prefs = LanguagePrefs::fixed(&["tr"]);
    let _ = Catalog::load(&[src], &prefs); // ısınma
    let runs = 5;
    let start = Instant::now();
    for _ in 0..runs {
        let c = Catalog::load(&[src], &prefs);
        assert!(c.has("fx-msg-1999"));
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
    println!("Catalog::load 2000 mesaj × 2 dil: {ms:.2} ms");
    #[cfg(not(debug_assertions))]
    assert!(ms < 5.0, "başlangıç bütçesi aşıldı: {ms:.2} ms");
}

