//! ADR-0008 blob sahneleri, tam ekran bir katman yüzeyinde (izole sway görsel kontrolü).
//! YALNIZCA test compositor'ında çalıştırın (`tools/test-compositor.sh`, `tools/blob-shots.sh`).
//!
//! Sahne tanımları altın görüntü testiyle ortak: `nuxut-render/tests/it/support/scenes.rs`.
//!
//! ```text
//! blob_scenes [SAHNE] [--animate]     # varsayılan: top
//! blob_scenes --list                  # sahne adları
//! ```
//!
//! `--animate`: panel açılışı tema yayıyla (`theme.motion.spatial`) gerçek zamanlı oynar.
//! Deterministik kareler için stdin'den komut: `t <ms>` animasyonu o zamanda dondurup çizer,
//! `q` çıkar. Ekrana çizildikten sonra stdout'a `drawn` yazılır (betikler bekleyebilsin).

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

#[path = "../../nuxut-render/tests/it/support/scenes.rs"]
mod scenes;

use nuxut_core::Size;
use nuxut_motion::Animated;
use nuxut_wayland::{
    Anchor, App, Context, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent,
    SurfaceId,
};
use scenes::SceneParams;

enum Message {
    /// Animasyonu `t` anında dondur.
    Freeze(Duration),
    Quit,
}

struct Demo {
    scene: String,
    animate: bool,
    surface: Option<(SurfaceId, Size)>,
    open: Option<Animated<f32>>,
    /// Animasyonun başladığı an (dondurma bu ana göre).
    start: Option<Instant>,
    frozen: Option<Duration>,
}

impl Demo {
    fn open_value(&mut self, now: Instant) -> f32 {
        if !self.animate {
            return 1.0;
        }
        let start = *self.start.get_or_insert(now);
        let at = self.frozen.map_or(now, |t| start + t);
        self.open.as_ref().map_or(1.0, |a| a.value(at))
    }
}

impl App for Demo {
    type Message = Message;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        let mut spec = LayerSurfaceSpec::new("nuxut-blob-scenes", output.id);
        spec.layer = Layer::Top;
        spec.anchor = Anchor::ALL;
        spec.exclusive_zone = -1;
        self.surface = Some((cx.create_layer_surface(spec), Size::ZERO));
        if self.animate {
            let start = Instant::now();
            let mut open = Animated::new(0.0f32, cx.theme().motion.spatial);
            open.set_target(1.0, start);
            self.open = Some(open);
            self.start = Some(start);
        }
        // stdin komutları (betikler için); EOF'ta thread biter.
        let sender = cx.sender();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let mut words = line.split_whitespace();
                let message = match (
                    words.next(),
                    words.next().and_then(|v| v.parse::<u64>().ok()),
                ) {
                    (Some("t"), Some(ms)) => Message::Freeze(Duration::from_millis(ms)),
                    (Some("q"), _) => Message::Quit,
                    _ => continue,
                };
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn message(&mut self, cx: &mut Context<Self>, message: Message) {
        match message {
            Message::Freeze(t) => {
                self.frozen = Some(t);
                if let Some((id, _)) = self.surface {
                    cx.request_frame(id);
                }
            }
            Message::Quit => cx.quit(),
        }
    }

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        _scale: f32,
    ) {
        if let Some((id, s)) = self.surface.as_mut()
            && *id == surface
        {
            *s = size;
            cx.request_frame(surface);
        }
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        now: Instant,
        painter: &mut Painter,
    ) {
        let Some((_, size)) = self.surface.filter(|(id, _)| *id == surface) else {
            return;
        };
        let open = self.open_value(now);
        let params = SceneParams::from_theme(cx.theme(), size);
        // Zıt bir zemin: koyu yüzey rengi compositor'ın siyah arka planında seçilsin diye
        // serbest alan (delik) vurgu rengiyle görünür.
        painter.rect(
            nuxut_core::Rect::new(0.0, 0.0, size.width, size.height),
            &nuxut_render::RectStyle::filled(cx.theme().colors.primary, nuxut_core::Corners::ZERO),
        );
        if let Some(field) = scenes::scene(&params, &self.scene, open) {
            painter.blob(&field);
        }
        let running = self.animate
            && self.frozen.is_none()
            && self.open.as_mut().is_some_and(|a| !a.is_settled(now));
        if running {
            cx.request_frame(surface);
        } else if !self.animate || self.frozen.is_some() {
            // Kare kuyruğa alındı; betik kısa bir beklemeden sonra görüntü alır. Gerçek zamanlı
            // animasyonun sonu bildirilmez: `t` komutlarıyla sayaç bire bir kalsın.
            println!("drawn");
            let _ = std::io::stdout().flush();
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let params = SceneParams::from_theme(&nuxut_wayland::Theme::default(), Size::new(480.0, 300.0));
    if args.iter().any(|a| a == "--list") {
        for name in scenes::names(&params) {
            println!("{name}");
        }
        return;
    }
    let animate = args.iter().any(|a| a == "--animate");
    let scene = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "top".into());
    if scenes::scene(&params, &scene, 1.0).is_none() {
        eprintln!("bilinmeyen sahne: {scene} (--list)");
        std::process::exit(2);
    }
    let demo = Demo {
        scene,
        animate,
        surface: None,
        open: None,
        start: None,
        frozen: None,
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
