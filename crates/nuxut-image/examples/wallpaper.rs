//! Duvar kağıdı + ikon demosu. Varsayılan: ekranın ortasında, tıklama geçiren yuvarlak köşeli
//! bir "önizleme" yüzeyi (canlı shell'in duvar kağıdının üstünde görünsün diye Top katmanı);
//! kaynaklar sırayla çapraz geçişle değişir, altta temadan ikon şeridi.
//!
//! ```sh
//! cargo run -p nuxut-image --example wallpaper
//! NUXUT_WALLPAPERS="a.jpg:b.gif" NUXUT_DEMO_PERIOD=4 cargo run -p nuxut-image --example wallpaper
//! NUXUT_DEMO_LAYER=background …   # gerçek duvar kağıdı gibi tam ekran Background katmanı
//! NUXUT_DEMO_FIT=contain NUXUT_DEMO_BACKDROP=blur …   # letterbox zemini: none / color / blur
//!   (NUXUT_DEMO_BACKDROP_RADIUS, mantıksal px, varsayılan 48)
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_image::{Decoded, IconTheme, ImageError, LoadRequest, Loader, Trust, Wallpaper, WallpaperBackdrop, WallpaperDecoder, WallpaperDecoderOptions};
use nuxut_render::{ImageData, ImageFit, ImageStyle, Painter, RectStyle, Texture, TextureOptions};
use nuxut_wayland::{
    Anchor, App, Context, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, PointerEvent, SurfaceId, TimerId,
};

const ICONS: [&str; 8] = [
    "kitty",
    "firefox",
    "org.gnome.Nautilus",
    "utilities-terminal",
    "folder",
    "audio-volume-high",
    "network-wireless",
    "preferences-system",
];
const ICON_SIZE: f32 = 32.0;

enum Msg {
    Wake,
    Icon(usize, Result<Decoded, ImageError>),
}

struct Screen {
    output: OutputId,
    size: Size,
    scale: f32,
    wallpaper: Wallpaper,
    anim_timer: Option<TimerId>,
}

impl Screen {
    fn target_px(&self) -> (u32, u32) {
        ((self.size.width * self.scale).round() as u32, (self.size.height * self.scale).round() as u32)
    }
}

struct Demo {
    /// Süreç başına tek çözücü; çıkışlar paylaşır. Örnek: süreç içi (ürün yalıtılmış işçiyi kullanır).
    decoder: Arc<WallpaperDecoder>,
    full: bool,
    fit: ImageFit,
    /// `none` / `color` / `blur` (NUXUT_DEMO_BACKDROP).
    backdrop: String,
    backdrop_radius: f32,
    sources: Vec<PathBuf>,
    /// Gösterilen kaynağın indeksi.
    current: Option<usize>,
    period: Duration,
    cycle: Option<TimerId>,
    screens: HashMap<SurfaceId, Screen>,
    loader: Loader,
    theme: Arc<IconTheme>,
    icons: Vec<Option<Texture>>,
    pending_icons: Vec<(usize, ImageData)>,
    icons_requested: bool,
}

impl Demo {
    fn show(&mut self, cx: &mut Context<Self>, index: usize) {
        let Some(path) = self.sources.get(index).cloned() else { return };
        self.current = Some(index);
        eprintln!("kaynak: {}", path.display());
        for (&id, s) in &mut self.screens {
            let px = s.target_px();
            if px.0 > 0 && px.1 > 0 {
                s.wallpaper.set_source(&path, px);
            }
            cx.request_frame(id);
        }
    }
}

impl App for Demo {
    type Message = Msg;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        let spec = if self.full {
            LayerSurfaceSpec {
                layer: Layer::Background,
                anchor: Anchor::ALL,
                exclusive_zone: -1,
                ..LayerSurfaceSpec::new("nuxut-wallpaper-demo", output.id)
            }
        } else {
            LayerSurfaceSpec {
                layer: Layer::Top,
                anchor: Anchor::NONE,
                size: (960, 600),
                keyboard: KeyboardMode::None,
                ..LayerSurfaceSpec::new("nuxut-wallpaper-demo", output.id)
            }
        };
        let id = cx.create_layer_surface(spec);
        cx.set_input_region(id, &[]);
        let sender = cx.sender();
        let mut wallpaper = Wallpaper::new(self.fit, cx.theme().motion.effects, self.decoder.clone(), move || {
            let _ = sender.send(Msg::Wake);
        });
        // Letterbox zemini: renkler tema token'ı.
        let surface = cx.theme().colors.surface;
        wallpaper.set_backdrop(match self.backdrop.as_str() {
            "blur" => WallpaperBackdrop::Blur { radius: self.backdrop_radius, fallback: surface },
            "color" => WallpaperBackdrop::Color(surface),
            _ => WallpaperBackdrop::None,
        });
        if !self.full {
            wallpaper.set_radii(Corners::uniform(cx.theme().shape.radius_large));
        }
        self.screens.insert(id, Screen { output: output.id, size: Size::ZERO, scale: 1.0, wallpaper, anim_timer: None });
        if !self.icons_requested {
            self.icons_requested = true;
            let scale = output.scale.max(1.0).ceil() as u32;
            for (i, name) in ICONS.iter().enumerate() {
                let sender = cx.sender();
                let req = LoadRequest::icon(self.theme.clone(), *name, ICON_SIZE as u32, scale).with_trust(Trust::System);
                self.loader.load(req, move |r| {
                    let _ = sender.send(Msg::Icon(i, r));
                });
            }
        }
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.screens.retain(|&id, s| {
            let keep = s.output != output;
            if !keep {
                cx.destroy_surface(id);
            }
            keep
        });
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        if let Some(s) = self.screens.get_mut(&surface) {
            s.size = size;
            s.scale = scale;
        }
        match self.current {
            None => {
                self.show(cx, 0);
                self.cycle = Some(cx.set_timer(self.period));
            }
            Some(i) => {
                // Boyut değişti: aynı kaynağı yeni boyuta yeniden çöz (aynı istekse no-op).
                if let (Some(path), Some(s)) = (self.sources.get(i), self.screens.get_mut(&surface)) {
                    let px = s.target_px();
                    s.wallpaper.set_source(path, px);
                }
            }
        }
        cx.request_frame(surface);
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn message(&mut self, cx: &mut Context<Self>, message: Msg) {
        match message {
            Msg::Wake => {}
            Msg::Icon(i, Ok(d)) => {
                let img = d.into_first_frame();
                let (width, height) = img.size();
                self.pending_icons.push((i, ImageData { width, height, pixels: Arc::from(img.into_pixels()) }));
            }
            Msg::Icon(i, Err(e)) => eprintln!("ikon {}: {e}", ICONS[i]),
        }
        for &id in self.screens.keys() {
            cx.request_frame(id);
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.cycle == Some(id) {
            let next = self.current.map_or(0, |i| (i + 1) % self.sources.len().max(1));
            self.show(cx, next);
            self.cycle = Some(cx.set_timer(self.period));
            return;
        }
        for (&sid, s) in &mut self.screens {
            if s.anim_timer == Some(id) {
                s.anim_timer = None;
                cx.request_frame(sid);
            }
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let theme = cx.theme().clone();
        for (i, data) in self.pending_icons.drain(..) {
            self.icons.resize(ICONS.len(), None);
            self.icons[i] = Some(painter.upload_texture(data, TextureOptions::default()));
        }
        let Some(s) = self.screens.get_mut(&surface) else { return };
        let bounds = Rect::from_origin_size(Point::ZERO, painter.size());
        let status = s.wallpaper.draw(painter, bounds, now);

        // İkon şeridi.
        let n = self.icons.iter().flatten().count() as f32;
        if n > 0.0 {
            let gap = theme.space(3);
            let pad = theme.space(3);
            let w = n * ICON_SIZE + (n - 1.0) * gap + pad * 2.0;
            let h = ICON_SIZE + pad * 2.0;
            let strip = Rect::new((bounds.width - w) * 0.5, bounds.height - h - theme.space(6), w, h);
            let fill = theme.colors.surface.with_alpha(0.82);
            painter.rect(strip, &RectStyle::filled(fill, Corners::uniform(theme.shape.radius_medium)));
            let mut x = strip.x + pad;
            for tex in self.icons.iter().flatten() {
                let r = Rect::new(x, strip.y + pad, ICON_SIZE, ICON_SIZE);
                painter.image(r, tex, &ImageStyle::new(ImageFit::Contain));
                x += ICON_SIZE + gap;
            }
        }

        if status.redraw {
            cx.request_frame(surface);
        } else if let Some(d) = status.next_frame_in
            && s.anim_timer.is_none()
        {
            s.anim_timer = Some(cx.set_timer(d));
        }
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let sources: Vec<PathBuf> = match std::env::var("NUXUT_WALLPAPERS") {
        Ok(v) => v.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect(),
        Err(_) => vec![
            "/usr/share/backgrounds/archlinux/simple.png".into(),
            "/usr/share/doc/kitty/html/_images/quake-hypr.webp".into(),
        ],
    };
    if sources.is_empty() {
        eprintln!("NUXUT_WALLPAPERS boş");
        std::process::exit(2);
    }
    let period = std::env::var("NUXUT_DEMO_PERIOD")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map_or(Duration::from_secs(6), Duration::from_secs_f32);
    let theme = Arc::new(IconTheme::system());
    eprintln!("ikon teması: {}", theme.theme_name());
    let demo = Demo {
        decoder: WallpaperDecoder::in_process_unsandboxed(WallpaperDecoderOptions::default()),
        full: std::env::var("NUXUT_DEMO_LAYER").is_ok_and(|v| v == "background"),
        fit: match std::env::var("NUXUT_DEMO_FIT").as_deref() {
            Ok("contain") => ImageFit::Contain,
            Ok("fill") => ImageFit::Fill,
            _ => ImageFit::Cover,
        },
        backdrop: std::env::var("NUXUT_DEMO_BACKDROP").unwrap_or_default(),
        backdrop_radius: std::env::var("NUXUT_DEMO_BACKDROP_RADIUS").ok().and_then(|v| v.parse().ok()).unwrap_or(48.0),
        sources,
        current: None,
        period,
        cycle: None,
        screens: HashMap::new(),
        loader: Loader::new(2),
        theme,
        icons: Vec::new(),
        pending_icons: Vec::new(),
        icons_requested: false,
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        eprintln!("hata: {e}");
        std::process::exit(1);
    }
}
