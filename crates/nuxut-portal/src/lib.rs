//! xdg-desktop-portal istemcisi: dosya seçici (`org.freedesktop.portal.FileChooser`).
//!
//! Framework'ün geri kalanı D-Bus'a bağlanmaz; bu crate isteğe bağlıdır (cephede `portal`
//! özelliği). Sözleşme: `docs/ARCHITECTURE.md` → "Portal (nuxut-portal)".
//!
//! # Akış
//! [`Portal::open_file`] / [`Portal::save_file`] hemen bir [`PortalRequestId`] döner; istek kısa
//! ömürlü bir thread'de yürür (oturum bus'ına bağlantı ilk istekte, tembel). Sonuç her istek
//! için **tam bir kez** [`PortalResponse`] olarak `Portal::new`'e verilen alıcıya gelir — bu
//! çağrının içinden değil, istek thread'inden. Nuxut uygulamasında alıcı `cx.sender()` ile
//! olay döngüsüne (calloop kanalı) iletir:
//!
//! ```ignore
//! let sender = cx.sender();
//! let portal = Portal::new(move |response| {
//!     let _ = sender.send(Message::FileChosen(response));
//! });
//! ```
//!
//! # Ebeveyn pencere
//! Diyalog pencerenin üstüne kipli açılsın diye [`ParentWindow::wayland`] (nuxut-wayland
//! `cx.export_toplevel` → `ToplevelHandle::as_str`). Layer-shell yüzeyleri dışa aktarılamaz;
//! onlarda [`ParentWindow::none`].
//!
//! # Gizlilik
//! Seçilen yollar, başlangıç klasörü/adı kullanıcı içeriğidir: hiçbir seviyede loglanmaz,
//! `Debug` çıktıları yalnız sayı/bayrak gösterir (ARCHITECTURE değişmez 9).

//!
//! # Platform
//! xdg-desktop-portal Linux'a özgüdür. ADR-0003 kural 2: API her platformda derlenir; Linux
//! dışında her istek (yine istek thread'inden, tam bir kez) [`PortalError::Unavailable`] ile
//! biter. Uygulama aynı yedeği (yolu elle yazdırma) sunar.

#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod options;
#[cfg(target_os = "linux")]
mod request;
#[cfg(not(target_os = "linux"))]
#[path = "request_stub.rs"]
mod request;
mod uri;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

pub use uri::file_uri_to_path;

/// Bir portal isteğinin kimliği (süreç içinde benzersiz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortalRequestId(u64);

impl PortalRequestId {
    /// Ham sayı (log/ayırt etme için; kullanıcı içeriği değil).
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Diyaloğun ebeveyni: xdg-desktop-portal "window identifier" biçimi.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ParentWindow(String);

impl ParentWindow {
    /// Ebeveynsiz (layer-shell yüzeyi ya da pencere yok): `""`.
    pub fn none() -> Self {
        Self(String::new())
    }

    /// xdg-foreign-v2 tanıtıcısı (`ToplevelHandle::as_str`): `wayland:<tanıtıcı>`.
    pub fn wayland(handle: &str) -> Self {
        Self(format!("wayland:{handle}"))
    }

    /// Hazır tanımlayıcı (`wayland:…` ya da `x11:<xid hex>`), olduğu gibi.
    pub fn from_identifier(identifier: impl Into<String>) -> Self {
        Self(identifier.into())
    }

    /// Portal'a giden dize.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ParentWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Tanıtıcı başka istemciye pencereye yapışma yetkisi verir: yalnız tür.
        let kind = self.0.split_once(':').map_or("none", |(kind, _)| kind);
        write!(f, "ParentWindow({kind})")
    }
}

/// Filtre kuralı: glob (`*.conf`, büyük/küçük harf duyarlı) ya da MIME türü (`image/png`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterRule {
    Glob(String),
    Mime(String),
}

/// Adlandırılmış dosya filtresi ("Resimler": `image/*`). Ad kullanıcıya görünür: çevrilmiş
/// (`t!`) verilmeli.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub rules: Vec<FilterRule>,
}

impl Filter {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rules: Vec::new(),
        }
    }

    /// Glob kuralı ekler (`*.ovpn`).
    pub fn glob(mut self, pattern: impl Into<String>) -> Self {
        self.rules.push(FilterRule::Glob(pattern.into()));
        self
    }

    /// MIME kuralı ekler (`image/png`, `image/*`).
    pub fn mime(mut self, mime: impl Into<String>) -> Self {
        self.rules.push(FilterRule::Mime(mime.into()));
        self
    }
}

/// `OpenFile` seçenekleri. `title` ve `accept_label` kullanıcıya görünür: çevrilmiş verilmeli.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct OpenFileOptions {
    pub title: String,
    /// Kabul düğmesi metni (`None`: arka ucun varsayılanı, "Aç").
    pub accept_label: Option<String>,
    /// Birden çok seçim.
    pub multiple: bool,
    /// Dosya yerine dizin seçimi (FileChooser sürüm 3+; eski portal yok sayar).
    pub directory: bool,
    pub filters: Vec<Filter>,
    /// Başta seçili filtre (`filters` içindeki sıra); geçersiz sıra yok sayılır.
    pub current_filter: Option<usize>,
    /// Başlangıç klasörü (sürüm 4+; eski portal yok sayar).
    pub current_folder: Option<PathBuf>,
    /// Ebeveyne kipli (varsayılan `true`: portal varsayılanı da kiplidir).
    pub modal: bool,
}

impl OpenFileOptions {
    /// Başlıkla, kipli, tek dosya.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            modal: true,
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for OpenFileOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenFileOptions")
            .field("multiple", &self.multiple)
            .field("directory", &self.directory)
            .field("filters", &self.filters.len())
            .field("current_folder", &self.current_folder.is_some())
            .field("modal", &self.modal)
            .finish_non_exhaustive()
    }
}

/// `SaveFile` seçenekleri. `title`/`accept_label` çevrilmiş; `current_name` önerilen ad.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SaveFileOptions {
    pub title: String,
    pub accept_label: Option<String>,
    /// Önerilen dosya adı (`yedek.toml`).
    pub current_name: Option<String>,
    pub current_folder: Option<PathBuf>,
    pub filters: Vec<Filter>,
    pub current_filter: Option<usize>,
    pub modal: bool,
}

impl SaveFileOptions {
    /// Başlıkla, kipli.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            modal: true,
            ..Self::default()
        }
    }
}

impl std::fmt::Debug for SaveFileOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveFileOptions")
            .field("current_name", &self.current_name.is_some())
            .field("current_folder", &self.current_folder.is_some())
            .field("filters", &self.filters.len())
            .field("modal", &self.modal)
            .finish_non_exhaustive()
    }
}

/// İstek neden yol vermedi.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PortalError {
    /// Kullanıcı vazgeçti (yanıt 1) ya da [`Portal::cancel`] (Request.Close).
    #[error("portal isteği iptal edildi")]
    Cancelled,
    /// Oturum bus'ı yok, xdg-desktop-portal ya da FileChooser arka ucu yok/yanıt vermiyor.
    #[error("dosya seçici portalı yok")]
    Unavailable,
    /// Portal reddetti ya da etkileşim başka biçimde bitti (yanıt 2, AccessDenied/NotAllowed).
    #[error("portal isteği reddedildi")]
    Denied,
    /// Başarılı yanıtta kullanılabilir `file://` URI yok.
    #[error("portal yanıtı geçersiz")]
    InvalidResponse,
    /// Başka bir D-Bus hatası; yalnız hata adı (ileti metni kullanıcı içeriği taşıyabilir).
    #[error("D-Bus hatası: {0}")]
    Dbus(String),
}

/// Bir isteğin sonucu. `Debug` yolları göstermez (yalnız sayı).
#[derive(Clone, PartialEq, Eq)]
pub struct PortalResponse {
    pub id: PortalRequestId,
    /// Seçilen yollar (açmada ≥ 1, kaydetmede 1). Document portal yolları
    /// (`/run/user/<uid>/doc/…`) geçerlidir.
    pub result: Result<Vec<PathBuf>, PortalError>,
}

impl std::fmt::Debug for PortalResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let result = match &self.result {
            Ok(paths) => format!("Ok({} yol)", paths.len()),
            Err(e) => format!("Err({e:?})"),
        };
        f.debug_struct("PortalResponse")
            .field("id", &self.id)
            .field("result", &format_args!("{result}"))
            .finish()
    }
}

/// Hangi bus'a bağlanılır.
// Linux dışında bağlanılacak bus yok (istek `Unavailable`); tür API eşitliği için durur.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone)]
enum Bus {
    Session,
    /// Testler: özel `dbus-daemon` adresi (kullanıcının oturum bus'ına dokunulmaz).
    Address(String),
}

type Sink = Box<dyn FnMut(PortalResponse) + Send>;

struct Inner {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    bus: Bus,
    /// Tembel bağlantı; başarısız bağlanma saklanmaz (sonraki istek yeniden dener).
    #[cfg(target_os = "linux")]
    connection: Mutex<Option<zbus::Connection>>,
    sink: Mutex<Sink>,
    /// Süren istekler: iptal kanalı.
    pending: Mutex<HashMap<PortalRequestId, async_channel::Sender<()>>>,
}

impl Inner {
    fn deliver(&self, response: PortalResponse) {
        let known = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&response.id)
            .is_some();
        if !known {
            return;
        }
        tracing::debug!(id = response.id.0, ?response, "portal sonucu");
        let mut sink = self.sink.lock().unwrap_or_else(PoisonError::into_inner);
        (sink)(response);
    }
}

/// Süreç genelinde istek sayacı (kimlik ve `handle_token`).
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Portal istemcisi. Ucuz kopyalanmaz; uygulama başına bir tane tutulur. Düşerken süren
/// istekler iptal edilir (diyaloglar kapanır, sonuçları yine alıcıya `Cancelled` olarak gelir).
pub struct Portal {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Portal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        f.debug_struct("Portal")
            .field("pending", &pending)
            .finish_non_exhaustive()
    }
}

impl Portal {
    /// Oturum bus'ındaki `org.freedesktop.portal.Desktop` için. Bağlantı ilk istekte kurulur.
    /// `sink` istek thread'lerinden çağrılır (sırayla, kilit altında); kısa tutulmalı.
    pub fn new(sink: impl FnMut(PortalResponse) + Send + 'static) -> Self {
        Self::with_bus(Bus::Session, Box::new(sink))
    }

    /// Belirli bir bus adresi (`unix:path=…`) için; testler özel `dbus-daemon` ile kullanır.
    pub fn with_address(
        address: impl Into<String>,
        sink: impl FnMut(PortalResponse) + Send + 'static,
    ) -> Self {
        Self::with_bus(Bus::Address(address.into()), Box::new(sink))
    }

    fn with_bus(bus: Bus, sink: Sink) -> Self {
        Self {
            inner: Arc::new(Inner {
                bus,
                #[cfg(target_os = "linux")]
                connection: Mutex::new(None),
                sink: Mutex::new(sink),
                pending: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Dosya (ya da `directory` ile dizin) açma diyaloğu.
    pub fn open_file(&self, parent: &ParentWindow, options: OpenFileOptions) -> PortalRequestId {
        self.start(parent, request::Kind::Open(options))
    }

    /// Kaydetme diyaloğu; sonuç tek yol (dosya henüz var olmayabilir).
    pub fn save_file(&self, parent: &ParentWindow, options: SaveFileOptions) -> PortalRequestId {
        self.start(parent, request::Kind::Save(options))
    }

    /// Süren isteği iptal eder (Request.Close; diyalog kapanır). Sonuç `Cancelled` olarak
    /// gelir. Bitmiş/bilinmeyen kimlik yok sayılır.
    pub fn cancel(&self, id: PortalRequestId) {
        let tx = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
            .cloned();
        if let Some(tx) = tx {
            let _ = tx.try_send(());
        }
    }

    /// Süren istek sayısı.
    pub fn pending(&self) -> usize {
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    fn start(&self, parent: &ParentWindow, kind: request::Kind) -> PortalRequestId {
        let id = PortalRequestId(NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let (cancel_tx, cancel_rx) = async_channel::bounded(1);
        self.inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, cancel_tx);
        let inner = Arc::clone(&self.inner);
        let parent = parent.as_str().to_owned();
        let spawned = std::thread::Builder::new()
            .name("nuxut-portal".into())
            .spawn(move || {
                let result = request::run(&inner, id, &parent, kind, cancel_rx);
                inner.deliver(PortalResponse { id, result });
            });
        if let Err(e) = spawned {
            tracing::warn!("portal thread'i başlatılamadı: {e}");
            // Sözleşme: sonuç çağrının içinden gelmez; thread yoksa yine de tam bir kez gelmeli.
            // Alıcı çağıranın kilidini tutmuyor (sink ayrı kilit), doğrudan teslim güvenli.
            self.inner.deliver(PortalResponse {
                id,
                result: Err(PortalError::Unavailable),
            });
        }
        id
    }
}

impl Drop for Portal {
    fn drop(&mut self) {
        let pending = self
            .inner
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        for tx in pending.values() {
            let _ = tx.try_send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_user_content() {
        let parent = ParentWindow::wayland("gizli-tanitici");
        assert_eq!(parent.as_str(), "wayland:gizli-tanitici");
        assert_eq!(format!("{parent:?}"), "ParentWindow(wayland)");
        assert_eq!(format!("{:?}", ParentWindow::none()), "ParentWindow(none)");

        let mut open = OpenFileOptions::new("Başlık");
        open.current_folder = Some("/home/gizli/klasor".into());
        let shown = format!("{open:?}");
        assert!(!shown.contains("gizli"), "{shown}");

        let mut save = SaveFileOptions::new("Kaydet");
        save.current_name = Some("gizli-ad.toml".into());
        save.current_folder = Some("/home/gizli".into());
        let shown = format!("{save:?}");
        assert!(!shown.contains("gizli"), "{shown}");

        let response = PortalResponse {
            id: PortalRequestId(7),
            result: Ok(vec!["/home/gizli/dosya.png".into()]),
        };
        let shown = format!("{response:?}");
        assert!(!shown.contains("gizli"), "{shown}");
        assert!(shown.contains("1 yol"), "{shown}");
    }

    #[test]
    fn filter_builder() {
        let f = Filter::new("Resimler").mime("image/*").glob("*.webp");
        assert_eq!(
            f.rules,
            vec![
                FilterRule::Mime("image/*".into()),
                FilterRule::Glob("*.webp".into())
            ]
        );
    }

    #[test]
    fn options_default_modal() {
        assert!(OpenFileOptions::new("x").modal);
        assert!(SaveFileOptions::new("x").modal);
    }
}
