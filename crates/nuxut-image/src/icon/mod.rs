//! freedesktop ikon teması araması (Icon Theme Specification 0.13).
//!
//! Sıra: kullanıcı teması → `Inherits` zinciri → `hicolor` → pixmaps dizinleri. Her temada önce
//! boyutu tam eşleşen dizin, yoksa en yakın boyutlu dizin seçilir. Dizin içerikleri ilk
//! erişimde bir kez listelenip önbelleklenir; arama sonuçları da (bulunamayanlar dahil)
//! önbelleklenir. [`IconTheme`] `Send + Sync`'tir, [`crate::Loader`] thread'leriyle paylaşılır.

mod desktop;
mod index;
pub(crate) mod installed;
mod settings;
mod sni;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

pub use desktop::desktop_icon_name;
pub use installed::{
    InstalledCursorTheme, InstalledIconTheme, MAX_ENTRIES_PER_DIR, MAX_INDEX_BYTES, MAX_NAME_CHARS, MAX_THEMES,
    cursor_themes_in, icon_themes_in, installed_cursor_themes, installed_icon_themes,
};
use index::ThemeIndex;
pub(crate) use installed::theme_inherits;
pub(crate) use index::parse_ini;
pub use settings::detect_theme_name;
pub use sni::{best_sni_pixmap, sni_pixmap_to_image};

use crate::{DecodeOptions, Fit, Image, ImageError, Limits, TargetSize, Trust};

/// Desteklenen uzantılar, tercih sırasıyla (XPM çözülemediği için yok).
const EXTENSIONS: [&str; 2] = ["png", "svg"];
const FALLBACK_THEME: &str = "hicolor";
/// Seçili tema kurulu değilse (ayar dosyasında adı geçen ama silinmiş tema) GTK gibi önce
/// bu temaya düşülür; `hicolor` uygulama ikonlarını içerir ama `folder` gibi genel adları
/// içermez.
const DEFAULT_THEME: &str = "Adwaita";
/// Arama önbelleği bu kadar girdiyi aşınca temizlenir (sınırsız büyümesin).
const MAX_CACHED_LOOKUPS: usize = 4096;
/// Kabul edilen en uzun ikon adı (D-Bus'tan gelen dış girdi).
const MAX_NAME_LEN: usize = 255;

/// İkon teması çözücüsü.
pub struct IconTheme {
    theme: String,
    base_dirs: Vec<PathBuf>,
    pixmap_dirs: Vec<PathBuf>,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Tema adı → index (bulunamadıysa `None`).
    themes: HashMap<String, Option<Arc<ThemeIndex>>>,
    /// Dizin → dosya adları (yoksa `None`).
    listings: HashMap<PathBuf, Option<Arc<HashSet<String>>>>,
    lookups: HashMap<(String, u32, u32), Option<PathBuf>>,
}

/// Arama dizinleri ev dizinini (kullanıcı adını), önbellek ikon adlarını ve yollarını taşır:
/// yalnızca tema adı ve sayılar yazılır.
impl std::fmt::Debug for IconTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lookups = self.state.lock().map_or(0, |s| s.lookups.len());
        f.debug_struct("IconTheme")
            .field("theme", &self.theme)
            .field("base_dirs", &self.base_dirs.len())
            .field("pixmap_dirs", &self.pixmap_dirs.len())
            .field("lookups", &lookups)
            .finish()
    }
}

impl IconTheme {
    /// Kullanıcının ayarlarından tema adını bulur ([`detect_theme_name`]), standart arama
    /// dizinlerini kullanır.
    pub fn system() -> Self {
        Self::new(detect_theme_name().unwrap_or_else(|| FALLBACK_THEME.to_owned()))
    }

    /// Standart arama dizinleriyle belirli bir tema.
    pub fn new(theme: impl Into<String>) -> Self {
        Self::with_dirs(theme, default_base_dirs(), vec![PathBuf::from("/usr/share/pixmaps")])
    }

    /// Özel arama dizinleri (testler ve gömülü kurulumlar için). `base_dirs` sırası önceliktir.
    pub fn with_dirs(theme: impl Into<String>, base_dirs: Vec<PathBuf>, pixmap_dirs: Vec<PathBuf>) -> Self {
        Self { theme: theme.into(), base_dirs, pixmap_dirs, state: Mutex::new(State::default()) }
    }

    pub fn theme_name(&self) -> &str {
        &self.theme
    }

    /// Seçili tema arama dizinlerinde kurulu mu (değilse `Adwaita` → `hicolor` kullanılır).
    pub fn is_installed(&self) -> bool {
        self.theme_index(&self.theme).is_some()
    }

    /// Tüm önbellekleri boşaltır (tema dosyaları değişince, ör. paket kurulumu).
    pub fn clear_cache(&self) {
        *self.lock() = State::default();
    }

    /// `name` ikonunun dosyası. `size` mantıksal piksel, `scale` tamsayı ölçek (1, 2…).
    /// `name` mutlak yol ya da `file://` URI ise doğrudan o dosya (varsa) döner.
    pub fn lookup(&self, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_NAME_LEN {
            return None;
        }
        if let Some(path) = as_path(name) {
            return path.is_file().then_some(path);
        }
        // "firefox.png" gibi uzantılı adlar yaygın bir hata; uzantı atılır.
        let name = EXTENSIONS.iter().find_map(|e| name.strip_suffix(&format!(".{e}"))).unwrap_or(name);
        if !valid_name(name) {
            return None;
        }
        let (size, scale) = (size.max(1), scale.max(1));
        let key = (name.to_owned(), size, scale);
        if let Some(hit) = self.lock().lookups.get(&key) {
            return hit.clone();
        }
        let found = self.find(name, size, scale);
        let mut st = self.lock();
        if st.lookups.len() >= MAX_CACHED_LOOKUPS {
            st.lookups.clear();
        }
        st.lookups.insert(key, found.clone());
        found
    }

    /// [`lookup`](Self::lookup), bulunamazsa ad sondan kısaltılarak denenir
    /// (`network-wireless-signal-good` → `network-wireless-signal` → …).
    /// Yol/`file://` adlarda kısaltma yapılmaz.
    pub fn lookup_with_fallbacks(&self, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        let mut name = name.trim();
        if as_path(name).is_some() {
            return self.lookup(name, size, scale);
        }
        loop {
            if let Some(p) = self.lookup(name, size, scale) {
                return Some(p);
            }
            name = name.rsplit_once('-')?.0;
        }
    }

    /// Güvensiz varsayılanla (`Untrusted`: SVG çözülmez) [`load_as`](Self::load_as).
    #[deprecated(note = "güveni açıkça verin: load_as(name, size, scale, Trust)")]
    pub fn load(&self, name: &str, size: u32, scale: u32) -> Result<Image, ImageError> {
        self.load_as(name, size, scale, Trust::Untrusted)
    }

    /// Bul ve `size × scale` fiziksel piksele (en fazla; en-boy korunur) çöz. Dosyanın güveni
    /// isteğinkini asla aşmaz: `Untrusted` istekten çözülen dosya her zaman `Untrusted`'tır.
    /// `System` istekte yalnızca geçerli bir tema adıyla ev dizini dışındaki bir arama
    /// dizininde bulunan dosya `System` kalır; ad bir yol/`file://` ise ya da dosya
    /// kullanıcının yazabildiği bir dizindeyse (`~/.icons`, `~/.local/share/icons`) ya da
    /// Flatpak/Snap dışa aktarımıysa `Untrusted`'a düşer. Güven açılmış dosyanın kendisinden
    /// (fd) denetlenir ve aynı fd'den çözülür.
    pub fn load_as(&self, name: &str, size: u32, scale: u32, trust: Trust) -> Result<Image, ImageError> {
        let (file, path, opts) = self.open_as(name, size, scale, trust)?;
        crate::decode::decode_open_file(file, &path, &opts).map(crate::Decoded::into_first_frame)
    }

    /// [`load_as`](Self::load_as)'ın ilk yarısı: dosyayı bulur, açar ve etkin güveni (fd'den)
    /// belirler. Dönen seçeneklerin güveni `Untrusted` ise [`crate::Loader`] baytları yalıtılmış
    /// işçiye verir (ADR-0007).
    pub(crate) fn open_as(&self, name: &str, size: u32, scale: u32, trust: Trust) -> Result<(std::fs::File, PathBuf, DecodeOptions), ImageError> {
        let path = self.lookup_with_fallbacks(name, size, scale).ok_or_else(|| ImageError::NotFound(name.to_owned()))?;
        let px = size.max(1) * scale.max(1);
        let file = crate::decode::open_regular(&path)?;
        let trust = match trust {
            Trust::System if as_path(name.trim()).is_none() => {
                let dirs: Vec<PathBuf> = self.base_dirs.iter().chain(&self.pixmap_dirs).cloned().collect();
                if system_origin(&file, &dirs, dirs::home_dir().as_deref()) { Trust::System } else { Trust::Untrusted }
            }
            _ => Trust::Untrusted,
        };
        Ok((file, path, icon_options(px, trust)))
    }

    /// Adaylardan ilk bulunanı [`load_as`](Self::load_as) ile çözer: önce her ad tam haliyle
    /// (sırayla), hiçbiri yoksa sondan kısaltmalı ([`lookup_with_fallbacks`](Self::lookup_with_fallbacks)).
    /// Önizlemeler için (`folder` → `inode-directory`, `utilities-terminal` → `kitty` …).
    pub fn load_first_as<S: AsRef<str>>(&self, names: &[S], size: u32, scale: u32, trust: Trust) -> Result<Image, ImageError> {
        let name = self.first_name(names, size, scale).ok_or_else(|| ImageError::NotFound(String::new()))?;
        self.load_as(name, size, scale, trust)
    }

    /// [`load_first_as`](Self::load_first_as)'ın seçtiği ad (önce tam adlar, sonra kısaltmalı).
    pub(crate) fn first_name<'a, S: AsRef<str>>(&self, names: &'a [S], size: u32, scale: u32) -> Option<&'a str> {
        let names = || names.iter().map(AsRef::as_ref);
        names()
            .find(|n| self.lookup(n, size, scale).is_some())
            .or_else(|| names().find(|n| self.lookup_with_fallbacks(n, size, scale).is_some()))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Zehirlenmiş kilit: önbellek her zaman tutarlı (yalnızca tam değerler eklenir).
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn find(&self, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        let mut visited = HashSet::new();
        if let Some(p) = self.find_in_theme(&self.theme, name, size, scale, &mut visited) {
            return Some(p);
        }
        if self.theme_index(&self.theme).is_none()
            && let Some(p) = self.find_in_theme(DEFAULT_THEME, name, size, scale, &mut visited)
        {
            return Some(p);
        }
        if let Some(p) = self.find_in_theme(FALLBACK_THEME, name, size, scale, &mut visited) {
            return Some(p);
        }
        // Spec: LookupFallbackIcon — temasız dizinler (`<base>/name.ext`, pixmaps).
        let dirs: Vec<PathBuf> = self.base_dirs.iter().chain(&self.pixmap_dirs).cloned().collect();
        dirs.iter().find_map(|d| self.file_in(d, name))
    }

    fn find_in_theme(
        &self,
        theme: &str,
        name: &str,
        size: u32,
        scale: u32,
        visited: &mut HashSet<String>,
    ) -> Option<PathBuf> {
        if !visited.insert(theme.to_owned()) {
            return None;
        }
        let index = self.theme_index(theme)?;
        if let Some(p) = self.lookup_in(theme, &index, name, size, scale) {
            return Some(p);
        }
        for parent in &index.inherits {
            if let Some(p) = self.find_in_theme(parent, name, size, scale, visited) {
                return Some(p);
            }
        }
        None
    }

    /// Spec: LookupIcon — önce tam eşleşme, sonra en küçük boyut uzaklığı.
    fn lookup_in(&self, theme: &str, index: &ThemeIndex, name: &str, size: u32, scale: u32) -> Option<PathBuf> {
        let roots: Vec<PathBuf> = self.base_dirs.iter().map(|b| b.join(theme)).filter(|r| r.is_dir()).collect();
        for dir in index.dirs.iter().filter(|d| d.matches(size, scale)) {
            if let Some(p) = roots.iter().find_map(|r| self.file_in(&r.join(&dir.path), name)) {
                return Some(p);
            }
        }
        let mut best: Option<(u32, PathBuf)> = None;
        for dir in &index.dirs {
            let dist = dir.distance(size, scale);
            if best.as_ref().is_some_and(|(d, _)| *d <= dist) {
                continue;
            }
            if let Some(p) = roots.iter().find_map(|r| self.file_in(&r.join(&dir.path), name)) {
                best = Some((dist, p));
            }
        }
        best.map(|(_, p)| p)
    }

    fn file_in(&self, dir: &Path, name: &str) -> Option<PathBuf> {
        let listing = self.listing(dir)?;
        EXTENSIONS.iter().map(|e| format!("{name}.{e}")).find(|f| listing.contains(f)).map(|f| dir.join(f))
    }

    fn listing(&self, dir: &Path) -> Option<Arc<HashSet<String>>> {
        if let Some(l) = self.lock().listings.get(dir) {
            return l.clone();
        }
        let listing = std::fs::read_dir(dir).ok().map(|rd| {
            Arc::new(rd.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect::<HashSet<_>>())
        });
        self.lock().listings.insert(dir.to_owned(), listing.clone());
        listing
    }

    fn theme_index(&self, theme: &str) -> Option<Arc<ThemeIndex>> {
        if let Some(t) = self.lock().themes.get(theme) {
            return t.clone();
        }
        let index = if theme.contains('/') || theme.starts_with('.') {
            None
        } else {
            self.base_dirs.iter().find_map(|b| {
                let text = std::fs::read_to_string(b.join(theme).join("index.theme")).ok()?;
                Some(Arc::new(ThemeIndex::parse(&text)))
            })
        };
        if index.is_none() && theme != FALLBACK_THEME {
            tracing::debug!("ikon teması bulunamadı: {theme}");
        }
        self.lock().themes.insert(theme.to_owned(), index.clone());
        index
    }
}

/// Güvensiz varsayılanla (`Untrusted`: SVG çözülmez) [`load_icon_file_as`].
#[deprecated(note = "güveni açıkça verin: load_icon_file_as(path, px, Trust)")]
pub fn load_icon_file(path: &Path, px: u32) -> Result<Image, ImageError> {
    load_icon_file_as(path, px, Trust::Untrusted)
}

/// İkon dosyasını en fazla `px`×`px`'e sığacak şekilde çözer (SVG o boyuta rasterleşir;
/// `Untrusted`'ta SVG çözülmez).
pub fn load_icon_file_as(path: &Path, px: u32, trust: Trust) -> Result<Image, ImageError> {
    crate::decode_file(path, &icon_options(px, trust)).map(crate::Decoded::into_first_frame)
}

fn icon_options(px: u32, trust: Trust) -> DecodeOptions {
    DecodeOptions {
        limits: Limits::ICON.with_trust(trust),
        target: Some(TargetSize::new(px.max(1), px.max(1), Fit::Contain)),
        animate: false,
    }
}

/// Root'a ait olsa da içeriği uygulama yayıncısından gelen dışa aktarma dizinleri: Flatpak
/// (sistem kurulumu) ve Snap ikonları sistem teması sayılmaz.
#[cfg(unix)]
const SANDBOX_EXPORTS: [&str; 3] = ["/var/lib/flatpak", "/var/lib/snapd", "/snap"];

/// Açılmış ikon dosyası sistem teması mı: fd'nin gerçek yolu ev dizini DIŞINDAKİ bir arama
/// dizininin altında, Flatpak/Snap dışa aktarımı değil, dosya (fd'den `fstat`) ve `/`'a kadar
/// her üst dizini root'un, grup/diğerleri yazamıyor. Kullanıcının yazabildiği hiçbir yer
/// (`~/.icons`, `~/.local/share/icons`, `/tmp`, uygulamanın verdiği `IconThemePath` gibi
/// `with_dirs`'e konmuş bir dizin) güveni yükseltemez. Üst dizinleri yalnızca root
/// değiştirebildiği için denetlenen yol, okunacak fd ile aynı dosyada kalır.
#[cfg(unix)]
pub(crate) fn system_origin(file: &std::fs::File, dirs: &[PathBuf], home: Option<&Path>) -> bool {
    use std::os::fd::AsRawFd;
    let Ok(meta) = file.metadata() else { return false };
    if !meta.is_file() || !root_only(&meta) {
        return false;
    }
    // Silinmiş dosyada yolun sonuna " (deleted)" eklenir; üst dizin denetimi o zaman düşer.
    let Ok(path) = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())) else { return false };
    if home.is_some_and(|h| path.starts_with(h)) || sandbox_export(&path) {
        return false;
    }
    let in_search_dir = dirs
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| path.starts_with(&d) && !home.is_some_and(|h| d.starts_with(h)));
    in_search_dir && path.parent().is_some_and(|p| p.ancestors().all(|a| std::fs::metadata(a).is_ok_and(|m| root_only(&m))))
}

/// Unix dışı: sahiplik/izin denetlenemez, tema dosyası hiçbir zaman `System` sayılmaz.
#[cfg(not(unix))]
pub(crate) fn system_origin(_file: &std::fs::File, _dirs: &[PathBuf], _home: Option<&Path>) -> bool {
    false
}

#[cfg(unix)]
fn sandbox_export(path: &Path) -> bool {
    SANDBOX_EXPORTS.iter().any(|p| path.starts_with(p)) || path.components().any(|c| c.as_os_str() == "flatpak")
}

#[cfg(unix)]
fn root_only(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.uid() == 0 && m.mode() & 0o022 == 0
}

/// Spec'teki arama dizinleri: `$HOME/.icons`, `$XDG_DATA_HOME/icons`, `$XDG_DATA_DIRS/icons`.
pub fn default_base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".icons"));
    }
    if let Some(data) = dirs::data_dir() {
        dirs.push(data.join("icons"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty());
    for d in data_dirs.as_deref().unwrap_or("/usr/local/share:/usr/share").split(':') {
        if !d.is_empty() {
            let p = Path::new(d).join("icons");
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    }
    dirs
}

/// Tema adı tek bir dosya adı bileşeni olmalı: `/` (mutlak yol dahil), `..`, NUL ve
/// denetim karakteri içeremez, `.` ile başlayamaz.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains("..") && !name.chars().any(|c| c == '/' || c.is_control())
}

fn as_path(name: &str) -> Option<PathBuf> {
    if let Some(rest) = name.strip_prefix("file://") {
        return Some(PathBuf::from(percent_decode(rest)));
    }
    name.starts_with('/').then(|| PathBuf::from(name))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_dirs_never_raise_trust() {
        let open = |p: &Path| std::fs::File::open(p).unwrap();
        let root = tempfile::tempdir().unwrap();
        let (home, sys) = (root.path().join("ev"), root.path().join("usr/share/icons"));
        let user_icons = home.join(".local/share/icons");
        std::fs::create_dir_all(&user_icons).unwrap();
        std::fs::create_dir_all(&sys).unwrap();
        let (bad, good) = (user_icons.join("x.svg"), sys.join("x.svg"));
        std::fs::write(&bad, "").unwrap();
        std::fs::write(&good, "").unwrap();
        let dirs = vec![user_icons, sys];
        assert!(!system_origin(&open(&bad), &dirs, Some(&home)), "ev dizinindeki dosya System olamaz");
        // Ev dizini dışında ama kullanıcının yazabildiği dizin (uygulamanın IconThemePath'i).
        assert!(!system_origin(&open(&good), &dirs, Some(&home)), "kullanıcıya ait dizin System olamaz");
        // Gerçek sistem dizini: root'un, yalnızca root yazabiliyor.
        let usr = PathBuf::from("/usr/share/icons");
        let f = usr.join("hicolor/index.theme");
        if f.is_file() {
            assert!(system_origin(&open(&f), std::slice::from_ref(&usr), Some(&home)));
            assert!(!system_origin(&open(&f), &dirs, Some(&home)), "arama dizinlerinin dışı");
        }
    }

    #[test]
    fn sandbox_exports_are_not_system() {
        for p in [
            "/var/lib/flatpak/exports/share/icons/hicolor/48x48/apps/org.x.png",
            "/var/lib/flatpak/app/org.x/current/active/export/share/icons/x.svg",
            "/var/lib/snapd/desktop/icons/snap.x.png",
            "/snap/x/1/meta/gui/x.svg",
            "/opt/flatpak/exports/share/icons/x.svg",
        ] {
            assert!(sandbox_export(Path::new(p)), "{p}");
        }
        assert!(!sandbox_export(Path::new("/usr/share/icons/hicolor/48x48/apps/firefox.png")));
    }

    #[test]
    fn names_are_single_components() {
        for ok in ["firefox", "org.kde.foo", "nm-signal-75", "a_b+c"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "../x", "a/b", "/etc/passwd", ".hidden", "a..b", "a\0b", "a\nb", ".."] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn decodes_file_uri() {
        assert_eq!(as_path("file:///tmp/a%20b.png"), Some(PathBuf::from("/tmp/a b.png")));
        assert_eq!(as_path("/x.png"), Some(PathBuf::from("/x.png")));
        assert_eq!(as_path("firefox"), None);
        assert_eq!(percent_decode("%2"), "%2");
    }
}
