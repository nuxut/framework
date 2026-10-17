//! `UserDirWatcher` runtime'sız: geçici veri dizininde dizin/dosya oluştur → olay → karar.

use std::fs;
use std::path::Path;

use nuxut_i18n::watch::UserDirWatcher;

fn write_atomic(path: &Path, text: &str) {
    let tmp = path.with_file_name(".swap.tmp");
    fs::write(&tmp, text).unwrap();
    fs::rename(&tmp, path).unwrap();
}

#[test]
fn follows_directory_creation_and_reports_real_changes() {
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("nuxut/locales");
    let mut w = UserDirWatcher::new(&root, &["shell", "../evil", ""]).unwrap();
    // Hiçbir alt dizin yok: veri dizini izlenir.
    assert_eq!(w.watched_dirs(), vec![data.path().to_owned()]);
    assert!(!w.process_events(), "olay yok");

    // İlgisiz ad veri dizininde: ilgisiz.
    fs::create_dir(data.path().join("other-app")).unwrap();
    assert!(!w.process_events());

    // Tüm zincir bir kerede oluşur: izleme domain dizinine iner.
    fs::create_dir_all(root.join("shell")).unwrap();
    assert!(w.process_events());
    assert_eq!(w.watched_dirs(), vec![root.join("shell")]);
    assert!(!w.take_change(), "boş dizin katalogu değiştirmez");

    fs::write(root.join("shell/tr.ftl"), "a = A\n").unwrap();
    assert!(w.process_events());
    assert!(w.take_change());

    // Aynı içerik atomik yeniden yazım: olay var, değişiklik yok.
    write_atomic(&root.join("shell/tr.ftl"), "a = A\n");
    assert!(w.process_events());
    assert!(!w.take_change());

    // .ftl dışı dosya ilgisiz.
    fs::write(root.join("shell/README"), "x").unwrap();
    assert!(!w.process_events());

    write_atomic(&root.join("shell/tr.ftl"), "a = B\n");
    assert!(w.process_events());
    assert!(w.take_change());

    // Domain dizini silinir → üst dizine geri çıkar; yeniden oluşunca tekrar iner.
    fs::remove_dir_all(root.join("shell")).unwrap();
    assert!(w.process_events());
    assert_eq!(w.watched_dirs(), vec![root.clone()]);
    assert!(w.take_change());
    fs::create_dir(root.join("shell")).unwrap();
    assert!(w.process_events());
    assert_eq!(w.watched_dirs(), vec![root.join("shell")]);
    fs::write(root.join("shell/en.ftl"), "a = E\n").unwrap();
    assert!(w.process_events(), "yeniden oluşan dizinde iz canlı");
    assert!(w.take_change());
}

#[test]
fn symlinked_domain_dir_is_not_descended() {
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("nuxut/locales");
    fs::create_dir_all(&root).unwrap();
    let elsewhere = data.path().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    std::os::unix::fs::symlink(&elsewhere, root.join("shell")).unwrap();
    let mut w = UserDirWatcher::new(&root, &["shell"]).unwrap();
    assert_eq!(w.watched_dirs(), vec![root.clone()]);
    // Bağın hedefindeki yazım izlenmez (olay gelmez).
    fs::write(elsewhere.join("tr.ftl"), "a = A\n").unwrap();
    assert!(!w.process_events());
}

#[test]
fn missing_data_home_is_not_watched() {
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("absent/nuxut/locales");
    let mut w = UserDirWatcher::new(&root, &["shell"]).unwrap();
    assert!(w.watched_dirs().is_empty());
    assert!(!w.process_events());
}

/// Çalışma başladıktan sonra kaydedilen domain (`set_domains`): yeni dizin izlenir, eski
/// domain'de birleştirmeyi bekleyen değişiklik kaybolmaz, aynı küme iş yapmaz.
#[test]
fn late_domains_are_watched() {
    let data = tempfile::tempdir().unwrap();
    let root = data.path().join("nuxut/locales");
    fs::create_dir_all(root.join("shell")).unwrap();
    fs::create_dir_all(root.join("settings")).unwrap();
    fs::write(root.join("settings/tr.ftl"), "s = S\n").unwrap();
    let mut w = UserDirWatcher::new(&root, &["shell"]).unwrap();
    assert_eq!(w.watched_dirs(), vec![root.join("shell")]);
    // Kayıtsız domain'deki yazım izlenmez.
    write_atomic(&root.join("settings/tr.ftl"), "s = T\n");
    assert!(!w.process_events());

    // Eski domain'de değişiklik var ama birleştirme henüz karar vermedi.
    fs::write(root.join("shell/tr.ftl"), "a = A\n").unwrap();
    assert!(w.process_events());
    assert!(w.set_domains(&["settings", "shell", "../evil"]));
    assert_eq!(w.domains(), vec!["settings".to_owned(), "shell".to_owned()]);
    assert_eq!(w.watched_dirs(), vec![root.join("settings"), root.join("shell")]);
    assert!(w.take_change(), "bekleyen değişiklik korunur");
    assert!(!w.take_change(), "yeni domain'in mevcut dosyaları taban");
    assert!(!w.set_domains(&["shell", "settings"]), "aynı küme");

    write_atomic(&root.join("settings/tr.ftl"), "s = U\n");
    assert!(w.process_events(), "sonradan kaydedilen domain izlenir");
    assert!(w.take_change());
}
