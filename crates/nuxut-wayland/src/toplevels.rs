//! Pencere listesi (`ext-foreign-toplevel-list-v1`) ve Hyprland adres eşlemesi
//! (`hyprland_toplevel_mapping_manager_v1`, Hyprland'e özgü).
//!
//! Liste tembeldir: `cx.toplevels_start()` çağrılmadıkça global'e bağlanılmaz ve compositor
//! uygulamaya hiçbir pencere bildirmez. Her ext tutamacına opak bir [`ToplevelId`] verilir; pencere
//! yakalama (`capture.rs`) bu kimlikle kaynak yaratır.
//!
//! Hyprland'de eşleme yöneticisi varsa her yeni tutamaç için tek `get_window_for_toplevel`
//! gönderilir; `Added` olayı yanıttan (adres ya da `failed`) SONRA gelir, böylece tüketici adresi
//! ilk olayda görür. XML `protocols/hyprland-toplevel-mapping-v1.xml` (çevrimdışı yeniden kuruldu;
//! bkz. `docs/design/capture.md`).
//!
//! Başlık ve uygulama kimliği kullanıcı içeriğidir: loglanmaz, `ToplevelInfo`'nun Debug'ı
//! redakte (değişmez 9). Dış girdi sınırlı: metin [`MAX_TEXT_BYTES`], tutamaç [`MAX_TOPLEVELS`].

use std::collections::HashMap;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryHandler};
use wayland_client::backend::ObjectId;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};

/// `wayland-scanner` ile üretilen istemci bağlaması (eşleme protokolü).
#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    missing_debug_implementations,
    clippy::all
)]
pub(crate) mod mapping_protocol {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::ext::foreign_toplevel_list::v1::client::*;
    use wayland_protocols_wlr::foreign_toplevel::v1::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::ext::foreign_toplevel_list::v1::client::__interfaces::*;
        use wayland_protocols_wlr::foreign_toplevel::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/hyprland-toplevel-mapping-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/hyprland-toplevel-mapping-v1.xml");
}

use mapping_protocol::hyprland_toplevel_mapping_manager_v1::HyprlandToplevelMappingManagerV1;
use mapping_protocol::hyprland_toplevel_window_mapping_handle_v1::{
    self, HyprlandToplevelWindowMappingHandleV1,
};

/// Liste global'inin arayüz adı.
pub(crate) const LIST_INTERFACE: &str = "ext_foreign_toplevel_list_v1";
/// Hyprland eşleme yöneticisinin arayüz adı.
pub(crate) const MAPPING_INTERFACE: &str = "hyprland_toplevel_mapping_manager_v1";

/// Başlık/uygulama kimliği bu kadar bayta (karakter sınırında) kırpılır.
pub const MAX_TEXT_BYTES: usize = 4096;
/// En çok bu kadar pencere izlenir; fazlası hemen bırakılır (sayılır, uyarılır).
pub const MAX_TOPLEVELS: usize = 1024;

/// Bir pencerenin runtime kimliği (ext tutamacı başına). Liste durunca ya da pencere kapanınca
/// geçersizdir; yeniden kullanılmaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToplevelId(pub(crate) u64);

#[cfg(any(test, feature = "test-util"))]
impl ToplevelId {
    pub fn for_test(raw: u64) -> Self {
        Self(raw)
    }
}

/// Bir pencerenin bilinen özellikleri. Debug redakte (başlık ve uygulama kimliği kullanıcı
/// içeriğidir; yalnızca uzunlukları gösterilir).
#[derive(Clone, PartialEq, Eq)]
pub struct ToplevelInfo {
    pub id: ToplevelId,
    /// Uygulama kimliği (Hyprland'de pencere sınıfı); bilinmiyorsa boş.
    pub app_id: String,
    /// Pencere başlığı; bilinmiyorsa boş.
    pub title: String,
    /// Hyprland pencere adresi (`hyprctl clients` `address`, xdph pencere listesindeki tam
    /// adres). Eşleme protokolü yoksa ya da compositor eşleyemediyse `None`.
    pub hyprland_address: Option<u64>,
}

impl std::fmt::Debug for ToplevelInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToplevelInfo")
            .field("id", &self.id)
            .field("app_id_len", &self.app_id.len())
            .field("title_len", &self.title.len())
            .field("hyprland_address", &self.hyprland_address.is_some())
            .finish()
    }
}

/// `App::toplevel` olayı. Ayrıntı `cx.toplevel(id)` ile okunur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToplevelEvent {
    /// Yeni pencere (ilk `done` ve varsa adres yanıtından sonra).
    Added(ToplevelId),
    /// Başlık ya da uygulama kimliği değişti.
    Changed(ToplevelId),
    /// Pencere kapandı ya da liste durdu; kimlik artık geçersiz.
    Removed(ToplevelId),
}

/// `toplevels_start` hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ToplevelListError {
    /// Compositor `ext-foreign-toplevel-list-v1` sunmuyor.
    #[error("compositor ext-foreign-toplevel-list-v1 sunmuyor")]
    Unsupported,
}

/// `window_address(hi, lo)` → 64 bit adres.
pub(crate) fn address_from_halves(hi: u32, lo: u32) -> u64 {
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Dış metni `MAX_TEXT_BYTES`'a, karakter sınırında kırpar.
pub(crate) fn bounded_text(mut s: String) -> String {
    if s.len() > MAX_TEXT_BYTES {
        let mut end = MAX_TEXT_BYTES;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    s
}

/// Tutamaç başına durum.
#[derive(Debug)]
struct Entry {
    handle: ExtForeignToplevelHandleV1,
    info: ToplevelInfo,
    /// `done` ile onaylanmamış (bekleyen) değişiklikler.
    pending_title: Option<String>,
    pending_app_id: Option<String>,
    /// İlk `done` geldi mi.
    done_once: bool,
    /// `Added` uygulamaya verildi mi (adres yanıtı bekleniyorsa `false`).
    announced: bool,
    /// Bekleyen eşleme isteği.
    mapping: Option<HyprlandToplevelWindowMappingHandleV1>,
}

/// Context'teki pencere listesi durumu.
#[derive(Debug, Default)]
pub(crate) struct Toplevels {
    registry: Option<WlRegistry>,
    list_global: Option<(u32, u32)>,
    mapping_global: Option<(u32, u32)>,
    list: Option<ExtForeignToplevelListV1>,
    mapping: Option<HyprlandToplevelMappingManagerV1>,
    entries: HashMap<ObjectId, Entry>,
    by_id: HashMap<ToplevelId, ObjectId>,
    /// Sınır yüzünden bırakılan tutamaç sayısı (log için).
    dropped: usize,
}

impl Toplevels {
    /// Yakalama için: kimliğin canlı ext tutamacı.
    pub(crate) fn handle(&self, id: ToplevelId) -> Option<&ExtForeignToplevelHandleV1> {
        let oid = self.by_id.get(&id)?;
        self.entries.get(oid).map(|e| &e.handle)
    }
}

impl<A: App> Context<A> {
    /// Compositor `ext-foreign-toplevel-list-v1` sunuyor mu (bağlanmaz).
    pub fn toplevels_supported(&self) -> bool {
        self.toplevels.list_global.is_some()
    }

    /// Pencere listesini başlatır: compositor mevcut ve yeni pencereleri bildirir,
    /// `App::toplevel` olayları gelir. Zaten başladıysa no-op.
    pub fn toplevels_start(&mut self) -> Result<(), ToplevelListError> {
        if self.toplevels.list.is_some() {
            return Ok(());
        }
        let (Some(registry), Some((name, version))) =
            (self.toplevels.registry.as_ref(), self.toplevels.list_global)
        else {
            return Err(ToplevelListError::Unsupported);
        };
        let list = registry.bind::<ExtForeignToplevelListV1, _, State<A>>(
            name,
            version.min(1),
            &self.qh,
            ListData,
        );
        if let Some((name, version)) = self.toplevels.mapping_global {
            let mapping = registry.bind::<HyprlandToplevelMappingManagerV1, _, State<A>>(
                name,
                version.min(1),
                &self.qh,
                NoEvents,
            );
            self.toplevels.mapping = Some(mapping);
        }
        self.toplevels.list = Some(list);
        tracing::debug!(
            mapping = self.toplevels.mapping.is_some(),
            "pencere listesi başlatıldı"
        );
        Ok(())
    }

    /// Listeyi durdurur: tüm tutamaçlar bırakılır, duyurulmuş her pencere için `Removed` gelir.
    pub fn toplevels_stop(&mut self) {
        let Some(list) = self.toplevels.list.take() else {
            return;
        };
        list.stop();
        list.destroy();
        if let Some(mapping) = self.toplevels.mapping.take() {
            mapping.destroy();
        }
        self.toplevels_clear();
        tracing::debug!("pencere listesi durduruldu");
    }

    /// Duyurulmuş pencereler (sıra belirsiz).
    pub fn toplevels(&self) -> impl Iterator<Item = &ToplevelInfo> {
        self.toplevels
            .entries
            .values()
            .filter(|e| e.announced)
            .map(|e| &e.info)
    }

    /// Duyurulmuş bir pencerenin bilgisi.
    pub fn toplevel(&self, id: ToplevelId) -> Option<&ToplevelInfo> {
        let oid = self.toplevels.by_id.get(&id)?;
        self.toplevels
            .entries
            .get(oid)
            .filter(|e| e.announced)
            .map(|e| &e.info)
    }

    /// Hyprland adresinden pencere (xdph listesi, `hyprctl clients`).
    pub fn toplevel_by_hyprland_address(&self, address: u64) -> Option<ToplevelId> {
        self.toplevels
            .entries
            .values()
            .find(|e| e.announced && e.info.hyprland_address == Some(address))
            .map(|e| e.info.id)
    }

    fn toplevels_clear(&mut self) {
        for (_, entry) in self.toplevels.entries.drain() {
            if let Some(m) = entry.mapping {
                m.destroy();
            }
            entry.handle.destroy();
            if entry.announced {
                self.pending
                    .push(RuntimeEvent::Toplevel(ToplevelEvent::Removed(entry.info.id)));
            }
        }
        self.toplevels.by_id.clear();
    }

    fn toplevel_new(&mut self, handle: ExtForeignToplevelHandleV1) {
        if self.toplevels.entries.len() >= MAX_TOPLEVELS {
            handle.destroy();
            self.toplevels.dropped += 1;
            tracing::warn!(
                dropped = self.toplevels.dropped,
                "pencere sınırı aşıldı; tutamaç bırakıldı"
            );
            return;
        }
        let id = ToplevelId(self.next_id());
        let oid = handle.id();
        self.toplevels.by_id.insert(id, oid.clone());
        self.toplevels.entries.insert(
            oid,
            Entry {
                handle,
                info: ToplevelInfo {
                    id,
                    app_id: String::new(),
                    title: String::new(),
                    hyprland_address: None,
                },
                pending_title: None,
                pending_app_id: None,
                done_once: false,
                announced: false,
                mapping: None,
            },
        );
    }

    fn toplevel_handle_event(
        &mut self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
    ) {
        use ext_foreign_toplevel_handle_v1::Event;
        let oid = handle.id();
        let Some(entry) = self.toplevels.entries.get_mut(&oid) else {
            return;
        };
        match event {
            Event::Title { title } => entry.pending_title = Some(bounded_text(title)),
            Event::AppId { app_id } => entry.pending_app_id = Some(bounded_text(app_id)),
            Event::Identifier { .. } => {}
            Event::Done => {
                let mut changed = false;
                if let Some(t) = entry.pending_title.take()
                    && t != entry.info.title
                {
                    entry.info.title = t;
                    changed = true;
                }
                if let Some(a) = entry.pending_app_id.take()
                    && a != entry.info.app_id
                {
                    entry.info.app_id = a;
                    changed = true;
                }
                let id = entry.info.id;
                if !entry.done_once {
                    entry.done_once = true;
                    match self.toplevels.mapping.as_ref() {
                        Some(manager) => {
                            entry.mapping = Some(manager.get_window_for_toplevel(
                                handle,
                                &self.qh,
                                MappingData(oid.clone()),
                            ));
                        }
                        None => {
                            entry.announced = true;
                            self.pending
                                .push(RuntimeEvent::Toplevel(ToplevelEvent::Added(id)));
                        }
                    }
                } else if changed && entry.announced {
                    self.pending
                        .push(RuntimeEvent::Toplevel(ToplevelEvent::Changed(id)));
                }
            }
            Event::Closed => {
                if let Some(entry) = self.toplevels.entries.remove(&oid) {
                    if let Some(m) = entry.mapping {
                        m.destroy();
                    }
                    entry.handle.destroy();
                    self.toplevels.by_id.remove(&entry.info.id);
                    if entry.announced {
                        self.pending
                            .push(RuntimeEvent::Toplevel(ToplevelEvent::Removed(entry.info.id)));
                    }
                }
            }
            _ => {}
        }
    }

    fn toplevel_mapped(&mut self, oid: &ObjectId, address: Option<u64>) {
        let Some(entry) = self.toplevels.entries.get_mut(oid) else {
            return;
        };
        if let Some(m) = entry.mapping.take() {
            m.destroy();
        }
        entry.info.hyprland_address = address;
        if !entry.announced {
            entry.announced = true;
            self.pending
                .push(RuntimeEvent::Toplevel(ToplevelEvent::Added(entry.info.id)));
        }
    }

    fn toplevel_list_finished(&mut self) {
        // Compositor listeyi bitirdi (stop yanıtı ya da kendi kararı).
        if let Some(list) = self.toplevels.list.take() {
            list.destroy();
            if let Some(mapping) = self.toplevels.mapping.take() {
                mapping.destroy();
            }
            self.toplevels_clear();
            tracing::debug!("pencere listesi compositor tarafından bitirildi");
        }
    }

    /// Kapanış.
    pub(crate) fn toplevels_shutdown(&mut self) {
        self.toplevels_stop();
    }
}

/// Liste nesnesinin verisi.
#[derive(Debug)]
pub(crate) struct ListData;

/// Tutamaç nesnesinin verisi.
#[derive(Debug)]
pub(crate) struct HandleData;

/// Eşleme tutamacının verisi: hangi ext tutamacı için.
#[derive(Debug)]
pub(crate) struct MappingData(ObjectId);

impl<A: App> Dispatch2<ExtForeignToplevelListV1, State<A>> for ListData {
    event_created_child!(State<A>, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, HandleData)
    ]);

    fn event(
        &self,
        state: &mut State<A>,
        list: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        use ext_foreign_toplevel_list_v1::Event;
        let live = state.cx.toplevels.list.as_ref() == Some(list);
        match event {
            Event::Toplevel { toplevel } => {
                if live {
                    state.cx.toplevel_new(toplevel);
                } else {
                    toplevel.destroy();
                }
            }
            Event::Finished if live => state.cx.toplevel_list_finished(),
            _ => {}
        }
    }
}

impl<A: App> Dispatch2<ExtForeignToplevelHandleV1, State<A>> for HandleData {
    fn event(
        &self,
        state: &mut State<A>,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        state.cx.toplevel_handle_event(handle, event);
    }
}

impl<A: App> Dispatch2<HyprlandToplevelWindowMappingHandleV1, State<A>> for MappingData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &HyprlandToplevelWindowMappingHandleV1,
        event: hyprland_toplevel_window_mapping_handle_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        use hyprland_toplevel_window_mapping_handle_v1::Event;
        let address = match event {
            Event::WindowAddress {
                address_hi,
                address_lo,
            } => Some(address_from_halves(address_hi, address_lo)),
            Event::Failed => None,
        };
        state.cx.toplevel_mapped(&self.0, address);
    }
}

/// Eşleme yöneticisinin olayı yok.
impl<D> Dispatch2<HyprlandToplevelMappingManagerV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &HyprlandToplevelMappingManagerV1,
        _: <HyprlandToplevelMappingManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// Global'lerin kaydı (açılışta ve ilanda); bağlanma `toplevels_start`'ta.
#[derive(Debug)]
pub(crate) struct ToplevelsGlobal;

impl ToplevelsGlobal {
    pub(crate) fn record<A: App>(state: &mut State<A>, interface: &str, name: u32, version: u32) {
        let slot = match interface {
            LIST_INTERFACE => &mut state.cx.toplevels.list_global,
            MAPPING_INTERFACE => &mut state.cx.toplevels.mapping_global,
            _ => return,
        };
        if slot.is_none() {
            *slot = Some((name, version));
        }
        let registry = state.registry().registry().clone();
        state.cx.toplevels.registry.get_or_insert(registry);
    }
}

impl<A: App> RegistryHandler<State<A>> for ToplevelsGlobal {
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
        let t = &mut state.cx.toplevels;
        if t.mapping_global.is_some_and(|(n, _)| n == name) {
            t.mapping_global = None;
            if let Some(m) = t.mapping.take() {
                m.destroy();
            }
        }
        if t.list_global.is_some_and(|(n, _)| n == name) {
            t.list_global = None;
            tracing::info!("ext_foreign_toplevel_list_v1 geri çekildi");
            state.cx.toplevels_stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_halves_join() {
        assert_eq!(address_from_halves(0, 0), 0);
        assert_eq!(address_from_halves(0x5555, 0x8a3c_1f00), 0x5555_8a3c_1f00);
        assert_eq!(address_from_halves(u32::MAX, u32::MAX), u64::MAX);
    }

    #[test]
    fn text_is_bounded_on_char_boundary() {
        assert_eq!(bounded_text("kısa".into()), "kısa");
        let long = "ş".repeat(MAX_TEXT_BYTES); // 2 bayt/karakter
        let cut = bounded_text(long);
        assert!(cut.len() <= MAX_TEXT_BYTES);
        assert_eq!(cut.len(), MAX_TEXT_BYTES);
        let odd = format!("a{}", "ş".repeat(MAX_TEXT_BYTES));
        let cut = bounded_text(odd);
        assert_eq!(cut.len(), MAX_TEXT_BYTES - 1, "karakter ortasında kesilmez");
        assert!(cut.is_char_boundary(cut.len()));
    }

    #[test]
    fn info_debug_is_redacted() {
        let info = ToplevelInfo {
            id: ToplevelId(7),
            app_id: "gizli.uygulama".into(),
            title: "Banka hesabım — gizli başlık".into(),
            hyprland_address: Some(0xdead_beef),
        };
        let shown = format!("{info:?}");
        assert!(!shown.contains("gizli"), "{shown}");
        assert!(!shown.contains("Banka"), "{shown}");
        assert!(!shown.contains("deadbeef") && !shown.contains("3735928559"), "{shown}");
        assert!(shown.contains("title_len"), "{shown}");
    }
}
