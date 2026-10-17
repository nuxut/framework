//! `.desktop` dizini: pencerenin `app_id`'sinden masaüstü girdisine ve onun `Icon=` değerine.
//!
//! Tarama tembeldir (ilk istekte, [`super::AppIcons`]'ın arka plan thread'inde) ve sınırlıdır:
//! en fazla [`MAX_DESKTOP_FILES`] dosya, dosya başına [`MAX_DESKTOP_BYTES`] bayt, alt dizin
//! derinliği [`MAX_DESKTOP_DEPTH`]; sembolik bağlı dizinler izlenmez (döngü yok), dosyalar
//! `O_NONBLOCK` açılır ve fd'den normal dosya olduğu doğrulanır (FIFO/aygıt bloklamaz).
//! Desktop Entry Spec: girdi kimliği `applications/` altındaki göreli yolun `/` → `-`
//! hâlidir; aynı kimlikte önce gelen dizin kazanır (`$XDG_DATA_HOME` sistemi gölgeler),
//! `Hidden=true` girdi kimliği tamamen siler.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::icon::parse_ini;

/// Taranan en fazla `.desktop` dosyası (tüm dizinler).
pub const MAX_DESKTOP_FILES: usize = 4096;
/// Okunan `.desktop` dosyasının en büyük boyutu.
pub const MAX_DESKTOP_BYTES: u64 = 64 * 1024;
/// `applications/` altında inilen en fazla alt dizin düzeyi (`kde4/…` gibi).
pub const MAX_DESKTOP_DEPTH: usize = 3;
/// Bir dizinde bakılan en fazla girdi (dosya + alt dizin).
const MAX_DIR_ENTRIES: usize = 8192;
/// Kabul edilen en uzun `app_id` / girdi kimliği.
pub const MAX_APP_ID_LEN: usize = 255;
/// Kabul edilen en uzun `Icon=` değeri (mutlak yol olabilir).
const MAX_ICON_VALUE_LEN: usize = 1024;

/// Bir `app_id`'nin hangi kuralla eşlendiği (tanı ve testler için).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopMatch {
    /// `<app_id>.desktop` (Flatpak dışa aktarımı dahil).
    Exact,
    /// Küçük harfli kimlik (`Firefox` → `firefox.desktop`).
    Lowercase,
    /// `StartupWMClass=` (büyük/küçük harf duyarsız).
    WmClass,
    /// Ters-DNS sonu: `org.gnome.Nautilus` → `nautilus.desktop` ya da `nautilus` →
    /// `org.gnome.Nautilus.desktop`.
    ReverseDns,
}

#[derive(Debug, Clone)]
struct Entry {
    /// `Icon=` değeri; yoksa ya da girdi gizliyse (`Hidden=true`, `Type` ≠ `Application`) `None`.
    icon: Option<String>,
}

/// Taranmış masaüstü girdileri ve eşleme tabloları.
#[derive(Default)]
pub struct DesktopIndex {
    apps: HashMap<String, Entry>,
    /// Küçük harfli kimlik → kimlik.
    lower: HashMap<String, String>,
    /// Küçük harfli `StartupWMClass` → kimlik.
    wm_class: HashMap<String, String>,
    /// Kimliğin son ters-DNS bileşeni (küçük harf) → kimlik (`org.gnome.Nautilus` → `nautilus`).
    last: HashMap<String, String>,
    /// Taranan dizinler ve o anki değişiklik zamanları (eskime denetimi).
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    files: usize,
}

/// Dizinler ev dizinini, girdiler uygulama adlarını taşır: yalnızca sayılar yazılır.
impl std::fmt::Debug for DesktopIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesktopIndex").field("apps", &self.apps.len()).field("files", &self.files).field("dirs", &self.stamps.len()).finish()
    }
}

impl DesktopIndex {
    /// `dirs` (öncelik sırasıyla `…/applications` dizinleri) taranır. Bloklayan IO: UI
    /// thread'inde çağırma.
    pub fn scan(dirs: &[PathBuf]) -> Self {
        let mut index = Self { stamps: dirs.iter().map(|d| (d.clone(), modified(d))).collect(), ..Self::default() };
        for dir in dirs {
            if index.files >= MAX_DESKTOP_FILES {
                tracing::debug!(limit = MAX_DESKTOP_FILES, "uygulama ikonları: .desktop sınırına ulaşıldı");
                break;
            }
            index.walk(dir, "", 0);
        }
        index
    }

    /// Taranan dizinlerden biri (eklendi/silindi/değişti) ya da dizin listesi değişti mi.
    /// Yalnızca üst dizinlerin değişiklik zamanına bakar (alt dizindeki değişikliği görmez).
    pub fn is_stale(&self, dirs: &[PathBuf]) -> bool {
        self.stamps.len() != dirs.len() || self.stamps.iter().zip(dirs).any(|((d, t), now)| d != now || *t != modified(now))
    }

    /// Okunan `.desktop` dosyası sayısı.
    pub fn len(&self) -> usize {
        self.apps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }

    /// `app_id`'nin masaüstü girdisi kimliği ve `Icon=` değeri. Kurallar sırayla denenir,
    /// `Icon=`'u olan ilk eşleşme döner: tam ad, küçük harf, `StartupWMClass`, ters-DNS sonu.
    pub fn find(&self, app_id: &str) -> Option<(&str, &str, DesktopMatch)> {
        let id = clean_app_id(app_id)?;
        let lower = id.to_lowercase();
        let last = id.contains('.').then(|| last_component(&lower));
        let candidates = [
            (Some(id), DesktopMatch::Exact),
            (self.lower.get(&lower).map(String::as_str), DesktopMatch::Lowercase),
            (self.wm_class.get(&lower).map(String::as_str), DesktopMatch::WmClass),
            (last.and_then(|l| self.lower.get(l)).map(String::as_str), DesktopMatch::ReverseDns),
            (self.last.get(&lower).map(String::as_str), DesktopMatch::ReverseDns),
        ];
        candidates.into_iter().find_map(|(cand, how)| {
            let (key, entry) = self.apps.get_key_value(cand?)?;
            entry.icon.as_deref().map(|icon| (key.as_str(), icon, how))
        })
    }

    /// [`find`](Self::find)'ın yalnızca `Icon=` değeri.
    pub fn icon_for(&self, app_id: &str) -> Option<&str> {
        self.find(app_id).map(|(_, icon, _)| icon)
    }

    fn walk(&mut self, dir: &Path, prefix: &str, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        // Belirlenimci sıra: aynı kimlik/sınıf için ilk gelen kazanır.
        let mut entries: Vec<(String, std::fs::FileType)> = rd
            .take(MAX_DIR_ENTRIES)
            .filter_map(|e| {
                let e = e.ok()?;
                Some((e.file_name().into_string().ok()?, e.file_type().ok()?))
            })
            .collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        for (name, kind) in entries {
            if self.files >= MAX_DESKTOP_FILES {
                return;
            }
            if name.starts_with('.') {
                continue;
            }
            let path = dir.join(&name);
            if kind.is_dir() {
                // Sembolik bağlı dizin `is_dir` değildir: izlenmez (döngü yok).
                if depth + 1 < MAX_DESKTOP_DEPTH {
                    self.walk(&path, &format!("{prefix}{name}-"), depth + 1);
                }
                continue;
            }
            let Some(stem) = name.strip_suffix(".desktop") else { continue };
            let id = format!("{prefix}{stem}");
            if id.len() > MAX_APP_ID_LEN || self.apps.contains_key(&id) {
                continue;
            }
            self.files += 1;
            let Some(text) = read_small(&path) else { continue };
            self.add(id, &text);
        }
    }

    fn add(&mut self, id: String, text: &str) {
        let ini = parse_ini(text);
        let Some(main) = ini.get("Desktop Entry") else { return };
        let flag = |k: &str| main.get(k).is_some_and(|v| v.eq_ignore_ascii_case("true"));
        let application = main.get("Type").is_none_or(|t| t == "Application");
        let icon = main
            .get("Icon")
            .map(|v| v.trim())
            .filter(|v| !v.is_empty() && v.len() <= MAX_ICON_VALUE_LEN && !v.chars().any(char::is_control))
            .filter(|_| application && !flag("Hidden"))
            .map(str::to_owned);
        if icon.is_none() {
            // Gizli girdi kimliği gölgeler (alt öncelikli dizindeki aynı kimlik kullanılmaz).
            self.apps.insert(id, Entry { icon: None });
            return;
        }
        let lower = id.to_lowercase();
        if let Some(class) = main.get("StartupWMClass").map(|c| c.trim().to_lowercase()).filter(|c| !c.is_empty() && c.len() <= MAX_APP_ID_LEN) {
            self.wm_class.entry(class).or_insert_with(|| id.clone());
        }
        if lower.contains('.') {
            self.last.entry(last_component(&lower).to_owned()).or_insert_with(|| id.clone());
        }
        self.lower.entry(lower).or_insert_with(|| id.clone());
        self.apps.insert(id, Entry { icon });
    }
}

/// Ters-DNS adının son bileşeni (`org.gnome.nautilus` → `nautilus`).
pub(crate) fn last_component(id: &str) -> &str {
    id.rsplit('.').next().unwrap_or(id)
}

/// Geçerli `app_id`: kırpılmış, `.desktop` soneki atılmış, 1..=255 bayt, `/` ve denetim
/// karakteri yok, `.` ile başlamaz.
pub(crate) fn clean_app_id(app_id: &str) -> Option<&str> {
    let id = app_id.trim();
    let id = id.strip_suffix(".desktop").unwrap_or(id);
    let ok = !id.is_empty() && id.len() <= MAX_APP_ID_LEN && !id.starts_with('.') && !id.contains('/') && !id.chars().any(char::is_control);
    ok.then_some(id)
}

fn modified(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

/// Sınırlı, bloklamayan okuma; geçersiz UTF-8 ya da özel dosya → `None`.
fn read_small(path: &Path) -> Option<String> {
    let file = crate::decode::open_regular(path).ok()?;
    let bytes = crate::decode::read_limited(file, path, MAX_DESKTOP_BYTES).ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn app(icon: &str, extra: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName=X\nIcon={icon}\n{extra}")
    }

    #[test]
    fn match_rules_in_order() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("applications");
        write(&a.join("firefox.desktop"), &app("firefox", "StartupWMClass=firefox\n"));
        write(&a.join("org.gnome.Nautilus.desktop"), &app("org.gnome.Nautilus", ""));
        write(&a.join("code.desktop"), &app("vscode", "StartupWMClass=Code\n"));
        write(&a.join("com.example.Tool.desktop"), &app("tool-icon", ""));
        write(&a.join("kde4/okular.desktop"), &app("okular", ""));
        let idx = DesktopIndex::scan(std::slice::from_ref(&a));
        assert_eq!(idx.find("firefox"), Some(("firefox", "firefox", DesktopMatch::Exact)));
        assert_eq!(idx.find("Firefox").map(|m| m.2), Some(DesktopMatch::Lowercase));
        assert_eq!(idx.find("Code"), Some(("code", "vscode", DesktopMatch::Lowercase)));
        assert_eq!(idx.find("org.gnome.Nautilus.desktop").map(|m| m.1), Some("org.gnome.Nautilus"));
        // Ters-DNS: kısa adla uzun kimlik ve uzun adla kısa kimlik.
        assert_eq!(idx.find("nautilus").map(|m| (m.0, m.2)), Some(("org.gnome.Nautilus", DesktopMatch::ReverseDns)));
        assert_eq!(idx.find("org.mozilla.firefox").map(|m| (m.0, m.2)), Some(("firefox", DesktopMatch::ReverseDns)));
        assert_eq!(idx.icon_for("kde4-okular"), Some("okular"));
        assert_eq!(idx.icon_for("unknown"), None);
        for bad in ["", "../x", "a/b", ".hidden", "a\nb"] {
            assert_eq!(idx.find(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn wm_class_matches_case_insensitively() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("applications");
        write(&a.join("jetbrains-idea.desktop"), &app("idea", "StartupWMClass=jetbrains-idea-ce\n"));
        let idx = DesktopIndex::scan(std::slice::from_ref(&a));
        assert_eq!(idx.find("JetBrains-IDEA-CE"), Some(("jetbrains-idea", "idea", DesktopMatch::WmClass)));
    }

    #[test]
    fn earlier_dir_wins_and_hidden_masks() {
        let d = tempfile::tempdir().unwrap();
        let (user, sys) = (d.path().join("user/applications"), d.path().join("sys/applications"));
        write(&user.join("a.desktop"), &app("user-a", ""));
        write(&sys.join("a.desktop"), &app("sys-a", ""));
        write(&user.join("b.desktop"), &app("x", "Hidden=true\n"));
        write(&sys.join("b.desktop"), &app("sys-b", ""));
        write(&sys.join("link.desktop"), "[Desktop Entry]\nType=Link\nIcon=web\n");
        let idx = DesktopIndex::scan(&[user, sys]);
        assert_eq!(idx.icon_for("a"), Some("user-a"));
        assert_eq!(idx.icon_for("b"), None, "Hidden=true sistem girdisini gizler");
        assert_eq!(idx.icon_for("link"), None, "Type=Link uygulama değil");
    }

    #[test]
    fn reads_are_bounded() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("applications");
        let mut big = app("big", "");
        big.push_str(&"#".repeat(MAX_DESKTOP_BYTES as usize));
        write(&a.join("big.desktop"), &big);
        write(&a.join("a/b/c/deep.desktop"), &app("deep", ""));
        write(&a.join("a/b/shallow.desktop"), &app("shallow", ""));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&a, a.join("loop")).unwrap();
            rustix::fs::mknodat(rustix::fs::CWD, a.join("fifo.desktop"), rustix::fs::FileType::Fifo, rustix::fs::Mode::from_raw_mode(0o600), 0)
                .unwrap();
        }
        let idx = DesktopIndex::scan(std::slice::from_ref(&a));
        assert_eq!(idx.icon_for("big"), None, "boyut sınırı");
        assert_eq!(idx.icon_for("a-b-c-deep"), None, "derinlik sınırı");
        assert_eq!(idx.icon_for("a-b-shallow"), Some("shallow"));
        assert_eq!(idx.icon_for("fifo"), None, "FIFO bloklamaz, reddedilir");
        assert!(idx.icon_for("loop-a-b-shallow").is_none(), "bağlı dizin izlenmez");
    }

    #[test]
    fn staleness_follows_dir_mtime() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("applications");
        let idx = DesktopIndex::scan(std::slice::from_ref(&a));
        assert!(!idx.is_stale(std::slice::from_ref(&a)));
        write(&a.join("new.desktop"), &app("new", ""));
        assert!(idx.is_stale(std::slice::from_ref(&a)), "dizin oluştu");
        assert!(idx.is_stale(&[]), "dizin listesi değişti");
    }
}
