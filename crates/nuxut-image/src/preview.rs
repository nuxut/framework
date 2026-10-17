//! Tema önizlemeleri (Ayarlar'daki imleç/ikon teması kartları): temsilci ikon rolleri, istek
//! kurucuları ve küçük, sınırlı bir çözülmüş görsel önbelleği.
//!
//! Akış (tembel): kart ilk kez göründüğünde uygulama her yuva için önce [`PreviewCache::get`]'e
//! bakar, yoksa [`cursor_request`] / [`icon_request`] ile [`crate::Loader`]'a iş verir; sonuç
//! [`PreviewCache::insert`] ile saklanır ve dokuya yüklenir. Boşta hiçbir iş yapılmaz; önbellek
//! LRU'dur ve [`PreviewCache::new`]'deki bayt bütçesini aşmaz (tipik bir kart: 9 imleç × 32² × 4
//! ≈ 37 KiB; varsayılan bütçe [`DEFAULT_PREVIEW_BUDGET`] yüzlerce kartı tutar).
//!
//! Güven (ADR-0007): imleç dosyaları her zaman güvenilmez ayrıştırılır (Xcursor kendi sınırlı
//! ayrıştırıcısı, hyprcursor kuralı [`crate::cursor::hyprcursor`]); ikonlar `Trust::System`
//! istenir ve dosya kullanıcı dizinindeyse (`~/.icons`, `~/.local/share/icons`; genel olarak
//! kök sahipli bir sistem dizini dışındaysa) `Untrusted`'a düşer. Kullanıcı dizinindeki SVG
//! (ikon teması ya da yalnızca-SVG hyprcursor şekli) [`crate::Loader::with_sandbox`] ile
//! kurulmuş yükleyicide yalıtılmış işçide tek kare olarak çözülür; işçi yoksa ya da
//! kullanılamıyorsa (`Unavailable`) süreç içine düşmez, yuva yer tutucu ("yok") gösterir.
//!
//! İş anahtarı: [`PreviewKey::load_key`] ile [`crate::Loader::load_keyed`] (tema+rol başına
//! tek bekleyen iş); tema değişince [`cancel_theme`]; sonuç [`outcome`] ile önbelleğe.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::cursor::{CursorRole, CursorTheme};
use crate::icon::IconTheme;
use crate::{Decoded, Frame, Image, LoadRequest, Trust};

/// Varsayılan önbellek bütçesi (çözülmüş piksel baytı).
pub const DEFAULT_PREVIEW_BUDGET: usize = 8 << 20;

/// İkon teması önizlemesinin temsilci rolleri. Her rolün aday adları [`names`](Self::names)
/// tablosundadır (freedesktop adı → yaygın uygulama/masaüstü adları).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconRole {
    Folder,
    Home,
    Downloads,
    FileManager,
    Terminal,
    Settings,
    WebBrowser,
    TextEditor,
    ImageFile,
    TextFile,
    Music,
    Trash,
}

impl IconRole {
    /// Tüm roller.
    pub const ALL: [Self; 12] = [
        Self::Folder,
        Self::Home,
        Self::Downloads,
        Self::FileManager,
        Self::Terminal,
        Self::Settings,
        Self::WebBrowser,
        Self::TextEditor,
        Self::ImageFile,
        Self::TextFile,
        Self::Music,
        Self::Trash,
    ];

    /// Tema kartı için varsayılan set: klasör, uygulama ve dosya türü ikonlarının karışımı
    /// (bir temanın "karakterini" en iyi gösteren sekiz ikon).
    pub const PREVIEW: [Self; 8] = [
        Self::Folder,
        Self::FileManager,
        Self::Terminal,
        Self::Settings,
        Self::WebBrowser,
        Self::TextEditor,
        Self::ImageFile,
        Self::Trash,
    ];

    /// Aday ikon adları, tercih sırasıyla.
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Self::Folder => &["folder", "inode-directory", "folder-blue"],
            Self::Home => &["user-home", "folder-home", "go-home"],
            Self::Downloads => &["folder-download", "folder-downloads", "emblem-downloads"],
            Self::FileManager => &[
                "system-file-manager",
                "org.gnome.Nautilus",
                "nautilus",
                "org.kde.dolphin",
                "dolphin",
                "thunar",
                "nemo",
                "pcmanfm",
            ],
            Self::Terminal => &[
                "utilities-terminal",
                "org.gnome.Terminal",
                "org.gnome.Console",
                "terminal",
                "kitty",
                "Alacritty",
                "org.kde.konsole",
                "konsole",
                "foot",
            ],
            Self::Settings => &[
                "preferences-system",
                "systemsettings",
                "org.gnome.Settings",
                "gnome-control-center",
                "preferences-desktop",
                "applications-system",
            ],
            Self::WebBrowser => &[
                "web-browser",
                "firefox",
                "org.mozilla.firefox",
                "chromium",
                "google-chrome",
                "applications-internet",
            ],
            Self::TextEditor => &[
                "accessories-text-editor",
                "org.gnome.TextEditor",
                "text-editor",
                "org.kde.kate",
                "kate",
                "gedit",
                "mousepad",
            ],
            Self::ImageFile => &["image-x-generic", "image-png", "image"],
            Self::TextFile => &["text-x-generic", "text-plain", "text"],
            Self::Music => &["audio-x-generic", "folder-music", "audio"],
            Self::Trash => &["user-trash", "user-trash-full", "trash-empty"],
        }
    }
}

/// İmleç önizleme isteği: `size` fiziksel piksel nominal boyut, yalnızca ilk kare (`animate`
/// ise animasyonlu imlecin tüm kareleri), hedefe sığdırılır (büyütülmez).
pub fn cursor_request(theme: Arc<CursorTheme>, role: CursorRole, size: u32, animate: bool) -> LoadRequest {
    let mut r = LoadRequest::cursor(theme, role, size, animate);
    let side = size.max(1);
    r.options.target = Some(crate::TargetSize::new(side, side, crate::Fit::Contain));
    r
}

/// İkon önizleme isteği: `size` mantıksal piksel, `scale` tamsayı ölçek; adaylardan ilk
/// bulunan (önce tam adlar, sonra kısaltmalı). `Trust::System` istenir; sistem dizini dışındaki
/// dosya `Untrusted`'a düşer ve SVG ise yalnızca yalıtılmış işçide çözülür (modül belgesi).
pub fn icon_request(theme: Arc<IconTheme>, role: IconRole, size: u32, scale: u32) -> LoadRequest {
    let names: Arc<[String]> = role.names().iter().map(|s| (*s).to_owned()).collect();
    LoadRequest::icon_any(theme, names, size, scale).with_trust(Trust::System)
}

/// Önbellek anahtarı: (tür, tema adı, rol, fiziksel piksel).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PreviewKey {
    Cursor { theme: String, role: CursorRole, px: u32 },
    Icon { theme: String, role: IconRole, px: u32 },
}

impl PreviewKey {
    fn theme(&self) -> &str {
        match self {
            Self::Cursor { theme, .. } | Self::Icon { theme, .. } => theme,
        }
    }

    /// [`crate::Loader::load_keyed`] anahtarı: tür + tema + rol özeti (piksel boyutu hariç),
    /// hiçbir zaman [`crate::NO_KEY`] değil. Aynı yuvanın yeni isteği (ör. ölçek değişti) eskisini
    /// iptal eder; yalıtılmış çözücü art arda başarısız olan tema+rolü susturur.
    ///
    /// **Dikkat:** anahtar yalnızca tür+tema+rolden türer. Aynı `Loader`'ı paylaşan iki tüketici
    /// (ör. iki ayrı önizleme ızgarası) aynı yuvayı isterse aynı iptal anahtarını üretir ve
    /// birbirinin işini iptal eder (biri `Err(Cancelled)` alır). Böyle tüketiciler ayrı `Loader`
    /// kullanmalı ya da [`crate::Loader::load_with_keys`] ile iptal anahtarına kendi kimliğini
    /// katmalı (kaynak anahtarı `load_key` kalabilir: susturma ortak olur).
    pub fn load_key(&self) -> u64 {
        match self {
            Self::Cursor { theme, role, .. } => load_key(b'c', theme, role.names()[0]),
            Self::Icon { theme, role, .. } => load_key(b'i', theme, role.names()[0]),
        }
    }
}

/// FNV-1a 64 (süreçler ve sürümler arasında kararlı; kriptografik değil, yalnızca birleştirme
/// ve susturma kimliği). Sıfır çıkarsa 1.
fn load_key(kind: u8, theme: &str, role: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [kind, 0].into_iter().chain(theme.bytes()).chain([0]).chain(role.bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    if h == crate::NO_KEY { 1 } else { h }
}

/// Bir temanın (imleç ve ikon, tüm roller) bekleyen önizleme işlerini iptal eder: kart başka
/// temaya geçince ya da kaldırılınca çağrılır. İptal edilen işlerin callback'i
/// `Err(Cancelled)` alır ([`outcome`] onu önbelleğe yazdırmaz).
pub fn cancel_theme(loader: &crate::Loader, theme: &str) {
    for role in CursorRole::ALL {
        loader.cancel_key(PreviewKey::Cursor { theme: theme.to_owned(), role, px: 0 }.load_key());
    }
    for role in IconRole::ALL {
        loader.cancel_key(PreviewKey::Icon { theme: theme.to_owned(), role, px: 0 }.load_key());
    }
}

/// Çözme sonucunu önbellek kaydına çevirir: `Some(entry)` saklanır (`Some(None)` = bilinen
/// "yok": temada yok, çözülemedi, işçi yok/öldü); `None` geçici bir sonuçtur (iptal, işçinin
/// hız sınırı/susturması) — saklanmaz, yuva yer tutucu kalır ve sonraki istekte yeniden denenir.
pub fn outcome(result: Result<Decoded, crate::ImageError>) -> Option<PreviewEntry> {
    use crate::{ImageError, SandboxFailure};
    match result {
        Ok(d) => Some(Some(PreviewImage::from_decoded(d))),
        Err(ImageError::Cancelled | ImageError::Sandbox(SandboxFailure::RateLimited)) => None,
        Err(_) => Some(None),
    }
}

/// Önizleme görseli: kareler (en az bir; animasyonlu imleçte birden çok). `Send + Sync`,
/// ucuz klonlanır (önbellek ile çözme thread'i arasında paylaşılır).
#[derive(Debug, Clone)]
pub struct PreviewImage {
    pub frames: Arc<[Frame]>,
}

impl PreviewImage {
    /// Çözülmüş görselden; akıtılan animasyonun yalnızca ilk karesi alınır (önizleme).
    pub fn from_decoded(d: Decoded) -> Self {
        let frames: Arc<[Frame]> = match d {
            Decoded::Still(image) => Arc::from([Frame { image, delay: std::time::Duration::ZERO }]),
            Decoded::Animated(a) => Arc::from(a.frames),
            Decoded::Streamed(s) => Arc::from([s.first_frame().clone()]),
        };
        Self { frames }
    }

    pub fn first(&self) -> &Image {
        &self.frames[0].image
    }

    pub fn byte_size(&self) -> usize {
        self.frames.iter().map(|f| f.image.pixels().len()).sum()
    }
}

/// Bir yuvanın sonucu: görsel ya da "temada yok/çözülemedi" (yeniden denenmez).
pub type PreviewEntry = Option<PreviewImage>;

/// Sınırlı LRU önizleme önbelleği (`Send + Sync`; çözme thread'lerinden de yazılabilir).
pub struct PreviewCache {
    budget: usize,
    inner: Mutex<Lru>,
}

#[derive(Default)]
struct Lru {
    /// En yeni sonda.
    items: VecDeque<(PreviewKey, PreviewEntry)>,
    bytes: usize,
}

impl std::fmt::Debug for PreviewCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let l = self.lock();
        write!(f, "PreviewCache({} öğe, {}/{} bayt)", l.items.len(), l.bytes, self.budget)
    }
}

impl Default for PreviewCache {
    fn default() -> Self {
        Self::new(DEFAULT_PREVIEW_BUDGET)
    }
}

fn entry_bytes(e: &PreviewEntry) -> usize {
    // "Yok" kaydı da az da olsa yer tutar (sınırsız sayıda birikmesin).
    e.as_ref().map_or(64, |d| d.byte_size().max(64))
}

impl PreviewCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self { budget: budget_bytes, inner: Mutex::new(Lru::default()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lru> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Kayıt (varsa en yeni yapılır): `Some(None)` = bilinen "yok".
    pub fn get(&self, key: &PreviewKey) -> Option<PreviewEntry> {
        let mut l = self.lock();
        let i = l.items.iter().position(|(k, _)| k == key)?;
        let item = l.items.remove(i)?;
        let value = item.1.clone();
        l.items.push_back(item);
        Some(value)
    }

    /// Ekler/değiştirir; bütçe aşılırsa en eskiler atılır (bütçeden büyük tek görsel saklanmaz).
    pub fn insert(&self, key: PreviewKey, entry: PreviewEntry) {
        let size = entry_bytes(&entry);
        let mut l = self.lock();
        if let Some(i) = l.items.iter().position(|(k, _)| *k == key)
            && let Some(old) = l.items.remove(i)
        {
            l.bytes -= entry_bytes(&old.1);
        }
        if size > self.budget {
            return;
        }
        while l.bytes + size > self.budget {
            let Some(old) = l.items.pop_front() else { break };
            l.bytes -= entry_bytes(&old.1);
        }
        l.bytes += size;
        l.items.push_back((key, entry));
    }

    /// Bir temanın tüm kayıtlarını atar (tema dosyaları değişti / yeniden tarama).
    pub fn forget_theme(&self, theme: &str) {
        let mut l = self.lock();
        let before = std::mem::take(&mut l.items);
        l.bytes = 0;
        for (k, v) in before {
            if k.theme() != theme {
                l.bytes += entry_bytes(&v);
                l.items.push_back((k, v));
            }
        }
    }

    pub fn clear(&self) {
        let mut l = self.lock();
        l.items.clear();
        l.bytes = 0;
    }

    /// Tutulan bayt.
    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    pub fn len(&self) -> usize {
        self.lock().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn still(side: u32) -> PreviewEntry {
        Some(PreviewImage::from_decoded(Decoded::Still(Image::from_premultiplied(side, side, vec![0; (side * side * 4) as usize]).unwrap())))
    }

    fn key(theme: &str, px: u32) -> PreviewKey {
        PreviewKey::Icon { theme: theme.into(), role: IconRole::Folder, px }
    }

    #[test]
    fn lru_respects_budget_and_recency() {
        let c = PreviewCache::new(3 * 16 * 16 * 4);
        for px in 0..3 {
            c.insert(key("a", px), still(16));
        }
        assert!(c.get(&key("a", 0)).is_some(), "0 en yeni olur");
        c.insert(key("a", 3), still(16));
        assert!(c.get(&key("a", 1)).is_none(), "en eski atıldı");
        assert!(c.get(&key("a", 0)).is_some());
        assert!(c.bytes() <= 3 * 16 * 16 * 4);
        // Bütçeden büyük görsel saklanmaz; aynı anahtar değiştirilir.
        c.insert(key("b", 0), still(64));
        assert!(c.get(&key("b", 0)).is_none());
        c.insert(key("a", 0), None);
        assert!(matches!(c.get(&key("a", 0)), Some(None)), "bilinen yok");
        c.forget_theme("a");
        assert!(c.is_empty() && c.bytes() == 0);
    }

    #[test]
    fn cache_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PreviewCache>();
        assert_send_sync::<PreviewImage>();
    }

    #[test]
    fn roles_have_valid_unique_names() {
        let mut seen = std::collections::HashSet::new();
        for r in IconRole::ALL {
            assert!(r.names().iter().all(|n| crate::icon::valid_name(n)), "{r:?}");
            assert!(seen.insert(r.names()[0]));
        }
    }

    #[test]
    fn load_keys_are_nonzero_stable_and_ignore_px() {
        let mut seen = std::collections::HashSet::new();
        for theme in ["Papirus", "Papirus-Dark", "Bibata"] {
            for role in IconRole::ALL {
                let k = PreviewKey::Icon { theme: theme.into(), role, px: 24 }.load_key();
                assert_ne!(k, crate::NO_KEY);
                assert_eq!(k, PreviewKey::Icon { theme: theme.into(), role, px: 48 }.load_key(), "px anahtara girmez");
                assert!(seen.insert(k), "{theme} {role:?}");
            }
            for role in CursorRole::ALL {
                assert!(seen.insert(PreviewKey::Cursor { theme: theme.into(), role, px: 24 }.load_key()));
            }
        }
        // Kararlı: sürümler arasında değişmez (susturma kimliği).
        assert_eq!(load_key(b'i', "", ""), 0x2b1e_cf19_2b66_fbc4);
    }

    #[test]
    fn outcome_keeps_transient_results_out_of_the_cache() {
        use crate::{ImageError, SandboxFailure};
        assert!(outcome(Err(ImageError::Cancelled)).is_none());
        assert!(outcome(Err(ImageError::Sandbox(SandboxFailure::RateLimited))).is_none());
        assert!(matches!(outcome(Err(ImageError::Sandbox(SandboxFailure::Unavailable))), Some(None)));
        assert!(matches!(outcome(Err(ImageError::Unsupported("SVG"))), Some(None)));
        assert!(matches!(outcome(Ok(Decoded::Still(Image::from_premultiplied(1, 1, vec![0; 4]).unwrap()))), Some(Some(_))));
    }

    #[test]
    fn cancel_theme_cancels_pending_preview_jobs() {
        let mut loader = crate::Loader::new(1);
        let (tx, rx) = std::sync::mpsc::channel();
        // Thread'i meşgul tut: önizleme işleri kuyrukta beklesin.
        let gate = Arc::new(Mutex::new(()));
        let held = gate.lock().unwrap();
        let g = gate.clone();
        loader.load(LoadRequest::path("/nonexistent", crate::DecodeOptions::default()), move |_| drop(g.lock()));
        let theme = Arc::new(IconTheme::with_dirs("A", Vec::new(), Vec::new()));
        for (name, role) in [("A", IconRole::Folder), ("B", IconRole::Folder), ("A", IconRole::Trash)] {
            let key = PreviewKey::Icon { theme: name.into(), role, px: 24 };
            let tx = tx.clone();
            loader.load_keyed(key.load_key(), icon_request(theme.clone(), role, 24, 1), move |r| {
                tx.send((name, role, outcome(r).is_none())).unwrap();
            });
        }
        cancel_theme(&loader, "A");
        drop(held);
        let mut got: Vec<_> = (0..3).map(|_| rx.recv().unwrap()).filter(|g| g.2).map(|g| (g.0, g.1)).collect();
        got.sort_by_key(|g| format!("{g:?}"));
        assert_eq!(got, [("A", IconRole::Folder), ("A", IconRole::Trash)], "yalnızca A'nın işleri iptal");
    }

    #[test]
    fn icon_request_uses_candidates() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("T/48x48/apps");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.path().join("T/index.theme"), "[Icon Theme]\nName=T\nDirectories=48x48/apps\n[48x48/apps]\nSize=48\n").unwrap();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(48, 48, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        std::fs::write(dir.join("kitty.png"), png).unwrap();
        let theme = Arc::new(IconTheme::with_dirs("T", vec![root.path().to_owned()], Vec::new()));
        let got = icon_request(theme.clone(), IconRole::Terminal, 24, 2).run().unwrap();
        assert_eq!(got.size(), (48, 48));
        assert!(icon_request(theme, IconRole::Trash, 24, 1).run().is_err());
    }
}
