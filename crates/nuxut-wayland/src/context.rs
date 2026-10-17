//! `Context`: uygulamanın olay işleyicilerinden runtime'a eriştiği tek kapı.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use nuxut_config::ConfigWatcher;
use nuxut_core::input::CursorShape;
use nuxut_core::{Insets, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;
use smithay_client_toolkit::compositor::{CompositorState, Region};
use smithay_client_toolkit::session_lock::{SessionLock, SessionLockState, SessionLockSurface};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{self, LayerShell};
use smithay_client_toolkit::shell::xdg::XdgShell;
use smithay_client_toolkit::shell::xdg::window::{Window, WindowDecorations};
use wayland_client::protocol::wl_output::WlOutput;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{Connection, QueueHandle};
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

use crate::App;
use crate::activation::{ActivationRequestId, ActivationToken};
use crate::clipboard::{Clipboard, PasteRequestId};
use crate::cursor::Cursor;
use crate::ime::{Ime, TextInputState};
use crate::state::{FractionalData, NoEvents, State};
use crate::surface::{LayerPlacement, Presentation, Role, SizeHints, SurfaceEntry, place_layer};
use crate::types::{
    Anchor, KeyboardMode, Layer, LayerSurfaceSpec, LockEnd, LockError, LockState, OutputId,
    OutputTarget, ResizeEdge, SurfaceId, TimerId, WindowSpec, WindowState,
};

/// Arka plan thread'lerinden `App::message`'a mesaj gönderir. Ucuzca kopyalanır.
#[derive(Debug)]
pub struct MessageSender<M>(calloop::channel::Sender<M>);

impl<M> Clone for MessageSender<M> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<M> MessageSender<M> {
    /// Mesajı kuyruğa koyar ve olay döngüsünü uyandırır (calloop channel). Runtime
    /// kapandıysa [`Closed`] döner; mesaj düşürülür.
    pub fn send(&self, message: M) -> Result<(), Closed> {
        self.0.send(message).map_err(|_| Closed)
    }
}

/// [`MessageSender::send`]: alıcı olay döngüsü artık yok.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("nuxut runtime kapandı; mesaj iletilemedi")]
pub struct Closed;

#[derive(Debug)]
pub(crate) struct OutputEntry {
    pub wl: WlOutput,
    pub scale: i32,
    /// `wl_output.transform` kodu (0..=7).
    pub transform: u32,
}

/// Etkin kilit: istek (`Pending`) ya da kurulmuş (`Locked`).
pub(crate) struct ActiveLock {
    pub session: SessionLock,
    pub state: LockState,
}

/// Context'ten uygulamaya, olay işleyicisi bittikten sonra verilecek olaylar.
#[derive(Debug)]
pub(crate) enum RuntimeEvent {
    LockSurfaceAdded(SurfaceId, OutputId),
    LockFinished(LockEnd),
    ActivationToken(ActivationRequestId, Option<ActivationToken>),
    ToplevelExported(
        crate::foreign::ExportRequestId,
        SurfaceId,
        Option<crate::foreign::ToplevelHandle>,
    ),
    FocusGrabCleared(crate::focus_grab::FocusGrabId),
    ColorTransformBlocked,
    Toplevel(crate::toplevels::ToplevelEvent),
    Captured(
        crate::capture::CaptureId,
        Result<crate::capture::CapturedImage, crate::capture::CaptureError>,
    ),
    Idle(crate::idle::IdleWatchId, bool),
    DataControl(crate::data_control::DataControlEvent),
    DataRead(
        crate::data_control::DataReadId,
        crate::data_control::ReadOutcome,
    ),
    Paste(
        PasteRequestId,
        Result<nuxut_core::clipboard::ClipboardText, nuxut_core::clipboard::PasteError>,
    ),
}

/// `wl_display.sync` ile, şimdiye dek gönderilen isteklerin compositor'da işlendiğini bekler
/// (ayrı kuyruk; ana kuyruğun olayları kaybolmaz, okunup kendi kuyruğuna konur).
fn sync_roundtrip(conn: &Connection) -> Result<(), wayland_client::DispatchError> {
    struct Done(bool);
    impl wayland_client::Dispatch<wayland_client::protocol::wl_callback::WlCallback, ()> for Done {
        fn event(
            state: &mut Self,
            _: &wayland_client::protocol::wl_callback::WlCallback,
            event: wayland_client::protocol::wl_callback::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wayland_client::protocol::wl_callback::Event::Done { .. } = event {
                state.0 = true;
            }
        }
    }
    let mut queue = conn.new_event_queue::<Done>();
    let qh = queue.handle();
    conn.display().sync(&qh, ());
    let mut done = Done(false);
    while !done.0 {
        queue.blocking_dispatch(&mut done)?;
    }
    Ok(())
}
pub(crate) struct Protocols {
    pub compositor: CompositorState,
    pub layer_shell: LayerShell,
    /// Normal pencereler için; yoksa `create_window` hata loglar.
    pub xdg_shell: Option<XdgShell>,
    /// ext-session-lock; global yoksa `lock()` hata döner.
    pub session_lock: SessionLockState,
    /// İkisi birlikte varsa kesirli ölçek yolu kullanılır.
    pub fractional: Option<(WpFractionalScaleManagerV1, WpViewporter)>,
    /// İşaretçi imleç teması (xcursor yedeği) için.
    pub shm: Option<WlShm>,
    /// xdg-activation-v1; yoksa belirteç istekleri `None` ile biter.
    pub activation: Option<XdgActivationV1>,
}

/// Uygulamanın runtime'a eriştiği bağlam. Olay işleyicilerine ve `draw`'a `&mut` verilir.
pub struct Context<A: App> {
    pub(crate) qh: QueueHandle<State<A>>,
    pub(crate) handle: LoopHandle<'static, State<A>>,
    pub(crate) protocols: Protocols,
    pub(crate) outputs: HashMap<OutputId, OutputEntry>,
    pub(crate) surfaces: HashMap<SurfaceId, SurfaceEntry>,
    /// Yok edilmeyi bekleyen yüzeyler; EGL yüzeyi runtime tarafından olay turu sonunda
    /// (Wayland nesnelerinden önce) yok edilir.
    pub(crate) graveyard: Vec<SurfaceEntry>,
    next_id: u64,
    timers: HashMap<TimerId, RegistrationToken>,
    pub(crate) theme: Theme,
    /// `draw` süresince renderer'a ödünç verilir (`None`); ilk kullanımda yaratılır.
    pub(crate) text: Option<TextSystem>,
    /// Açılışta EGL ile paralel başlatılan font taraması (`run_with`); ilk kullanımda beklenir.
    pub(crate) text_loading: Option<std::thread::JoinHandle<TextSystem>>,
    sender: MessageSender<A::Message>,
    /// `framework` her zaman izlenir; uygulamanın istedikleri ek olarak.
    watched: Vec<String>,
    watcher: Option<(ConfigWatcher, RegistrationToken)>,
    /// `palette.toml` (state dizini) izleyicisi.
    palette_watcher: Option<(ConfigWatcher, RegistrationToken)>,
    /// Kullanıcı çeviri dizini izleyicisi (`$XDG_DATA_HOME/nuxut/locales/<domain>`); yalnızca
    /// kayıtlı i18n kaynağı varsa.
    i18n_watch: Option<I18nWatch>,
    /// Son işaretçi basma (koltuk, serial): `begin_move`/`begin_resize` için.
    pub(crate) last_press: Option<(WlSeat, u32)>,
    /// Son girdi (tuş ya da işaretçi basma; koltuk, serial): etkinleştirme belirteci için.
    pub(crate) last_input: Option<(WlSeat, u32)>,
    /// Oturum kilidi (tek).
    pub(crate) lock: Option<ActiveLock>,
    /// İşleyici bittikten sonra uygulamaya verilecek olaylar (`State::after_dispatch`).
    pub(crate) pending: Vec<RuntimeEvent>,
    /// Metin girişi / IME ve klavye düzeni.
    pub(crate) ime: Ime,
    /// Sistem panosu ve birincil seçim.
    pub(crate) clipboard: Clipboard,
    /// Odak kapma (hyprland_focus_grab_v1).
    pub(crate) focus_grabs: crate::focus_grab::FocusGrabs,
    /// Pencere dışa aktarma (xdg-foreign-unstable-v2).
    pub(crate) foreign: crate::foreign::Foreign,
    /// Renk dönüşüm matrisi (hyprland_ctm_control_v1).
    pub(crate) color_transforms: crate::color_transform::ColorTransforms,
    /// Pencere listesi (ext-foreign-toplevel-list-v1) ve Hyprland adres eşlemesi.
    pub(crate) toplevels: crate::toplevels::Toplevels,
    /// Tek kare yakalama (ext-image-copy-capture-v1 / wlr-screencopy).
    pub(crate) captures: crate::capture::Captures,
    /// Boşta bildirimi (ext-idle-notify-v1).
    pub(crate) idle: crate::idle::IdleNotify,
    /// Sanal klavye (zwp-virtual-keyboard-v1, ayrı bağlantıda).
    pub(crate) vkbd: crate::virtual_keyboard::VirtualKeyboard,
    /// Pano izleme ve odaksız seçim (ext/wlr data-control).
    pub(crate) data_control: crate::data_control::DataControl,
    /// İmleç şekli (yüzey başına) ve temalı işaretçi.
    pub(crate) cursor: Cursor,
    /// Dokunmatik ekran: birincil temasın işaretçi öykünmesi (`touch`).
    pub(crate) touch: crate::touch::TouchEmulator,
    /// Birincil temasın bekleme zamanlayıcısı (`TOUCH_TAP_DELAY`).
    pub(crate) touch_hold: Option<RegistrationToken>,
    /// `set_cursor` gibi hemen uygulanan istekler için.
    pub(crate) conn: Connection,
    pub(crate) quit: bool,
    pub(crate) in_draw: bool,
    /// AT-SPI adaptörleri (ADR-0011).
    #[cfg(feature = "a11y")]
    pub(crate) a11y: crate::a11y::A11y,
    /// Kilit yüzeyleri de erişilebilirlik ağacı yayımlar mı (`set_a11y_lock_surfaces`; varsayılan
    /// kapalı, ADR-0011 "Kilit ekranı").
    #[cfg(feature = "a11y")]
    a11y_lock_surfaces: bool,
}

impl<A: App> std::fmt::Debug for Context<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("outputs", &self.outputs.len())
            .field("surfaces", &self.surfaces.len())
            .field("timers", &self.timers.len())
            .field("watched", &self.watched)
            .finish_non_exhaustive()
    }
}

impl<A: App> Context<A> {
    pub(crate) fn new(
        qh: QueueHandle<State<A>>,
        handle: LoopHandle<'static, State<A>>,
        protocols: Protocols,
        theme: Theme,
        sender: calloop::channel::Sender<A::Message>,
        conn: Connection,
    ) -> Self {
        Self {
            qh,
            handle,
            protocols,
            outputs: HashMap::new(),
            surfaces: HashMap::new(),
            graveyard: Vec::new(),
            next_id: 0,
            timers: HashMap::new(),
            theme,
            text: None,
            text_loading: None,
            sender: MessageSender(sender),
            watched: vec![nuxut_theme::CONFIG_NAME.to_owned()],
            watcher: None,
            palette_watcher: None,
            i18n_watch: None,
            last_press: None,
            last_input: None,
            lock: None,
            pending: Vec::new(),
            ime: Ime::default(),
            clipboard: Clipboard::default(),
            focus_grabs: Default::default(),
            foreign: Default::default(),
            color_transforms: Default::default(),
            toplevels: Default::default(),
            captures: Default::default(),
            idle: Default::default(),
            vkbd: Default::default(),
            data_control: Default::default(),
            cursor: Cursor::default(),
            touch: Default::default(),
            touch_hold: None,
            conn,
            quit: false,
            in_draw: false,
            #[cfg(feature = "a11y")]
            a11y: crate::a11y::A11y::new(None),
            #[cfg(feature = "a11y")]
            a11y_lock_surfaces: false,
        }
    }

    pub(crate) fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn structural_change(&self, what: &str) -> bool {
        debug_assert!(!self.in_draw, "draw içinde yapı değişmez: {what}");
        if self.in_draw {
            tracing::error!("draw içinde yapı değişikliği yok sayıldı: {what}");
            return false;
        }
        true
    }

    /// Bir layer-shell yüzeyi yaratır. Yüzey ilk configure'dan sonra `App::surface_resized`
    /// ve ardından `App::draw` alır. `OutputTarget::Compositor` (ya da bilinmeyen çıkış):
    /// çıkışı compositor seçer, `surface_output` ilk `wl_surface.enter`e dek `None`.
    pub fn create_layer_surface(&mut self, spec: LayerSurfaceSpec) -> SurfaceId {
        let id = SurfaceId(self.next_id());
        if !self.structural_change("create_layer_surface") {
            return id;
        }
        let placement = place_layer(
            spec.output,
            |o| self.outputs.get(&o).map(|e| e.scale),
            self.outputs.values().map(|e| e.scale),
        );
        if let OutputTarget::Output(requested) = spec.output
            && placement.wire.is_none()
        {
            tracing::warn!(
                ?requested,
                "bilinmeyen çıkış; compositor çıkışı kendisi seçecek"
            );
        }
        let output = placement.wire.and_then(|o| self.outputs.get(&o));
        let scale = placement.scale;

        let p = &self.protocols;
        let wl_surface = p.compositor.create_surface_with_data(&self.qh, None, 1, id);
        let fractional = p.fractional.as_ref().map(|(manager, viewporter)| {
            (
                manager.get_fractional_scale(&wl_surface, &self.qh, FractionalData(id)),
                viewporter.get_viewport(&wl_surface, &self.qh, NoEvents),
            )
        });
        let layer = p.layer_shell.create_layer_surface(
            &self.qh,
            wl_surface,
            to_wlr_layer(spec.layer),
            Some(spec.namespace.clone()),
            output.map(|o| &o.wl),
        );
        layer.set_anchor(wlr_layer::Anchor::from_bits_truncate(u32::from(
            spec.anchor.bits(),
        )));
        // Karşılıklı anchor'u olmayan eksende 0 boyut protokol hatasıdır ve tüm bağlantıyı
        // düşürür; yüzeyi reddetmek yerine 1'e yükselt ve uyar.
        let size = sanitize_size(spec.size, spec.anchor);
        if size != spec.size {
            tracing::warn!(
                namespace = spec.namespace,
                requested = ?spec.size,
                used = ?size,
                "boyut 0 olan eksende karşılıklı anchor yok; 1 kullanılıyor"
            );
        }
        layer.set_size(size.0, size.1);
        layer.set_exclusive_zone(spec.exclusive_zone);
        let m = spec.margin;
        layer.set_margin(m.top as i32, m.right as i32, m.bottom as i32, m.left as i32);
        layer.set_keyboard_interactivity(to_wlr_keyboard(spec.keyboard));
        // İlk commit tamponsuz: compositor configure ile cevap verir.
        layer.commit();

        self.surfaces.insert(
            id,
            SurfaceEntry {
                output: placement.wire,
                follows_enter: placement.follows_enter,
                requested: size,
                size,
                scale,
                configured: false,
                wants_frame: true,
                callback_pending: false,
                scale_dirty: true,
                fractional,
                placement: Some(crate::surface::PlacementState::new(LayerPlacement::new(
                    spec.anchor,
                    spec.margin,
                    size,
                ))),
                damage: Default::default(),
                has_buffer: false,
                presentation: Presentation::default(),
                gpu: None,
                shm: None,
                role: Role::Layer(layer),
            },
        );
        #[cfg(feature = "a11y")]
        self.a11y.surface_added(id, Some(crate::a11y::LayerPlace { anchor: spec.anchor, margin: spec.margin }));
        id
    }

    /// Yüzeyi kaldırır. Bilinmeyen kimlik yok sayılır.
    pub fn destroy_surface(&mut self, id: SurfaceId) {
        if !self.structural_change("destroy_surface") {
            return;
        }
        // Grab beyaz listesinden wl_surface hâlâ canlıyken çıkar.
        self.focus_grab_forget_surface(id);
        // Dışa aktarma `xdg_toplevel`'dan önce bırakılır.
        self.foreign.forget(id, &mut self.pending);
        if let Some(entry) = self.surfaces.remove(&id) {
            self.graveyard.push(entry);
        }
        self.ime.forget(id);
        self.cursor.tracker.forget(id);
        self.touch.surface_removed(id);
    }

    /// Yüzeyin erişilebilirlik ağacı bir yardımcı teknoloji (Orca) için yayımlanıyor mu
    /// (ADR-0011). `a11y` özelliği kapalıyken ya da AT yokken `false`: uygulama a11y'ye özgü
    /// pahalı işi (ör. duyuru metni hazırlamak) buna bağlayabilir.
    pub fn a11y_active(&self, surface: SurfaceId) -> bool {
        #[cfg(feature = "a11y")]
        return self.a11y.is_active(surface);
        #[cfg(not(feature = "a11y"))]
        {
            let _ = surface;
            false
        }
    }

    /// Yüzey yeniden çizilmeden değişen erişilebilir durum (ör. yalnız ad değişti): ağaç bu
    /// turun sonunda yeniden kurulur. Çizilen yüzeyin ağacı zaten kendiliğinden yenilenir.
    /// AT yokken no-op.
    pub fn a11y_invalidate(&mut self, surface: SurfaceId) {
        #[cfg(feature = "a11y")]
        self.a11y.invalidate(surface);
        #[cfg(not(feature = "a11y"))]
        let _ = surface;
    }

    /// Erişilebilirliği askıya al / sürdür (ADR-0011 tehdit modeli). Askıdayken tüm yüzeylerin
    /// ağacı AT'ye içeriksiz (yalnız kök) gösterilir, AT eylemleri atılır, pencere odağı
    /// bildirilmez; `App::accessibility*` çağrılmaz. Oturum başka bir süreçle kilitliyken
    /// (nuxut-lock; runtime'ın kendi kilidi `input_allowed` ile zaten kapalı) uygulama bunu
    /// açmak ZORUNDA: a11y bus'ında erişim denetimi yok, aynı kullanıcının her süreci ağacı okuyup
    /// eylem çağırabilir. Özellik kapalıyken no-op.
    pub fn set_a11y_suspended(&mut self, suspended: bool) {
        #[cfg(feature = "a11y")]
        self.a11y.set_suspended(suspended);
        #[cfg(not(feature = "a11y"))]
        let _ = suspended;
    }

    /// Kilit yüzeyleri (ext-session-lock) erişilebilirlik ağacı yayımlasın (ADR-0011 "Kilit
    /// ekranı"; varsayılan kapalı). Yalnız kilit ekranı uygulaması, `lock_session`'dan ÖNCE açar
    /// ve şu kuralları üstlenir: ağaçta saat, tarih, ad, durum ve parola alanı (yalnız boş/dolu;
    /// değer ve uzunluk yok, `TextInput::access` değil); AT
    /// yalnız okur ve odak taşır (`App::accessibility_action` yalnız `Focus`); kilidi açma ve
    /// parola yalnız gerçek girdiyle; boş parolada açma düğmesi devre dışı. Askı
    /// (`set_a11y_suspended`) kilit sürecini kapsamaz: kilit ekranı askıya alınmaz. Özellik
    /// kapalıyken no-op.
    pub fn set_a11y_lock_surfaces(&mut self, on: bool) {
        #[cfg(feature = "a11y")]
        {
            self.a11y_lock_surfaces = on;
        }
        #[cfg(not(feature = "a11y"))]
        let _ = on;
    }

    /// Yüzeyi erişilebilirlik ağacından tamamen çıkarır (adaptörü düşer): içeriği olmayan
    /// yardımcı yüzeyler (dışlama şeritleri, duvar kağıdı) Orca'da boş pencere olarak görünmesin.
    /// Geri alınamaz (yüzey ömrü boyunca). Özellik kapalıyken no-op.
    pub fn a11y_exclude(&mut self, surface: SurfaceId) {
        #[cfg(feature = "a11y")]
        self.a11y.retain(|id| id != surface);
        #[cfg(not(feature = "a11y"))]
        let _ = surface;
    }

    pub fn a11y_suspended(&self) -> bool {
        #[cfg(feature = "a11y")]
        return self.a11y.is_suspended();
        #[cfg(not(feature = "a11y"))]
        false
    }

    /// Metin girişi durumu. `Some`: bu yüzey (text-input) odaktayken IME etkin ve durum ona
    /// gönderilir; durum değişince (imleç, çevre metin) yeniden çağrılır. `None`: kapalı.
    /// IME ya da protokol yoksa no-op.
    pub fn set_text_input(&mut self, id: SurfaceId, state: Option<TextInputState>) {
        match state {
            Some(state) => {
                if self.ime.states.get(&id) == Some(&state) {
                    return;
                }
                self.ime.states.insert(id, state);
            }
            None => {
                if self.ime.states.remove(&id).is_none() {
                    return;
                }
            }
        }
        if self.ime.focus == Some(id) {
            self.ime.sync();
        }
    }

    /// Etkin xkb düzeninin adı ("Turkish", "English (US)"…); klavye yoksa `None`.
    pub fn keyboard_layout(&self) -> Option<&str> {
        self.ime.layout_name()
    }

    /// Tıklamaların yakalandığı bölgeyi ayarlar. Dikdörtgenler mantıksal ve yüzeye göredir;
    /// runtime her birini dışa doğru tam sayıya yuvarlar (`Rect::round_out`). Boş dilim = tüm
    /// tıklamalar alttaki pencerelere geçer. Hiç çağrılmazsa tüm yüzey tıklama alır.
    pub fn set_input_region(&mut self, id: SurfaceId, rects: &[Rect]) {
        if !self.structural_change("set_input_region") {
            return;
        }
        let Some(entry) = self.surfaces.get(&id) else {
            return;
        };
        let region = match Region::new(&self.protocols.compositor) {
            Ok(region) => region,
            Err(e) => {
                tracing::warn!("wl_region yaratılamadı: {e}");
                return;
            }
        };
        for rect in rects {
            let r = rect.round_out();
            if r.width > 0.0 && r.height > 0.0 {
                region.add(r.x as i32, r.y as i32, r.width as i32, r.height as i32);
            }
        }
        let surface = entry.role.wl_surface();
        surface.set_input_region(Some(region.wl_region()));
        // Çift tamponlu durum: tampon varsa kare beklemeden uygula; yoksa ilk swap uygular
        // (tamponsuz commit kilit yüzeyinde protokol hatası). `region` burada yok edilebilir.
        if entry.has_buffer {
            surface.commit();
        }
    }

    /// Yalnızca layer yüzeyleri; pencerelerde etkisiz.
    pub fn set_exclusive_zone(&mut self, id: SurfaceId, zone: i32) {
        match self.surfaces.get(&id).map(|e| &e.role) {
            Some(Role::Layer(layer)) => {
                layer.set_exclusive_zone(zone);
                layer.commit();
            }
            Some(Role::Window { .. } | Role::Lock(_)) => {
                tracing::debug!(?id, "set_exclusive_zone yalnızca layer yüzeyinde")
            }
            None => {}
        }
    }

    /// Layer yüzeyinin anchor'ını, margin'ini ve (verilirse) mantıksal boyutunu değiştirir
    /// (`None`: istenen boyut korunur). Değişiklik yüzeyin bir sonraki karesinin commit'iyle
    /// tek seferde gider: kare beklenirken gelen çağrılar birleşir, son değer kazanır (yüzen
    /// panel sürüklenirken kare başına çağrılabilir). Bu yüzden yüzeye kare istenir
    /// (`App::draw` çağrılır; içerik değişmediyse tamponsuz commit). Gönderilenle aynıysa istek
    /// gitmez. Boyut değişince compositor configure gönderir → `App::surface_resized`. Henüz
    /// tampon yoksa ilk karenin commit'ine biner. Pencere ve kilit yüzeyinde etkisiz.
    pub fn set_layer_placement(
        &mut self,
        id: SurfaceId,
        anchor: Anchor,
        margin: Insets,
        size: Option<(u32, u32)>,
    ) {
        let Some(entry) = self.surfaces.get_mut(&id) else {
            return;
        };
        let Some(placement) = entry.placement.as_mut() else {
            tracing::debug!(?id, "set_layer_placement yalnızca layer yüzeyinde");
            return;
        };
        let wanted = size.unwrap_or(placement.current().size);
        let size = sanitize_size(wanted, anchor);
        if size != wanted {
            tracing::warn!(
                ?id,
                requested = ?wanted,
                used = ?size,
                "boyut 0 olan eksende karşılıklı anchor yok; 1 kullanılıyor"
            );
        }
        if !placement.request(LayerPlacement::new(anchor, margin, size)) {
            return;
        }
        if !placement.has_pending() {
            // Gönderilene geri dönüldü: bekleyen iptal, kare gerekmez.
            return;
        }
        if entry.has_buffer {
            entry.wants_frame = true;
        } else if let Some(_p) = entry.apply_placement() {
            // İlk tampondan önce: durum ilk karenin commit'iyle gider.
            #[cfg(feature = "a11y")]
            self.a11y.set_place(id, crate::a11y::LayerPlace { anchor: _p.anchor, margin: _p.insets() });
        }
    }

    /// Yalnızca layer yüzeyleri; pencerelerde etkisiz (pencereler normal odak kurallarına uyar).
    pub fn set_keyboard_mode(&mut self, id: SurfaceId, mode: KeyboardMode) {
        match self.surfaces.get(&id).map(|e| &e.role) {
            Some(Role::Layer(layer)) => {
                layer.set_keyboard_interactivity(to_wlr_keyboard(mode));
                layer.commit();
            }
            Some(Role::Window { .. } | Role::Lock(_)) => {
                tracing::debug!(?id, "set_keyboard_mode yalnızca layer yüzeyinde")
            }
            None => {}
        }
    }

    /// Normal uygulama penceresi (`xdg_toplevel`) yaratır. Layer yüzeyleriyle aynı yoldan
    /// `surface_resized`/`draw`/girdi alır; başlık çubuğunu uygulama çizer.
    pub fn create_window(&mut self, spec: WindowSpec) -> SurfaceId {
        let id = SurfaceId(self.next_id());
        if !self.structural_change("create_window") {
            return id;
        }
        let p = &self.protocols;
        let Some(xdg_shell) = p.xdg_shell.as_ref() else {
            tracing::error!("compositor xdg_wm_base sunmuyor; pencere yaratılamadı");
            return id;
        };
        let wl_surface = p.compositor.create_surface_with_data(&self.qh, None, 1, id);
        let fractional = p.fractional.as_ref().map(|(manager, viewporter)| {
            (
                manager.get_fractional_scale(&wl_surface, &self.qh, FractionalData(id)),
                viewporter.get_viewport(&wl_surface, &self.qh, NoEvents),
            )
        });
        let window =
            xdg_shell.create_window(wl_surface, WindowDecorations::RequestServer, &self.qh);
        window.set_title(spec.title.clone());
        window.set_app_id(spec.app_id.clone());
        let hints = SizeHints::new(spec.min_size, spec.max_size);
        let raw = (
            spec.min_size.unwrap_or((0, 0)),
            spec.max_size.unwrap_or((0, 0)),
        );
        if (hints.min, hints.max) != raw {
            tracing::warn!(
                app_id = spec.app_id,
                min = ?spec.min_size,
                max = ?spec.max_size,
                used_min = ?hints.min,
                used_max = ?hints.max,
                "pencere boyut ipuçları çelişkili/taşkın; düzeltildi"
            );
        }
        send_size_hints(&window, hints);
        // İlk commit tamponsuz: compositor configure ile cevap verir.
        window.commit();

        let size = (spec.size.0.max(1), spec.size.1.max(1));
        self.surfaces.insert(
            id,
            SurfaceEntry {
                output: None,
                follows_enter: false,
                requested: size,
                size,
                scale: 1.0,
                configured: false,
                wants_frame: true,
                callback_pending: false,
                scale_dirty: true,
                fractional,
                placement: None,
                damage: Default::default(),
                has_buffer: false,
                presentation: Presentation::default(),
                gpu: None,
                shm: None,
                role: Role::Window {
                    window,
                    state: WindowState::default(),
                    hints,
                },
            },
        );
        #[cfg(feature = "a11y")]
        self.a11y.surface_added(id, None);
        id
    }

    fn window(&self, id: SurfaceId) -> Option<&Window> {
        match self.surfaces.get(&id).map(|e| &e.role) {
            Some(Role::Window { window, .. }) => Some(window),
            _ => None,
        }
    }

    pub fn set_window_title(&mut self, id: SurfaceId, title: &str) {
        if let Some(window) = self.window(id) {
            window.set_title(title.to_owned());
        }
    }

    /// Çalışan pencerenin app-id'sini değiştirir (`xdg_toplevel.set_app_id`; hemen geçerli,
    /// compositor pencere kurallarını yeniden değerlendirebilir). Layer/kilit yüzeyinde etkisiz.
    pub fn set_app_id(&mut self, id: SurfaceId, app_id: &str) {
        match self.surfaces.get(&id).map(|e| &e.role) {
            Some(Role::Window { window, .. }) => window.set_app_id(app_id.to_owned()),
            Some(Role::Layer(_) | Role::Lock(_)) => {
                tracing::debug!(?id, "set_app_id yalnızca pencerede");
            }
            None => {}
        }
    }

    /// En küçük mantıksal boyut; `None` ya da 0 eksen = sınır yok. Mevcut `max`'ı aşarsa
    /// `max` ona çıkar (protokol `max < min`'i bağlantıyı düşüren hatayla reddeder).
    /// Çift tamponlu: pencere zaten çizildiyse hemen commit edilir, yoksa ilk karede geçerli.
    pub fn set_min_size(&mut self, id: SurfaceId, size: Option<(u32, u32)>) {
        self.update_size_hints(id, "set_min_size", |h| h.set_min(size));
    }

    /// En büyük mantıksal boyut; `None` ya da 0 eksen = sınır yok. Mevcut `min`'in altındaysa
    /// `min` ona iner. Sabit boyutlu pencere: `min == max`.
    pub fn set_max_size(&mut self, id: SurfaceId, size: Option<(u32, u32)>) {
        self.update_size_hints(id, "set_max_size", |h| h.set_max(size));
    }

    fn update_size_hints(
        &mut self,
        id: SurfaceId,
        what: &str,
        change: impl FnOnce(&mut SizeHints) -> bool,
    ) {
        let in_draw = self.in_draw;
        let Some(entry) = self.surfaces.get_mut(&id) else {
            return;
        };
        let Role::Window { window, hints, .. } = &mut entry.role else {
            tracing::debug!(?id, "{what} yalnızca pencerede");
            return;
        };
        let before = *hints;
        if change(hints) {
            tracing::warn!(?id, min = ?hints.min, max = ?hints.max, "{what}: çelişen ipucu düzeltildi");
        }
        if *hints == before {
            return;
        }
        send_size_hints(window, *hints);
        // draw içindeyse swap'ın commit'i uygular; tampon yoksa ilk swap.
        if entry.has_buffer && !in_draw {
            entry.role.wl_surface().commit();
        }
    }

    /// Yaşayan `xdg_toplevel` pencere sayısı. Runtime son pencere kapanınca çıkmaz;
    /// uygulama `close_requested`/kendi kapatmasında bununla karar verir.
    pub fn window_count(&self) -> usize {
        self.surfaces
            .values()
            .filter(|s| matches!(s.role, Role::Window { .. }))
            .count()
    }

    /// Son işaretçi basma serial'ıyla etkileşimli taşıma (başlık çubuğundaki `Press`
    /// işleyicisinden çağrılır). Serial yoksa yok sayılır.
    pub fn begin_move(&mut self, id: SurfaceId) {
        if let (Some(window), Some((seat, serial))) = (self.window(id), self.last_press.as_ref()) {
            window.move_(seat, *serial);
        }
    }

    /// Son işaretçi basma serial'ıyla etkileşimli yeniden boyutlandırma.
    pub fn begin_resize(&mut self, id: SurfaceId, edge: ResizeEdge) {
        if let (Some(window), Some((seat, serial))) = (self.window(id), self.last_press.as_ref()) {
            window.resize(seat, *serial, to_xdg_edge(edge));
        }
    }

    pub fn set_maximized(&mut self, id: SurfaceId, maximized: bool) {
        if let Some(window) = self.window(id) {
            if maximized {
                window.set_maximized()
            } else {
                window.unset_maximized()
            }
        }
    }

    pub fn set_minimized(&mut self, id: SurfaceId) {
        if let Some(window) = self.window(id) {
            window.set_minimized();
        }
    }

    /// Son configure'daki pencere durumu; pencere değilse `None`.
    pub fn window_state(&self, id: SurfaceId) -> Option<WindowState> {
        match self.surfaces.get(&id).map(|e| &e.role) {
            Some(Role::Window { state, .. }) => Some(*state),
            _ => None,
        }
    }

    // ---------------------------------------------------------------------------------------
    // Oturum kilidi (ext-session-lock-v1). Güvenlik kuralları: docs/ARCHITECTURE.md.
    // ---------------------------------------------------------------------------------------

    /// Kilit ister. Başarı = istek gönderildi; kilit ANCAK `App::session_locked` ile kurulur.
    pub fn lock_session(&mut self) -> Result<(), LockError> {
        if !self.structural_change("lock_session") {
            return Err(LockError::InDraw);
        }
        if self.lock.is_some() {
            return Err(LockError::AlreadyActive);
        }
        let session = self
            .protocols
            .session_lock
            .lock(&self.qh)
            .map_err(|_| LockError::Unsupported)?;
        self.lock = Some(ActiveLock {
            session,
            state: LockState::Pending,
        });
        // Kilit varken pano yok: süren yapıştırmalar içerik vermeden biter.
        self.cancel_pastes_for_lock();
        // Kilit varken yakalama yok: yürüyen işler kare vermeden biter.
        self.capture_abort_for_lock();
        let outputs: Vec<OutputId> = self.outputs.keys().copied().collect();
        for output in outputs {
            self.create_lock_surface(output);
        }
        tracing::info!(
            outputs = self.outputs.len(),
            "oturum kilidi istendi (Pending)"
        );
        Ok(())
    }

    /// Yalnızca `Locked` iken: `unlock_and_destroy` gönderilir ve compositor'ın işlediği
    /// (`wl_display.sync`) beklenir; ardından `quit` güvenlidir. Kilit yüzeyleri kaldırılır ve
    /// `lock_finished(Unlocked)` gelir.
    pub fn unlock_session(&mut self) -> Result<(), LockError> {
        if !self.structural_change("unlock_session") {
            return Err(LockError::InDraw);
        }
        if self.lock.as_ref().map(|l| l.state) != Some(LockState::Locked) {
            tracing::warn!(state = ?self.lock_state(), "unlock_session: oturum kilitli değil");
            return Err(LockError::NotLocked);
        }
        let Some(lock) = self.lock.take() else {
            return Err(LockError::NotLocked);
        };
        lock.session.unlock();
        if let Err(e) = sync_roundtrip(&self.conn) {
            // Bağlantı koptuysa compositor kilidi korur (güvenli taraf); yine de temizle.
            tracing::error!("kilit açma senkronlanamadı: {e}");
        }
        drop(lock);
        self.remove_lock_surfaces();
        self.pending
            .push(RuntimeEvent::LockFinished(LockEnd::Unlocked));
        tracing::info!("oturum kilidi açıldı (Unlocked)");
        Ok(())
    }

    pub fn lock_state(&self) -> LockState {
        self.lock.as_ref().map_or(LockState::Unlocked, |l| l.state)
    }

    /// Kilit istendiyse/kurulduysa klavye ve IME olayları yalnızca kilit yüzeylerine gider
    /// (compositor zaten böyle yapar; savunma derinliği).
    pub(crate) fn input_allowed(&self, id: SurfaceId) -> bool {
        self.lock.is_none() || self.is_lock_surface(id)
    }

    pub fn is_lock_surface(&self, id: SurfaceId) -> bool {
        matches!(self.surfaces.get(&id).map(|e| &e.role), Some(Role::Lock(_)))
    }

    /// `output` için kilit yüzeyi yaratır ve uygulamaya `lock_surface_added` kuyruğa alır.
    /// Çıkışın zaten kilit yüzeyi varsa (ör. `output_added` içinden `lock_session`) yeniden
    /// yaratmaz: aynı çıkışa ikinci yüzey protokol hatasıdır (`duplicate_output`).
    pub(crate) fn create_lock_surface(&mut self, output: OutputId) -> Option<SurfaceId> {
        self.lock.as_ref()?;
        if self
            .surfaces
            .values()
            .any(|s| s.output == Some(output) && matches!(s.role, Role::Lock(_)))
        {
            return None;
        }
        let wl_output = self.outputs.get(&output)?.wl.clone();
        let scale = self.outputs.get(&output).map_or(1, |o| o.scale).max(1) as f32;
        let id = SurfaceId(self.next_id());
        let p = &self.protocols;
        let wl_surface = p.compositor.create_surface_with_data(&self.qh, None, 1, id);
        // Savunma derinliği (kural 2): tüm yüzey opak; compositor alttakini çizmez. Yüzey
        // dışındaki kısım protokol gereği yok sayılır; ilk swap'ın commit'iyle uygulanır.
        match Region::new(&p.compositor) {
            Ok(region) => {
                region.add(0, 0, i32::MAX, i32::MAX);
                wl_surface.set_opaque_region(Some(region.wl_region()));
            }
            Err(e) => tracing::warn!("kilit yüzeyi için wl_region yaratılamadı: {e}"),
        }
        let fractional = p.fractional.as_ref().map(|(manager, viewporter)| {
            (
                manager.get_fractional_scale(&wl_surface, &self.qh, FractionalData(id)),
                viewporter.get_viewport(&wl_surface, &self.qh, NoEvents),
            )
        });
        let surface = self
            .lock
            .as_ref()?
            .session
            .create_lock_surface(wl_surface, &wl_output, &self.qh);
        self.surfaces.insert(
            id,
            SurfaceEntry {
                output: Some(output),
                follows_enter: false,
                requested: (1, 1),
                size: (1, 1),
                scale,
                configured: false,
                wants_frame: true,
                callback_pending: false,
                has_buffer: false,
                presentation: Presentation::default(),
                scale_dirty: true,
                fractional,
                placement: None,
                damage: Default::default(),
                gpu: None,
                shm: None,
                role: Role::Lock(surface),
            },
        );
        self.pending
            .push(RuntimeEvent::LockSurfaceAdded(id, output));
        // Kilit yüzeyi ağaca yalnız uygulamanın açık izniyle girer (ADR-0011 "Kilit ekranı"); yüzey
        // tüm çıkışı kaplar.
        #[cfg(feature = "a11y")]
        if self.a11y_lock_surfaces {
            self.a11y.surface_added(id, Some(crate::a11y::LayerPlace { anchor: crate::Anchor::ALL, margin: nuxut_core::Insets::ZERO }));
        }
        Some(id)
    }

    /// Tüm kilit yüzeylerini kaldırır (unlock / finished sonrası).
    pub(crate) fn remove_lock_surfaces(&mut self) {
        let ids: Vec<SurfaceId> = self
            .surfaces
            .iter()
            .filter(|(_, s)| matches!(s.role, Role::Lock(_)))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.focus_grab_forget_surface(id);
            if let Some(entry) = self.surfaces.remove(&id) {
                self.graveyard.push(entry);
            }
            self.ime.forget(id);
            self.cursor.tracker.forget(id);
            self.touch.surface_removed(id);
        }
    }

    pub(crate) fn lock_surface_id_of(&self, surface: &SessionLockSurface) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .find(
                |(_, s)| matches!(&s.role, Role::Lock(l) if l.wl_surface() == surface.wl_surface()),
            )
            .map(|(id, _)| *id)
    }

    /// Bir sonraki kareyi ister. Compositor'ın frame callback'iyle hizalanır; aynı karede
    /// birden çok çağrı tek kare üretir.
    pub fn request_frame(&mut self, id: SurfaceId) {
        if let Some(entry) = self.surfaces.get_mut(&id) {
            entry.wants_frame = true;
        }
    }

    /// Son configure'daki mantıksal boyut; henüz configure gelmediyse `None`.
    pub fn surface_size(&self, id: SurfaceId) -> Option<Size> {
        self.surfaces
            .get(&id)
            .filter(|s| s.configured)
            .map(|s| s.logical_size())
    }

    /// Yüzeyin çıkışı: açık çıkışlı layer ve kilit yüzeyinde yaratıldığı çıkış; çıkışı
    /// compositor'a bırakılmış layer yüzeyinde `wl_surface.enter`le öğrenilen çıkış (o zamana
    /// dek `None`); pencerede `None`.
    pub fn surface_output(&self, id: SurfaceId) -> Option<OutputId> {
        self.surfaces.get(&id).and_then(|s| s.output)
    }

    /// Yüzeyin etkin ölçeği (kesirli olabilir).
    pub fn surface_scale(&self, id: SurfaceId) -> Option<f32> {
        self.surfaces.get(&id).map(|s| s.scale)
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Çizim dışı metin ölçümü/şekillendirme. **`draw` içinde çağrılamaz** (sistem o sırada
    /// renderer'dadır); çizimde `painter.text_system()` kullanılır.
    pub fn text(&mut self) -> &mut TextSystem {
        assert!(
            !self.in_draw,
            "Context::text draw içinde çağrılamaz; düzeni olay işleyicisinde üret"
        );
        if self.text.is_none() {
            let text = self.take_text();
            self.text = Some(text);
        }
        self.text.get_or_insert_with(TextSystem::new)
    }

    /// Metin sistemini sahiplenir: hazırsa onu, arka planda yükleniyorsa bekleyip onu, yoksa
    /// yenisini verir. Çağıran işi bitince `self.text`'e geri koyar.
    pub(crate) fn take_text(&mut self) -> TextSystem {
        if let Some(text) = self.text.take() {
            return text;
        }
        match self.text_loading.take().map(std::thread::JoinHandle::join) {
            Some(Ok(text)) => text,
            Some(Err(_)) => {
                tracing::warn!("font yükleme thread'i çöktü; yeniden yükleniyor");
                TextSystem::new()
            }
            None => TextSystem::new(),
        }
    }

    pub fn sender(&self) -> MessageSender<A::Message> {
        self.sender.clone()
    }

    /// `after` sonra bir kez `App::timer(id)` çağrılır.
    pub fn set_timer(&mut self, after: Duration) -> TimerId {
        let id = TimerId(self.next_id());
        let result = self
            .handle
            .insert_source(Timer::from_duration(after), move |_, _, state| {
                if state.cx.timers.remove(&id).is_some() {
                    state.app.timer(&mut state.cx, id);
                }
                TimeoutAction::Drop
            });
        match result {
            Ok(token) => {
                self.timers.insert(id, token);
            }
            Err(e) => tracing::error!("timer kurulamadı: {e}"),
        }
        id
    }

    /// Henüz tetiklenmemiş timer'ı iptal eder; tetiklenmiş/bilinmeyen kimlik yok sayılır.
    pub fn cancel_timer(&mut self, id: TimerId) {
        if let Some(token) = self.timers.remove(&id) {
            self.handle.remove(token);
        }
    }

    /// `<config_dir>/<name>.toml` değişince `App::config_changed(name)` çağrılır.
    pub fn watch_config(&mut self, name: &str) {
        if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
            tracing::warn!(name, "geçersiz config adı; izlenmiyor");
            return;
        }
        if !self.watched.iter().any(|n| n == name) {
            self.watched.push(name.to_owned());
            self.install_watcher();
        }
    }

    /// Yüzeyin imleç şeklini ayarlar (kurallar DESIGN.md "İmleç"). Yüzey başına saklanır:
    /// işaretçi o yüzeydeyse hemen uygulanır (aynı şekil tekrar istenirse çağrı yapılmaz),
    /// değilse yüzeye girildiğinde uygulanır.
    pub fn set_cursor(&mut self, id: SurfaceId, shape: CursorShape) {
        if !self.surfaces.contains_key(&id) {
            return;
        }
        if let Some(shape) = self.cursor.tracker.set(id, shape) {
            self.cursor.apply(&self.conn, shape);
        }
    }
    /// Döngüden çıkılır; olay turu bitince `run` döner.
    pub fn quit(&mut self) {
        self.quit = true;
    }

    /// İzlenen ad listesiyle yeni bir inotify izleyicisi kurar (eskisini kaldırır).
    pub(crate) fn install_watcher(&mut self) {
        if let Some((_, token)) = self.watcher.take() {
            self.handle.remove(token);
        }
        let Some(dir) = nuxut_config::config_dir() else {
            tracing::warn!("config dizini bulunamadı; config izlenmiyor");
            return;
        };
        let files: Vec<String> = self
            .watched
            .iter()
            .map(|name| format!("{name}.toml"))
            .collect();
        let names: Vec<&str> = files.iter().map(String::as_str).collect();
        let watcher = match ConfigWatcher::new(&dir, &names) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("config dizini izlenemiyor: {e}");
                return;
            }
        };
        // İzleyici Context'te kalır; döngüye aynı inotify örneğinin kopya tanımlayıcısı verilir.
        let fd: OwnedFd = match watcher.fd().try_clone_to_owned() {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!("inotify fd kopyalanamadı: {e}");
                return;
            }
        };
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        match self.handle.insert_source(source, |_, _, state| {
            state.config_event();
            Ok(PostAction::Continue)
        }) {
            Ok(token) => {
                tracing::debug!(names = ?self.watched, "config izleniyor");
                self.watcher = Some((watcher, token));
            }
            Err(e) => tracing::warn!("config izleyicisi döngüye eklenemedi: {e}"),
        }
    }

    /// Üretilmiş paleti (`$XDG_STATE_HOME/nuxut/palette.toml`) izler. Ayrı dizin olduğu için
    /// ayrı bir inotify; yalnızca bir kez, başlangıçta kurulur.
    pub(crate) fn install_palette_watcher(&mut self) {
        let Some(path) = nuxut_theme::palette_path() else {
            tracing::debug!("state dizini yok; palet izlenmiyor");
            return;
        };
        let Some(dir) = path.parent() else { return };
        let watcher = match ConfigWatcher::new(dir, &[nuxut_theme::PALETTE_FILE]) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("palet dizini izlenemiyor: {e}");
                return;
            }
        };
        let fd: OwnedFd = match watcher.fd().try_clone_to_owned() {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!("inotify fd kopyalanamadı: {e}");
                return;
            }
        };
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        match self.handle.insert_source(source, |_, _, state| {
            state.palette_event();
            Ok(PostAction::Continue)
        }) {
            Ok(token) => {
                tracing::debug!("palet izleniyor");
                self.palette_watcher = Some((watcher, token));
            }
            Err(e) => tracing::warn!("palet izleyicisi döngüye eklenemedi: {e}"),
        }
    }

    /// Bekleyen palet olaylarını tüketir; `palette.toml` değiştiyse `true`.
    pub(crate) fn palette_changed(&mut self) -> bool {
        self.palette_watcher
            .as_mut()
            .is_some_and(|(watcher, _)| !watcher.changed_files().is_empty())
    }

    /// Kullanıcı çeviri dizinini izler (ADR-0010; kayıtlı domain'ler). Başlangıçta bir kez:
    /// kayıt kancası ([`nuxut_i18n::set_register_hook`]) döngüye bir ping bağlar — çalışma
    /// başladıktan sonra kaydedilen domain de izlenir (ilk kayıtta izleyici o an kurulur).
    /// Veri dizini yoksa izleyici kurulmaz.
    pub(crate) fn install_i18n_watcher(&mut self) {
        match calloop::ping::make_ping() {
            Ok((ping, source)) => match self.handle.insert_source(source, |_, _, state| state.i18n_registered()) {
                Ok(_) => nuxut_i18n::set_register_hook(Some(std::sync::Arc::new(move || ping.ping()))),
                Err(e) => tracing::warn!("çeviri kayıt pingi döngüye eklenemedi: {e}"),
            },
            Err(e) => tracing::warn!("çeviri kayıt pingi kurulamadı: {e}"),
        }
        self.sync_i18n_watcher();
    }

    /// İzlenen domain'leri kayıtla eşitler: izleyici yoksa (ve kayıt varsa) kurar, varsa
    /// domain kümesini günceller.
    pub(crate) fn sync_i18n_watcher(&mut self) {
        let sources = nuxut_i18n::registered();
        if sources.is_empty() {
            return;
        }
        let domains: Vec<&str> = sources.iter().map(|s| s.domain).collect();
        if let Some(w) = self.i18n_watch.as_mut() {
            if w.watcher.set_domains(&domains) {
                tracing::debug!(domains = domains.len(), "çeviri izleme domain'leri güncellendi");
            }
            return;
        }
        let Some(root) = nuxut_i18n::default_user_dir() else {
            tracing::debug!("veri dizini yok; çeviri dizini izlenmiyor");
            return;
        };
        let watcher = match nuxut_i18n::watch::UserDirWatcher::new(&root, &domains) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("çeviri dizini izlenemiyor: {e}");
                return;
            }
        };
        let fd: OwnedFd = match watcher.fd().try_clone_to_owned() {
            Ok(fd) => fd,
            Err(e) => {
                tracing::warn!("inotify fd kopyalanamadı: {e}");
                return;
            }
        };
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        match self.handle.insert_source(source, |_, _, state| {
            state.i18n_event();
            Ok(PostAction::Continue)
        }) {
            Ok(token) => {
                tracing::debug!(domains = domains.len(), "çeviri dizini izleniyor");
                self.i18n_watch = Some(I18nWatch {
                    watcher,
                    _token: token,
                    debounce: nuxut_i18n::watch::Debounce::default(),
                    timer: false,
                });
            }
            Err(e) => tracing::warn!("çeviri izleyicisi döngüye eklenemedi: {e}"),
        }
    }

    /// Bekleyen çeviri dizini olaylarını tüketir; ilgiliyse birleştirme zamanlayıcısını kurar
    /// (zaten kuruluysa son tarih zamanlayıcı uyandığında uzatılır).
    pub(crate) fn i18n_events(&mut self) {
        let Some(w) = self.i18n_watch.as_mut() else {
            return;
        };
        if !w.watcher.process_events() {
            return;
        }
        let deadline = w.debounce.event(Instant::now());
        if w.timer {
            return;
        }
        let result = self
            .handle
            .insert_source(Timer::from_deadline(deadline), |_, _, state| {
                state.i18n_timer()
            });
        match result {
            Ok(_) => w.timer = true,
            Err(e) => tracing::warn!("çeviri birleştirme zamanlayıcısı kurulamadı: {e}"),
        }
    }

    /// Birleştirme zamanlayıcısı uyandı. Zamanı geldiyse
    /// `Ok(katalog girdisi değişti mi)` (zamanlayıcı düşer); erkense `Err(yeni son tarih)`.
    pub(crate) fn i18n_poll(&mut self) -> Result<bool, Instant> {
        let Some(w) = self.i18n_watch.as_mut() else {
            return Ok(false);
        };
        match w.debounce.poll(Instant::now()) {
            nuxut_i18n::watch::DebouncePoll::Wait(deadline) => Err(deadline),
            nuxut_i18n::watch::DebouncePoll::Idle => {
                w.timer = false;
                Ok(false)
            }
            nuxut_i18n::watch::DebouncePoll::Fire => {
                w.timer = false;
                Ok(w.watcher.take_change())
            }
        }
    }

    /// Bekleyen değişiklikleri okur; değişen dosyaların adlarını (uzantısız) döndürür.
    pub(crate) fn changed_configs(&mut self) -> Vec<String> {
        let Some((watcher, _)) = self.watcher.as_mut() else {
            return Vec::new();
        };
        watcher
            .changed_files()
            .into_iter()
            .filter_map(|path| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
            .collect()
    }

    pub(crate) fn window_id_of(&self, window: &Window) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .find(|(_, s)| matches!(&s.role, Role::Window { window: w, .. } if w == window))
            .map(|(id, _)| *id)
    }
    pub(crate) fn surface_id_of(&self, layer: &wlr_layer::LayerSurface) -> Option<SurfaceId> {
        self.surfaces
            .iter()
            .find(|(_, s)| matches!(&s.role, Role::Layer(l) if l == layer))
            .map(|(id, _)| *id)
    }

    pub(crate) fn output_id_of(&self, output: &WlOutput) -> Option<OutputId> {
        self.outputs
            .iter()
            .find(|(_, o)| o.wl == *output)
            .map(|(id, _)| *id)
    }
}

/// Kullanıcı çeviri dizini izleme durumu (bkz. `nuxut_i18n::watch`).
#[derive(Debug)]
pub(crate) struct I18nWatch {
    watcher: nuxut_i18n::watch::UserDirWatcher,
    /// Kaynak döngüde kalır (süreç boyu); kaldırılmaz.
    _token: RegistrationToken,
    debounce: nuxut_i18n::watch::Debounce,
    /// Birleştirme zamanlayıcısı döngüde mi (boşta zamanlayıcı yok).
    timer: bool,
}

/// Karşılıklı anchor'u olmayan eksende 0 → 1.
fn sanitize_size(size: (u32, u32), anchor: Anchor) -> (u32, u32) {
    let w = if size.0 == 0 && !anchor.contains(Anchor::LEFT | Anchor::RIGHT) {
        1
    } else {
        size.0
    };
    let h = if size.1 == 0 && !anchor.contains(Anchor::TOP | Anchor::BOTTOM) {
        1
    } else {
        size.1
    };
    (w, h)
}

fn to_wlr_layer(layer: Layer) -> wlr_layer::Layer {
    match layer {
        Layer::Background => wlr_layer::Layer::Background,
        Layer::Bottom => wlr_layer::Layer::Bottom,
        Layer::Top => wlr_layer::Layer::Top,
        Layer::Overlay => wlr_layer::Layer::Overlay,
    }
}

/// Çift tamponlu en küçük/en büyük boyut isteklerini gönderir (değerler önceden doğrulanmış).
fn send_size_hints(window: &Window, hints: SizeHints) {
    let ((min_w, min_h), (max_w, max_h)) = hints.wire();
    let toplevel = window.xdg_toplevel();
    toplevel.set_min_size(min_w, min_h);
    toplevel.set_max_size(max_w, max_h);
}

fn to_xdg_edge(edge: ResizeEdge) -> xdg_toplevel::ResizeEdge {
    match edge {
        ResizeEdge::Top => xdg_toplevel::ResizeEdge::Top,
        ResizeEdge::Bottom => xdg_toplevel::ResizeEdge::Bottom,
        ResizeEdge::Left => xdg_toplevel::ResizeEdge::Left,
        ResizeEdge::Right => xdg_toplevel::ResizeEdge::Right,
        ResizeEdge::TopLeft => xdg_toplevel::ResizeEdge::TopLeft,
        ResizeEdge::TopRight => xdg_toplevel::ResizeEdge::TopRight,
        ResizeEdge::BottomLeft => xdg_toplevel::ResizeEdge::BottomLeft,
        ResizeEdge::BottomRight => xdg_toplevel::ResizeEdge::BottomRight,
    }
}
fn to_wlr_keyboard(mode: KeyboardMode) -> wlr_layer::KeyboardInteractivity {
    match mode {
        KeyboardMode::None => wlr_layer::KeyboardInteractivity::None,
        KeyboardMode::OnDemand => wlr_layer::KeyboardInteractivity::OnDemand,
        KeyboardMode::Exclusive => wlr_layer::KeyboardInteractivity::Exclusive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_is_send_static_clone() {
        fn assert_bounds<T: Clone + Send + 'static>() {}
        assert_bounds::<MessageSender<String>>();
    }

    /// Font taraması `nuxut-fonts` thread'inde yapılır; TextSystem Send olmazsa derlenmez.
    #[test]
    fn text_system_is_send() {
        fn assert_send<T: Send + 'static>() {}
        assert_send::<TextSystem>();
    }

    #[test]
    fn zero_size_needs_opposite_anchors() {
        let top_strip = Anchor::TOP | Anchor::LEFT | Anchor::RIGHT;
        assert_eq!(sanitize_size((0, 8), top_strip), (0, 8));
        assert_eq!(sanitize_size((0, 0), top_strip), (0, 1));
        assert_eq!(sanitize_size((0, 0), Anchor::ALL), (0, 0));
        assert_eq!(sanitize_size((0, 0), Anchor::NONE), (1, 1));
    }
}
