//! Uygulama ikonları: pencerenin `app_id`'sinden (Wayland `app_id`, Hyprland `class`, bildirim
//! `desktop-entry`) freedesktop tema ikonuna. Tasarım: `docs/design/app-icons.md`.
//!
//! Akış: [`AppIcons::get`] UI thread'inde yalnızca önbelleğe bakar (IO yok); yoksa işi tek,
//! düşük öncelikli arka plan thread'ine verir ve [`AppIcon::Pending`] döner. Thread:
//! 1. `.desktop` dizinini tembel tarar ([`DesktopIndex`]; sınırlı) ve `app_id`'yi eşler:
//!    tam ad → küçük harf → `StartupWMClass` → ters-DNS sonu (Flatpak dışa aktarım dizinleri
//!    arama yolundadır);
//! 2. aday adları (`Icon=`, Steam `steam_icon_<n>`, `app_id`'nin kendisi, küçük harfi, ters-DNS
//!    sonu) seçili ikon temasında tam adla arar ([`IconTheme`]: kullanıcının teması →
//!    `Inherits` → `hicolor` → pixmaps; boyut/ölçek seçimi, PNG ve SVG); `Icon=` mutlak yol
//!    olabilir;
//! 3. dosyayı `Trust::System` isteğiyle çözer: güven yalnızca düşer (ev dizini, Flatpak/Snap
//!    dışa aktarımı, mutlak yol → `Untrusted`) ve `Untrusted` dosya verilen yalıtılmış
//!    çözücüde çözülür (ADR-0007; yoksa SVG reddedilir, raster süreç içinde);
//! 4. sonucu önbelleğe yazar ve `notify`'ı çağırır (uygulama bir mesajla döngüyü uyandırıp
//!    yeniden çizer).
//!
//! Önbellek sınırlı LRU'dur ([`MAX_CACHED_ICONS`], [`MAX_CACHED_BYTES`]); bulunamayanlar da
//! (negatif) saklanır. Tema değişince ([`AppIcons::set_theme_name`]) ya da
//! [`AppIcons::refresh`] ile hepsi düşer (nesil artar; eski nesilden gelen sonuç atılır).
//! `applications` dizinlerinden biri değişince (uygulama kuruldu/kaldırıldı) dizin yeniden
//! taranır ve negatif sonuçlar silinir. Boşta iş yok: thread kuyrukta bekler, timer/izleyici
//! yoktur.

mod desktop;
#[cfg(feature = "render")]
mod textures;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

pub use desktop::{DesktopIndex, DesktopMatch, MAX_APP_ID_LEN, MAX_DESKTOP_BYTES, MAX_DESKTOP_DEPTH, MAX_DESKTOP_FILES};
#[cfg(feature = "render")]
pub use textures::{AppIconTextures, MAX_APP_ICON_TEXTURES};

use crate::{Image, IconTheme, IsolatedDecoder, LoadRequest, Trust};

/// Önbellekteki en fazla ikon (bulunamayanlar dahil).
pub const MAX_CACHED_ICONS: usize = 256;
/// Önbellekteki çözülmüş piksellerin bayt bütçesi.
pub const MAX_CACHED_BYTES: usize = 8 << 20;
/// Bekleyen en fazla iş; doluysa istek reddedilir ve sonraki `get`'te yeniden denenir.
pub const APP_ICON_QUEUE: usize = 64;
/// Kabul edilen mantıksal boyut ve ölçek aralıkları (hedef ≤ 256 × 4 fiziksel px).
pub const MAX_APP_ICON_SIZE: u32 = 256;
pub const MAX_APP_ICON_SCALE: u32 = 4;

/// Bir ikon isteğinin şu anki durumu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppIcon {
    /// Arka planda çözülüyor; bitince `notify` çağrılır.
    Pending,
    /// Hazır (premultiplied RGBA8, en fazla `size × scale` kare; en-boy korunur).
    Ready(Arc<Image>),
    /// Ne masaüstü girdisi ne tema ikonu bulundu, ya da çözülemedi: çağıran genel ikonu
    /// (ör. `IconName::Window`) çizer.
    Missing,
}

/// Arama dizinleri. [`AppIconDirs::from_env`] XDG ortamından kurar; testler ve gömülü
/// kurulumlar kendi dizinlerini verebilir.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct AppIconDirs {
    /// `…/applications` dizinleri, öncelik sırasıyla.
    pub applications: Vec<PathBuf>,
    /// İkon teması taban dizinleri (`…/icons`), öncelik sırasıyla.
    pub icons: Vec<PathBuf>,
    /// Temasız yedek dizinler (`/usr/share/pixmaps`).
    pub pixmaps: Vec<PathBuf>,
}

/// Dizinler ev dizinini taşır: yalnızca sayılar.
impl std::fmt::Debug for AppIconDirs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppIconDirs")
            .field("applications", &self.applications.len())
            .field("icons", &self.icons.len())
            .field("pixmaps", &self.pixmaps.len())
            .finish()
    }
}

/// Flatpak sistem kurulumunun dışa aktarım kökü (her zaman eklenir; yoksa taranmaz).
const FLATPAK_SYSTEM_EXPORTS: &str = "/var/lib/flatpak/exports/share";

impl AppIconDirs {
    /// `$HOME`, `$XDG_DATA_HOME`, `$XDG_DATA_DIRS` ortam değişkenlerinden.
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok();
        Self::from_vars(dirs::home_dir().as_deref(), var("XDG_DATA_HOME").as_deref(), var("XDG_DATA_DIRS").as_deref())
    }

    /// Spec'in kuralları, değerler verilerek: göreli `XDG_DATA_HOME` yok sayılır
    /// (`~/.local/share`), boş `XDG_DATA_DIRS` → `/usr/local/share:/usr/share`. Flatpak dışa
    /// aktarım dizinleri (`<data_home>/flatpak/exports/share`, `/var/lib/flatpak/exports/share`)
    /// listede yoksa sona eklenir.
    pub fn from_vars(home: Option<&Path>, data_home: Option<&str>, data_dirs: Option<&str>) -> Self {
        let data_home = data_home
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.map(|h| h.join(".local/share")));
        let mut shares: Vec<PathBuf> = data_home.iter().cloned().collect();
        let data_dirs = data_dirs.filter(|v| !v.is_empty()).unwrap_or("/usr/local/share:/usr/share");
        shares.extend(data_dirs.split(':').map(PathBuf::from).filter(|p| p.is_absolute()));
        shares.extend(data_home.map(|h| h.join("flatpak/exports/share")));
        shares.push(PathBuf::from(FLATPAK_SYSTEM_EXPORTS));
        let mut uniq: Vec<PathBuf> = Vec::new();
        for s in shares {
            if !uniq.contains(&s) {
                uniq.push(s);
            }
        }
        let mut icons: Vec<PathBuf> = home.map(|h| h.join(".icons")).into_iter().collect();
        icons.extend(uniq.iter().map(|s| s.join("icons")));
        Self {
            applications: uniq.iter().map(|s| s.join("applications")).collect(),
            icons,
            pixmaps: vec![PathBuf::from("/usr/share/pixmaps")],
        }
    }
}

/// [`AppIcons`] kurulum seçenekleri.
#[derive(Debug, Clone, Default)]
pub struct AppIconOptions {
    /// Güveni `Untrusted`'a düşen ikonların yalıtılmış çözücüsü (ADR-0007). `None`: o ikonlarda
    /// SVG reddedilir, raster süreç içinde çözülür.
    pub sandbox: Option<Arc<dyn IsolatedDecoder>>,
    /// İkon teması adı (`framework.toml` `[desktop] icon_theme`). `None`: kullanıcının GTK/Qt
    /// ayarlarından ([`crate::icon::detect_theme_name`]), o da yoksa `hicolor`.
    pub theme: Option<String>,
    /// Arama dizinleri; `None` = [`AppIconDirs::from_env`] (ilk istekte, arka planda).
    pub dirs: Option<AppIconDirs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    app_id: String,
    size: u32,
    scale: u32,
}

#[derive(Debug, Clone)]
enum Slot {
    Pending,
    Ready(Arc<Image>),
    Missing,
}

#[derive(Debug)]
struct CacheEntry {
    slot: Slot,
    used: u64,
}

#[derive(Debug, Default)]
struct State {
    generation: u64,
    /// İstenen tema adı (`None` = algıla).
    theme: Option<String>,
    map: HashMap<Key, CacheEntry>,
    tick: u64,
    bytes: usize,
}

impl State {
    fn touch(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    fn clear(&mut self) {
        self.generation += 1;
        self.map.clear();
        self.bytes = 0;
    }

    /// İş sonucunu yazar (yalnızca hâlâ bekleyen girdiye) ve sınırları korur.
    fn complete(&mut self, key: &Key, image: Option<Image>) {
        let used = self.touch();
        let Some(entry) = self.map.get_mut(key) else { return };
        if !matches!(entry.slot, Slot::Pending) {
            return;
        }
        entry.used = used;
        entry.slot = match image {
            Some(img) => {
                self.bytes += img.byte_size();
                Slot::Ready(Arc::new(img))
            }
            None => Slot::Missing,
        };
        self.evict();
    }

    /// En eski kullanılan, bekleyen olmayan girdileri sınırlar altına inene dek atar.
    fn evict(&mut self) {
        while self.map.len() > MAX_CACHED_ICONS || self.bytes > MAX_CACHED_BYTES {
            let victim = self.map.iter().filter(|(_, e)| !matches!(e.slot, Slot::Pending)).min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone());
            let Some(victim) = victim else { return };
            if let Some(CacheEntry { slot: Slot::Ready(img), .. }) = self.map.remove(&victim) {
                self.bytes -= img.byte_size();
            }
        }
    }

    /// Yeni kurulan uygulamalar görünsün diye negatif sonuçlar düşer.
    fn forget_missing(&mut self) {
        self.map.retain(|_, e| !matches!(e.slot, Slot::Missing));
    }
}

struct Shared {
    state: Mutex<State>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        // Zehirlenmiş kilit: durum her adımda tutarlı (tam değerler yazılır).
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Job {
    key: Key,
    generation: u64,
}

struct WorkerConfig {
    sandbox: Option<Arc<dyn IsolatedDecoder>>,
    dirs: Option<AppIconDirs>,
    notify: Box<dyn FnMut() + Send>,
}

enum Worker {
    /// Henüz başlamadı (ilk istekte başlar: runtime'ın sinyal maskesini miras alsın diye).
    Idle(WorkerConfig),
    Running { tx: SyncSender<Job>, thread: JoinHandle<()> },
    /// Başlatılamadı: her istek `Missing`.
    Failed,
}

struct Inner {
    shared: Arc<Shared>,
    worker: Mutex<Worker>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let worker = std::mem::replace(self.worker.get_mut().unwrap_or_else(|e| e.into_inner()), Worker::Failed);
        if let Worker::Running { tx, thread } = worker {
            drop(tx);
            if thread.join().is_err() {
                tracing::error!("uygulama ikonu thread'i panikle sonlandı");
            }
        }
    }
}

/// `app_id` → ikon çözücüsü. Ucuz klonlanır, uygulama genelinde bir tane paylaşılır
/// (`Send + Sync`); çıkış/yüzey başına dokular [`AppIconTextures`]'ta (`render` özelliği).
#[derive(Clone)]
pub struct AppIcons {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for AppIcons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.inner.shared.lock();
        f.debug_struct("AppIcons").field("generation", &st.generation).field("cached", &st.map.len()).field("bytes", &st.bytes).finish()
    }
}

impl AppIcons {
    /// `notify`: bir ikon çözüldüğünde (bulunamadığında da) arka plan thread'inden çağrılır;
    /// tipik olarak `let _ = sender.send(Msg::AppIcon)`. Hiçbir IO yapmaz; thread ilk
    /// istekte başlar.
    pub fn new(options: AppIconOptions, notify: impl FnMut() + Send + 'static) -> Self {
        let state = State { theme: options.theme.filter(|t| valid_theme(t)), ..State::default() };
        let shared = Arc::new(Shared { state: Mutex::new(state) });
        let config = WorkerConfig { sandbox: options.sandbox, dirs: options.dirs, notify: Box::new(notify) };
        Self { inner: Arc::new(Inner { shared, worker: Mutex::new(Worker::Idle(config)) }) }
    }

    /// `app_id`'nin `size` (mantıksal px) × `scale` (tamsayı ölçek; kesirli ölçekte yukarı
    /// yuvarla) ikonu. UI thread'inde çağrılır: yalnızca önbelleğe bakar, yoksa iş kuyruğa
    /// girer. Geçersiz `app_id` (boş, `/`, denetim karakteri, > 255 bayt) → `Missing`.
    pub fn get(&self, app_id: &str, size: u32, scale: u32) -> AppIcon {
        let Some(id) = desktop::clean_app_id(app_id) else { return AppIcon::Missing };
        let key = Key { app_id: id.to_owned(), size: size.clamp(1, MAX_APP_ICON_SIZE), scale: scale.clamp(1, MAX_APP_ICON_SCALE) };
        let generation = {
            let mut st = self.inner.shared.lock();
            let used = st.touch();
            if let Some(e) = st.map.get_mut(&key) {
                e.used = used;
                return match &e.slot {
                    Slot::Pending => AppIcon::Pending,
                    Slot::Ready(img) => AppIcon::Ready(Arc::clone(img)),
                    Slot::Missing => AppIcon::Missing,
                };
            }
            st.map.insert(key.clone(), CacheEntry { slot: Slot::Pending, used });
            st.generation
        };
        match self.submit(Job { key: key.clone(), generation }) {
            Ok(()) => AppIcon::Pending,
            Err(outcome) => {
                let mut st = self.inner.shared.lock();
                if st.generation == generation {
                    st.map.remove(&key);
                }
                outcome
            }
        }
    }

    /// İkon teması adını değiştirir (`None` = kullanıcının ayarlarından algıla). Değiştiyse
    /// tüm önbellek düşer, nesil artar ve `true` döner; doku önbellekleri bir sonraki
    /// erişimde temizlenir. Geçersiz ad (`/`, `.` ile başlayan) `None` sayılır.
    pub fn set_theme_name(&self, theme: Option<&str>) -> bool {
        let theme = theme.filter(|t| valid_theme(t)).map(str::to_owned);
        let mut st = self.inner.shared.lock();
        if st.theme == theme {
            return false;
        }
        st.theme = theme;
        st.clear();
        true
    }

    /// Her şeyi yeniden çözer: tema yeniden algılanır, `.desktop` dizini yeniden taranır
    /// (ör. kullanıcının GTK ikon teması değişti, ikon paketi kuruldu).
    pub fn refresh(&self) {
        self.inner.shared.lock().clear();
    }

    /// Önbellek nesli: her geçersizleştirmede artar (doku önbellekleri buna bakar).
    pub fn generation(&self) -> u64 {
        self.inner.shared.lock().generation
    }

    /// Önbellekteki girdi sayısı ve çözülmüş piksel baytı (tanı/test).
    pub fn cache_stats(&self) -> (usize, usize) {
        let st = self.inner.shared.lock();
        (st.map.len(), st.bytes)
    }

    fn submit(&self, job: Job) -> Result<(), AppIcon> {
        let mut worker = self.inner.worker.lock().unwrap_or_else(|e| e.into_inner());
        if let Worker::Idle(_) = &*worker {
            let Worker::Idle(config) = std::mem::replace(&mut *worker, Worker::Failed) else { unreachable!() };
            *worker = spawn(Arc::clone(&self.inner.shared), config);
        }
        let Worker::Running { tx, .. } = &*worker else { return Err(AppIcon::Missing) };
        match tx.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                tracing::debug!(capacity = APP_ICON_QUEUE, "uygulama ikonu kuyruğu dolu; sonra yeniden denenecek");
                Err(AppIcon::Pending)
            }
            Err(TrySendError::Disconnected(_)) => Err(AppIcon::Missing),
        }
    }
}

fn spawn(shared: Arc<Shared>, config: WorkerConfig) -> Worker {
    let (tx, rx) = std::sync::mpsc::sync_channel(APP_ICON_QUEUE);
    match std::thread::Builder::new().name("nuxut-app-icons".into()).spawn(move || run(&rx, &shared, config)) {
        Ok(thread) => Worker::Running { tx, thread },
        Err(e) => {
            tracing::error!("uygulama ikonu thread'i başlatılamadı: {e}");
            Worker::Failed
        }
    }
}

/// Arka plan thread'i: nesil başına tema ve `.desktop` dizini, iş başına çözüm.
fn run(rx: &Receiver<Job>, shared: &Shared, config: WorkerConfig) {
    crate::loader::lower_thread_priority();
    let WorkerConfig { sandbox, dirs, mut notify } = config;
    let dirs = dirs.unwrap_or_else(AppIconDirs::from_env);
    let mut current: Option<(u64, Arc<IconTheme>)> = None;
    let mut index: Option<DesktopIndex> = None;
    while let Ok(job) = rx.recv() {
        let (generation, theme_name) = {
            let st = shared.lock();
            (st.generation, st.theme.clone())
        };
        if job.generation != generation {
            continue; // Önbellek temizlendi; girdi zaten yok.
        }
        let theme = match &current {
            Some((g, t)) if *g == generation => Arc::clone(t),
            _ => {
                let name = theme_name.or_else(crate::icon::detect_theme_name).unwrap_or_else(|| "hicolor".to_owned());
                let theme = Arc::new(IconTheme::with_dirs(name, dirs.icons.clone(), dirs.pixmaps.clone()));
                if current.is_some() {
                    index = None; // Elle yenileme/tema değişimi: dizin de yeniden taranır.
                }
                current = Some((generation, Arc::clone(&theme)));
                theme
            }
        };
        let index = match &mut index {
            Some(idx) if !idx.is_stale(&dirs.applications) => idx,
            slot => {
                if slot.is_some() {
                    shared.lock().forget_missing();
                }
                slot.insert(DesktopIndex::scan(&dirs.applications))
            }
        };
        let names = candidate_names(index, &job.key.app_id);
        let image = resolve(&theme, &names, &job.key, sandbox.as_deref());
        {
            let mut st = shared.lock();
            if st.generation != job.generation {
                continue;
            }
            st.complete(&job.key, image);
        }
        (notify)();
    }
}

/// Tema araması için aday adlar, öncelik sırasıyla (tekrarsız): masaüstü girdisinin `Icon=`
/// değeri, Steam oyunu (`steam_app_<n>` → `steam_icon_<n>`), `app_id`, küçük harfi ve ters-DNS
/// sonu.
pub fn candidate_names(index: &DesktopIndex, app_id: &str) -> Vec<String> {
    let Some(id) = desktop::clean_app_id(app_id) else { return Vec::new() };
    let lower = id.to_lowercase();
    let mut out: Vec<String> = Vec::with_capacity(5);
    let mut push = |n: &str| {
        if !n.is_empty() && !out.iter().any(|o| o == n) {
            out.push(n.to_owned());
        }
    };
    if let Some(icon) = index.icon_for(id) {
        push(icon);
    }
    if let Some(n) = lower.strip_prefix("steam_app_").filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) {
        push(&format!("steam_icon_{n}"));
    }
    push(id);
    push(&lower);
    push(desktop::last_component(&lower));
    out
}

/// İlk tam adla bulunan ikonu çözer; çözülemezse (ör. işçisiz `Untrusted` SVG) sonraki ada geçer.
fn resolve(theme: &Arc<IconTheme>, names: &[String], key: &Key, sandbox: Option<&dyn IsolatedDecoder>) -> Option<Image> {
    for name in names {
        // Kısaltmalı yedek (`google-chrome` → `google`) uygulama ikonunda yanlış ikon verir:
        // yalnızca tam ad.
        if theme.lookup(name, key.size, key.scale).is_none() {
            continue;
        }
        let source = crate::derive_key(&[b"app-icon", key.app_id.as_bytes()]);
        let request = LoadRequest::icon(Arc::clone(theme), name.clone(), key.size, key.scale).with_trust(Trust::System);
        match request.run_with(sandbox, source) {
            Ok(decoded) => return Some(decoded.into_first_frame()),
            Err(e) => tracing::debug!(error = %e, "uygulama ikonu çözülemedi; sonraki aday"),
        }
    }
    None
}

fn valid_theme(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && !name.contains('/') && !name.starts_with('.') && !name.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_follow_xdg_rules() {
        let home = Path::new("/home/u");
        let d = AppIconDirs::from_vars(Some(home), None, Some("/opt/share::rel/x:/usr/share"));
        let apps: Vec<_> = d.applications.iter().map(|p| p.to_str().unwrap()).collect();
        assert_eq!(
            apps,
            [
                "/home/u/.local/share/applications",
                "/opt/share/applications",
                "/usr/share/applications",
                "/home/u/.local/share/flatpak/exports/share/applications",
                "/var/lib/flatpak/exports/share/applications",
            ]
        );
        assert_eq!(d.icons[0], home.join(".icons"));
        assert!(d.icons.contains(&PathBuf::from("/var/lib/flatpak/exports/share/icons")));
        // Göreli XDG_DATA_HOME yok sayılır; boş XDG_DATA_DIRS varsayılana döner; Flatpak dizini
        // listede varsa tekrar eklenmez.
        let d = AppIconDirs::from_vars(Some(home), Some("rel"), Some(""));
        assert_eq!(d.applications[0], home.join(".local/share/applications"));
        assert!(d.applications.contains(&PathBuf::from("/usr/local/share/applications")));
        let d = AppIconDirs::from_vars(None, Some("/d"), Some("/var/lib/flatpak/exports/share"));
        assert_eq!(d.applications.iter().filter(|p| p.starts_with(FLATPAK_SYSTEM_EXPORTS)).count(), 1);
    }

    #[test]
    fn candidates_are_ordered_and_unique() {
        let idx = DesktopIndex::default();
        assert_eq!(candidate_names(&idx, "org.gnome.Nautilus"), ["org.gnome.Nautilus", "org.gnome.nautilus", "nautilus"]);
        assert_eq!(candidate_names(&idx, "kitty"), ["kitty"]);
        assert_eq!(candidate_names(&idx, "steam_app_570"), ["steam_icon_570", "steam_app_570"]);
        assert!(candidate_names(&idx, "../etc").is_empty());
    }

    fn image(side: u32) -> Image {
        Image::from_premultiplied(side, side, vec![0; (side * side * 4) as usize]).unwrap()
    }

    #[test]
    fn lru_bounds_entries_and_bytes_and_keeps_pending() {
        let mut st = State::default();
        let key = |i: usize| Key { app_id: format!("app{i}"), size: 32, scale: 1 };
        for i in 0..MAX_CACHED_ICONS + 10 {
            let used = st.touch();
            st.map.insert(key(i), CacheEntry { slot: Slot::Pending, used });
            st.complete(&key(i), (i % 2 == 0).then(|| image(32)));
        }
        assert_eq!(st.map.len(), MAX_CACHED_ICONS);
        assert!(!st.map.contains_key(&key(0)), "en eski atıldı");
        // Bayt bütçesi: büyük görseller eskileri iter, bekleyenler asla atılmaz.
        let used = st.touch();
        st.map.insert(key(9999), CacheEntry { slot: Slot::Pending, used });
        for i in 0..40 {
            let k = Key { app_id: format!("big{i}"), size: 256, scale: 1 };
            let used = st.touch();
            st.map.insert(k.clone(), CacheEntry { slot: Slot::Pending, used });
            st.complete(&k, Some(image(256)));
        }
        assert!(st.bytes <= MAX_CACHED_BYTES, "{}", st.bytes);
        assert!(matches!(st.map.get(&key(9999)).map(|e| &e.slot), Some(Slot::Pending)));
        let ready: usize = st.map.values().filter_map(|e| if let Slot::Ready(i) = &e.slot { Some(i.byte_size()) } else { None }).sum();
        assert_eq!(ready, st.bytes, "bayt sayacı tutarlı");
    }
}
