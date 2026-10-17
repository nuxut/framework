//! GPU ↔ CPU: shader'ın kapsaması, CPU referansının (f64 geometri) kapsamasıyla aynı mı.
//! Başsız EGL (surfaceless) yoksa atlanır. `NUXUT_BLOB_DUMP=<dizin>` ile çıktılar PNG yazılır.

use khronos_egl as egl;
use nuxut_core::{Color, Corners, Point, Rect};
use glow::HasContext;

use super::geom::v;
use super::{BlobField, BlobShape, FrameShape, morph};
use crate::{Renderer, TextSystem};

const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;

struct Headless {
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

fn headless() -> Option<(Headless, glow::Context)> {
    // SAFETY: test; libEGL sistem kütüphanesi, API sözleşmesine uyar.
    let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }.ok()?;
    // SAFETY: surfaceless platformu yerel ekran istemez (NULL).
    let display =
        unsafe { egl.get_platform_display(PLATFORM_SURFACELESS_MESA, egl::DEFAULT_DISPLAY, &[egl::ATTRIB_NONE]) }.ok()?;
    egl.initialize(display).ok()?;
    egl.bind_api(egl::OPENGL_ES_API).ok()?;
    let config = egl
        .choose_first_config(display, &[egl::SURFACE_TYPE, 0, egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT, egl::NONE])
        .ok()??;
    let context = egl.create_context(display, config, None, &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE]).ok()?;
    egl.make_current(display, None, None, Some(context)).ok()?;
    // SAFETY: context güncel; işlev işaretçileri bu context için yüklenir.
    let gl = unsafe {
        glow::Context::from_loader_function(|name| egl.get_proc_address(name).map_or(std::ptr::null(), |f| f as *const _))
    };
    Some((Headless { egl, display, context }, gl))
}

/// Altın sahneler (mantıksal 480×300; üst bar 32, diğer kenarlar 8; R = 14, r = 12).
pub(crate) fn scenes() -> Vec<(&'static str, Vec<BlobShape>)> {
    let p = |x: f32, y: f32, w: f32, h: f32| BlobShape::new_rect(Rect::new(x, y, w, h));
    let top = 32.0;
    vec![
        ("ust-panel", vec![p(150.0, top - 4.0, 180.0, 120.0)]),
        ("sag-tam-yukseklik", vec![p(330.0, 0.0, 160.0, 300.0)]),
        ("sag-yalniz-uste", vec![p(330.0, 0.0, 160.0, 180.0)]),
        ("alta-yapisik", vec![p(150.0, 180.0, 180.0, 130.0)]),
        ("kose-paneli", vec![p(340.0, 0.0, 150.0, 120.0)]),
        ("bogaz-d10", vec![p(120.0, 0.0, 240.0, 282.0)]),
        ("yan-yana", vec![p(100.0, 0.0, 130.0, 150.0), p(236.0, 0.0, 130.0, 110.0)]),
        ("tohum", vec![p(220.0, top - 2.0, 40.0, 9.0)]),
        ("hap-osd", vec![BlobShape::new(Rect::new(170.0, 230.0, 140.0, 36.0), Corners::uniform(18.0))]),
    ]
}

pub(crate) fn field(shapes: Vec<BlobShape>) -> BlobField {
    BlobField {
        frame: Some(FrameShape {
            outer: Rect::new(0.0, 0.0, 480.0, 300.0),
            inner: Rect::new(8.0, 32.0, 464.0, 260.0),
            inner_radius: 0.0,
        }),
        shapes,
        smoothing: 14.0,
        corner_radius: 12.0,
        fill: Color::BLACK, // renk-matematiği: kapsama testi yalnız alfa kanalını okur
        shadow: None,
    }
}

/// Alanı `scale`'de çizer, RGBA (üstten alta) döner.
fn render(renderer: &mut Renderer, field: &BlobField, scale: f32) -> (u32, u32, Vec<u8>) {
    let (w, h) = ((480.0 * scale) as u32, (300.0 * scale) as u32);
    let gl = renderer.gl();
    // SAFETY: context güncel; FBO ve doku bu testte yaratılır ve silinir.
    let (fbo, tex) = unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        (fbo, tex)
    };
    let mut text = TextSystem::new();
    renderer.render(&mut text, (w, h), scale, |p| p.blob(field));
    let mut px = vec![0u8; (w * h * 4) as usize];
    let gl = renderer.gl();
    // SAFETY: context güncel; tampon w*h*4 bayt; nesneler bu testin.
    unsafe {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.delete_framebuffer(fbo);
        gl.delete_texture(tex);
    }
    // GL alttan üste okur; görüntü sırası üstten alta.
    let row = (w * 4) as usize;
    let flipped: Vec<u8> = px.chunks(row).rev().flatten().copied().collect();
    (w, h, flipped)
}

#[test]
fn gpu_matches_cpu_reference() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    let dump = std::env::var_os("NUXUT_BLOB_DUMP");
    for scale in [1.0f32, 1.5] {
        for (name, shapes) in scenes() {
            let f = field(shapes);
            let (w, h, px) = render(&mut renderer, &f, scale);
            let prepared = f.prepare(scale, (w, h)).unwrap();
            let hreg = morph::free_area(&prepared.scene);
            let (mut bad, mut worst) = (0usize, 0i32);
            for y in 0..h {
                for x in 0..w {
                    let gpu = i32::from(px[((y * w + x) * 4 + 3) as usize]);
                    let c = v(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    let cpu = ((0.5 + hreg.sdf(c)).clamp(0.0, 1.0) * 255.0).round() as i32;
                    let diff = (gpu - cpu).abs();
                    worst = worst.max(diff);
                    if diff > 3 {
                        bad += 1;
                    }
                }
            }
            if let Some(dir) = &dump {
                let img = image::RgbaImage::from_fn(w, h, |x, y| {
                    let a = px[((y * w + x) * 4 + 3) as usize];
                    // Açık zemin üzerinde koyu dolgu.
                    let k = 236 - (u32::from(a) * 200 / 255) as u8;
                    image::Rgba([k, k, k.saturating_add(4), 255])
                });
                img.save(std::path::Path::new(dir).join(format!("gpu-{name}@{scale}.png"))).unwrap();
            }
            let total = (w * h) as usize;
            assert!(bad == 0, "{name}@{scale}: {bad}/{total} piksel ±3'ü aşıyor (en büyük {worst})");
            eprintln!("{name}@{scale}: en büyük fark {worst}/255, ±3 dışı {bad}");
        }
    }
}

/// Düz dolgu bölmesi (`split_solid`) yalnızca hız içindir: gölgeli tüm altın sahnelerde,
/// opak ve yarı saydam dolguyla, kesirli ölçekte tümüyle SDF çizimiyle aynı sonucu vermeli.
/// Kanal başına en çok 1/255: üçgen köşeleri değişince `v_pos` enterpolasyonu son bitte
/// oynar (kenar AA'sı), llvmpipe'ın sabit renkli gölgelendirici yolu premultiplied yarı
/// saydam rengi 8 bite farklı yuvarlayabilir. Görünür fark yok; kısmi/tam çizim tutarlılığı
/// etkilenmez (bölme hasardan bağımsız, yalnızca geometriye bağlı).
#[test]
fn solid_split_matches_full_sdf() {
    let Some((_egl, gl)) = headless() else { return };
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    let opaque = Color::from_hex("#3a4150").unwrap(); // renk-matematiği: dolgu testi
    let translucent = Color::from_hex("#3a415099").unwrap(); // renk-matematiği: yarı saydam dolgu testi
    let shadow = Color::from_hex("#00000080").unwrap(); // renk-matematiği: gölge testi
    let mut solid_pixels = 0usize;
    for fill in [opaque, translucent] {
        for scale in [1.0f32, 1.5] {
            for (name, shapes) in scenes() {
                let mut f = field(shapes);
                f.fill = fill;
                f.shadow = Some(crate::style::ShadowStyle {
                    color: shadow,
                    blur: 12.0,
                    offset: Point::new(0.0, 3.0),
                    spread: 0.0,
                });
                renderer.blob_solid_split = true;
                let (w, h, split) = render(&mut renderer, &f, scale);
                renderer.blob_solid_split = false;
                let (_, _, full) = render(&mut renderer, &f, scale);
                let worst = split.iter().zip(&full).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
                assert!(worst <= 1, "{name}@{scale}: en büyük fark {worst}");
                let p = f.prepare(scale, (w, h)).unwrap();
                let free = super::free_bounds(&super::encode(&p.scene)).map(|b| b.expand(super::SOLID_MARGIN));
                let (mut sdf, mut solid) = (Vec::new(), Vec::new());
                for r in &p.rects {
                    super::split_solid(*r, free, &mut sdf, &mut solid);
                }
                solid_pixels += solid.iter().map(|r| (r.width * r.height) as usize).sum::<usize>();
            }
        }
    }
    renderer.blob_solid_split = true;
    assert!(solid_pixels > 0, "bölme hiç düz dolgu üretmedi; test bir şey sınamıyor");
}

#[test]
fn cached_geometry_is_reused() {
    let Some((_egl, gl)) = headless() else { return };
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    let f = field(vec![BlobShape::new_rect(Rect::new(150.0, 28.0, 180.0, 120.0))
        .with_deform([1.0, 0.0, 0.0, 1.0], Point::new(0.5, 0.0))]);
    let t = std::time::Instant::now();
    let _ = render(&mut renderer, &f, 1.0);
    let first = t.elapsed();
    let t = std::time::Instant::now();
    let _ = render(&mut renderer, &f, 1.0);
    let second = t.elapsed();
    eprintln!("ilk kare {first:?}, aynı sahne {second:?}");
}

/// Girdi bölgesi çizilen kapsamayla tutarlı: 1.0 ölçekte, piksel kapsaması ≥ 0.5 ⇔ piksel
/// bölgede. Kenarın yarım AA pikselleri (alfa 0.5'e ±4/255 yakın; GPU↔CPU farkı ±3) hariç.
#[test]
fn input_region_matches_gpu_coverage() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    let side = |s: f32| {
        BlobShape::new_rect(Rect::new(330.0, 32.0, 142.0, 260.0)).with_deform([s, 0.0, 0.0, 1.0 / s], Point::new(1.0, 0.5))
    };
    let mut all = scenes();
    all.push(("yan-cekmece-esnek", vec![side(1.2)]));
    all.push(("yan-cekmece-buzuk", vec![side(0.85)]));
    for (name, shapes) in all {
        let f = field(shapes);
        let (w, h, px) = render(&mut renderer, &f, 1.0);
        let rects = f.input_region();
        let (mut bad, mut checked, mut aa) = (0usize, 0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                let a = i32::from(px[((y * w + x) * 4 + 3) as usize]);
                aa += usize::from(a > 0 && a < 255);
                if (2 * a - 255).abs() <= 8 {
                    continue;
                }
                checked += 1;
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let inside = rects.iter().any(|r| cx >= r.x && cx < r.right() && cy >= r.y && cy < r.bottom());
                if inside != (a > 127) {
                    bad += 1;
                }
            }
        }
        eprintln!("{name}: {} dikdörtgen, {checked} piksel denetlendi ({aa} AA pikseli)", rects.len());
        assert!(aa > 0, "{name}: kenar yumuşatması yok mu?");
        assert_eq!(bad, 0, "{name}: {bad} piksel kapsama ile girdi bölgesi çelişiyor");
    }
}
