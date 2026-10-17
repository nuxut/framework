//! Köşe renkli (bilineer) yuvarlatılmış dikdörtgen: renk seçici alanları, ton şeridi,
//! yumuşak geçişli arka planlar.

use glow::HasContext;

use crate::error::RenderError;

/// Gradyan örneği (fiziksel piksel). Düzen `gradient.vert` ile aynı.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GradientInstance {
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    /// Sol-üst, sağ-üst, sağ-alt, sol-alt; premultiplied.
    pub colors: [[f32; 4]; 4],
}

#[derive(Debug)]
pub(crate) struct GradientPipeline {
    pub program: glow::Program,
    pub viewport: Option<glow::UniformLocation>,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
}

impl GradientPipeline {
    pub fn new(gl: &glow::Context, program: glow::Program, viewport: Option<glow::UniformLocation>) -> Result<Self, RenderError> {
        // SAFETY: context güncel; yalnızca nesne oluşturma.
        let (vao, vbo) = unsafe {
            (gl.create_vertex_array().map_err(RenderError::Gl)?, gl.create_buffer().map_err(RenderError::Gl)?)
        };
        Ok(Self { program, viewport, vao, vbo })
    }

    /// Örnekleri yükler (karede bir kez).
    ///
    /// # Safety
    /// Context güncel olmalı.
    pub unsafe fn upload(&self, gl: &glow::Context, instances: &[GradientInstance]) {
        // SAFETY: GradientInstance yalnızca f32 alanlı #[repr(C)]; bayt görünümü tam boyutlu.
        unsafe {
            let bytes = std::slice::from_raw_parts(instances.as_ptr().cast::<u8>(), size_of_val(instances));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
        }
    }

    /// # Safety
    /// Context güncel, `upload` bu karede çağrılmış ve `range` yüklenen örnekler içinde olmalı.
    pub unsafe fn draw(&self, gl: &glow::Context, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        let stride = size_of::<GradientInstance>() as i32;
        let base = (range.start * size_of::<GradientInstance>()) as i32;
        // SAFETY: öznitelik ofsetleri GradientInstance düzenine karşılık gelir (6 × vec4).
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            for i in 0..6u32 {
                gl.enable_vertex_attrib_array(i);
                gl.vertex_attrib_pointer_f32(i, 4, glow::FLOAT, false, stride, base + i as i32 * 16);
                gl.vertex_attrib_divisor(i, 1);
            }
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, range.len() as i32);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        // SAFETY: nesneler bu boru hattına aittir.
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            gl.delete_buffer(self.vbo);
        }
    }
}
