//! `Renderer`: tek GL context'i, tüm yüzeylerin paylaştığı shader'lar ve glyph atlas'ı.

use glow::HasContext;

use crate::atlas::{Entry, Full, GlyphAtlas, Kind};
use crate::damage::{DamageHistory, PxRect, Region, Repaint};
use crate::gradient::GradientPipeline;
use crate::stroke::StrokePipeline;
use crate::error::RenderError;
use crate::painter::{Op, Painter, QuadInstance, Recording};
use crate::text::TextSystem;
use crate::blur::BlurPipeline;
use crate::texture::ImagePipeline;

const HEADER: &str = "#version 300 es\nprecision highp float;\nprecision highp int;\n";
const COMMON: &str = include_str!("shaders/common.glsl");

/// Glyph örneği (fiziksel piksel). Düzen `glyph.vert` ile aynı.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct GlyphInstance {
    dest: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    kind: f32,
}

#[derive(Debug)]
struct Program {
    program: glow::Program,
    viewport: Option<glow::UniformLocation>,
}

#[derive(Debug)]
struct BlobUniforms {
    count: Option<glow::UniformLocation>,
    fill: Option<glow::UniformLocation>,
    shadow: Option<glow::UniformLocation>,
    shadow_geom: Option<glow::UniformLocation>,
    /// `blob_solid.frag`'ın `u_fill`'i.
    solid_fill: Option<glow::UniformLocation>,
}

/// Blob ilkellerinin doku birimi (glif atlası 0 ve 1'de).
const BLOB_TEXTURE_UNIT: u32 = 2;
/// Doku satırında ilkel sayısı (`blob.frag` PRIM_ROW ile aynı; ilkel başına iki doku).
const BLOB_PRIM_ROW: usize = 256;
/// Önbellekte tutulan son blob geometrileri (yüzey başına bir alan; birkaç yüzey).
const BLOB_CACHE: usize = 4;

/// Blob geometrisi yalnızca şekil değişince hesaplanır; aynı sahne yeniden çizilirken önbellekten
/// gelir, dokuya da yalnızca değişince yüklenir.
#[derive(Default)]
struct BlobCache {
    entries: Vec<(crate::blob::morph::Scene, std::rc::Rc<Vec<crate::blob::GpuPrim>>)>,
    uploaded: Option<std::rc::Rc<Vec<crate::blob::GpuPrim>>>,
}

/// Çizime hazır blob (kare başına bir kez hazırlanır, hasar parçası başına çizilir).
struct BlobDraw {
    prims: std::rc::Rc<Vec<crate::blob::GpuPrim>>,
    fill: [f32; 4],
    shadow: [f32; 4],
    shadow_geom: [f32; 3],
    /// SDF ve düz dolgu üçgenleri (fiziksel px köşeler).
    sdf: Vec<[f32; 2]>,
    solid: Vec<[f32; 2]>,
}

/// Bir karenin hedefi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTarget {
    pub size_px: (u32, u32),
    pub scale: f32,
    /// `EGL_BUFFER_AGE_EXT`; bilinmiyorsa 0 (tüm kare çizilir).
    pub buffer_age: u32,
}

pub struct Renderer {
    gl: glow::Context,
    quad: Program,
    glyph: Program,
    blob: Program,
    /// Blob'un serbest alandan uzak dolu parçaları (düz dolgu; `blob::split_solid`).
    blob_solid: Program,
    /// Testlerde kapatılır: düz dolgu bölmesinin piksel-piksel aynı sonucu verdiği doğrulanır.
    pub(crate) blob_solid_split: bool,
    blob_uniforms: BlobUniforms,
    quad_vao: glow::VertexArray,
    quad_vbo: glow::Buffer,
    glyph_vao: glow::VertexArray,
    glyph_vbo: glow::Buffer,
    blob_vao: glow::VertexArray,
    blob_vbo: glow::Buffer,
    /// Blob ilkelleri (RGBA32F, `blob.frag`).
    blob_tex: glow::Texture,
    blob_cache: std::cell::RefCell<BlobCache>,
    atlas: GlyphAtlas,
    gradient: GradientPipeline,
    stroke: StrokePipeline,
    image: ImagePipeline,
    blur: BlurPipeline,
    /// Kaynağı henüz hazır olmayan bulanıklık işleri (sonraki karelerde denenir).
    pending_blurs: Vec<crate::texture::BlurJob>,
    rec: Recording,
    /// `gl::GlGraveyard` ile gönderilen değerler; karenin başında ve `Drop`'ta düşürülür.
    graveyard: std::sync::mpsc::Receiver<crate::gl::GraveItem>,
    glyph_instances: Vec<GlyphInstance>,
    /// Her `Op::Glyphs` için `glyph_instances` aralığı (kayıt sırasıyla).
    glyph_ranges: Vec<std::ops::Range<usize>>,
    light_gamma_loc: Option<glow::UniformLocation>,
    light_gamma: f32,
}

/// Açık metin kapsama üssünün varsayılanı (bkz. [`Renderer::set_text_gamma`]).
pub const DEFAULT_TEXT_GAMMA: f32 = 0.72;

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("atlas_mask_px", &self.atlas.mask.size())
            .field("atlas_color_px", &self.atlas.page_size(Kind::Color))
            .finish_non_exhaustive()
    }
}

impl Renderer {
    /// GL context'i güncel (current) olmalı ve en az OpenGL ES 3.0 sağlamalı.
    pub fn new(gl: glow::Context) -> Result<Self, RenderError> {
        let version = gl.version();
        if version.major < 3 {
            return Err(RenderError::Unsupported(format!("{}.{} {}", version.major, version.minor, version.vendor_info)));
        }
        let quad = compile(&gl, "quad", include_str!("shaders/quad.vert"), include_str!("shaders/quad.frag"))?;
        let glyph = compile(&gl, "glyph", include_str!("shaders/glyph.vert"), include_str!("shaders/glyph.frag"))?;
        let blob = compile(&gl, "blob", include_str!("shaders/blob.vert"), include_str!("shaders/blob.frag"))?;
        let blob_solid = compile(&gl, "blob_solid", include_str!("shaders/blob.vert"), include_str!("shaders/blob_solid.frag"))?;
        // SAFETY: context güncel; program az önce başarıyla bağlandı; isimler sabit.
        let blob_uniforms = unsafe {
            let u = |name: &str| gl.get_uniform_location(blob.program, name);
            BlobUniforms {
                count: u("u_count"),
                fill: u("u_fill"),
                shadow: u("u_shadow"),
                shadow_geom: u("u_shadow_geom"),
                solid_fill: gl.get_uniform_location(blob_solid.program, "u_fill"),
            }
        };
        // SAFETY: context güncel; doku az önce oluşturulur, yalnızca parametreler ayarlanır
        // (RGBA32F filtrelenmez: NEAREST, texelFetch).
        let blob_tex = unsafe {
            let t = gl.create_texture().map_err(RenderError::Gl)?;
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::NEAREST as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::NEAREST as i32);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(Some(blob.program));
            gl.uniform_1_i32(gl.get_uniform_location(blob.program, "u_prims").as_ref(), BLOB_TEXTURE_UNIT as i32);
            gl.use_program(None);
            t
        };
        // SAFETY: context güncel; doku birimleri sabit ve GLES 3.0 en az 16 birim sağlar.
        unsafe {
            gl.use_program(Some(glyph.program));
            gl.uniform_1_i32(gl.get_uniform_location(glyph.program, "u_mask").as_ref(), 0);
            gl.uniform_1_i32(gl.get_uniform_location(glyph.program, "u_color").as_ref(), 1);
            gl.use_program(None);
        }
        // SAFETY: context güncel; yalnızca nesne oluşturma.
        let (quad_vao, quad_vbo, glyph_vao, glyph_vbo, blob_vao, blob_vbo) = unsafe {
            let mk = || -> Result<(glow::VertexArray, glow::Buffer), RenderError> {
                Ok((gl.create_vertex_array().map_err(RenderError::Gl)?, gl.create_buffer().map_err(RenderError::Gl)?))
            };
            let (a, b) = mk()?;
            let (c, d) = mk()?;
            let (e, f) = mk()?;
            (a, b, c, d, e, f)
        };
        // SAFETY: VAO/VBO az önce oluşturuldu; konum 0 ve vec2 düzeni blob.vert ile aynı.
        unsafe {
            gl.bind_vertex_array(Some(blob_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(blob_vbo));
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, 8, 0);
            gl.bind_vertex_array(None);
        }
        // SAFETY: context güncel; program bağlı.
        let light_gamma_loc = unsafe { gl.get_uniform_location(glyph.program, "u_light_gamma") };
        let gradient_program =
            compile(&gl, "gradient", include_str!("shaders/gradient.vert"), include_str!("shaders/gradient.frag"))?;
        let gradient = GradientPipeline::new(&gl, gradient_program.program, gradient_program.viewport)?;
        let stroke_program = compile(&gl, "stroke", include_str!("shaders/stroke.vert"), include_str!("shaders/stroke.frag"))?;
        let stroke = StrokePipeline::new(&gl, stroke_program.program, stroke_program.viewport)?;
        let image_program = compile(&gl, "image", include_str!("shaders/image.vert"), include_str!("shaders/image.frag"))?;
        let image = ImagePipeline::new(&gl, image_program.program, image_program.viewport)?;
        let blur_down = compile(&gl, "blur_down", include_str!("shaders/blur.vert"), include_str!("shaders/blur_down.frag"))?;
        let blur_up = compile(&gl, "blur_up", include_str!("shaders/blur.vert"), include_str!("shaders/blur_up.frag"))?;
        let blur = BlurPipeline::new(&gl, blur_down.program, blur_up.program)?;
        let (grave_tx, graveyard) = std::sync::mpsc::channel();
        let rec = Recording { release: Some(image.release_sender()), graveyard: Some(grave_tx), ..Recording::default() };
        let atlas = GlyphAtlas::new(&gl)?;
        Ok(Self {
            gl,
            quad,
            glyph,
            blob,
            blob_solid,
            blob_solid_split: true,
            blob_uniforms,
            quad_vao,
            quad_vbo,
            glyph_vao,
            glyph_vbo,
            blob_vao,
            blob_vbo,
            blob_tex,
            blob_cache: std::cell::RefCell::new(BlobCache::default()),
            atlas,
            gradient,
            stroke,
            image,
            blur,
            pending_blurs: Vec::new(),
            rec,
            graveyard,
            glyph_instances: Vec::new(),
            glyph_ranges: Vec::new(),
            light_gamma_loc,
            light_gamma: DEFAULT_TEXT_GAMMA,
        })
    }

    /// Açık renkli metnin kapsama üssü: 1.0 = düzeltme yok, küçüldükçe kalınlaşır.
    /// [0.4, 1.0] aralığına sınırlanır.
    pub fn set_text_gamma(&mut self, gamma: f32) {
        self.light_gamma = if gamma.is_finite() { gamma.clamp(0.4, 1.0) } else { DEFAULT_TEXT_GAMMA };
    }

    pub fn gl(&self) -> &glow::Context {
        &self.gl
    }

    /// Canlı görsel dokusu sayısı ve yaklaşık GPU baytı (mipmap dahil).
    pub fn texture_stats(&self) -> (usize, usize) {
        self.image.stats()
    }

    /// Glyph atlas'ı sayfa kenarları (px): `(maske, renk)`; renk sayfası ilk renkli glyph'e
    /// (emoji) dek ayrılmaz (`None`). Sayfalar bağımsız büyür. Ölçüm/tanı için.
    pub fn glyph_atlas_sizes(&self) -> (u32, Option<u32>) {
        (self.atlas.page_size(Kind::Mask), self.atlas.color.as_ref().map(|p| p.size()))
    }

    /// Kare bütçesi (`UPLOAD_BUDGET_PER_FRAME`) yüzünden bekleyen doku yüklemesi var mı. Varsa
    /// runtime bir kare daha ister; yükleme sonraki `render_frame`'lerde sürer.
    pub fn has_pending_uploads(&self) -> bool {
        self.image.has_deferred() || !self.pending_blurs.is_empty()
    }

    /// Hedef framebuffer'ı (bağlı olan, genelde 0) temizler (şeffaf) ve `f`'in kaydettiği
    /// çizimleri gönderir. Hasar takibi yok: her zaman tüm kare. Buffer swap çağıranın işidir.
    pub fn render(&mut self, text: &mut TextSystem, size_px: (u32, u32), scale: f32, f: impl FnOnce(&mut Painter)) {
        let mut history = DamageHistory::new();
        self.render_frame(text, &mut history, FrameTarget { size_px, scale, buffer_age: 0 }, f);
    }

    /// Hasar takipli çizim. `history` yüzey başına tutulur; `target.buffer_age` EGL'in
    /// `EGL_BUFFER_AGE_EXT` değeridir (bilinmiyorsa 0). Yalnızca birikmiş hasar bölgesi
    /// temizlenip yeniden çizilir. Dönen değer **bu karenin** hasarıdır:
    /// parçaları `eglSwapBuffersWithDamage`'a (`PxRect::to_egl`) verilir; `Partial` boşsa hiçbir şey
    /// değişmemiştir.
    pub fn render_frame(
        &mut self,
        text: &mut TextSystem,
        history: &mut DamageHistory,
        target: FrameTarget,
        f: impl FnOnce(&mut Painter),
    ) -> Repaint {
        let FrameTarget { size_px, buffer_age, .. } = target;
        let scale = if target.scale.is_finite() && target.scale > 0.0 { target.scale } else { 1.0 };
        // Önceki karelerde düşen tutamakların dokuları (bu karede kimse onlara başvuramaz).
        self.image.collect_garbage(&self.gl);
        let mut rec = std::mem::take(&mut self.rec);
        rec.clear();
        // Önceki karenin geçişleri (son başvurular) bırakıldı; GL çöplüğü context güncelken boşalır.
        self.drain_graveyard();
        {
            let mut painter = Painter::new(&mut rec, text, size_px, scale);
            f(&mut painter);
        }
        // Kaplama katmanı (ipucu, açılır menü) her şeyin üstüne, kırpmasız.
        rec.merge_overlay();
        // Dokular erken dönüşten önce yüklenir: hiçbir şey çizilmese de sonraki kareler hazır bulur.
        self.image.upload(&self.gl, &mut rec.uploads);
        // Bulanıklıklar yüklemelerden sonra: kaynak bu karede yüklendiyse hemen kullanılır.
        self.pending_blurs.append(&mut rec.blurs);
        self.blur.run(&self.gl, &mut self.image, &mut self.pending_blurs);
        self.image.allocate_targets(&self.gl, &mut rec.targets);
        self.image.run_passes(&self.gl, &rec.passes);
        // Yeni boyutta tampon baştandır: uygulama küçük bir bölge bildirse de yüzeyin tamamı
        // yeni (compositor'a da tümü bildirilir).
        let current = if history.is_reset_for(size_px) { Repaint::Full } else { rec.damage.unwrap_or(Repaint::Full) };
        let region = match history.accumulate(size_px, buffer_age, current) {
            Repaint::Full => None,
            Repaint::Partial(r) => Some(r),
        };
        if region.as_ref().is_some_and(Region::is_empty) {
            // Tampon zaten güncel; çizecek bir şey yok.
            self.rec = rec;
            return current;
        }
        self.resolve_glyphs(text, &rec);
        self.flush(&rec, size_px, scale, region);
        self.rec = rec;
        current
    }

    /// Kayıttaki glyph'leri atlas'tan çözer. Atlas dolarsa büyütülür ve çözüm baştan yapılır
    /// (önceki uv'ler geçersiz olur).
    fn resolve_glyphs(&mut self, text: &mut TextSystem, rec: &Recording) {
        for _attempt in 0..4 {
            self.glyph_instances.clear();
            self.glyph_ranges.clear();
            let mut full = None;
            'ops: for op in &rec.ops {
                let Op::Glyphs(range) = op else { continue };
                let start = self.glyph_instances.len();
                for g in &rec.glyphs[range.clone()] {
                    match self.atlas.get_or_insert(&self.gl, text, g.key) {
                        Ok(Some(e)) => {
                            let inst = glyph_instance(&e, g.x, g.y, g.color, self.page_size(e.kind));
                            self.glyph_instances.push(inst);
                        }
                        Ok(None) => {}
                        Err(Full(kind)) => {
                            full = Some(kind);
                            break 'ops;
                        }
                    }
                }
                self.glyph_ranges.push(start..self.glyph_instances.len());
            }
            let Some(kind) = full else {
                return;
            };
            match self.atlas.grow(&self.gl, kind) {
                Ok(true) => {}
                Ok(false) => {
                    tracing::warn!("glyph atlas'ı en büyük boyutta ve dolu; bu karede metin eksik çizilebilir");
                }
                Err(e) => {
                    tracing::error!("glyph atlas'ı büyütülemedi: {e}");
                    break;
                }
            }
        }
        // Başarısız denemelerden kalan yarım aralıkları tamamla ki op sırası bozulmasın.
        let ops = rec.ops.iter().filter(|op| matches!(op, Op::Glyphs(_))).count();
        while self.glyph_ranges.len() < ops {
            let end = self.glyph_instances.len();
            self.glyph_ranges.push(end..end);
        }
    }

    fn page_size(&self, kind: Kind) -> f32 {
        match kind {
            Kind::Mask => self.atlas.page_size(Kind::Mask) as f32,
            Kind::Color => self.atlas.page_size(Kind::Color) as f32,
        }
    }

    /// Kaydı çizer. `region` `None` = tüm kare; yoksa her ayrık parça için komut listesi bir
    /// kez daha, o parçaya scissor'la gönderilir (yüklemeler ve uniform'lar bir kez). Parçalar
    /// örtüşmediğinden hiçbir piksel iki kez harmanlanmaz.
    fn flush(&mut self, rec: &Recording, size_px: (u32, u32), scale: f32, region: Option<Region>) {
        let gl = &self.gl;
        let (w, h) = (size_px.0 as f32, size_px.1 as f32);
        // Kırpma = bu geçişin hasar parçası ∩ uygulamanın clip'i.
        let scissor = |gl: &glow::Context, part: Option<PxRect>, clip: Option<PxRect>| {
            let r = match (part, clip) {
                (None, None) => None,
                (Some(a), Some(b)) => Some(a.intersect(&b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
            };
            // SAFETY: context güncel; scissor değerleri tamsayı, boyutlar negatif değil.
            unsafe {
                match r {
                    None => gl.disable(glow::SCISSOR_TEST),
                    Some(r) => {
                        gl.enable(glow::SCISSOR_TEST);
                        gl.scissor(r.x, size_px.1 as i32 - r.bottom(), r.width.max(0), r.height.max(0));
                    }
                }
            }
        };
        // Geçişler: tüm kare için tek, kısmi hasarda parça başına bir.
        let parts: &[PxRect] = region.as_ref().map_or(&[], Region::rects);
        let passes = if region.is_none() { 1 } else { parts.len() };
        let part = |i: usize| parts.get(i).copied();
        // SAFETY: context güncel; tüm nesneler bu renderer'a ait; tampon yüklemeleri
        // `bytemuck`suz ama #[repr(C)] f32 dizilerinin bayt görünümüdür (as_bytes); çizim
        // aralıkları yüklenen örnek sayısı içinde kalır.
        unsafe {
            gl.viewport(0, 0, size_px.0 as i32, size_px.1 as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            for i in 0..passes {
                scissor(gl, part(i), None);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            if rec.ops.is_empty() {
                gl.disable(glow::SCISSOR_TEST);
                return;
            }
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);

            if !rec.quads.is_empty() {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.quad_vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&rec.quads), glow::STREAM_DRAW);
            }
            if !rec.gradients.is_empty() {
                self.gradient.upload(gl, &rec.gradients);
            }
            if !rec.strokes.is_empty() {
                self.stroke.upload(gl, &rec.strokes);
            }
            if !self.glyph_instances.is_empty() {
                gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.glyph_vbo));
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&self.glyph_instances), glow::STREAM_DRAW);
            }
            gl.use_program(Some(self.gradient.program));
            gl.uniform_2_f32(self.gradient.viewport.as_ref(), w, h);
            gl.use_program(Some(self.stroke.program));
            gl.uniform_2_f32(self.stroke.viewport.as_ref(), w, h);
            let (image_program, image_viewport) = self.image.program();
            gl.use_program(Some(image_program));
            gl.uniform_2_f32(image_viewport, w, h);
            for p in [&self.quad, &self.glyph, &self.blob, &self.blob_solid] {
                gl.use_program(Some(p.program));
                gl.uniform_2_f32(p.viewport.as_ref(), w, h);
            }

            let blobs: Vec<Option<BlobDraw>> =
                rec.blobs.iter().map(|b| self.prepare_blob(b, size_px, scale)).collect();
            // Kısmi karede komut başına kapsam: parçayla kesişmeyen çizim o geçişte hiç
            // gönderilmez (parça başına geçişin maliyeti yalnızca o parçaya düşen çizimler).
            let reach: Vec<Option<PxRect>> = if region.is_some() { self.op_reach(rec, &blobs) } else { Vec::new() };
            for i in 0..passes {
                let part = part(i);
                scissor(gl, part, None);
                let mut glyph_op = 0;
                for (k, op) in rec.ops.iter().enumerate() {
                    if let (Some(p), Some(Some(r))) = (part, reach.get(k))
                        && p.intersect(r).is_empty()
                    {
                        if matches!(op, Op::Glyphs(_)) {
                            glyph_op += 1;
                        }
                        continue;
                    }
                    match op {
                        Op::Quads(range) => self.draw_quads(range.clone()),
                        Op::Gradients(range) => self.gradient.draw(gl, range.clone()),
                        Op::Strokes(range) => self.stroke.draw(gl, range.clone()),
                        Op::Glyphs(_) => {
                            let range = self.glyph_ranges.get(glyph_op).cloned().unwrap_or(0..0);
                            glyph_op += 1;
                            self.draw_glyphs(range);
                        }
                        Op::Blob(i) => {
                            if let Some(Some(d)) = blobs.get(*i) {
                                self.draw_blob(d);
                            }
                        }
                        Op::Image(i) => self.image.draw(gl, &rec.images[*i]),
                        Op::Clip(None) => scissor(gl, part, None),
                        Op::Clip(Some([x0, y0, x1, y1])) => {
                            scissor(gl, part, Some(PxRect::new(*x0, *y0, x1 - x0, y1 - y0)))
                        }
                    }
                }
            }
            gl.disable(glow::SCISSOR_TEST);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    /// # Safety
    /// `flush` içinden, quad VBO'su yüklenmişken çağrılır.
    unsafe fn draw_quads(&self, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        let gl = &self.gl;
        let stride = size_of::<QuadInstance>() as i32;
        let base = (range.start * size_of::<QuadInstance>()) as i32;
        // SAFETY: öznitelik ofsetleri QuadInstance'ın #[repr(C)] düzenine karşılık gelir ve
        // `range` yüklenen örnek sayısı içindedir.
        unsafe {
            gl.use_program(Some(self.quad.program));
            gl.bind_vertex_array(Some(self.quad_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.quad_vbo));
            for i in 0..5u32 {
                gl.enable_vertex_attrib_array(i);
                gl.vertex_attrib_pointer_f32(i, 4, glow::FLOAT, false, stride, base + i as i32 * 16);
                gl.vertex_attrib_divisor(i, 1);
            }
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, range.len() as i32);
        }
    }

    /// # Safety
    /// `flush` içinden, glyph VBO'su yüklenmişken çağrılır.
    unsafe fn draw_glyphs(&self, range: std::ops::Range<usize>) {
        if range.is_empty() {
            return;
        }
        let gl = &self.gl;
        let stride = size_of::<GlyphInstance>() as i32;
        let base = (range.start * size_of::<GlyphInstance>()) as i32;
        // SAFETY: ofsetler GlyphInstance düzeniyle aynı; dokular atlas'a ait ve canlı.
        unsafe {
            gl.use_program(Some(self.glyph.program));
            gl.uniform_1_f32(self.light_gamma_loc.as_ref(), self.light_gamma);
            gl.bind_vertex_array(Some(self.glyph_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.glyph_vbo));
            for i in 0..3u32 {
                gl.enable_vertex_attrib_array(i);
                gl.vertex_attrib_pointer_f32(i, 4, glow::FLOAT, false, stride, base + i as i32 * 16);
                gl.vertex_attrib_divisor(i, 1);
            }
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_f32(3, 1, glow::FLOAT, false, stride, base + 48);
            gl.vertex_attrib_divisor(3, 1);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas.mask.texture));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, self.atlas.color_texture());
            gl.active_texture(glow::TEXTURE0);
            gl.draw_arrays_instanced(glow::TRIANGLE_STRIP, 0, 4, range.len() as i32);
        }
    }

    /// Her komutun çizebileceği en geniş bölge (fiziksel, korumacı: köşe genişletmeleri
    /// gölgelendiricilerdeki gibi, üstüne 2 px). `None` = bilinmiyor/sonsuz değil → her parçada
    /// çizilir; boş dikdörtgen = hiçbir şey çizmez. `Clip` komutları hep işlenir (`None`).
    fn op_reach(&self, rec: &Recording, blobs: &[Option<BlobDraw>]) -> Vec<Option<PxRect>> {
        // x, y, w, h dizilerinin birleşimi; sonlu olmayan değer → None.
        fn cover(rects: impl Iterator<Item = [f32; 4]>, pad: f32) -> Option<PxRect> {
            let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
            for [x, y, w, h] in rects {
                if !(x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()) {
                    return None;
                }
                x0 = x0.min(x - pad);
                y0 = y0.min(y - pad);
                x1 = x1.max(x + w.max(0.0) + pad);
                y1 = y1.max(y + h.max(0.0) + pad);
            }
            if x0 > x1 {
                return Some(PxRect::default());
            }
            let m = 2.0;
            let clamp = |v: f32| v.clamp(i32::MIN as f32 / 2.0, i32::MAX as f32 / 2.0) as i32;
            let (l, t) = (clamp((x0 - m).floor()), clamp((y0 - m).floor()));
            Some(PxRect::new(l, t, clamp((x1 + m).ceil()) - l, clamp((y1 + m).ceil()) - t))
        }
        let mut glyph_op = 0;
        rec.ops
            .iter()
            .map(|op| match op {
                // Quad payı (params[2]; gölge, AA) gölgelendiricideki kadar genişletilir.
                Op::Quads(r) => rec.quads.get(r.clone()).and_then(|q| {
                    cover(
                        q.iter().map(|q| {
                            let p = q.params[2].max(0.0);
                            [q.rect[0] - p, q.rect[1] - p, q.rect[2] + 2.0 * p, q.rect[3] + 2.0 * p]
                        }),
                        0.0,
                    )
                }),
                Op::Gradients(r) => rec.gradients.get(r.clone()).and_then(|g| cover(g.iter().map(|g| g.rect), 1.0)),
                Op::Strokes(r) => rec.strokes.get(r.clone()).and_then(|s| cover(s.iter().map(|s| s.bounds()), 0.0)),
                Op::Glyphs(_) => {
                    let range = self.glyph_ranges.get(glyph_op).cloned().unwrap_or(0..0);
                    glyph_op += 1;
                    self.glyph_instances.get(range).and_then(|g| cover(g.iter().map(|g| g.dest), 0.0))
                }
                Op::Blob(i) => match blobs.get(*i) {
                    Some(Some(d)) => cover(d.sdf.chunks(6).chain(d.solid.chunks(6)).map(|t| {
                        // Üçgen çifti: [x0,y0], [x1,y0], [x0,y1], ..., [x1,y1].
                        let (a, b) = (t[0], t[t.len() - 1]);
                        [a[0], a[1], b[0] - a[0], b[1] - a[1]]
                    }), 0.0),
                    _ => Some(PxRect::default()),
                },
                Op::Image(i) => rec.images.get(*i).and_then(|d| cover(std::iter::once(d.rect), 1.0)),
                Op::Clip(_) => None,
            })
            .collect()
    }

    /// Blob'u çizime hazırlar: geometri (önbellekli), düz dolgu bölmesi ve köşe listeleri.
    /// Kare başına bir kez; hasar parçası başına geçişler yalnızca [`Self::draw_blob`] yapar.
    fn prepare_blob(&self, field: &crate::blob::BlobField, size_px: (u32, u32), scale: f32) -> Option<BlobDraw> {
        let p = field.prepare(scale, size_px)?;
        let mut cache = self.blob_cache.borrow_mut();
        let prims = match cache.entries.iter().position(|(s, _)| *s == p.scene) {
            Some(i) => {
                let hit = cache.entries.remove(i);
                let prims = hit.1.clone();
                cache.entries.push(hit);
                prims
            }
            None => {
                let t = std::time::Instant::now();
                let prims = std::rc::Rc::new(crate::blob::encode(&p.scene));
                tracing::trace!(us = t.elapsed().as_micros() as u64, prims = prims.len(), "blob geometrisi");
                if cache.entries.len() >= BLOB_CACHE {
                    cache.entries.remove(0);
                }
                cache.entries.push((p.scene.clone(), prims.clone()));
                prims
            }
        };
        // Serbest alandan uzak dolu parçalar düz dolguyla (aynı renk, alan döngüsü yok): çerçeve
        // şeritleri ve bar gibi geniş dolu bölgeler yazılım rasterleştiricide piksel başına
        // tüm ilkelleri dolaşmaz.
        let free = crate::blob::free_bounds(&prims).map(|b| b.expand(crate::blob::SOLID_MARGIN));
        let (mut sdf, mut solid) = (Vec::new(), Vec::new());
        for r in &p.rects {
            if self.blob_solid_split {
                crate::blob::split_solid(*r, free, &mut sdf, &mut solid);
            } else {
                sdf.push(*r);
            }
        }
        let triangles = |rects: &[nuxut_core::Rect]| -> Vec<[f32; 2]> {
            rects
                .iter()
                .flat_map(|r| {
                    let (x0, y0, x1, y1) = (r.x, r.y, r.right(), r.bottom());
                    [[x0, y0], [x1, y0], [x0, y1], [x0, y1], [x1, y0], [x1, y1]]
                })
                .collect()
        };
        Some(BlobDraw {
            prims,
            fill: p.fill,
            shadow: p.shadow,
            shadow_geom: p.shadow_geom,
            sdf: triangles(&sdf),
            solid: triangles(&solid),
        })
    }

    /// # Safety
    /// `flush` içinden çağrılır.
    unsafe fn draw_blob(&self, d: &BlobDraw) {
        let gl = &self.gl;
        let u = &self.blob_uniforms;
        let mut cache = self.blob_cache.borrow_mut();
        let upload = !cache.uploaded.as_ref().is_some_and(|u| std::rc::Rc::ptr_eq(u, &d.prims));
        // SAFETY: doku bu renderer'ın; boyut ilkel sayısından (satır başına BLOB_PRIM_ROW ilkel,
        // ilkel başına iki RGBA32F doku), veri tam o boyutta sıfırla doldurulur. Köşe verisi
        // hazırlıkta hesaplanan üçgen listesi.
        unsafe {
            gl.active_texture(glow::TEXTURE0 + BLOB_TEXTURE_UNIT);
            gl.bind_texture(glow::TEXTURE_2D, Some(self.blob_tex));
            if upload && !d.prims.is_empty() {
                let rows = d.prims.len().div_ceil(BLOB_PRIM_ROW);
                let mut data = vec![0.0f32; rows * BLOB_PRIM_ROW * 8];
                data[..d.prims.len() * 8].copy_from_slice(as_f32(&d.prims));
                let w = (BLOB_PRIM_ROW * 2) as i32;
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA32F as i32,
                    w,
                    rows as i32,
                    0,
                    glow::RGBA,
                    glow::FLOAT,
                    glow::PixelUnpackData::Slice(Some(as_bytes(&data))),
                );
                cache.uploaded = Some(d.prims.clone());
            }
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(Some(self.blob_vao));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.blob_vbo));
            if !d.sdf.is_empty() {
                gl.use_program(Some(self.blob.program));
                gl.uniform_1_i32(u.count.as_ref(), d.prims.len() as i32);
                gl.uniform_4_f32_slice(u.fill.as_ref(), &d.fill);
                gl.uniform_4_f32_slice(u.shadow.as_ref(), &d.shadow);
                gl.uniform_3_f32_slice(u.shadow_geom.as_ref(), &d.shadow_geom);
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&d.sdf), glow::STREAM_DRAW);
                gl.draw_arrays(glow::TRIANGLES, 0, d.sdf.len() as i32);
            }
            if !d.solid.is_empty() {
                gl.use_program(Some(self.blob_solid.program));
                gl.uniform_4_f32_slice(u.solid_fill.as_ref(), &d.fill);
                gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, as_bytes(&d.solid), glow::STREAM_DRAW);
                gl.draw_arrays(glow::TRIANGLES, 0, d.solid.len() as i32);
            }
        }
    }
}

impl Renderer {
    /// GL çöplüğündeki değerleri düşürür (context güncel olmalı).
    fn drain_graveyard(&mut self) {
        while let Ok(value) = self.graveyard.try_recv() {
            drop(value);
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Hiç kare çizilmeden kapanırsa da: önce kayıttaki geçişler (dış nesnelere son
        // başvurular), sonra çöplük; context hâlâ güncel (aşağıdaki not).
        self.rec.clear();
        self.drain_graveyard();
        let gl = &self.gl;
        // SAFETY: tüm nesneler bu renderer'a aittir ve başka yerde tutulmaz. Context'in hâlâ
        // güncel olması çağıranın sorumluluğu (runtime renderer'ı EGL'den önce düşürür).
        unsafe {
            for p in [&self.quad, &self.glyph, &self.blob, &self.blob_solid] {
                gl.delete_program(p.program);
            }
            for vao in [self.quad_vao, self.glyph_vao, self.blob_vao] {
                gl.delete_vertex_array(vao);
            }
            for vbo in [self.quad_vbo, self.glyph_vbo, self.blob_vbo] {
                gl.delete_buffer(vbo);
            }
            gl.delete_texture(self.blob_tex);
        }
        self.gradient.destroy(gl);
        self.stroke.destroy(gl);
        self.atlas.destroy(gl);
        self.image.destroy(&self.gl);
        self.blur.destroy(&self.gl);
        self.atlas.destroy(&self.gl);
    }
}

fn glyph_instance(e: &Entry, x: i32, y: i32, color: [f32; 4], page: f32) -> GlyphInstance {
    let (gx, gy) = ((x + e.left) as f32, (y - e.top) as f32);
    let (w, h) = (e.width as f32, e.height as f32);
    GlyphInstance {
        dest: [gx, gy, w, h],
        uv: [e.x as f32 / page, e.y as f32 / page, (e.x as f32 + w) / page, (e.y as f32 + h) / page],
        color,
        kind: if e.kind == Kind::Color { 1.0 } else { 0.0 },
    }
}

/// `#[repr(C)]` düz f32 yapılarının bayt görünümü.
fn as_bytes<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: T yalnızca f32 alanlardan oluşan #[repr(C)] yapılar/diziler (dolgu yok);
    // uzunluk size_of_val ile tam; hizalama u8 için 1.
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), size_of_val(v)) }
}

fn compile(gl: &glow::Context, name: &'static str, vert: &str, frag: &str) -> Result<Program, RenderError> {
    // SAFETY: context güncel; kaynak dizgileri sabit; başarısız nesneler silinir.
    unsafe {
        let program = gl.create_program().map_err(RenderError::Gl)?;
        let mut shaders = Vec::new();
        for (stage, src) in [(glow::VERTEX_SHADER, format!("{HEADER}{vert}")), (glow::FRAGMENT_SHADER, format!("{HEADER}{COMMON}{frag}"))] {
            let shader = gl.create_shader(stage).map_err(RenderError::Gl)?;
            gl.shader_source(shader, &src);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                for s in shaders {
                    gl.delete_shader(s);
                }
                gl.delete_program(program);
                return Err(RenderError::Compile { name, log });
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        let ok = gl.get_program_link_status(program);
        for s in shaders {
            gl.detach_shader(program, s);
            gl.delete_shader(s);
        }
        if !ok {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(RenderError::Link { name, log });
        }
        let viewport = gl.get_uniform_location(program, "u_viewport");
        Ok(Program { program, viewport })
    }
}

fn as_f32(v: &[crate::blob::GpuPrim]) -> &[f32] {
    v.as_flattened()
}
