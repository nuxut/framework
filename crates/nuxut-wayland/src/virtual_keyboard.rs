//! Sanal klavye (`zwp-virtual-keyboard-unstable-v1`): ekran klavyesinin odaktaki pencereye tuş
//! göndermesi. Sanal klavye HER pencereye girdi verebilir; bu yüzden yalnızca açıkça çağıran
//! kullanır, runtime hiçbir zaman kendiliğinden kurmaz ve tuş göndermez.
//!
//! Sözleşme (runtime uygular):
//! - **Ayrı bağlantı.** Sanal klavye uygulamanın ana Wayland bağlantısında DEĞİL, ilk
//!   `virtual_keyboard_set_keymap`'te açılan ikinci bir bağlantıdadır. Gerekçe: protokoldeki
//!   bütün hatalar ölümcül protokol hatasıdır (yetkisiz istemciye `unauthorized`, keymap'siz
//!   tuşa `no_keymap`; wlroots derlenemeyen keymap'e `no_memory` verir) ve bağlantıyı koparır.
//!   Ana bağlantıda olsalar kabuk tümden düşerdi; burada yalnızca bu bağlantı ölür:
//!   `unauthorized` → `virtual_keyboard_supported() == false`, tek uyarı, oturum boyunca kalıcı.
//!   Ana bağlantıda yönetici global'i yalnızca *kaydedilir*, hiç bağlanmaz.
//! - `set_keymap` eşzamanlıdır: bağlantı + kayıt turu + nesne + keymap + bir gidiş-dönüş
//!   (yerel sokette milisaniyenin altında). Compositor'ın reddi bu yüzden aynı çağrıda
//!   `Err(Unauthorized)` olarak döner. Keymap önce yerelde xkbcommon ile derlenir; derlenmeyen
//!   metin gönderilmez. Aynı metin ikinci kez gönderilmez (no-op).
//! - Keymap değişince önce basılı tuşlar bırakılır ve değiştiriciler sıfırlanır (eski keymap'in
//!   maskeleri yenisinde başka anlama gelebilir), sonra yeni keymap aynı nesneye gönderilir.
//! - Basılı tuşlar runtime'da izlenir: basılıya ikinci basma ve basılı olmayanı bırakma
//!   gönderilmez; en çok `MAX_PRESSED` tuş birlikte basılı olabilir.
//! - Tuş tekrarı yapılmaz. Basılı tutulan tuşu hedef istemci compositor'ın `repeat_info`'suyla
//!   kendisi tekrarlar; uygulama kendi tekrarını basma/bırakma çiftleriyle yapar.
//! - Değiştiriciler (wlroots dahil) tuşlardan türetilmez: uygulama `virtual_keyboard_modifiers`
//!   ile kendi keymap'ine göre gönderir.
//! - `virtual_keyboard_release`: basılıları bırakır, değiştiricileri sıfırlar, nesneyi yok eder
//!   ve bağlantıyı kapatır (gizliyken sıfır kaynak). `Context` düşerken de aynısı yapılır.
//!   Süreç çökerse bağlantı kapanır; basılı tuşları compositor bırakır (wlroots
//!   `wlr_keyboard_finish`).
//! - Koltuk (ikinci bağlantıda ilk `wl_seat`) giderse nesne yok edilir, basılı tuş kaydı
//!   unutulur; koltuk gelince nesne yeniden yaratılır ve son keymap yeniden gönderilir.
//! - Loglarda tuş kodu yok (yazılan metni ele verir); yalnızca sayı ve hata türü.

use std::io::Write as _;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Instant;

use calloop::generic::Generic;
use calloop::{Interest, Mode, PostAction, RegistrationToken};
use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};
use smithay_client_toolkit::reexports::protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use smithay_client_toolkit::registry::RegistryHandler;
use wayland_client::backend::WaylandError;
use wayland_client::backend::protocol::ProtocolError;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, DispatchError, EventQueue, Proxy, QueueHandle};
use xkbcommon::xkb;

use crate::App;
use crate::context::Context;
use crate::state::State;

/// Yönetici global'inin arayüz adı.
pub(crate) const MANAGER_INTERFACE: &str = "zwp_virtual_keyboard_manager_v1";
/// Yöneticinin `unauthorized` hata kodu.
const ERROR_UNAUTHORIZED: u32 = 0;
/// `wl_keyboard.keymap_format.xkb_v1`.
const KEYMAP_FORMAT_XKB_V1: u32 = 1;
/// En büyük evdev kodu (`KEY_MAX`, linux/input-event-codes.h).
pub const KEY_MAX: u32 = 0x2ff;
/// Birlikte basılı tutulabilecek en çok tuş.
pub const MAX_PRESSED: usize = 16;
/// Keymap metninin üst sınırı (compositor keymap'leri ~50–100 KB).
pub const MAX_KEYMAP_LEN: usize = 1 << 20;

/// Sanal klavye hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualKeyboardError {
    /// Compositor protokolü sunmuyor ya da ikinci bağlantı kurulamadı / protokol hatasıyla
    /// koptu (kalıcı; `virtual_keyboard_supported() == false`).
    Unsupported,
    /// Compositor istemciyi yetkisiz saydı (`unauthorized`; kalıcı).
    Unauthorized,
    /// Keymap boş, `MAX_KEYMAP_LEN`'den büyük, içinde NUL var ya da xkbcommon derleyemedi.
    InvalidKeymap,
    /// Önce `virtual_keyboard_set_keymap` gerekir (ya da `release` sonrası yeniden).
    NoKeymap,
    /// Koltuk yok; nesne koltuk gelince kurulur.
    NoSeat,
    /// evdev kodu `KEY_MAX`'ı aşıyor.
    InvalidKey,
    /// `MAX_PRESSED` tuş zaten basılı.
    TooManyKeys,
    /// Bağlantı koptu (G/Ç); sonraki `set_keymap` yeniden kurar.
    Disconnected,
}

impl std::fmt::Display for VirtualKeyboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "compositor sanal klavye sunmuyor",
            Self::Unauthorized => "compositor sanal klavyeye yetki vermedi",
            Self::InvalidKeymap => "geçersiz keymap",
            Self::NoKeymap => "sanal klavyenin keymap'i yok",
            Self::NoSeat => "koltuk yok",
            Self::InvalidKey => "geçersiz evdev kodu",
            Self::TooManyKeys => "çok fazla basılı tuş",
            Self::Disconnected => "sanal klavye bağlantısı koptu",
        })
    }
}

impl std::error::Error for VirtualKeyboardError {}

/// Saf durum: basılı tuşlar ve son gönderilen değiştiriciler. Wayland'e dokunmaz (birim testli).
/// `Debug` tuş kodlarını yazmaz (yazılan metni ele verir).
#[derive(Default, PartialEq, Eq)]
pub(crate) struct KeyBook {
    pressed: Vec<u32>,
    /// depressed, latched, locked, group.
    mods: [u32; 4],
}

impl std::fmt::Debug for KeyBook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyBook")
            .field("pressed", &self.pressed.len())
            .field("mods_set", &(self.mods != [0; 4]))
            .finish()
    }
}

impl KeyBook {
    /// Tuş durumunu kaydeder; `Ok(true)`: compositor'a gönderilmeli.
    pub(crate) fn key(&mut self, code: u32, pressed: bool) -> Result<bool, VirtualKeyboardError> {
        if code > KEY_MAX {
            return Err(VirtualKeyboardError::InvalidKey);
        }
        let at = self.pressed.iter().position(|&c| c == code);
        match (pressed, at) {
            (true, Some(_)) | (false, None) => Ok(false),
            (true, None) if self.pressed.len() >= MAX_PRESSED => {
                Err(VirtualKeyboardError::TooManyKeys)
            }
            (true, None) => {
                self.pressed.push(code);
                Ok(true)
            }
            (false, Some(i)) => {
                self.pressed.remove(i);
                Ok(true)
            }
        }
    }

    /// Değiştiricileri kaydeder; `true`: değişti, gönderilmeli.
    pub(crate) fn modifiers(&mut self, mods: [u32; 4]) -> bool {
        std::mem::replace(&mut self.mods, mods) != mods
    }

    /// Hepsini bırakır: bırakılacak tuşlar (basılma sırasıyla) ve değiştiriciler sıfırlanmalı mı.
    pub(crate) fn drain(&mut self) -> (Vec<u32>, bool) {
        let mods = std::mem::take(&mut self.mods) != [0; 4];
        (std::mem::take(&mut self.pressed), mods)
    }

    /// Aygıt gitti (compositor bıraktı): kayıt adımsız silinir.
    pub(crate) fn forget(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn pressed_count(&self) -> usize {
        self.pressed.len()
    }
}

/// Keymap metnini doğrular ve kablodaki biçime çevirir (sondaki NUL'lar atılır).
pub(crate) fn validate_keymap(keymap: &str) -> Result<&str, VirtualKeyboardError> {
    let text = keymap.trim_end_matches('\0');
    if text.is_empty() || text.len() > MAX_KEYMAP_LEN || text.contains('\0') {
        return Err(VirtualKeyboardError::InvalidKeymap);
    }
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    xkb::Keymap::new_from_string(
        &context,
        text.to_owned(),
        xkb::KEYMAP_FORMAT_TEXT_V1,
        xkb::KEYMAP_COMPILE_NO_FLAGS,
    )
    .map(|_| text)
    .ok_or(VirtualKeyboardError::InvalidKeymap)
}

/// Keymap'i mühürlü memfd'ye yazar (sonunda NUL; dosya sisteminde iz yok, compositor yalnız
/// okuyabilir). Dönen boyut NUL dahil.
pub(crate) fn keymap_memfd(text: &str) -> std::io::Result<(OwnedFd, u32)> {
    let size = u32::try_from(text.len() + 1)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let fd = memfd_create(
        "nuxut-virtual-keymap",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?;
    let mut file = std::fs::File::from(fd);
    file.write_all(text.as_bytes())?;
    file.write_all(&[0])?;
    fcntl_add_seals(
        &file,
        SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
    )?;
    Ok((file.into(), size))
}

/// Protokol hatasının türü: yöneticinin `unauthorized`'ı ayrı, gerisi desteklenmiyor.
pub(crate) fn classify(error: &ProtocolError) -> VirtualKeyboardError {
    if error.object_interface == MANAGER_INTERFACE && error.code == ERROR_UNAUTHORIZED {
        VirtualKeyboardError::Unauthorized
    } else {
        VirtualKeyboardError::Unsupported
    }
}

fn wayland_error(e: &WaylandError) -> VirtualKeyboardError {
    match e {
        WaylandError::Protocol(p) => classify(p),
        WaylandError::Io(_) => VirtualKeyboardError::Disconnected,
    }
}

fn dispatch_error(e: &DispatchError) -> VirtualKeyboardError {
    match e {
        DispatchError::Backend(w) => wayland_error(w),
        DispatchError::BadMessage { .. } => VirtualKeyboardError::Unsupported,
    }
}

/// İkinci bağlantının olay verisi: koltuk ve yönetici global'lerinin izlenmesi.
#[derive(Debug, Default)]
pub(crate) struct VkClient {
    seat: Option<(WlSeat, u32)>,
    manager_name: u32,
    manager_lost: bool,
    /// Koltuk değişti: nesne yeniden kurulmalı.
    seat_dirty: bool,
}

impl wayland_client::Dispatch<WlRegistry, GlobalListContents> for VkClient {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == WlSeat::interface().name && state.seat.is_none() => {
                state.seat = Some((registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ()), name));
                state.seat_dirty = true;
            }
            wl_registry::Event::GlobalRemove { name } => {
                if state.seat.as_ref().is_some_and(|(_, n)| *n == name) {
                    if let Some((seat, _)) = state.seat.take()
                        && seat.version() >= 5
                    {
                        seat.release();
                    }
                    state.seat_dirty = true;
                }
                if name == state.manager_name {
                    state.manager_lost = true;
                }
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(VkClient: ignore WlSeat);
wayland_client::delegate_noop!(VkClient: ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(VkClient: ZwpVirtualKeyboardV1);

/// Açık ikinci bağlantı ve (koltuk varken) sanal klavye nesnesi.
struct Session {
    conn: Connection,
    queue: EventQueue<VkClient>,
    client: VkClient,
    manager: ZwpVirtualKeyboardManagerV1,
    keyboard: Option<ZwpVirtualKeyboardV1>,
    /// `key.time` saati (tanımsız taban, tek nesne için ortak; milisaniye, sarar).
    epoch: Instant,
}

impl Session {
    fn open() -> Result<Self, VirtualKeyboardError> {
        let conn = Connection::connect_to_env().map_err(|e| {
            tracing::warn!("sanal klavye bağlantısı kurulamadı: {e}");
            VirtualKeyboardError::Disconnected
        })?;
        let (globals, queue) = registry_queue_init::<VkClient>(&conn).map_err(|e| {
            tracing::warn!("sanal klavye kayıt turu başarısız: {e}");
            VirtualKeyboardError::Disconnected
        })?;
        let qh = queue.handle();
        let manager = globals
            .bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
            .map_err(|_| VirtualKeyboardError::Unsupported)?;
        let mut client = VkClient::default();
        let (seat_global, manager_name) = globals.contents().with_list(|list| {
            let seat = list
                .iter()
                .find(|g| g.interface == WlSeat::interface().name)
                .map(|g| (g.name, g.version));
            let manager = list
                .iter()
                .find(|g| g.interface == MANAGER_INTERFACE)
                .map_or(0, |g| g.name);
            (seat, manager)
        });
        client.manager_name = manager_name;
        if let Some((name, version)) = seat_global {
            let seat: WlSeat = globals.registry().bind(name, version.min(5), &qh, ());
            client.seat = Some((seat, name));
        }
        Ok(Self {
            conn,
            queue,
            client,
            manager,
            keyboard: None,
            epoch: Instant::now(),
        })
    }

    fn time(&self) -> u32 {
        // Protokol: milisaniye, tanımsız taban; sarması beklenir.
        self.epoch.elapsed().as_millis() as u32
    }

    /// Koltuk varsa nesneyi yaratır ve keymap'i gönderir.
    fn create_keyboard(&mut self, keymap: &(OwnedFd, u32)) {
        let Some((seat, _)) = self.client.seat.as_ref() else {
            return;
        };
        let keyboard = self
            .manager
            .create_virtual_keyboard(seat, &self.queue.handle(), ());
        keyboard.keymap(KEYMAP_FORMAT_XKB_V1, keymap.0.as_fd(), keymap.1);
        self.keyboard = Some(keyboard);
    }

    fn destroy_keyboard(&mut self) {
        if let Some(keyboard) = self.keyboard.take() {
            keyboard.destroy();
        }
    }

    /// Basılıları bırakır, değiştiricileri sıfırlar (nesne varsa).
    fn release_all(&self, book: &mut KeyBook) {
        let (keys, mods) = book.drain();
        let Some(keyboard) = self.keyboard.as_ref() else {
            return;
        };
        let time = self.time();
        for code in keys {
            keyboard.key(time, code, 0);
        }
        if mods {
            keyboard.modifiers(0, 0, 0, 0);
        }
    }

    fn flush(&self) -> Result<(), VirtualKeyboardError> {
        match self.conn.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(self.error_after(&e)),
        }
    }

    /// Bağlantı hatasının asıl nedeni (G/Ç hatası protokol hatasından sonra da gelebilir).
    fn error_after(&self, e: &WaylandError) -> VirtualKeyboardError {
        match self.conn.protocol_error() {
            Some(p) => classify(&p),
            None => wayland_error(e),
        }
    }

    fn roundtrip(&mut self) -> Result<(), VirtualKeyboardError> {
        self.queue
            .roundtrip(&mut self.client)
            .map(drop)
            .map_err(|e| match self.conn.protocol_error() {
                Some(p) => classify(&p),
                None => dispatch_error(&e),
            })
    }

    /// Soket okunabilir: olayları okur ve işler.
    fn pump(&mut self) -> Result<(), VirtualKeyboardError> {
        if let Some(guard) = self.queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(self.error_after(&e)),
            }
        }
        self.queue
            .dispatch_pending(&mut self.client)
            .map(drop)
            .map_err(|e| match self.conn.protocol_error() {
                Some(p) => classify(&p),
                None => dispatch_error(&e),
            })
    }
}

/// `Context`'teki sanal klavye durumu.
#[derive(Default)]
pub(crate) struct VirtualKeyboard {
    /// Ana bağlantının kayıt defterinde yönetici global'inin adı (bağlanmaz, yalnız kayıt).
    pub global: Option<u32>,
    /// Kalıcı hata (`Unauthorized`/`Unsupported`): oturum boyunca desteklenmiyor.
    blocked: Option<VirtualKeyboardError>,
    session: Option<Session>,
    /// İkinci bağlantının olay kaynağı.
    token: Option<RegistrationToken>,
    /// Son gönderilen keymap: metin (aynısı tekrar gönderilmez) ve mühürlü memfd (koltuk
    /// yeniden gelince tekrar gönderilir).
    keymap: Option<(String, (OwnedFd, u32))>,
    book: KeyBook,
    /// Compositor'ın şu anki keymap'i (`Context::keyboard_keymap`).
    pub compositor_keymap: Option<String>,
}

impl std::fmt::Debug for VirtualKeyboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VirtualKeyboard")
            .field("global", &self.global.is_some())
            .field("blocked", &self.blocked)
            .field("session", &self.session.is_some())
            .field("keymap", &self.keymap.is_some())
            .field("book", &self.book)
            .finish_non_exhaustive()
    }
}

impl VirtualKeyboard {
    /// Basılıları bırakır, nesneyi yok eder, bağlantıyı kapatır. Olay kaynağı çağıranın işi.
    fn teardown(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.release_all(&mut self.book);
            session.destroy_keyboard();
            if let Err(e) = session.flush() {
                tracing::debug!(%e, "sanal klavye kapanırken gönderilemedi");
            }
        }
        self.book.forget();
        self.keymap = None;
    }
}

impl Drop for VirtualKeyboard {
    fn drop(&mut self) {
        // Basılı tuş compositor'da takılı kalmasın.
        self.teardown();
    }
}

impl<A: App> Context<A> {
    /// Compositor sanal klavye sunuyor mu (`zwp_virtual_keyboard_manager_v1` ilan edilmiş ve
    /// compositor reddetmemiş). Global sonradan ilan edilebilir/geri çekilebilir.
    pub fn virtual_keyboard_supported(&self) -> bool {
        self.vkbd.global.is_some() && self.vkbd.blocked.is_none()
    }

    /// Compositor'ın şu anki keymap'i (XKB_V1 metni; `wl_keyboard.keymap`). Ekran klavyesi
    /// bunu `virtual_keyboard_set_keymap`'e verir ki tuş kodları kullanıcının düzeniyle aynı
    /// karakteri üretsin. Değişince `App::keyboard_layout_changed` gelir (düzen adları
    /// değiştiyse). Koltukta klavye yoksa `None`.
    pub fn keyboard_keymap(&self) -> Option<&str> {
        self.vkbd.compositor_keymap.as_deref()
    }

    /// Sanal klavyeyi kurar (ilk çağrıda ikinci bağlantıyı açar) ve keymap'i gönderir; düzen
    /// değişince yeniden çağrılır. Aynı metin yeniden gönderilmez. Değişen keymap'ten önce
    /// basılı tuşlar bırakılır ve değiştiriciler sıfırlanır. Eşzamanlıdır: compositor'ın
    /// reddi (`Unauthorized`) bu çağrıda döner. Koltuk yoksa `Ok` döner, nesne koltuk gelince
    /// kurulur (`virtual_keyboard_key` o zamana dek `NoSeat`).
    pub fn virtual_keyboard_set_keymap(&mut self, keymap: &str) -> Result<(), VirtualKeyboardError> {
        if let Some(e) = self.vkbd.blocked {
            return Err(e);
        }
        if self.vkbd.global.is_none() {
            return Err(VirtualKeyboardError::Unsupported);
        }
        let text = validate_keymap(keymap)?;
        if self.vkbd.session.is_some()
            && self.vkbd.keymap.as_ref().is_some_and(|(t, _)| t == text)
        {
            return Ok(());
        }
        let blob = keymap_memfd(text).map_err(|e| {
            tracing::warn!("keymap memfd'ye yazılamadı: {e}");
            VirtualKeyboardError::InvalidKeymap
        })?;
        if self.vkbd.session.is_none() {
            let session = match Session::open() {
                Ok(s) => s,
                Err(e) => {
                    self.vkbd_fail(e, false);
                    return Err(e);
                }
            };
            self.vkbd.session = Some(session);
            self.vkbd_install_source();
        }
        let vkbd = &mut self.vkbd;
        let session = vkbd.session.as_mut().expect("oturum yukarıda açıldı");
        match session.keyboard.as_ref() {
            Some(keyboard) => {
                session.release_all(&mut vkbd.book);
                keyboard.keymap(KEYMAP_FORMAT_XKB_V1, blob.0.as_fd(), blob.1);
            }
            None => session.create_keyboard(&blob),
        }
        let has_keyboard = session.keyboard.is_some();
        if let Err(e) = session.roundtrip() {
            self.vkbd_fail(e, false);
            return Err(e);
        }
        self.vkbd.keymap = Some((text.to_owned(), blob));
        // Gidiş-dönüşte gelen koltuk değişikliği.
        self.vkbd_sync_seat();
        tracing::debug!(len = text.len(), seat = has_keyboard, "sanal klavye keymap'i gönderildi");
        Ok(())
    }

    /// Tuş basma/bırakma (evdev kodu; ör. `KEY_A` = 30). Basılıya basma ve basılı olmayanı
    /// bırakma gönderilmez. Runtime tekrar üretmez.
    pub fn virtual_keyboard_key(
        &mut self,
        evdev_code: u32,
        pressed: bool,
    ) -> Result<(), VirtualKeyboardError> {
        let session = self.vkbd_ready()?;
        let time = session.time();
        if !self.vkbd.book.key(evdev_code, pressed)? {
            return Ok(());
        }
        let session = self.vkbd.session.as_ref().expect("vkbd_ready doğruladı");
        let keyboard = session.keyboard.as_ref().expect("vkbd_ready doğruladı");
        keyboard.key(time, evdev_code, u32::from(pressed));
        self.vkbd_flush()
    }

    /// Değiştirici ve grup durumu (`xkb_state_serialize_mods` maskeleri, uygulamanın keymap'ine
    /// göre). Değişmediyse gönderilmez.
    pub fn virtual_keyboard_modifiers(
        &mut self,
        depressed: u32,
        latched: u32,
        locked: u32,
        group: u32,
    ) -> Result<(), VirtualKeyboardError> {
        self.vkbd_ready()?;
        if !self.vkbd.book.modifiers([depressed, latched, locked, group]) {
            return Ok(());
        }
        let session = self.vkbd.session.as_ref().expect("vkbd_ready doğruladı");
        let keyboard = session.keyboard.as_ref().expect("vkbd_ready doğruladı");
        keyboard.modifiers(depressed, latched, locked, group);
        self.vkbd_flush()
    }

    /// Basılı tuşları bırakır, değiştiricileri sıfırlar, nesneyi yok eder ve bağlantıyı
    /// kapatır (klavye gizlenince). Sonrasında yeniden `virtual_keyboard_set_keymap` gerekir.
    /// Kurulmamışken etkisizdir.
    pub fn virtual_keyboard_release(&mut self) {
        if self.vkbd.session.is_none() {
            return;
        }
        let pressed = self.vkbd.book.pressed_count();
        self.vkbd.teardown();
        self.vkbd_remove_source();
        tracing::debug!(pressed, "sanal klavye bırakıldı");
    }

    /// Tuş/değiştirici göndermeye hazır oturum.
    fn vkbd_ready(&self) -> Result<&Session, VirtualKeyboardError> {
        if let Some(e) = self.vkbd.blocked {
            return Err(e);
        }
        let Some(session) = self.vkbd.session.as_ref() else {
            return Err(VirtualKeyboardError::NoKeymap);
        };
        if session.keyboard.is_none() {
            return Err(VirtualKeyboardError::NoSeat);
        }
        Ok(session)
    }

    fn vkbd_flush(&mut self) -> Result<(), VirtualKeyboardError> {
        let Some(session) = self.vkbd.session.as_ref() else {
            return Ok(());
        };
        session.flush().inspect_err(|&e| self.vkbd_fail(e, false))
    }

    /// Bağlantı hatası: oturum kapanır. `Unauthorized`/`Unsupported` kalıcıdır (tek uyarı);
    /// `Disconnected` sonraki `set_keymap`'te yeniden denenir. `from_source`: olay kaynağının
    /// kendi geri çağrısındayız (kaynak `PostAction::Remove` ile düşer).
    fn vkbd_fail(&mut self, e: VirtualKeyboardError, from_source: bool) {
        // Bağlantı ölü: istek gönderilemez; basılı tuşları compositor istemciyle birlikte bırakır.
        self.vkbd.session = None;
        self.vkbd.book.forget();
        self.vkbd.keymap = None;
        if from_source {
            self.vkbd.token = None;
        } else {
            self.vkbd_remove_source();
        }
        match e {
            VirtualKeyboardError::Unauthorized | VirtualKeyboardError::Unsupported => {
                if self.vkbd.blocked.is_none() {
                    tracing::warn!(%e, "sanal klavye bu oturumda kapalı");
                }
                self.vkbd.blocked = Some(e);
            }
            _ => tracing::warn!(%e, "sanal klavye bağlantısı kapandı"),
        }
    }

    fn vkbd_install_source(&mut self) {
        let Some(session) = self.vkbd.session.as_ref() else {
            return;
        };
        let fd = match session.conn.backend().poll_fd().try_clone_to_owned() {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!("sanal klavye fd'si kopyalanamadı: {e}");
                return;
            }
        };
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        match self.handle.insert_source(source, |_, _, state| {
            Ok(if state.cx.vkbd_readable() {
                PostAction::Continue
            } else {
                PostAction::Remove
            })
        }) {
            Ok(token) => self.vkbd.token = Some(token),
            Err(e) => tracing::warn!("sanal klavye döngüye eklenemedi: {e}"),
        }
    }

    fn vkbd_remove_source(&mut self) {
        if let Some(token) = self.vkbd.token.take() {
            self.handle.remove(token);
        }
    }

    /// İkinci bağlantı okunabilir. `false`: oturum kapandı, kaynak düşmeli.
    fn vkbd_readable(&mut self) -> bool {
        let Some(session) = self.vkbd.session.as_mut() else {
            self.vkbd.token = None;
            return false;
        };
        if let Err(e) = session.pump() {
            self.vkbd_fail(e, true);
            return false;
        }
        if self.vkbd.session.as_ref().is_some_and(|s| s.client.manager_lost) {
            tracing::info!("zwp_virtual_keyboard_manager_v1 geri çekildi");
            self.vkbd.teardown();
            self.vkbd.token = None;
            return false;
        }
        self.vkbd_sync_seat();
        let Some(session) = self.vkbd.session.as_ref() else {
            return false;
        };
        match session.flush() {
            Ok(()) => true,
            Err(e) => {
                self.vkbd_fail(e, true);
                false
            }
        }
    }

    /// Koltuk değiştiyse nesneyi yeniden kurar ve son keymap'i yeniden gönderir.
    fn vkbd_sync_seat(&mut self) {
        let vkbd = &mut self.vkbd;
        let Some(session) = vkbd.session.as_mut() else {
            return;
        };
        if !std::mem::take(&mut session.client.seat_dirty) {
            return;
        }
        if session.keyboard.is_some() {
            // Koltuk gitti ya da değişti: aygıtla birlikte basılılar compositor'da bırakılır.
            session.destroy_keyboard();
            vkbd.book.forget();
        }
        if let Some((_, blob)) = vkbd.keymap.as_ref() {
            session.create_keyboard(blob);
        }
        tracing::debug!(
            seat = session.client.seat.is_some(),
            "sanal klavye koltuğu değişti"
        );
    }
}

/// Ana bağlantıda yönetici global'inin kaydı (bağlanmaz): açılışta `run_with`'ten ve çalışma
/// sırasında ilanı/geri çekilmesi.
#[derive(Debug)]
pub(crate) struct VirtualKeyboardGlobal;

impl<A: App> RegistryHandler<State<A>> for VirtualKeyboardGlobal {
    fn new_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        interface: &str,
        _: u32,
    ) {
        if interface == MANAGER_INTERFACE && state.cx.vkbd.global.is_none() {
            state.cx.vkbd.global = Some(name);
        }
    }

    fn remove_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        _: &str,
    ) {
        if state.cx.vkbd.global == Some(name) {
            state.cx.vkbd.global = None;
            // İkinci bağlantı kendi geri çekilme olayını da alır; burada hemen kapatılır.
            state.cx.virtual_keyboard_release();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::os::fd::AsRawFd as _;

    use super::*;

    fn us_keymap() -> String {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        xkb::Keymap::new_from_names(&context, "", "", "us", "", None, xkb::COMPILE_NO_FLAGS)
            .expect("us keymap")
            .get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
    }

    #[test]
    fn keymap_memfd_is_nul_terminated_and_sealed() {
        let text = "xkb_keymap { };";
        let (fd, size) = keymap_memfd(text).unwrap();
        assert_eq!(size as usize, text.len() + 1);
        let seals = rustix::fs::fcntl_get_seals(&fd).unwrap();
        assert!(seals.contains(SealFlags::WRITE | SealFlags::SHRINK | SealFlags::GROW));
        assert!(seals.contains(SealFlags::SEAL));
        // Compositor gibi baştan okunur (kopya fd aynı dosya; konum sıfırlanır).
        let mut file = std::fs::File::from(fd);
        rustix::fs::seek(&file, rustix::fs::SeekFrom::Start(0)).unwrap();
        let mut back = Vec::new();
        file.read_to_end(&mut back).unwrap();
        assert_eq!(back.last(), Some(&0));
        assert_eq!(&back[..back.len() - 1], text.as_bytes());
        // Mühür: yazılamaz.
        assert!(file.write_all(b"x").is_err());
        assert!(file.as_raw_fd() >= 0);
    }

    #[test]
    fn keymap_is_validated_before_sending() {
        let us = us_keymap();
        assert_eq!(validate_keymap(&us), Ok(us.as_str()));
        // Sondaki NUL'lar atılır (compositor metni NUL'lu gelebilir).
        let with_nul = format!("{us}\0\0");
        assert_eq!(validate_keymap(&with_nul), Ok(us.as_str()));
        assert_eq!(validate_keymap(""), Err(VirtualKeyboardError::InvalidKeymap));
        assert_eq!(validate_keymap("\0"), Err(VirtualKeyboardError::InvalidKeymap));
        assert_eq!(
            validate_keymap("xkb_keymap {\0 };"),
            Err(VirtualKeyboardError::InvalidKeymap)
        );
        assert_eq!(
            validate_keymap("bu bir keymap değil"),
            Err(VirtualKeyboardError::InvalidKeymap)
        );
        let huge = "x".repeat(MAX_KEYMAP_LEN + 1);
        assert_eq!(validate_keymap(&huge), Err(VirtualKeyboardError::InvalidKeymap));
    }

    #[test]
    fn pressed_keys_are_tracked() {
        let mut book = KeyBook::default();
        assert_eq!(book.key(30, true), Ok(true));
        // Basılıya ikinci basma ve basılı olmayanı bırakma gönderilmez.
        assert_eq!(book.key(30, true), Ok(false));
        assert_eq!(book.key(31, false), Ok(false));
        assert_eq!(book.key(42, true), Ok(true));
        assert_eq!(book.pressed_count(), 2);
        assert_eq!(book.key(30, false), Ok(true));
        assert_eq!(book.key(30, false), Ok(false));
        assert_eq!(book.pressed_count(), 1);
        assert_eq!(book.key(KEY_MAX + 1, true), Err(VirtualKeyboardError::InvalidKey));
        assert_eq!(book.key(KEY_MAX, true), Ok(true));
    }

    #[test]
    fn pressed_keys_are_bounded() {
        let mut book = KeyBook::default();
        for code in 0..MAX_PRESSED as u32 {
            assert_eq!(book.key(code + 1, true), Ok(true));
        }
        assert_eq!(book.key(200, true), Err(VirtualKeyboardError::TooManyKeys));
        // Bırakma her zaman mümkün.
        assert_eq!(book.key(1, false), Ok(true));
        assert_eq!(book.key(200, true), Ok(true));
    }

    #[test]
    fn release_drains_keys_and_modifiers() {
        let mut book = KeyBook::default();
        assert!(!book.modifiers([0; 4]));
        assert!(book.modifiers([1, 0, 2, 0]));
        assert!(!book.modifiers([1, 0, 2, 0]));
        book.key(42, true).unwrap();
        book.key(30, true).unwrap();
        assert_eq!(book.drain(), (vec![42, 30], true));
        // İkinci bırakma adımsız.
        assert_eq!(book.drain(), (Vec::new(), false));
        assert_eq!(book, KeyBook::default());
        // Unutma (aygıt gitti): adım üretmeden siler.
        book.key(30, true).unwrap();
        book.modifiers([1, 0, 0, 0]);
        book.forget();
        assert_eq!(book.drain(), (Vec::new(), false));
    }

    #[test]
    fn debug_does_not_leak_key_codes() {
        let mut book = KeyBook::default();
        book.key(123, true).unwrap();
        let s = format!("{book:?}");
        assert!(!s.contains("123"), "{s}");
        assert!(s.contains("pressed: 1"), "{s}");
    }

    #[test]
    fn unauthorized_is_told_apart() {
        let err = |interface: &str, code| ProtocolError {
            code,
            object_id: 3,
            object_interface: interface.to_owned(),
            message: String::new(),
        };
        assert_eq!(
            classify(&err(MANAGER_INTERFACE, 0)),
            VirtualKeyboardError::Unauthorized
        );
        // Klavyenin `no_keymap`'i (aynı kod, başka arayüz) ve diğerleri kalıcı "desteklenmiyor".
        assert_eq!(
            classify(&err("zwp_virtual_keyboard_v1", 0)),
            VirtualKeyboardError::Unsupported
        );
        assert_eq!(classify(&err("wl_display", 2)), VirtualKeyboardError::Unsupported);
    }
}
