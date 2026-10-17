//! Başsız (surfaceless EGL) altın görüntü düzeneği: sahneyi FBO'ya çizer, RGBA okur, PNG
//! altın görüntüyle karşılaştırır.
//!
//! - Tolerans: kanal başına ±[`TOLERANCE`]/255; aşan piksel sayısı ve en büyük fark raporlanır.
//! - `NUXUT_BLESS=1`: altın görüntüyü yazar/günceller (karşılaştırma yapılmaz).
//! - Fark varsa `target/golden-diff/<ad>.{expected,actual,diff}.png` yazılır.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use khronos_egl as egl;
use nuxut_render::glow::{self, HasContext};
use nuxut_render::{Painter, Renderer, TextSystem};

/// Kanal başına izin verilen fark (0–255).
pub const TOLERANCE: u8 = 2;

const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

/// Güncel GL context'i olan başsız EGL; düşünce context yok edilir.
pub struct Headless {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
}

impl Drop for Headless {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

/// Surfaceless EGL + GLES 3; yoksa `None` (test atlanır).
pub fn headless() -> Option<(Headless, glow::Context)> {
    // SAFETY: test; libEGL sistem kütüphanesi, API sözleşmesine uyar.
    let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }.ok()?;
    // SAFETY: surfaceless platformu yerel ekran istemez (NULL).
    let display = unsafe {
        egl.get_platform_display(PLATFORM_SURFACELESS_MESA, egl::DEFAULT_DISPLAY, &[egl::ATTRIB_NONE])
    }
    .ok()?;
    egl.initialize(display).ok()?;
    egl.bind_api(egl::OPENGL_ES_API).ok()?;
    let config = egl
        .choose_first_config(display, &[egl::SURFACE_TYPE, 0, egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT, egl::NONE])
        .ok()??;
    let context = egl
        .create_context(display, config, None, &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE])
        .ok()?;
    egl.make_current(display, None, None, Some(context)).ok()?;
    // SAFETY: context güncel; işlev işaretçileri bu context için yüklenir.
    let gl = unsafe {
        glow::Context::from_loader_function(|name| {
            egl.get_proc_address(name).map_or(std::ptr::null(), |f| f as *const _)
        })
    };
    Some((Headless { egl, display, context }, gl))
}

/// Okunmuş RGBA8 görüntü (üst satır önce).
#[derive(Clone, PartialEq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Bir renderer ve yeniden kullanılan FBO ile çizim.
pub struct Harness {
    _egl: Headless,
    renderer: Renderer,
    text: TextSystem,
    fbo: Option<(glow::Framebuffer, glow::Texture, (u32, u32))>,
}

impl Harness {
    pub fn new() -> Option<Self> {
        let (egl, gl) = headless()?;
        let renderer = Renderer::new(gl).ok()?;
        Some(Self { _egl: egl, renderer, text: TextSystem::new(), fbo: None })
    }

    /// `size` mantıksal viewport'u `scale`'de çizer ve okur (fiziksel = mantıksal × ölçek).
    pub fn render(&mut self, size: (f32, f32), scale: f32, f: impl FnOnce(&mut Painter)) -> Rgba {
        let (w, h) = ((size.0 * scale).round() as u32, (size.1 * scale).round() as u32);
        self.bind_target(w, h);
        self.renderer.render(&mut self.text, (w, h), scale, f);
        let gl = self.renderer.gl();
        let mut flipped = vec![0u8; (w * h * 4) as usize];
        // SAFETY: hedef FBO bağlı, tampon w*h*4.
        unsafe {
            gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut flipped)));
        }
        // GL alt satırdan başlar; PNG üstten.
        let row = (w * 4) as usize;
        let pixels = flipped.chunks_exact(row).rev().flatten().copied().collect();
        Rgba { width: w, height: h, pixels }
    }

    fn bind_target(&mut self, w: u32, h: u32) {
        let gl = self.renderer.gl();
        if let Some((fbo, tex, size)) = self.fbo {
            if size == (w, h) {
                // SAFETY: FBO bu context'e ait.
                unsafe { gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo)) };
                return;
            }
            // SAFETY: eski hedef bu context'e ait ve artık kullanılmıyor.
            unsafe {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
            }
        }
        // SAFETY: yeni RGBA8 hedef; renderer bağlı FBO'ya çizer.
        unsafe {
            let tex = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            gl.bind_texture(glow::TEXTURE_2D, None);
            self.fbo = Some((fbo, tex, (w, h)));
        }
    }
}

/// Karşılaştırma sonucu.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Tolerans içinde.
    Match,
    /// Altın görüntü yok (henüz üretilmedi).
    Missing,
    /// `NUXUT_BLESS=1`: yazıldı.
    Blessed,
    /// Boyut farklı.
    SizeMismatch { expected: (u32, u32), actual: (u32, u32) },
    /// Toleransı aşan piksel sayısı ve en büyük kanal farkı.
    Differs { pixels: usize, max: u8 },
}

/// İki görüntünün farkı: toleransı aşan piksel sayısı, en büyük kanal farkı, fark görüntüsü.
pub fn diff(expected: &Rgba, actual: &Rgba) -> (usize, u8, Rgba) {
    let mut over = 0;
    let mut max = 0u8;
    let mut out = Vec::with_capacity(actual.pixels.len());
    for (e, a) in expected.pixels.chunks_exact(4).zip(actual.pixels.chunks_exact(4)) {
        let d = e.iter().zip(a).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0);
        max = max.max(d);
        if d > TOLERANCE {
            over += 1;
        }
        // Fark görüntüsü: aşan pikseller kırmızı (şiddet farkla), diğerleri soluk gri.
        let v = if d > TOLERANCE { 128u8.saturating_add(d / 2) } else { 0 };
        let g = if d > TOLERANCE { 0 } else { a[3] / 6 };
        out.extend_from_slice(&[v.max(g), g, g, 255]);
    }
    (over, max, Rgba { width: actual.width, height: actual.height, pixels: out })
}

/// `tests/golden`; `NUXUT_GOLDEN_DIR` ile başka dizin (deneme çıktıları, ör. scratchpad).
pub fn golden_dir() -> PathBuf {
    std::env::var_os("NUXUT_GOLDEN_DIR")
        .map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"), PathBuf::from)
}

pub fn diff_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden-diff")
}

pub fn bless_requested() -> bool {
    std::env::var_os("NUXUT_BLESS").is_some_and(|v| v != "0")
}

/// `<dir>/<name>.png` ile karşılaştırır (ya da `bless` ise yazar). Fark varsa `diff_dir`'e
/// beklenen/gerçek/fark PNG'leri yazılır.
pub fn check(dir: &Path, diff_dir: &Path, name: &str, actual: &Rgba, bless: bool) -> Outcome {
    let path = dir.join(format!("{name}.png"));
    if bless {
        std::fs::create_dir_all(dir).unwrap();
        save(&path, actual);
        return Outcome::Blessed;
    }
    let Some(expected) = load(&path) else {
        return Outcome::Missing;
    };
    if (expected.width, expected.height) != (actual.width, actual.height) {
        write_diff(diff_dir, name, &expected, actual, None);
        return Outcome::SizeMismatch {
            expected: (expected.width, expected.height),
            actual: (actual.width, actual.height),
        };
    }
    let (pixels, max, image) = diff(&expected, actual);
    if pixels == 0 {
        return Outcome::Match;
    }
    write_diff(diff_dir, name, &expected, actual, Some(&image));
    Outcome::Differs { pixels, max }
}

fn write_diff(dir: &Path, name: &str, expected: &Rgba, actual: &Rgba, diff: Option<&Rgba>) {
    std::fs::create_dir_all(dir).unwrap();
    save(&dir.join(format!("{name}.expected.png")), expected);
    save(&dir.join(format!("{name}.actual.png")), actual);
    if let Some(d) = diff {
        save(&dir.join(format!("{name}.diff.png")), d);
    }
}

pub fn save(path: &Path, image: &Rgba) {
    image::save_buffer(path, &image.pixels, image.width, image.height, image::ExtendedColorType::Rgba8)
        .unwrap_or_else(|e| panic!("{} yazılamadı: {e}", path.display()));
}

pub fn load(path: &Path) -> Option<Rgba> {
    let img = image::open(path).ok()?.into_rgba8();
    Some(Rgba { width: img.width(), height: img.height(), pixels: img.into_raw() })
}
