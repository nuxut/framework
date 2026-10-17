//! Duvar kağıdı bulanıklığı, başsız (surfaceless EGL) GL ile: bir kez üretilir ve saklanır,
//! yarıçap/ölçek değişince yeniden üretilir, çapraz geçişte iki katman da bulanıktır, keskin
//! kare hiç görünmez, kapatınca kopyalar bırakılır. EGL/GLES yoksa testler atlanır.

use std::path::Path;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use image::ImageEncoder;
use khronos_egl as egl;
use nuxut_core::{Point, Rect, Size};
use nuxut_image::{Wallpaper, WallpaperBackdrop, WallpaperDecoder, WallpaperDecoderOptions, WallpaperStatus};
use nuxut_motion::{CubicBezier, Curve, Motion};
use nuxut_render::glow::{self, HasContext};
use nuxut_render::{ImageFit, Renderer, TextSystem};

const PLATFORM_SURFACELESS_MESA: egl::Enum = 0x31DD;
const W: u32 = 256;
const H: u32 = 64;
const SHARP_BYTES: u64 = (W * H * 4) as u64;

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

/// Hedef FBO: `W`×`H` fiziksel piksel.
fn bind_target(gl: &glow::Context) {
    // SAFETY: test FBO'su; context güncel.
    unsafe {
        let tex = gl.create_texture().unwrap();
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_storage_2d(glow::TEXTURE_2D, 1, glow::RGBA8, W as i32, H as i32);
        let fbo = gl.create_framebuffer().unwrap();
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(tex), 0);
    }
}

/// Sol yarı siyah, sağ yarı `right` gri (opak) PNG.
fn write_png(path: &Path, right: u8) {
    let img = image::RgbaImage::from_fn(W, H, |x, _| {
        if x < W / 2 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([right, right, right, 255]) }
    });
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), W, H, image::ExtendedColorType::Rgba8).unwrap();
    std::fs::write(path, out).unwrap();
}

/// Kenarın solu yumuşamış mı (keskin kenarda siyah kalır).
fn is_soft(row: &[u8]) -> bool {
    row[(W / 2 - 3) as usize] > 8
}

struct Rig {
    _headless: Headless,
    renderer: Renderer,
    text: TextSystem,
    wp: Wallpaper,
    loaded: mpsc::Receiver<()>,
    dir: tempfile::TempDir,
}

impl Rig {
    fn new(motion: impl Into<Motion>) -> Option<Self> {
        let Some((headless, gl)) = headless() else {
            eprintln!("EGL surfaceless yok; test atlandı");
            return None;
        };
        let renderer = Renderer::new(gl).unwrap();
        bind_target(renderer.gl());
        let (tx, loaded) = mpsc::channel();
        let wp = Wallpaper::new(ImageFit::Fill, motion, WallpaperDecoder::in_process_unsandboxed(WallpaperDecoderOptions::default()), move || {
            let _ = tx.send(());
        });
        Some(Self { _headless: headless, renderer, text: TextSystem::new(), wp, loaded, dir: tempfile::tempdir().unwrap() })
    }

    fn load(&mut self, name: &str, right: u8) {
        let path = self.dir.path().join(name);
        write_png(&path, right);
        self.wp.set_source(&path, (W, H));
        self.loaded.recv_timeout(Duration::from_secs(5)).expect("yükleme bitmedi");
    }

    fn frame(&mut self, scale: f32, now: Instant) -> WallpaperStatus {
        let mut status = WallpaperStatus::default();
        let wp = &mut self.wp;
        let rect = Rect::from_origin_size(Point::ZERO, Size::new(W as f32 / scale, H as f32 / scale));
        self.renderer.render(&mut self.text, (W, H), scale, |p| status = wp.draw(p, rect, now));
        status
    }

    /// Yeniden çizim istendikçe kare çizer; her karede `check` ortadaki satırı görür.
    fn settle_checked(&mut self, scale: f32, now: Instant, mut check: impl FnMut(&[u8])) -> WallpaperStatus {
        for _ in 0..20 {
            let st = self.frame(scale, now);
            check(&self.middle_row());
            if !st.redraw {
                return st;
            }
        }
        panic!("duvar kağıdı durulmadı");
    }

    fn settle(&mut self, scale: f32, now: Instant) -> WallpaperStatus {
        self.settle_checked(scale, now, |_| {})
    }

    /// Ortadaki satırın kırmızı kanalı.
    fn middle_row(&self) -> Vec<u8> {
        let mut px = vec![0u8; (W * 4) as usize];
        // SAFETY: tek satır okuma, tampon tam; hedef FBO bağlı.
        unsafe {
            self.renderer.gl().read_pixels(
                0,
                (H / 2) as i32,
                W as i32,
                1,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut px)),
            );
        }
        px.chunks(4).map(|c| c[0]).collect()
    }
}

#[test]
fn blur_is_made_once_and_regenerated_on_change() {
    let Some(mut rig) = Rig::new(Motion::INSTANT) else { return };
    let now = Instant::now();
    rig.wp.set_blur(Some(12.0));
    rig.load("a.png", 255);
    // İlk görünen kare de bulanık (keskin kare hiç çizilmez).
    rig.settle_checked(1.0, now, |row| {
        assert!(row.iter().all(|&v| v == 0) || is_soft(row), "keskin kare çizildi: {:?}", &row[120..136]);
    });
    let row = rig.middle_row();
    assert!(is_soft(&row), "kenar bulanık: {:?}", &row[120..136]);
    assert!(row[2] < 10 && row[(W - 3) as usize] > 245, "uçlar korunur");
    assert_eq!(rig.wp.texture_bytes(), 2 * SHARP_BYTES, "bulanık kopya bütçeye sayılır");

    // Durulunca boşta iş yok: bulanıklık her karede yeniden istenseydi (sonuç GPU'da kare
    // sonunda hazır olur) her kare yeniden çizim isterdi.
    for _ in 0..3 {
        assert_eq!(rig.frame(1.0, now), WallpaperStatus::default(), "kare başına bulanıklık istendi");
    }

    // Yarıçap değişince yeniden üretilir.
    rig.wp.set_blur(Some(2.0));
    assert!(rig.frame(1.0, now).redraw, "yarıçap değişti, yeni kopya bekleniyor");
    rig.settle(1.0, now);
    let narrow = rig.middle_row();
    let i = (W / 2 - 12) as usize;
    assert!(narrow[i] < row[i], "küçük yarıçap daha dar yayılır: {} / {}", narrow[i], row[i]);

    // Ölçek değişince yeniden üretilir.
    assert!(rig.frame(2.0, now).redraw, "ölçek değişti, yeni kopya bekleniyor");
    assert_eq!(rig.settle(2.0, now), WallpaperStatus::default());

    // Kapatınca kopyalar bırakılır, keskin görsel çizilir.
    rig.wp.set_blur(None);
    assert!(!rig.frame(2.0, now).redraw);
    assert_eq!(rig.wp.texture_bytes(), SHARP_BYTES);
    let sharp = rig.middle_row();
    assert!(sharp[(W / 2 - 2) as usize] < 5 && sharp[(W / 2 + 1) as usize] > 250, "keskin kenar");
}

#[test]
fn crossfade_blurs_both_layers() {
    let Some(mut rig) = Rig::new(Curve::new(CubicBezier::LINEAR, 1.0)) else { return };
    let t0 = Instant::now();
    rig.wp.set_blur(Some(12.0));
    rig.load("a.png", 255);
    rig.settle(1.0, t0);
    rig.load("b.png", 128);
    // Yeni katman bulanık kopyası hazır olunca devreye girer (saat t0'da durduğu için geçiş
    // başında kalır); geçişin hiçbir karesi keskin değil.
    for _ in 0..10 {
        assert!(rig.frame(1.0, t0).redraw, "geçiş sürüyor");
        assert!(is_soft(&rig.middle_row()));
    }
    let mid = t0 + Duration::from_millis(500);
    rig.frame(1.0, mid);
    let row = rig.middle_row();
    assert!(is_soft(&row), "geçişte bulanık: {:?}", &row[120..136]);
    // Sağ uç iki katmanın karışımı (255 → 128).
    let right = row[(W - 3) as usize];
    assert!((140..245).contains(&right), "karışım: {right}");
    assert_eq!(rig.wp.texture_bytes(), 4 * SHARP_BYTES, "geçişte iki katman + iki kopya");

    let st = rig.settle(1.0, t0 + Duration::from_secs(2));
    assert_eq!(st, WallpaperStatus::default());
    assert_eq!(rig.wp.texture_bytes(), 2 * SHARP_BYTES, "eski katman ve kopyası bırakıldı");
    let row = rig.middle_row();
    assert!(is_soft(&row) && (120..136).contains(&row[(W - 3) as usize]));
}

#[test]
fn invalid_radius_disables_blur() {
    let mut wp = Wallpaper::new(ImageFit::Cover, Motion::INSTANT, WallpaperDecoder::in_process_unsandboxed(WallpaperDecoderOptions::default()), || {});
    for r in [0.0, -3.0, f32::NAN, f32::INFINITY] {
        wp.set_blur(Some(r));
        assert_eq!(wp.blur(), None, "{r}");
    }
    wp.set_blur(Some(8.0));
    assert_eq!(wp.blur(), Some(8.0));
    assert_eq!(wp.texture_bytes(), 0);
}

/// İki kareli GIF (50 ms): A = sol siyah/sağ beyaz, B = tamamen siyah.
fn write_gif(path: &Path) {
    let a = image::RgbaImage::from_fn(W, H, |x, _| if x < W / 2 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255; 4]) });
    let b = image::RgbaImage::from_pixel(W, H, image::Rgba([0, 0, 0, 255]));
    let delay = image::Delay::from_numer_denom_ms(50, 1);
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        enc.encode_frames([image::Frame::from_parts(a, 0, 0, delay), image::Frame::from_parts(b, 0, 0, delay)]).unwrap();
    }
    std::fs::write(path, out).unwrap();
}

/// ADR-0007 K5 / S2-a: animasyonlu duvar kağıdı ilk karede durağan gösterilir; zamanlayıcı
/// yok, `animated_source` işaretli. Bulanıklık açılıp kapanınca da kare değişmez.
#[test]
fn animated_source_shows_first_frame_still() {
    let Some(mut rig) = Rig::new(Motion::INSTANT) else { return };
    let t0 = Instant::now();
    let path = rig.dir.path().join("anim.gif");
    write_gif(&path);
    rig.wp.set_source(&path, (W, H));
    rig.loaded.recv_timeout(Duration::from_secs(5)).expect("yükleme bitmedi");
    assert_eq!(rig.settle(1.0, t0), WallpaperStatus::default(), "durağan: timer yok");
    assert!(rig.wp.animated_source());
    let first = rig.middle_row();
    let right = (W - 3) as usize;
    assert!(first[right] > 200 && first[2] < 50, "ilk kare (A): sol siyah, sağ beyaz");
    for ms in [25, 50, 75, 1000] {
        assert_eq!(rig.frame(1.0, t0 + Duration::from_millis(ms)), WallpaperStatus::default(), "{ms} ms");
        assert_eq!(rig.middle_row(), first, "{ms} ms: kare ilerledi");
    }
    rig.wp.set_blur(Some(12.0));
    rig.settle(1.0, t0);
    rig.wp.set_blur(None);
    rig.settle(1.0, t0 + Duration::from_secs(2));
    assert_eq!(rig.middle_row(), first);
    assert_eq!(rig.wp.texture_bytes(), SHARP_BYTES);
    // Durağan kaynak işaretsiz.
    rig.load("still.png", 255);
    rig.settle(1.0, t0);
    assert!(!rig.wp.animated_source());
}

#[test]
fn toggling_blur_crossfades_without_jump() {
    let Some(mut rig) = Rig::new(Curve::new(CubicBezier::LINEAR, 1.0)) else { return };
    let t0 = Instant::now();
    let probe = (W / 2 - 3) as usize;
    rig.load("a.png", 255);
    rig.settle(1.0, t0);
    assert!(rig.middle_row()[probe] < 3, "başta keskin");

    // Açılış: kopya hazırlanırken ve geçişin başında keskin kalır (tek karede atlama yok).
    rig.wp.set_blur(Some(12.0));
    for _ in 0..5 {
        assert!(rig.frame(1.0, t0).redraw);
        assert!(rig.middle_row()[probe] < 3, "geçiş başında bulanığa atladı");
    }
    rig.frame(1.0, t0 + Duration::from_millis(500));
    let half = rig.middle_row()[probe];
    assert_eq!(rig.settle(1.0, t0 + Duration::from_secs(2)), WallpaperStatus::default());
    let full = rig.middle_row()[probe];
    assert!(full > 20, "tam bulanık: {full}");
    assert!(half > full / 4 && half < full - full / 4, "yarı yolda karışım: {half} / {full}");
    assert_eq!(rig.wp.texture_bytes(), 2 * SHARP_BYTES);

    // Kapanış: bulanıktan keskine geçer, kopya geçiş bitince bırakılır.
    rig.wp.set_blur(None);
    let t1 = t0 + Duration::from_secs(3);
    assert!(rig.frame(1.0, t1).redraw);
    assert_eq!(rig.middle_row()[probe], full, "kapanışın başında hâlâ bulanık");
    rig.frame(1.0, t1 + Duration::from_millis(500));
    let back = rig.middle_row()[probe];
    assert!(back > 2 && back < full, "kapanışta karışım: {back}");
    assert_eq!(rig.wp.texture_bytes(), 2 * SHARP_BYTES, "geçiş sürerken kopya tutulur");
    assert_eq!(rig.settle(1.0, t1 + Duration::from_secs(2)), WallpaperStatus::default());
    assert!(rig.middle_row()[probe] < 3, "sonda keskin");
    assert_eq!(rig.wp.texture_bytes(), SHARP_BYTES, "kopya bırakıldı");
}

// --- Zemin (letterbox) ---

/// Dikey görsel: `PW`×`H`, sol yarı siyah, sağ yarı beyaz. `Contain` ile 256×64 hedefte
/// ortada x = 112…144 aralığına sığar; kalan alan letterbox.
const PW: u32 = 32;
const PORTRAIT_BYTES: u64 = (PW * H * 4) as u64;
/// Letterbox'ta, görselden uzak örnek noktaları ve görselin siyah yarısından bir nokta.
const LEFT: usize = 2;
const RIGHT: usize = (W - 3) as usize;
const INSIDE_BLACK: usize = 118;

fn portrait(path: &Path) {
    let img = image::RgbaImage::from_fn(PW, H, |x, _| {
        if x < PW / 2 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255, 255, 255, 255]) }
    });
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), PW, H, image::ExtendedColorType::Rgba8).unwrap();
    std::fs::write(path, out).unwrap();
}

fn gray(v: f32) -> nuxut_core::Color {
    nuxut_core::Color::rgb(v, v, v)
}

fn near(got: u8, want: u8) -> bool {
    got.abs_diff(want) <= 3
}

impl Rig {
    fn load_portrait(&mut self, name: &str) {
        let path = self.dir.path().join(name);
        portrait(&path);
        self.wp.set_source(&path, (W, H));
        self.loaded.recv_timeout(Duration::from_secs(5)).expect("yükleme bitmedi");
    }
}

#[test]
fn color_backdrop_fills_letterbox_and_empty_surface() {
    let Some(mut rig) = Rig::new(Motion::INSTANT) else { return };
    let now = Instant::now();
    rig.wp.set_fit(ImageFit::Contain);
    rig.wp.set_backdrop(WallpaperBackdrop::Color(gray(0.5)));
    // Görsel yokken de zemin boyanır (yüzey opak kalır).
    rig.frame(1.0, now);
    assert!(rig.middle_row().iter().all(|&v| near(v, 128)), "boş yüzeyde zemin");
    rig.load_portrait("p.png");
    assert_eq!(rig.settle(1.0, now), WallpaperStatus::default());
    let row = rig.middle_row();
    assert!(near(row[LEFT], 128) && near(row[RIGHT], 128), "letterbox zemin rengi: {} {}", row[LEFT], row[RIGHT]);
    assert!(row[INSIDE_BLACK] < 3, "görsel zeminin üstünde");
    assert_eq!(rig.wp.texture_bytes(), PORTRAIT_BYTES, "düz renk zemin doku tutmaz");

    // Değişince hasar + yeni renk; aynı değer tekrar verilince iş yok.
    rig.wp.set_backdrop(WallpaperBackdrop::Color(gray(1.0)));
    rig.frame(1.0, now);
    assert!(near(rig.middle_row()[LEFT], 255));
    rig.wp.set_backdrop(WallpaperBackdrop::Color(gray(1.0)));
    assert_eq!(rig.frame(1.0, now), WallpaperStatus::default());

    // None: letterbox saydam kalır (hedef FBO temizlenmiş, 0).
    rig.wp.set_backdrop(WallpaperBackdrop::None);
    rig.frame(1.0, now);
    assert_eq!(rig.middle_row()[LEFT], 0);
}

#[test]
fn blur_backdrop_is_made_once_and_released() {
    let Some(mut rig) = Rig::new(Motion::INSTANT) else { return };
    let now = Instant::now();
    rig.wp.set_fit(ImageFit::Contain);
    let fallback = gray(0.5);
    rig.wp.set_backdrop(WallpaperBackdrop::Blur { radius: 8.0, fallback });
    rig.load_portrait("p.png");
    // Görsel, zemin kopyası hazır olmadan görünmez: görsel görünen her karede letterbox
    // yedek renk değil, görselin bulanık hâli.
    rig.settle_checked(1.0, now, |row| {
        if row[INSIDE_BLACK] < 3 {
            assert!(!near(row[LEFT], 128), "görsel yedek renkli letterbox'la göründü");
        }
    });
    let row = rig.middle_row();
    // Cover yerleşimli kopya: sol letterbox görselin siyah, sağ letterbox beyaz yarısından.
    assert!(row[LEFT] < 40 && row[RIGHT] > 215, "zemin görselden: {} {}", row[LEFT], row[RIGHT]);
    assert!(row[INSIDE_BLACK] < 3, "görsel keskin");
    assert_eq!(rig.wp.texture_bytes(), 2 * PORTRAIT_BYTES, "kopya küçük dokuda, bütçeye sayılır");
    // Durulunca boşta iş yok (kare başına bulanıklık yok).
    for _ in 0..3 {
        assert_eq!(rig.frame(1.0, now), WallpaperStatus::default(), "kare başına zemin bulanıklığı");
    }

    // Yarıçap ve ölçek değişince yeniden üretilir.
    rig.wp.set_backdrop(WallpaperBackdrop::Blur { radius: 2.0, fallback });
    assert!(rig.frame(1.0, now).redraw, "yarıçap değişti");
    assert_eq!(rig.settle(1.0, now), WallpaperStatus::default());
    assert!(rig.frame(2.0, now).redraw, "ölçek değişti");
    assert_eq!(rig.settle(2.0, now), WallpaperStatus::default());
    assert_eq!(rig.wp.texture_bytes(), 2 * PORTRAIT_BYTES);

    // Contain dışında kopya üretilmez: yedek renk.
    rig.wp.set_fit(ImageFit::Cover);
    rig.frame(2.0, now);
    assert_eq!(rig.wp.texture_bytes(), PORTRAIT_BYTES, "Cover'da kopya bırakıldı");
    rig.wp.set_fit(ImageFit::Contain);
    rig.settle(1.0, now);
    assert_eq!(rig.wp.texture_bytes(), 2 * PORTRAIT_BYTES);

    // Düz renge geçince kopya bırakılır.
    rig.wp.set_backdrop(WallpaperBackdrop::Color(gray(1.0)));
    rig.frame(1.0, now);
    assert_eq!(rig.wp.texture_bytes(), PORTRAIT_BYTES);
    assert!(near(rig.middle_row()[LEFT], 255));

    // Geçersiz yarıçap: yalnızca yedek renk.
    rig.wp.set_backdrop(WallpaperBackdrop::Blur { radius: f32::NAN, fallback });
    rig.frame(1.0, now);
    assert_eq!(rig.wp.texture_bytes(), PORTRAIT_BYTES);
    assert!(near(rig.middle_row()[LEFT], 128));
}

#[test]
fn animated_source_blur_backdrop_is_made_once() {
    let Some(mut rig) = Rig::new(Motion::INSTANT) else { return };
    let t0 = Instant::now();
    let a = image::RgbaImage::from_fn(PW, H, |x, _| if x < PW / 2 { image::Rgba([0, 0, 0, 255]) } else { image::Rgba([255; 4]) });
    let b = image::RgbaImage::from_pixel(PW, H, image::Rgba([0, 0, 0, 255]));
    let delay = image::Delay::from_numer_denom_ms(50, 1);
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        enc.set_repeat(image::codecs::gif::Repeat::Infinite).unwrap();
        enc.encode_frames([image::Frame::from_parts(a, 0, 0, delay), image::Frame::from_parts(b, 0, 0, delay)]).unwrap();
    }
    let path = rig.dir.path().join("anim.gif");
    std::fs::write(&path, out).unwrap();
    rig.wp.set_fit(ImageFit::Contain);
    rig.wp.set_backdrop(WallpaperBackdrop::Blur { radius: 8.0, fallback: gray(0.5) });
    rig.wp.set_source(&path, (W, H));
    rig.loaded.recv_timeout(Duration::from_secs(5)).expect("yükleme bitmedi");
    assert_eq!(rig.settle(1.0, t0), WallpaperStatus::default(), "durağan ilk kare: timer yok");
    assert!(rig.wp.animated_source());
    let backdrop = rig.middle_row()[RIGHT];
    for ms in [60, 110, 160, 210] {
        let st = rig.frame(1.0, t0 + Duration::from_millis(ms));
        assert_eq!(st, WallpaperStatus::default(), "{ms} ms: yeni zemin kopyası ya da kare istendi");
        assert_eq!(rig.middle_row()[RIGHT], backdrop, "{ms} ms: zemin ilk kareden, sabit");
    }
    assert_eq!(rig.wp.texture_bytes(), 2 * PORTRAIT_BYTES);
}

#[test]
fn backdrop_color_accessor() {
    let c = gray(0.25);
    assert_eq!(WallpaperBackdrop::None.color(), None);
    assert_eq!(WallpaperBackdrop::Color(c).color(), Some(c));
    assert_eq!(WallpaperBackdrop::Blur { radius: 4.0, fallback: c }.color(), Some(c));
    assert_eq!(Wallpaper::new(ImageFit::Contain, Motion::INSTANT, WallpaperDecoder::in_process_unsandboxed(WallpaperDecoderOptions::default()), || {}).backdrop(), WallpaperBackdrop::None);
}

/// Bellek içi kare: sol yarı siyah, sağ yarı `right` gri (`write_png` ile aynı desen).
fn memory_frame(w: u32, h: u32, right: u8) -> nuxut_image::WallpaperFrame {
    let px: Vec<u8> = (0..w * h).flat_map(|i| if i % w < w / 2 { [0, 0, 0, 255] } else { [right, right, right, 255] }).collect();
    nuxut_image::Image::from_straight(w, h, px).unwrap().into()
}

/// `set_image`: dosyadan bellek içi kareye mevcut yoldaki gibi çapraz geçiş; durulunca boşta
/// iş yok; aynı anahtar + boyut yeniden verilince hiçbir şey olmaz; çıkış boyutu değişince
/// yeni boyutta kare kabul edilir.
#[test]
fn memory_image_crossfades_like_a_file_and_dedups() {
    let Some(mut rig) = Rig::new(Curve::new(CubicBezier::LINEAR, 1.0)) else { return };
    let t0 = Instant::now();
    let right = (W - 3) as usize;
    rig.load("a.png", 255);
    rig.settle(1.0, t0);
    let frame = memory_frame(W, H, 128);
    rig.wp.set_image(7, frame.clone());
    assert!(matches!(rig.wp.source(), Some(nuxut_image::WallpaperSource::Image(7))));
    // Doku hazır olunca geçiş başlar (saat t0da durur: başında kalır).
    for _ in 0..5 {
        assert!(rig.frame(1.0, t0).redraw, "geçiş sürüyor");
    }
    rig.frame(1.0, t0 + Duration::from_millis(500));
    let mid = rig.middle_row()[right];
    assert!((140..245).contains(&mid), "karışım: {mid}");
    assert_eq!(rig.settle(1.0, t0 + Duration::from_secs(2)), WallpaperStatus::default(), "durulunca boşta iş yok");
    assert!((126..131).contains(&rig.middle_row()[right]));
    assert_eq!(rig.wp.texture_bytes(), SHARP_BYTES, "eski katman bırakıldı");
    // Aynı anahtar + boyut: no-op (her configure'da çağrılabilir).
    rig.wp.set_image(7, frame);
    assert_eq!(rig.frame(1.0, t0 + Duration::from_secs(3)), WallpaperStatus::default());
    // Çıkış boyutu değişti: çağıran yeni boyutta yeniden çizer; aynı anahtar kabul edilir.
    rig.wp.set_image(7, memory_frame(W / 2, H / 2, 128));
    for _ in 0..5 {
        assert!(rig.frame(1.0, t0 + Duration::from_secs(3)).redraw, "geçiş sürüyor");
    }
    rig.settle(1.0, t0 + Duration::from_secs(5));
    assert_eq!(rig.wp.texture_bytes(), SHARP_BYTES / 4);
    assert!((126..131).contains(&rig.middle_row()[right]), "yerleşim GPU'da");
}

/// Bellek içi kare süren dosya isteğini geçersiz kılar: geç gelen dosya sonucu gösterilmez.
/// Geçersiz kare reddedilir, eski görsel kalır.
#[test]
fn memory_image_supersedes_pending_file_and_rejects_invalid() {
    let Some(mut rig) = Rig::new(Motion::INSTANT) else { return };
    let t0 = Instant::now();
    let right = (W - 3) as usize;
    let path = rig.dir.path().join("late.png");
    write_png(&path, 255);
    rig.wp.set_source(&path, (W, H));
    rig.wp.set_image(1, memory_frame(W, H, 60));
    // Dosya isteği iptal edildi ya da sonucu gelirse kuşağı eski: atılır.
    let _ = rig.loaded.recv_timeout(Duration::from_millis(500));
    rig.settle(1.0, t0);
    assert!((58..63).contains(&rig.middle_row()[right]), "bellek içi kare gösteriliyor");

    let bad = nuxut_image::WallpaperFrame { width: 4, height: 4, pixels: std::sync::Arc::from(vec![0u8; 3]), animated_source: false };
    assert!(!bad.is_valid());
    rig.wp.set_image(2, bad);
    assert!(rig.wp.last_error().is_some());
    assert!(matches!(rig.wp.source(), Some(nuxut_image::WallpaperSource::Image(1))), "eski kaynak kalır");
    rig.settle(1.0, t0);
    assert!((58..63).contains(&rig.middle_row()[right]));
}
