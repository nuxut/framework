//! Sistem panosu: `wl_data_device` (Ctrl+C/V) ve `zwp_primary_selection_v1` (seç + orta tık).
//!
//! Yalnız metin (`nuxut_core::clipboard`: `text/plain;charset=utf-8`, `text/plain`). Okuma ve
//! yazma olay döngüsünde bloklamadan yapılır: boru O_NONBLOCK, calloop kaynağı hazır oldukça
//! okur/yazar; okuma boyut sınırı ve zaman aşımıyla, yazma zaman aşımıyla biter. Kendi
//! seçimimizi yapıştırmak boru kullanmaz (bellekten). İçerik hiçbir seviyede loglanmaz
//! (değişmez 9: yalnız tür, uzunluk, kimlik). Oturum kilidi istenmiş/kuruluyken pano
//! kullanılmaz: kopyalama reddedilir, yapıştırma `Locked` döner, süren okumalar iptal edilir.

use std::collections::HashMap;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::OwnedFd;
use std::sync::Arc;

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, Mode, PostAction, RegistrationToken};
use nuxut_core::clipboard::{
    self, COPY_WRITE_TIMEOUT, ClipboardText, CopyError, OFFERED_MIMES, PASTE_TIMEOUT, PasteError,
    Selection,
};
use smithay_client_toolkit::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use smithay_client_toolkit::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use smithay_client_toolkit::data_device_manager::data_source::{
    CopyPasteSource, DataSourceHandler,
};
use smithay_client_toolkit::data_device_manager::{DataDeviceManagerState, WritePipe};
use smithay_client_toolkit::primary_selection::PrimarySelectionManagerState;
use smithay_client_toolkit::primary_selection::device::{
    PrimarySelectionDevice, PrimarySelectionDeviceHandler,
};
use smithay_client_toolkit::primary_selection::selection::{
    PrimarySelectionSource, PrimarySelectionSourceHandler,
};
use wayland_client::protocol::wl_data_device::WlDataDevice;
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_source::WlDataSource;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1;
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1;

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::State;

/// `request_paste` isteğinin kimliği; sonuç `App::paste` ile gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PasteRequestId(pub(crate) u64);

/// Aynı anda başka istemcilere süren en çok yazma; fazlası hemen kapatılır (boru başına
/// metnin bir `Arc`'ı ve iki calloop kaynağı tutulur, sınırsız büyümesin).
const MAX_WRITES: usize = 8;
/// Tek okuma/yazma çağrısının parçası.
const CHUNK: usize = 64 * 1024;

/// Boruya yazılacak yük: pano metni ya da data-control seçiminin baytları (düşerken son
/// sahipse sıfırlanır).
pub(crate) enum Payload {
    Text(Arc<str>),
    Bytes(crate::data_control::SharedBytes),
}

impl Payload {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Text(t) => t.as_bytes(),
            Self::Bytes(b) => b.as_bytes(),
        }
    }
}

/// Sahip olduğumuz seçim: kaynak nesnesi (Drop'ta yok edilir) ve sunduğumuz metin.
struct Owned<S> {
    source: S,
    text: Arc<str>,
}

/// Süren bir boru aktarımı: G/Ç kaynağı ve zaman aşımı.
struct Transfer {
    io: RegistrationToken,
    timer: RegistrationToken,
}

/// Panonun Wayland tarafı (Context'te).
#[derive(Default)]
pub(crate) struct Clipboard {
    pub data_manager: Option<DataDeviceManagerState>,
    pub primary_manager: Option<PrimarySelectionManagerState>,
    data_device: Option<DataDevice>,
    primary_device: Option<PrimarySelectionDevice>,
    clipboard: Option<Owned<CopyPasteSource>>,
    primary: Option<Owned<PrimarySelectionSource>>,
    reads: HashMap<PasteRequestId, Transfer>,
    writes: HashMap<u64, Transfer>,
}

impl Clipboard {
    /// Klavyesi olan koltuk için aygıtlar (bir kez; ilk klavyeli koltuk).
    pub(crate) fn seat_added<A: App>(&mut self, qh: &QueueHandle<State<A>>, seat: &WlSeat) {
        if self.data_device.is_none()
            && let Some(manager) = &self.data_manager
        {
            self.data_device = Some(manager.get_data_device(qh, seat));
        }
        if self.primary_device.is_none()
            && let Some(manager) = &self.primary_manager
        {
            self.primary_device = Some(manager.get_selection_device(qh, seat));
        }
    }

    /// Koltuk klavyesini kaybetti: aygıtlar ve sahip olunan seçimler bırakılır.
    pub(crate) fn seat_removed(&mut self) {
        self.clipboard = None;
        self.primary = None;
        self.data_device = None;
        self.primary_device = None;
    }

    fn supported(&self, selection: Selection) -> bool {
        match selection {
            Selection::Clipboard => self.data_device.is_some(),
            Selection::Primary => self.primary_device.is_some(),
        }
    }

    fn owned_text(&self, selection: Selection) -> Option<&Arc<str>> {
        match selection {
            Selection::Clipboard => self.clipboard.as_ref().map(|o| &o.text),
            Selection::Primary => self.primary.as_ref().map(|o| &o.text),
        }
    }
}

impl<A: App> Context<A> {
    /// `text`'i seçime yazar (sahiplenir). Compositor seçimi ancak son girdi olayının (tuş ya
    /// da tıklama) serial'iyle kabul eder; bu yüzden kopyalama bir girdi işleyicisinden
    /// çağrılmalı. Başka bir istemci seçimi alınca metin bırakılır. Oturum kilidi
    /// istenmiş/kuruluysa reddedilir.
    pub fn set_clipboard(&mut self, selection: Selection, text: &str) -> Result<(), CopyError> {
        if self.lock.is_some() {
            return Err(CopyError::Locked);
        }
        if !self.clipboard.supported(selection) {
            return Err(CopyError::Unsupported);
        }
        let Some((_, serial)) = self.last_input.clone() else {
            return Err(CopyError::NoInput);
        };
        let text: Arc<str> = Arc::from(text);
        let cb = &mut self.clipboard;
        match selection {
            Selection::Clipboard => {
                let (Some(manager), Some(device)) = (&cb.data_manager, &cb.data_device) else {
                    return Err(CopyError::Unsupported);
                };
                // İşaret MIME'ı: data-control izleyicisi teklifi bizim sayar (`own`); istenirse
                // `serve_refusal` boruyu boş kapatır.
                let mimes = OFFERED_MIMES
                    .iter()
                    .copied()
                    .chain([crate::data_control::owner_mime()]);
                let source = manager.create_copy_paste_source(&self.qh, mimes);
                source.set_selection(device, serial);
                // Eskisi (varsa) burada düşer: kaynağı yok edilir.
                cb.clipboard = Some(Owned { source, text });
            }
            Selection::Primary => {
                let (Some(manager), Some(device)) = (&cb.primary_manager, &cb.primary_device)
                else {
                    return Err(CopyError::Unsupported);
                };
                let mimes = OFFERED_MIMES
                    .iter()
                    .copied()
                    .chain([crate::data_control::owner_mime()]);
                let source = manager.create_selection_source(&self.qh, mimes);
                source.set_selection(device, serial);
                cb.primary = Some(Owned { source, text });
            }
        }
        tracing::debug!(?selection, "pano seçimi sahiplenildi");
        Ok(())
    }

    /// Seçimdeki metni ister. Sonuç her istek için tam bir kez `App::paste` ile, bu çağrının
    /// içinden değil, gelir; okuma olay döngüsünü bloklamaz ve en geç `PASTE_TIMEOUT`'ta biter.
    pub fn request_paste(&mut self, selection: Selection) -> PasteRequestId {
        let id = PasteRequestId(self.next_id());
        if let Err(e) = self.start_paste(id, selection) {
            tracing::debug!(id = id.0, ?selection, error = ?e, "yapıştırma başlamadı");
            self.pending.push(RuntimeEvent::Paste(id, Err(e)));
        }
        id
    }

    fn start_paste(&mut self, id: PasteRequestId, selection: Selection) -> Result<(), PasteError> {
        if self.lock.is_some() {
            return Err(PasteError::Locked);
        }
        let cb = &self.clipboard;
        if !cb.supported(selection) {
            return Err(PasteError::Unsupported);
        }
        // Kendi seçimimiz: boru yok (compositor üzerinden kendimize yazıp okumak gereksiz).
        if let Some(text) = cb.owned_text(selection) {
            let text = ClipboardText::new(text.to_string());
            self.pending.push(RuntimeEvent::Paste(id, Ok(text)));
            return Ok(());
        }
        let pipe = match selection {
            Selection::Clipboard => {
                let offer = cb
                    .data_device
                    .as_ref()
                    .and_then(|d| d.data().selection_offer())
                    .ok_or(PasteError::Empty)?;
                let mime = offer
                    .with_mime_types(|m| clipboard::preferred_mime(m).map(str::to_owned))
                    .ok_or(PasteError::NotText)?;
                offer.receive(mime).map_err(|_| PasteError::Io)?
            }
            Selection::Primary => {
                let offer = cb
                    .primary_device
                    .as_ref()
                    .and_then(|d| d.data().selection_offer())
                    .ok_or(PasteError::Empty)?;
                let mime = offer
                    .with_mime_types(|m| clipboard::preferred_mime(m).map(str::to_owned))
                    .ok_or(PasteError::NotText)?;
                offer.receive(mime).map_err(|_| PasteError::Io)?
            }
        };
        let file = nonblocking(OwnedFd::from(pipe)).ok_or(PasteError::Io)?;
        self.start_read(id, file)?;
        tracing::debug!(id = id.0, ?selection, "yapıştırma okuması başladı");
        Ok(())
    }

    fn start_read(&mut self, id: PasteRequestId, file: File) -> Result<(), PasteError> {
        let mut buf: Vec<u8> = Vec::new();
        // Okuma parçası: ilk uyanmada bir kez ayrılır, sonraki uyanmalarda yeniden kullanılır.
        let mut chunk: Vec<u8> = Vec::new();
        let io = self
            .handle
            .insert_source(
                Generic::new(file, Interest::READ, Mode::Level),
                move |_, file, state: &mut State<A>| {
                    if chunk.is_empty() {
                        chunk.resize(CHUNK, 0);
                    }
                    let result = loop {
                        match (&**file).read(&mut chunk) {
                            Ok(0) => break clipboard::decode(std::mem::take(&mut buf)),
                            Ok(n) if buf.len() + n > clipboard::MAX_PASTE_BYTES => {
                                break Err(PasteError::TooLarge);
                            }
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                                return Ok(PostAction::Continue);
                            }
                            Err(e) if e.kind() == ErrorKind::Interrupted => {}
                            Err(_) => break Err(PasteError::Io),
                        }
                    };
                    state.cx.finish_read(id, result, false);
                    Ok(PostAction::Remove)
                },
            )
            .map_err(|_| PasteError::Io)?;
        let timer = self
            .handle
            .insert_source(
                Timer::from_duration(PASTE_TIMEOUT),
                move |_, _, state: &mut State<A>| {
                    state.cx.finish_read(id, Err(PasteError::Timeout), true);
                    TimeoutAction::Drop
                },
            )
            .map_err(|_| {
                self.handle.remove(io);
                PasteError::Io
            })?;
        self.clipboard.reads.insert(id, Transfer { io, timer });
        Ok(())
    }

    /// Okumayı bitirir ve sonucu kuyruğa alır. `from_timer`: çağıran zaman aşımı (G/Ç kaynağı
    /// buradan kaldırılır); değilse çağıran G/Ç kaynağıdır (kendini `Remove` ile kaldırır).
    fn finish_read(
        &mut self,
        id: PasteRequestId,
        result: Result<ClipboardText, PasteError>,
        from_timer: bool,
    ) {
        let Some(transfer) = self.clipboard.reads.remove(&id) else {
            return;
        };
        if from_timer {
            self.handle.remove(transfer.io);
        } else {
            self.handle.remove(transfer.timer);
        }
        match &result {
            Ok(text) => tracing::debug!(id = id.0, bytes = text.len(), "yapıştırma okundu"),
            Err(e) => tracing::debug!(id = id.0, error = ?e, "yapıştırma başarısız"),
        }
        self.pending.push(RuntimeEvent::Paste(id, result));
    }

    /// Oturum kilidi istendi: süren okumalar `Locked` ile biter (kilit sırasında içerik
    /// uygulamaya varmaz).
    pub(crate) fn cancel_pastes_for_lock(&mut self) {
        let ids: Vec<PasteRequestId> = self.clipboard.reads.keys().copied().collect();
        for id in ids {
            if let Some(transfer) = self.clipboard.reads.remove(&id) {
                self.handle.remove(transfer.io);
                self.handle.remove(transfer.timer);
                self.pending
                    .push(RuntimeEvent::Paste(id, Err(PasteError::Locked)));
            }
        }
    }

    /// Başka bir istemci seçimimizi istedi: metni boruya bloklamadan yazar.
    fn serve(&mut self, text: Option<Arc<str>>, mime: &str, pipe: WritePipe) {
        let Some(text) = text else {
            return;
        };
        // Reddedilen istekte boru burada düşer (kapanır); okuyan taraf boş veri görür.
        if let Some(reason) = serve_refusal(self.lock.is_some(), mime, self.clipboard.writes.len())
        {
            tracing::debug!(reason, "pano isteği reddedildi; boru kapatıldı");
            return;
        }
        self.serve_payload(Payload::Text(text), OwnedFd::from(pipe));
    }

    /// Yükü boruya bloklamadan yazar (`COPY_WRITE_TIMEOUT` ile sınırlı; aynı anda en çok
    /// `MAX_WRITES`, fazlası hemen kapatılır). Pano ve data-control kaynakları paylaşır.
    pub(crate) fn serve_payload(&mut self, payload: Payload, fd: OwnedFd) {
        if self.clipboard.writes.len() >= MAX_WRITES {
            tracing::debug!("çok fazla eşzamanlı pano yazması; boru kapatıldı");
            return;
        }
        let Some(file) = nonblocking(fd) else {
            return;
        };
        let key = self.next_id();
        let mut offset = 0usize;
        let io = self.handle.insert_source(
            Generic::new(file, Interest::WRITE, Mode::Level),
            move |_, file, state: &mut State<A>| {
                let bytes = payload.as_bytes();
                while offset < bytes.len() {
                    let end = bytes.len().min(offset + CHUNK);
                    match (&**file).write(&bytes[offset..end]) {
                        Ok(n) => offset += n,
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            return Ok(PostAction::Continue);
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => {}
                        // Okuyan taraf kapattı (EPIPE) ya da başka hata: bırak.
                        Err(_) => break,
                    }
                }
                state.cx.finish_write(key, false);
                Ok(PostAction::Remove)
            },
        );
        let Ok(io) = io else {
            return;
        };
        let timer = self.handle.insert_source(
            Timer::from_duration(COPY_WRITE_TIMEOUT),
            move |_, _, state: &mut State<A>| {
                tracing::debug!("pano yazması zaman aşımı; boru kapatıldı");
                state.cx.finish_write(key, true);
                TimeoutAction::Drop
            },
        );
        match timer {
            Ok(timer) => {
                self.clipboard.writes.insert(key, Transfer { io, timer });
            }
            Err(_) => self.handle.remove(io),
        }
    }

    fn finish_write(&mut self, key: u64, from_timer: bool) {
        if let Some(transfer) = self.clipboard.writes.remove(&key) {
            if from_timer {
                self.handle.remove(transfer.io);
            } else {
                self.handle.remove(transfer.timer);
            }
        }
    }
}

/// Başka istemcinin seçimimizi okuma isteği neden reddedilir (`None`: yazılır). Kilit
/// istenmiş/kuruluyken hiç yazılmaz (savunma derinliği: kilitte odak zaten bizdedir).
fn serve_refusal(locked: bool, mime: &str, writes: usize) -> Option<&'static str> {
    if locked {
        Some("kilit")
    } else if !OFFERED_MIMES.contains(&mime) {
        Some("sunulmayan tür")
    } else if writes >= MAX_WRITES {
        Some("çok fazla eşzamanlı yazma")
    } else {
        None
    }
}

/// Boruyu O_NONBLOCK yapar; olay döngüsü hiçbir zaman okuma/yazmada beklemez.
fn nonblocking(fd: OwnedFd) -> Option<File> {
    match rustix::io::ioctl_fionbio(&fd, true) {
        Ok(()) => Some(File::from(fd)),
        Err(e) => {
            tracing::debug!("pano borusu bloklamasız yapılamadı: {e}");
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------
// sctk işleyicileri. Sürükle-bırak desteklenmiyor: DnD olayları yok sayılır (teklif kabul
// edilmez; sctk teklifleri leave'de yok eder).
// ---------------------------------------------------------------------------------------------

impl<A: App> DataDeviceHandler for State<A> {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }

    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}

    /// Yeni teklif sctk'de saklanır; `request_paste` onu okur.
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}

    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl<A: App> DataOfferHandler for State<A> {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }

    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl<A: App> DataSourceHandler for State<A> {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }

    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        pipe: WritePipe,
    ) {
        let text = self
            .cx
            .clipboard
            .clipboard
            .as_ref()
            .filter(|o| o.source.inner() == source)
            .map(|o| o.text.clone());
        self.cx.serve(text, &mime, pipe);
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        let cb = &mut self.cx.clipboard;
        if cb
            .clipboard
            .as_ref()
            .is_some_and(|o| o.source.inner() == source)
        {
            tracing::debug!("pano seçimi başka istemciye geçti");
            cb.clipboard = None;
        }
    }

    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}

    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

impl<A: App> PrimarySelectionDeviceHandler for State<A> {
    fn selection(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &ZwpPrimarySelectionDeviceV1,
    ) {
    }
}

impl<A: App> PrimarySelectionSourceHandler for State<A> {
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
        mime: String,
        pipe: WritePipe,
    ) {
        let text = self
            .cx
            .clipboard
            .primary
            .as_ref()
            .filter(|o| o.source.inner() == source)
            .map(|o| o.text.clone());
        self.cx.serve(text, &mime, pipe);
    }

    fn cancelled(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &ZwpPrimarySelectionSourceV1,
    ) {
        let cb = &mut self.cx.clipboard;
        if cb
            .primary
            .as_ref()
            .is_some_and(|o| o.source.inner() == source)
        {
            tracing::debug!("birincil seçim başka istemciye geçti");
            cb.primary = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serve_refuses_when_locked_unoffered_or_busy() {
        assert_eq!(serve_refusal(false, clipboard::MIME_TEXT_UTF8, 0), None);
        assert_eq!(
            serve_refusal(false, clipboard::MIME_TEXT, MAX_WRITES - 1),
            None
        );
        assert!(serve_refusal(true, clipboard::MIME_TEXT_UTF8, 0).is_some());
        assert!(serve_refusal(false, "image/png", 0).is_some());
        assert!(serve_refusal(false, clipboard::MIME_TEXT_UTF8, MAX_WRITES).is_some());
    }
}
