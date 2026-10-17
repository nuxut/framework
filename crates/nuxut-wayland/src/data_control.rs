//! Pano izleme ve odaksız seçim (data-control): `ext-data-control-v1`, yoksa
//! `zwlr-data-control-unstable-v1` (sürüm 1–2). Pano geçmişi gibi tüketiciler içindir. Pano
//! (`Selection::Clipboard`) her zaman izlenir; birincil seçim (`Selection::Primary`) yalnızca
//! `data_control_watch_primary(true)` ile olay üretir (wlr sürüm 1'de yok).
//!
//! Tasarım (`docs/ARCHITECTURE.md` → "Pano izleme — data-control"):
//! - **Tembel.** Global açılışta yalnızca kaydedilir; `Context::data_control_start` çağrılana
//!   dek bağlanmaz, aygıt yaratılmaz, compositor bize hiçbir seçim bildirmez.
//! - **Olaylar ana thread'de** (`App::data_control`): `SelectionOffered { selection, id, mimes,
//!   own, sensitive, truncated }` / `SelectionCleared { selection }` / `Stopped`. MIME listesi
//!   ≤ [`MAX_MIMES`] öğe, her ad ≤ [`MAX_MIME_LEN`] bayt; fazlası düşer ve `truncated`
//!   işaretlenir.
//! - **`sensitive`**: teklif parola/gizli işaretli bir tür sunuyor ([`is_sensitive_mime`]: ör.
//!   `x-kde-passwordManagerHint`); framework böyle bir teklifin hiçbir türünü okumaz
//!   (`DataReadError::Sensitive`).
//! - **`own`**: teklif bu sürecin koyduğu seçim. Bizim tüm pano kaynaklarımız
//!   (`set_clipboard` ve `data_control_set_selection`) süreç başına rastgele bir işaret MIME'ı
//!   ([`owner_mime`]) da sunar; teklifte görülürse `own = true` ve işaret listeden atılır.
//! - **Okuma ana döngüyü bloklamaz.** [`DataControlHandle::read`] çağıranın thread'inde
//!   (ana thread değil) boru açar, `receive` isteğini kanalla ana döngüye verir ve boruyu
//!   `poll` ile sınırlı ve zaman aşımlı okur. `Context::data_control_read` aynı okumayı kısa
//!   ömürlü bir thread'de yapıp sonucu `App::data_control_read` ile verir. Kalıcı thread yok;
//!   boşta yalnızca uyuyan bir calloop kanalı.
//! - **Framework kendiliğinden hiçbir teklifi okumaz**; okuma yalnızca uygulama isteyince ve
//!   yalnızca güncel teklifin sunduğu bir MIME için yapılır (eski kimlik → `Gone`).
//! - **Gizlilik.** Okuma tamponu `Zeroizing`; büyürken eski tampon sıfırlanır (yeniden
//!   ayırmada kopya kalmaz), sınır/zaman aşımı/hata yolunda yarım tampon sıfırlanır. Seçime
//!   konan baytlar paylaşılan `Arc<[u8]>`'tır: seçim yerine geçince/iptal edilince ya da yazma
//!   bitince son sahibi framework ise sıfırlanır ([`SharedBytes`]). Oturum kilidi
//!   istenmiş/kuruluyken okuma ve seçim koyma reddedilir, kendi seçimimiz yazılmaz. İçerik
//!   hiçbir seviyede loglanmaz (değişmez 9).

use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{ErrorKind, Read};
use std::os::fd::{AsFd, OwnedFd};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use calloop::RegistrationToken;
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryHandler};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};
use zeroize::{Zeroize, Zeroizing};

use crate::App;
use crate::Selection;
use crate::clipboard::Payload;
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};

/// `ext_data_control_manager_v1` arayüz adı.
pub(crate) const EXT_INTERFACE: &str = "ext_data_control_manager_v1";
/// `zwlr_data_control_manager_v1` arayüz adı (yedek).
pub(crate) const WLR_INTERFACE: &str = "zwlr_data_control_manager_v1";

/// Teklif başına en çok MIME türü; fazlası düşer (`truncated`).
pub const MAX_MIMES: usize = 64;
/// MIME adının en çok bayt uzunluğu; daha uzun ad düşer (`truncated`).
pub const MAX_MIME_LEN: usize = 256;
/// Okuma parçası.
const CHUNK: usize = 64 * 1024;
/// `DataControlHandle::set_selection` yanıt bekleme süresi.
const SET_REPLY_TIMEOUT: Duration = Duration::from_secs(1);
/// İşaret MIME'ının öneki (ardından süreç başına rastgele 16 onaltılık hane).
pub const OWNER_MIME_PREFIX: &str = "application/x-nuxut-owner-";

/// KDE / KeePassXC parola yöneticisi ipucu türü (değeri genelde `secret`). Varlığı yeter.
pub const PASSWORD_HINT_MIME: &str = "x-kde-passwordManagerHint";

/// Tür, teklifi hassas kılar mı: [`PASSWORD_HINT_MIME`] ya da adında `secret` / `password`
/// geçen tür (ASCII büyük/küçük harf duyarsız). Karar yalnızca ada bakar; içerik okunmaz.
pub fn is_sensitive_mime(mime: &str) -> bool {
    let contains = |needle: &str| {
        mime.as_bytes()
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
    };
    mime.eq_ignore_ascii_case(PASSWORD_HINT_MIME) || contains("secret") || contains("password")
}

/// Bu sürecin pano kaynaklarına eklenen işaret MIME'ı (süreç başına rastgele). Teklifte
/// görülürse seçim bizimdir (`own`). İçerik taşımaz; istenirse boru boş kapatılır.
pub fn owner_mime() -> &'static str {
    static OWNER: OnceLock<String> = OnceLock::new();
    OWNER.get_or_init(|| {
        let mut h = RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        format!("{OWNER_MIME_PREFIX}{:016x}", h.finish())
    })
}

/// Seçim teklifinin kimliği (yalnızca en son teklif okunabilir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataOfferId(pub(crate) u64);

/// `Context::data_control_read` isteğinin kimliği; sonuç `App::data_control_read` ile gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataReadId(pub(crate) u64);

/// Compositor'ın sunduğu data-control protokolü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataControlProtocol {
    /// `ext_data_control_manager_v1` (tercih edilen).
    Ext,
    /// `zwlr_data_control_manager_v1` (yedek; sürüm 1).
    Wlr,
}

/// `App::data_control` olayı. `Selection::Primary` olayları yalnızca
/// `data_control_watch_primary(true)` iken gelir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataControlEvent {
    /// Yeni seçim. `mimes`: sunulan türler (≤ [`MAX_MIMES`], her biri ≤ [`MAX_MIME_LEN`]
    /// bayt, tekrarsız, işaret MIME'ı atılmış). `own`: seçimi bu süreç koydu (geçmişe
    /// yeniden alınmamalı). `sensitive`: parola/gizli işaretli tür sunuldu
    /// ([`is_sensitive_mime`]; sınırı aşıp düşen türler dahil) — okunamaz. `truncated`: sınırı
    /// aşan türler düştü (liste eksik; temkinli tüketici teklifi atlar).
    SelectionOffered {
        selection: Selection,
        id: DataOfferId,
        mimes: Vec<String>,
        own: bool,
        sensitive: bool,
        truncated: bool,
    },
    /// Seçim boşaldı (önceki teklif artık okunamaz).
    SelectionCleared { selection: Selection },
    /// İzleme bitti: compositor aygıtı bitirdi (`finished`), koltuk ya da global gitti.
    /// Güncel teklif ve bizim seçimimiz bırakıldı; yeniden `data_control_start` denenebilir.
    Stopped,
}

/// `data_control_start` hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DataControlError {
    #[error("compositor data-control sunmuyor")]
    Unsupported,
    #[error("koltuk yok")]
    NoSeat,
}

/// Okuma hatası (içerik taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DataReadError {
    /// Teklif artık güncel değil (yeni seçim geldi, seçim boşaldı ya da izleme durdu).
    #[error("teklif artık geçerli değil")]
    Gone,
    /// Teklif bu türü sunmuyor.
    #[error("tür sunulmuyor")]
    NotOffered,
    /// Teklif parola/gizli işaretli (`sensitive`); hiçbir türü okunmaz.
    #[error("hassas teklif")]
    Sensitive,
    /// Oturum kilidi istenmiş/kurulu.
    #[error("oturum kilitli")]
    Locked,
    /// İçerik `max_bytes`'ı aşıyor; okuma durdu.
    #[error("sınırı aşıyor")]
    TooLarge,
    /// Süre doldu.
    #[error("zaman aşımı")]
    Timeout,
    /// Boru hatası.
    #[error("okunamadı")]
    Io,
    /// Bloklayan okuma ana (olay döngüsü) thread'inden çağrıldı.
    #[error("ana thread'den bloklayan okuma")]
    MainThread,
    /// Runtime kapandı.
    #[error("runtime kapandı")]
    Closed,
}

/// Seçim koyma hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SetSelectionError {
    /// İzleme başlatılmamış ya da durmuş (aygıt yok).
    #[error("data-control başlatılmadı")]
    NotStarted,
    /// Birincil seçim bu protokolde yok (wlr sürüm 1).
    #[error("birincil seçim desteklenmiyor")]
    Unsupported,
    /// Boş liste, çok fazla tür, geçersiz/tekrarlı ya da ayrılmış MIME adı.
    #[error("geçersiz MIME listesi")]
    Invalid,
    #[error("oturum kilitli")]
    Locked,
    /// Bloklayan çağrı ana thread'den yapıldı (`Context::data_control_set_selection` kullan).
    #[error("ana thread'den bloklayan çağrı")]
    MainThread,
    #[error("runtime kapandı")]
    Closed,
}

/// Kuyruktaki okuma sonucu; `Debug` yalnızca uzunluk yazar (değişmez 9).
pub(crate) struct ReadOutcome(pub(crate) Result<Zeroizing<Vec<u8>>, DataReadError>);

impl std::fmt::Debug for ReadOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Ok(b) => write!(f, "Ok({} bayt)", b.len()),
            Err(e) => write!(f, "Err({e:?})"),
        }
    }
}

/// Seçime konan, paylaşılan bayt tamponu. Düşerken son sahip buysa içerik sıfırlanır.
#[derive(Clone)]
pub(crate) struct SharedBytes(Arc<[u8]>);

impl SharedBytes {
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for SharedBytes {
    fn drop(&mut self) {
        zeroize_if_unique(&mut self.0);
    }
}

/// `Arc`'ın tek sahibiysek içeriği sıfırlar (başka sahip — ör. uygulamanın geçmişi — varsa
/// dokunulmaz; o kendi kopyasından sorumludur). Sıfırlandıysa `true`.
fn zeroize_if_unique(bytes: &mut Arc<[u8]>) -> bool {
    match Arc::get_mut(bytes) {
        Some(b) => {
            b.zeroize();
            true
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------------------------
// Saf parçalar: MIME listesi, seçim doğrulama, sınırlı okuma.
// ---------------------------------------------------------------------------------------------

/// Bir teklifin `offer` olaylarıyla biriken MIME listesi (sınırlı).
#[derive(Debug, Default, Clone)]
pub(crate) struct MimeList {
    mimes: Vec<String>,
    own: bool,
    sensitive: bool,
    truncated: bool,
}

impl MimeList {
    fn push(&mut self, mime: String, marker: &str) {
        if mime == marker {
            self.own = true;
            return;
        }
        // Sınır kontrolünden önce: düşen tür de teklifi hassas kılar.
        if is_sensitive_mime(&mime) {
            self.sensitive = true;
        }
        if mime.is_empty() || mime.len() > MAX_MIME_LEN {
            self.truncated = true;
        } else if self.mimes.contains(&mime) {
            // Tekrar: yok sayılır.
        } else if self.mimes.len() >= MAX_MIMES {
            self.truncated = true;
        } else {
            self.mimes.push(mime);
        }
    }
}

/// `set_selection` listesini doğrular: 1..=MAX_MIMES öğe, her ad 1..=MAX_MIME_LEN bayt,
/// kontrol karakteri yok, tekrar yok, işaret öneki ayrılmış.
fn validate_selection(data: &[(String, Arc<[u8]>)]) -> Result<(), SetSelectionError> {
    if data.is_empty() || data.len() > MAX_MIMES {
        return Err(SetSelectionError::Invalid);
    }
    for (i, (mime, _)) in data.iter().enumerate() {
        let bad = mime.is_empty()
            || mime.len() > MAX_MIME_LEN
            || mime.chars().any(char::is_control)
            || mime.starts_with(OWNER_MIME_PREFIX)
            || data[..i].iter().any(|(m, _)| m == mime);
        if bad {
            return Err(SetSelectionError::Invalid);
        }
    }
    Ok(())
}

/// `buf`'a ekler; kapasite yetmezse yeni `Zeroizing` tampona taşır (eskisi düşerken
/// sıfırlanır — `Vec` yeniden ayırmasında içerik kopyası serbest bellekte kalmaz).
/// Kapasite `limit`'i aşmaz (`need` daha büyükse `need`).
fn append_zeroizing(buf: &mut Zeroizing<Vec<u8>>, data: &[u8], limit: usize) {
    let need = buf.len() + data.len();
    if need > buf.capacity() {
        let cap = need
            .max(buf.capacity().saturating_mul(2))
            .max(4096)
            .min(limit.max(need));
        let mut next = Zeroizing::new(Vec::with_capacity(cap));
        next.extend_from_slice(buf);
        *buf = next;
    }
    buf.extend_from_slice(data);
}

/// Boruyu `deadline`'a dek, en çok `max_bytes` okur (EOF'a kadar). Aşımda `TooLarge` (okuma
/// durur, boru kapanır), süre dolunca `Timeout`. Her hata yolunda yarım tampon sıfırlanır.
pub(crate) fn read_limited(
    fd: OwnedFd,
    max_bytes: usize,
    deadline: Instant,
) -> Result<Zeroizing<Vec<u8>>, DataReadError> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    rustix::io::ioctl_fionbio(&fd, true).map_err(|_| DataReadError::Io)?;
    let mut file = File::from(fd);
    let mut buf: Zeroizing<Vec<u8>> = Zeroizing::new(Vec::new());
    let mut chunk: Zeroizing<Vec<u8>> = Zeroizing::new(vec![0; CHUNK]);
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return Ok(buf),
            Ok(n) if buf.len() + n > max_bytes => return Err(DataReadError::TooLarge),
            Ok(n) => {
                append_zeroizing(&mut buf, &chunk[..n], max_bytes);
                continue;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(DataReadError::Io),
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(DataReadError::Timeout);
        }
        let ts = Timespec {
            tv_sec: i64::try_from(left.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: left.subsec_nanos().into(),
        };
        let mut fds = [PollFd::new(&file, PollFlags::IN)];
        match poll(&mut fds, Some(&ts)) {
            // Hazır (IN ya da HUP): sonraki `read` karar verir.
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => {}
            Err(_) => return Err(DataReadError::Io),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Protokol sarmalayıcıları (ext / wlr).
// ---------------------------------------------------------------------------------------------

enum Manager {
    Ext(ExtDataControlManagerV1),
    Wlr(ZwlrDataControlManagerV1),
}

enum Device {
    Ext(ExtDataControlDeviceV1),
    Wlr(ZwlrDataControlDeviceV1),
}

pub(crate) enum Offer {
    Ext(ExtDataControlOfferV1),
    Wlr(ZwlrDataControlOfferV1),
}

enum Source {
    Ext(ExtDataControlSourceV1),
    Wlr(ZwlrDataControlSourceV1),
}

impl Manager {
    fn protocol(&self) -> DataControlProtocol {
        match self {
            Self::Ext(_) => DataControlProtocol::Ext,
            Self::Wlr(_) => DataControlProtocol::Wlr,
        }
    }

    fn get_device<A: App>(&self, seat: &WlSeat, qh: &QueueHandle<State<A>>) -> Device {
        match self {
            Self::Ext(m) => Device::Ext(m.get_data_device(seat, qh, DeviceData)),
            Self::Wlr(m) => Device::Wlr(m.get_data_device(seat, qh, DeviceData)),
        }
    }

    fn create_source<A: App>(&self, qh: &QueueHandle<State<A>>) -> Source {
        match self {
            Self::Ext(m) => Source::Ext(m.create_data_source(qh, SourceData)),
            Self::Wlr(m) => Source::Wlr(m.create_data_source(qh, SourceData)),
        }
    }

    fn destroy(&self) {
        match self {
            Self::Ext(m) => m.destroy(),
            Self::Wlr(m) => m.destroy(),
        }
    }
}

impl Device {
    fn id(&self) -> ObjectId {
        match self {
            Self::Ext(d) => d.id(),
            Self::Wlr(d) => d.id(),
        }
    }

    /// Birincil seçim için çağıran sürümü doğrulamış olmalı (wlr ≥ 2).
    fn set_selection(&self, selection: Selection, source: &Source) {
        match (self, source, selection) {
            (Self::Ext(d), Source::Ext(s), Selection::Clipboard) => d.set_selection(Some(s)),
            (Self::Ext(d), Source::Ext(s), Selection::Primary) => {
                d.set_primary_selection(Some(s));
            }
            (Self::Wlr(d), Source::Wlr(s), Selection::Clipboard) => d.set_selection(Some(s)),
            (Self::Wlr(d), Source::Wlr(s), Selection::Primary) => {
                d.set_primary_selection(Some(s));
            }
            // Aynı yöneticiden yaratılırlar; karışık çift oluşmaz.
            _ => {}
        }
    }

    fn destroy(&self) {
        match self {
            Self::Ext(d) => d.destroy(),
            Self::Wlr(d) => d.destroy(),
        }
    }
}

impl Offer {
    fn data(&self) -> Option<&OfferData> {
        match self {
            Self::Ext(o) => o.data::<OfferData>(),
            Self::Wlr(o) => o.data::<OfferData>(),
        }
    }

    fn receive(&self, mime: String, fd: std::os::fd::BorrowedFd<'_>) {
        match self {
            Self::Ext(o) => o.receive(mime, fd),
            Self::Wlr(o) => o.receive(mime, fd),
        }
    }

    fn destroy(&self) {
        match self {
            Self::Ext(o) => o.destroy(),
            Self::Wlr(o) => o.destroy(),
        }
    }
}

impl Source {
    fn id(&self) -> ObjectId {
        match self {
            Self::Ext(s) => s.id(),
            Self::Wlr(s) => s.id(),
        }
    }

    fn offer(&self, mime: String) {
        match self {
            Self::Ext(s) => s.offer(mime),
            Self::Wlr(s) => s.offer(mime),
        }
    }

    fn destroy(&self) {
        match self {
            Self::Ext(s) => s.destroy(),
            Self::Wlr(s) => s.destroy(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Runtime durumu.
// ---------------------------------------------------------------------------------------------

/// Güncel teklif.
struct Current<O> {
    id: DataOfferId,
    offer: O,
    list: MimeList,
}

impl<O> Current<O> {
    fn event(&self, selection: Selection) -> DataControlEvent {
        DataControlEvent::SelectionOffered {
            selection,
            id: self.id,
            mimes: self.list.mimes.clone(),
            own: self.list.own,
            sensitive: self.list.sensitive,
            truncated: self.list.truncated,
        }
    }
}

/// Saf durum makinesi: seçim başına güncel teklif, birincil seçim izleme anahtarı, olay teslim
/// kapısı ve okuma kabulü. Protokol nesnesinden (`O`) bağımsız; birim testli.
pub(crate) struct OfferBook<O> {
    clipboard: Option<Current<O>>,
    primary: Option<Current<O>>,
    watch_primary: bool,
    /// Aygıt yaratıldıktan sonra compositor birincil seçimi bildirdi mi (bilinmeden açılan
    /// izleme sahte `SelectionCleared` vermesin; ilk olay zaten gelecek).
    primary_known: bool,
}

impl<O> Default for OfferBook<O> {
    fn default() -> Self {
        Self {
            clipboard: None,
            primary: None,
            watch_primary: false,
            primary_known: false,
        }
    }
}

impl<O> OfferBook<O> {
    fn slot(&mut self, selection: Selection) -> &mut Option<Current<O>> {
        match selection {
            Selection::Clipboard => &mut self.clipboard,
            Selection::Primary => &mut self.primary,
        }
    }

    fn reports(&self, selection: Selection) -> bool {
        selection == Selection::Clipboard || self.watch_primary
    }

    /// Seçim değişti. Dönüş: yok edilecek eski teklif ve (izleniyorsa) teslim edilecek olay.
    fn replace(
        &mut self,
        selection: Selection,
        next: Option<(DataOfferId, O, MimeList)>,
    ) -> (Option<O>, Option<DataControlEvent>) {
        let reports = self.reports(selection);
        if selection == Selection::Primary {
            self.primary_known = true;
        }
        let slot = self.slot(selection);
        let old = slot.take().map(|c| c.offer);
        let event = match next {
            Some((id, offer, list)) => {
                let cur = Current { id, offer, list };
                let event = cur.event(selection);
                *slot = Some(cur);
                event
            }
            None => DataControlEvent::SelectionCleared { selection },
        };
        (old, reports.then_some(event))
    }

    /// Birincil seçim olaylarını aç/kapat. Açılınca güncel durum hemen olay olarak döner
    /// (compositor yalnızca değişimde bildirir).
    fn set_watch_primary(&mut self, on: bool) -> Option<DataControlEvent> {
        let was = std::mem::replace(&mut self.watch_primary, on);
        if !on || was || !self.primary_known {
            return None;
        }
        Some(match &self.primary {
            Some(cur) => cur.event(Selection::Primary),
            None => DataControlEvent::SelectionCleared {
                selection: Selection::Primary,
            },
        })
    }

    fn find(&self, id: DataOfferId) -> Option<&Current<O>> {
        [&self.clipboard, &self.primary]
            .into_iter()
            .flatten()
            .find(|c| c.id == id)
    }

    /// Kuyruktaki olay hâlâ teslim edilmeli mi: teklif güncel ve seçimi izleniyor.
    fn deliverable(&self, event: &DataControlEvent) -> bool {
        match event {
            DataControlEvent::SelectionOffered { selection, id, .. } => {
                self.reports(*selection) && self.find(*id).is_some()
            }
            DataControlEvent::SelectionCleared { selection } => self.reports(*selection),
            DataControlEvent::Stopped => true,
        }
    }

    /// Okuma kabulü: kilit, güncellik, hassasiyet, tür.
    fn admit_read(&self, id: DataOfferId, mime: &str, locked: bool) -> Result<&O, DataReadError> {
        if locked {
            return Err(DataReadError::Locked);
        }
        let cur = self.find(id).ok_or(DataReadError::Gone)?;
        if cur.list.sensitive {
            return Err(DataReadError::Sensitive);
        }
        if !cur.list.mimes.iter().any(|m| m == mime) {
            return Err(DataReadError::NotOffered);
        }
        Ok(&cur.offer)
    }

    /// Tüm teklifler (yok edilmek üzere); izleme anahtarı korunur, birincil durum bilinmez olur.
    fn clear(&mut self) -> impl Iterator<Item = O> + use<O> {
        self.primary_known = false;
        [self.clipboard.take(), self.primary.take()]
            .into_iter()
            .flatten()
            .map(|c| c.offer)
    }
}

/// Bizim koyduğumuz seçim: kaynak (Drop'ta değil, açıkça yok edilir) ve türlere göre baytlar.
struct Owned {
    source: Source,
    data: Vec<(String, SharedBytes)>,
}

/// Ana döngüye kanal komutları (tutamaçtan ve okuma thread'lerinden).
pub(crate) enum Command {
    Receive {
        id: DataOfferId,
        mime: String,
        fd: OwnedFd,
        reply: mpsc::SyncSender<Result<(), DataReadError>>,
    },
    SetSelection {
        selection: Selection,
        data: Vec<(String, Arc<[u8]>)>,
        reply: mpsc::SyncSender<Result<(), SetSelectionError>>,
    },
    ReadDone(DataReadId, Result<Zeroizing<Vec<u8>>, DataReadError>),
}

/// Context'teki data-control durumu.
#[derive(Default)]
pub(crate) struct DataControl {
    /// Tembel bağlama için kayıt defteri (ilk global kaydında).
    registry: Option<WlRegistry>,
    /// Kayıtlı globaller (ad, sürüm).
    ext_global: Option<(u32, u32)>,
    wlr_global: Option<(u32, u32)>,
    /// Bağlı yönetici, global adı ve bağlanan sürüm.
    manager: Option<(Manager, u32, u32)>,
    /// Varsayılan koltuk (ilk `wl_seat`).
    seat: Option<WlSeat>,
    device: Option<Device>,
    offers: OfferBook<Offer>,
    owned_clipboard: Option<Owned>,
    owned_primary: Option<Owned>,
    /// Kanal (ilk `start`'ta kurulur) ve tutamaç.
    channel: Option<(DataControlHandle, RegistrationToken)>,
}

impl DataControl {
    fn owned_slot(&mut self, selection: Selection) -> &mut Option<Owned> {
        match selection {
            Selection::Clipboard => &mut self.owned_clipboard,
            Selection::Primary => &mut self.owned_primary,
        }
    }

    fn primary_supported(&self) -> bool {
        match &self.manager {
            Some((Manager::Ext(_), _, _)) => true,
            Some((Manager::Wlr(_), _, version)) => *version >= 2,
            None => false,
        }
    }
}

/// Bloklayan okuma ve seçim koyma için iş parçacıkları arası tutamaç (`Clone + Send + Sync`).
/// Ana (olay döngüsü) thread'inden bloklayan çağrılar `MainThread` döner.
#[derive(Clone)]
pub struct DataControlHandle {
    tx: calloop::channel::Sender<Command>,
    main: ThreadId,
}

impl std::fmt::Debug for DataControlHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataControlHandle").finish_non_exhaustive()
    }
}

impl DataControlHandle {
    /// Teklifin `mime` türünü en çok `max_bytes` bayt ve `timeout` süreyle okur; çağıranın
    /// thread'ini bloklar (ana thread'den çağrılamaz). Teklif güncel değilse `Gone`, hassassa
    /// `Sensitive`, tür sunulmuyorsa `NotOffered`, sınır aşılırsa `TooLarge` (okuma durur),
    /// süre dolarsa `Timeout`. Dönen tampon düşerken sıfırlanır.
    pub fn read(
        &self,
        id: DataOfferId,
        mime: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> Result<Zeroizing<Vec<u8>>, DataReadError> {
        if std::thread::current().id() == self.main {
            return Err(DataReadError::MainThread);
        }
        let deadline = Instant::now() + timeout;
        let (reader, writer) = std::io::pipe().map_err(|_| DataReadError::Io)?;
        let (reply, answer) = mpsc::sync_channel(1);
        self.tx
            .send(Command::Receive {
                id,
                mime: mime.to_owned(),
                fd: OwnedFd::from(writer),
                reply,
            })
            .map_err(|_| DataReadError::Closed)?;
        match answer.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(DataReadError::Timeout),
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(DataReadError::Closed),
        }
        read_limited(OwnedFd::from(reader), max_bytes, deadline)
    }

    /// Seçimi bu (MIME, bayt) çiftleriyle koyar (`Context::data_control_set_selection`'ın
    /// thread'ler arası biçimi; ana döngünün yanıtını en çok 1 sn bekler).
    pub fn set_selection(
        &self,
        selection: Selection,
        data: Vec<(String, Arc<[u8]>)>,
    ) -> Result<(), SetSelectionError> {
        if std::thread::current().id() == self.main {
            return Err(SetSelectionError::MainThread);
        }
        validate_selection(&data)?;
        let (reply, answer) = mpsc::sync_channel(1);
        self.tx
            .send(Command::SetSelection {
                selection,
                data,
                reply,
            })
            .map_err(|_| SetSelectionError::Closed)?;
        answer
            .recv_timeout(SET_REPLY_TIMEOUT)
            .map_err(|_| SetSelectionError::Closed)?
    }
}

impl<A: App> Context<A> {
    /// Compositor'ın sunduğu data-control protokolü (`None`: yok). Bağlanmadan bilinir.
    pub fn data_control_protocol(&self) -> Option<DataControlProtocol> {
        let dc = &self.data_control;
        if let Some((m, _, _)) = &dc.manager {
            return Some(m.protocol());
        }
        match (dc.ext_global, dc.wlr_global, force_wlr()) {
            (_, Some(_), true) => Some(DataControlProtocol::Wlr),
            (Some(_), _, _) => Some(DataControlProtocol::Ext),
            (None, Some(_), _) => Some(DataControlProtocol::Wlr),
            (None, None, _) => None,
        }
    }

    /// Birincil seçim izlenebilir/konabilir mi (ext, ya da wlr sürüm ≥ 2). Bağlanmadan bilinir.
    pub fn data_control_primary_supported(&self) -> bool {
        let dc = &self.data_control;
        if dc.manager.is_some() {
            return dc.primary_supported();
        }
        match self.data_control_protocol() {
            Some(DataControlProtocol::Ext) => true,
            Some(DataControlProtocol::Wlr) => dc.wlr_global.is_some_and(|(_, v)| v >= 2),
            None => false,
        }
    }

    /// Pano izlemeyi başlatır (yönetici bağlanır, varsayılan koltukta aygıt yaratılır) ve
    /// tutamacı döner. Compositor güncel seçimi hemen bildirir (`SelectionOffered` ya da
    /// `SelectionCleared`). Zaten başlatılmışsa aynı tutamaç.
    pub fn data_control_start(&mut self) -> Result<DataControlHandle, DataControlError> {
        self.data_control_bind()?;
        let handle = self.data_control_channel()?;
        if self.data_control.device.is_none() {
            let dc = &mut self.data_control;
            let seat = dc.seat.clone().ok_or(DataControlError::NoSeat)?;
            let Some((manager, _, _)) = &dc.manager else {
                return Err(DataControlError::Unsupported);
            };
            dc.device = Some(manager.get_device(&seat, &self.qh));
            tracing::debug!(protocol = ?manager.protocol(), "pano izleme başladı");
        }
        Ok(handle)
    }

    /// Birincil seçim (`Selection::Primary`) olaylarını açar/kapatır (varsayılan kapalı).
    /// İzlenirken açılınca güncel birincil seçim hemen olay olarak gelir (compositor henüz
    /// bildirmediyse ilk bildirimiyle; `start`'tan hemen sonra açmak güvenli). Desteklenmiyorsa
    /// (wlr sürüm 1) açık olsa da olay gelmez (`data_control_primary_supported`).
    pub fn data_control_watch_primary(&mut self, on: bool) {
        let event = self.data_control.offers.set_watch_primary(on);
        if let Some(event) = event
            && self.data_control.device.is_some()
            && self.data_control.primary_supported()
        {
            self.pending.push(RuntimeEvent::DataControl(event));
        }
    }

    /// İzlemeyi durdurur: aygıt ve güncel teklifler bırakılır (olay gelmez). Bizim
    /// koyduğumuz seçim, başka istemci yerine geçene dek sunulmaya devam eder.
    pub fn data_control_stop(&mut self) {
        let dc = &mut self.data_control;
        for offer in dc.offers.clear() {
            offer.destroy();
        }
        if let Some(device) = dc.device.take() {
            device.destroy();
            tracing::debug!("pano izleme durdu");
        }
    }

    /// İzleme başlatılmışsa tutamaç.
    pub fn data_control_handle(&self) -> Option<DataControlHandle> {
        self.data_control
            .device
            .as_ref()
            .and(self.data_control.channel.as_ref())
            .map(|(h, _)| h.clone())
    }

    /// Seçimi (MIME, bayt) çiftleriyle koyar; odak ya da serial istemez. Kaynak, başka bir
    /// istemci seçimi alana dek (ya da süreç bitene dek) sunulur; her istek
    /// `COPY_WRITE_TIMEOUT` ile sınırlı, bloklamadan yazılır. Aynı seçimdeki önceki
    /// kaynağımız yok edilir. İzleme başlatılmış olmalı. Kaynak işaret MIME'ını da sunar:
    /// geri gelen teklif `own = true`.
    pub fn data_control_set_selection(
        &mut self,
        selection: Selection,
        data: Vec<(String, Arc<[u8]>)>,
    ) -> Result<(), SetSelectionError> {
        if self.lock.is_some() {
            return Err(SetSelectionError::Locked);
        }
        validate_selection(&data)?;
        let dc = &mut self.data_control;
        if dc.device.is_none() {
            return Err(SetSelectionError::NotStarted);
        }
        if selection == Selection::Primary && !dc.primary_supported() {
            return Err(SetSelectionError::Unsupported);
        }
        let (Some((manager, _, _)), Some(device)) = (&dc.manager, &dc.device) else {
            return Err(SetSelectionError::NotStarted);
        };
        let source = manager.create_source(&self.qh);
        for (mime, _) in &data {
            source.offer(mime.clone());
        }
        source.offer(owner_mime().to_owned());
        device.set_selection(selection, &source);
        let count = data.len();
        let bytes: usize = data.iter().map(|(_, b)| b.len()).sum();
        let data = data.into_iter().map(|(m, b)| (m, SharedBytes(b))).collect();
        if let Some(old) = dc.owned_slot(selection).replace(Owned { source, data }) {
            // Eski baytlar burada düşer (son sahip bizsek sıfırlanır).
            old.source.destroy();
        }
        tracing::debug!(
            ?selection,
            mimes = count,
            bytes,
            "seçim data-control ile kondu"
        );
        Ok(())
    }

    /// Okumayı kısa ömürlü bir thread'de yapar (bkz. [`DataControlHandle::read`]); sonuç her
    /// istek için tam bir kez `App::data_control_read` ile, bu çağrının içinden değil, gelir.
    pub fn data_control_read(
        &mut self,
        id: DataOfferId,
        mime: &str,
        max_bytes: usize,
        timeout: Duration,
    ) -> DataReadId {
        let rid = DataReadId(self.next_id());
        let handle = self.data_control_handle();
        let Some(handle) = handle else {
            self.pending.push(RuntimeEvent::DataRead(
                rid,
                ReadOutcome(Err(DataReadError::Gone)),
            ));
            return rid;
        };
        let mime = mime.to_owned();
        let tx = handle.tx.clone();
        let spawned = std::thread::Builder::new()
            .name("nuxut-dc-read".into())
            .spawn(move || {
                let result = handle.read(id, &mime, max_bytes, timeout);
                // Runtime kapandıysa sonuç düşer (tampon sıfırlanır).
                let _ = tx.send(Command::ReadDone(rid, result));
            });
        if spawned.is_err() {
            self.pending.push(RuntimeEvent::DataRead(
                rid,
                ReadOutcome(Err(DataReadError::Io)),
            ));
        }
        rid
    }

    /// Kuyruktaki olay hâlâ teslim edilmeli mi (teslim kapısı).
    pub(crate) fn data_control_deliverable(&self, event: &DataControlEvent) -> bool {
        self.data_control.offers.deliverable(event)
    }

    fn data_control_bind(&mut self) -> Result<(), DataControlError> {
        if self.data_control.manager.is_some() {
            return Ok(());
        }
        let dc = &self.data_control;
        let Some(registry) = &dc.registry else {
            return Err(DataControlError::Unsupported);
        };
        let (manager, name, version) = match (dc.ext_global, dc.wlr_global) {
            (Some((name, _)), wlr) if !(force_wlr() && wlr.is_some()) => {
                let m = registry
                    .bind::<ExtDataControlManagerV1, _, State<A>>(name, 1, &self.qh, NoEvents);
                (Manager::Ext(m), name, 1)
            }
            (_, Some((name, advertised))) => {
                // Sürüm 2: birincil seçim olayı ve isteği.
                let version = advertised.min(2);
                let m = registry.bind::<ZwlrDataControlManagerV1, _, State<A>>(
                    name, version, &self.qh, NoEvents,
                );
                (Manager::Wlr(m), name, version)
            }
            _ => return Err(DataControlError::Unsupported),
        };
        tracing::debug!(protocol = ?manager.protocol(), version, "data-control yöneticisi bağlandı");
        self.data_control.manager = Some((manager, name, version));
        Ok(())
    }

    fn data_control_channel(&mut self) -> Result<DataControlHandle, DataControlError> {
        if let Some((handle, _)) = &self.data_control.channel {
            return Ok(handle.clone());
        }
        let (tx, rx) = calloop::channel::channel::<Command>();
        let token = self
            .handle
            .insert_source(rx, |event, _, state: &mut State<A>| {
                if let calloop::channel::Event::Msg(cmd) = event {
                    state.cx.data_control_command(cmd);
                }
            })
            .map_err(|_| DataControlError::Unsupported)?;
        let handle = DataControlHandle {
            tx,
            main: std::thread::current().id(),
        };
        self.data_control.channel = Some((handle.clone(), token));
        Ok(handle)
    }

    fn data_control_command(&mut self, cmd: Command) {
        match cmd {
            Command::Receive {
                id,
                mime,
                fd,
                reply,
            } => {
                let result = self.data_control_receive(id, mime, &fd);
                // Bizim yazma ucumuz hemen kapanır: okuyan yalnızca kaynağın ucunu bekler.
                drop(fd);
                if let Err(e) = &result {
                    tracing::debug!(id = id.0, error = ?e, "pano okuması reddedildi");
                }
                let _ = reply.send(result);
            }
            Command::SetSelection {
                selection,
                data,
                reply,
            } => {
                let _ = reply.send(self.data_control_set_selection(selection, data));
            }
            Command::ReadDone(id, result) => {
                match &result {
                    Ok(b) => tracing::debug!(id = id.0, bytes = b.len(), "pano okundu"),
                    Err(e) => tracing::debug!(id = id.0, error = ?e, "pano okunamadı"),
                }
                self.pending
                    .push(RuntimeEvent::DataRead(id, ReadOutcome(result)));
            }
        }
    }

    fn data_control_receive(
        &mut self,
        id: DataOfferId,
        mime: String,
        fd: &OwnedFd,
    ) -> Result<(), DataReadError> {
        let locked = self.lock.is_some();
        let offer = self.data_control.offers.admit_read(id, &mime, locked)?;
        offer.receive(mime, fd.as_fd());
        self.conn.flush().map_err(|_| DataReadError::Io)?;
        tracing::debug!(id = id.0, "pano okuması başladı");
        Ok(())
    }

    fn data_control_selection(
        &mut self,
        device: ObjectId,
        selection: Selection,
        offer: Option<Offer>,
    ) {
        if self.data_control.device.as_ref().map(Device::id) != Some(device) {
            if let Some(offer) = offer {
                offer.destroy();
            }
            return;
        }
        let next = offer.map(|offer| {
            let list = offer
                .data()
                .map(|d| std::mem::take(&mut *d.0.lock().unwrap_or_else(|e| e.into_inner())))
                .unwrap_or_default();
            let id = DataOfferId(self.next_id());
            tracing::debug!(
                id = id.0,
                ?selection,
                mimes = list.mimes.len(),
                own = list.own,
                sensitive = list.sensitive,
                truncated = list.truncated,
                "pano teklifi"
            );
            (id, offer, list)
        });
        let (old, event) = self.data_control.offers.replace(selection, next);
        if let Some(old) = old {
            old.destroy();
        }
        if let Some(event) = event {
            self.pending.push(RuntimeEvent::DataControl(event));
        }
    }

    /// Aygıt bitti / koltuk ya da global gitti: her şey bırakılır, `Stopped` (izleniyorduysa).
    fn data_control_reset(&mut self, drop_manager: bool) {
        let dc = &mut self.data_control;
        let was_active = dc.device.is_some();
        for offer in dc.offers.clear() {
            offer.destroy();
        }
        if let Some(device) = dc.device.take() {
            device.destroy();
        }
        for owned in [dc.owned_clipboard.take(), dc.owned_primary.take()]
            .into_iter()
            .flatten()
        {
            owned.source.destroy();
        }
        if drop_manager && let Some((manager, _, _)) = dc.manager.take() {
            manager.destroy();
        }
        if was_active {
            self.pending
                .push(RuntimeEvent::DataControl(DataControlEvent::Stopped));
        }
    }

    fn data_control_send(&mut self, source: ObjectId, mime: &str, fd: OwnedFd) {
        let locked = self.lock.is_some();
        let dc = &self.data_control;
        let payload = [&dc.owned_clipboard, &dc.owned_primary]
            .into_iter()
            .flatten()
            .find(|o| o.source.id() == source)
            .and_then(|o| o.data.iter().find(|(m, _)| m == mime))
            .map(|(_, b)| Payload::Bytes(b.clone()));
        let Some(payload) = payload else {
            // Bilinmeyen kaynak, sunulmayan tür ya da işaret MIME'ı: boru boş kapanır.
            return;
        };
        if locked {
            tracing::debug!("kilitliyken pano isteği reddedildi; boru kapatıldı");
            return;
        }
        self.serve_payload(payload, fd);
    }

    fn data_control_cancelled(&mut self, source: ObjectId) {
        let dc = &mut self.data_control;
        for selection in [Selection::Clipboard, Selection::Primary] {
            let slot = dc.owned_slot(selection);
            if slot.as_ref().is_some_and(|o| o.source.id() == source)
                && let Some(owned) = slot.take()
            {
                owned.source.destroy();
                tracing::debug!(?selection, "data-control seçimi başka istemciye geçti");
            }
        }
    }

    /// Compositor aygıtı bitirdi (güncel aygıtsa): `Stopped`.
    fn data_control_finished(&mut self, device: ObjectId) {
        if self.data_control.device.as_ref().map(Device::id) == Some(device) {
            tracing::info!("data-control aygıtı bitti");
            self.data_control_reset(false);
        }
    }

    /// Varsayılan koltuk (ilk `wl_seat`).
    pub(crate) fn data_control_seat_added(&mut self, seat: WlSeat) {
        if self.data_control.seat.is_none() {
            self.data_control.seat = Some(seat);
        }
    }

    /// Koltuk gidiyor: varsayılan koltuksa izleme durur; `next` yeni varsayılan olur.
    pub(crate) fn data_control_seat_removed(&mut self, seat: &WlSeat, next: Option<WlSeat>) {
        if self.data_control.seat.as_ref() != Some(seat) {
            return;
        }
        self.data_control.seat = next;
        self.data_control_reset(false);
    }
}

/// Test düğmesi: `NUXUT_DATA_CONTROL=wlr` her ikisi de varken wlr yedeğini seçtirir.
fn force_wlr() -> bool {
    std::env::var_os("NUXUT_DATA_CONTROL").is_some_and(|v| v == "wlr")
}

// ---------------------------------------------------------------------------------------------
// Dispatch.
// ---------------------------------------------------------------------------------------------

/// Aygıt nesnesi verisi.
#[derive(Debug)]
pub(crate) struct DeviceData;

/// Teklif nesnesi verisi: `offer` olaylarıyla biriken türler.
#[derive(Debug, Default)]
pub(crate) struct OfferData(Mutex<MimeList>);

/// Kaynak nesnesi verisi.
#[derive(Debug)]
pub(crate) struct SourceData;

impl<A: App> Dispatch2<ExtDataControlDeviceV1, State<A>> for DeviceData {
    event_created_child!(State<A>, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, OfferData::default())
    ]);

    fn event(
        &self,
        state: &mut State<A>,
        device: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        use ext_data_control_device_v1::Event;
        match event {
            Event::Selection { id } => {
                state.cx.data_control_selection(
                    device.id(),
                    Selection::Clipboard,
                    id.map(Offer::Ext),
                );
            }
            Event::PrimarySelection { id } => {
                state.cx.data_control_selection(
                    device.id(),
                    Selection::Primary,
                    id.map(Offer::Ext),
                );
            }
            Event::Finished => state.cx.data_control_finished(device.id()),
            _ => {}
        }
    }
}

impl<A: App> Dispatch2<ZwlrDataControlDeviceV1, State<A>> for DeviceData {
    event_created_child!(State<A>, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, OfferData::default())
    ]);

    fn event(
        &self,
        state: &mut State<A>,
        device: &ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        use zwlr_data_control_device_v1::Event;
        match event {
            Event::Selection { id } => {
                state.cx.data_control_selection(
                    device.id(),
                    Selection::Clipboard,
                    id.map(Offer::Wlr),
                );
            }
            Event::PrimarySelection { id } => {
                state.cx.data_control_selection(
                    device.id(),
                    Selection::Primary,
                    id.map(Offer::Wlr),
                );
            }
            Event::Finished => state.cx.data_control_finished(device.id()),
            _ => {}
        }
    }
}

impl<A: App> Dispatch2<ExtDataControlOfferV1, State<A>> for OfferData {
    fn event(
        &self,
        _: &mut State<A>,
        _: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(mime_type, owner_mime());
        }
    }
}

impl<A: App> Dispatch2<ZwlrDataControlOfferV1, State<A>> for OfferData {
    fn event(
        &self,
        _: &mut State<A>,
        _: &ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            self.0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(mime_type, owner_mime());
        }
    }
}

impl<A: App> Dispatch2<ExtDataControlSourceV1, State<A>> for SourceData {
    fn event(
        &self,
        state: &mut State<A>,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.cx.data_control_send(source.id(), &mime_type, fd);
            }
            ext_data_control_source_v1::Event::Cancelled => {
                state.cx.data_control_cancelled(source.id());
            }
            _ => {}
        }
    }
}

impl<A: App> Dispatch2<ZwlrDataControlSourceV1, State<A>> for SourceData {
    fn event(
        &self,
        state: &mut State<A>,
        source: &ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.cx.data_control_send(source.id(), &mime_type, fd);
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.cx.data_control_cancelled(source.id());
            }
            _ => {}
        }
    }
}

/// Yöneticilerin olayı yok.
impl<D> Dispatch2<ExtDataControlManagerV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

impl<D> Dispatch2<ZwlrDataControlManagerV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &ZwlrDataControlManagerV1,
        _: <ZwlrDataControlManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// Globallerin kaydı (açılışta `run_with`'ten) ve çalışma sırasında ilanı/geri çekilmesi.
/// Bağlanma tembeldir (`data_control_start`).
#[derive(Debug)]
pub(crate) struct DataControlGlobal;

impl DataControlGlobal {
    pub(crate) fn record<A: App>(state: &mut State<A>, interface: &str, name: u32, version: u32) {
        if interface != EXT_INTERFACE && interface != WLR_INTERFACE {
            return;
        }
        let registry = state.registry().registry().clone();
        let dc = &mut state.cx.data_control;
        dc.registry.get_or_insert(registry);
        match interface {
            EXT_INTERFACE if dc.ext_global.is_none() => dc.ext_global = Some((name, version)),
            WLR_INTERFACE if dc.wlr_global.is_none() => dc.wlr_global = Some((name, version)),
            _ => {}
        }
    }
}

impl<A: App> RegistryHandler<State<A>> for DataControlGlobal {
    fn new_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        interface: &str,
        version: u32,
    ) {
        Self::record(state, interface, name, version);
    }

    fn remove_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        _: &str,
    ) {
        let dc = &mut state.cx.data_control;
        if dc.ext_global.is_some_and(|(n, _)| n == name) {
            dc.ext_global = None;
        }
        if dc.wlr_global.is_some_and(|(n, _)| n == name) {
            dc.wlr_global = None;
        }
        if dc.manager.as_ref().is_some_and(|(_, n, _)| *n == name) {
            tracing::info!("data-control global'i geri çekildi");
            state.cx.data_control_reset(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const MARK: &str = "application/x-nuxut-owner-0000000000000000";

    #[test]
    fn mime_list_caps_count_and_length_and_flags_truncation() {
        let mut l = MimeList::default();
        for i in 0..MAX_MIMES + 5 {
            l.push(format!("type/{i}"), MARK);
        }
        assert_eq!(l.mimes.len(), MAX_MIMES);
        assert!(l.truncated);

        let mut l = MimeList::default();
        l.push("a".repeat(MAX_MIME_LEN), MARK);
        assert!(!l.truncated);
        l.push("a".repeat(MAX_MIME_LEN + 1), MARK);
        l.push(String::new(), MARK);
        assert_eq!(l.mimes.len(), 1);
        assert!(l.truncated);
    }

    #[test]
    fn mime_list_detects_and_strips_owner_marker_and_dedups() {
        let mut l = MimeList::default();
        l.push("text/plain".into(), MARK);
        l.push(MARK.into(), MARK);
        l.push("text/plain".into(), MARK);
        assert!(l.own);
        assert!(!l.truncated);
        assert_eq!(l.mimes, ["text/plain"]);
        // İşaret, sınır dolduktan sonra gelse de tanınır.
        let mut l = MimeList::default();
        for i in 0..MAX_MIMES {
            l.push(format!("t/{i}"), MARK);
        }
        l.push(MARK.into(), MARK);
        assert!(l.own);
        assert!(!l.truncated);
    }

    #[test]
    fn sensitive_mimes() {
        for m in [
            "x-kde-passwordManagerHint",
            "X-KDE-PASSWORDMANAGERHINT",
            "application/x-nuxut-secret",
            "application/x-Password",
            "text/x-SECRET-thing",
        ] {
            assert!(is_sensitive_mime(m), "{m}");
        }
        for m in ["text/plain", "image/png", "text/uri-list", "sec/ret", ""] {
            assert!(!is_sensitive_mime(m), "{m}");
        }
    }

    #[test]
    fn mime_list_flags_sensitive_even_when_dropped() {
        let mut l = MimeList::default();
        l.push("text/plain".into(), MARK);
        l.push(PASSWORD_HINT_MIME.into(), MARK);
        assert!(l.sensitive);
        // Liste listede kalır (tüketici görsün), okuma yine reddedilir.
        assert!(l.mimes.iter().any(|m| m == PASSWORD_HINT_MIME));

        let mut l = MimeList::default();
        for i in 0..MAX_MIMES {
            l.push(format!("t/{i}"), MARK);
        }
        l.push("application/x-secret".into(), MARK);
        assert!(l.truncated);
        assert!(l.sensitive);
        let mut l = MimeList::default();
        l.push(format!("x/password{}", "a".repeat(MAX_MIME_LEN)), MARK);
        assert!(l.truncated && l.sensitive);
        // İşaret hassas sayılmaz.
        let mut l = MimeList::default();
        l.push(MARK.into(), MARK);
        assert!(l.own && !l.sensitive);
    }

    fn list(mimes: &[&str]) -> MimeList {
        let mut l = MimeList::default();
        for m in mimes {
            l.push((*m).to_owned(), MARK);
        }
        l
    }

    const CB: Selection = Selection::Clipboard;
    const PR: Selection = Selection::Primary;

    #[test]
    fn book_replaces_and_returns_old_offer() {
        let mut b = OfferBook::<&str>::default();
        let (old, ev) = b.replace(CB, Some((DataOfferId(1), "o1", list(&["text/plain"]))));
        assert_eq!(old, None);
        assert_eq!(
            ev,
            Some(DataControlEvent::SelectionOffered {
                selection: CB,
                id: DataOfferId(1),
                mimes: vec!["text/plain".into()],
                own: false,
                sensitive: false,
                truncated: false,
            })
        );
        let (old, ev) = b.replace(CB, None);
        assert_eq!(old, Some("o1"));
        assert_eq!(
            ev,
            Some(DataControlEvent::SelectionCleared { selection: CB })
        );
        assert_eq!(
            b.admit_read(DataOfferId(1), "text/plain", false),
            Err(DataReadError::Gone)
        );
    }

    #[test]
    fn book_gates_stale_offers() {
        let mut b = OfferBook::<&str>::default();
        let (_, first) = b.replace(CB, Some((DataOfferId(1), "o1", list(&["a/b"]))));
        let (_, second) = b.replace(CB, Some((DataOfferId(2), "o2", list(&["a/b"]))));
        // Kuyrukta bekleyen ilk olay artık teslim edilmez.
        assert!(!b.deliverable(&first.unwrap()));
        assert!(b.deliverable(&second.unwrap()));
        assert!(b.deliverable(&DataControlEvent::Stopped));
    }

    #[test]
    fn book_read_admission() {
        let mut b = OfferBook::<&str>::default();
        b.replace(
            CB,
            Some((DataOfferId(1), "o1", list(&["text/plain", "image/png"]))),
        );
        assert_eq!(b.admit_read(DataOfferId(1), "image/png", false), Ok(&"o1"));
        assert_eq!(
            b.admit_read(DataOfferId(1), "text/html", false),
            Err(DataReadError::NotOffered)
        );
        assert_eq!(
            b.admit_read(DataOfferId(1), MARK, false),
            Err(DataReadError::NotOffered)
        );
        assert_eq!(
            b.admit_read(DataOfferId(1), "text/plain", true),
            Err(DataReadError::Locked)
        );
        assert_eq!(
            b.admit_read(DataOfferId(9), "text/plain", false),
            Err(DataReadError::Gone)
        );
        // Hassas teklif: sunulan güvenli tür de okunmaz.
        b.replace(
            CB,
            Some((
                DataOfferId(2),
                "o2",
                list(&["text/plain", PASSWORD_HINT_MIME]),
            )),
        );
        assert_eq!(
            b.admit_read(DataOfferId(2), "text/plain", false),
            Err(DataReadError::Sensitive)
        );
        assert_eq!(
            b.admit_read(DataOfferId(2), PASSWORD_HINT_MIME, false),
            Err(DataReadError::Sensitive)
        );
    }

    #[test]
    fn book_own_flag_passes_through() {
        let mut b = OfferBook::<&str>::default();
        let (_, ev) = b.replace(CB, Some((DataOfferId(1), "o", list(&["text/plain", MARK]))));
        let Some(DataControlEvent::SelectionOffered { own, mimes, .. }) = ev else {
            panic!("olay yok");
        };
        assert!(own);
        assert_eq!(mimes, ["text/plain"]);
    }

    #[test]
    fn book_primary_is_tracked_silently_until_watched() {
        let mut b = OfferBook::<&str>::default();
        // İzlenmezken olay yok ama teklif tutulur.
        let (_, ev) = b.replace(PR, Some((DataOfferId(1), "p1", list(&["text/plain"]))));
        assert_eq!(ev, None);
        // Açılınca güncel durum hemen olay olur; ikinci açma tekrar etmez.
        let ev = b.set_watch_primary(true);
        assert!(matches!(
            ev,
            Some(DataControlEvent::SelectionOffered {
                selection: PR,
                id: DataOfferId(1),
                ..
            })
        ));
        assert_eq!(b.set_watch_primary(true), None);
        assert_eq!(b.admit_read(DataOfferId(1), "text/plain", false), Ok(&"p1"));
        // Pano ve birincil bağımsız.
        let (old, _) = b.replace(CB, Some((DataOfferId(2), "c2", list(&["text/plain"]))));
        assert_eq!(old, None);
        assert_eq!(b.admit_read(DataOfferId(1), "text/plain", false), Ok(&"p1"));
        // Kapatınca kuyruktaki birincil olaylar düşer, pano etkilenmez.
        let queued = DataControlEvent::SelectionCleared { selection: PR };
        assert!(b.deliverable(&queued));
        assert_eq!(b.set_watch_primary(false), None);
        assert!(!b.deliverable(&queued));
        assert!(b.deliverable(&DataControlEvent::SelectionCleared { selection: CB }));
        // Durum bilinmeden açma: olay yok (compositor'ın ilk bildirimi gelecek).
        let mut e = OfferBook::<&str>::default();
        assert_eq!(e.set_watch_primary(true), None);
        let (_, ev) = e.replace(PR, None);
        assert_eq!(
            ev,
            Some(DataControlEvent::SelectionCleared { selection: PR })
        );
        // Bilinen boş birincil seçimde açma: Cleared.
        let mut e = OfferBook::<&str>::default();
        e.replace(PR, None);
        assert_eq!(
            e.set_watch_primary(true),
            Some(DataControlEvent::SelectionCleared { selection: PR })
        );
    }

    #[test]
    fn book_clear_returns_all_offers() {
        let mut b = OfferBook::<&str>::default();
        b.replace(CB, Some((DataOfferId(1), "c", list(&["a/b"]))));
        b.replace(PR, Some((DataOfferId(2), "p", list(&["a/b"]))));
        b.set_watch_primary(true);
        let mut all: Vec<_> = b.clear().collect();
        all.sort_unstable();
        assert_eq!(all, ["c", "p"]);
        assert_eq!(
            b.admit_read(DataOfferId(1), "a/b", false),
            Err(DataReadError::Gone)
        );
        // Anahtar korunur; birincil durum yeniden bilinmez.
        assert!(b.watch_primary);
        assert!(!b.primary_known);
    }

    #[test]
    fn owner_mime_is_stable_and_prefixed() {
        let m = owner_mime();
        assert!(m.starts_with(OWNER_MIME_PREFIX));
        assert_eq!(m.len(), OWNER_MIME_PREFIX.len() + 16);
        assert_eq!(m, owner_mime());
    }

    fn sel(mimes: &[&str]) -> Vec<(String, Arc<[u8]>)> {
        mimes
            .iter()
            .map(|m| ((*m).to_owned(), Arc::from(&b"x"[..])))
            .collect()
    }

    #[test]
    fn selection_validation() {
        assert_eq!(
            validate_selection(&sel(&["text/plain", "image/png"])),
            Ok(())
        );
        let bad = SetSelectionError::Invalid;
        assert_eq!(validate_selection(&[]), Err(bad));
        assert_eq!(validate_selection(&sel(&[""])), Err(bad));
        assert_eq!(validate_selection(&sel(&["a/b", "a/b"])), Err(bad));
        assert_eq!(validate_selection(&sel(&["a\nb"])), Err(bad));
        assert_eq!(validate_selection(&sel(&[owner_mime()])), Err(bad));
        let long = "a".repeat(MAX_MIME_LEN + 1);
        assert_eq!(validate_selection(&sel(&[&long])), Err(bad));
        let many: Vec<String> = (0..=MAX_MIMES).map(|i| format!("t/{i}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(validate_selection(&sel(&many)), Err(bad));
        assert_eq!(validate_selection(&sel(&many[..MAX_MIMES])), Ok(()));
    }

    #[test]
    fn zeroize_only_when_sole_owner() {
        let mut a: Arc<[u8]> = Arc::from(&b"secret"[..]);
        let other = a.clone();
        assert!(!zeroize_if_unique(&mut a));
        assert_eq!(&*other, b"secret");
        drop(other);
        assert!(zeroize_if_unique(&mut a));
        assert!(a.iter().all(|&b| b == 0));
    }

    #[test]
    fn shared_bytes_drop_zeroizes_last_owner_only() {
        let app: Arc<[u8]> = Arc::from(&b"keep"[..]);
        let ours = SharedBytes(app.clone());
        let copy = ours.clone();
        drop(ours);
        drop(copy);
        // Uygulama hâlâ sahip: dokunulmadı.
        assert_eq!(&*app, b"keep");
    }

    #[test]
    fn append_grows_without_losing_data_and_respects_limit() {
        let mut buf = Zeroizing::new(Vec::new());
        let mut expect = Vec::new();
        for i in 0..100u8 {
            let piece = vec![i; 1000];
            append_zeroizing(&mut buf, &piece, 100_000);
            expect.extend_from_slice(&piece);
        }
        assert_eq!(*buf, expect);
        assert!(buf.capacity() <= 100_000);
    }

    fn pipe() -> (OwnedFd, std::io::PipeWriter) {
        let (r, w) = std::io::pipe().unwrap();
        (OwnedFd::from(r), w)
    }

    #[test]
    fn read_until_eof_within_limit() {
        let (r, mut w) = pipe();
        let t = std::thread::spawn(move || {
            w.write_all(&vec![7u8; 200_000]).unwrap();
        });
        let got = read_limited(r, 200_000, Instant::now() + Duration::from_secs(5)).unwrap();
        t.join().unwrap();
        assert_eq!(got.len(), 200_000);
        assert!(got.iter().all(|&b| b == 7));
    }

    #[test]
    fn read_over_limit_is_too_large() {
        let (r, mut w) = pipe();
        let t = std::thread::spawn(move || {
            // Okuyan erken kapatır: EPIPE beklenir, yok sayılır.
            let _ = w.write_all(&[1u8; 1001]);
        });
        let err = read_limited(r, 1000, Instant::now() + Duration::from_secs(5)).unwrap_err();
        t.join().unwrap();
        assert_eq!(err, DataReadError::TooLarge);
    }

    #[test]
    fn silent_writer_times_out() {
        let (r, mut w) = pipe();
        w.write_all(b"partial").unwrap();
        let start = Instant::now();
        let err = read_limited(r, 1000, start + Duration::from_millis(150)).unwrap_err();
        assert_eq!(err, DataReadError::Timeout);
        assert!(start.elapsed() >= Duration::from_millis(150));
        assert!(start.elapsed() < Duration::from_secs(2));
        drop(w);
    }

    #[test]
    fn empty_pipe_reads_empty() {
        let (r, w) = pipe();
        drop(w);
        let got = read_limited(r, 10, Instant::now() + Duration::from_secs(1)).unwrap();
        assert!(got.is_empty());
    }

    #[test]
    fn errors_carry_no_content() {
        for e in [
            DataReadError::Gone,
            DataReadError::TooLarge,
            DataReadError::Timeout,
        ] {
            assert!(!format!("{e} {e:?}").contains("secret"));
        }
    }
}
