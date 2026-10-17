//! Kurulu ikon ve imleç temalarının listesi (Ayarlar seçicileri, shell ADR-0004 §9 durumu).
//!
//! Arama dizinleri [`super::default_base_dirs`] sırasıdır (`~/.icons`,
//! `$XDG_DATA_HOME/icons`, `$XDG_DATA_DIRS/*/icons`); aynı ad birden çok dizinde varsa
//! **ilk** dizindeki `index.theme` kazanır (Icon Theme Spec ve [`super::IconTheme`] ile aynı
//! öncelik). İmleç yetenekleri (`cursors/`, hyprcursor manifesti) tüm dizinlerden birleştirilir.
//!
//! Sınırlar ve güvenlik (dizinler kullanıcının ve paketlerin yazabildiği yerler):
//! - yalnızca bir düzey okunur (özyineleme yok → symlink döngüsü takılma yaratamaz; döngüdeki
//!   bağlantı `ELOOP` ile "yok" sayılır), kanonik yolu aynı olan arama dizinleri bir kez taranır;
//! - dizin başına en çok [`MAX_ENTRIES_PER_DIR`] girdi, toplam en çok [`MAX_THEMES`] tema;
//! - `index.theme` yalnızca normal dosyaysa ve en çok [`MAX_INDEX_BYTES`] ise okunur (FIFO/aygıt
//!   engellemesin diye `O_NONBLOCK` ile açılır); bozuk/UTF-8 olmayan içerik tema adını düşürmez,
//!   yalnızca alanlar boş kalır;
//! - tema adı tek dosya adı bileşeni olmalı (`/`, `..`, denetim karakteri, `.` ile başlama yok);
//!   `default` (XCursor yönlendirmesi `~/.icons/default`) liste dışıdır;
//! - `Name` denetim karakterlerinden arındırılır ve [`MAX_NAME_CHARS`] karakterle sınırlanır.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use super::index::parse_ini;

/// Toplam en çok tema (ikon + imleç ayrı ayrı).
pub const MAX_THEMES: usize = 512;
/// Bir arama dizininde bakılan en çok girdi.
pub const MAX_ENTRIES_PER_DIR: usize = 4096;
/// `index.theme` en büyük boyutu.
pub const MAX_INDEX_BYTES: u64 = 64 * 1024;
/// Görünen ad en çok bu kadar karakter.
pub const MAX_NAME_CHARS: usize = 128;
/// `Inherits` en çok bu kadar ad.
const MAX_INHERITS: usize = 32;

/// Kurulu bir ikon teması (`index.theme`'inde `Directories` olan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledIconTheme {
    /// Dizin adı: GTK/Qt ayarlarına yazılan ad (`Papirus-Dark`).
    pub id: String,
    /// `[Icon Theme] Name` (yoksa `id`).
    pub name: String,
    /// `Inherits` (geçerli adlar, sırayla).
    pub inherits: Vec<String>,
    /// `Hidden=true`: seçicilerde gösterilmemeli (ör. `hicolor`, `Adwaita`); yedek olarak
    /// kullanılabilir.
    pub hidden: bool,
    /// Aynı tema imleç de içeriyor mu (`cursors/`).
    pub has_cursors: bool,
    /// `index.theme`'in bulunduğu tema dizini (en yüksek öncelikli).
    pub dir: PathBuf,
}

/// Kurulu bir imleç teması (`cursors/` dizini ya da hyprcursor manifesti olan).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledCursorTheme {
    /// Dizin adı: `XCURSOR_THEME`, `gtk-cursor-theme-name`, `hyprctl setcursor` adı.
    pub id: String,
    /// `[Icon Theme] Name` (yoksa `id`).
    pub name: String,
    /// `Inherits` (XCursor yedek zinciri).
    pub inherits: Vec<String>,
    /// XCursor biçimi var (`cursors/` dizini).
    pub xcursor: bool,
    /// Hyprcursor biçimi var (`manifest.hl` ya da `manifest.toml`).
    pub hyprcursor: bool,
    /// `Hidden=true` (`index.theme`): seçicilerde gösterilmemeli; ikon kaydındaki
    /// [`InstalledIconTheme::hidden`] ile aynı kural. `index.theme`'siz tema `false`.
    pub hidden: bool,
    /// İlk bulunduğu tema dizini (en yüksek öncelikli).
    pub dir: PathBuf,
}

/// Standart arama dizinlerindeki ikon temaları, görünen ada göre sıralı.
pub fn installed_icon_themes() -> Vec<InstalledIconTheme> {
    icon_themes_in(&super::default_base_dirs())
}

/// Standart arama dizinlerindeki imleç temaları, görünen ada göre sıralı.
pub fn installed_cursor_themes() -> Vec<InstalledCursorTheme> {
    cursor_themes_in(&super::default_base_dirs())
}

/// `base_dirs` (öncelik sırasıyla) altındaki ikon temaları.
pub fn icon_themes_in(base_dirs: &[PathBuf]) -> Vec<InstalledIconTheme> {
    let mut out: Vec<InstalledIconTheme> = scan(base_dirs)
        .into_iter()
        .filter_map(|t| {
            let index = t.index?;
            index.has_directories.then(|| InstalledIconTheme {
                name: index.name.unwrap_or_else(|| t.id.clone()),
                inherits: index.inherits,
                hidden: index.hidden,
                has_cursors: t.xcursor,
                dir: index.dir,
                id: t.id,
            })
        })
        .collect();
    out.sort_by_key(|t| sort_key(&t.name, &t.id));
    out
}

/// `base_dirs` (öncelik sırasıyla) altındaki imleç temaları.
pub fn cursor_themes_in(base_dirs: &[PathBuf]) -> Vec<InstalledCursorTheme> {
    let mut out: Vec<InstalledCursorTheme> = scan(base_dirs)
        .into_iter()
        .filter(|t| t.xcursor || t.hyprcursor)
        .map(|t| {
            let (name, inherits, hidden) = match t.index {
                Some(i) => (i.name, i.inherits, i.hidden),
                None => (None, Vec::new(), false),
            };
            // index.theme adı yoksa hyprcursor manifestinin `name`'i.
            let name = name.or_else(|| {
                t.hyprcursor.then(|| crate::cursor::hyprcursor::read_manifest(&t.first_dir).and_then(|m| m.name)).flatten()
            });
            InstalledCursorTheme {
                name: name.unwrap_or_else(|| t.id.clone()),
                inherits,
                xcursor: t.xcursor,
                hyprcursor: t.hyprcursor,
                hidden,
                dir: t.first_dir,
                id: t.id,
            }
        })
        .collect();
    out.sort_by_key(|t| sort_key(&t.name, &t.id));
    out
}

fn sort_key(name: &str, id: &str) -> (String, String) {
    (name.to_lowercase(), id.to_owned())
}

#[derive(Debug)]
struct Index {
    name: Option<String>,
    inherits: Vec<String>,
    hidden: bool,
    has_directories: bool,
    dir: PathBuf,
}

#[derive(Debug)]
struct Found {
    id: String,
    first_dir: PathBuf,
    index: Option<Index>,
    xcursor: bool,
    hyprcursor: bool,
}

/// Tüm arama dizinlerini bir düzey tarar; ad başına bilgileri öncelik sırasıyla birleştirir.
fn scan(base_dirs: &[PathBuf]) -> Vec<Found> {
    let mut seen_bases: Vec<PathBuf> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut found: HashMap<String, Found> = HashMap::new();
    for base in base_dirs {
        let Ok(canon) = base.canonicalize() else { continue };
        if seen_bases.contains(&canon) {
            continue;
        }
        seen_bases.push(canon);
        let Ok(rd) = std::fs::read_dir(base) else { continue };
        for entry in rd.take(MAX_ENTRIES_PER_DIR).filter_map(Result::ok) {
            let Ok(id) = entry.file_name().into_string() else { continue };
            if !valid_theme_id(&id) {
                continue;
            }
            let dir = entry.path();
            // `metadata` bağlantıyı izler; döngü/kırık bağlantı hata verir → atlanır.
            if !std::fs::metadata(&dir).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let xcursor = std::fs::metadata(dir.join("cursors")).is_ok_and(|m| m.is_dir());
            let hyprcursor = ["manifest.hl", "manifest.toml"]
                .iter()
                .any(|f| std::fs::metadata(dir.join(f)).is_ok_and(|m| m.is_file()));
            let index = found.get(&id).is_none_or(|f| f.index.is_none()).then(|| read_index(&dir)).flatten();
            if !found.contains_key(&id) {
                if found.len() >= MAX_THEMES {
                    continue;
                }
                order.push(id.clone());
                found.insert(
                    id.clone(),
                    Found { id: id.clone(), first_dir: dir.clone(), index: None, xcursor: false, hyprcursor: false },
                );
            }
            let f = found.get_mut(&id).expect("az önce eklendi");
            f.xcursor |= xcursor;
            f.hyprcursor |= hyprcursor;
            if f.index.is_none() {
                f.index = index;
            }
        }
    }
    order.into_iter().filter_map(|id| found.remove(&id)).collect()
}

/// `dir/index.theme`'i sınırlı okur. Dosya yoksa, normal dosya değilse, çok büyükse ya da
/// `[Icon Theme]` bölümü yoksa `None`.
fn read_index(dir: &Path) -> Option<Index> {
    let path = dir.join("index.theme");
    let file = crate::decode::open_regular(&path).ok()?;
    if file.metadata().ok()?.len() > MAX_INDEX_BYTES {
        tracing::debug!("index.theme çok büyük, atlandı");
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_INDEX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return None;
    }
    let text = String::from_utf8_lossy(&bytes);
    let ini = parse_ini(&text);
    let head = ini.get("Icon Theme")?;
    let name = head.get("Name").map(|n| clean_name(n)).filter(|n| !n.is_empty());
    let inherits = head
        .get("Inherits")
        .map(|v| parse_inherits(v).into_iter().filter(|s| valid_theme_id(s)).take(MAX_INHERITS).map(str::to_owned).collect())
        .unwrap_or_default();
    let hidden = head.get("Hidden").is_some_and(|v| v.trim().eq_ignore_ascii_case("true"));
    let has_directories = head.get("Directories").is_some_and(|v| v.split(',').any(|d| !d.trim().is_empty()));
    Some(Index { name, inherits, hidden, has_directories, dir: dir.to_owned() })
}

/// `Inherits` değeri: virgül (spec) ya da noktalı virgülle (libXcursor) ayrılmış adlar; her ad
/// kırpılır ve çift tırnaktan arındırılır (`Inherits="hicolor"` yaygın: Bibata).
fn parse_inherits(value: &str) -> Vec<&str> {
    value.split([',', ';']).map(|s| s.trim().trim_matches('"').trim()).filter(|s| !s.is_empty()).collect()
}

/// `dir/index.theme`'in `Inherits` listesi (imleç teması zinciri için; aynı sınırlı okuma).
pub(crate) fn theme_inherits(dir: &Path) -> Option<Vec<String>> {
    read_index(dir).map(|i| i.inherits)
}

/// Görünen ad: denetim karakterleri atılır, kırpılır, [`MAX_NAME_CHARS`] ile sınırlanır.
fn clean_name(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect::<String>().trim().to_owned()
}

/// Tema dizin adı: tek bileşen, gizli değil, `default` değil, en çok 255 bayt.
fn valid_theme_id(id: &str) -> bool {
    id != "default" && id.len() <= 255 && super::valid_name(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn icon_index(name: &str, extra: &str) -> String {
        format!("[Icon Theme]\nName={name}\nName[tr]=Yerel\n{extra}Directories=16x16/apps\n\n[16x16/apps]\nSize=16\n")
    }

    #[test]
    fn lists_icon_and_cursor_themes_with_priority() {
        let root = tempfile::tempdir().unwrap();
        let (user, sys) = (root.path().join("user"), root.path().join("sys"));
        // Kullanıcı kopyası sistemdekini gölgeler.
        write(&user.join("Papirus/index.theme"), &icon_index("Papirus (kullanıcı)", "Inherits=breeze,hicolor\n"));
        write(&sys.join("Papirus/index.theme"), &icon_index("Papirus (sistem)", ""));
        write(&sys.join("hicolor/index.theme"), &icon_index("Hicolor", "Hidden=true\n"));
        // Yalnızca imleç: Directories yok, cursors/ var.
        write(&sys.join("Bibata/index.theme"), "[Icon Theme]\nName=Bibata Modern\nInherits=Adwaita\nHidden=True\n");
        std::fs::create_dir_all(sys.join("Bibata/cursors")).unwrap();
        // index.theme'siz hyprcursor teması.
        write(&user.join("Hypr-Only/manifest.hl"), "name = Hypr\ncursors_directory = hyprcursors\n");
        // index.theme kullanıcıda yok ama cursors/ kullanıcıda; ad sistemdekinden.
        std::fs::create_dir_all(user.join("Mixed/cursors")).unwrap();
        write(&sys.join("Mixed/index.theme"), "[Icon Theme]\nName=Karışık\n");
        // Yok sayılanlar: default yönlendirmesi, gizli dizin, düz dosya, index'siz ikon dizini.
        write(&user.join("default/index.theme"), "[Icon Theme]\nInherits=Bibata\n");
        std::fs::create_dir_all(user.join("default/cursors")).unwrap();
        write(&user.join(".hidden/index.theme"), &icon_index("Gizli", ""));
        write(&user.join("file-not-dir"), "x");
        std::fs::create_dir_all(sys.join("NoIndex/16x16")).unwrap();

        let dirs = vec![user.clone(), sys.clone(), user.clone(), root.path().join("yok")];
        let icons = icon_themes_in(&dirs);
        let ids: Vec<_> = icons.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["hicolor", "Papirus"]);
        let papirus = &icons[1];
        assert_eq!(papirus.name, "Papirus (kullanıcı)");
        assert_eq!(papirus.inherits, ["breeze", "hicolor"]);
        assert_eq!(papirus.dir, user.join("Papirus"));
        assert!(!papirus.hidden && !papirus.has_cursors);
        assert!(icons[0].hidden);

        let cursors = cursor_themes_in(&dirs);
        let summary: Vec<_> = cursors.iter().map(|c| (c.id.as_str(), c.name.as_str(), c.xcursor, c.hyprcursor)).collect();
        assert_eq!(
            summary,
            [
                ("Bibata", "Bibata Modern", true, false),
                ("Hypr-Only", "Hypr", false, true),
                ("Mixed", "Karışık", true, false),
            ]
        );
        assert_eq!(cursors[0].inherits, ["Adwaita"]);
        // Hidden=true (büyük/küçük harf duyarsız) ikon kaydıyla aynı; index.theme'siz tema gizli değil.
        let hidden: Vec<_> = cursors.iter().map(|c| c.hidden).collect();
        assert_eq!(hidden, [true, false, false]);
        assert_eq!(cursors[2].dir, user.join("Mixed"));
    }

    #[test]
    fn hostile_inputs_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().join("icons");
        // Çok büyük index.theme okunmaz (ama imleç yeteneği yine görülür).
        let big = format!("[Icon Theme]\nName=Big\nDirectories=a\n#{}\n", "x".repeat(MAX_INDEX_BYTES as usize));
        write(&base.join("Big/index.theme"), &big);
        std::fs::create_dir_all(base.join("Big/cursors")).unwrap();
        // Bozuk/ikili içerik, bölümsüz dosya.
        std::fs::create_dir_all(base.join("Binary")).unwrap();
        std::fs::write(base.join("Binary/index.theme"), [0xff, 0xfe, 0x00, b'[', 0x80]).unwrap();
        write(&base.join("NoSection/index.theme"), "Name=X\nDirectories=a\n");
        // Ad: denetim karakteri ve aşırı uzunluk; geçersiz Inherits girdileri.
        let long = "Ä".repeat(500);
        write(
            &base.join("Weird/index.theme"),
            &format!("[Icon Theme]\nName=\u{1b}[31m{long}\nInherits=../etc, ok,  ,.x,a/b\nDirectories=a\n"),
        );
        // index.theme bir dizin.
        std::fs::create_dir_all(base.join("DirIndex/index.theme")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            // Symlink döngüleri: tema dizini kendine, index.theme kendine.
            symlink(base.join("Loop"), base.join("Loop")).unwrap();
            std::fs::create_dir_all(base.join("SelfIndex")).unwrap();
            symlink(base.join("SelfIndex/index.theme"), base.join("SelfIndex/index.theme")).unwrap();
            // Arama dizinine geri dönen bağlantı (bir düzey okunduğu için zararsız).
            symlink(&base, base.join("Back")).unwrap();
            // FIFO index.theme engellemez.
            std::fs::create_dir_all(base.join("Fifo")).unwrap();
            let fifo = base.join("Fifo/index.theme");
            let made = std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success());
            if made {
                assert!(read_index(&base.join("Fifo")).is_none());
            }
        }
        let icons = icon_themes_in(std::slice::from_ref(&base));
        let ids: Vec<_> = icons.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["Weird"], "{icons:?}");
        let weird = &icons[0];
        assert_eq!(weird.name.chars().count(), MAX_NAME_CHARS);
        assert!(!weird.name.chars().any(char::is_control));
        assert_eq!(weird.inherits, ["ok"]);
        let cursors = cursor_themes_in(std::slice::from_ref(&base));
        assert_eq!(cursors.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["Big"]);
        assert_eq!(cursors[0].name, "Big", "okunmayan index → ad dizinden");
    }

    #[test]
    fn inherits_accepts_quotes_and_semicolons() {
        assert_eq!(parse_inherits("\"hicolor\""), ["hicolor"]);
        assert_eq!(parse_inherits(" a ; \"b\",c,, \"\" "), ["a", "b", "c"]);
        let root = tempfile::tempdir().unwrap();
        write(&root.path().join("Bib/index.theme"), "[Icon Theme]\nName=Bib\nInherits=\"Adwaita\"\n");
        assert_eq!(theme_inherits(&root.path().join("Bib")).unwrap(), ["Adwaita"]);
    }

    #[test]
    fn theme_count_is_capped() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..MAX_THEMES + 5 {
            std::fs::create_dir_all(root.path().join(format!("t{i:04}/cursors"))).unwrap();
        }
        assert_eq!(cursor_themes_in(&[root.path().to_owned()]).len(), MAX_THEMES);
    }
}
