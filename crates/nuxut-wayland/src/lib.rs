//! Nuxut Wayland çalışma zamanı.
//!
//! `smithay-client-toolkit` + `calloop` üzerinde layer-shell yüzeyleri, tek EGL/GLES 3.0
//! context'i, girdi ve olay döngüsü. Uygulama [`App`]'i uygular ve [`run`]'a verir; runtime
//! çıkışları bildirir, yüzeyleri configure eder ve kareleri compositor'ın frame callback'leriyle
//! hizalayarak yalnızca istendiğinde çizer (boşta sıfır iş).
//!
//! Sözleşme: `docs/ARCHITECTURE.md` → "Çalışma zamanı sözleşmesi".
//!
//! Hata ayıklama: `NUXUT_DEBUG_SURFACES=1` her yüzeyin sınırını eflatun çerçeveyle boyar.

#[cfg(feature = "a11y")]
mod a11y;
mod activation;
mod capture;
mod capture_pixels;
mod clipboard;
mod color_transform;
mod context;
mod cursor;
mod data_control;
mod egl;
mod focus_grab;
mod foreign;
mod idle;
mod ime;
pub mod ipc;
mod keyfocus;
mod keysym;
mod profile;
mod shm;
mod state;
mod surface;
mod touch;
mod toplevels;
mod types;
mod virtual_keyboard;

use std::time::Instant;

use calloop::EventLoop;
use calloop::signals::{Signal, Signals};
use calloop_wayland_source::WaylandSource;
use nuxut_core::Size;
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::data_device_manager::DataDeviceManagerState;
use smithay_client_toolkit::output::OutputState;
use smithay_client_toolkit::primary_selection::PrimarySelectionManagerState;
use smithay_client_toolkit::registry::RegistryState;
use smithay_client_toolkit::seat::SeatState;
use smithay_client_toolkit::shell::wlr_layer::LayerShell;
use smithay_client_toolkit::shell::xdg::XdgShell;
use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;
use wayland_protocols::wp::fractional_scale::v1::client::wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1;
use wayland_protocols::wp::viewporter::client::wp_viewporter::WpViewporter;

pub use activation::{ActivationRequestId, ActivationToken};
pub use capture::{
    CaptureBackend, CaptureError, CaptureId, CaptureOptions, CaptureSource, CaptureSupport,
    CapturedImage, MAX_BUFFER_BYTES as MAX_CAPTURE_BUFFER_BYTES,
};
pub use clipboard::PasteRequestId;
pub use color_transform::{ColorMatrix, ColorTransformError, ColorTransformState};
pub use context::{Closed, Context, MessageSender};
pub use data_control::{
    DataControlError, DataControlEvent, DataControlHandle, DataControlProtocol, DataOfferId,
    DataReadError, DataReadId, MAX_MIME_LEN, MAX_MIMES, OWNER_MIME_PREFIX, PASSWORD_HINT_MIME,
    SetSelectionError, is_sensitive_mime, owner_mime,
};
pub use focus_grab::FocusGrabId;
pub use foreign::{ExportRequestId, ToplevelHandle};
pub use idle::{IdleInhibitors, IdleSupport, IdleWatchError, IdleWatchId};
pub use ime::{ImeEvent, SurroundingText, TextInputState, TextPurpose};
pub use nuxut_core::clipboard::{ClipboardText, CopyError, PasteError, Selection};
/// Girdi tipleri `nuxut_core::input`'ta (nuxut-ui platform bilmeden tüketir).
pub use nuxut_core::input::{
    Button, CursorShape, Key, KeyEvent, KeyState, Modifiers, PointerEvent, PointerKind, PointerSource,
    SecretText, TOUCH_SLOP, TOUCH_TAP_DELAY, TouchEvent, TouchId, TouchPhase,
};
pub use nuxut_render::{Painter, TextSystem};
pub use nuxut_theme::Theme;
pub use virtual_keyboard::{KEY_MAX, MAX_KEYMAP_LEN, MAX_PRESSED, VirtualKeyboardError};
pub use toplevels::{ToplevelEvent, ToplevelId, ToplevelInfo, ToplevelListError};
pub use types::{
    Anchor, KeyboardMode, Layer, LayerSurfaceSpec, LockEnd, LockError, LockState, OutputId,
    OutputInfo, OutputTarget, ResizeEdge, RunError, SurfaceId, TimerId, WindowSpec, WindowState,
};

/// `DataControlHandle::read` sonucu bu tamponla gelir (düşerken sıfırlanır).
pub use zeroize::Zeroizing;

use context::Protocols;
use state::{Gfx, NoEvents, State};

/// Bir Nuxut uygulaması. Tüm çağrılar olay döngüsü thread'inde yapılır.
#[allow(unused_variables)]
pub trait App: Sized + 'static {
    /// Arka plan thread'lerinden gelen mesajlar (IPC olayları vb.).
    type Message: Send + 'static;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo);
    fn output_changed(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {}
    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId);

    /// Bir yüzey ilk kez veya yeniden boyutlandı (configure) ya da ölçeği değişti.
    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        scale: f32,
    ) {
    }

    /// Yüzeyin ilk tamponu compositor tarafından sunuldu: ilk tamponlu commit'e bağlı frame
    /// callback'i `done` oldu (commit değil, gerçek sunum). Yüzey başına bir kez; yeniden
    /// yaratılan yüzey için yeniden. Yok edilmiş yüzey için gelmez; ek kare/uyanma istemez.
    /// Layer, pencere ve kilit yüzeylerinde aynı. Kilit ekranı "gösterildi" durumunu
    /// (ör. LockedHint) buna bağlar. Görünmeyen yüzeyde compositor callback'i geciktirebilir.
    fn surface_presented(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {}

    /// Çıkışı compositor'a bırakılmış layer yüzeyinin (`OutputTarget::Compositor`) çıkışı
    /// öğrenildi (`wl_surface.enter`: `Some`) ya da yüzey çıkışından ayrıldı (`leave`: `None`).
    /// Açık çıkışlı yüzeyler, pencereler ve kilit yüzeyleri için gelmez. Ölçek ayrıca
    /// `surface_resized` ile gelir. Çıkış kaldırılınca compositor yüzeyi kapatır (sessizce
    /// kaldırılır; uygulama `output_removed` alır) — açık çıkışlı yüzeylerle aynı.
    fn surface_output_changed(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        output: Option<OutputId>,
    ) {
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent);
    /// Ham dokunma olayı (dokunmatik ekran, tüm temaslar). `Down`'da `true` dönmek o temasın
    /// dizisini sahiplenir: işaretçi öykünmesi yapılmaz (çoklu dokunma bilen yüzeyler, ör. ekran
    /// klavyesi). Varsayılan `false`: sahiplenilmeyen ilk temas `PointerSource::Touch` kaynaklı
    /// işaretçi olaylarıyla `pointer`'a gelir. Dönüş değeri yalnızca `Down`'da okunur.
    fn touch(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: TouchEvent) -> bool {
        false
    }
    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {}
    /// Yüzey klavye odağını aldı (`true`) ya da kaybetti (`false`). Panel dışına (bir
    /// pencereye) tıklanınca odak kaybıyla kapanan paneller bunu kullanır. Yok edilmiş yüzey
    /// için çağrılmaz.
    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {}

    /// Compositor pencereyi kapatmak istedi (ör. `killactive`). Varsayılan: yüzeyi yok et.
    /// Yalnızca bu pencere kapanır; runtime son pencere kapanınca ÇIKMAZ — uygulama
    /// `if cx.window_count() == 0 { cx.quit() }` ile kendisi karar verir.
    fn close_requested(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        cx.destroy_surface(surface);
    }
    /// IME düzenlemesi (bir `done` içinde sırayla: silme → commit → preedit). Yalnızca
    /// `cx.set_text_input(surface, Some(..))` ile metin girişi açık yüzeylere gelir.
    fn ime(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: ImeEvent) {}
    /// Etkin klavye düzeni ya da düzen listesi değişti (`cx.keyboard_layout()`).
    fn keyboard_layout_changed(&mut self, cx: &mut Context<Self>) {}
    /// Değiştiriciler ya da kilit durumları (Caps/Num Lock) değişti; klavye odağındaki yüzey
    /// için. Her `KeyEvent.modifiers` da güncel değeri taşır.
    fn modifiers_changed(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        modifiers: Modifiers,
    ) {
    }

    /// Kilit yüzeyi yaratıldı (kilit isteğinde her çıkış için; kilitliyken eklenen çıkış için
    /// de). Aynı draw/pointer/keyboard yolu; tampon OPAK çizilmeli (docs: Oturum kilidi).
    fn lock_surface_added(&mut self, cx: &mut Context<Self>, surface: SurfaceId, output: OutputId) {
    }
    /// Compositor `locked` gönderdi: oturum ancak şimdi kilitli sayılır.
    fn session_locked(&mut self, cx: &mut Context<Self>) {}
    /// Kilit kurulmadı ya da bitti; kilit yüzeyleri kaldırıldı.
    fn lock_finished(&mut self, cx: &mut Context<Self>, end: LockEnd) {}

    /// `cx.request_activation_token` sonucu; her istek için tam bir kez. `None`: protokol yok,
    /// yüzey yok ya da oturum kilidi etkin. Belirteç loglanmamalı.
    fn activation_token(
        &mut self,
        cx: &mut Context<Self>,
        id: ActivationRequestId,
        token: Option<ActivationToken>,
    ) {
    }

    /// `cx.export_toplevel` sonucu; her istek için tam bir kez. `None`: protokol yok, yüzey yok,
    /// pencere değil ya da yüzey tanıtıcı gelmeden kaldırıldı. Tanıtıcı loglanmamalı.
    fn toplevel_exported(
        &mut self,
        cx: &mut Context<Self>,
        id: ExportRequestId,
        surface: SurfaceId,
        handle: Option<ToplevelHandle>,
    ) {
    }

    /// `cx.request_paste` sonucu; her istek için tam bir kez. Metin loglanmamalı.
    fn paste(
        &mut self,
        cx: &mut Context<Self>,
        id: PasteRequestId,
        result: Result<ClipboardText, PasteError>,
    ) {
    }

    /// Compositor bir odak kapmayı bitirdi (`cx.focus_grab`; çoğunlukla beyaz liste dışına
    /// tıklama ya da başka bir grab). Grab artık etkisiz, beyaz listesi boş; kimlik geçerli
    /// (`focus_grab_set_surfaces` ile yeniden kurulur ya da `focus_grab_release`). Uygulamanın
    /// kendi bıraktığı/boşalttığı grab için gelmez. Paneller burada kapanır.
    fn focus_grab_cleared(&mut self, cx: &mut Context<Self>, id: FocusGrabId) {}

    /// Renk dönüşümünü başka bir istemci (ör. kullanıcının hyprsunset'i) yönetiyor: son
    /// `set_color_transforms` compositor'da etkisiz kaldı, durum `ColorTransformState::Blocked`.
    /// Runtime kendiliğinden yeniden denemez; `color_transform_release` sonrası sonraki `set`
    /// yeniden dener. Protokol sürüm 1 ise gelmez.
    fn color_transform_blocked(&mut self, cx: &mut Context<Self>) {}

    /// Pencere listesi olayı (`cx.toplevels_start` sonrası). Ayrıntı `cx.toplevel(id)`;
    /// başlık/uygulama kimliği loglanmamalı.
    fn toplevel(&mut self, cx: &mut Context<Self>, event: ToplevelEvent) {}

    /// `cx.capture` sonucu; her istek için tam bir kez (`capture_cancel` edilmediyse).
    /// Pikseller kullanıcı içeriğidir: loglanmaz, diske yazılmaz.
    fn captured(
        &mut self,
        cx: &mut Context<Self>,
        id: CaptureId,
        result: Result<CapturedImage, CaptureError>,
    ) {
    }

    /// Boşta izlemesi (`cx.idle_watch`) durum değiştirdi: `true` süre doldu (compositor
    /// `idled`), `false` kullanıcı döndü (`resumed`). Yalnızca değişimde gelir; koltuk ya da
    /// protokol kaybolursa boşta olan izleme için `false` gelir. `idle_unwatch` sonrası gelmez.
    fn idle(&mut self, cx: &mut Context<Self>, id: IdleWatchId, idle: bool) {}

    /// Pano izleme olayı (`cx.data_control_start` sonrası; `Selection::Primary` yalnızca
    /// `cx.data_control_watch_primary(true)` ile). `SelectionOffered` yalnızca hâlâ güncel
    /// teklif için gelir. MIME listesi içerik değildir ama framework teklifi kendiliğinden
    /// okumaz: okuma yalnızca uygulama isteyince. `own = true` teklif bizim koyduğumuzdur.
    fn data_control(&mut self, cx: &mut Context<Self>, event: DataControlEvent) {}

    /// `cx.data_control_read` sonucu; her istek için tam bir kez. İçerik loglanmamalı; tampon
    /// düşerken sıfırlanır.
    fn data_control_read(
        &mut self,
        cx: &mut Context<Self>,
        id: DataReadId,
        result: Result<Zeroizing<Vec<u8>>, DataReadError>,
    ) {
    }

    /// Pencere durumu değişti (maximize/activated/tiled…); ilk configure'da da çağrılır.
    /// Boyut ayrıca `surface_resized` ile gelir.
    fn window_state_changed(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        state: WindowState,
    ) {
    }

    fn message(&mut self, cx: &mut Context<Self>, message: Self::Message) {}
    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {}

    /// framework.toml değişti; `cx.theme()` yeni tema. Yalnızca `[locale]` değiştiyse gelmez.
    fn theme_changed(&mut self, cx: &mut Context<Self>) {}
    /// framework.toml `[locale]` ya da kullanıcı çeviri dosyaları
    /// (`$XDG_DATA_HOME/nuxut/locales/<domain>/*.ftl`, 200 ms birleştirilmiş, içerik gerçekten
    /// değiştiyse) değişti (ADR-0010 `LocaleChanged`): süreç geneli katalog
    /// (`nuxut::i18n::current()`) zaten yenilendi; bileşenler metinlerini yeniden ister,
    /// yerleşim yenilenir (tek kare). Biçimler: `cx.theme().locale.formats` / `.hour_cycle`;
    /// saat/tarih locale'i `nuxut::i18n::time_locale()` (burada yeniden alınır). Yalnızca dil
    /// değiştiyse `theme_changed` gelmez.
    fn locale_changed(&mut self, cx: &mut Context<Self>) {}
    /// `cx.watch_config(name)` ile izlenen dosya değişti.
    fn config_changed(&mut self, cx: &mut Context<Self>, name: &str) {}

    /// Kare çizimi. `now` karedeki tüm animasyonlar için ortak saat. Animasyon sürüyorsa
    /// `cx.request_frame(surface)` çağrılır; çağrılmazsa yüzey bir sonraki olaya kadar uyur.
    /// Burada yüzey yaratılmaz/yok edilmez, input bölgesi değişmez, `cx.text()` çağrılmaz.
    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        now: Instant,
        painter: &mut Painter,
    );

    /// SIGTERM/SIGINT/SIGHUP geldi. Varsayılan `cx.quit()`; uygulama geciktirmek isterse
    /// kendisi sonra `quit` der. Çıkışta `run` normal döner ve `App` düşürülür (Drop çalışır).
    fn terminate(&mut self, cx: &mut Context<Self>) {
        cx.quit();
    }

    /// Erişilebilirlik ağacı (ADR-0011): yüzeyin görünür bileşenlerini `tree`'ye ekler
    /// (`button.access(tree, "save")`…; kök adı `tree.root().label(..)`). Yalnız `a11y`
    /// özelliğiyle ve bir yardımcı teknoloji (Orca) bu yüzeyin ağacını istemişken, çizimden
    /// SONRA çağrılır (yüzey çizildiyse, etkinleşmede ya da `cx.a11y_invalidate`; animasyonda en
    /// çok 100 ms'de bir). Kapalıyken hiç çağrılmaz: maliyet sıfır. Ağaç içeriği loglanmaz.
    fn accessibility(&mut self, cx: &mut Context<Self>, surface: SurfaceId, tree: &mut nuxut_access::AccessTree) {}

    /// Yardımcı teknolojinin eylem isteği (tıkla, odakla, artır…), ana thread'de. Metin yazan
    /// eylemler hiç gelmez (ADR-0011 karar 6). Uygulama isteği ilgili bileşenin olay yoluna
    /// çevirir (`toggle.access_action(..)`, `Focus` → `FocusScope::focus`) ve gerekiyorsa yeniden
    /// çizim ister; değişen durum bir sonraki çizimden sonra ağaca yansır.
    fn accessibility_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: nuxut_access::AccessRequest) {}

    /// Başka bir süreçten (`ipc::send_args`) gelen komut; boş satır → `args = []`. Yanıt
    /// hemen ya da zaman aşımına kadar `reply.send` ile verilir. Komutlar asla shell'e verilmez.
    fn command(&mut self, cx: &mut Context<Self>, args: Vec<String>, reply: ipc::Reply) {
        reply.send("error: komut desteklenmiyor");
    }
}

/// `run_with` seçenekleri.
#[derive(Debug, Default)]
pub struct RunOptions {
    /// Tek örnek IPC'si (`ipc::try_single_instance` → `Instance::Primary`); gelen komutlar
    /// `App::command`'a verilir.
    /// Yanıt penceresi `Listener::with_reply_timeout` ile (varsayılan `ipc::TIMEOUT`).
    pub ipc: Option<ipc::Listener>,
}

/// Varsayılan seçeneklerle [`run_with`].
pub fn run<A: App>(app: A) -> Result<(), RunError> {
    run_with(app, RunOptions::default())
}

/// Compositor'a bağlanır ve uygulama `cx.quit()` diyene ya da bağlantı kopana dek çalışır.
///
/// Sinyaller: SIGTERM/SIGINT/SIGHUP bu thread'de maskelenir ve signalfd ile `App::terminate`'e
/// gelir. Maske, `run` içinde Wayland/EGL'den (Mesa'nın thread'lerinden) önce kurulur; sonradan
/// yaratılan thread'ler maskeyi miras alır. `run`'dan **önce** yaratılmış thread'ler maskesizdir
/// ve sinyal onlara düşerse süreç varsayılan eylemle ölür: arka plan thread'lerini olay
/// işleyicilerinde (ör. ilk `output_added`) başlatın.
pub fn run_with<A: App>(app: A, options: RunOptions) -> Result<(), RunError> {
    let mut profile = profile::StartupProfile::new();
    let mut event_loop: EventLoop<'static, State<A>> =
        EventLoop::try_new().map_err(|e| RunError::EventLoop(e.to_string()))?;
    let handle = event_loop.handle();
    let signals = Signals::new(&[Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP])
        .map_err(|e| RunError::EventLoop(format!("sinyal kaynağı: {e}")))?;
    handle
        .insert_source(signals, |event, _, state: &mut State<A>| {
            tracing::info!(signal = ?event.signal(), "sonlandırma sinyali");
            state.app.terminate(&mut state.cx);
        })
        .map_err(|e| RunError::EventLoop(e.error.to_string()))?;
    let ipc_token = match options.ipc {
        Some(listener) => Some(
            ipc::serve(&handle, listener, |state: &mut State<A>, args, reply| {
                state.app.command(&mut state.cx, args, reply);
            })
            .map_err(|e| RunError::EventLoop(format!("IPC: {e}")))?,
        ),
        None => None,
    };

    profile.mark("döngü");
    let conn = Connection::connect_to_env().map_err(|e| RunError::Connect(e.to_string()))?;
    let (globals, queue) =
        registry_queue_init::<State<A>>(&conn).map_err(|e| RunError::Connect(e.to_string()))?;
    let qh = queue.handle();

    WaylandSource::new(conn.clone(), queue)
        .insert(handle.clone())
        .map_err(|e| RunError::EventLoop(e.error.to_string()))?;

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|_| RunError::MissingProtocol("wl_compositor"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|_| RunError::MissingProtocol("zwlr_layer_shell_v1"))?;
    let fractional = match (
        globals.bind::<WpFractionalScaleManagerV1, _, _>(&qh, 1..=1, NoEvents),
        globals.bind::<WpViewporter, _, _>(&qh, 1..=1, NoEvents),
    ) {
        (Ok(manager), Ok(viewporter)) => Some((manager, viewporter)),
        _ => {
            tracing::info!(
                "wp_fractional_scale_v1/wp_viewporter yok; tamsayı buffer scale kullanılacak"
            );
            None
        }
    };
    let shm = globals
        .bind::<wayland_client::protocol::wl_shm::WlShm, _, _>(&qh, 1..=1, NoEvents)
        .inspect_err(|_| tracing::warn!("wl_shm yok; imleç teması kullanılamaz"))
        .ok();
    let text_input_manager = globals
        .bind::<wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .inspect_err(|_| tracing::debug!("zwp_text_input_manager_v3 yok; IME devre dışı"))
        .ok();
    let xdg_shell = XdgShell::bind(&globals, &qh)
        .inspect_err(|_| tracing::info!("xdg_wm_base yok; create_window kullanılamaz"))
        .ok();
    let activation = globals
        .bind::<wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .inspect_err(|_| tracing::debug!("xdg_activation_v1 yok; etkinleştirme belirteci verilemez"))
        .ok();

    profile.mark("wayland+protokoller");
    let egl = Egl::new(&conn)?;
    profile.mark("egl");
    // Font taraması (~30–70 ms) arka planda; configure beklemesi ve Renderer::new ile çakışır.
    // EGL'den SONRA başlar: EGL'yle aynı anda başlatmak (Mesa/LLVM yüklemesiyle G/Ç ve CPU
    // yarışı) yük altında ölçümde uzun kuyruklar verdi. Sinyal maskesini miras alır. İlk draw
    // ya da ilk `cx.text()` bekler.
    let text_loading = std::thread::Builder::new()
        .name("nuxut-fonts".into())
        .spawn(|| {
            let t = Instant::now();
            let text = TextSystem::new();
            tracing::debug!(
                ms = t.elapsed().as_secs_f64() * 1000.0,
                "font taraması bitti"
            );
            text
        })
        .inspect_err(|e| tracing::debug!("font thread'i başlatılamadı: {e}"))
        .ok();
    let debug_surfaces = std::env::var_os("NUXUT_DEBUG_SURFACES").is_some_and(|v| v != "0");

    // Katmanlı tema (framework.toml + palette.toml); bozuk palette son geçerli palet korunur.
    let mut theme_loader = nuxut_theme::ThemeLoader::new();
    let theme = match theme_loader.load() {
        // Uyarıları nuxut-theme kendisi loglar.
        Ok(loaded) => loaded.value,
        Err(e) => {
            // Bozuk dosya: varsayılan tema ama birleşik görünüm kapalı (dış hedef yazılmaz).
            tracing::warn!(error = %e.log_summary(), "framework.toml yüklenemedi, varsayılan tema (birleşik görünüm kapalı)");
            Theme::broken_config()
        }
    };
    // Dil ayarı: uygulama `nuxut::i18n::init` ile aynı ayarı zaten kurduysa iş yok; kaynak
    // kaydedip init etmediyse katalog burada kurulur.
    nuxut_i18n::configure(&theme.locale);

    let (sender, channel) = calloop::channel::channel::<A::Message>();
    handle
        .insert_source(channel, |event, _, state: &mut State<A>| {
            if let calloop::channel::Event::Msg(message) = event {
                state.app.message(&mut state.cx, message);
            }
        })
        .map_err(|e| RunError::EventLoop(e.error.to_string()))?;

    // Erişilebilirlik (ADR-0011): at-spi yoksa ya da NUXUT_A11Y=0 ise kanal da kurulmaz.
    #[cfg(feature = "a11y")]
    let a11y_tx = if a11y::available() {
        let (tx, channel) = calloop::channel::channel::<a11y::A11yEvent>();
        handle
            .insert_source(channel, |event, _, state: &mut State<A>| {
                if let calloop::channel::Event::Msg(event) = event {
                    state.a11y_event(event);
                }
            })
            .map_err(|e| RunError::EventLoop(e.error.to_string()))?;
        Some(tx)
    } else {
        None
    };

    let protocols = Protocols {
        compositor,
        layer_shell,
        xdg_shell,
        session_lock: smithay_client_toolkit::session_lock::SessionLockState::new(&globals, &qh),
        shm,
        fractional,
        activation,
    };
    #[allow(unused_mut)]
    let mut cx = Context::new(qh.clone(), handle, protocols, theme, sender, conn.clone());
    #[cfg(feature = "a11y")]
    {
        cx.a11y = a11y::A11y::new(a11y_tx);
    }
    let gfx = Gfx {
        egl,
        renderer: None,
        gl: None,
        debug_surfaces,
        software: None,
    };
    let mut state = State::new(
        app,
        cx,
        gfx,
        RegistryState::new(&globals),
        OutputState::new(&globals, &qh),
        SeatState::new(&globals, &qh),
    );
    state.theme_loader = theme_loader;
    state.cx.text_loading = text_loading;
    profile.mark("tema+durum");
    state.profile = profile;
    state.cx.ime.manager = text_input_manager;
    state.cx.clipboard.data_manager = DataDeviceManagerState::bind(&globals, &qh)
        .inspect_err(|_| tracing::info!("wl_data_device_manager yok; pano devre dışı"))
        .ok();
    state.cx.clipboard.primary_manager = PrimarySelectionManagerState::bind(&globals, &qh)
        .inspect_err(|_| tracing::debug!("zwp_primary_selection yok; birincil seçim devre dışı"))
        .ok();
    state.cx.foreign.exporter = globals
        .bind::<wayland_protocols::xdg::foreign::zv2::client::zxdg_exporter_v2::ZxdgExporterV2, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .inspect_err(|_| tracing::debug!("zxdg_exporter_v2 yok; pencere dışa aktarılamaz"))
        .ok();
    state.cx.foreign.importer = globals
        .bind::<wayland_protocols::xdg::foreign::zv2::client::zxdg_importer_v2::ZxdgImporterV2, _, _>(
            &qh,
            1..=1,
            NoEvents,
        )
        .inspect_err(|_| tracing::debug!("zxdg_importer_v2 yok; yabancı ebeveyn kurulamaz"))
        .ok();
    // hyprland_focus_grab_v1 (Hyprland'e özgü); yoksa `cx.focus_grab` `None`. Sonradan ilan
    // edilirse `FocusGrabGlobal` bağlar.
    let focus_grab_global = state
        .registry
        .globals_by_interface(focus_grab::MANAGER_INTERFACE)
        .next()
        .map(|g| g.name);
    match focus_grab_global {
        Some(name) => focus_grab::FocusGrabGlobal::bind(&mut state, &qh, name),
        None => tracing::debug!("hyprland_focus_grab_manager_v1 yok; odak kapma devre dışı"),
    }
    // hyprland_ctm_control_manager_v1 (Hyprland'e özgü): yalnızca kaydedilir; yönetici ilk
    // birim olmayan `set_color_transforms`'ta bağlanır (bağlanmak CTM sahipliğini alır).
    let ctm_global = state
        .registry
        .globals_by_interface(color_transform::MANAGER_INTERFACE)
        .next()
        .map(|g| (g.name, g.version));
    match ctm_global {
        Some((name, version)) => {
            color_transform::ColorTransformGlobal::record(&mut state, name, version);
        }
        None => tracing::debug!("hyprland_ctm_control_manager_v1 yok; renk dönüşümü devre dışı"),
    }
    // ext-idle-notify-v1: varsayılan koltuk (açılıştaki ilk wl_seat; sonradan gelen koltuklar
    // `SeatHandler::new_seat`'ten), sonra notifier. Yoksa `cx.idle_support()` Unsupported; sonradan
    // ilan edilirse `IdleNotifyGlobal` bağlar.
    if let Some(seat) = state.seat_state.seats().next() {
        state.cx.idle_seat_added(seat);
    }
    let idle_global = state
        .registry
        .globals_by_interface(idle::NOTIFIER_INTERFACE)
        .next()
        .map(|g| g.name);
    match idle_global {
        Some(name) => idle::IdleNotifyGlobal::bind(&mut state, &qh, name),
        None => tracing::debug!("ext_idle_notifier_v1 yok; boşta bildirimi devre dışı"),
    }
    // zwp_virtual_keyboard_manager_v1: yalnızca kaydedilir, ana bağlantıda hiç bağlanmaz
    // (sanal klavye ayrı bağlantıda, `cx.virtual_keyboard_set_keymap`'te).
    state.cx.vkbd.global = state
        .registry
        .globals_by_interface(virtual_keyboard::MANAGER_INTERFACE)
        .next()
        .map(|g| g.name);
    // ext-data-control-v1 / zwlr-data-control-v1: yalnızca kaydedilir; yönetici ve aygıt
    // `cx.data_control_start`'ta (izlemeyen uygulamaya compositor seçim bildirmez).
    for seat in state.seat_state.seats() {
        state.cx.data_control_seat_added(seat);
    }
    let dc_globals: Vec<(String, u32, u32)> =
        [data_control::EXT_INTERFACE, data_control::WLR_INTERFACE]
            .into_iter()
            .flat_map(|i| {
                state
                    .registry
                    .globals_by_interface(i)
                    .map(|g| (g.interface.clone(), g.name, g.version))
                    .collect::<Vec<_>>()
            })
            .collect();
    for (interface, name, version) in dc_globals {
        data_control::DataControlGlobal::record(&mut state, &interface, name, version);
    }
    // Pencere listesi, Hyprland adres eşlemesi ve yakalama: yalnızca kaydedilir; bağlanma
    // `toplevels_start` / ilk `capture`'da (boşta sıfır iş, compositor pencere bildirmez).
    let lazy_globals: Vec<(String, u32, u32)> = [
        toplevels::LIST_INTERFACE,
        toplevels::MAPPING_INTERFACE,
        capture::EXT_MANAGER_INTERFACE,
        capture::EXT_OUTPUT_SOURCE_INTERFACE,
        capture::EXT_TOPLEVEL_SOURCE_INTERFACE,
        capture::WLR_INTERFACE,
    ]
    .into_iter()
    .flat_map(|i| {
        state
            .registry
            .globals_by_interface(i)
            .map(|g| (g.interface.clone(), g.name, g.version))
            .collect::<Vec<_>>()
    })
    .collect();
    for (interface, name, version) in lazy_globals {
        toplevels::ToplevelsGlobal::record(&mut state, &interface, name, version);
        capture::CaptureGlobal::record(&mut state, &interface, name, version);
    }
    state.cx.install_watcher();
    state.cx.install_palette_watcher();
    state.cx.install_i18n_watcher();

    let result = loop {
        if let Err(e) = event_loop.dispatch(None, &mut state) {
            break Err(RunError::Disconnected(e.to_string()));
        }
        state.after_dispatch();
        #[cfg(feature = "a11y")]
        state.a11y_update();
        if state.cx.quit {
            break Ok(());
        }
    };
    // Kilitliyken çıkış (docs: Oturum kilidi, kural 3): kilit nesnesine istek gönderilmez —
    // `unlock_and_destroy` kilidi açar, `destroy` `locked`'tan sonra protokol hatasıdır.
    // Bağlantı kapanınca compositor oturumu kilitli tutar. `Pending`'de `destroy` geçerlidir.
    if let Some(lock) = state.cx.lock.take()
        && lock.state == LockState::Locked
    {
        tracing::warn!("kilitliyken çıkılıyor; oturum kilitli kalır");
        std::mem::forget(lock);
    }
    // EGL yüzeyleri wl_surface'lerden, EGL display'i bağlantıdan önce gitmeli.
    state.shutdown();
    if let Err(e) = conn.flush() {
        tracing::debug!("kapanışta flush: {e}");
    }
    // IPC kaynağını açıkça kaldır: socket dosyası silinsin (döngü düşmese bile).
    if let Some(token) = ipc_token {
        event_loop.handle().remove(token);
    }
    drop(state);
    drop(event_loop);
    result
}

use egl::Egl;
