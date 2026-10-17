//! EGL: tek display, tek GLES 3.0 context, yüzey başına pencere yüzeyi.
//!
//! `libEGL.so.1` çalışma zamanında yüklenir (derleme anında GL başlığı/kütüphanesi
//! gerekmez). Tüm yüzeyler aynı context'i paylaşır; shader, atlas ve tamponlar bir kez
//! yaratılır.

use std::ffi::c_void;

use khronos_egl as egl;
use nuxut_render::glow;
use wayland_client::{Connection, Proxy, protocol::wl_surface::WlSurface};
use wayland_egl::WlEglSurface;

use crate::RunError;

type Instance = egl::DynamicInstance<egl::EGL1_4>;

/// `EGL_KHR_platform_wayland`.
const PLATFORM_WAYLAND_KHR: egl::Enum = 0x31D8;

pub(crate) struct Egl {
    instance: Instance,
    display: egl::Display,
    config: egl::Config,
    context: egl::Context,
    /// `EGL_EXT_buffer_age`.
    buffer_age: bool,
    /// `eglSwapBuffersWithDamage{KHR,EXT}` (ikisinin imzası aynı).
    swap_with_damage: Option<SwapWithDamageFn>,
    /// Canlı EGL pencere yüzeyi sayısı (sızıntı denetimi).
    live: std::cell::Cell<usize>,
}

/// `EGLBoolean eglSwapBuffersWithDamageKHR(EGLDisplay, EGLSurface, const EGLint *rects, EGLint n)`.
type SwapWithDamageFn = unsafe extern "system" fn(
    egl::EGLDisplay,
    egl::EGLSurface,
    *const egl::Int,
    egl::Int,
) -> egl::Boolean;

/// `EGL_BUFFER_AGE_EXT`.
const BUFFER_AGE_EXT: egl::Int = 0x313D;

impl std::fmt::Debug for Egl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Egl")
            .field("display", &self.display)
            .finish_non_exhaustive()
    }
}

/// Bir `wl_surface`'e bağlı EGL pencere yüzeyi. `Egl::destroy_surface` ile yok edilmeli;
/// `wl_egl_window` (`window`) `wl_surface`'ten önce düşmelidir.
#[derive(Debug)]
pub(crate) struct GpuSurface {
    surface: egl::Surface,
    window: WlEglSurface,
    size: (i32, i32),
    first_use: bool,
}

impl GpuSurface {
    /// İlk kez bağlanıyorsa `true` döner (yüzey başına bir kez yapılacak ayarlar için).
    pub(crate) fn take_first_use(&mut self) -> bool {
        std::mem::replace(&mut self.first_use, false)
    }

    /// Fiziksel boyutu değiştirir; bir sonraki `swap_buffers` yeni boyutta tampon üretir.
    pub(crate) fn resize(&mut self, size: (i32, i32)) {
        if size != self.size {
            self.window.resize(size.0, size.1, 0, 0);
            self.size = size;
        }
    }
}

impl Egl {
    pub(crate) fn new(conn: &Connection) -> Result<Self, RunError> {
        // SAFETY: libEGL.so.1 sistem EGL kütüphanesidir ve EGL 1.4 ABI'sini sağlar;
        // `load_required` eksik sembolde hata döner, sahte kütüphane dışında UB yoktur.
        let instance = unsafe { Instance::load_required() }
            .map_err(|e| RunError::Egl(format!("libEGL yüklenemedi: {e}")))?;

        let wl_display = conn.backend().display_ptr().cast::<c_void>();
        let display = Self::platform_display(&instance, wl_display)?;
        instance
            .initialize(display)
            .map_err(|e| RunError::Egl(format!("eglInitialize: {e}")))?;
        instance
            .bind_api(egl::OPENGL_ES_API)
            .map_err(|e| RunError::Egl(format!("eglBindAPI: {e}")))?;

        let config_attrs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES3_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            // Premultiplied alpha'lı şeffaf yüzeyler için zorunlu.
            egl::ALPHA_SIZE,
            8,
            egl::NONE,
        ];
        let config = instance
            .choose_first_config(display, &config_attrs)
            .map_err(|e| RunError::Egl(format!("eglChooseConfig: {e}")))?
            .ok_or_else(|| RunError::Egl("RGBA8 + GLES3 config yok".into()))?;

        let context_attrs = [
            egl::CONTEXT_MAJOR_VERSION,
            3,
            egl::CONTEXT_MINOR_VERSION,
            0,
            egl::NONE,
        ];
        let context = instance
            .create_context(display, config, None, &context_attrs)
            .map_err(|e| RunError::Egl(format!("eglCreateContext (GLES 3.0): {e}")))?;

        let extensions = instance
            .query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let has = |name: &str| extensions.split_ascii_whitespace().any(|e| e == name);
        let buffer_age = has("EGL_EXT_buffer_age");
        let swap_with_damage = [
            (
                "EGL_KHR_swap_buffers_with_damage",
                "eglSwapBuffersWithDamageKHR",
            ),
            (
                "EGL_EXT_swap_buffers_with_damage",
                "eglSwapBuffersWithDamageEXT",
            ),
        ]
        .into_iter()
        .filter(|(ext, _)| has(ext))
        .find_map(|(_, func)| instance.get_proc_address(func))
        // SAFETY: eklenti display'de ilan edilmiş; bu ad için EGL'nin döndürdüğü gösterici
        // eklentinin tanımladığı imzaya (SwapWithDamageFn) sahiptir.
        .map(|f| unsafe { std::mem::transmute::<extern "system" fn(), SwapWithDamageFn>(f) });
        tracing::debug!(
            buffer_age,
            swap_with_damage = swap_with_damage.is_some(),
            "EGL hasar eklentileri"
        );

        Ok(Self {
            instance,
            display,
            config,
            context,
            buffer_age,
            swap_with_damage,
            live: std::cell::Cell::new(0),
        })
    }

    /// Bağlı yüzeyin arka tamponunun yaşı (`make_current`'tan sonra). Bilinmiyorsa 0.
    pub(crate) fn buffer_age(&self, gpu: &GpuSurface) -> u32 {
        if !self.buffer_age {
            return 0;
        }
        self.instance
            .query_surface(self.display, gpu.surface, BUFFER_AGE_EXT)
            .map_or(0, |age| age.max(0) as u32)
    }

    /// Yalnızca `rects` (her biri sol-alt orijinli `[x, y, w, h]`, fiziksel) hasarlı olarak
    /// gönderir; Mesa her birini ayrı `wl_surface.damage_buffer` yapar. Eklenti yoksa ya da
    /// liste boşsa normal swap.
    pub(crate) fn swap_buffers_with_damage(
        &self,
        gpu: &GpuSurface,
        rects: &[[i32; 4]],
    ) -> Result<(), egl::Error> {
        let Some(swap) = self.swap_with_damage.filter(|_| !rects.is_empty()) else {
            return self.swap_buffers(gpu);
        };
        // SAFETY: display/yüzey canlı ve bu context'e ait; `rects` ardışık `rects.len()` adet
        // 4 elemanlı i32 dizisi (düz `EGLint` dizisi), çağrı süresince yaşar.
        let ok = unsafe {
            swap(
                self.display.as_ptr(),
                gpu.surface.as_ptr(),
                rects.as_ptr().cast::<i32>(),
                rects.len() as i32,
            )
        };
        if ok == egl::TRUE {
            Ok(())
        } else {
            Err(self.instance.get_error().unwrap_or(egl::Error::BadSurface))
        }
    }

    fn platform_display(
        instance: &Instance,
        wl_display: *mut c_void,
    ) -> Result<egl::Display, RunError> {
        if let Some(egl15) = instance.upcast::<egl::EGL1_5>() {
            // SAFETY: `wl_display` canlı bir libwayland-client `wl_display*`'idir ve bağlantı
            // (dolayısıyla gösterici) `Egl`'den uzun yaşar (runtime önce Egl'i düşürür).
            match unsafe {
                egl15.get_platform_display(PLATFORM_WAYLAND_KHR, wl_display, &[egl::ATTRIB_NONE])
            } {
                Ok(display) => return Ok(display),
                Err(e) => tracing::debug!(
                    "eglGetPlatformDisplay başarısız ({e}); eglGetDisplay deneniyor"
                ),
            }
        }
        // SAFETY: yukarıdakiyle aynı gösterici; Mesa wl_display'i arayüz göstericisinden tanır.
        unsafe { instance.get_display(wl_display) }
            .ok_or_else(|| RunError::Egl("Wayland EGLDisplay alınamadı".into()))
    }

    pub(crate) fn create_surface(
        &self,
        wl_surface: &WlSurface,
        size: (i32, i32),
    ) -> Result<GpuSurface, String> {
        let size = (size.0.max(1), size.1.max(1));
        let window =
            WlEglSurface::new(wl_surface.id(), size.0, size.1).map_err(|e| e.to_string())?;
        // SAFETY: `window.ptr()` az önce yaratılmış geçerli bir `wl_egl_window*`; display ve
        // config bu Wayland bağlantısından. `window`, EGL yüzeyiyle birlikte `GpuSurface`'te
        // tutulur ve yüzey yok edilmeden düşmez.
        let surface = unsafe {
            self.instance.create_window_surface(
                self.display,
                self.config,
                window.ptr() as egl::NativeWindowType,
                None,
            )
        }
        .map_err(|e| format!("eglCreateWindowSurface: {e}"))?;
        self.live.set(self.live.get() + 1);
        Ok(GpuSurface {
            surface,
            window,
            size,
            first_use: true,
        })
    }

    pub(crate) fn live_surfaces(&self) -> usize {
        self.live.get()
    }

    pub(crate) fn destroy_surface(&self, gpu: GpuSurface) {
        self.live.set(self.live.get().saturating_sub(1));
        // Güncel yüzeyi yok etmeden önce context'i yüzeysiz yap.
        if self.instance.get_current_surface(egl::DRAW) == Some(gpu.surface) {
            let _ = self
                .instance
                .make_current(self.display, None, None, Some(self.context));
        }
        if let Err(e) = self.instance.destroy_surface(self.display, gpu.surface) {
            tracing::warn!("eglDestroySurface: {e}");
        }
        // `gpu.window` burada düşer: wl_egl_window_destroy, wl_surface'ten önce.
        drop(gpu.window);
    }

    /// Context'i yüzeye bağlar. Başarısızsa kare atlanır (compositor yüzeyi yok etmiş olabilir).
    pub(crate) fn make_current(&self, gpu: &GpuSurface) -> Result<(), egl::Error> {
        self.instance.make_current(
            self.display,
            Some(gpu.surface),
            Some(gpu.surface),
            Some(self.context),
        )
    }

    /// Context'i yüzeysiz güncel yapar (GL nesnelerini yüzeyler gittikten sonra silmek için).
    pub(crate) fn make_current_surfaceless(&self) -> Result<(), egl::Error> {
        self.instance
            .make_current(self.display, None, None, Some(self.context))
    }

    /// Kare zamanlaması frame callback ile yapılır; EGL'nin kendi beklemesini kapat.
    pub(crate) fn disable_vsync(&self) {
        if let Err(e) = self.instance.swap_interval(self.display, 0) {
            tracing::debug!("eglSwapInterval(0): {e}");
        }
    }

    pub(crate) fn swap_buffers(&self, gpu: &GpuSurface) -> Result<(), egl::Error> {
        self.instance.swap_buffers(self.display, gpu.surface)
    }

    /// Context güncelken çağrılmalı.
    pub(crate) fn load_gl(&self) -> glow::Context {
        // SAFETY: context bu thread'de güncel; `get_proc_address` dönen göstericiler bu
        // context için geçerli GLES fonksiyonlarıdır.
        unsafe {
            glow::Context::from_loader_function(|name| {
                self.instance
                    .get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const c_void)
            })
        }
    }
}

impl Drop for Egl {
    fn drop(&mut self) {
        let _ = self.instance.make_current(self.display, None, None, None);
        let _ = self.instance.destroy_context(self.display, self.context);
        let _ = self.instance.terminate(self.display);
    }
}
