//! Uygulama ikonları uçtan uca: sahte XDG veri dizinleri (`XDG_DATA_HOME`/`XDG_DATA_DIRS`
//! değerleri `AppIconDirs::from_vars`'a verilir; süreç ortamı değiştirilmez, testler paralel
//! koşar), sahte tema ve `.desktop` dosyaları.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use image::{ImageEncoder, RgbaImage};
use nuxut_image::{AppIcon, AppIconDirs, AppIconOptions, AppIcons, DecodeOptions, Image, ImageError, IsolatedDecoder, Isolation};

/// Yalnızca canlılık sınırı (asılı kalan iş testi bitirsin): doğruluk saate bağlı değil.
const LIVENESS: Duration = Duration::from_secs(120);

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const GRAY: [u8; 4] = [128, 128, 128, 255];
const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" fill="red"/></svg>"#;

fn png(rgba: [u8; 4]) -> Vec<u8> {
    let img = RgbaImage::from_pixel(48, 48, image::Rgba(rgba));
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), 48, 48, image::ExtendedColorType::Rgba8).unwrap();
    out
}

fn write(path: &Path, bytes: impl AsRef<[u8]>) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn desktop(icon: &str, extra: &str) -> String {
    format!("[Desktop Entry]\nType=Application\nName=Uygulama\nName[tr]=Uygulama\nExec=true\nIcon={icon}\n{extra}")
}

const THEME: &str = "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps,scalable/apps\n\n[48x48/apps]\nSize=48\nType=Threshold\n\n\
                     [scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n";

/// root/
///   home/.local/share/applications/      firefox (StartupWMClass), flatpak dışa aktarımı
///   sys/applications/                    org.gnome.Nautilus, code (WMClass=Code), abs (mutlak yol), svgapp
///   sys/icons/hicolor/                   mavi Nautilus, yeşil firefox, gri vscode, svgapp.svg
///   sys/icons/Red/  (Inherits=hicolor)   kırmızı Nautilus
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    dirs: AppIconDirs,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let home = root.join("home");
    let share = home.join(".local/share");
    let sys = root.join("sys");
    write(&share.join("applications/firefox.desktop"), desktop("firefox", "StartupWMClass=Navigator\n"));
    let flat = share.join("flatpak/exports/share");
    write(&flat.join("applications/com.example.Flat.desktop"), desktop("com.example.Flat", ""));
    write(&flat.join("icons/hicolor/index.theme"), THEME);
    write(&flat.join("icons/hicolor/48x48/apps/com.example.Flat.png"), png(GREEN));
    let apps = sys.join("applications");
    write(&apps.join("org.gnome.Nautilus.desktop"), desktop("org.gnome.Nautilus", ""));
    write(&apps.join("code.desktop"), desktop("vscode", "StartupWMClass=Code\n"));
    write(&apps.join("abs.desktop"), desktop(root.join("opt/abs.png").to_str().unwrap(), ""));
    write(&root.join("opt/abs.png"), png(GREEN));
    write(&apps.join("svgapp.desktop"), desktop("svgapp", ""));
    let icons = sys.join("icons");
    write(&icons.join("hicolor/index.theme"), THEME);
    write(&icons.join("hicolor/48x48/apps/org.gnome.Nautilus.png"), png(BLUE));
    write(&icons.join("hicolor/48x48/apps/firefox.png"), png(GREEN));
    write(&icons.join("hicolor/48x48/apps/vscode.png"), png(GRAY));
    write(&icons.join("hicolor/48x48/apps/kitty.png"), png(GRAY));
    write(&icons.join("hicolor/scalable/apps/svgapp.svg"), SVG);
    write(&icons.join("Red/index.theme"), "[Icon Theme]\nName=Red\nInherits=hicolor\nDirectories=48x48/apps\n\n[48x48/apps]\nSize=48\nType=Threshold\n");
    write(&icons.join("Red/48x48/apps/org.gnome.Nautilus.png"), png(RED));
    let dirs = AppIconDirs::from_vars(Some(&home), None, Some(sys.to_str().unwrap()));
    Fixture { _dir: dir, root, dirs }
}

fn icons(f: &Fixture, theme: &str, sandbox: Option<Arc<dyn IsolatedDecoder>>) -> (AppIcons, Receiver<()>) {
    let (tx, rx) = channel();
    let tx = Mutex::new(tx);
    let options = AppIconOptions { sandbox, theme: Some(theme.to_owned()), dirs: Some(f.dirs.clone()) };
    let icons = AppIcons::new(options, move || {
        let _ = tx.lock().unwrap().send(());
    });
    (icons, rx)
}

/// Sonuç gelene dek bekler (her `notify` bir sonuç).
fn resolve(icons: &AppIcons, rx: &Receiver<()>, app_id: &str, size: u32, scale: u32) -> AppIcon {
    loop {
        match icons.get(app_id, size, scale) {
            AppIcon::Pending => rx.recv_timeout(LIVENESS).expect("ikon sonucu gelmedi"),
            done => return done,
        }
    }
}

fn color(icon: &AppIcon) -> Option<[u8; 4]> {
    match icon {
        AppIcon::Ready(img) => Some(img.straight_pixel(img.width() / 2, img.height() / 2)),
        _ => None,
    }
}

#[test]
fn app_ids_map_to_desktop_entries_and_theme_icons() {
    let f = fixture();
    let (icons, rx) = icons(&f, "hicolor", None);
    // Tam ad, ters-DNS sonu, küçük harf, StartupWMClass, Flatpak dışa aktarımı, app_id'nin kendisi.
    for (app_id, want) in [
        ("org.gnome.Nautilus", BLUE),
        ("nautilus", BLUE),
        ("Code", GRAY),
        ("Navigator", GREEN),
        ("org.mozilla.firefox", GREEN),
        ("com.example.Flat", GREEN),
        ("kitty", GRAY),
        ("abs", GREEN),
    ] {
        assert_eq!(color(&resolve(&icons, &rx, app_id, 48, 1)), Some(want), "{app_id}");
    }
    assert_eq!(resolve(&icons, &rx, "bilinmeyen-uygulama", 48, 1), AppIcon::Missing);
    // Negatif önbellek: ikinci istek kuyruğa girmez.
    assert_eq!(icons.get("bilinmeyen-uygulama", 48, 1), AppIcon::Missing);
    assert_eq!(icons.get("a/b", 48, 1), AppIcon::Missing, "geçersiz app_id");
}

#[test]
fn size_and_scale_select_the_target() {
    let f = fixture();
    let (icons, rx) = icons(&f, "hicolor", None);
    let AppIcon::Ready(small) = resolve(&icons, &rx, "kitty", 16, 1) else { panic!("kitty") };
    assert_eq!(small.size(), (16, 16), "hedefe küçültülür");
    let AppIcon::Ready(hidpi) = resolve(&icons, &rx, "kitty", 24, 2) else { panic!("kitty@2") };
    assert_eq!(hidpi.size(), (48, 48));
}

#[test]
fn theme_change_invalidates_cache() {
    let f = fixture();
    let (icons, rx) = icons(&f, "hicolor", None);
    assert_eq!(color(&resolve(&icons, &rx, "org.gnome.Nautilus", 48, 1)), Some(BLUE));
    let generation = icons.generation();
    assert!(icons.set_theme_name(Some("Red")));
    assert!(!icons.set_theme_name(Some("Red")), "aynı ad değişiklik değil");
    assert!(icons.generation() > generation);
    assert_eq!(icons.cache_stats(), (0, 0), "önbellek boşaldı");
    assert_eq!(color(&resolve(&icons, &rx, "org.gnome.Nautilus", 48, 1)), Some(RED));
    // Red'de olmayan ikon Inherits → hicolor'dan.
    assert_eq!(color(&resolve(&icons, &rx, "kitty", 48, 1)), Some(GRAY));
}

#[test]
fn new_applications_clear_negative_results() {
    let f = fixture();
    let (icons, rx) = icons(&f, "hicolor", None);
    assert_eq!(resolve(&icons, &rx, "yeni", 48, 1), AppIcon::Missing);
    // Uygulama kuruldu: sistem applications dizininin değişiklik zamanı değişir.
    std::thread::sleep(Duration::from_millis(20));
    write(&f.root.join("sys/applications/yeni.desktop"), desktop("kitty", ""));
    // Sonraki iş dizinin eskidiğini görür, negatif sonuçları atar.
    assert!(matches!(resolve(&icons, &rx, "code", 48, 1), AppIcon::Ready(_)));
    assert_eq!(color(&resolve(&icons, &rx, "yeni", 48, 1)), Some(GRAY));
}

/// Kullanıcıya ait (geçici) dizindeki SVG `Untrusted`'a düşer: yalıtılmış çözücü yoksa süreç
/// içinde ayrıştırılmaz, varsa ona gider.
#[test]
fn untrusted_svg_goes_to_the_sandbox() {
    let f = fixture();
    let (plain, rx) = icons(&f, "hicolor", None);
    assert_eq!(resolve(&plain, &rx, "svgapp", 48, 1), AppIcon::Missing);
    let fake = Arc::new(FakeSandbox::default());
    let (isolated, rx) = icons(&f, "hicolor", Some(fake.clone()));
    assert_eq!(color(&resolve(&isolated, &rx, "svgapp", 48, 1)), Some(RED));
    assert_eq!(*fake.calls.lock().unwrap(), 1);
}

#[derive(Debug, Default)]
struct FakeSandbox {
    calls: Mutex<u32>,
}

impl IsolatedDecoder for FakeSandbox {
    fn isolation(&self) -> Isolation {
        Isolation::Process
    }

    fn decode(&self, _key: u64, bytes: Arc<[u8]>, _opts: &DecodeOptions) -> Result<Image, ImageError> {
        *self.calls.lock().unwrap() += 1;
        assert!(bytes.starts_with(b"<svg"), "yalnızca SVG baytları işçiye gider");
        Image::from_straight(4, 4, [255, 0, 0, 255].repeat(16))
    }
}
