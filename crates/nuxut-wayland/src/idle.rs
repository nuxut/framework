//! Boşta bildirimi (`ext-idle-notify-v1`): kullanıcı belirli bir süre girdi vermezse compositor
//! `idled`, girdi gelince `resumed` gönderir. Shell'in boşta servisi (karartma, ekranı kapatma,
//! kilit, askıya alma) bunun üzerine kurulur. Sayaç compositor'dadır: runtime yoklama yapmaz,
//! zamanlayıcı kurmaz (boşta sıfır CPU).
//!
//! Sözleşme (runtime uygular):
//! - Global yoksa durum açıkça `IdleSupport::Unsupported`; `idle_watch`
//!   `Err(IdleWatchError::Unsupported)` döner (uygulama özelliği kapatır, sessiz `None` yok).
//! - İki tür izleme (global 1..=2 aralığında bağlanır):
//!   - `IdleInhibitors::Respect` (varsayılan) → `get_idle_notification`: Wayland idle
//!     inhibitor'ları (`zwp_idle_inhibit`, ör. tam ekran video) etkinken `idled` gelmez.
//!     Karartma/ekran kapatma/kilit/askıya alma için doğru tür.
//!   - `IdleInhibitors::Ignore` → yalnızca sürüm 2'deki `get_input_idle_notification`:
//!     inhibitor'lar yok sayılır, salt girdi yokluğu ölçülür ("uzakta" durumu gibi). Sürüm 1
//!     global'de `Err(IdleWatchError::InputIdleUnsupported)`; global sonradan sürüm 1 olarak
//!     yeniden ilan edilirse var olan `Ignore` izleme `Respect` olarak kurulur (uyarı loglanır).
//! - Ayar değişince yeniden kayıt: `idle_set_timeout` aynı kimlikle nesneyi yok edip yeni
//!   süreyle yeniden yaratır (protokolde süre değiştirme isteği yok; sayaç baştan başlar).
//!   İzleme o an boştaysa `idle=false` verilir.
//! - Süre kabloda `u32` milisaniyedir: `[1 ms, u32::MAX ms]` aralığına kırpılır (0 ve
//!   milisaniye altı 1 ms olur — "hemen boşta" anlamı korunur).
//! - Her izleme bağımsız bir bildirim nesnesidir; aynı anda birden çok izleme olabilir.
//! - Koltuk (varsayılan: ilk `wl_seat`) yokken kurulan izleme bekler; koltuk gelince nesne
//!   yaratılır. Koltuk ya da notifier global'i kaybolursa nesneler yok edilir, izlemeler
//!   bekleyen duruma döner ve yeniden gelince sayaç baştan kurulur. Kayıp anında boşta olan
//!   izlemeye `idle=false` verilir (uygulama "boşta" durumunda takılı kalmaz).
//! - Olaylar yalnızca durum değiştiğinde iletilir (aynı değer iki kez gelmez); eski nesne
//!   kuşağının geç olayı yok sayılır. `idle_unwatch` sonrası o kimlik için olay gelmez.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryHandler};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};

/// Notifier global'inin arayüz adı.
pub(crate) const NOTIFIER_INTERFACE: &str = "ext_idle_notifier_v1";

/// `Context::idle_watch` ile kurulan izlemenin kimliği. Tekrar kullanılmaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdleWatchId(pub(crate) u64);

/// Compositor'ın boşta bildirimi desteği (`Context::idle_support`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleSupport {
    /// `ext_idle_notifier_v1` global'i yok: boşta izlemesi kurulamaz.
    Unsupported,
    /// Global bağlı. `ignore_inhibitors`: sürüm ≥ 2, `IdleInhibitors::Ignore` kullanılabilir.
    Supported { ignore_inhibitors: bool },
}

/// İzlemenin Wayland idle inhibitor'larına tutumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IdleInhibitors {
    /// `get_idle_notification`: inhibitor etkinken (ör. tam ekran video) boşta sayılmaz.
    /// Karartma, ekran kapatma, kilit, askıya alma için.
    #[default]
    Respect,
    /// `get_input_idle_notification` (protokol sürüm 2): inhibitor'lar yok sayılır, yalnızca
    /// girdi yokluğu ölçülür ("uzakta" durumu gibi; ekran/kilit için KULLANILMAZ).
    Ignore,
}

/// `idle_watch` hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleWatchError {
    /// Compositor `ext-idle-notify-v1` sunmuyor (`IdleSupport::Unsupported`).
    Unsupported,
    /// `IdleInhibitors::Ignore` istendi ama global sürüm 1 (`get_input_idle_notification` yok).
    InputIdleUnsupported,
}

impl std::fmt::Display for IdleWatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "compositor ext-idle-notify-v1 sunmuyor",
            Self::InputIdleUnsupported => {
                "ext-idle-notify-v1 sürüm 1: inhibitor'ları yok sayan izleme yok"
            }
        })
    }
}

impl std::error::Error for IdleWatchError {}

/// Süreyi kablodaki milisaniyeye çevirir: `[1, u32::MAX]` aralığına kırpılır.
pub(crate) fn timeout_ms(timeout: Duration) -> u32 {
    timeout.as_millis().clamp(1, u128::from(u32::MAX)) as u32
}

/// Defterin protokol nesnelerine uygulanacak adımları (sırayla).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdleOp {
    Create {
        id: IdleWatchId,
        timeout_ms: u32,
        generation: u64,
    },
    Destroy(IdleWatchId),
}

#[derive(Debug)]
struct WatchRecord {
    timeout_ms: u32,
    inhibitors: IdleInhibitors,
    /// Canlı nesnenin kuşağı; `None`: nesne yok (bekliyor).
    generation: Option<u64>,
    /// Uygulamaya en son bildirilen durum.
    idle: bool,
}

/// Saf durum makinesi: izlemeler, nesne kuşakları, bildirilen durum. Wayland'e dokunmaz;
/// `IdleOp` üretir (birim testli). `ready`: notifier ve koltuk var (nesne yaratılabilir).
#[derive(Debug, Default)]
pub(crate) struct IdleBook {
    watches: BTreeMap<IdleWatchId, WatchRecord>,
    next_generation: u64,
}

impl IdleBook {
    fn create(&mut self, id: IdleWatchId) -> Option<IdleOp> {
        let rec = self.watches.get_mut(&id)?;
        if rec.generation.is_some() {
            return None;
        }
        self.next_generation += 1;
        rec.generation = Some(self.next_generation);
        Some(IdleOp::Create {
            id,
            timeout_ms: rec.timeout_ms,
            generation: self.next_generation,
        })
    }

    /// Yeni izleme; hazırsa hemen yaratılır, değilse bekler.
    pub fn insert(
        &mut self,
        id: IdleWatchId,
        timeout_ms: u32,
        inhibitors: IdleInhibitors,
        ready: bool,
    ) -> Vec<IdleOp> {
        self.watches.insert(
            id,
            WatchRecord {
                timeout_ms,
                inhibitors,
                generation: None,
                idle: false,
            },
        );
        if ready {
            self.create(id).into_iter().collect()
        } else {
            Vec::new()
        }
    }

    /// Süre değişti (ayar). Bilinmeyen kimlikte `None`. Canlı nesne varsa yok edilip yeni
    /// kuşakla yeniden yaratılır (sayaç baştan); bekleyen izlemede yalnızca süre güncellenir.
    /// İkinci değer: izleme boşta bildirilmişti, uygulamaya `idle=false` verilmeli.
    pub fn retime(&mut self, id: IdleWatchId, timeout_ms: u32) -> Option<(Vec<IdleOp>, bool)> {
        let rec = self.watches.get_mut(&id)?;
        rec.timeout_ms = timeout_ms;
        let was_idle = std::mem::take(&mut rec.idle);
        if rec.generation.take().is_none() {
            return Some((Vec::new(), was_idle));
        }
        let mut ops = vec![IdleOp::Destroy(id)];
        ops.extend(self.create(id));
        Some((ops, was_idle))
    }

    /// İzleme kayıtlı mı (`remove` edilmemiş).
    pub fn contains(&self, id: IdleWatchId) -> bool {
        self.watches.contains_key(&id)
    }

    /// İzlemenin inhibitor tutumu (bilinmeyen kimlikte varsayılan).
    fn inhibitors(&self, id: IdleWatchId) -> IdleInhibitors {
        self.watches
            .get(&id)
            .map(|r| r.inhibitors)
            .unwrap_or_default()
    }

    /// İzlemeyi siler; canlı nesne varsa `Destroy`. Bilinmeyen/silinmiş kimlikte adım yok
    /// (idempotent).
    pub fn remove(&mut self, id: IdleWatchId) -> Vec<IdleOp> {
        match self.watches.remove(&id) {
            Some(rec) if rec.generation.is_some() => vec![IdleOp::Destroy(id)],
            _ => Vec::new(),
        }
    }

    /// Notifier/koltuk durumu değişti. Hazırsa bekleyen her izleme yaratılır. Hazır değilse
    /// canlı nesneler yok edilir; boşta bildirilmiş olanlar ikinci listede döner (uygulamaya
    /// `idle=false` verilmeli).
    pub fn set_ready(&mut self, ready: bool) -> (Vec<IdleOp>, Vec<IdleWatchId>) {
        let ids: Vec<IdleWatchId> = self.watches.keys().copied().collect();
        if ready {
            return (
                ids.into_iter().filter_map(|id| self.create(id)).collect(),
                Vec::new(),
            );
        }
        let mut ops = Vec::new();
        let mut resumed = Vec::new();
        for (id, rec) in &mut self.watches {
            if rec.generation.take().is_some() {
                ops.push(IdleOp::Destroy(*id));
            }
            if std::mem::take(&mut rec.idle) {
                resumed.push(*id);
            }
        }
        (ops, resumed)
    }

    /// Compositor olayı. Canlı kuşaktan ve durum değiştiriyorsa `true` (uygulamaya iletilir).
    pub fn event(&mut self, id: IdleWatchId, generation: u64, idle: bool) -> bool {
        match self.watches.get_mut(&id) {
            Some(rec) if rec.generation == Some(generation) && rec.idle != idle => {
                rec.idle = idle;
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    fn is_live(&self, id: IdleWatchId) -> bool {
        self.watches
            .get(&id)
            .is_some_and(|r| r.generation.is_some())
    }
}

/// Context'teki boşta bildirimi durumu: notifier (global adıyla), varsayılan koltuk, defter,
/// canlı nesneler.
#[derive(Debug, Default)]
pub(crate) struct IdleNotify {
    pub notifier: Option<(ExtIdleNotifierV1, u32)>,
    seat: Option<WlSeat>,
    book: IdleBook,
    objects: HashMap<IdleWatchId, ExtIdleNotificationV1>,
}

impl IdleNotify {
    fn ready(&self) -> bool {
        self.notifier.is_some() && self.seat.is_some()
    }
}

impl<A: App> Context<A> {
    /// Compositor'ın boşta bildirimi desteği. Global sonradan ilan edilebilir/geri
    /// çekilebilir; değer o anki durumdur.
    pub fn idle_support(&self) -> IdleSupport {
        match &self.idle.notifier {
            None => IdleSupport::Unsupported,
            Some((notifier, _)) => IdleSupport::Supported {
                ignore_inhibitors: notifier.version() >= 2,
            },
        }
    }

    /// Boşta izlemesi kurar (`IdleInhibitors::Respect`): kullanıcı `timeout` boyunca girdi
    /// vermezse `App::idle(.., id, true)`, ardından ilk girdide `App::idle(.., id, false)`.
    /// Koltuk varsayılan (ilk `wl_seat`). Süre `[1 ms, u32::MAX ms]` aralığına kırpılır.
    pub fn idle_watch(&mut self, timeout: Duration) -> Result<IdleWatchId, IdleWatchError> {
        self.idle_watch_with(timeout, IdleInhibitors::Respect)
    }

    /// `idle_watch`, inhibitor tutumu seçilerek. `Ignore` protokol sürüm 2 ister.
    pub fn idle_watch_with(
        &mut self,
        timeout: Duration,
        inhibitors: IdleInhibitors,
    ) -> Result<IdleWatchId, IdleWatchError> {
        match self.idle_support() {
            IdleSupport::Unsupported => return Err(IdleWatchError::Unsupported),
            IdleSupport::Supported {
                ignore_inhibitors: false,
            } if inhibitors == IdleInhibitors::Ignore => {
                return Err(IdleWatchError::InputIdleUnsupported);
            }
            IdleSupport::Supported { .. } => {}
        }
        let id = IdleWatchId(self.next_id());
        let ms = timeout_ms(timeout);
        let ready = self.idle.ready();
        let ops = self.idle.book.insert(id, ms, inhibitors, ready);
        tracing::debug!(
            id = id.0,
            timeout_ms = ms,
            ?inhibitors,
            ready,
            "boşta izlemesi kuruldu"
        );
        self.apply_idle_ops(&ops);
        Ok(id)
    }

    /// Ayar değişti: izlemeyi yeni süreyle yeniden kaydeder (aynı kimlik; nesne yok edilip
    /// yeniden yaratılır, sayaç baştan başlar). İzleme o an boştaysa `App::idle(.., false)`
    /// gelir. Bilinmeyen/bitmiş kimlikte `false`.
    pub fn idle_set_timeout(&mut self, id: IdleWatchId, timeout: Duration) -> bool {
        let ms = timeout_ms(timeout);
        let Some((ops, was_idle)) = self.idle.book.retime(id, ms) else {
            return false;
        };
        tracing::debug!(
            id = id.0,
            timeout_ms = ms,
            "boşta izlemesi yeniden kaydedildi"
        );
        self.apply_idle_ops(&ops);
        if was_idle {
            self.pending.push(RuntimeEvent::Idle(id, false));
        }
        true
    }

    /// İzlemeyi bitirir (bildirim nesnesi yok edilir; bu kimlik için artık olay gelmez).
    /// Bilinmeyen ya da zaten bitmiş kimlik yok sayılır.
    pub fn idle_unwatch(&mut self, id: IdleWatchId) {
        let ops = self.idle.book.remove(id);
        self.apply_idle_ops(&ops);
    }

    /// İzleme hâlâ kayıtlı mı (kuyruktaki olayın teslim kapısı).
    pub(crate) fn idle_watch_alive(&self, id: IdleWatchId) -> bool {
        self.idle.book.contains(id)
    }

    fn apply_idle_ops(&mut self, ops: &[IdleOp]) {
        for &op in ops {
            match op {
                IdleOp::Create {
                    id,
                    timeout_ms,
                    generation,
                } => {
                    let (Some((notifier, _)), Some(seat)) =
                        (self.idle.notifier.as_ref(), self.idle.seat.as_ref())
                    else {
                        continue;
                    };
                    let data = IdleData { id, generation };
                    let ignore = self.idle.book.inhibitors(id) == IdleInhibitors::Ignore;
                    let object = if ignore && notifier.version() >= 2 {
                        notifier.get_input_idle_notification(timeout_ms, seat, &self.qh, data)
                    } else {
                        if ignore {
                            tracing::warn!(
                                id = id.0,
                                "notifier sürüm 1: Ignore izleme inhibitor'lara uyarak kuruldu"
                            );
                        }
                        notifier.get_idle_notification(timeout_ms, seat, &self.qh, data)
                    };
                    if let Some(old) = self.idle.objects.insert(id, object) {
                        old.destroy();
                    }
                }
                IdleOp::Destroy(id) => {
                    if let Some(object) = self.idle.objects.remove(&id) {
                        object.destroy();
                    }
                }
            }
        }
    }

    /// Hazırlık (notifier + koltuk) değişmiş olabilir: defteri eşitle.
    fn sync_idle_ready(&mut self) {
        let (ops, resumed) = self.idle.book.set_ready(self.idle.ready());
        self.apply_idle_ops(&ops);
        if !ops.is_empty() {
            tracing::debug!(
                steps = ops.len(),
                ready = self.idle.ready(),
                "boşta izlemeleri eşitlendi"
            );
        }
        for id in resumed {
            self.pending.push(RuntimeEvent::Idle(id, false));
        }
    }

    /// Yeni koltuk: varsayılan koltuk yoksa bu olur ve bekleyen izlemeler kurulur.
    pub(crate) fn idle_seat_added(&mut self, seat: WlSeat) {
        if self.idle.seat.is_some() {
            return;
        }
        self.idle.seat = Some(seat);
        self.sync_idle_ready();
    }

    /// Koltuk gidiyor: varsayılan koltuksa nesneler yok edilir; `next` (kalan bir koltuk)
    /// varsa izlemeler onunla yeniden kurulur.
    pub(crate) fn idle_seat_removed(&mut self, seat: &WlSeat, next: Option<WlSeat>) {
        if self.idle.seat.as_ref() != Some(seat) {
            return;
        }
        self.idle.seat = None;
        self.sync_idle_ready();
        if let Some(next) = next {
            self.idle_seat_added(next);
        }
    }

    fn idle_event(&mut self, id: IdleWatchId, generation: u64, idle: bool) {
        if self.idle.book.event(id, generation, idle) {
            tracing::debug!(id = id.0, idle, "boşta durumu");
            self.pending.push(RuntimeEvent::Idle(id, idle));
        }
    }
}

/// Bildirim nesnesinin verisi: izleme kimliği ve kuşak.
#[derive(Debug)]
pub(crate) struct IdleData {
    id: IdleWatchId,
    generation: u64,
}

impl<A: App> Dispatch2<ExtIdleNotificationV1, State<A>> for IdleData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        let idle = match event {
            ext_idle_notification_v1::Event::Idled => true,
            ext_idle_notification_v1::Event::Resumed => false,
            _ => return,
        };
        state.cx.idle_event(self.id, self.generation, idle);
    }
}

/// Notifier'ın olayı yok.
impl<D> Dispatch2<ExtIdleNotifierV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// Notifier global'inin bağlanması (açılışta `run_with`'ten) ve çalışma sırasında
/// ilanı/geri çekilmesi.
#[derive(Debug)]
pub(crate) struct IdleNotifyGlobal;

impl IdleNotifyGlobal {
    /// Global listesinde varsa bağlar (sürüm 1..=2).
    pub(crate) fn bind<A: App>(state: &mut State<A>, qh: &QueueHandle<State<A>>, name: u32) {
        if state.cx.idle.notifier.is_some() {
            return;
        }
        match state
            .registry()
            .bind_specific::<ExtIdleNotifierV1, _, _>(qh, name, 1..=2, NoEvents)
        {
            Ok(notifier) => {
                tracing::debug!(
                    version = notifier.version(),
                    "ext_idle_notifier_v1 bağlandı"
                );
                state.cx.idle.notifier = Some((notifier, name));
                state.cx.sync_idle_ready();
            }
            Err(e) => tracing::debug!("ext_idle_notifier_v1 bağlanamadı: {e}"),
        }
    }
}

impl<A: App> RegistryHandler<State<A>> for IdleNotifyGlobal {
    fn new_global(
        state: &mut State<A>,
        _: &Connection,
        qh: &QueueHandle<State<A>>,
        name: u32,
        interface: &str,
        _: u32,
    ) {
        if interface == NOTIFIER_INTERFACE {
            Self::bind(state, qh, name);
        }
    }

    fn remove_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        _: &str,
    ) {
        if state
            .cx
            .idle
            .notifier
            .as_ref()
            .is_some_and(|(_, n)| *n == name)
        {
            // Önce bildirim nesneleri (notifier canlıyken), sonra notifier.
            let notifier = state.cx.idle.notifier.take();
            state.cx.sync_idle_ready();
            if let Some((notifier, _)) = notifier {
                notifier.destroy();
            }
            tracing::info!("ext_idle_notifier_v1 geri çekildi");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W1: IdleWatchId = IdleWatchId(1);
    const W2: IdleWatchId = IdleWatchId(2);
    const R: IdleInhibitors = IdleInhibitors::Respect;

    #[test]
    fn timeout_is_clamped_to_wire_range() {
        assert_eq!(timeout_ms(Duration::ZERO), 1);
        assert_eq!(timeout_ms(Duration::from_micros(300)), 1);
        assert_eq!(timeout_ms(Duration::from_millis(1500)), 1500);
        assert_eq!(timeout_ms(Duration::from_secs(300)), 300_000);
        assert_eq!(timeout_ms(Duration::from_secs(u64::MAX)), u32::MAX);
    }

    #[test]
    fn ready_watch_is_created_immediately() {
        let mut book = IdleBook::default();
        assert_eq!(
            book.insert(W1, 2000, R, true),
            vec![IdleOp::Create {
                id: W1,
                timeout_ms: 2000,
                generation: 1
            }]
        );
        assert!(book.is_live(W1));
    }

    #[test]
    fn pending_watches_are_created_when_seat_arrives() {
        let mut book = IdleBook::default();
        assert!(book.insert(W1, 1000, R, false).is_empty());
        assert!(book.insert(W2, 5000, R, false).is_empty());
        assert!(!book.is_live(W1));
        let (ops, resumed) = book.set_ready(true);
        assert_eq!(
            ops,
            vec![
                IdleOp::Create {
                    id: W1,
                    timeout_ms: 1000,
                    generation: 1
                },
                IdleOp::Create {
                    id: W2,
                    timeout_ms: 5000,
                    generation: 2
                },
            ]
        );
        assert!(resumed.is_empty());
        // Tekrar hazır: canlı nesne yeniden yaratılmaz.
        assert_eq!(book.set_ready(true), (Vec::new(), Vec::new()));
    }

    #[test]
    fn unwatch_is_idempotent() {
        let mut book = IdleBook::default();
        book.insert(W1, 1000, R, true);
        assert_eq!(book.remove(W1), vec![IdleOp::Destroy(W1)]);
        assert!(book.remove(W1).is_empty());
        // Bekleyen (nesnesiz) izleme: adım yok.
        book.insert(W2, 1000, R, false);
        assert!(book.remove(W2).is_empty());
        assert!(book.remove(IdleWatchId(99)).is_empty());
        // Silinmiş izlemenin geç olayı iletilmez; kuyruktaki olay da teslimde düşülür.
        assert!(!book.event(W1, 1, true));
        assert!(!book.contains(W1));
    }

    #[test]
    fn seat_loss_destroys_and_resumes_idle_watches() {
        let mut book = IdleBook::default();
        book.insert(W1, 1000, R, true);
        book.insert(W2, 9000, R, true);
        assert!(book.event(W1, 1, true));
        let (ops, resumed) = book.set_ready(false);
        assert_eq!(ops, vec![IdleOp::Destroy(W1), IdleOp::Destroy(W2)]);
        assert_eq!(resumed, vec![W1]);
        // Kaybolan nesnenin geç olayı yok sayılır.
        assert!(!book.event(W1, 1, true));
        // Koltuk geri geldi: yeni kuşakla yeniden kurulur.
        let (ops, _) = book.set_ready(true);
        assert_eq!(
            ops,
            vec![
                IdleOp::Create {
                    id: W1,
                    timeout_ms: 1000,
                    generation: 3
                },
                IdleOp::Create {
                    id: W2,
                    timeout_ms: 9000,
                    generation: 4
                },
            ]
        );
        assert!(book.event(W1, 3, true));
    }

    #[test]
    fn events_are_delivered_only_on_change() {
        let mut book = IdleBook::default();
        book.insert(W1, 1000, R, true);
        // Başlangıç durumu "boşta değil": resumed tek başına iletilmez.
        assert!(!book.event(W1, 1, false));
        assert!(book.event(W1, 1, true));
        assert!(!book.event(W1, 1, true));
        assert!(book.event(W1, 1, false));
        // Başka kuşaktan olay yok sayılır.
        assert!(!book.event(W1, 7, true));
    }

    #[test]
    fn retime_recreates_live_watch_with_new_generation() {
        let mut book = IdleBook::default();
        book.insert(W1, 1000, R, true);
        assert!(book.event(W1, 1, true));
        let (ops, was_idle) = book.retime(W1, 5000).unwrap();
        assert_eq!(
            ops,
            vec![
                IdleOp::Destroy(W1),
                IdleOp::Create {
                    id: W1,
                    timeout_ms: 5000,
                    generation: 2
                },
            ]
        );
        assert!(was_idle);
        // Eski kuşağın geç olayı yok sayılır; yeni kuşak baştan "boşta değil".
        assert!(!book.event(W1, 1, false));
        assert!(!book.event(W1, 2, false));
        assert!(book.event(W1, 2, true));
    }

    #[test]
    fn retime_pending_or_unknown_watch() {
        let mut book = IdleBook::default();
        assert!(book.retime(W1, 10).is_none());
        book.insert(W1, 1000, R, false);
        assert_eq!(book.retime(W1, 3000), Some((Vec::new(), false)));
        // Koltuk gelince yeni süreyle yaratılır.
        let (ops, _) = book.set_ready(true);
        assert_eq!(
            ops,
            vec![IdleOp::Create {
                id: W1,
                timeout_ms: 3000,
                generation: 1
            }]
        );
        // Boşta olmayan canlı izleme: resumed yok.
        let (_, was_idle) = book.retime(W1, 4000).unwrap();
        assert!(!was_idle);
    }

    #[test]
    fn inhibitor_kind_is_kept_per_watch() {
        let mut book = IdleBook::default();
        book.insert(W1, 1000, R, true);
        book.insert(W2, 1000, IdleInhibitors::Ignore, true);
        assert_eq!(book.inhibitors(W1), IdleInhibitors::Respect);
        assert_eq!(book.inhibitors(W2), IdleInhibitors::Ignore);
        // Yeniden kayıt türü korur.
        book.retime(W2, 2000);
        assert_eq!(book.inhibitors(W2), IdleInhibitors::Ignore);
        assert_eq!(IdleInhibitors::default(), IdleInhibitors::Respect);
    }
}
