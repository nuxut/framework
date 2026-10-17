//! Video duvar kağıdı demosu: kaynak önce içe aktarılır (önbellekte yoksa dönüştürülür,
//! ilerleme stderr'e), sonra libmpv ile yuvarlak köşeli önizleme yüzeyinde oynatılır.
//!
//! ```sh
//! NUXUT_VIDEO=klip.mkv cargo run --release -p nuxut-video --example video_wallpaper
//! NUXUT_DEMO_LAYER=background …   # tam ekran Background katmanı
//! NUXUT_DEMO_AUDIO=1 …            # sesi aç
//! NUXUT_DEMO_FIT=contain …        # letterbox
//! NUXUT_DEMO_BACKDROP=surface …   # letterbox zemini: none | surface | primary | blur
//! NUXUT_DEMO_UNDERLAY=1 …         # videonun altına yüzey arka planı (saydam zemin denetimi)
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_image::{IsolatedDecoder, WallpaperBackdrop};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};
use nuxut_render::{ImageFit, Painter, RectStyle};
use nuxut_video::{ImportEvent, ImportRequest, Imported, Importer, VideoPlayer};
use nuxut_wayland::{
    Anchor, App, Context, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, PointerEvent, SurfaceId,
};

enum Msg {
    Wake,
    Imported(Imported),
    Failed(String),
}

struct Screen {
    output: OutputId,
    player: VideoPlayer,
}

struct Demo {
    full: bool,
    source: PathBuf,
    importer: Importer,
    started: bool,
    screens: HashMap<SurfaceId, Screen>,
    /// Videonun altına yüzeyin kendi arka planı (saydam letterbox'ı göstermek için).
    underlay: bool,
}

impl App for Demo {
    type Message = Msg;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        let spec = if self.full {
            LayerSurfaceSpec {
                layer: Layer::Background,
                anchor: Anchor::ALL,
                exclusive_zone: -1,
                ..LayerSurfaceSpec::new("nuxut-video-demo", output.id)
            }
        } else {
            LayerSurfaceSpec {
                layer: Layer::Top,
                anchor: Anchor::NONE,
                size: (960, 540),
                keyboard: KeyboardMode::None,
                ..LayerSurfaceSpec::new("nuxut-video-demo", output.id)
            }
        };
        let id = cx.create_layer_surface(spec);
        cx.set_input_region(id, &[]);
        let sender = cx.sender();
        let mut player = match VideoPlayer::new(move || {
            let _ = sender.send(Msg::Wake);
        }) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{e}");
                cx.quit();
                return;
            }
        };
        if std::env::var("NUXUT_DEMO_FIT").is_ok_and(|v| v == "contain") {
            player.set_fit(ImageFit::Contain);
        }
        // Letterbox zemini: tema rolü (none | surface | primary | blur).
        let colors = &cx.theme().colors;
        player.set_backdrop(match std::env::var("NUXUT_DEMO_BACKDROP").as_deref() {
            Ok("surface") => WallpaperBackdrop::Color(colors.surface),
            Ok("primary") => WallpaperBackdrop::Color(colors.primary),
            Ok("blur") => WallpaperBackdrop::Blur { radius: 48.0, fallback: colors.surface_container },
            _ => WallpaperBackdrop::None,
        });
        if !self.full {
            player.set_radii(Corners::uniform(cx.theme().shape.radius_large));
        }
        if std::env::var_os("NUXUT_DEMO_AUDIO").is_some() {
            player.set_muted(false);
            player.set_volume(40.0);
        }
        self.screens.insert(id, Screen { output: output.id, player });
        if !self.started {
            self.started = true;
            let target = if self.full {
                ((output.size.width * output.scale).round() as u32, (output.size.height * output.scale).round() as u32)
            } else {
                (960, 540)
            };
            let req = ImportRequest { source: self.source.clone(), target_px: target };
            let sender = cx.sender();
            let started = Instant::now();
            let _job = self.importer.start(req, move |ev| match ev {
                ImportEvent::Progress(p) => eprint!("\riçe aktarma %{:.0}   ", p * 100.0),
                ImportEvent::Done(Ok(i)) => {
                    eprintln!("\riçe aktarıldı ({:.1} s): {}", started.elapsed().as_secs_f32(), i.path().display());
                    let _ = sender.send(Msg::Imported(i));
                }
                ImportEvent::Done(Err(e)) => {
                    let _ = sender.send(Msg::Failed(e.to_string()));
                }
            });
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

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, _size: Size, _scale: f32) {
        cx.request_frame(surface);
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn message(&mut self, cx: &mut Context<Self>, message: Msg) {
        match message {
            Msg::Wake => {}
            Msg::Imported(i) => {
                for s in self.screens.values_mut() {
                    if let Err(e) = s.player.load(i.path()) {
                        eprintln!("{e}");
                    }
                }
            }
            Msg::Failed(e) => {
                eprintln!("içe aktarma başarısız: {e}");
                cx.quit();
            }
        }
        for &id in self.screens.keys() {
            cx.request_frame(id);
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let Some(s) = self.screens.get_mut(&surface) else { return };
        let rect = Rect::from_origin_size(Point::ZERO, painter.size());
        if self.underlay {
            let fill = cx.theme().colors.error;
            painter.rect(rect, &RectStyle::filled(fill, Corners::uniform(cx.theme().shape.radius_large)));
        }
        if s.player.draw(painter, rect, now).redraw {
            cx.request_frame(surface);
        }
    }
}

fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let Some(source) = std::env::var_os("NUXUT_VIDEO").map(PathBuf::from) else {
        eprintln!("NUXUT_VIDEO=<dosya> verin");
        std::process::exit(2);
    };
    let demo = Demo {
        full: std::env::var("NUXUT_DEMO_LAYER").is_ok_and(|v| v == "background"),
        source,
        importer: {
            // Kullanıcı görseli yalıtılmış işçide (ADR-0007); işçi `NUXUT_IMAGE_WORKER` ya da kurulu yerden.
            let decoder: Arc<dyn IsolatedDecoder> = Arc::new(Sandbox::new(SandboxConfig::wallpaper()));
            Importer::with_dir(std::env::var_os("NUXUT_DEMO_CACHE").map_or_else(Importer::default_dir, Into::into), decoder)
        },
        started: false,
        screens: HashMap::new(),
        underlay: std::env::var_os("NUXUT_DEMO_UNDERLAY").is_some(),
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        eprintln!("hata: {e}");
        std::process::exit(1);
    }
}
