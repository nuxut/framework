//! Yazılım rasterleştiricide (llvmpipe/softpipe) sunum: GL çizimi yüzey başına bir FBO'ya
//! yapılır, yalnızca değişen bölge `glReadPixels` ile kendi `wl_shm` tamponlarımıza kopyalanır.
//!
//! Neden: Mesa'nın Wayland swrast yolu her `eglSwapBuffers`'ta (hasarlı swap'ta da) tüm arka
//! tamponu shm tamponuna kopyalar; 1920×1080 yüzeyde saniyelik saat tıkı bile ~4 ms CPU
//! (ölçüm: `damage_bench`, `docs/ARCHITECTURE.md` "Yazılım sunumu"). Burada FBO hep son kareyi
//! taşır (yaşı 1: renderer yalnızca bu karenin hasarını çizer); shm tamponunun yaşına göre
//! biriken bölge ([`DamageHistory`]) okunur ve compositor'a yalnızca bu karenin hasarı bildirilir.
//!
//! GPU sürücülerinde kullanılmaz (EGL pencere yüzeyi + buffer age zaten sıfır kopya).

use std::os::fd::AsFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nuxut_render::glow::{self, HasContext};
use nuxut_render::{DamageHistory, PxRect, Repaint};
use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::protocol::{
    wl_buffer::WlBuffer, wl_shm, wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
};
use wayland_client::{Connection, Proxy, QueueHandle};

use crate::state::NoEvents;

/// Aynı anda en çok bu kadar shm tamponu (compositor genelde biri bırakılmadan diğerini ister).
const MAX_BUFFERS: usize = 3;

/// Yazılım sunumu seçimi: `NUXUT_SOFTWARE_PRESENT=1` her zaman, `0` hiçbir zaman; yoksa
/// GL_RENDERER bir yazılım rasterleştiricisiyse.
pub(crate) fn wanted(renderer: &str, env: Option<&str>) -> bool {
    match env.map(str::trim) {
        Some("1") => true,
        Some("0") => false,
        _ => {
            let r = renderer.to_ascii_lowercase();
            ["llvmpipe", "softpipe", "swrast"]
                .iter()
                .any(|s| r.contains(s))
        }
    }
}

/// Bir shm tamponunun olay verisi: compositor `release` gönderince meşgul bayrağı iner.
#[derive(Debug)]
pub(crate) struct BufferData(Arc<AtomicBool>);

impl<D> Dispatch2<WlBuffer, D> for BufferData {
    fn event(
        &self,
        _: &mut D,
        _: &WlBuffer,
        event: wayland_client::protocol::wl_buffer::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
        if let wayland_client::protocol::wl_buffer::Event::Release = event {
            self.0.store(false, Ordering::Release);
        }
    }
}

impl<D> Dispatch2<WlShmPool, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &WlShmPool,
        _: <WlShmPool as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// Kendi bellek eşlemesiyle bir `wl_buffer` (ARGB8888, premultiplied).
#[derive(Debug)]
struct ShmBuffer {
    buffer: WlBuffer,
    pool: WlShmPool,
    map: *mut u8,
    len: usize,
    size: (u32, u32),
    busy: Arc<AtomicBool>,
    /// Bu tampona en son yazılan sunumun sırası (`Swapchain::presented`); hiç yazılmadıysa `None`.
    written: Option<u64>,
}

impl ShmBuffer {
    fn new<D>(shm: &WlShm, qh: &QueueHandle<D>, size: (u32, u32)) -> Result<Self, String>
    where
        D: 'static,
        D: wayland_client::Dispatch<WlShmPool, NoEvents>
            + wayland_client::Dispatch<WlBuffer, BufferData>,
    {
        let (w, h) = (size.0.max(1), size.1.max(1));
        let stride = w.checked_mul(4).ok_or("tampon çok geniş")?;
        let len = (stride as usize)
            .checked_mul(h as usize)
            .ok_or("tampon çok büyük")?;
        let len_i32 = i32::try_from(len).map_err(|_| "tampon 2 GiB'ı aşıyor")?;
        let fd = rustix::fs::memfd_create("nuxut-shm", rustix::fs::MemfdFlags::CLOEXEC)
            .map_err(|e| format!("memfd_create: {e}"))?;
        rustix::fs::ftruncate(&fd, len as u64).map_err(|e| format!("ftruncate: {e}"))?;
        // SAFETY: yeni, `len` bayta büyütülmüş, yalnızca bizim tuttuğumuz memfd; eşleme `Drop`'ta
        // aynı uzunlukla kaldırılır ve gösterici başka yere verilmez.
        let map = unsafe {
            rustix::mm::mmap(
                std::ptr::null_mut(),
                len,
                rustix::mm::ProtFlags::READ | rustix::mm::ProtFlags::WRITE,
                rustix::mm::MapFlags::SHARED,
                &fd,
                0,
            )
        }
        .map_err(|e| format!("mmap: {e}"))?
        .cast::<u8>();
        let pool = shm.create_pool(fd.as_fd(), len_i32, qh, NoEvents);
        let busy = Arc::new(AtomicBool::new(false));
        let buffer = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            qh,
            BufferData(busy.clone()),
        );
        // fd burada kapanır: compositor kendi kopyasını aldı, eşleme fd'den bağımsız yaşar.
        Ok(Self {
            buffer,
            pool,
            map,
            len,
            size: (w, h),
            busy,
            written: None,
        })
    }

    fn pixels(&mut self) -> &mut [u8] {
        // SAFETY: `map` `len` baytlık, yazılabilir, bu tampon yaşadıkça geçerli eşleme; `&mut
        // self` dışında Rust tarafında başka erişim yok (compositor yalnızca meşgulken okur ve
        // meşgul tampona yazılmaz).
        unsafe { std::slice::from_raw_parts_mut(self.map, self.len) }
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        // Meşgul de olsa yok edilebilir (protokol: compositor içeriği zaten almış olur).
        self.buffer.destroy();
        self.pool.destroy();
        // SAFETY: `new`'deki eşlemenin kendisi; bir kez, aynı uzunlukla kaldırılır.
        if let Err(e) = unsafe { rustix::mm::munmap(self.map.cast(), self.len) } {
            tracing::warn!("munmap: {e}");
        }
    }
}

/// Sıradaki tampon seçimi (saf): boş ve doğru boyutlu ilk tampon; yoksa yeni tampon yeri.
/// `None`: hepsi meşgul ve üst sınırda.
fn pick(busy: &[bool]) -> Option<usize> {
    busy.iter()
        .position(|b| !b)
        .or((busy.len() < MAX_BUFFERS).then_some(busy.len()))
}

/// Tamponun yaşı (EGL `buffer_age` anlamı): 0 = içerik bilinmiyor, 1 = son sunum, ...
fn age(written: Option<u64>, presented: u64) -> u32 {
    written.map_or(0, |w| u32::try_from(presented + 1 - w).unwrap_or(u32::MAX))
}

/// GL'den okunan alt-üst (sol-alt orijinli) RGBA/BGRA satırlarını shm tamponuna üst-alt yazar.
/// `swizzle`: kaynak RGBA ise BGRA'ya çevirir (ARGB8888 küçük-uçlu bellekte B,G,R,A).
fn copy_rows(src: &[u8], region: PxRect, dst: &mut [u8], dst_width: u32, swizzle: bool) {
    let (w, h) = (region.width as usize, region.height as usize);
    let row = w * 4;
    let stride = dst_width as usize * 4;
    for i in 0..h {
        let s = &src[(h - 1 - i) * row..][..row];
        let off = (region.y as usize + i) * stride + region.x as usize * 4;
        let d = &mut dst[off..off + row];
        if swizzle {
            for (d, s) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
        } else {
            d.copy_from_slice(s);
        }
    }
}

/// [`ShmSurface::present`]'in sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presented {
    /// Tamponlu commit yapıldı.
    Buffer,
    /// Hiçbir şey değişmedi: tamponsuz commit (frame callback ve bekleyen durum gitsin diye).
    Unchanged,
    /// Tüm tamponlar compositor'da: tamponsuz commit; çağıran bir kare daha istemeli (sonraki
    /// sunum tüm kareyi kopyalar).
    Deferred,
}

/// Bir yüzeyin yazılım sunum durumu: çizim hedefi FBO ve shm tampon zinciri.
#[derive(Debug)]
pub(crate) struct ShmSurface {
    fbo: Option<(glow::Framebuffer, glow::Renderbuffer)>,
    fbo_size: (u32, u32),
    buffers: Vec<ShmBuffer>,
    /// Yapılan sunum (tamponlu commit) sayısı.
    presented: u64,
    /// shm tamponları için hasar geçmişi (FBO'nunki `SurfaceEntry::damage`).
    history: DamageHistory,
    scratch: Vec<u8>,
    /// FBO'da henüz hiçbir shm tamponuna gitmemiş içerik var (`Presented::Deferred`).
    stale: bool,
}

impl ShmSurface {
    pub(crate) fn new() -> Self {
        Self {
            fbo: None,
            fbo_size: (0, 0),
            buffers: Vec::new(),
            presented: 0,
            history: DamageHistory::new(),
            scratch: Vec::new(),
            stale: false,
        }
    }

    /// FBO'yu (gerekirse yeni boyutta) bağlar ve çizim için tampon yaşını döndürür: yeni FBO 0
    /// (tüm kare), aksi halde 1 (FBO hep son kareyi taşır).
    pub(crate) fn bind(&mut self, gl: &glow::Context, size: (u32, u32)) -> Result<u32, String> {
        let size = (size.0.max(1), size.1.max(1));
        let mut age = 1;
        if self.fbo.is_none() || self.fbo_size != size {
            self.delete_fbo(gl);
            // SAFETY: context güncel (yüzeysiz); nesneler bu yüzeyin, `destroy`/yeniden boyutta silinir.
            let fbo = unsafe {
                let rb = gl.create_renderbuffer()?;
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
                gl.renderbuffer_storage(
                    glow::RENDERBUFFER,
                    glow::RGBA8,
                    size.0 as i32,
                    size.1 as i32,
                );
                let fb = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::RENDERBUFFER,
                    Some(rb),
                );
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                if status != glow::FRAMEBUFFER_COMPLETE {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                    gl.delete_framebuffer(fb);
                    gl.delete_renderbuffer(rb);
                    return Err(format!("FBO eksik (0x{status:x})"));
                }
                (fb, rb)
            };
            self.fbo = Some(fbo);
            self.fbo_size = size;
            age = 0;
        }
        // SAFETY: FBO bu yüzeyin ve tam.
        unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, self.fbo.map(|f| f.0)) };
        Ok(age)
    }

    /// Çizilen kareyi gönderir. `repaint` bu karenin hasarı (renderer'ın dönüşü).
    pub(crate) fn present<D>(
        &mut self,
        gl: &glow::Context,
        surface: &WlSurface,
        shm: &WlShm,
        qh: &QueueHandle<D>,
        repaint: Repaint,
    ) -> Result<Presented, String>
    where
        D: 'static,
        D: wayland_client::Dispatch<WlShmPool, NoEvents>
            + wayland_client::Dispatch<WlBuffer, BufferData>,
    {
        // Önceki kare sunulamadıysa (tamponlar meşguldü) FBO'daki içerik tümüyle gönderilir.
        let repaint = if self.stale { Repaint::Full } else { repaint };
        if repaint.is_nothing() {
            surface.commit();
            return Ok(Presented::Unchanged);
        }
        let size = self.fbo_size;
        self.buffers.retain(|b| b.size == size);
        let busy: Vec<bool> = self
            .buffers
            .iter()
            .map(|b| b.busy.load(Ordering::Acquire))
            .collect();
        let Some(i) = pick(&busy) else {
            // Bu karenin hasarı kaybolmasın: FBO'da sunulmamış içerik var; sonraki sunum (hasarsız
            // kare olsa da) tüm kareyi kopyalar.
            self.stale = true;
            surface.commit();
            return Ok(Presented::Deferred);
        };
        if i == self.buffers.len() {
            self.buffers.push(ShmBuffer::new(shm, qh, size)?);
        }
        // Tamponun yaşına göre birikmiş bölge: her ayrık parça ayrı okunup kopyalanır (uzak iki
        // parçanın arası kopyalanmaz).
        let copy = self
            .history
            .accumulate(size, age(self.buffers[i].written, self.presented), repaint)
            .region(size);
        let bgra = !copy.is_empty()
            && gl
                .supported_extensions()
                .contains("GL_EXT_read_format_bgra");
        for region in copy.rects().iter().copied() {
            let len = region.width as usize * region.height as usize * 4;
            self.scratch.resize(len, 0);
            // SAFETY: FBO bağlı ve tam; okuma bölgesi FBO içinde, hedef dilim tam w*h*4 bayt
            // (RGBA8 satırları 4'e hizalı, PACK_ALIGNMENT varsayılanı 4).
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, self.fbo.map(|f| f.0));
                gl.read_pixels(
                    region.x,
                    size.1 as i32 - region.bottom(),
                    region.width,
                    region.height,
                    if bgra { glow::BGRA } else { glow::RGBA },
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut self.scratch[..len])),
                );
            }
            copy_rows(
                &self.scratch[..len],
                region,
                self.buffers[i].pixels(),
                size.0,
                !bgra,
            );
        }
        let buf = &mut self.buffers[i];
        surface.attach(Some(&buf.buffer), 0, 0);
        if surface.version() >= 4 {
            // Compositor'a yalnızca bu karenin parçaları.
            for d in repaint.region(size).rects() {
                surface.damage_buffer(d.x, d.y, d.width, d.height);
            }
        } else {
            // Yüzey koordinatlı eski istek: ölçek/viewport hesabına girmeden tümü.
            surface.damage(0, 0, i32::MAX, i32::MAX);
        }
        surface.commit();
        buf.busy.store(true, Ordering::Release);
        self.presented += 1;
        buf.written = Some(self.presented);
        self.stale = false;
        Ok(Presented::Buffer)
    }

    fn delete_fbo(&mut self, gl: &glow::Context) {
        if let Some((fb, rb)) = self.fbo.take() {
            // SAFETY: context güncel; nesneler bu yüzeyin ve artık kullanılmıyor.
            unsafe {
                gl.bind_framebuffer(glow::FRAMEBUFFER, None);
                gl.delete_framebuffer(fb);
                gl.delete_renderbuffer(rb);
            }
        }
    }

    /// GL nesnelerini (context güncelken) ve shm tamponlarını bırakır; `wl_surface`'ten önce.
    pub(crate) fn destroy(mut self, gl: Option<&glow::Context>) {
        if let Some(gl) = gl {
            self.delete_fbo(gl);
        }
        self.buffers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_by_renderer_and_env() {
        assert!(wanted("llvmpipe (LLVM 23.1.1, 256 bits)", None));
        assert!(wanted("softpipe", None));
        assert!(!wanted(
            "AMD Radeon RX 7800 XT (radeonsi, navi32, LLVM 20.1.0, DRM 3.61)",
            None
        ));
        assert!(!wanted("llvmpipe", Some("0")));
        assert!(wanted("Mesa Intel(R) Graphics", Some("1")));
        assert!(wanted("llvmpipe", Some("auto")));
    }

    #[test]
    fn picks_free_buffer_then_grows_then_gives_up() {
        assert_eq!(pick(&[]), Some(0));
        assert_eq!(pick(&[true, false]), Some(1));
        assert_eq!(pick(&[true, true]), Some(2));
        assert_eq!(pick(&[true; MAX_BUFFERS]), None);
    }

    #[test]
    fn buffer_age_counts_presents() {
        assert_eq!(age(None, 5), 0);
        assert_eq!(age(Some(5), 5), 1);
        assert_eq!(age(Some(3), 5), 3);
    }

    #[test]
    fn rows_are_flipped_and_swizzled() {
        // 2×2 bölge, GL sırası alttan üste: alt satır [1,2], üst satır [3,4].
        let px = |n: u8| [n, n + 10, n + 20, 255];
        let src: Vec<u8> = [px(1), px(2), px(3), px(4)].concat();
        let mut dst = vec![0u8; 3 * 3 * 4];
        copy_rows(&src, PxRect::new(1, 1, 2, 2), &mut dst, 3, true);
        let at = |x: usize, y: usize| &dst[(y * 3 + x) * 4..][..4];
        // Üst satır (y=1) GL'in üst satırı: 3, 4; BGRA.
        assert_eq!(at(1, 1), &[23, 13, 3, 255]);
        assert_eq!(at(2, 1), &[24, 14, 4, 255]);
        assert_eq!(at(1, 2), &[21, 11, 1, 255]);
        assert_eq!(at(0, 0), &[0, 0, 0, 0], "bölge dışına yazılmaz");
        let mut plain = vec![0u8; 3 * 3 * 4];
        copy_rows(&src, PxRect::new(1, 1, 2, 2), &mut plain, 3, false);
        assert_eq!(&plain[(3 + 1) * 4..][..4], &px(3));
    }
}
