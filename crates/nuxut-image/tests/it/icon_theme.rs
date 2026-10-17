//! Sahte tema dizinleriyle ikon araması (Icon Theme Spec).

use std::path::{Path, PathBuf};

use nuxut_image::IconTheme;

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"/>"#;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// base/
///   Test/   (Inherits=Parent,Test — döngü)  16/apps (Fixed), 16@2/apps, 48/apps (Threshold)
///   Parent/ scalable/apps
///   hicolor/ 32x32/apps
/// pixmaps/
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("icons");
    let pixmaps = dir.path().join("pixmaps");
    write(
        &base.join("Test/index.theme"),
        "[Icon Theme]\nName=Test\nInherits=Parent,Test\nDirectories=16/apps,48/apps\nScaledDirectories=16@2/apps\n\n\
         [16/apps]\nSize=16\nType=Fixed\n\n[16@2/apps]\nSize=16\nScale=2\nType=Fixed\n\n[48/apps]\nSize=48\nType=Threshold\n",
    );
    write(&base.join("Test/16/apps/term.png"), "");
    write(&base.join("Test/16@2/apps/term.svg"), SVG);
    write(&base.join("Test/48/apps/term.svg"), SVG);
    write(&base.join("Test/48/apps/only-big.png"), "");
    write(
        &base.join("Parent/index.theme"),
        "[Icon Theme]\nName=Parent\nDirectories=scalable/apps\n\n[scalable/apps]\nSize=64\nMinSize=8\nMaxSize=512\nType=Scalable\n",
    );
    write(&base.join("Parent/scalable/apps/browser.svg"), SVG);
    write(&base.join("Parent/scalable/apps/network-wireless.svg"), SVG);
    write(&base.join("hicolor/index.theme"), "[Icon Theme]\nName=Hicolor\nDirectories=32x32/apps\n\n[32x32/apps]\nSize=32\nType=Threshold\n");
    write(&base.join("hicolor/32x32/apps/legacy.png"), "");
    write(&base.join("Adwaita/index.theme"), "[Icon Theme]\nName=Adwaita\nInherits=hicolor\nDirectories=scalable/places\n\n[scalable/places]\nSize=16\nMinSize=8\nMaxSize=512\nType=Scalable\n");
    write(&base.join("Adwaita/scalable/places/folder.svg"), SVG);
    write(&pixmaps.join("ancient.png"), "");
    (dir, base, pixmaps)
}

fn theme(base: &Path, pixmaps: &Path, name: &str) -> IconTheme {
    IconTheme::with_dirs(name, vec![base.to_owned()], vec![pixmaps.to_owned()])
}

fn rel(p: Option<PathBuf>, base: &Path) -> String {
    p.map(|p| p.strip_prefix(base.parent().unwrap()).unwrap().display().to_string()).unwrap_or_default()
}

#[test]
fn exact_size_then_closest() {
    let (_d, base, pix) = fixture();
    let t = theme(&base, &pix, "Test");
    assert_eq!(rel(t.lookup("term", 16, 1), &base), "icons/Test/16/apps/term.png");
    assert_eq!(rel(t.lookup("term", 16, 2), &base), "icons/Test/16@2/apps/term.svg");
    assert_eq!(rel(t.lookup("term", 47, 1), &base), "icons/Test/48/apps/term.svg");
    // 24 hiçbir dizinle eşleşmez: 16 (uzaklık 8) ile 48-2 (uzaklık 22) arasından 16.
    assert_eq!(rel(t.lookup("term", 24, 1), &base), "icons/Test/16/apps/term.png");
    // Yalnızca büyük boyutu olan ikon: en yakın yine de bulunur.
    assert_eq!(rel(t.lookup("only-big", 16, 1), &base), "icons/Test/48/apps/only-big.png");
}

#[test]
fn inheritance_hicolor_and_pixmaps() {
    let (_d, base, pix) = fixture();
    let t = theme(&base, &pix, "Test");
    assert_eq!(rel(t.lookup("browser", 22, 1), &base), "icons/Parent/scalable/apps/browser.svg");
    assert_eq!(rel(t.lookup("legacy", 22, 1), &base), "icons/hicolor/32x32/apps/legacy.png");
    assert_eq!(rel(t.lookup("ancient", 22, 1), &base), "pixmaps/ancient.png");
    assert_eq!(t.lookup("missing", 22, 1), None);
    // Kurulu olmayan tema: GTK gibi Adwaita, sonra hicolor.
    let gone = theme(&base, &pix, "Papirus-Dark");
    assert!(!gone.is_installed());
    assert_eq!(rel(gone.lookup("legacy", 22, 1), &base), "icons/hicolor/32x32/apps/legacy.png");
    assert_eq!(rel(gone.lookup("folder", 22, 1), &base), "icons/Adwaita/scalable/places/folder.svg");
    // Kurulu temada Adwaita'ya düşülmez (spec: tema zinciri → hicolor).
    assert!(t.is_installed());
    assert_eq!(t.lookup("folder", 22, 1), None);
}

#[test]
fn names_paths_and_fallbacks() {
    let (_d, base, pix) = fixture();
    let t = theme(&base, &pix, "Test");
    assert_eq!(rel(t.lookup("browser.png", 22, 1), &base), "icons/Parent/scalable/apps/browser.svg");
    assert_eq!(t.lookup("../hicolor/legacy", 22, 1), None);
    assert_eq!(t.lookup(&"x".repeat(300), 22, 1), None);
    let abs = base.join("hicolor/32x32/apps/legacy.png");
    assert_eq!(t.lookup(abs.to_str().unwrap(), 22, 1), Some(abs.clone()));
    assert_eq!(t.lookup(&format!("file://{}", abs.display()), 22, 1), Some(abs));
    assert_eq!(
        rel(t.lookup_with_fallbacks("network-wireless-signal-good", 22, 1), &base),
        "icons/Parent/scalable/apps/network-wireless.svg"
    );
}

#[test]
fn results_are_cached_until_cleared() {
    let (_d, base, pix) = fixture();
    let t = theme(&base, &pix, "Test");
    assert!(t.lookup("browser", 22, 1).is_some());
    std::fs::remove_file(base.join("Parent/scalable/apps/browser.svg")).unwrap();
    assert!(t.lookup("browser", 22, 1).is_some(), "önbellekten gelmeli");
    t.clear_cache();
    assert_eq!(t.lookup("browser", 22, 1), None);
}

#[test]
fn loads_svg_icon_at_physical_size() {
    let (_d, base, pix) = fixture();
    let t = theme(&base, &pix, "Test");
    let path = t.lookup("browser", 24, 2).unwrap();
    let img = nuxut_image::icon::load_icon_file_as(&path, 48, nuxut_image::Trust::System).unwrap();
    assert_eq!(img.size(), (48, 48));
}
