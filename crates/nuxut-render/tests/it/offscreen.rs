//! Başsız (surfaceless EGL) GL testi: shader'lar derlenir, örnek bir sahne çizilir ve
//! pikseller okunarak doğrulanır. EGL/GLES yoksa test atlanır.
//! `NUXUT_DUMP=<dosya.ppm>` verilirse çıktı görüntüsü yazılır.

use khronos_egl as egl;
use nuxut_core::{Color, Corners, Point, Rect};
use nuxut_render::glow::{self, HasContext};
use nuxut_render::{
    BlobField, BlobShape, Border, DamageHistory, FrameShape, FrameTarget, ImageData, ImageFit, ImageStyle, PxRect,
    RectStyle, Renderer, Repaint, ShadowStyle, TextStyle, TextSystem, TextureOptions, UPLOAD_BUDGET_PER_FRAME,
};

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

#[test]
fn renders_scene() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (640u32, 400u32);
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    let gl = renderer.gl();
    // SAFETY: context güncel; FBO ve doku bu testte yaratılır.
    let (fbo, tex) = unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
        assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
        (fbo, tex)
    };
    let _ = gl;

    let mut text = TextSystem::new();
    let label = text.layout("Nuxut — puding 🍮", &TextStyle::new("sans-serif", 16.0, 500), None);
    let surface = Color::from_hex("#12141a").unwrap();
    let field = BlobField {
        frame: Some(FrameShape {
            outer: Rect::new(0.0, 0.0, 320.0, 200.0),
            inner: Rect::new(6.0, 24.0, 308.0, 170.0),
            inner_radius: 14.0,
        }),
        shapes: vec![BlobShape {
            rect: Rect::new(110.0, 24.0, 120.0, 70.0),
            radii: Corners::new(0.0, 0.0, 16.0, 16.0),
            deform: [0.93, 0.0, 0.0, 1.08],
            deform_anchor: Point::new(0.5, 0.0),
        }],
        smoothing: 14.0,
        corner_radius: 16.0,
        fill: surface,
        shadow: Some(ShadowStyle {
            color: Color::BLACK.with_alpha(0.5),
            blur: 16.0,
            offset: Point::new(0.0, 2.0),
            spread: 0.0,
        }),
    };
    renderer.render(&mut text, (w, h), 2.0, |p| {
        // Arka plan: blob'un ve gölgenin görülebilmesi için açık zemin.
        p.rect(Rect::new(0.0, 0.0, 320.0, 200.0), &RectStyle::filled(Color::from_hex("#6d7a8c").unwrap(), Corners::ZERO));
        p.blob(&field);
        let card = Rect::new(24.0, 120.0, 120.0, 56.0);
        p.shadow(card, Corners::uniform(14.0), &ShadowStyle {
            color: Color::BLACK.with_alpha(0.42),
            blur: 16.0,
            offset: Point::new(0.0, 2.0),
            spread: 0.0,
        });
        p.rect(card, &RectStyle {
            fill: Color::from_hex("#1a1d25").unwrap(),
            radii: Corners::uniform(14.0),
            border: Some(Border { width: 1.0, color: Color::from_hex("#9ec5ff").unwrap() }),
        });
        p.push_clip(Rect::new(24.0, 120.0, 120.0, 56.0));
        p.text(&label, Point::new(34.0, 138.0), Color::from_hex("#e4e6ee").unwrap());
        p.pop_clip();
        p.text(&label, Point::new(170.0, 140.0), Color::from_hex("#e4e6ee").unwrap());
    });

    let gl = renderer.gl();
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    // SAFETY: tampon boyutu w*h*4 ile tam; FBO bağlı.
    unsafe {
        gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut pixels)));
        assert_eq!(gl.get_error(), glow::NO_ERROR);
    }
    // GL satırları alttan başlar; y aşağı çevir.
    let at = |x: u32, y: u32| {
        let i = (((h - 1 - y) * w + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    };
    // Çerçevenin sol kenarı: yüzey rengi.
    assert_eq!(&at(4, 200)[..3], &[0x12, 0x14, 0x1a]);
    // Ekranın ortası (iç alan, şekillerden uzak): arka plan.
    let bg = at(560, 300);
    assert!(bg[0] > 0x50, "iç alan boyanmamalı: {bg:?}");
    // Panelin içi (bar'dan sarkan şekil): yüzey rengi.
    assert_eq!(&at(340, 150)[..3], &[0x12, 0x14, 0x1a]);
    // Kartın içi.
    assert_eq!(&at(100, 340)[..3], &[0x1a, 0x1d, 0x25]);

    // Hasar takibi: ikinci karede her yer kırmızıya boyanır ama yalnızca küçük bir bölge
    // hasarlı bildirilir; tampon yaşı 1 → bölge dışı önceki kareden kalmalı.
    let mut history = DamageHistory::new();
    let target = FrameTarget { size_px: (w, h), scale: 2.0, buffer_age: 0 };
    let red = Color::from_hex("#ff0000").unwrap();
    let full = Rect::new(0.0, 0.0, 320.0, 200.0);
    renderer.render_frame(&mut text, &mut history, target, |p| {
        p.rect(full, &RectStyle::filled(Color::from_hex("#00ff00").unwrap(), Corners::ZERO));
    });
    let repaint = renderer.render_frame(&mut text, &mut history, FrameTarget { buffer_age: 1, ..target }, |p| {
        p.damage(Rect::new(10.0, 10.0, 20.0, 20.0));
        p.rect(full, &RectStyle::filled(red, Corners::ZERO));
    });
    assert_eq!(repaint, Repaint::rect(PxRect::new(19, 19, 42, 42)));
    let gl = renderer.gl();
    let mut px = [0u8; 4];
    let read = |x: i32, y: i32, px: &mut [u8; 4]| {
        // SAFETY: 1x1 okuma, tampon 4 bayt.
        unsafe {
            gl.read_pixels(x, h as i32 - 1 - y, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(px)));
        }
    };
    read(40, 40, &mut px);
    assert_eq!(px, [255, 0, 0, 255], "hasarlı bölge yeniden çizilmeli");
    read(200, 200, &mut px);
    assert_eq!(px, [0, 255, 0, 255], "hasar dışı önceki kareden kalmalı");

    if let Ok(path) = std::env::var("NUXUT_DUMP") {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                out.extend_from_slice(&at(x, y)[..3]);
            }
        }
        std::fs::write(path, out).unwrap();
    }
    // SAFETY: nesneler bu testte yaratıldı.
    unsafe {
        gl.delete_framebuffer(fbo);
        gl.delete_texture(tex);
    }
}

/// Koyu zeminde açık metin: gamma düzeltmesi kapsamayı artırmalı, koyu metni değiştirmemeli.
/// `NUXUT_DUMP_TEXT=<dosya.ppm>`: üstte düzeltmesiz, altta düzeltmeli.
#[test]
fn text_gamma_thickens_light_text() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (520u32, 220u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let small = text.layout("Hızlı ayarlar · Bluetooth kapalı · 13px", &TextStyle::new("sans-serif", 13.0, 500), None);
    let dark_bg = Color::from_hex("#12141a").unwrap();
    let light_bg = Color::from_hex("#e4e6ee").unwrap();
    let mut shots = Vec::new();
    for gamma in [1.0, nuxut_render::DEFAULT_TEXT_GAMMA] {
        renderer.set_text_gamma(gamma);
        renderer.render(&mut text, (w, h / 2), 1.0, |p| {
            p.rect(Rect::new(0.0, 0.0, w as f32, 55.0), &RectStyle::filled(dark_bg, Corners::ZERO));
            p.rect(Rect::new(0.0, 55.0, w as f32, 55.0), &RectStyle::filled(light_bg, Corners::ZERO));
            p.text(&small, Point::new(12.0, 18.0), Color::from_hex("#e4e6ee").unwrap());
            p.text(&small, Point::new(12.0, 73.0), Color::from_hex("#12141a").unwrap());
        });
        let mut px = vec![0u8; (w * h / 2 * 4) as usize];
        let gl = renderer.gl();
        // SAFETY: tampon boyutu tam.
        unsafe {
            gl.read_pixels(0, 0, w as i32, (h / 2) as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        shots.push(px);
    }
    // GL satırları alttan: üst yarı (koyu zemin) = satır 55..110.
    let ink = |px: &[u8], dark: bool| -> u64 {
        let rows: Vec<u32> = if dark { (55..110).collect() } else { (0..55).collect() };
        rows.iter()
            .flat_map(|&y| (0..w).map(move |x| ((y * w + x) * 4) as usize))
            .map(|i| if dark { u64::from(px[i]) - 0x12 } else { 0xe4 - u64::from(px[i]).min(0xe4) })
            .sum()
    };
    let (before, after) = (ink(&shots[0], true), ink(&shots[1], true));
    eprintln!("açık metin mürekkebi: {before} → {after}");
    assert!(after > before + before / 20, "açık metin kalınlaşmalı: {before} → {after}");
    assert_eq!(ink(&shots[0], false), ink(&shots[1], false), "koyu metin değişmemeli");

    if let Ok(path) = std::env::var("NUXUT_DUMP_TEXT") {
        let hh = h / 2;
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for shot in &shots {
            for y in 0..hh {
                for x in 0..w {
                    let i = (((hh - 1 - y) * w + x) * 4) as usize;
                    out.extend_from_slice(&shot[i..i + 3]);
                }
            }
        }
        std::fs::write(path, out).unwrap();
    }
}

/// Köşe renkli gradyan: yatay kırmızı → mavi, yuvarlatılmış köşe dışı boş.
#[test]
fn gradient_rect_interpolates() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (200u32, 100u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let (red, blue) = (Color::from_hex("#ff0000").unwrap(), Color::from_hex("#0000ff").unwrap());
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.gradient_rect(Rect::new(0.0, 0.0, 200.0, 100.0), Corners::uniform(30.0), [red, blue, blue, red]);
    });
    let gl = renderer.gl();
    let read = |x: i32, y: i32| {
        let mut px = [0u8; 4];
        // SAFETY: 1x1 okuma.
        unsafe {
            gl.read_pixels(x, h as i32 - 1 - y, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        px
    };
    let left = read(5, 50);
    let mid = read(100, 50);
    let right = read(194, 50);
    assert!(left[0] > 240 && left[2] < 15, "{left:?}");
    assert!(right[2] > 240 && right[0] < 15, "{right:?}");
    assert!((i32::from(mid[0]) - 128).abs() < 8 && (i32::from(mid[2]) - 128).abs() < 8, "{mid:?}");
    assert_eq!(read(1, 1)[3], 0, "yuvarlatılmış köşe dışı boş olmalı");
}

/// Değişken font FILL ekseni: Material Symbols "settings" dolu çizildiğinde daha çok mürekkep.
/// Font kurulu değilse atlanır. `NUXUT_DUMP_ICON=<dosya.ppm>`: solda FILL 0, sağda 1.
#[test]
fn icon_fill_axis() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (128u32, 64u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let style = TextStyle::new("Material Symbols Rounded", 48.0, 400);
    let outline = text.layout("settings", &style, None);
    let filled = text.layout("settings", &style.clone().with_fill(1.0), None);
    if outline.size().width > 60.0 {
        eprintln!("Material Symbols kurulu değil (ligatür yok); test atlandı");
        return;
    }
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.text(&outline, Point::new(8.0, 0.0), Color::WHITE);
        p.text(&filled, Point::new(72.0, 0.0), Color::WHITE);
    });
    let gl = renderer.gl();
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: tampon boyutu tam.
    unsafe {
        gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
    }
    let ink = |x0: u32, x1: u32| -> u64 {
        (0..h).flat_map(|y| (x0..x1).map(move |x| ((y * w + x) * 4) as usize)).map(|i| u64::from(px[i + 3])).sum()
    };
    let (a, b) = (ink(0, 64), ink(64, 128));
    eprintln!("ikon mürekkebi: FILL 0 = {a}, FILL 1 = {b}");
    assert!(a > 0, "ikon çizilmeli");
    assert!(b > a * 3 / 2, "dolu ikon belirgin şekilde daha fazla mürekkep içermeli: {a} → {b}");
    if let Ok(path) = std::env::var("NUXUT_DUMP_ICON") {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let i = (((h - 1 - y) * w + x) * 4) as usize;
                out.extend_from_slice(&px[i..i + 3]);
            }
        }
        std::fs::write(path, out).unwrap();
    }
}

/// Zengin metin görsel kontrolü: kalın/italik/altı çizili/üstü çizili/renkli bağlantı.
/// `NUXUT_DUMP_RICH=<dosya.ppm>` ile görüntü yazılır; testin kendisi çizimin hatasız olduğunu
/// ve bağlantı renginin piksellere geçtiğini doğrular.
#[test]
fn rich_text_renders() {
    use nuxut_render::SpanStyle;
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (520u32, 60u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let primary = Color::from_hex("#9ec5ff").unwrap();
    let n = SpanStyle::default();
    let layout = text.layout_rich(
        &[
            ("Düz ", n),
            ("kalın", SpanStyle { bold: true, ..n }),
            (" ", n),
            ("italik", SpanStyle { italic: true, ..n }),
            (" ", n),
            ("çizili", SpanStyle { strikethrough: true, ..n }),
            (" ve ", n),
            ("bağlantı", SpanStyle { underline: true, color: Some(primary), link: Some(1), ..n }),
        ],
        &TextStyle::new("sans-serif", 16.0, 400),
        None,
    );
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.rect(Rect::new(0.0, 0.0, w as f32, h as f32), &RectStyle::filled(Color::from_hex("#12141a").unwrap(), Corners::ZERO));
        p.text(&layout, Point::new(12.0, 16.0), Color::from_hex("#e4e6ee").unwrap());
    });
    let gl = renderer.gl();
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: tampon boyutu tam.
    unsafe {
        gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
    }
    // Bağlantı rengi (mavimsi: b > r + 30) piksellere geçmiş olmalı.
    let bluish = px.chunks(4).filter(|c| i32::from(c[2]) > i32::from(c[0]) + 30).count();
    assert!(bluish > 20, "bağlantı rengi görünmeli ({bluish})");
    if let Ok(path) = std::env::var("NUXUT_DUMP_RICH") {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let i = (((h - 1 - y) * w + x) * 4) as usize;
                out.extend_from_slice(&px[i..i + 3]);
            }
        }
        std::fs::write(path, out).unwrap();
    }
}

/// Çizgi ve yay: kapsül/yay SDF kapsaması, yuvarlak uçlar, yay açısı sınırı.
/// `NUXUT_DUMP_STROKE=<dosya.ppm>` ile görüntü yazılır.
#[test]
fn strokes_and_arcs() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (240u32, 120u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let c = Color::from_hex("#9ec5ff").unwrap();
    let curve: Vec<Point> = (0..=40)
        .map(|i| {
            let t = i as f32 / 40.0;
            Point::new(130.0 + t * 100.0, 100.0 - nuxut_motion_like_ease(t) * 80.0)
        })
        .collect();
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.line(Point::new(10.0, 20.0), Point::new(110.0, 20.0), 4.0, c);
        p.polyline(&curve, 3.0, c);
        // Saat 12'den başlayıp saat yönünde çeyrek yay (merkez 60,80, r 25).
        p.arc(Point::new(60.0, 80.0), 25.0, -std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2, 4.0, c);
    });
    let gl = renderer.gl();
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: tampon boyutu tam.
    unsafe {
        gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
    }
    let alpha = |x: u32, y: u32| px[(((h - 1 - y) * w + x) * 4 + 3) as usize];
    // Doğrunun ortası dolu, 4 px ötesi boş; yuvarlak uç: uçtan 1 px ötesi kısmen dolu.
    assert_eq!(alpha(60, 20), 255);
    assert_eq!(alpha(60, 25), 0);
    assert!(alpha(111, 20) > 0, "yuvarlak uç");
    assert_eq!(alpha(116, 20), 0);
    // Yay: saat 12 (60, 55) ve saat 3 (85, 80) dolu; saat 6 (60, 105) ve saat 9 (35, 80) boş.
    assert_eq!(alpha(60, 55), 255, "yay başı");
    assert_eq!(alpha(85, 80), 255, "yay sonu");
    assert_eq!(alpha(60, 105), 0, "yay dışı");
    assert_eq!(alpha(35, 80), 0, "yay dışı");
    // Çeyreğin ortası (45°): merkezden r uzaklıkta dolu, merkez boş.
    let d = 25.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert_eq!(alpha((60.0 + d).round() as u32, (80.0 - d).round() as u32), 255);
    assert_eq!(alpha(60, 80), 0);
    if let Ok(path) = std::env::var("NUXUT_DUMP_STROKE") {
        let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
        for y in 0..h {
            for x in 0..w {
                let i = (((h - 1 - y) * w + x) * 4) as usize;
                out.extend_from_slice(&px[i..i + 3]);
            }
        }
        std::fs::write(path, out).unwrap();
    }
}

/// Eğri altı alan (`Painter::area`): yarı saydam renkte sütun ek yerlerinde çift karışım ya da
/// boşluk yok, eğik üst kenar ve taban doğru yerde, `fade` tabana doğru söndürür, kesirli
/// ölçekte de ek yeri yok.
#[test]
fn area_under_polyline() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    // Tek FBO (en büyük ölçeğe göre); her ölçek sol-alt köşesindeki w×h bölgeye çizer.
    let mut renderer = Renderer::new(gl).unwrap();
    let rgl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = rgl.create_texture().unwrap();
        rgl.bind_texture(glow::TEXTURE_2D, Some(tex));
        rgl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, 300, 150);
        let fbo = rgl.create_framebuffer().unwrap();
        rgl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        rgl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    for scale in [1.0f32, 1.5] {
        let (w, h) = ((200.0 * scale) as u32, (100.0 * scale) as u32);
        let mut text = TextSystem::new();
        let c = Color::rgba(1.0, 1.0, 1.0, 0.5);
        // Sol yarı: düz tepe (y 20), kesirli x adımlarıyla 37 sütun; sağ yarı: 45° eğik iniş + sönen.
        let flat: Vec<Point> = (0..=37).map(|i| Point::new(2.0 + i as f32 * 2.6, 20.0)).collect();
        let slope = [Point::new(110.0, 20.0), Point::new(150.0, 60.0)];
        renderer.render(&mut text, (w, h), scale, |p| {
            p.area(&flat, 90.0, c, 1.0);
            p.area(&slope, 90.0, c, 0.0);
            // Sonlu olmayan nokta parçayı atlar; panik yok.
            p.area(&[Point::new(160.0, 10.0), Point::new(f32::NAN, 10.0), Point::new(190.0, 10.0)], 90.0, c, 1.0);
        });
        let rgl = renderer.gl();
        let mut px = vec![0u8; (w * h * 4) as usize];
        // SAFETY: tampon boyutu tam.
        unsafe {
            rgl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        let alpha = |x: f32, y: f32| {
            let (x, y) = ((x * scale) as u32, (y * scale) as u32);
            px[(((h - 1 - y) * w + x) * 4 + 3) as usize]
        };
        // Düz kısım: her sütun sınırında aynı opaklık (≈ 128; ek yerinde çift karışım 192 olurdu, boşluk 0).
        for xi in 3..96 {
            let a = alpha(xi as f32 + 0.25, 50.0);
            assert!((126..=130).contains(&a), "ölçek {scale}, x {xi}: {a}");
        }
        assert_eq!(alpha(50.0, 15.0), 0, "tepenin üstü boş");
        assert_eq!(alpha(50.0, 95.0), 0, "tabanın altı boş");
        // Eğik kenar: (130, 40) kenarın üstünde; kenarın altı dolu, üstü boş.
        assert!(alpha(130.0, 46.0) > 0, "kenarın altı dolu");
        assert_eq!(alpha(130.0, 34.0), 0, "kenarın üstü boş");
        // fade = 0: tepede güçlü, tabana yakın neredeyse saydam.
        assert!(alpha(112.0, 24.0) > 100);
        assert!(alpha(112.0, 88.0) < 10, "tabana doğru söner");
        // NaN noktalı parça çizilmedi.
        assert_eq!(alpha(170.0, 50.0), 0);
    }
}


fn nuxut_motion_like_ease(t: f32) -> f32 {
    // Basit ease-out (test için; motion crate'ine bağımlılık eklememek için).
    1.0 - (1.0 - t).powi(3)
}

/// Görsel dokusu: yerleşim (fill/cover/contain), köşe kırpma, opaklık, güncelleme ve
/// tutamak düşünce serbest bırakma.
#[test]
fn image_textures() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (200u32, 100u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let read = |r: &Renderer, x: i32, y: i32| {
        let mut px = [0u8; 4];
        // SAFETY: 1x1 okuma.
        unsafe {
            r.gl().read_pixels(x, h as i32 - 1 - y, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        px
    };
    // 2x1: sol kırmızı, sağ mavi.
    let halves = ImageData { width: 2, height: 1, pixels: vec![255, 0, 0, 255, 0, 0, 255, 255].into() };
    let mut text = TextSystem::new();
    let full = Rect::new(0.0, 0.0, 200.0, 100.0);

    // Fill + köşe yarıçapı: sol kırmızı, sağ mavi, köşe dışı boş.
    let mut tex = None;
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let t = p.upload_texture(halves.clone(), TextureOptions::default());
        p.image(full, &t, &ImageStyle::new(ImageFit::Fill).with_radii(Corners::uniform(30.0)));
        tex = Some(t);
    });
    let tex = tex.unwrap();
    assert_eq!(renderer.texture_stats().0, 1);
    let (l, r) = (read(&renderer, 20, 50), read(&renderer, 180, 50));
    assert!(l[0] > 240 && l[2] < 15 && l[3] == 255, "{l:?}");
    assert!(r[2] > 240 && r[0] < 15, "{r:?}");
    assert_eq!(read(&renderer, 1, 1)[3], 0, "köşe dışı boş olmalı");

    // Contain: 2:1 doku 200x100'e tam sığar; 100x100 hedefte ortada 100x50 bant.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.image(Rect::new(0.0, 0.0, 100.0, 100.0), &tex, &ImageStyle::new(ImageFit::Contain));
    });
    assert_eq!(read(&renderer, 50, 10)[3], 0, "letterbox üstü boş");
    assert_eq!(read(&renderer, 20, 50)[3], 255);
    // Cover: 100x100 hedefte ortadan kırpar → yalnızca kırmızı/mavi sınırı ortada.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.image(Rect::new(0.0, 0.0, 100.0, 100.0), &tex, &ImageStyle::new(ImageFit::Cover));
    });
    assert_eq!(read(&renderer, 50, 5)[3], 255, "cover kutuyu doldurur");
    assert_eq!(read(&renderer, 150, 50)[3], 0, "kutu dışına taşmaz");

    // Opaklık: premultiplied çıktı yarıya iner.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.image(full, &tex, &ImageStyle::new(ImageFit::Fill).with_opacity(0.5));
    });
    let half = read(&renderer, 20, 50);
    assert!((i32::from(half[3]) - 128).abs() <= 2 && (i32::from(half[0]) - 128).abs() <= 2, "{half:?}");

    // Güncelleme: aynı boyutta yeşil.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.update_texture(&tex, ImageData { width: 2, height: 1, pixels: vec![0, 255, 0, 255, 0, 255, 0, 255].into() });
        p.image(full, &tex, &ImageStyle::new(ImageFit::Fill));
    });
    let g = read(&renderer, 20, 50);
    assert!(g[1] > 240 && g[0] < 15, "{g:?}");

    // Geçersiz veri yüklenmez, çizim atlanır (panic yok).
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let bad = p.upload_texture(ImageData { width: 4, height: 4, pixels: vec![0; 3].into() }, TextureOptions::default());
        p.image(full, &bad, &ImageStyle::default());
    });
    assert_eq!(read(&renderer, 100, 50)[3], 0);

    // Son tutamak düşünce bir sonraki karede doku silinir.
    drop(tex);
    renderer.render(&mut text, (w, h), 1.0, |_| {});
    assert_eq!(renderer.texture_stats().0, 0);

    // Mipmap'li büyük doku küçük çizilir.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let big = ImageData { width: 256, height: 256, pixels: vec![200; 256 * 256 * 4].into() };
        let t = p.upload_texture(big, TextureOptions { mipmaps: true });
        p.image(Rect::new(0.0, 0.0, 16.0, 16.0), &t, &ImageStyle::default());
    });
    assert_eq!(read(&renderer, 8, 8), [200, 200, 200, 200]);

    // Boyama: doku yalnızca alfa maskesi; çıktı = renk × alfa.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let mask = ImageData { width: 1, height: 1, pixels: vec![128, 128, 128, 128].into() };
        let t = p.upload_texture(mask, TextureOptions::default());
        p.image(full, &t, &ImageStyle::new(ImageFit::Fill).with_tint(Color::from_hex("#ff0000").unwrap()));
    });
    let tinted = read(&renderer, 100, 50);
    assert!((i32::from(tinted[0]) - 128).abs() <= 2 && tinted[1] == 0 && tinted[2] == 0, "{tinted:?}");
    assert!((i32::from(tinted[3]) - 128).abs() <= 2, "{tinted:?}");

    // Kare bütçesi: bütçeyi aşan ikinci yükleme sonraki kareye kalır.
    let side = 4096u32;
    let big = || ImageData { width: side, height: 3000, pixels: vec![255; (side * 3000 * 4) as usize].into() };
    assert!(2 * (side * 3000 * 4) as usize > UPLOAD_BUDGET_PER_FRAME);
    let mut handles = Vec::new();
    renderer.render(&mut text, (w, h), 1.0, |p| {
        handles.push(p.upload_texture(big(), TextureOptions::default()));
        handles.push(p.upload_texture(big(), TextureOptions::default()));
    });
    assert!(handles[0].is_ready() && !handles[1].is_ready());
    assert!(renderer.has_pending_uploads());
    renderer.render(&mut text, (w, h), 1.0, |_| {});
    assert!(handles[1].is_ready() && !renderer.has_pending_uploads());
}

/// Dual Kawase bulanıklık: keskin siyah/beyaz kenar yumuşak bir geçişe dönüşür, uzak
/// bölgeler korunur, sonuç yeniden kullanılabilir ve sonraki çizimler bozulmaz.
#[test]
fn blur_texture_smooths_edges() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (256u32, 64u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let read_row = |r: &Renderer, y: i32| {
        let mut px = vec![0u8; (w * 4) as usize];
        // SAFETY: tek satır okuma, tampon tam.
        unsafe {
            r.gl().read_pixels(0, h as i32 - 1 - y, w as i32, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        px.chunks(4).map(|c| c[0]).collect::<Vec<u8>>()
    };
    // Sol yarı siyah, sağ yarı beyaz (opak).
    let pixels: Vec<u8> = (0..h)
        .flat_map(|_| (0..w).flat_map(|x| if x < w / 2 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }))
        .collect();
    let mut text = TextSystem::new();
    let full = Rect::new(0.0, 0.0, w as f32, h as f32);
    let mut blurred = None;
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let src = p.upload_texture(ImageData { width: w, height: h, pixels: pixels.into() }, TextureOptions::default());
        let b = p.blur_texture(&src, 24.0);
        p.image(full, &b, &ImageStyle::new(ImageFit::Fill));
        blurred = Some(b);
    });
    let blurred = blurred.unwrap();
    assert!(blurred.is_ready(), "kaynak aynı karede yüklendiyse bulanıklık da hazır olmalı");
    assert_eq!(blurred.size(), (w, h));
    assert!(!renderer.has_pending_uploads());
    let row = read_row(&renderer, 32);
    // Uzak bölgeler korunur.
    assert!(row[4] < 20, "sol uç siyah kalmalı: {}", row[4]);
    assert!(row[(w - 5) as usize] > 235, "sağ uç beyaz kalmalı: {}", row[(w - 5) as usize]);
    // Kenar yumuşar: tam ortada ara ton, kenarın iki yanında da kademeli geçiş.
    let mid = row[(w / 2) as usize];
    assert!((60..200).contains(&mid), "kenar ortası ara ton olmalı: {mid}");
    let ramp = &row[(w / 2 - 16) as usize..(w / 2 + 16) as usize];
    assert!(ramp.windows(2).all(|p| p[1] + 2 >= p[0]), "geçiş monoton olmalı: {ramp:?}");
    assert!(ramp[0] > 5 && ramp[31] < 250, "geçiş geniş olmalı: {} … {}", ramp[0], ramp[31]);

    // Sonuç sonraki karede yeniden kullanılır (yeniden hesaplanmaz); başka çizimler normal.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        p.image(full, &blurred, &ImageStyle::new(ImageFit::Fill));
        p.rect(Rect::new(0.0, 0.0, 8.0, 8.0), &RectStyle::filled(Color::from_hex("#00ff00").unwrap(), Corners::ZERO));
    });
    let row0 = read_row(&renderer, 2);
    assert_eq!(row0[2], 0, "yeşil kare kırmızı kanalda 0");
    let row = read_row(&renderer, 32);
    assert_eq!(row[(w / 2) as usize], mid, "önbellekli sonuç aynı");
}

/// Kaynağın güncellemesi kare bütçesine takılırsa bulanıklık eski içerikten üretilmez: sonuç
/// güncelleme yüklenene dek hazır değildir, sonra yeni içeriği gösterir.
#[test]
fn blur_waits_for_deferred_source_update() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (32u32, 8u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let solid = |v: u8| ImageData { width: w, height: h, pixels: (0..w * h).flat_map(|_| [v, v, v, 255]).collect::<Vec<u8>>().into() };
    let mut text = TextSystem::new();
    let full = Rect::new(0.0, 0.0, w as f32, h as f32);

    // Kare 1: kaynak siyah yüklenir.
    let mut src = None;
    renderer.render(&mut text, (w, h), 1.0, |p| src = Some(p.upload_texture(solid(0), TextureOptions::default())));
    let src = src.unwrap();
    assert!(src.is_ready());

    // Kare 2: iki büyük yükleme bütçeyi doldurur; kaynağın beyaz güncellemesi sonraki kareye
    // kalır. Aynı karede istenen bulanıklık eski (siyah) içerikten üretilmemeli.
    let side = 4096u32;
    let big = || ImageData { width: side, height: 3000, pixels: vec![255; (side * 3000 * 4) as usize].into() };
    let mut keep = Vec::new();
    let mut blurred = None;
    renderer.render(&mut text, (w, h), 1.0, |p| {
        keep.push(p.upload_texture(big(), TextureOptions::default()));
        keep.push(p.upload_texture(big(), TextureOptions::default()));
        p.update_texture(&src, solid(255));
        blurred = Some(p.blur_texture(&src, 4.0));
    });
    let blurred = blurred.unwrap();
    assert!(renderer.has_pending_uploads());
    assert!(!blurred.is_ready(), "kaynağın güncellemesi beklerken bulanıklık hazır olmamalı");

    // Kare 3: güncelleme yüklenir, bulanıklık yeni (beyaz) içerikten üretilir.
    renderer.render(&mut text, (w, h), 1.0, |p| p.image(full, &blurred, &ImageStyle::new(ImageFit::Fill)));
    assert!(blurred.is_ready() && !renderer.has_pending_uploads());
    let mut px = [0u8; 4];
    // SAFETY: tek piksel okuma.
    unsafe {
        renderer.gl().read_pixels((w / 2) as i32, (h / 2) as i32, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
    }
    assert!(px[0] > 245, "bulanıklık güncel (beyaz) kaynaktan üretilmeli: {px:?}");
    drop(keep);
}

use nuxut_render::gl::{GlPass, PainterGlExt};

/// Dış GL geçişi: hedef dokuya çizer, durumu bozar; renderer durumu geri kurar ve doku
/// normal görsel gibi çizilir.
#[test]
fn gl_pass_renders_into_target() {
    struct Green;
    impl GlPass for Green {
        fn render(&self, gl: &glow::Context, fbo: u32, size: (u32, u32)) {
            assert!(fbo != 0);
            assert_eq!(size, (8, 4));
            // SAFETY: test; context güncel. Kasıtlı olarak durumu bozar.
            unsafe {
                gl.clear_color(0.0, 1.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.viewport(0, 0, 1, 1);
                gl.enable(glow::SCISSOR_TEST);
                gl.scissor(0, 0, 1, 1);
                gl.disable(glow::BLEND);
                gl.blend_equation(glow::FUNC_REVERSE_SUBTRACT);
                gl.bind_buffer(glow::ARRAY_BUFFER, None);
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 3);
                gl.pixel_store_i32(glow::PACK_ALIGNMENT, 8);
                // mpv'nin GL renderer'ı gibi: PBO'lar bağlı kalırsa sonraki doku yüklemesi
                // PBO'dan ofset olarak okunur.
                let pbo = gl.create_buffer().unwrap();
                gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(pbo));
                gl.buffer_data_size(glow::PIXEL_UNPACK_BUFFER, 4096, glow::STREAM_DRAW);
                let pack = gl.create_buffer().unwrap();
                gl.bind_buffer(glow::PIXEL_PACK_BUFFER, Some(pack));
                gl.buffer_data_size(glow::PIXEL_PACK_BUFFER, 4096, glow::STREAM_READ);
                // Sampler bağlı kalırsa mipmap'siz dokular eksik sayılır (siyah örnek).
                let sampler = gl.create_sampler().unwrap();
                gl.sampler_parameter_i32(sampler, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
                gl.bind_sampler(0, Some(sampler));
                gl.bind_sampler(1, Some(sampler));
                gl.enable(glow::CULL_FACE);
                gl.cull_face(glow::FRONT_AND_BACK);
                gl.enable(glow::POLYGON_OFFSET_FILL);
            }
        }
    }
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let (w, h) = (100u32, 50u32);
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let pass: std::sync::Arc<dyn GlPass> = std::sync::Arc::new(Green);
    let blue = Color::from_hex("#0000ff").unwrap();
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let t = p.render_target(8, 4);
        p.gl_pass(&t, pass.clone());
        p.image(Rect::new(0.0, 0.0, 50.0, 50.0), &t, &ImageStyle::new(ImageFit::Fill));
        // Geçişten sonra normal çizim de doğru yere ve karışımla düşmeli.
        p.rect(Rect::new(50.0, 0.0, 50.0, 50.0), &RectStyle::filled(blue.with_alpha(0.5), Corners::ZERO));
    });
    let gl = renderer.gl();
    let read = |x: i32, y: i32| {
        let mut px = [0u8; 4];
        // SAFETY: 1x1 okuma.
        unsafe { gl.read_pixels(x, h as i32 - 1 - y, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px))) };
        px
    };
    assert_eq!(read(25, 25), [0, 255, 0, 255], "hedef doku çizildi");
    let b = read(75, 25);
    assert!(b[2] > 120 && b[2] < 136 && b[3] > 120 && b[3] < 136, "karışım geri kuruldu: {b:?}");
    // Geçişin bozduğu UNPACK_ROW_LENGTH sıfırlanmazsa bu yükleme bozulurdu.
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let t = p.upload_texture(ImageData { width: 2, height: 2, pixels: [255, 0, 0, 255].repeat(4).into() }, TextureOptions::default());
        p.gl_pass(&t, pass.clone()); // hedef olmayan dokuya geçiş: atlanır, uyarı
        p.image(Rect::new(0.0, 0.0, 100.0, 50.0), &t, &ImageStyle::new(ImageFit::Fill));
    });
    let gl = renderer.gl();
    let mut px = [0u8; 4];
    // SAFETY: 1x1 okuma.
    unsafe { gl.read_pixels(90, 5, 1, 1, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px))) };
    assert_eq!(px, [255, 0, 0, 255]);
}

/// GL çöplüğü: başka thread'den gönderilen değer hemen değil, renderer'ın bir sonraki karesinin
/// başında, renderer'ın thread'inde (context güncel) düşer. Renderer hiç kare çizmeden düşerse
/// kendi `Drop`'unda boşaltır; renderer yoksa değer sızdırılır (context'siz GL çağrısı yok).
#[test]
fn gl_graveyard_drops_on_render_thread() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    struct Probe(Arc<AtomicUsize>, Arc<Mutex<Vec<std::thread::ThreadId>>>);
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
            self.1.lock().unwrap().push(std::thread::current().id());
        }
    }
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let mut renderer = Renderer::new(gl).unwrap();
    let mut text = TextSystem::new();
    let (count, threads) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
    let probe = || Box::new(Probe(count.clone(), threads.clone()));
    let mut grave = None;
    renderer.render(&mut text, (8, 8), 1.0, |p| grave = Some(p.gl_graveyard()));
    let grave = grave.unwrap();
    let g = grave.clone();
    let p = probe();
    std::thread::spawn(move || g.defer(p)).join().unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 0, "gönderen thread'de düşmemeli");
    renderer.render(&mut text, (8, 8), 1.0, |_| {});
    assert_eq!(count.load(Ordering::SeqCst), 1, "sonraki karenin başında düşmeli");
    // Kare çizilmeden kapanış: Renderer::drop boşaltır.
    grave.defer(probe());
    drop(renderer);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert!(threads.lock().unwrap().iter().all(|t| *t == std::thread::current().id()), "renderer'ın thread'inde");
    // Renderer yok: sızdırılır, düşürülmez.
    grave.defer(probe());
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

/// `pixels` (opak RGBA, `w`×`h`) `radius` ile bulanıklaştırılıp aynı boyutta çizilir; kırmızı
/// kanal üstten satır sırasıyla döner. EGL yoksa `None`.
fn blurred_red(w: u32, h: u32, pixels: Vec<u8>, radius: f32) -> Option<Vec<u8>> {
    let (_egl, gl) = headless()?;
    let mut renderer = Renderer::new(gl).unwrap();
    let gl = renderer.gl();
    // SAFETY: test FBO'su.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
    let mut text = TextSystem::new();
    let full = Rect::new(0.0, 0.0, w as f32, h as f32);
    renderer.render(&mut text, (w, h), 1.0, |p| {
        let src = p.upload_texture(ImageData { width: w, height: h, pixels: pixels.into() }, TextureOptions::default());
        let b = p.blur_texture(&src, radius);
        p.image(full, &b, &ImageStyle::new(ImageFit::Fill));
    });
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: tam kare okuma, tampon tam.
    unsafe {
        renderer.gl().read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
    }
    // GL satırları alttan üste.
    Some((0..h).rev().flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| px[((y * w + x) * 4) as usize]).collect())
}

fn gray_ppm(path: &str, w: u32, h: u32, red: &[u8]) {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    out.extend(red.iter().flat_map(|&v| [v, v, v]));
    std::fs::write(path, out).unwrap();
}

/// Bulanıklığın kenar koşulu ayna (t050): görüntü dışındaki örnekler içeriden yansıtılır.
/// Kenara kıstırmada (clamp-to-edge) görüntünün en dış satırı/sütunu her seviyede dışarıya
/// sonsuza dek uzuyor, köşe texeli dış çeyreğin tamamını dolduruyordu: kenardaki ince bir çizgi
/// ya da köşedeki küçük bir leke bulanık görüntüde geniş bir kenar bandı / köşe lekesi olarak
/// kalıyordu. Ayna kenarla ince içerik yalnızca ikiye katlanır, bulanıklıkta söner.
/// Boyut ikinin kuvveti değil (ara seviyelerde tek sayılar). `NUXUT_DUMP_BLUR=<önek>` ile
/// `<önek>-*.ppm` yazılır.
#[test]
fn blur_edges_mirror_instead_of_smearing() {
    let (w, h) = (330u32, 210u32);
    // Siyah zemin; üst kenarda 2 px beyaz çizgi, dört köşede 4×4 beyaz leke.
    let edge = |v: u32, m: u32| v < 4 || v >= m - 4;
    let pixels: Vec<u8> = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| if y < 2 || (edge(x, w) && edge(y, h)) { [255, 255, 255, 255] } else { [0, 0, 0, 255] })
        .collect();
    // Ölçüm (r = 16): clamp üst kenar 32, köşe 15; ayna 21, 5. Sınırlar ikisinin arasında.
    {
        let radius = 16.0f32;
        let Some(red) = blurred_red(w, h, pixels.clone(), radius) else {
            eprintln!("EGL surfaceless yok; test atlandı");
            return;
        };
        if let Ok(prefix) = std::env::var("NUXUT_DUMP_BLUR") {
            gray_ppm(&format!("{prefix}-edges-{radius}.ppm"), w, h, &red);
        }
        let at = |x: u32, y: u32| red[(y * w + x) as usize];
        let top = at(w / 2, 0);
        let corners = [at(0, h - 1), at(w - 1, h - 1)];
        assert!(top < 27, "r={radius}: kenar çizgisi dışarı uzatılmış (clamp): üst kenar {top}");
        assert!(corners.iter().all(|&c| c < 10), "r={radius}: köşe lekesi dışarı uzatılmış (clamp): {corners:?}");
    }
}

/// Dama tahtası: kareler yarıçapa göre ince olunca köşeye hizalı da kayık da olsa köşeler iç bölgeyle
/// aynı tona iner. Not: kareler yarıçap kadar iriyse ve köşeye hizalıysa köşe karesi her yansıtıcı
/// kenar koşulunda (ayna da clamp da; referans Gauss da) 2×2 kat büyük tek renkli blok olur ve köşe
/// daha az bulanık görünür — bu bulanıklığın değil görüntünün özelliği (t050 ölçümü).
#[test]
fn blur_checkerboard_corners_follow_interior() {
    let (w, h) = (330u32, 210u32);
    for shift in [0u32, 5] {
        let cell = 10;
        let pixels: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .flat_map(|(x, y)| if ((x + shift) / cell + (y + shift) / cell) % 2 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] })
            .collect();
        let Some(red) = blurred_red(w, h, pixels, 48.0) else {
            eprintln!("EGL surfaceless yok; test atlandı");
            return;
        };
        let mean = |x0: u32, y0: u32, n: u32| {
            let s: u32 = (y0..y0 + n).flat_map(|y| (x0..x0 + n).map(move |x| (x, y))).map(|(x, y)| u32::from(red[(y * w + x) as usize])).sum();
            s as f32 / (n * n) as f32
        };
        let center = mean(w / 2 - 30, h / 2 - 30, 60);
        let n = 6;
        let corners = [mean(0, 0, n), mean(w - n, 0, n), mean(0, h - n, n), mean(w - n, h - n, n)];
        assert!((110.0..145.0).contains(&center), "shift={shift}: iç bölge ara ton olmalı: {center}");
        for c in corners {
            assert!((c - center).abs() < 12.0, "shift={shift}: köşe lekesi — köşeler {corners:?}, iç {center}");
        }
    }
}

/// Takas zinciri benzetimi (EGL `buffer_age`): 3 tamponlu döngüde kısmi çizim, aynı son durumun
/// tam çiziminin piksel-piksel aynısını vermeli — artık (stale) piksel yok. Araya giren "hiçbir
/// şey değişmedi" kareleri swap etmez; geçmişe yazılırlarsa yaş hesabı kayar ve eski bir
/// tampondaki bölge yeniden çizilmeden kalır (gerileme bekçisi).
#[test]
fn partial_redraw_with_buffer_rotation_matches_full() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    const BUFFERS: usize = 3;
    let (w, h) = (320u32, 120u32);
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    let gl = renderer.gl();
    // SAFETY: context güncel; FBO'lar ve dokular bu testte yaratılır.
    let fbos: Vec<glow::Framebuffer> = (0..=BUFFERS)
        .map(|_| unsafe {
            let tex = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
            fbo
        })
        .collect();
    let read = |renderer: &Renderer, fbo: glow::Framebuffer| {
        let gl = renderer.gl();
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        // SAFETY: tampon w*h*4 bayt; FBO tam.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut pixels)));
        }
        pixels
    };

    let mut text = TextSystem::new();
    let style = TextStyle::new("sans-serif", 14.0, 500);
    let bg = Color::from_hex("#20242c").unwrap();
    let bar = Color::from_hex("#3a4150").unwrap();
    let fg = Color::from_hex("#e4e6ee").unwrap();
    let accent = Color::from_hex("#9ec5ff").unwrap();
    // Adımın durumu: gezinen kutu ve değişen metin (saat benzeri).
    let state = |step: u32| {
        let boxed = Rect::new(4.0 + (step * 37 % 250) as f32, 44.0, 30.0 + (step % 3) as f32 * 9.0, 28.0);
        let label_at = Point::new(4.0 + (step * 53 % 230) as f32, 86.0);
        (boxed, label_at, format!("{:02}:{:02}", step * 7 % 60, step))
    };
    let scene = |p: &mut nuxut_render::Painter, step: u32, label: &nuxut_render::TextLayout| {
        let (boxed, label_at, _) = state(step);
        p.rect(Rect::new(0.0, 0.0, w as f32, h as f32), &RectStyle::filled(bg, Corners::ZERO));
        p.rect(Rect::new(0.0, 0.0, w as f32, 36.0), &RectStyle::filled(bar, Corners::ZERO));
        p.rect(boxed, &RectStyle::filled(accent, Corners::uniform(8.0)));
        p.text(label, label_at, fg);
    };
    let damage_of = |step: u32, label: &nuxut_render::TextLayout| {
        let (boxed, at, _) = state(step);
        let s = label.size();
        [boxed, Rect::new(at.x, at.y, s.width, s.height).expand(2.0)]
    };

    let mut history = DamageHistory::new();
    // Tampon başına son swap numarası; `swaps` şimdiye dek yapılan swap sayısı.
    let mut presented: [Option<u32>; BUFFERS] = [None; BUFFERS];
    let (mut swaps, mut next, mut last_presented) = (0u32, 0usize, None);
    let mut prev: Option<[Rect; 2]> = None;
    let mut step = 0u32;
    for frame in 0..24u32 {
        // Her dördüncü kare: hiçbir şey değişmedi (boş hasar, swap yok).
        let idle = frame % 4 == 3;
        if !idle {
            step += 1;
        }
        let buf = next % BUFFERS;
        let age = presented[buf].map_or(0, |k| swaps + 1 - k);
        let label = text.layout(&state(step).2, &style, None);
        let now = damage_of(step, &label);
        // SAFETY: FBO bu testin; çizim bağlı framebuffer'a gider.
        unsafe { renderer.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbos[buf])) };
        let repaint = renderer.render_frame(&mut text, &mut history, FrameTarget { size_px: (w, h), scale: 1.0, buffer_age: age }, |p| {
            if idle {
                p.damage(Rect::ZERO);
            } else {
                for r in prev.iter().flatten().chain(&now) {
                    p.damage(*r);
                }
            }
            scene(p, step, &label);
        });
        if repaint.is_nothing() {
            assert!(idle, "değişen kare boş hasar döndürmemeli (kare {frame})");
            continue;
        }
        swaps += 1;
        presented[buf] = Some(swaps);
        last_presented = Some(buf);
        next += 1;
        prev = Some(now);
    }

    let shown = read(&renderer, fbos[last_presented.expect("en az bir swap")]);
    let label = text.layout(&state(step).2, &style, None);
    // SAFETY: başvuru FBO'su bu testin.
    unsafe { renderer.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbos[BUFFERS])) };
    renderer.render(&mut text, (w, h), 1.0, |p| scene(p, step, &label));
    let reference = read(&renderer, fbos[BUFFERS]);
    let differing = shown.chunks(4).zip(reference.chunks(4)).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "kısmi çizimde {differing} artık piksel (tam çizimle fark)");
}

/// Çoklu hasar bölgesi: aynı karede birbirinden uzak iki değişiklik (solda gölgeli blob paneli
/// büyüyüp küçülür, sağ uçta saat metni ve gölgeli kart değişir) ayrı parçalar olarak kalmalı
/// (tek kutuya şişmemeli) ve 3 tamponlu döngüde kısmi çizim tam çizimin piksel-piksel aynısı
/// olmalı — parça başına geçiş örtüşen pikseli iki kez harmanlamaz, taşan efektler (blob
/// filetosu, gölge) parçanın payıyla birlikte doğru. 1 ve 1,5 ölçek.
#[test]
fn multi_rect_damage_matches_full() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let mut renderer = Renderer::new(gl).expect("shader'lar derlenmeli");
    for scale in [1.0f32, 1.5] {
        multi_rect_case(&mut renderer, scale);
    }
}

fn multi_rect_case(renderer: &mut Renderer, scale: f32) {
    const BUFFERS: usize = 3;
    let (lw, lh) = (800.0f32, 160.0f32);
    let (w, h) = ((lw * scale).round() as u32, (lh * scale).round() as u32);
    let gl = renderer.gl();
    // SAFETY: context güncel; FBO'lar ve dokular bu testte yaratılır.
    let fbos: Vec<glow::Framebuffer> = (0..=BUFFERS)
        .map(|_| unsafe {
            let tex = gl.create_texture().unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, w as i32, h as i32);
            let fbo = gl.create_framebuffer().unwrap();
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
            assert_eq!(gl.check_framebuffer_status(glow::FRAMEBUFFER), glow::FRAMEBUFFER_COMPLETE);
            fbo
        })
        .collect();
    let read = |renderer: &Renderer, fbo: glow::Framebuffer| {
        let gl = renderer.gl();
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        // SAFETY: tampon w*h*4 bayt; FBO tam.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.read_pixels(0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut pixels)));
        }
        pixels
    };

    let mut text = TextSystem::new();
    let style = TextStyle::new("sans-serif", 14.0, 500);
    let bg = Color::from_hex("#6d7a8c").unwrap();
    let fg = Color::from_hex("#e4e6ee").unwrap();
    let card_fill = Color::from_hex("#1a1d25").unwrap();
    let shadow = ShadowStyle { color: Color::BLACK.with_alpha(0.45), blur: 10.0, offset: Point::new(0.0, 2.0), spread: 0.0 };
    // Bar benzeri çerçeve (üstte 32 px şerit) ve soldan sarkan panel.
    let field = |step: u32| BlobField {
        frame: Some(FrameShape { outer: Rect::new(0.0, 0.0, lw, lh), inner: Rect::new(4.0, 32.0, lw - 8.0, lh - 36.0), inner_radius: 10.0 }),
        shapes: vec![BlobShape::new_rect(Rect::new(40.0, 32.0, 120.0, 20.0 + (step % 4) as f32 * 14.0))],
        smoothing: 10.0,
        corner_radius: 12.0,
        fill: Color::from_hex("#12141a").unwrap(),
        shadow: Some(shadow),
    };
    let card = |step: u32| Rect::new(640.0, 90.0 + (step % 3) as f32 * 6.0, 120.0, 40.0);
    let label_at = Point::new(700.0, 8.0);
    let label_text = |step: u32| format!("{:02}:{:02}", step % 24, step * 7 % 60);
    let scene = |p: &mut nuxut_render::Painter, step: u32, label: &nuxut_render::TextLayout| {
        p.rect(Rect::new(0.0, 0.0, lw, lh), &RectStyle::filled(bg, Corners::ZERO));
        p.blob(&field(step));
        p.text(label, label_at, fg);
        p.shadow(card(step), Corners::uniform(10.0), &shadow);
        p.rect(card(step), &RectStyle::filled(card_fill, Corners::uniform(10.0)));
    };
    // Adımın hasarı: blob panelinin, saatin ve kartın (gölgesiyle) kapladığı yerler.
    let damage_of = |step: u32, label: &nuxut_render::TextLayout| {
        let f = field(step);
        let s = label.size();
        let sh = shadow.blur * 1.5 + 2.0;
        [f.damage_rect(&f.shapes[0]), Rect::new(label_at.x, label_at.y, s.width, s.height).expand(2.0), card(step).expand(sh)]
    };

    let mut history = DamageHistory::new();
    let mut presented: [Option<u32>; BUFFERS] = [None; BUFFERS];
    let (mut swaps, mut next, mut last_presented) = (0u32, 0usize, None);
    let mut prev: Option<[Rect; 3]> = None;
    let mut max_parts = 0;
    for step in 0..14u32 {
        let buf = next % BUFFERS;
        let age = presented[buf].map_or(0, |k| swaps + 1 - k);
        let label = text.layout(&label_text(step), &style, None);
        let now = damage_of(step, &label);
        // SAFETY: FBO bu testin; çizim bağlı framebuffer'a gider.
        unsafe { renderer.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbos[buf])) };
        let repaint = renderer.render_frame(&mut text, &mut history, FrameTarget { size_px: (w, h), scale, buffer_age: age }, |p| {
            for r in prev.iter().flatten().chain(&now) {
                p.damage(*r);
            }
            scene(p, step, &label);
        });
        if let Repaint::Partial(region) = repaint {
            // Sol panel ile sağ uçtaki saat/kart tek kutuda birleşmemeli.
            assert!(region.len() >= 2, "uzak bölgeler birleşti: {region:?}");
            assert!(region.area() * 4 < region.bounds().area() * 3, "parçalar sınır kutusundan küçük olmalı: {region:?}");
            max_parts = max_parts.max(region.len());
        }
        swaps += 1;
        presented[buf] = Some(swaps);
        last_presented = Some(buf);
        next += 1;
        prev = Some(now);
    }
    assert!(max_parts >= 2, "kısmi kare olmadı");

    let shown = read(renderer, fbos[last_presented.expect("en az bir swap")]);
    let label = text.layout(&label_text(13), &style, None);
    // SAFETY: başvuru FBO'su bu testin.
    unsafe { renderer.gl().bind_framebuffer(glow::FRAMEBUFFER, Some(fbos[BUFFERS])) };
    renderer.render(&mut text, (w, h), scale, |p| scene(p, 13, &label));
    let reference = read(renderer, fbos[BUFFERS]);
    let differing = shown.chunks(4).zip(reference.chunks(4)).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0, "ölçek {scale}: çoklu parça kısmi çizimde {differing} farklı piksel");
    // SAFETY: FBO'lar bu testin; varsayılan framebuffer'a dön.
    unsafe {
        let gl = renderer.gl();
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        for f in fbos {
            gl.delete_framebuffer(f);
        }
    }
}

/// Glyph atlas'ı sayfaları bağımsız: yalnız metin çizilirken renk sayfası hiç ayrılmaz, maske
/// büyüse de; ilk emoji renk sayfasını küçük ayırır ve maske boyutuna dokunmaz (perf 2026-10:
/// renk sayfası maskeyle birlikte 64 MB'a çıkıyordu).
#[test]
fn glyph_atlas_pages_grow_independently() {
    let Some((_egl, gl)) = headless() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let mut renderer = Renderer::new(gl).unwrap();
    let mut text = TextSystem::new();
    let (w, h) = (256, 256);
    assert_eq!(renderer.glyph_atlas_sizes().1, None, "başta renk sayfası yok");
    // Büyük boyutlu çok sayıda farklı glyph: 512'lik maske sayfası dolar ve büyür.
    let line: String = ('!'..='~').chain('À'..='ž').collect();
    let mut grew = false;
    for size in [40.0, 56.0, 72.0, 96.0] {
        let l = text.layout(&line, &TextStyle::new("sans-serif", size, 400), None);
        renderer.render(&mut text, (w, h), 1.0, |p| p.text(&l, Point::new(0.0, 0.0), Color::from_hex("#e4e6ee").unwrap()));
        grew |= renderer.glyph_atlas_sizes().0 > 512;
    }
    assert!(grew, "maske büyümeliydi: {:?}", renderer.glyph_atlas_sizes());
    assert_eq!(renderer.glyph_atlas_sizes().1, None, "yalnız metinde renk sayfası ayrılmaz");
    let mask = renderer.glyph_atlas_sizes().0;
    let emoji = text.layout("🍮", &TextStyle::new("sans-serif", 24.0, 400), None);
    renderer.render(&mut text, (w, h), 1.0, |p| p.text(&emoji, Point::new(0.0, 0.0), Color::WHITE));
    match renderer.glyph_atlas_sizes() {
        (m, Some(c)) => {
            assert_eq!(m, mask, "renk ayrımı maskeyi değiştirmez");
            assert!(c <= 256, "renk sayfası küçük başlar: {c}");
        }
        // Renkli emoji fontu yoksa glif maske olarak gelir: renk sayfası yine yok.
        (_, None) => eprintln!("renkli emoji fontu yok; renk sayfası denetimi atlandı"),
    }
}
