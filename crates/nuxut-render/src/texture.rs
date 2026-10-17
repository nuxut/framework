//! Görsel dokuları ve `Painter::image`: yuvarlatılmış köşe SDF kırpmalı, premultiplied doku
//! çizimi.
//!
//! Yaşam döngüsü: `Painter::upload_texture` yalnızca kaydeder (Painter GL çağırmaz); `Renderer`
//! aynı karede çizimden önce yükler. [`Texture`] bir `Arc` tutamağıdır; son klon düşünce id
//! renderer'ın serbest bırakma kuyruğuna gider ve sonraki karede GL dokusu silinir — tutamak
//! hangi thread'de düşerse düşsün.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use glow::HasContext;
use nuxut_core::{Color, Corners, Rect};

use crate::error::RenderError;

/// Kabul edilen en büyük doku kenarı (GL sınırı daha küçükse o geçerli).
pub const MAX_TEXTURE_SIDE: u32 = 8192;
/// Bir karede GL'e gönderilen en fazla doku baytı. Aşan yüklemeler sonraki karelere kalır
/// (her karede en az bir yükleme yapılır); büyük duvar kağıdı bir kareyi takılmaz.
pub const UPLOAD_BUDGET_PER_FRAME: usize = 64 << 20;

/// Yüklenecek piksel verisi: satırlar sıkı, **premultiplied** RGBA8.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    /// Paylaşımlı: animasyon kareleri kopyalanmadan yeniden yüklenebilir.
    pub pixels: Arc<[u8]>,
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImageData({}x{})", self.width, self.height)
    }
}

impl ImageData {
    fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && (self.width as usize).checked_mul(self.height as usize).and_then(|n| n.checked_mul(4))
                == Some(self.pixels.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextureOptions {
    /// Mipmap üret (`LINEAR_MIPMAP_LINEAR`): doku çizildiği boyuttan çok büyükse kenar
    /// titremesini önler. Zaten hedef boyuta küçültülmüş görsellerde gereksiz.
    pub mipmaps: bool,
}

/// GPU dokusu tutamağı. Ucuz klonlanır; son klon düşünce doku serbest bırakılır.
#[derive(Clone)]
pub struct Texture(Arc<Handle>);

pub(crate) struct Handle {
    id: u64,
    /// GL dokusu yüklendi (bekleyen ilk yükleme bitti).
    ready: std::sync::atomic::AtomicBool,
    /// `w << 32 | h`; `update_texture` boyut değiştirebilir.
    size: AtomicU64,
    release: Option<Sender<u64>>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        if let Some(tx) = &self.release {
            // Renderer düştüyse kanal kapalıdır; doku zaten silinmiştir.
            let _ = tx.send(self.id);
        }
    }
}

impl std::fmt::Debug for Texture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (w, h) = self.size();
        write!(f, "Texture(#{} {w}x{h})", self.0.id)
    }
}

impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id
    }
}

impl Texture {
    /// Piksel boyutu.
    pub fn size(&self) -> (u32, u32) {
        let v = self.0.size.load(Ordering::Relaxed);
        ((v >> 32) as u32, v as u32)
    }

    /// İlk yükleme GL'e gitti mi. Kare bütçesi yüzünden bekleyen doku çizilmez; uygulama
    /// `false` iken bir kare daha ister (ör. çapraz geçişi doku hazır olunca başlatır).
    pub fn is_ready(&self) -> bool {
        self.0.ready.load(Ordering::Acquire)
    }

    pub(crate) fn id(&self) -> u64 {
        self.0.id
    }

    fn set_size(&self, w: u32, h: u32) {
        self.0.size.store(pack(w, h), Ordering::Relaxed);
    }
}

fn pack(w: u32, h: u32) -> u64 {
    (u64::from(w) << 32) | u64::from(h)
}

/// Görselin hedef dikdörtgene yerleşimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageFit {
    /// Dikdörtgeni kaplar; taşan kısım kırpılır (ortadan).
    #[default]
    Cover,
    /// Dikdörtgenin içine sığar, ortalanır; köşe yarıçapları görselin kendisine uygulanır.
    Contain,
    /// Dikdörtgene gerilir.
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageStyle {
    pub fit: ImageFit,
    pub radii: Corners,
    /// 0…1, görselin alpha'sıyla çarpılır.
    pub opacity: f32,
    /// Tek renkli boyama: çıktı = renk × doku alfası (sembolik/tepsi ikonları tema rengiyle).
    pub tint: Option<Color>,
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self { fit: ImageFit::Cover, radii: Corners::ZERO, opacity: 1.0, tint: None }
    }
}

impl ImageStyle {
    pub fn new(fit: ImageFit) -> Self {
        Self { fit, ..Self::default() }
    }

    pub fn with_radii(self, radii: Corners) -> Self {
        Self { radii, ..self }
    }

    pub fn with_opacity(self, opacity: f32) -> Self {
        Self { opacity, ..self }
    }

    pub fn with_tint(self, tint: Color) -> Self {
        Self { tint: Some(tint), ..self }
    }
}

impl ImageFit {
    /// `texture` boyutundaki (piksel; yalnızca en-boy oranı kullanılır) görselin `rect`'e
    /// yerleşimi: çizildiği dikdörtgen ve doku koordinatları `[u0, v0, u1, v1]`.
    /// `Painter::image` ile aynı hesap. `Contain`'de dikdörtgen `rect`'ten küçük olabilir
    /// (letterbox), `Cover`'da doku koordinatları kırpılır; boş `rect` olduğu gibi döner.
    pub fn placement(self, rect: Rect, texture: (u32, u32)) -> (Rect, [f32; 4]) {
        layout(rect, texture, self)
    }
}

/// `fit`'e göre (mantıksal) hedef dikdörtgen ve doku koordinatları (u0, v0, u1, v1).
pub(crate) fn layout(rect: Rect, tex: (u32, u32), fit: ImageFit) -> (Rect, [f32; 4]) {
    let (tw, th) = (tex.0.max(1) as f32, tex.1.max(1) as f32);
    let full = [0.0, 0.0, 1.0, 1.0];
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return (rect, full);
    }
    let (ra, ta) = (rect.width / rect.height, tw / th);
    match fit {
        ImageFit::Fill => (rect, full),
        ImageFit::Cover => {
            if ta > ra {
                // Görsel daha geniş: yanlardan kırp.
                let u = ra / ta;
                (rect, [(1.0 - u) * 0.5, 0.0, (1.0 + u) * 0.5, 1.0])
            } else {
                let v = ta / ra;
                (rect, [0.0, (1.0 - v) * 0.5, 1.0, (1.0 + v) * 0.5])
            }
        }
        ImageFit::Contain => {
            let (w, h) = if ta > ra { (rect.width, rect.width / ta) } else { (rect.height * ta, rect.height) };
            let r = Rect::new(rect.x + (rect.width - w) * 0.5, rect.y + (rect.height - h) * 0.5, w, h);
            (r, full)
        }
    }
}

/// Kayıtta bekleyen yükleme.
#[derive(Debug)]
pub(crate) struct Upload {
    pub id: u64,
    /// Hazır bayrağı için tutamağın zayıf kopyası (tutamak düştüyse yükleme atlanır).
    pub handle: std::sync::Weak<Handle>,
    pub data: ImageData,
    /// `None` = mevcut dokunun seçenekleri (güncelleme).
    pub options: Option<TextureOptions>,
}

/// Kayıtta bekleyen bulanıklık işi: `src`'nin bulanık kopyası `dst` tutamağına üretilir.
#[derive(Debug)]
pub(crate) struct BlurJob {
    /// Kaynak, iş bitene kadar canlı tutulur.
    pub src: Texture,
    pub dst_id: u64,
    pub dst: std::sync::Weak<Handle>,
    /// Fiziksel piksel.
    pub radius_px: f32,
}

/// Bir görsel çizimi (fiziksel piksel).
#[derive(Debug, Clone)]
pub(crate) struct ImageDraw {
    /// Kare boyunca canlı tutar: aynı karede yükleyip düşürülen doku yine çizilir.
    pub texture: Texture,
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub radii: [f32; 4],
    pub opacity: f32,
    /// Premultiplied boyama rengi; alfa 0 = boyama yok.
    pub tint: [f32; 4],
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Painter tarafı: yeni tutamak üretir (id + serbest bırakma kanalı).
pub(crate) fn new_texture(release: Option<Sender<u64>>, w: u32, h: u32) -> Texture {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    Texture(Arc::new(Handle { id, ready: std::sync::atomic::AtomicBool::new(false), size: AtomicU64::new(pack(w, h)), release }))
}

pub(crate) fn weak(t: &Texture) -> std::sync::Weak<Handle> {
    Arc::downgrade(&t.0)
}

pub(crate) fn resize_handle(t: &Texture, w: u32, h: u32) {
    t.set_size(w, h);
}

#[derive(Debug)]
struct GlTexture {
    texture: glow::Texture,
    width: u32,
    height: u32,
    mipmaps: bool,
    /// `render_target` dokuları için framebuffer.
    fbo: Option<glow::Framebuffer>,
}

/// Renderer tarafı: GL dokuları, görsel programı ve serbest bırakma kuyruğu.
#[derive(Debug)]
pub(crate) struct ImagePipeline {
    program: glow::Program,
    viewport: Option<glow::UniformLocation>,
    u_rect: Option<glow::UniformLocation>,
    u_uv: Option<glow::UniformLocation>,
    u_radii: Option<glow::UniformLocation>,
    u_opacity: Option<glow::UniformLocation>,
    u_tint: Option<glow::UniformLocation>,
    vao: glow::VertexArray,
    textures: HashMap<u64, GlTexture>,
    /// Kare bütçesini aşıp sonraki kareye kalan yüklemeler (sırası korunur).
    deferred: Vec<Upload>,
    release_tx: Sender<u64>,
    release_rx: Receiver<u64>,
    max_side: u32,
}

impl ImagePipeline {
    pub fn new(gl: &glow::Context, program: glow::Program, viewport: Option<glow::UniformLocation>) -> Result<Self, RenderError> {
        // SAFETY: context güncel; program az önce bağlandı; yalnızca nesne/uniform sorgusu.
        let (vao, u_rect, u_uv, u_radii, u_opacity, u_tint, max_side) = unsafe {
            let u = |n: &str| gl.get_uniform_location(program, n);
            gl.use_program(Some(program));
            gl.uniform_1_i32(u("u_tex").as_ref(), 0);
            gl.use_program(None);
            let max = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(1) as u32;
            (gl.create_vertex_array().map_err(RenderError::Gl)?, u("u_rect"), u("u_uv"), u("u_radii"), u("u_opacity"), u("u_tint"), max)
        };
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        Ok(Self {
            program,
            viewport,
            u_rect,
            u_uv,
            u_radii,
            u_opacity,
            u_tint,
            vao,
            deferred: Vec::new(),
            textures: HashMap::new(),
            release_tx,
            release_rx,
            max_side: max_side.min(MAX_TEXTURE_SIDE),
        })
    }

    pub fn release_sender(&self) -> Sender<u64> {
        self.release_tx.clone()
    }

    pub fn program(&self) -> (glow::Program, Option<&glow::UniformLocation>) {
        (self.program, self.viewport.as_ref())
    }

    /// Canlı doku sayısı ve toplam bayt (mipmap dahil yaklaşık).
    pub fn stats(&self) -> (usize, usize) {
        let bytes = self
            .textures
            .values()
            .map(|t| t.width as usize * t.height as usize * 4 * if t.mipmaps { 4 } else { 3 } / 3)
            .sum();
        (self.textures.len(), bytes)
    }

    /// Bekleyen yüklemeleri GL'e gönderir. Geçersiz/aşırı büyük veri atlanır (doku yok →
    /// çizim atlanır).
    pub fn upload(&mut self, gl: &glow::Context, uploads: &mut Vec<Upload>) {
        // Önce önceki karelerden kalanlar; aynı doku için yalnızca en yeni yükleme gerekir.
        let mut queue = std::mem::take(&mut self.deferred);
        for up in uploads.drain(..) {
            queue.retain(|q| q.id != up.id);
            queue.push(up);
        }
        let mut spent = 0usize;
        let mut rest = queue.into_iter();
        for up in rest.by_ref() {
            if spent > 0 && spent + up.data.pixels.len() > UPLOAD_BUDGET_PER_FRAME {
                self.deferred.push(up);
                break;
            }
            let Some(handle) = up.handle.upgrade() else { continue };
            let ImageData { width: w, height: h, .. } = up.data;
            if !up.data.is_valid() || w > self.max_side || h > self.max_side {
                tracing::warn!("doku yüklenmedi: {w}x{h} ({} bayt, sınır {} px)", up.data.pixels.len(), self.max_side);
                continue;
            }
            spent += up.data.pixels.len();
            let prev = self.textures.get(&up.id);
            let mipmaps = up.options.map(|o| o.mipmaps).or(prev.map(|t| t.mipmaps)).unwrap_or(false);
            let existing = prev.filter(|t| t.width == w && t.height == h && t.mipmaps == mipmaps).map(|t| t.texture);
            // SAFETY: context güncel; veri uzunluğu w*h*4 doğrulandı; doku bu boru hattına ait.
            unsafe {
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                let tex = match existing {
                    Some(tex) => {
                        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                        gl.tex_sub_image_2d(
                            glow::TEXTURE_2D,
                            0,
                            0,
                            0,
                            w as i32,
                            h as i32,
                            glow::RGBA,
                            glow::UNSIGNED_BYTE,
                            glow::PixelUnpackData::Slice(Some(&up.data.pixels[..])),
                        );
                        tex
                    }
                    None => {
                        if let Some(old) = self.textures.remove(&up.id) {
                            delete(gl, &old);
                        }
                        let Ok(tex) = gl.create_texture() else {
                            tracing::error!("GL dokusu yaratılamadı");
                            continue;
                        };
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
                            glow::PixelUnpackData::Slice(Some(&up.data.pixels[..])),
                        );
                        let min = if mipmaps { glow::LINEAR_MIPMAP_LINEAR } else { glow::LINEAR };
                        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, min as i32);
                        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                        self.textures.insert(up.id, GlTexture { texture: tex, width: w, height: h, mipmaps, fbo: None });
                        tex
                    }
                };
                if mipmaps {
                    gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                    gl.generate_mipmap(glow::TEXTURE_2D);
                }
                gl.bind_texture(glow::TEXTURE_2D, None);
            }
            handle.ready.store(true, Ordering::Release);
        }
        self.deferred.extend(rest);
    }

    /// Bütçe yüzünden sonraki kareye kalan yükleme var mı.
    pub fn has_deferred(&self) -> bool {
        !self.deferred.is_empty()
    }

    /// Düşürülen tutamakların dokularını siler.
    pub fn collect_garbage(&mut self, gl: &glow::Context) {
        while let Ok(id) = self.release_rx.try_recv() {
            if let Some(t) = self.textures.remove(&id) {
                delete(gl, &t);
            }
        }
    }

    /// # Safety
    /// `flush` içinden; context güncel, viewport uniform'u bu karede ayarlanmış olmalı.
    pub unsafe fn draw(&self, gl: &glow::Context, d: &ImageDraw) {
        let Some(t) = self.textures.get(&d.texture.id()) else { return };
        // SAFETY: doku canlı; köşeler gl_VertexID'den üretilir (öznitelik yok).
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_4_f32_slice(self.u_rect.as_ref(), &d.rect);
            gl.uniform_4_f32_slice(self.u_uv.as_ref(), &d.uv);
            gl.uniform_4_f32_slice(self.u_radii.as_ref(), &d.radii);
            gl.uniform_1_f32(self.u_opacity.as_ref(), d.opacity);
            gl.uniform_4_f32_slice(self.u_tint.as_ref(), &d.tint);
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.texture));
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// Dokunun bütçe yüzünden sonraki kareye kalmış bir güncellemesi var mı. Varsa GPU'daki
    /// içerik eskidir: ondan türetilen işler (bulanıklık) beklemeli.
    pub(crate) fn has_pending_upload(&self, id: u64) -> bool {
        self.deferred.iter().any(|u| u.id == id)
    }

    /// Yüklenmiş dokunun GL adı ve boyutu (bulanıklık gibi GPU işlemleri için).
    pub(crate) fn gl_texture(&self, id: u64) -> Option<(glow::Texture, u32, u32)> {
        self.textures.get(&id).map(|t| (t.texture, t.width, t.height))
    }

    /// GPU'da üretilmiş bir dokuyu (`tex`) işin hedef tutamağına bağlar ve hazır işaretler.
    /// Tutamak düştüyse doku hemen silinir.
    pub(crate) fn adopt(&mut self, gl: &glow::Context, job: &BlurJob, tex: glow::Texture, w: u32, h: u32) {
        let Some(handle) = job.dst.upgrade() else {
            // SAFETY: doku az önce bu boru hattında yaratıldı; kimse başvurmuyor.
            unsafe { gl.delete_texture(tex) };
            return;
        };
        if let Some(old) = self.textures.insert(job.dst_id, GlTexture { texture: tex, width: w, height: h, mipmaps: false, fbo: None }) {
            delete(gl, &old);
        }
        handle.size.store(pack(w, h), Ordering::Relaxed);
        handle.ready.store(true, Ordering::Release);
    }

    pub fn destroy(&mut self, gl: &glow::Context) {
        // SAFETY: tüm nesneler bu boru hattına aittir.
        unsafe {
            for (_, t) in self.textures.drain() {
                delete(gl, &t);
            }
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
    }
}

fn delete(gl: &glow::Context, t: &GlTexture) {
    // SAFETY: doku ve framebuffer bu boru hattına aittir; tutamağı kalmadı.
    unsafe {
        if let Some(f) = t.fbo {
            gl.delete_framebuffer(f);
        }
        gl.delete_texture(t.texture);
    }
}

use crate::gl::GlPass;

/// Bekleyen boş hedef doku ayırma.
#[derive(Debug)]
pub(crate) struct TargetAlloc {
    pub id: u64,
    pub handle: std::sync::Weak<Handle>,
    pub width: u32,
    pub height: u32,
}

/// Bekleyen GL geçişi; doku kare boyunca canlı tutulur.
pub(crate) struct PassJob {
    pub target: Texture,
    pub pass: Arc<dyn GlPass>,
}

impl std::fmt::Debug for PassJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PassJob({:?})", self.target)
    }
}

impl ImagePipeline {
    /// Boş RGBA8 hedef dokuları ve framebuffer'larını ayırır (bütçe dışı: piksel yüklemesi yok).
    pub fn allocate_targets(&mut self, gl: &glow::Context, targets: &mut Vec<TargetAlloc>) {
        for t in targets.drain(..) {
            let Some(handle) = t.handle.upgrade() else { continue };
            let (w, h) = (t.width.clamp(1, self.max_side), t.height.clamp(1, self.max_side));
            if let Some(old) = self.textures.remove(&t.id) {
                delete(gl, &old);
            }
            // SAFETY: context güncel; boş (NULL) ayırma; nesneler bu boru hattına ait.
            let created = unsafe {
                let Ok(tex) = gl.create_texture() else { continue };
                gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA8 as i32, w as i32, h as i32, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                for (k, v) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, k, v as i32);
                }
                gl.bind_texture(glow::TEXTURE_2D, None);
                let prev = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
                let fbo = gl.create_framebuffer().ok();
                if let Some(f) = fbo {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(f));
                    gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
                    // Şeffaf başlasın (ilk geçişe kadar).
                    gl.clear_color(0.0, 0.0, 0.0, 0.0);
                    gl.disable(glow::SCISSOR_TEST);
                    gl.viewport(0, 0, w as i32, h as i32);
                    gl.clear(glow::COLOR_BUFFER_BIT);
                    gl.bind_framebuffer(glow::FRAMEBUFFER, framebuffer_from_raw(prev));
                }
                GlTexture { texture: tex, width: w, height: h, mipmaps: false, fbo }
            };
            self.textures.insert(t.id, created);
            handle.ready.store(true, Ordering::Release);
        }
    }

    /// GL geçişlerini çalıştırır; her geçişten sonra renderer'ın dayandığı durumu geri yükler.
    pub fn run_passes(&mut self, gl: &glow::Context, passes: &[PassJob]) {
        for job in passes {
            let Some(t) = self.textures.get(&job.target.id()) else { continue };
            let Some(fbo) = t.fbo else {
                tracing::warn!("gl_pass: {:?} bir render_target değil", job.target);
                continue;
            };
            let size = (t.width, t.height);
            // SAFETY: context güncel; sorgular salt okunur; geri yüklenen değerler az önce okundu.
            unsafe {
                let prev_fb = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
                let mut vp = [0i32; 4];
                gl.get_parameter_i32_slice(glow::VIEWPORT, &mut vp);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.viewport(0, 0, size.0 as i32, size.1 as i32);
                gl.disable(glow::SCISSOR_TEST);
                job.pass.render(gl, fbo.0.get(), size);
                // Dış kod durumu değiştirmiş olabilir: renderer'ın varsaydıklarını geri kur.
                gl.bind_framebuffer(glow::FRAMEBUFFER, framebuffer_from_raw(prev_fb));
                gl.viewport(vp[0], vp[1], vp[2], vp[3]);
                gl.use_program(None);
                gl.bind_vertex_array(None);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
                // PBO bağlı kalırsa sonraki tex_(sub_)image_2d PBO'dan ofset okur.
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, None);
                // Renderer'ın kullandığı doku birimleri (0: maske/görsel, 1: renkli atlas):
                // sampler nesnesi dokunun kendi filtresini ezer.
                for unit in [glow::TEXTURE1, glow::TEXTURE0] {
                    gl.active_texture(unit);
                    gl.bind_sampler(unit - glow::TEXTURE0, None);
                    gl.bind_texture(glow::TEXTURE_2D, None);
                }
                gl.disable(glow::SCISSOR_TEST);
                gl.disable(glow::STENCIL_TEST);
                gl.disable(glow::DEPTH_TEST);
                gl.disable(glow::CULL_FACE);
                gl.disable(glow::POLYGON_OFFSET_FILL);
                gl.color_mask(true, true, true, true);
                gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
                gl.pixel_store_i32(glow::PACK_ALIGNMENT, 4);
                gl.blend_equation(glow::FUNC_ADD);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            }
        }
    }
}

fn framebuffer_from_raw(raw: i32) -> Option<glow::Framebuffer> {
    std::num::NonZeroU32::new(raw as u32).map(glow::NativeFramebuffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_crops_center_contain_letterboxes() {
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        // Kare doku, geniş hedef: cover dikeyde kırpar.
        let (dst, uv) = ImageFit::Cover.placement(r, (100, 100));
        assert_eq!(dst, r);
        assert_eq!(uv, [0.0, 0.25, 1.0, 0.75]);
        // Contain: 100x100 ortada.
        let (dst, uv) = ImageFit::Contain.placement(r, (100, 100));
        assert_eq!(dst, Rect::new(50.0, 0.0, 100.0, 100.0));
        assert_eq!(uv, [0.0, 0.0, 1.0, 1.0]);
        // Çok geniş doku, kare hedef: cover yanlardan.
        let (_, uv) = layout(Rect::new(0.0, 0.0, 100.0, 100.0), (400, 100), ImageFit::Cover);
        assert_eq!(uv, [0.375, 0.0, 0.625, 1.0]);
    }

    #[test]
    fn dropping_last_handle_queues_release() {
        let (tx, rx) = std::sync::mpsc::channel();
        let t = new_texture(Some(tx), 4, 2);
        assert_eq!(t.size(), (4, 2));
        let t2 = t.clone();
        drop(t);
        assert!(rx.try_recv().is_err());
        let id = t2.id();
        drop(t2);
        assert_eq!(rx.try_recv(), Ok(id));
    }

    #[test]
    fn validates_image_data() {
        assert!(ImageData { width: 2, height: 2, pixels: vec![0; 16].into() }.is_valid());
        assert!(!ImageData { width: 2, height: 2, pixels: vec![0; 15].into() }.is_valid());
        assert!(!ImageData { width: 0, height: 2, pixels: vec![].into() }.is_valid());
    }
}
