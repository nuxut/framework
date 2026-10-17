//! Süreç geneli katalog: `set_current`/`current`, `t!`, iş parçacıkları arası görünürlük,
//! `register`/`configure`. Süreç geneli durum paylaşıldığı için tek test (ayrı ikili).

use std::sync::Arc;

use nuxut_i18n::{Catalog, HourCycle, LanguagePrefs, LocaleConfig, LocaleSetting, t};

#[test]
fn process_wide_catalog() {
    // Kurulmadan sayı locale'i ortamdan (Locale::numeric_from_env ile aynı).
    assert_eq!(nuxut_i18n::numeric_locale(), nuxut_locale::Locale::numeric_from_env());
    assert_eq!(nuxut_i18n::current().numeric().name(), "en_US");

    // Kurulmadan: boş katalog, anahtar döner.
    assert_eq!(t!("fx-hello"), "fx-hello");

    let src = nuxut_i18n::embed!("fx", "tests/fixtures/good");
    nuxut_i18n::set_current(Arc::new(Catalog::load(&[src], &LanguagePrefs::fixed(&["en"]))));
    assert_eq!(t!("fx-hello"), "Hello");
    assert_eq!(t!("fx-greet", name = "Ada"), "Hello Ada");
    let name = String::from("Bob");
    assert_eq!(t!("fx-greet", name = name.as_str(),), "Hello Bob");
    assert_eq!(t!("fx-files", count = 2), "2 files");

    // Başka iş parçacığı yeni değeri görür; değişim önbelleği geçersiz kılar.
    let seen = std::thread::spawn(|| t!("fx-hello")).join().unwrap();
    assert_eq!(seen, "Hello");
    nuxut_i18n::set_current(Arc::new(Catalog::load(&[src], &LanguagePrefs::fixed(&["de"]))));
    assert_eq!(t!("fx-hello"), "Hallo");
    let seen = std::thread::spawn(|| t!("fx-hello")).join().unwrap();
    assert_eq!(seen, "Hallo");

    // Kayıt + configure: aynı ayar ikinci kez iş yapmaz. Kayıt kancası yalnız yeni domain'de
    // (runtime çalışırken eklenen domain'i izlemeye alır).
    let hooked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let h = hooked.clone();
    nuxut_i18n::set_register_hook(Some(Arc::new(move || {
        // Kanca kayıt kilidi dışında: kayıtları okuyabilir.
        assert!(!nuxut_i18n::registered().is_empty());
        h.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    })));
    nuxut_i18n::register(src);
    assert_eq!(nuxut_i18n::registered().len(), 1);
    nuxut_i18n::register(src);
    assert_eq!(nuxut_i18n::registered().len(), 1);
    assert_eq!(hooked.load(std::sync::atomic::Ordering::SeqCst), 1, "yeniden kayıt kancayı çağırmaz");
    nuxut_i18n::set_register_hook(None);
    let cfg = LocaleConfig { language: LocaleSetting::parse("tr"), formats: LocaleSetting::AUTO, ..Default::default() };
    assert!(nuxut_i18n::configure(&cfg));
    assert!(!nuxut_i18n::configure(&cfg));
    // NUXUT_LANG ortamda ayarlıysa geliştirici geçersiz kılması kazanır; değilse tr.
    if std::env::var_os("NUXUT_LANG").is_none() {
        assert_eq!(t!("fx-hello"), "Merhaba");
        assert_eq!(nuxut_i18n::current().languages()[0].to_string(), "tr");
    }
    let cfg = LocaleConfig { language: LocaleSetting::parse("en"), formats: LocaleSetting::AUTO, ..Default::default() };
    assert!(nuxut_i18n::configure(&cfg));
    if std::env::var_os("NUXUT_LANG").is_none() {
        assert_eq!(t!("fx-hello"), "Hello");
    }

    // Sayı locale'i [locale] formats'ı izler (dilden bağımsız); NUMBER aynı locale'le biçimler.
    let cfg = LocaleConfig { language: LocaleSetting::parse("en"), formats: LocaleSetting::parse("tr_TR"), ..Default::default() };
    assert!(nuxut_i18n::configure(&cfg));
    assert_eq!(nuxut_i18n::numeric_locale().name(), "tr_TR");
    assert_eq!(nuxut_i18n::current().numeric().name(), "tr_TR");
    assert_eq!(nuxut_i18n::numeric_locale().format_percent(0.29, 0), "%29");
    let cfg = LocaleConfig { language: LocaleSetting::parse("en"), formats: LocaleSetting::parse("de_DE"), ..Default::default() };
    assert!(nuxut_i18n::configure(&cfg));
    assert_eq!(nuxut_i18n::numeric_locale().name(), "de_DE");
    assert!(!nuxut_i18n::time_locale().uses_12_hour(), "de_DE auto → 24 saat");
    // Yalnızca hour_cycle değişimi de ayar değişimidir (runtime → locale_changed).
    let cfg = LocaleConfig { hour_cycle: HourCycle::H12.into(), ..cfg };
    assert!(nuxut_i18n::configure(&cfg));
    assert!(!nuxut_i18n::configure(&cfg));
    assert_eq!(nuxut_i18n::time_locale().name(), "de_DE");
    assert_eq!(nuxut_i18n::time_locale().format_hm(13, 5).unwrap(), "1:05 PM");
    // Elle kurulan katalog kendi locale'ini taşır.
    let prefs = LanguagePrefs::fixed(&["en"]).with_numeric(nuxut_locale::Locale::parse("fr_FR").unwrap());
    nuxut_i18n::set_current(Arc::new(Catalog::load(&[src], &prefs)));
    assert_eq!(nuxut_i18n::numeric_locale().name(), "fr_FR");
}
