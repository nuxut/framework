//! Tek kare çıkış/pencere yakalama: `ext-image-copy-capture-v1` (+ `ext-image-capture-source-v1`
//! çıkış ve pencere kaynakları), çıkış için yedek `zwlr_screencopy_manager_v1`.
//!
//! Tasarım ve gerekçe `docs/design/capture.md`, sözleşme `docs/ARCHITECTURE.md` "Pencere listesi ve
//! yakalama". Özet:
//! - Talep üzerine tek kare; oturum/kaynak/kare istek bitince yok edilir (boşta sıfır iş).
//!   Yöneticiler ilk yakalamada bağlanır.
//! - Tampon `wl_shm` + memfd (dosya sisteminde iz yok), salt okunur eşleme. Tavan
//!   [`MAX_BUFFER_BYTES`]; aynı anda [`MAX_ACTIVE`] iş, fazlası FIFO kuyrukta ([`MAX_QUEUED`]).
//! - Kaynakta küçültme yok: tam kare alınır, `nuxut-capture` thread'inde kutu filtresiyle
//!   `max_size`'a sığdırılır ve premultiplied RGBA8'e çevrilir (`capture_pixels.rs`).
//! - Piksel, başlık ve uygulama kimliği loglanmaz (değişmez 9); `CapturedImage` Debug redakte.
//! - Bu süreç oturum kilidi isterken/tutarken ya da uygulama dış kilidi bildirdiyse
//!   (`set_session_locked_hint`, logind `LockedHint`) yakalama `Locked` ile reddedilir.

use std::collections::{HashMap, VecDeque};
use std::os::fd::AsFd;
use std::ptr::NonNull;
use std::time::Duration;

use calloop::RegistrationToken;
use calloop::timer::{TimeoutAction, Timer};
use nuxut_render::{ImageData, MAX_TEXTURE_SIDE};
use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryHandler};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shm::{self, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::{Connection, Proxy, QueueHandle, WEnum};
use wayland_protocols::ext::image_capture_source::v1::client::ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1, FailureReason,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{
    self, ZwlrScreencopyFrameV1,
};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use crate::App;
use crate::capture_pixels::{self, PixelFormat, Source};
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};
use crate::toplevels::ToplevelId;
use crate::types::OutputId;

pub(crate) const EXT_MANAGER_INTERFACE: &str = "ext_image_copy_capture_manager_v1";
pub(crate) const EXT_OUTPUT_SOURCE_INTERFACE: &str = "ext_output_image_capture_source_manager_v1";
pub(crate) const EXT_TOPLEVEL_SOURCE_INTERFACE: &str =
    "ext_foreign_toplevel_image_capture_source_manager_v1";
pub(crate) const WLR_INTERFACE: &str = "zwlr_screencopy_manager_v1";

/// Tampon başına bayt tavanı: 7680×4320×4 (8K UHD). Aşan kaynak `TooLarge`.
pub const MAX_BUFFER_BYTES: usize = 7680 * 4320 * 4;
/// Aynı anda yürüyen iş (oturum + çevirme) sayısı; bellek en kötü `MAX_ACTIVE × MAX_BUFFER_BYTES`.
pub const MAX_ACTIVE: usize = 2;
/// Bekleyen istek sınırı; aşan istek `Busy`.
pub const MAX_QUEUED: usize = 256;
/// Varsayılan süre sınırı (oturum başladıktan sonra).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
/// ext `failed(buffer_constraints)` sonrası en çok bu kadar deneme.
const MAX_ATTEMPTS: u8 = 2;

/// `cx.capture` isteğinin kimliği; sonuç `App::captured` ile gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CaptureId(pub(crate) u64);

/// Ne yakalanacak.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureSource {
    /// Bir çıkışın (monitör) tamamı.
    Output(OutputId),
    /// Bir pencere (`cx.toplevels_start` sonrası bilinen).
    Toplevel(ToplevelId),
}

/// Yakalama seçenekleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureOptions {
    /// Sonuç bu kutuya en-boy korunarak sığdırılır (büyütülmez). Her kenar 1..=8192'ye kırpılır.
    pub max_size: (u32, u32),
    /// İmleç kareye çizilsin mi (varsayılan hayır).
    pub paint_cursor: bool,
    /// Oturum başladıktan sonra en çok bu kadar beklenir; sonra `Timeout`.
    pub timeout: Duration,
}

impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            max_size: (MAX_TEXTURE_SIDE, MAX_TEXTURE_SIDE),
            paint_cursor: false,
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl CaptureOptions {
    /// Önizleme: `max_size` kutusu, diğerleri varsayılan.
    pub fn preview(width: u32, height: u32) -> Self {
        Self {
            max_size: (width, height),
            ..Self::default()
        }
    }

    fn clamped_max(&self) -> (u32, u32) {
        (
            self.max_size.0.clamp(1, MAX_TEXTURE_SIDE),
            self.max_size.1.clamp(1, MAX_TEXTURE_SIDE),
        )
    }
}

/// Yakalanan kare: dik, `max_size`'a sığmış premultiplied RGBA8. Debug redakte (yalnız boyut).
#[derive(Clone, PartialEq, Eq)]
pub struct CapturedImage {
    /// `Painter::upload_texture`'a doğrudan verilebilir.
    pub image: ImageData,
    /// Compositor tamponunun boyutu (fiziksel piksel, dönüşümden önce).
    pub source_size: (u32, u32),
}

impl std::fmt::Debug for CapturedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "CapturedImage({}x{} ← {}x{})",
            self.image.width, self.image.height, self.source_size.0, self.source_size.1
        )
    }
}

/// Yakalama hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// Compositor bu kaynak türü için yakalama protokolü sunmuyor.
    #[error("compositor bu kaynak için yakalama sunmuyor")]
    Unsupported,
    /// Bilinmeyen çıkış ya da pencere (liste başlatılmadı, kapandı, kaldırıldı).
    #[error("bilinmeyen kaynak")]
    UnknownSource,
    /// Kaynak yakalama sırasında gitti (pencere kapandı, çıkış kaldırıldı).
    #[error("kaynak yakalama sırasında kayboldu")]
    SourceGone,
    /// Oturum kilitli: bu süreç kilit istedi/tutuyor ya da dış kilit bildirildi.
    #[error("oturum kilitliyken yakalama yapılmaz")]
    Locked,
    /// Tampon `MAX_BUFFER_BYTES`'ı aşıyor.
    #[error("kaynak çok büyük ({width}x{height})")]
    TooLarge { width: u32, height: u32 },
    /// Compositor desteklenen bir 8 bit `wl_shm` biçimi önermedi.
    #[error("kullanılabilir tampon biçimi yok")]
    NoUsableFormat,
    /// Kuyruk dolu.
    #[error("çok fazla bekleyen yakalama")]
    Busy,
    /// Süre doldu (gizli/görünmeyen pencere kare üretmeyebilir).
    #[error("yakalama zaman aşımı")]
    Timeout,
    /// Compositor kareyi reddetti.
    #[error("compositor yakalamayı reddetti")]
    Failed,
    /// Tampon kurulamadı (memfd/mmap) ya da compositor tutarsız boyut verdi.
    #[error("yakalama tamponu kurulamadı")]
    Buffer,
}

/// Çıkış yakalamanın hangi protokolle yapılacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureBackend {
    ExtImageCopy,
    WlrScreencopy,
}

/// Compositor desteği (bağlanmadan, global ilanlarından).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureSupport {
    pub output: Option<CaptureBackend>,
    /// Pencere yakalama (ext kopya + pencere kaynağı + pencere listesi).
    pub toplevel: bool,
}

/// Test düğmesi: `NUXUT_CAPTURE=wlr` ext varken de çıkış için wlr yedeğini seçtirir.
fn force_wlr() -> bool {
    std::env::var_os("NUXUT_CAPTURE").is_some_and(|v| v == "wlr")
}

// ---------------------------------------------------------------------------------------------
// Bellek eşlemesi.
// ---------------------------------------------------------------------------------------------

/// memfd'nin salt okunur eşlemesi. Çevirme thread'ine taşınır; düşünce kaldırılır.
struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: eşleme yalnızca bu değere aittir (gösterici başka yere verilmez); salt okunur
// erişilir ve `Drop`'ta bir kez kaldırılır. Thread'e taşındığında Wayland nesneleri (havuz, tampon)
// zaten yok edilmiştir; compositor `ready` sonrası tampona yazmaz.
unsafe impl Send for Mapping {}

impl Mapping {
    fn new(len: usize) -> Result<(Self, std::os::fd::OwnedFd), String> {
        let fd = rustix::fs::memfd_create("nuxut-capture", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|e| format!("memfd_create: {e}"))?;
        rustix::fs::ftruncate(&fd, len as u64).map_err(|e| format!("ftruncate: {e}"))?;
        // SAFETY: yeni, `len` bayta büyütülmüş, yalnızca bizim tuttuğumuz memfd; eşleme `Drop`'ta
        // aynı uzunlukla kaldırılır. Salt okunur: compositor yazar, biz okuruz.
        let ptr = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                rustix::mm::ProtFlags::READ,
                rustix::mm::MapFlags::SHARED,
                &fd,
                0,
            )
        }
        .map_err(|e| format!("mmap: {e}"))?;
        let ptr = NonNull::new(ptr.cast::<u8>()).ok_or("mmap boş gösterici")?; // i18n: tanı shm eşleme hatası, yalnızca loga
        Ok((Self { ptr, len }, fd))
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` `len` baytlık, okunabilir, bu değer yaşadıkça geçerli eşleme. Compositor
        // kare `ready` olduktan sonra yazmaz (bu noktadan önce okunmaz).
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `new`'deki eşlemenin kendisi; bir kez, aynı uzunlukla kaldırılır.
        if let Err(e) = unsafe { rustix::mm::munmap(self.ptr.as_ptr().cast(), self.len) } {
            tracing::warn!("munmap: {e}");
        }
    }
}

/// Bir yakalama tamponu: `wl_buffer` + havuz + eşleme.
struct CaptureBuffer {
    buffer: WlBuffer,
    pool: WlShmPool,
    map: Mapping,
    format: PixelFormat,
    width: u32,
    height: u32,
    stride: usize,
}

impl CaptureBuffer {
    fn new<D>(
        shm: &WlShm,
        qh: &QueueHandle<D>,
        format: PixelFormat,
        (width, height): (u32, u32),
        stride: usize,
    ) -> Result<Self, CaptureError>
    where
        D: 'static
            + wayland_client::Dispatch<WlShmPool, NoEvents>
            + wayland_client::Dispatch<WlBuffer, NoEvents>,
    {
        let len = buffer_len(width, height, stride)?;
        let (Ok(len_i32), Ok(w), Ok(h), Ok(s)) = (
            i32::try_from(len),
            i32::try_from(width),
            i32::try_from(height),
            i32::try_from(stride),
        ) else {
            return Err(CaptureError::TooLarge { width, height });
        };
        let (map, fd) = Mapping::new(len).map_err(|e| {
            tracing::warn!("yakalama tamponu: {e}");
            CaptureError::Buffer
        })?;
        let pool = shm.create_pool(fd.as_fd(), len_i32, qh, NoEvents);
        let wl_format = match wl_shm::Format::try_from(format.shm_code()) {
            Ok(f) => f,
            Err(()) => {
                pool.destroy();
                return Err(CaptureError::NoUsableFormat);
            }
        };
        let buffer = pool.create_buffer(0, w, h, s, wl_format, qh, NoEvents);
        // fd burada kapanır: compositor kendi kopyasını aldı.
        Ok(Self {
            buffer,
            pool,
            map,
            format,
            width,
            height,
            stride,
        })
    }

    /// Wayland nesnelerini bırakır, eşlemeyi döndürür.
    fn into_mapping(self) -> Mapping {
        self.buffer.destroy();
        self.pool.destroy();
        self.map
    }
}

/// Tampon uzunluğu; tavanı ve tutarsızlığı denetler.
pub(crate) fn buffer_len(width: u32, height: u32, stride: usize) -> Result<usize, CaptureError> {
    if width == 0 || height == 0 {
        return Err(CaptureError::Buffer);
    }
    let row = (width as usize)
        .checked_mul(4)
        .ok_or(CaptureError::TooLarge { width, height })?;
    if stride < row {
        return Err(CaptureError::Buffer);
    }
    let len = stride
        .checked_mul(height as usize)
        .ok_or(CaptureError::TooLarge { width, height })?;
    if len > MAX_BUFFER_BYTES {
        return Err(CaptureError::TooLarge { width, height });
    }
    Ok(len)
}

// ---------------------------------------------------------------------------------------------
// Kabul ve kuyruk (saf).
// ---------------------------------------------------------------------------------------------

/// Kuyruktaki istek.
#[derive(Debug, Clone, Copy)]
struct Queued {
    id: CaptureId,
    source: CaptureSource,
    options: CaptureOptions,
}

/// Kabul defteri: yürüyen işler (en çok `MAX_ACTIVE`) ve FIFO kuyruk. Aynı kaynağın ikinci isteği
/// birincisi bitene dek bekler: compositor aynı kaynağa eşzamanlı ikinci oturumda kareyi ancak
/// içerik değişince verebilir (sway 1.12'de durağan pencerede zaman aşımı görüldü).
#[derive(Debug, Default)]
pub(crate) struct Admission {
    running: Vec<(CaptureId, CaptureSource)>,
    queue: VecDeque<Queued>,
}

impl Admission {
    fn source_busy(&self, source: CaptureSource) -> bool {
        self.running.iter().any(|(_, s)| *s == source)
    }

    /// Yeni istek: hemen başlatılabilirse `Some` (yuva ayrıldı), kuyruğa girdiyse `None`,
    /// kuyruk doluysa `Err(Busy)`.
    fn admit(&mut self, q: Queued) -> Result<Option<Queued>, CaptureError> {
        if self.running.len() < MAX_ACTIVE && !self.source_busy(q.source) {
            self.running.push((q.id, q.source));
            Ok(Some(q))
        } else if self.queue.len() < MAX_QUEUED {
            self.queue.push_back(q);
            Ok(None)
        } else {
            Err(CaptureError::Busy)
        }
    }

    /// `id`'nin yuvası boşaldı; başlatılabilecek ilk bekleyen (kaynağı meşgul olmayan) varsa
    /// yuvayı ona verir.
    fn release(&mut self, id: CaptureId) -> Option<Queued> {
        self.running.retain(|(r, _)| *r != id);
        if self.running.len() >= MAX_ACTIVE {
            return None;
        }
        let pos = self
            .queue
            .iter()
            .position(|q| !self.source_busy(q.source))?;
        let next = self.queue.remove(pos)?;
        self.running.push((next.id, next.source));
        Some(next)
    }

    /// Kuyruktan çıkarır (iptal); bulunduysa `true`.
    fn dequeue(&mut self, id: CaptureId) -> bool {
        let before = self.queue.len();
        self.queue.retain(|q| q.id != id);
        before != self.queue.len()
    }
}

// ---------------------------------------------------------------------------------------------
// Context durumu.
// ---------------------------------------------------------------------------------------------

/// ext oturumu.
struct ExtJob {
    source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
    frame: Option<ExtImageCopyCaptureFrameV1>,
    buffer: Option<CaptureBuffer>,
    size: Option<(u32, u32)>,
    formats: Vec<u32>,
    /// Kare yaratıldıktan sonra yeni kısıtlar (`done`) geldi mi.
    fresh_constraints: bool,
    transform: u32,
    attempts: u8,
}

/// wlr-screencopy karesi.
struct WlrJob {
    frame: ZwlrScreencopyFrameV1,
    /// `buffer` olaylarındaki shm önerileri: (biçim, w, h, adım).
    offers: Vec<(u32, u32, u32, u32)>,
    buffer: Option<CaptureBuffer>,
    y_invert: bool,
    /// Çıkışın `wl_output.transform`'u: wlr tamponu çıkışın doğal yönündedir ve dönüşüm
    /// olayı yoktur; ext'in `transform` olayıyla aynı anlamda uygulanır.
    transform: u32,
}

enum Job {
    Ext(ExtJob),
    Wlr(WlrJob),
}

struct Active {
    options: CaptureOptions,
    opaque: bool,
    timer: Option<RegistrationToken>,
    job: Job,
}

/// Çevirme thread'inin sonucu.
type WorkerResult = (CaptureId, Result<CapturedImage, CaptureError>);

/// Context'teki yakalama durumu.
#[derive(Default)]
pub(crate) struct Captures {
    registry: Option<WlRegistry>,
    ext_global: Option<(u32, u32)>,
    ext_output_global: Option<(u32, u32)>,
    ext_toplevel_global: Option<(u32, u32)>,
    wlr_global: Option<(u32, u32)>,
    ext: Option<ExtImageCopyCaptureManagerV1>,
    ext_output: Option<ExtOutputImageCaptureSourceManagerV1>,
    ext_toplevel: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    wlr: Option<ZwlrScreencopyManagerV1>,
    admission: Admission,
    active: HashMap<CaptureId, Active>,
    /// Çevirme thread'indeki işler; değer: iptal edildi mi (sonuç düşülür, yuva thread bitince boşalır).
    converting: HashMap<CaptureId, bool>,
    worker_tx: Option<calloop::channel::Sender<WorkerResult>>,
    /// Uygulamanın bildirdiği dış kilit (logind `LockedHint`); bkz. `set_session_locked_hint`.
    locked_hint: bool,
}

/// Yakalama kapısı: bu süreç kilit istedi/tutuyor (`lock_state() != Unlocked`) ya da uygulama
/// oturumun başka bir kilitleyiciyle kilitli olduğunu bildirdi.
fn capture_blocked(own_lock: bool, locked_hint: bool) -> bool {
    own_lock || locked_hint
}

/// Kapı kapalıyken teslim edilecek sonuç: kilitten önce alınmış kare de verilmez.
fn gate_result(
    blocked: bool,
    result: Result<CapturedImage, CaptureError>,
) -> Result<CapturedImage, CaptureError> {
    if blocked && result.is_ok() {
        Err(CaptureError::Locked)
    } else {
        result
    }
}

impl std::fmt::Debug for Captures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Captures")
            .field("active", &self.active.len())
            .field("converting", &self.converting.len())
            .field("queued", &self.admission.queue.len())
            .finish_non_exhaustive()
    }
}

impl<A: App> Context<A> {
    /// Compositor'ın yakalama desteği (bağlanmaz).
    pub fn capture_support(&self) -> CaptureSupport {
        let c = &self.captures;
        let ext_output = c.ext_global.is_some() && c.ext_output_global.is_some();
        let output = if ext_output && !(force_wlr() && c.wlr_global.is_some()) {
            Some(CaptureBackend::ExtImageCopy)
        } else if c.wlr_global.is_some() {
            Some(CaptureBackend::WlrScreencopy)
        } else {
            None
        };
        CaptureSupport {
            output,
            toplevel: c.ext_global.is_some()
                && c.ext_toplevel_global.is_some()
                && self.toplevels_supported(),
        }
    }

    /// Kaynağın tek karesini ister. Sonuç her istek için tam bir kez `App::captured` ile, bu
    /// çağrının içinden değil, gelir (`capture_cancel` edilmediyse). Aynı anda en çok
    /// `MAX_ACTIVE` yakalama yürür, aynı kaynağın istekleri sırayla; fazlası FIFO bekler.
    pub fn capture(&mut self, source: CaptureSource, options: CaptureOptions) -> CaptureId {
        let id = CaptureId(self.next_id());
        let q = Queued {
            id,
            source,
            options,
        };
        match self.captures.admission.admit(q) {
            Ok(Some(q)) => {
                if !self.capture_start(q) {
                    self.capture_slot_freed(id);
                }
            }
            Ok(None) => tracing::debug!(id = id.0, "yakalama kuyrukta"),
            Err(e) => self.pending.push(RuntimeEvent::Captured(id, Err(e))),
        }
        id
    }

    /// Oturumun dışarıdan (başka bir kilitleyici, `loginctl lock-session`) kilitli olduğunu
    /// bildirir; kaynak logind `LockedHint`'idir ve onu izlemek uygulamanın işidir (runtime D-Bus'a
    /// bağlanmaz). `true` iken yeni yakalama `Locked`, yürüyen ve kuyruktaki işler `Locked` ile
    /// biter, çevrilmekte olan kare teslim edilmez. Varsayılan `false`. Kapalı-güvenli kullanım:
    /// izlemeye başlarken `true`, yalnız okunmuş "kilitsiz" ile `false`; durum okunamıyorsa `true`
    /// kalır (capture.md §4).
    pub fn set_session_locked_hint(&mut self, locked: bool) {
        if self.captures.locked_hint == locked {
            return;
        }
        self.captures.locked_hint = locked;
        tracing::debug!(locked, "yakalama: dış kilit bildirimi");
        if locked {
            self.capture_abort_for_lock();
        }
    }

    pub(crate) fn capture_is_blocked(&self) -> bool {
        capture_blocked(self.lock.is_some(), self.captures.locked_hint)
    }

    /// Kilit başladı: yürüyen işler `Locked` ile biter; boşalan yuvaya geçen bekleyenler
    /// `capture_begin`'de aynı hatayı alır; çevrilenler teslimde `gate_result`'tan geçer.
    pub(crate) fn capture_abort_for_lock(&mut self) {
        let ids: Vec<CaptureId> = self.captures.active.keys().copied().collect();
        for id in ids {
            self.capture_fail(id, CaptureError::Locked);
        }
    }

    /// Yakalamayı iptal eder; sonucu gelmez. Bilinmeyen/bitmiş kimlik yok sayılır. Çevirme
    /// sürüyorsa yuva (ve bellek) thread bitince boşalır.
    pub fn capture_cancel(&mut self, id: CaptureId) {
        if self.captures.admission.dequeue(id) {
            return;
        }
        if let Some(active) = self.captures.active.remove(&id) {
            self.capture_dispose(active);
            self.capture_slot_freed(id);
        } else if let Some(cancelled) = self.captures.converting.get_mut(&id) {
            *cancelled = true;
        }
    }

    /// Yuvası ayrılmış isteği başlatır; başlatılamadıysa hatası kuyruğa konur, `false`.
    fn capture_start(&mut self, q: Queued) -> bool {
        match self.capture_begin(&q) {
            Ok(mut active) => {
                active.timer = self.capture_timer(q.id, q.options.timeout);
                self.captures.active.insert(q.id, active);
                tracing::debug!(id = q.id.0, kind = source_kind(&q.source), "yakalama başladı");
                true
            }
            Err(e) => {
                tracing::debug!(id = q.id.0, error = ?e, "yakalama başlatılamadı");
                self.pending.push(RuntimeEvent::Captured(q.id, Err(e)));
                false
            }
        }
    }

    fn capture_begin(&mut self, q: &Queued) -> Result<Active, CaptureError> {
        if self.capture_is_blocked() {
            return Err(CaptureError::Locked);
        }
        if self.protocols.shm.is_none() {
            return Err(CaptureError::Unsupported);
        }
        let opaque = matches!(q.source, CaptureSource::Output(_));
        let job = match q.source {
            CaptureSource::Output(output) => {
                let (wl, output_transform) = self
                    .outputs
                    .get(&output)
                    .map(|e| (e.wl.clone(), e.transform))
                    .ok_or(CaptureError::UnknownSource)?;
                match self.capture_support().output {
                    Some(CaptureBackend::ExtImageCopy) => {
                        self.capture_bind_ext();
                        let (Some(manager), Some(sources)) =
                            (&self.captures.ext, &self.captures.ext_output)
                        else {
                            return Err(CaptureError::Unsupported);
                        };
                        let source = sources.create_source(&wl, &self.qh, NoEvents);
                        Job::Ext(ext_session(manager, source, q, &self.qh))
                    }
                    Some(CaptureBackend::WlrScreencopy) => {
                        self.capture_bind_wlr();
                        let manager = self
                            .captures
                            .wlr
                            .as_ref()
                            .ok_or(CaptureError::Unsupported)?;
                        let frame = manager.capture_output(
                            i32::from(q.options.paint_cursor),
                            &wl,
                            &self.qh,
                            CaptureData(q.id),
                        );
                        Job::Wlr(WlrJob {
                            frame,
                            offers: Vec::new(),
                            buffer: None,
                            y_invert: false,
                            transform: output_transform,
                        })
                    }
                    None => return Err(CaptureError::Unsupported),
                }
            }
            CaptureSource::Toplevel(toplevel) => {
                if !self.capture_support().toplevel {
                    return Err(CaptureError::Unsupported);
                }
                let handle = self
                    .toplevels
                    .handle(toplevel)
                    .cloned()
                    .ok_or(CaptureError::UnknownSource)?;
                self.capture_bind_ext();
                let (Some(manager), Some(sources)) =
                    (&self.captures.ext, &self.captures.ext_toplevel)
                else {
                    return Err(CaptureError::Unsupported);
                };
                let source = sources.create_source(&handle, &self.qh, NoEvents);
                Job::Ext(ext_session(manager, source, q, &self.qh))
            }
        };
        Ok(Active {
            options: q.options,
            opaque,
            timer: None,
            job,
        })
    }

    fn capture_bind_ext(&mut self) {
        let c = &mut self.captures;
        let Some(registry) = c.registry.as_ref() else {
            return;
        };
        if c.ext.is_none()
            && let Some((name, version)) = c.ext_global
        {
            c.ext = Some(registry.bind::<ExtImageCopyCaptureManagerV1, _, State<A>>(
                name,
                version.min(1),
                &self.qh,
                NoEvents,
            ));
        }
        if c.ext_output.is_none()
            && let Some((name, version)) = c.ext_output_global
        {
            c.ext_output = Some(
                registry.bind::<ExtOutputImageCaptureSourceManagerV1, _, State<A>>(
                    name,
                    version.min(1),
                    &self.qh,
                    NoEvents,
                ),
            );
        }
        if c.ext_toplevel.is_none()
            && let Some((name, version)) = c.ext_toplevel_global
        {
            c.ext_toplevel = Some(registry
                .bind::<ExtForeignToplevelImageCaptureSourceManagerV1, _, State<A>>(
                    name,
                    version.min(1),
                    &self.qh,
                    NoEvents,
                ));
        }
    }

    fn capture_bind_wlr(&mut self) {
        let c = &mut self.captures;
        if c.wlr.is_none()
            && let (Some(registry), Some((name, version))) = (c.registry.as_ref(), c.wlr_global)
        {
            c.wlr = Some(registry.bind::<ZwlrScreencopyManagerV1, _, State<A>>(
                name,
                version.min(3),
                &self.qh,
                NoEvents,
            ));
        }
    }

    fn capture_timer(&mut self, id: CaptureId, after: Duration) -> Option<RegistrationToken> {
        self.handle
            .insert_source(Timer::from_duration(after), move |_, _, state| {
                if let Some(active) = state.cx.captures.active.get_mut(&id) {
                    // Kaynak bu dönüşle zaten düşüyor.
                    active.timer = None;
                    state.cx.capture_fail(id, CaptureError::Timeout);
                }
                TimeoutAction::Drop
            })
            .inspect_err(|e| tracing::error!("yakalama zamanlayıcısı kurulamadı: {e}"))
            .ok()
    }

    /// Yürüyen işin Wayland nesnelerini ve zamanlayıcısını bırakır.
    fn capture_dispose(&mut self, active: Active) {
        if let Some(token) = active.timer {
            self.handle.remove(token);
        }
        match active.job {
            Job::Ext(job) => {
                if let Some(frame) = job.frame {
                    frame.destroy();
                }
                if let Some(buffer) = job.buffer {
                    drop(buffer.into_mapping());
                }
                job.session.destroy();
                job.source.destroy();
            }
            Job::Wlr(job) => {
                job.frame.destroy();
                if let Some(buffer) = job.buffer {
                    drop(buffer.into_mapping());
                }
            }
        }
    }

    /// `id`'nin yuvası boşaldı: başlatılabilen ilk bekleyen başlatılır; başlatılamazsa (hatası
    /// kuyruğa kondu) sıradaki denenir.
    fn capture_slot_freed(&mut self, id: CaptureId) {
        let mut freed = id;
        while let Some(next) = self.captures.admission.release(freed) {
            if self.capture_start(next) {
                break;
            }
            freed = next.id;
        }
    }

    fn capture_fail(&mut self, id: CaptureId, error: CaptureError) {
        if let Some(active) = self.captures.active.remove(&id) {
            tracing::debug!(id = id.0, ?error, "yakalama başarısız");
            self.capture_dispose(active);
            self.pending.push(RuntimeEvent::Captured(id, Err(error)));
            self.capture_slot_freed(id);
        }
    }

    /// Kare hazır: Wayland nesneleri bırakılır, çevirme thread'e verilir.
    fn capture_ready(&mut self, id: CaptureId) {
        let Some(active) = self.captures.active.remove(&id) else {
            return;
        };
        if let Some(token) = active.timer {
            self.handle.remove(token);
        }
        let (buffer, transform, y_invert) = match active.job {
            Job::Ext(job) => {
                if let Some(frame) = job.frame {
                    frame.destroy();
                }
                job.session.destroy();
                job.source.destroy();
                (job.buffer, job.transform, false)
            }
            Job::Wlr(job) => {
                job.frame.destroy();
                (job.buffer, job.transform, job.y_invert)
            }
        };
        let Some(buffer) = buffer else {
            self.pending
                .push(RuntimeEvent::Captured(id, Err(CaptureError::Buffer)));
            self.capture_slot_freed(id);
            return;
        };
        let (format, width, height, stride) =
            (buffer.format, buffer.width, buffer.height, buffer.stride);
        let map = buffer.into_mapping();
        let work = Conversion {
            map,
            format,
            width,
            height,
            stride,
            opaque: active.opaque,
            transform,
            y_invert,
            max: active.options.clamped_max(),
        };
        self.captures.converting.insert(id, false);
        let Some(tx) = self.capture_worker_channel() else {
            let result = work.run();
            self.capture_converted(id, result);
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("nuxut-capture".into())
            .spawn(move || {
                let result = work.run();
                // Runtime kapandıysa sonuç düşer.
                let _ = tx.send((id, result));
            });
        if let Err(e) = spawned {
            tracing::warn!("yakalama thread'i başlatılamadı: {e}");
            self.capture_fail_converting(id);
        }
    }

    fn capture_fail_converting(&mut self, id: CaptureId) {
        if self.captures.converting.remove(&id).is_some() {
            self.pending
                .push(RuntimeEvent::Captured(id, Err(CaptureError::Buffer)));
            self.capture_slot_freed(id);
        }
    }

    fn capture_worker_channel(&mut self) -> Option<calloop::channel::Sender<WorkerResult>> {
        if let Some(tx) = &self.captures.worker_tx {
            return Some(tx.clone());
        }
        let (tx, rx) = calloop::channel::channel::<WorkerResult>();
        let inserted = self.handle.insert_source(rx, |event, _, state| {
            if let calloop::channel::Event::Msg((id, result)) = event {
                state.cx.capture_converted(id, result);
            }
        });
        match inserted {
            Ok(_) => {
                self.captures.worker_tx = Some(tx.clone());
                Some(tx)
            }
            Err(e) => {
                tracing::warn!("yakalama kanalı kurulamadı: {}", e.error);
                None
            }
        }
    }

    fn capture_converted(&mut self, id: CaptureId, result: Result<CapturedImage, CaptureError>) {
        match self.captures.converting.remove(&id) {
            // Kapanışta temizlendi.
            None => return,
            // İptal edildi: sonuç düşer, yuva şimdi boşalır.
            Some(true) => {
                self.capture_slot_freed(id);
                return;
            }
            Some(false) => {}
        }
        let result = gate_result(self.capture_is_blocked(), result);
        match &result {
            Ok(img) => tracing::debug!(
                id = id.0,
                w = img.image.width,
                h = img.image.height,
                "yakalama hazır"
            ),
            Err(e) => tracing::debug!(id = id.0, error = ?e, "yakalama çevrilemedi"),
        }
        self.pending.push(RuntimeEvent::Captured(id, result));
        self.capture_slot_freed(id);
    }

    // --- ext olayları ---

    fn capture_ext_session_event(&mut self, id: CaptureId, event: ext_image_copy_capture_session_v1::Event) {
        use ext_image_copy_capture_session_v1::Event;
        let Some(Active {
            job: Job::Ext(job), ..
        }) = self.captures.active.get_mut(&id)
        else {
            return;
        };
        match event {
            Event::BufferSize { width, height } => job.size = Some((width, height)),
            Event::ShmFormat { format } if job.formats.len() < 64 => {
                job.formats.push(match format {
                    WEnum::Value(f) => f.into(),
                    WEnum::Unknown(u) => u,
                });
            }
            Event::Done => {
                if job.frame.is_some() {
                    job.fresh_constraints = true;
                    return;
                }
                if let Err(e) = self.capture_ext_frame(id) {
                    self.capture_fail(id, e);
                }
            }
            Event::Stopped => self.capture_fail(id, CaptureError::SourceGone),
            _ => {}
        }
    }

    /// Kısıtlar tamam: tampon + kare + yakala.
    fn capture_ext_frame(&mut self, id: CaptureId) -> Result<(), CaptureError> {
        let Some(shm) = self.protocols.shm.clone() else {
            return Err(CaptureError::Unsupported);
        };
        let Some(Active {
            job: Job::Ext(job), ..
        }) = self.captures.active.get_mut(&id)
        else {
            return Ok(());
        };
        let (width, height) = job.size.ok_or(CaptureError::Buffer)?;
        let format =
            capture_pixels::choose_format(job.formats.iter().copied()).ok_or_else(|| {
                tracing::debug!(id = id.0, offered = ?job.formats, "uygun shm biçimi yok");
                CaptureError::NoUsableFormat
            })?;
        let stride = (width as usize)
            .checked_mul(4)
            .ok_or(CaptureError::TooLarge { width, height })?;
        let buffer = CaptureBuffer::new(&shm, &self.qh, format, (width, height), stride)?;
        let frame = job.session.create_frame(&self.qh, CaptureData(id));
        frame.attach_buffer(&buffer.buffer);
        frame.damage_buffer(
            0,
            0,
            i32::try_from(width).unwrap_or(i32::MAX),
            i32::try_from(height).unwrap_or(i32::MAX),
        );
        frame.capture();
        tracing::debug!(id = id.0, width, height, format = format.shm_code(), "ext kare istendi");
        if let Some(old) = job.buffer.replace(buffer) {
            drop(old.into_mapping());
        }
        job.frame = Some(frame);
        job.fresh_constraints = false;
        job.attempts += 1;
        Ok(())
    }

    fn capture_ext_frame_event(&mut self, id: CaptureId, event: ext_image_copy_capture_frame_v1::Event) {
        use ext_image_copy_capture_frame_v1::Event;
        let Some(Active {
            job: Job::Ext(job), ..
        }) = self.captures.active.get_mut(&id)
        else {
            return;
        };
        match event {
            Event::Transform { transform } => {
                job.transform = match transform {
                    WEnum::Value(t) => t.into(),
                    WEnum::Unknown(u) => u,
                };
            }
            Event::Ready => self.capture_ready(id),
            Event::Failed { reason } => {
                let reason = match reason {
                    WEnum::Value(r) => Some(r),
                    WEnum::Unknown(_) => None,
                };
                match reason {
                    Some(FailureReason::BufferConstraints) if job.attempts < MAX_ATTEMPTS => {
                        if let Some(frame) = job.frame.take() {
                            frame.destroy();
                        }
                        tracing::debug!(id = id.0, "tampon kısıtları değişti; yeniden deneniyor");
                        if job.fresh_constraints
                            && let Err(e) = self.capture_ext_frame(id)
                        {
                            self.capture_fail(id, e);
                        }
                        // Aksi hâlde sonraki `done` yeniden dener.
                    }
                    Some(FailureReason::Stopped) => self.capture_fail(id, CaptureError::SourceGone),
                    _ => self.capture_fail(id, CaptureError::Failed),
                }
            }
            _ => {}
        }
    }

    // --- wlr olayları ---

    fn capture_wlr_event(&mut self, id: CaptureId, event: zwlr_screencopy_frame_v1::Event) {
        use zwlr_screencopy_frame_v1::Event;
        let Some(Active {
            job: Job::Wlr(job), ..
        }) = self.captures.active.get_mut(&id)
        else {
            return;
        };
        match event {
            Event::Buffer {
                format,
                width,
                height,
                stride,
            } => {
                let code = match format {
                    WEnum::Value(f) => f.into(),
                    WEnum::Unknown(u) => u,
                };
                if job.offers.len() < 16 {
                    job.offers.push((code, width, height, stride));
                }
                // Sürüm < 3: `buffer_done` yok; ilk öneriyle kopyalanır.
                if job.frame.version() < 3
                    && let Err(e) = self.capture_wlr_copy(id)
                {
                    self.capture_fail(id, e);
                }
            }
            Event::BufferDone => {
                if let Err(e) = self.capture_wlr_copy(id) {
                    self.capture_fail(id, e);
                }
            }
            Event::Flags {
                flags: WEnum::Value(f),
            } => job.y_invert = f.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
            Event::Ready { .. } => self.capture_ready(id),
            Event::Failed => self.capture_fail(id, CaptureError::Failed),
            _ => {}
        }
    }

    fn capture_wlr_copy(&mut self, id: CaptureId) -> Result<(), CaptureError> {
        let Some(shm) = self.protocols.shm.clone() else {
            return Err(CaptureError::Unsupported);
        };
        let Some(Active {
            job: Job::Wlr(job), ..
        }) = self.captures.active.get_mut(&id)
        else {
            return Ok(());
        };
        if job.buffer.is_some() {
            return Ok(());
        }
        let format = capture_pixels::choose_format(job.offers.iter().map(|o| o.0))
            .ok_or(CaptureError::NoUsableFormat)?;
        let &(_, width, height, stride) = job
            .offers
            .iter()
            .find(|o| PixelFormat::from_shm(o.0) == Some(format))
            .ok_or(CaptureError::NoUsableFormat)?;
        let buffer = CaptureBuffer::new(&shm, &self.qh, format, (width, height), stride as usize)?;
        job.frame.copy(&buffer.buffer);
        tracing::debug!(id = id.0, width, height, format = format.shm_code(), "wlr kare istendi");
        job.buffer = Some(buffer);
        Ok(())
    }

    /// Kapanış: yürüyen işler sessizce bırakılır, yöneticiler yok edilir.
    pub(crate) fn capture_shutdown(&mut self) {
        let ids: Vec<CaptureId> = self.captures.active.keys().copied().collect();
        for id in ids {
            if let Some(active) = self.captures.active.remove(&id) {
                self.capture_dispose(active);
            }
        }
        self.captures.admission.queue.clear();
        self.captures.converting.clear();
        let c = &mut self.captures;
        if let Some(m) = c.ext.take() {
            m.destroy();
        }
        if let Some(m) = c.ext_output.take() {
            m.destroy();
        }
        if let Some(m) = c.ext_toplevel.take() {
            m.destroy();
        }
        if let Some(m) = c.wlr.take() {
            m.destroy();
        }
    }
}

fn source_kind(source: &CaptureSource) -> &'static str {
    match source {
        CaptureSource::Output(_) => "output",
        CaptureSource::Toplevel(_) => "toplevel",
    }
}

fn ext_session<A: App>(
    manager: &ExtImageCopyCaptureManagerV1,
    source: ExtImageCaptureSourceV1,
    q: &Queued,
    qh: &QueueHandle<State<A>>,
) -> ExtJob {
    let options = if q.options.paint_cursor {
        Options::PaintCursors
    } else {
        Options::empty()
    };
    let session = manager.create_session(&source, options, qh, CaptureData(q.id));
    ExtJob {
        source,
        session,
        frame: None,
        buffer: None,
        size: None,
        formats: Vec::new(),
        fresh_constraints: false,
        transform: 0,
        attempts: 0,
    }
}

/// Thread'de yapılacak çevirme.
struct Conversion {
    map: Mapping,
    format: PixelFormat,
    width: u32,
    height: u32,
    stride: usize,
    opaque: bool,
    transform: u32,
    y_invert: bool,
    max: (u32, u32),
}

impl Conversion {
    fn run(self) -> Result<CapturedImage, CaptureError> {
        let src = Source {
            bytes: self.map.bytes(),
            width: self.width,
            height: self.height,
            stride: self.stride,
            format: self.format,
            opaque: self.opaque,
        };
        let (pixels, w, h) =
            capture_pixels::render_preview(&src, self.transform, self.y_invert, self.max)
                .ok_or(CaptureError::Buffer)?;
        Ok(CapturedImage {
            image: ImageData {
                width: w,
                height: h,
                pixels: pixels.into(),
            },
            source_size: (self.width, self.height),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Dispatch.
// ---------------------------------------------------------------------------------------------

/// Oturum/kare nesnelerinin verisi: hangi istek.
#[derive(Debug)]
pub(crate) struct CaptureData(CaptureId);

impl<A: App> Dispatch2<ExtImageCopyCaptureSessionV1, State<A>> for CaptureData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        state.cx.capture_ext_session_event(self.0, event);
    }
}

impl<A: App> Dispatch2<ExtImageCopyCaptureFrameV1, State<A>> for CaptureData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        state.cx.capture_ext_frame_event(self.0, event);
    }
}

impl<A: App> Dispatch2<ZwlrScreencopyFrameV1, State<A>> for CaptureData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        state.cx.capture_wlr_event(self.0, event);
    }
}

macro_rules! no_events {
    ($($ty:ty),* $(,)?) => {$(
        impl<D> Dispatch2<$ty, D> for NoEvents {
            fn event(
                &self,
                _: &mut D,
                _: &$ty,
                _: <$ty as Proxy>::Event,
                _: &Connection,
                _: &QueueHandle<D>,
            ) {
            }
        }
    )*};
}

no_events!(
    ExtImageCopyCaptureManagerV1,
    ExtOutputImageCaptureSourceManagerV1,
    ExtForeignToplevelImageCaptureSourceManagerV1,
    ExtImageCaptureSourceV1,
    ZwlrScreencopyManagerV1,
    WlBuffer,
);

/// Global'lerin kaydı; bağlanma ilk yakalamada.
#[derive(Debug)]
pub(crate) struct CaptureGlobal;

impl CaptureGlobal {
    pub(crate) fn record<A: App>(state: &mut State<A>, interface: &str, name: u32, version: u32) {
        let c = &mut state.cx.captures;
        let slot = match interface {
            EXT_MANAGER_INTERFACE => &mut c.ext_global,
            EXT_OUTPUT_SOURCE_INTERFACE => &mut c.ext_output_global,
            EXT_TOPLEVEL_SOURCE_INTERFACE => &mut c.ext_toplevel_global,
            WLR_INTERFACE => &mut c.wlr_global,
            _ => return,
        };
        if slot.is_none() {
            *slot = Some((name, version));
        }
        let registry = state.registry().registry().clone();
        state.cx.captures.registry.get_or_insert(registry);
    }
}

impl<A: App> RegistryHandler<State<A>> for CaptureGlobal {
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
        // Bağlı yönetici bırakılır; yürüyen oturumlar compositor'dan `stopped`/`failed` alır,
        // yoksa zaman aşımıyla biter.
        let c = &mut state.cx.captures;
        let gone = |g: &mut Option<(u32, u32)>| {
            let hit = g.is_some_and(|(n, _)| n == name);
            if hit {
                *g = None;
            }
            hit
        };
        if gone(&mut c.ext_global)
            && let Some(m) = c.ext.take()
        {
            m.destroy();
        }
        if gone(&mut c.ext_output_global)
            && let Some(m) = c.ext_output.take()
        {
            m.destroy();
        }
        if gone(&mut c.ext_toplevel_global)
            && let Some(m) = c.ext_toplevel.take()
        {
            m.destroy();
        }
        if gone(&mut c.wlr_global)
            && let Some(m) = c.wlr.take()
        {
            m.destroy();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Her istek kendi çıkışını yakalar (kaynak çakışması yok).
    fn q(n: u64) -> Queued {
        qs(n, n as u32)
    }

    fn qs(n: u64, output: u32) -> Queued {
        Queued {
            id: CaptureId(n),
            source: CaptureSource::Output(OutputId(output)),
            options: CaptureOptions::default(),
        }
    }

    #[test]
    fn admission_runs_limited_and_queues_fifo() {
        let mut a = Admission::default();
        for n in 0..MAX_ACTIVE as u64 {
            assert!(a.admit(q(n)).unwrap().is_some());
        }
        assert!(a.admit(q(100)).unwrap().is_none(), "kuyruğa girer");
        assert!(a.admit(q(101)).unwrap().is_none());
        // İptal edilen kuyruktaki iş atlanır; FIFO.
        assert!(a.dequeue(CaptureId(100)));
        assert!(!a.dequeue(CaptureId(100)));
        assert_eq!(a.release(CaptureId(0)).map(|q| q.id), Some(CaptureId(101)));
        assert_eq!(a.release(CaptureId(1)).map(|q| q.id), None);
        assert_eq!(a.running.len(), MAX_ACTIVE - 1);
    }

    #[test]
    fn same_source_waits_for_the_previous_capture() {
        let mut a = Admission::default();
        assert!(a.admit(qs(1, 7)).unwrap().is_some());
        // Yuva boş ama kaynak meşgul: bekler; başka kaynak öne geçer.
        assert!(a.admit(qs(2, 7)).unwrap().is_none());
        assert!(a.admit(qs(3, 8)).unwrap().is_some());
        assert!(a.admit(qs(4, 9)).unwrap().is_none());
        // 3 bitti: 2 hâlâ meşgul kaynakta, 4 başlar.
        assert_eq!(a.release(CaptureId(3)).map(|q| q.id), Some(CaptureId(4)));
        // 1 bitti: 2 başlar.
        assert_eq!(a.release(CaptureId(1)).map(|q| q.id), Some(CaptureId(2)));
        assert!(a.queue.is_empty());
    }

    #[test]
    fn admission_rejects_when_queue_full() {
        let mut a = Admission::default();
        for n in 0..(MAX_ACTIVE + MAX_QUEUED) as u64 {
            assert!(a.admit(q(n)).is_ok());
        }
        assert_eq!(a.admit(q(9999)).unwrap_err(), CaptureError::Busy);
    }

    #[test]
    fn buffer_len_enforces_ceiling_and_consistency() {
        assert_eq!(buffer_len(1920, 1080, 1920 * 4), Ok(1920 * 1080 * 4));
        assert_eq!(buffer_len(7680, 4320, 7680 * 4), Ok(MAX_BUFFER_BYTES));
        assert_eq!(
            buffer_len(7681, 4320, 7681 * 4),
            Err(CaptureError::TooLarge {
                width: 7681,
                height: 4320
            })
        );
        assert_eq!(buffer_len(10, 10, 39), Err(CaptureError::Buffer), "dar adım");
        assert_eq!(buffer_len(0, 10, 40), Err(CaptureError::Buffer));
        assert!(matches!(
            buffer_len(u32::MAX, u32::MAX, usize::MAX),
            Err(CaptureError::TooLarge { .. })
        ));
    }

    #[test]
    fn options_clamp_and_preview() {
        let o = CaptureOptions::preview(0, 100_000);
        assert_eq!(o.clamped_max(), (1, MAX_TEXTURE_SIDE));
        assert!(!o.paint_cursor);
        assert_eq!(o.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn lock_gate_blocks_on_own_lock_or_external_hint() {
        assert!(!capture_blocked(false, false));
        assert!(capture_blocked(true, false));
        assert!(capture_blocked(false, true));
        assert!(capture_blocked(true, true));
    }

    #[test]
    fn lock_gate_withholds_frames_taken_before_lock() {
        let img = || {
            Ok(CapturedImage {
                image: ImageData {
                    width: 1,
                    height: 1,
                    pixels: vec![1, 2, 3, 4].into(),
                },
                source_size: (1, 1),
            })
        };
        assert_eq!(gate_result(true, img()).unwrap_err(), CaptureError::Locked);
        assert!(gate_result(false, img()).is_ok());
        // Hata türü korunur (kilit başka bir hatayı örtmez).
        assert_eq!(
            gate_result(true, Err(CaptureError::Timeout)).unwrap_err(),
            CaptureError::Timeout
        );
    }

    #[test]
    fn captured_image_debug_is_redacted() {
        let img = CapturedImage {
            image: ImageData {
                width: 2,
                height: 1,
                pixels: vec![201, 202, 203, 204, 205, 206, 207, 208].into(),
            },
            source_size: (4, 2),
        };
        let shown = format!("{img:?}");
        assert_eq!(shown, "CapturedImage(2x1 ← 4x2)");
        let event = format!("{:?}", RuntimeEvent::Captured(CaptureId(1), Ok(img)));
        assert!(!event.contains("201"), "{event}");
    }

    #[test]
    fn conversion_reads_mapping() {
        // Gerçek memfd eşlemesi: sıfırlarla dolu 2×2 ARGB → siyah, opak çıkış.
        let (map, _fd) = Mapping::new(16).unwrap();
        let img = Conversion {
            map,
            format: PixelFormat::Argb8888,
            width: 2,
            height: 2,
            stride: 8,
            opaque: true,
            transform: 0,
            y_invert: false,
            max: (1, 1),
        }
        .run()
        .unwrap();
        assert_eq!((img.image.width, img.image.height), (1, 1));
        assert_eq!(&img.image.pixels[..], &[0, 0, 0, 255]);
        assert_eq!(img.source_size, (2, 2));
    }
}
