//! Çizgi (stroke): yuvarlak uçlu doğru parçaları (kapsül SDF), çember yayları (yay SDF,
//! tek dörtgen) ve eğri altı alan sütunları; 1 px AA. Eğri grafikleri, dönen göstergeler,
//! dairesel ilerleme, küçük geçmiş grafikleri (sparkline) için.

use glow::HasContext;

use crate::error::RenderError;

/// Örnek türü (`param[1]`); shader aynı değerleri okur.
pub(crate) const KIND_SEGMENT: f32 = 0.0;
pub(crate) const KIND_ARC: f32 = 1.0;
pub(crate) const KIND_AREA: f32 = 2.0;

/// Çizgi örneği (fiziksel piksel). Düzen `stroke.vert` ile aynı.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StrokeInstance {
    /// Doğru: p0, p1. Yay: merkez, yarıçap, -. Alan: üst kenarın sol (x0, y0) ve sağ (x1, y1) ucu.
    pub a: [f32; 4],
    /// Yay: başlangıç, süpürme (radyan). Alan: taban y, tabandaki opaklık çarpanı, geçişin
    /// başladığı y (tüm alanın tepesi; sütunlar arası tek geçiş), -.
    pub b: [f32; 4],
    /// Premultiplied.
    pub color: [f32; 4],
    /// Yarı kalınlık (alanda kullanılmaz), tür ([`KIND_SEGMENT`], [`KIND_ARC`], [`KIND_AREA`]).
    pub param: [f32; 2],
}

impl StrokeInstance {
    /// Çizimin kapladığı dikdörtgen (fiziksel; AA payı dahil): x, y, w, h.
    pub(crate) fn bounds(&self) -> [f32; 4] {
        let pad = self.param[0] + 1.0;
        let [ax, ay, az, aw] = self.a;
        let (x0, y0, x1, y1) = if self.param[1] == KIND_ARC {
            let r = az + pad;
            (ax - r, ay - r, ax + r, ay + r)
        } else if self.param[1] == KIND_AREA {
            let base = self.b[0];
            (ax, ay.min(aw).min(base) - 1.0, az, ay.max(aw).max(base) + 1.0)
        } else {
            (ax.min(az) - pad, ay.min(aw) - pad, ax.max(az) + pad, ay.max(aw) + pad)
        };
        [x0, y0, x1 - x0, y1 - y0]
    }
}

#[derive(Debug)]
pub(crate) struct StrokePipeline {
    pub program: glow::Program,
    pub viewport: Option<glow::UniformLocation>,
    vao: glow::VertexArray,
    vbo: glow::Buffer,
}

impl StrokePipeline {
    pub fn new(gl: &glow::Context, program: glow::Program, viewport: Option<glow::UniformLocation>) -> Result<Self, RenderError> {
        // SAFETY: context güncel; yalnızca nesne oluşturma.
        let (vao, vbo) = unsafe {
            (gl.create_vertex_array().map_err(RenderError::Gl)?, gl.create_buffer().map_err(RenderError::Gl)?)
        };
        Ok(Self { program, viewport, vao, vbo })
    }

    /// # Safety
    /// Context güncel olmalı.
    pub unsafe fn upload(&self, gl: &glow::Context, instances: &[StrokeInstance]) {
        // SAFETY: StrokeInstance yalnızca f32 alanlı #[repr(C)]; dolgu yok (14 × f32).
        unsafe {
            let bytes = std::slice::from_raw_parts(instances.as_ptr().cast::<u8>(), size_of_val(instances));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes, glow::STREAM_DRAW);
        }
    }

    /// # Safety
    /// Context güncel, `upload` bu karede çağrılmış ve `range` yüklenenler içinde olmalı.
    pub unsafe fn draw(&self, gl: &glow::Context, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        let stride = size_of::<StrokeInstance>() as i32;
        let base = (range.start * size_of::<StrokeInstance>()) as i32;
        // SAFETY: ofsetler StrokeInstance düzenine karşılık gelir (vec4, vec4, vec4, vec2).
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            for (i, (size, offset)) in [(4, 0), (4, 16), (4, 32), (2, 48)].into_iter().enumerate() {
                let i = i as u32;
                gl.enable_vertex_attrib_array(i);
                gl.vertex_attrib_pointer_f32(i, size, glow::FLOAT, false, stride, base + offset);
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
