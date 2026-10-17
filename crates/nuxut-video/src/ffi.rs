//! libmpv'ye çalışma zamanında yüklenen asgari bağlama (client API 2.x + render API).
//!
//! Bu modül crate'in **tek** `unsafe` yeridir (ARCHITECTURE: nuxut-video istisnası). Dışarıya
//! yalnızca güvenli tutamaklar verir: [`Mpv`] (`mpv_handle`, `Send + Sync`; libmpv client
//! API'si thread güvenlidir) ve [`RenderContext`] (GL thread'inde, context güncelken kullanılır).
//! Kütüphane `libmpv.so.2`'den `dlopen` edilir; yoksa [`Library::load`] hata döner.

#![allow(unsafe_code)]

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::sync::{Arc, OnceLock};

use crate::VideoError;
use crate::source::MediaSource;

#[repr(C)]
struct MpvHandle {
    _private: [u8; 0],
}

#[repr(C)]
struct MpvRenderCtx {
    _private: [u8; 0],
}

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

const FORMAT_INT64: c_int = 4;
const FORMAT_DOUBLE: c_int = 5;

const RENDER_PARAM_INVALID: c_int = 0;
const RENDER_PARAM_API_TYPE: c_int = 1;
const RENDER_PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const RENDER_PARAM_OPENGL_FBO: c_int = 3;
const RENDER_PARAM_FLIP_Y: c_int = 4;
const RENDER_PARAM_ADVANCED_CONTROL: c_int = 10;
const RENDER_UPDATE_FRAME: u64 = 1;

type Callback = unsafe extern "C" fn(*mut c_void);

/// Yüklenmiş işlev tablosu.
struct Api {
    _lib: libloading::Library,
    client_api_version: unsafe extern "C" fn() -> u64,
    error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    free: unsafe extern "C" fn(*mut c_void),
    create: unsafe extern "C" fn() -> *mut MpvHandle,
    initialize: unsafe extern "C" fn(*mut MpvHandle) -> c_int,
    terminate_destroy: unsafe extern "C" fn(*mut MpvHandle),
    set_option_string: unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    set_property_string: unsafe extern "C" fn(*mut MpvHandle, *const c_char, *const c_char) -> c_int,
    get_property_string: unsafe extern "C" fn(*mut MpvHandle, *const c_char) -> *mut c_char,
    get_property: unsafe extern "C" fn(*mut MpvHandle, *const c_char, c_int, *mut c_void) -> c_int,
    command: unsafe extern "C" fn(*mut MpvHandle, *mut *const c_char) -> c_int,
    wait_event: unsafe extern "C" fn(*mut MpvHandle, f64) -> *mut RawEvent,
    set_wakeup_callback: unsafe extern "C" fn(*mut MpvHandle, Option<Callback>, *mut c_void),
    render_create: unsafe extern "C" fn(*mut *mut MpvRenderCtx, *mut MpvHandle, *mut RenderParam) -> c_int,
    render_set_update_callback: unsafe extern "C" fn(*mut MpvRenderCtx, Option<Callback>, *mut c_void),
    render_update: unsafe extern "C" fn(*mut MpvRenderCtx) -> u64,
    render_render: unsafe extern "C" fn(*mut MpvRenderCtx, *mut RenderParam) -> c_int,
    render_report_swap: unsafe extern "C" fn(*mut MpvRenderCtx),
    render_free: unsafe extern "C" fn(*mut MpvRenderCtx),
    /// `eglGetProcAddress` (render API'nin GL işlev yükleyicisi için).
    egl_get_proc: Option<unsafe extern "C" fn(*const c_char) -> *mut c_void>,
    _egl: Option<libloading::Library>,
}

static API: OnceLock<Result<Arc<Api>, String>> = OnceLock::new();

fn api() -> Result<Arc<Api>, VideoError> {
    API.get_or_init(|| load().map(Arc::new)).clone().map_err(VideoError::Unavailable)
}

/// libmpv yüklenebiliyor mu (ilk çağrıda `dlopen`; sonuç süreç boyunca önbellekli).
pub fn available() -> bool {
    api().is_ok()
}

fn load() -> Result<Api, String> {
    // SAFETY: libmpv, yüklenince yan etkisi olmayan (constructor'ı yalnızca iç durum kuran)
    // sistem kütüphanesidir; sembol imzaları client.h / render.h / render_gl.h (client API
    // 2.x) ile birebir aynıdır ve sürüm aşağıda denetlenir.
    unsafe {
        let lib = libloading::Library::new("libmpv.so.2").map_err(|e| format!("libmpv.so.2 yüklenemedi: {e}"))?;
        macro_rules! sym {
            ($name:literal) => {
                *lib.get(concat!($name, "\0").as_bytes()).map_err(|e| format!("{}: {e}", $name))?
            };
        }
        let (egl, egl_get_proc) = match libloading::Library::new("libEGL.so.1") {
            Ok(l) => {
                let f = l.get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"eglGetProcAddress\0").ok().map(|s| *s);
                (Some(l), f)
            }
            Err(_) => (None, None),
        };
        let api = Api {
            client_api_version: sym!("mpv_client_api_version"),
            error_string: sym!("mpv_error_string"),
            free: sym!("mpv_free"),
            create: sym!("mpv_create"),
            initialize: sym!("mpv_initialize"),
            terminate_destroy: sym!("mpv_terminate_destroy"),
            set_option_string: sym!("mpv_set_option_string"),
            set_property_string: sym!("mpv_set_property_string"),
            get_property_string: sym!("mpv_get_property_string"),
            get_property: sym!("mpv_get_property"),
            command: sym!("mpv_command"),
            wait_event: sym!("mpv_wait_event"),
            set_wakeup_callback: sym!("mpv_set_wakeup_callback"),
            render_create: sym!("mpv_render_context_create"),
            render_set_update_callback: sym!("mpv_render_context_set_update_callback"),
            render_update: sym!("mpv_render_context_update"),
            render_render: sym!("mpv_render_context_render"),
            render_report_swap: sym!("mpv_render_context_report_swap"),
            render_free: sym!("mpv_render_context_free"),
            egl_get_proc,
            _egl: egl,
            _lib: lib,
        };
        let v = (api.client_api_version)();
        if v >> 16 != 2 {
            return Err(format!("desteklenmeyen libmpv client API sürümü {}.{}", v >> 16, v & 0xffff));
        }
        Ok(api)
    }
}

fn cstr(s: &str) -> Result<CString, VideoError> {
    CString::new(s).map_err(|_| VideoError::Mpv(format!("NUL içeren dize ({} bayt)", s.len())))
}

/// Olay (yalnızca kullandıklarımız).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    None,
    Shutdown,
    FileLoaded,
    /// `error`: 0 = normal bitiş; `reason`: 0 EOF, 2 stop, 3 quit, 4 hata.
    EndFile { reason: i32, error: i32 },
    VideoReconfig,
    PlaybackRestart,
    Other(i32),
}

/// `mpv_handle`. Düşünce `mpv_terminate_destroy` (render context'i önce düşürülmeli).
pub struct Mpv {
    api: Arc<Api>,
    handle: *mut MpvHandle,
    /// Uyandırma callback'inin verisi; tutamak yaşadıkça yaşar.
    wake: Option<Box<Box<dyn Fn() + Send + Sync>>>,
    /// `mpv_wait_event` aynı anda tek thread'den çağrılabilir.
    events: std::sync::Mutex<()>,
    /// `mpv_terminate_destroy` arka plan thread'inde ([`Mpv::terminate_in_background`]).
    background_terminate: bool,
}

// SAFETY: libmpv client API'si thread güvenlidir (client.h "Thread safety"): aynı
// `mpv_handle` üzerinde her işlev her thread'den çağrılabilir; `mpv_wait_event` aynı anda tek
// thread'den çağrılmalı — `events` kilidiyle sağlanır.
unsafe impl Send for Mpv {}
// SAFETY: yukarıdaki gibi; `&self` alan yöntemler yalnızca thread güvenli işlevleri çağırır.
unsafe impl Sync for Mpv {}

impl std::fmt::Debug for Mpv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mpv")
    }
}

/// Her mpv örneğine çağıranın seçeneklerinden ÖNCE uygulanır ve çağıran bunları ezemez.
/// Kaynak güvenilmeyen medyadır: ağ, betik, yapılandırma, dış dosya ve iç başvuru yok.
/// (anahtar, değer, zorunlu): zorunlu olmayanlar eski mpv'de yoksa atlanır (o özellik de yok).
const HARDENING: &[(&str, &str, bool)] = &[
    ("config", "no", true),
    ("use-filedir-conf", "no", true),
    ("load-auto-profiles", "no", true),
    ("terminal", "no", true),
    // mpv iletileri yol ve medya üst verisi taşır (loglarda kullanıcı içeriği yok).
    ("msg-level", "all=no", true),
    ("input-default-bindings", "no", true),
    ("input-vo-keyboard", "no", true),
    ("input-ipc-server", "", true),
    ("load-scripts", "no", true),
    ("scripts", "", true),
    ("osc", "no", true),
    ("ytdl", "no", true),
    ("resume-playback", "no", true),
    ("save-position-on-quit", "no", true),
    // Playlist/EDL/cue/sıralı bölüm (mpv'nin kendi demuxer'ları) hiç denenmez.
    ("demuxer", "lavf", true),
    ("ordered-chapters", "no", true),
    ("load-unsafe-playlists", "no", true),
    // lavf iç açılışları (HLS/concat bölümleri, mov dış başvuruları) reddedilir.
    ("access-references", "no", true),
    ("demuxer-lavf-o", "protocol_whitelist=[file,fd]", true),
    ("stream-lavf-o", "protocol_whitelist=[file,fd]", true),
    ("sub-auto", "no", true),
    // Gömülü (default/forced bayraklı dahil) alt yazı izi seçilmez, duvar kağıdında görünmez.
    ("sid", "no", true),
    // libmpv OSD iletileri (yükleme, duraklatma, hata) render'a çizilmez.
    ("osd-level", "0", true),
    ("audio-file-auto", "no", true),
    ("cover-art-auto", "no", false),
    ("autoload-files", "no", false),
    ("load-stats-overlay", "no", false),
    ("load-console", "no", false),
    ("load-commands", "no", false),
    ("load-select", "no", false),
    ("load-positioning", "no", false),
];

const MPV_ERROR_OPTION_NOT_FOUND: c_int = -5;

impl Mpv {
    /// `mpv_create` + [`HARDENING`] + seçenekler + `mpv_initialize`.
    pub fn new(options: &[(&str, &str)]) -> Result<Self, VideoError> {
        if let Some((k, _)) = options.iter().find(|(k, _)| HARDENING.iter().any(|(h, _, _)| h == k)) {
            return Err(VideoError::Mpv(format!("sertleştirilmiş seçenek ezilemez: {k}")));
        }
        let api = api()?;
        // SAFETY: işlev tablosu yüklü; NULL dönüşü denetlenir.
        let handle = unsafe { (api.create)() };
        if handle.is_null() {
            return Err(VideoError::Mpv("mpv_create başarısız".into()));
        }
        let mpv = Self { api, handle, wake: None, events: std::sync::Mutex::new(()), background_terminate: false };
        for &(k, v, required) in HARDENING {
            let code = mpv.set_option(k, v)?;
            if required || code != MPV_ERROR_OPTION_NOT_FOUND {
                mpv.check(code, k)?;
            }
        }
        for (k, v) in options {
            let code = mpv.set_option(k, v)?;
            mpv.check(code, k)?;
        }
        // SAFETY: tutamak canlı; bir kez çağrılır.
        mpv.check(unsafe { (mpv.api.initialize)(mpv.handle) }, "mpv_initialize")?;
        Ok(mpv)
    }

    fn set_option(&self, k: &str, v: &str) -> Result<c_int, VideoError> {
        let (k, v) = (cstr(k)?, cstr(v)?);
        // SAFETY: tutamak canlı ve henüz başlatılmadı; dizeler NUL sonlu ve çağrı boyunca canlı.
        Ok(unsafe { (self.api.set_option_string)(self.handle, k.as_ptr(), v.as_ptr()) })
    }

    /// Kaynağı yükler. Dosya tanıtıcısının sahipliği mpv'ye ancak `loadfile` kabul edilince
    /// geçer (`fdclose://`): mpv fd'yi komutta değil, akışı açarken kullanır ve kapatır; komut
    /// reddedilirse fd bizde kalır ve burada kapanır. Yol ya da URL hiçbir zaman verilmez
    /// (`source` modülü).
    pub(crate) fn load(&self, source: MediaSource, replace: bool) -> Result<(), VideoError> {
        let mode = if replace { "replace" } else { "append-play" };
        match source {
            MediaSource::File(f) => self.load_file(f, mode),
            MediaSource::Builtin(s) => self.command(&["loadfile", s, mode]),
        }
    }

    fn load_file(&self, file: std::fs::File, mode: &str) -> Result<(), VideoError> {
        use std::os::fd::{AsRawFd, IntoRawFd, OwnedFd};
        let fd = OwnedFd::from(file);
        self.command(&["loadfile", &format!("fdclose://{}", fd.as_raw_fd()), mode])?;
        // Kabul edildi: kapatma artık mpv'nin.
        let _ = fd.into_raw_fd();
        Ok(())
    }

    /// Yüklenen dosya tek bir medya mı (playlist açılmadı). `FileLoaded`'dan sonra çağrılır.
    pub(crate) fn single_media(&self) -> bool {
        self.get_i64("playlist-count") == Some(1)
    }

    fn check(&self, code: c_int, what: &str) -> Result<(), VideoError> {
        if code >= 0 {
            return Ok(());
        }
        // SAFETY: mpv_error_string her hata kodu için statik bir C dizesi döner.
        let msg = unsafe { CStr::from_ptr((self.api.error_string)(code)) }.to_string_lossy();
        Err(VideoError::Mpv(format!("{what}: {msg}")))
    }

    pub fn set_property(&self, name: &str, value: &str) -> Result<(), VideoError> {
        let (k, v) = (cstr(name)?, cstr(value)?);
        // SAFETY: tutamak canlı; dizeler çağrı boyunca canlı.
        self.check(unsafe { (self.api.set_property_string)(self.handle, k.as_ptr(), v.as_ptr()) }, name)
    }

    pub fn get_string(&self, name: &str) -> Option<String> {
        let k = cstr(name).ok()?;
        // SAFETY: tutamak canlı; dönen işaretçi mpv_free ile serbest bırakılır.
        unsafe {
            let p = (self.api.get_property_string)(self.handle, k.as_ptr());
            if p.is_null() {
                return None;
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.api.free)(p.cast());
            Some(s)
        }
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let k = cstr(name).ok()?;
        let mut v = 0f64;
        // SAFETY: MPV_FORMAT_DOUBLE için çıktı bir `double`'dır.
        let r = unsafe { (self.api.get_property)(self.handle, k.as_ptr(), FORMAT_DOUBLE, (&raw mut v).cast()) };
        (r >= 0).then_some(v)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let k = cstr(name).ok()?;
        let mut v = 0i64;
        // SAFETY: MPV_FORMAT_INT64 için çıktı bir `int64_t`'dir.
        let r = unsafe { (self.api.get_property)(self.handle, k.as_ptr(), FORMAT_INT64, (&raw mut v).cast()) };
        (r >= 0).then_some(v)
    }

    pub fn command(&self, args: &[&str]) -> Result<(), VideoError> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect::<Result<_, _>>()?;
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|s| s.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: NULL sonlu dizi; dizeler çağrı boyunca canlı.
        self.check(unsafe { (self.api.command)(self.handle, ptrs.as_mut_ptr()) }, args.first().copied().unwrap_or("?"))
    }

    /// Sıradaki olay; `timeout` saniye bekler (0 = beklemeden).
    pub fn wait_event(&self, timeout: f64) -> Event {
        let _guard = self.events.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: kilit tek bekleyiciyi garanti eder; dönen olay bir sonraki
        // mpv_wait_event'e kadar geçerlidir ve burada hemen kopyalanır.
        unsafe {
            let e = &*(self.api.wait_event)(self.handle, timeout);
            match e.event_id {
                0 => Event::None,
                1 => Event::Shutdown,
                7 => {
                    let d = e.data.cast::<RawEndFile>();
                    if d.is_null() {
                        Event::EndFile { reason: 4, error: e.error }
                    } else {
                        Event::EndFile { reason: (*d).reason, error: (*d).error }
                    }
                }
                8 => Event::FileLoaded,
                17 => Event::VideoReconfig,
                21 => Event::PlaybackRestart,
                other => Event::Other(other),
            }
        }
    }

    /// Düşerken `mpv_terminate_destroy` arka plan thread'inde yapılır: render thread'inde
    /// düşen oynatıcı kareyi bekletmez. Kısa ömürlü yoklama/içe aktarma örnekleri bunu
    /// kullanmaz (eşzamanlı kapanır, peş peşe çekirdekler birikmez).
    pub fn terminate_in_background(&mut self) {
        self.background_terminate = true;
    }

    /// Olay geldiğinde (herhangi bir thread'den) çağrılır. Callback içinde mpv çağrılmamalı.
    pub fn set_wakeup_callback(&mut self, f: impl Fn() + Send + Sync + 'static) {
        let boxed: Box<Box<dyn Fn() + Send + Sync>> = Box::new(Box::new(f));
        let data = (&raw const *boxed).cast_mut().cast::<c_void>();
        // SAFETY: veri işaretçisi `self.wake` içinde tutulur ve tutamak yok edilene kadar
        // (terminate_destroy callback'i kaldırır) yaşar; trampolin yalnızca Fn çağırır.
        unsafe { (self.api.set_wakeup_callback)(self.handle, Some(trampoline), data) };
        self.wake = Some(boxed);
    }

    /// GL render context'i. GL context'i güncel olmalı (render thread'inde).
    pub fn create_render_context(&self, update: impl Fn() + Send + Sync + 'static) -> Result<RenderContext, VideoError> {
        let get_proc = self.api.egl_get_proc.ok_or_else(|| VideoError::Unavailable("libEGL yok".into()))?;
        let mut init = OpenGlInitParams { get_proc_address, get_proc_address_ctx: get_proc as *mut c_void };
        let api_type = c"opengl";
        let mut advanced: c_int = 0;
        let mut params = [
            RenderParam { kind: RENDER_PARAM_API_TYPE, data: api_type.as_ptr().cast_mut().cast() },
            RenderParam { kind: RENDER_PARAM_OPENGL_INIT_PARAMS, data: (&raw mut init).cast() },
            RenderParam { kind: RENDER_PARAM_ADVANCED_CONTROL, data: (&raw mut advanced).cast() },
            RenderParam { kind: RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        let mut ctx: *mut MpvRenderCtx = std::ptr::null_mut();
        // SAFETY: parametre dizisi INVALID ile sonlanır; işaret ettikleri çağrı boyunca canlı;
        // GL context'inin güncel olması çağıranın sorumluluğu (belgelendi).
        self.check(unsafe { (self.api.render_create)(&raw mut ctx, self.handle, params.as_mut_ptr()) }, "mpv_render_context_create")?;
        let boxed: Box<Box<dyn Fn() + Send + Sync>> = Box::new(Box::new(update));
        let data = (&raw const *boxed).cast_mut().cast::<c_void>();
        // SAFETY: veri `RenderContext.update` içinde context'ten uzun yaşar (free önce çağrılır).
        unsafe { (self.api.render_set_update_callback)(ctx, Some(trampoline), data) };
        Ok(RenderContext { api: self.api.clone(), ctx, _update: boxed })
    }
}

/// `mpv_terminate_destroy` çözücü/ses/demuxer thread'lerinin bitmesini bekler (yüzlerce ms,
/// takılan bir donanım çözücüsünde daha uzun). Oynatıcıyı düşüren thread render
/// thread'idir (GL çöplüğü); kare kaçmasın diye yok etme arka plan thread'inde yapılır.
struct Terminate {
    api: Arc<Api>,
    handle: *mut MpvHandle,
}

// SAFETY: client API'si thread güvenli; tutamak bu değere taşınmıştır, başka sahibi yoktur.
unsafe impl Send for Terminate {}

impl Terminate {
    fn run(self) {
        // SAFETY: tutamak bizim; bir kez yok edilir. Render context'i daha önce düşürüldü
        // (`RenderContext` belgesi); `Api` (kütüphane) bu değerle birlikte yaşar.
        unsafe { (self.api.terminate_destroy)(self.handle) };
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // SAFETY: tutamak canlı. Uyandırma callback'i eşzamanlı kaldırılır: libmpv callback'i
        // kilit altında çağırır, dönüşten sonra çağrı sürmez; callback verisi (`self.wake`) bu
        // gövdeden sonra güvenle düşer.
        unsafe { (self.api.set_wakeup_callback)(self.handle, None, std::ptr::null_mut()) };
        let job = Terminate { api: self.api.clone(), handle: self.handle };
        if !self.background_terminate {
            job.run();
            return;
        }
        let spawned = std::thread::Builder::new().name("mpv-terminate".into()).spawn(move || job.run());
        // Thread açılamadıysa kapanış (iş çalışmadan düştü; `Terminate` Drop'suz) burada,
        // eşzamanlı yapılır; tutamak sızdırılmaz.
        if let Err(e) = spawned {
            tracing::debug!("mpv-terminate thread'i açılamadı, eşzamanlı yok ediliyor: {e}");
            Terminate { api: self.api.clone(), handle: self.handle }.run();
        }
    }
}

unsafe extern "C" fn trampoline(data: *mut c_void) {
    // SAFETY: `data`, sahibi tarafından canlı tutulan `Box<dyn Fn() + Send + Sync>`'tir.
    let f = unsafe { &*(data.cast::<Box<dyn Fn() + Send + Sync>>()) };
    // Panik C sınırını geçmesin.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
}

unsafe extern "C" fn get_proc_address(ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: ctx, create_render_context'te verilen eglGetProcAddress işaretçisidir.
    unsafe {
        let f: unsafe extern "C" fn(*const c_char) -> *mut c_void = std::mem::transmute(ctx);
        f(name)
    }
}

/// `mpv_render_context`. Yalnızca GL thread'inde, context güncelken kullanılır; `Mpv`'den
/// önce düşürülmelidir.
pub struct RenderContext {
    api: Arc<Api>,
    ctx: *mut MpvRenderCtx,
    _update: Box<Box<dyn Fn() + Send + Sync>>,
}

// SAFETY: işaretçi yalnızca bu tip içinden kullanılır; render API'si "render thread'inden"
// çağrılmalı — kullanıcı (VideoPlayer) çağrıları tek GL thread'inde yapar ve Mutex ile korur.
unsafe impl Send for RenderContext {}

impl std::fmt::Debug for RenderContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RenderContext")
    }
}

impl RenderContext {
    /// Yeni kare var mı (update callback'inden sonra çağrılır).
    pub fn frame_ready(&mut self) -> bool {
        // SAFETY: context canlı; render thread'inde çağrılır.
        unsafe { (self.api.render_update)(self.ctx) & RENDER_UPDATE_FRAME != 0 }
    }

    /// Kareyi `fbo`'ya (fiziksel `w`×`h`) çizer. GL context güncel olmalı.
    pub fn render(&mut self, fbo: u32, w: i32, h: i32, flip_y: bool) -> Result<(), VideoError> {
        let mut target = OpenGlFbo { fbo: fbo as c_int, w, h, internal_format: 0 };
        let mut flip: c_int = c_int::from(flip_y);
        let mut params = [
            RenderParam { kind: RENDER_PARAM_OPENGL_FBO, data: (&raw mut target).cast() },
            RenderParam { kind: RENDER_PARAM_FLIP_Y, data: (&raw mut flip).cast() },
            RenderParam { kind: RENDER_PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        // SAFETY: parametreler çağrı boyunca canlı; context canlı ve GL güncel (çağıran).
        let r = unsafe { (self.api.render_render)(self.ctx, params.as_mut_ptr()) };
        // SAFETY: context canlı.
        unsafe { (self.api.render_report_swap)(self.ctx) };
        if r < 0 { Err(VideoError::Mpv(format!("mpv_render_context_render: {r}"))) } else { Ok(()) }
    }
}

impl Drop for RenderContext {
    fn drop(&mut self) {
        // SAFETY: context bizim; GL context'inin güncel olması gerekir. `VideoPlayer` bunu
        // `nuxut_render::gl::GlGraveyard` ile renderer'ın thread'ine taşır (karenin başında ya
        // da renderer düşerken, context güncel); context'siz GL çağrısı bazı sürücülerde çöker.
        unsafe {
            (self.api.render_set_update_callback)(self.ctx, None, std::ptr::null_mut());
            (self.api.render_free)(self.ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arka plan kapanışı ile eşzamanlı kapanış yan yana: ikisi de tutamağı bir kez yok eder,
    /// ardından yeni çekirdek açılabilir (kütüphane `Api` ile yaşar).
    #[test]
    fn background_and_sync_terminate() {
        let Ok(mut bg) = Mpv::new(&[("vo", "null"), ("ao", "null")]) else {
            eprintln!("libmpv yok; test atlandı");
            return;
        };
        bg.set_wakeup_callback(|| {});
        bg.terminate_in_background();
        drop(bg);
        let sync = Mpv::new(&[("vo", "null"), ("ao", "null")]).expect("ikinci çekirdek");
        assert!(sync.get_string("mpv-version").is_some());
        drop(sync);
    }

    #[test]
    fn hardening_hides_subtitles_and_osd() {
        for (k, v) in [("sid", "no"), ("osd-level", "0"), ("sub-auto", "no")] {
            assert!(HARDENING.contains(&(k, v, true)), "{k}={v} zorunlu olmalı");
        }
        let Ok(m) = Mpv::new(&[("vo", "null"), ("ao", "null")]) else {
            eprintln!("libmpv yok; test atlandı");
            return;
        };
        assert_eq!(m.get_string("sid").as_deref(), Some("no"));
        assert_eq!(m.get_string("osd-level").as_deref(), Some("0"));
        for (k, v) in [("sid", "auto"), ("osd-level", "3")] {
            assert!(Mpv::new(&[(k, v)]).is_err(), "{k} ezilebildi");
        }
    }

    /// Varsayılan bayraklı gömülü alt yazı izi olan mkv: iz var ama seçilmez.
    #[test]
    fn default_subtitle_track_is_not_selected() {
        let Ok(m) = Mpv::new(&[("vo", "null"), ("ao", "null"), ("pause", "yes")]) else {
            eprintln!("libmpv yok; test atlandı");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let (srt, mkv) = (dir.path().join("a.srt"), dir.path().join("a.mkv"));
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:05,000\nALT YAZI\n").unwrap();
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=64x36:rate=5:duration=1", "-i"])
            .arg(&srt)
            .args(["-c:v", "libx264", "-c:s", "srt", "-disposition:s:0", "default"])
            .arg(&mkv)
            .status()
            .is_ok_and(|s| s.success());
        if !made {
            eprintln!("ffmpeg yok; test atlandı");
            return;
        }
        m.load(MediaSource::File(crate::open_media(&mkv).unwrap()), true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "dosya yüklenmedi");
            if let Event::FileLoaded = m.wait_event(1.0) {
                break;
            }
        }
        let n = m.get_i64("track-list/count").unwrap();
        let sub = (0..n).find(|i| m.get_string(&format!("track-list/{i}/type")).as_deref() == Some("sub"));
        let sub = sub.expect("alt yazı izi dosyada olmalı");
        assert_eq!(m.get_string(&format!("track-list/{sub}/default")).as_deref(), Some("yes"));
        assert_eq!(m.get_string(&format!("track-list/{sub}/selected")).as_deref(), Some("no"));
        assert!(m.get_string("current-tracks/sub/id").is_none(), "alt yazı seçildi");
        // Karşı kanıt: iz seçilebilir; seçilmemesi yalnızca `sid=no`'dan.
        m.set_property("sid", "auto").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while m.get_string("current-tracks/sub/id").is_none() {
            assert!(std::time::Instant::now() < deadline, "sid=auto ile de seçilmedi (test dosyası geçersiz)");
            m.wait_event(0.1);
        }
    }

    /// `loadfile` reddedilirse fd mpv'ye geçmez, bizde kapanır (sızmaz).
    #[test]
    fn rejected_loadfile_closes_fd() {
        let Ok(m) = Mpv::new(&[("vo", "null"), ("ao", "null")]) else {
            eprintln!("libmpv yok; test atlandı");
            return;
        };
        use std::os::fd::AsRawFd;
        let file = tempfile::tempfile().unwrap();
        let raw = file.as_raw_fd();
        let id = |s: rustix::fs::Stat| (s.st_dev, s.st_ino);
        let before = id(rustix::fs::fstat(&file).unwrap());
        assert!(m.load_file(file, "gecersiz-kip").is_err());
        // SAFETY: yalnızca fd numarasının neyi gösterdiği sorulur (fstat); fd kullanılmaz.
        // Paralel testler numarayı yeniden almış olabilir: yalnızca aynı dosya sızıntıdır.
        let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(raw) };
        assert!(rustix::fs::fstat(fd).map(id).ok() != Some(before), "fd sızdı");
    }
}
