//! Uçtan uca (runtime'sız): geçici XDG_DATA_HOME'da kullanıcı dosyası yaz → izleyici olayı →
//! birleştirme → `reload` → süreç geneli katalog yeni metni verir. Ortam değiştirildiği için
//! ayrı ikili ve tek test.
#![cfg(target_os = "linux")]

use std::time::{Duration, Instant};

use nuxut_i18n::watch::{Debounce, DebouncePoll, UserDirWatcher};
use nuxut_i18n::{LocaleConfig, LocaleSetting, Source};

const SRC: Source = Source {
    domain: "wtest",
    embedded: &[("en", "w-hello = Hello\n")],
};

#[test]
fn user_file_change_reloads_catalog() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    // SAFETY: bu ikilide tek test var; ortam başka iş parçacığı başlamadan değiştirilir.
    unsafe {
        std::env::set_var("XDG_DATA_HOME", data.path());
        std::env::set_var("XDG_CONFIG_HOME", config.path());
        std::env::remove_var("NUXUT_LANG");
        std::env::set_var("LANGUAGE", "en");
    }
    nuxut_i18n::init(&[SRC]);
    nuxut_i18n::configure(&LocaleConfig {
        language: LocaleSetting::parse("en"),
        formats: LocaleSetting::AUTO,
        ..Default::default()
    });
    assert_eq!(nuxut_i18n::current().text("w-hello"), "Hello");

    let root = nuxut_i18n::default_user_dir().unwrap();
    assert!(root.starts_with(data.path()));
    let domains: Vec<&str> = nuxut_i18n::registered().iter().map(|s| s.domain).collect();
    let mut watcher = UserDirWatcher::new(&root, &domains).unwrap();
    let mut debounce = Debounce::default();

    // Patlama: dizin + iki yazım; tek tetik.
    std::fs::create_dir_all(root.join("wtest")).unwrap();
    std::fs::write(root.join("wtest/en.ftl"), "w-hello = Hi\n").unwrap();
    std::fs::write(root.join("wtest/en.ftl"), "w-hello = Hi there\n").unwrap();
    let t0 = Instant::now();
    assert!(watcher.process_events());
    let deadline = debounce.event(t0);
    assert_eq!(debounce.poll(t0), DebouncePoll::Wait(deadline));
    assert_eq!(
        debounce.poll(t0 + Duration::from_millis(200)),
        DebouncePoll::Fire
    );
    assert!(watcher.take_change());
    nuxut_i18n::reload();
    assert_eq!(nuxut_i18n::current().text("w-hello"), "Hi there");

    // Aynı içerik: yeniden kurma kararı yok.
    std::fs::write(root.join("wtest/en.ftl"), "w-hello = Hi there\n").unwrap();
    assert!(watcher.process_events());
    assert!(!watcher.take_change());

    // Dosya silinir: gömülüye dönülür.
    std::fs::remove_file(root.join("wtest/en.ftl")).unwrap();
    assert!(watcher.process_events());
    assert!(watcher.take_change());
    nuxut_i18n::reload();
    assert_eq!(nuxut_i18n::current().text("w-hello"), "Hello");
}
