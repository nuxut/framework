//! Doku bulanıklığı: dual Kawase (aşağı/yukarı örnekleme zinciri; görüntü dışı örnekler shader'da
//! aynalanır, ara dokular yine kenara kıstırılır — bkz. `shaders/blur_down.frag`). Kilit ekranı ve panel
//! arkası gibi durağan arka planlar için; sonuç bir [`Texture`](crate::Texture) olarak
//! önbelleklenir, her kare yeniden hesaplanmaz.

use glow::HasContext;

use crate::error::RenderError;
use crate::texture::{BlurJob, ImagePipeline};

/// Aşağı örnekleme adımı sınırı (2^6 = 64 kat küçülme sonrası bulanıklık zaten tam).
const MAX_LEVELS: u32 = 6;

/// Yarıçaptan (fiziksel piksel) seviye sayısı ve örnek ofseti. Her seviye etkili yarıçapı
/// yaklaşık ikiye katlar; ofset ara değerleri ayarlar.
pub(crate) fn plan(radius_px: f32) -> (u32, f32) {
    let r = if radius_px.is_finite() { radius_px.max(1.0) } else { 1.0 };
    let levels = ((r / 1.5).log2().round() as u32).clamp(1, MAX_LEVELS);
    let offset = (r / (1u32 << levels) as f32 * 1.5).clamp(0.5, 3.0);
    (levels, offset)
}

#[derive(Debug)]
struct Pass {
    program: glow::Program,
    half_pixel: Option<glow::UniformLocation>,
    offset: Option<glow::UniformLocation>,
}

#[derive(Debug)]
pub(crate) struct BlurPipeline {
    down: Pass,
    up: Pass,
    vao: glow::VertexArray,
    fbo: glow::Framebuffer,
}

impl BlurPipeline {
    pub fn new(gl: &glow::Context, down: glow::Program, up: glow::Program) -> Result<Self, RenderError> {
        // SAFETY: context güncel; programlar az önce bağlandı; yalnızca nesne/uniform sorgusu.
        unsafe {
            let pass = |program: glow::Program| {
                gl.use_program(Some(program));
                gl.uniform_1_i32(gl.get_uniform_location(program, "u_tex").as_ref(), 0);
                Pass {
                    program,
                    half_pixel: gl.get_uniform_location(program, "u_half_pixel"),
                    offset: gl.get_uniform_location(program, "u_offset"),
                }
            };
            let (down, up) = (pass(down), pass(up));
            gl.use_program(None);
            Ok(Self {
                down,
                up,
                vao: gl.create_vertex_array().map_err(RenderError::Gl)?,
                fbo: gl.create_framebuffer().map_err(RenderError::Gl)?,
            })
        }
    }

    /// Bekleyen bulanıklık işlerini çalıştırır: kaynak doku yüklü ve bekleyen güncellemesi
    /// yoksa sonucu hedef tutamağa bağlar ve hazır işaretler; değilse iş sonraki kareye kalır
    /// (`true` döner).
    /// Önceki framebuffer ve viewport geri yüklenir.
    pub fn run(&self, gl: &glow::Context, images: &mut ImagePipeline, jobs: &mut Vec<BlurJob>) -> bool {
        if jobs.is_empty() {
            return false;
        }
        let mut pending = Vec::new();
        // SAFETY: context güncel; tüm nesneler bu renderer'a ait; durum sonunda geri yüklenir.
        unsafe {
            let prev_fbo = gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
            let mut prev_viewport = [0i32; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut prev_viewport);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_vertex_array(Some(self.vao));
            for job in jobs.drain(..) {
                if job.dst.upgrade().is_none() {
                    continue; // sonuç artık istenmiyor
                }
                // Kaynağın bekleyen (bütçeye takılmış) güncellemesi varsa GPU'daki içerik eskidir:
                // eskiyi bulanıklaştırmak yerine güncelleme yüklenene dek bekle (sonuç hazır değil).
                let src = images.gl_texture(job.src.id()).filter(|_| !images.has_pending_upload(job.src.id()));
                let Some((src, w, h)) = src else {
                    pending.push(job); // kaynak henüz yüklenmedi ya da güncellemesi bekliyor
                    continue;
                };
                match self.blur(gl, src, w, h, job.radius_px) {
                    Some(out) => images.adopt(gl, &job, out, w, h),
                    None => tracing::warn!("bulanıklık uygulanamadı ({w}x{h})"),
                }
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, prev_fbo);
            gl.viewport(prev_viewport[0], prev_viewport[1], prev_viewport[2], prev_viewport[3]);
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        let deferred = !pending.is_empty();
        *jobs = pending;
        deferred
    }

    /// Aşağı zincir (yarım boyutlara), sonra yukarı zincir; son adım tam boyutlu yeni dokuya.
    ///
    /// # Safety
    /// Context güncel; `src` canlı bir doku.
    unsafe fn blur(&self, gl: &glow::Context, src: glow::Texture, w: u32, h: u32, radius_px: f32) -> Option<glow::Texture> {
        let (levels, offset) = plan(radius_px);
        // Seviye boyutları: 0 = tam, i = /2^i (en az 1 px).
        let sizes: Vec<(u32, u32)> = (0..=levels).map(|i| ((w >> i).max(1), (h >> i).max(1))).collect();
        // SAFETY: çağıranın sözleşmesi; ara dokular burada yaratılıp burada silinir.
        unsafe {
            let mut chain: Vec<glow::Texture> = Vec::with_capacity(levels as usize + 1);
            for &(lw, lh) in &sizes {
                chain.push(new_target(gl, lw, lh)?);
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.fbo));
            let draw = |pass: &Pass, from: glow::Texture, from_size: (u32, u32), to: glow::Texture, to_size: (u32, u32)| {
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(to), 0);
                gl.viewport(0, 0, to_size.0 as i32, to_size.1 as i32);
                gl.use_program(Some(pass.program));
                gl.uniform_2_f32(pass.half_pixel.as_ref(), 0.5 / from_size.0 as f32, 0.5 / from_size.1 as f32);
                gl.uniform_1_f32(pass.offset.as_ref(), offset);
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(from));
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            };
            // Aşağı: kaynak → seviye 1 → … → seviye n.
            draw(&self.down, src, sizes[0], chain[1], sizes[1]);
            for i in 1..levels as usize {
                draw(&self.down, chain[i], sizes[i], chain[i + 1], sizes[i + 1]);
            }
            // Yukarı: seviye n → … → seviye 0 (sonuç).
            for i in (0..levels as usize).rev() {
                draw(&self.up, chain[i + 1], sizes[i + 1], chain[i], sizes[i]);
            }
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, None, 0);
            for t in chain.drain(1..) {
                gl.delete_texture(t);
            }
            Some(chain[0])
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        // SAFETY: nesneler bu boru hattına aittir.
        unsafe {
            gl.delete_program(self.down.program);
            gl.delete_program(self.up.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_framebuffer(self.fbo);
        }
    }
}

/// Boş RGBA8 hedef doku (doğrusal süzme, kenara kıstırma).
///
/// # Safety
/// Context güncel.
unsafe fn new_target(gl: &glow::Context, w: u32, h: u32) -> Option<glow::Texture> {
    // SAFETY: çağıranın sözleşmesi; veri yok (NULL ile ayrılır).
    unsafe {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            w as i32,
            h as i32,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        for (p, v) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
        }
        Some(tex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_grows_with_radius() {
        let (l1, _) = plan(4.0);
        let (l2, _) = plan(32.0);
        let (l3, o3) = plan(1e9);
        assert!(l1 < l2 && l2 <= l3);
        assert_eq!(l3, MAX_LEVELS);
        assert!((0.5..=3.0).contains(&o3));
        assert_eq!(plan(f32::NAN).0, 1);
    }
}
