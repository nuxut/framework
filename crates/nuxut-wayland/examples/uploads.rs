//! Doku yükleme bütçesi demosu: bir karede `UPLOAD_BUDGET_PER_FRAME`'i aşan dokular sonraki
//! karelere kalır; runtime bekleyen yükleme oldukça kendiliğinden kare ister, bitince durur.
//!
//! İlk çıkışın sol üstünde 240x240 bir layer yüzeyi açar (`NUXUT_OUTPUT` ile çıkış seçilir).
//!
//! ```sh
//! UPLOAD_COUNT=3 UPLOAD_SIDE=5120 cargo run --release -p nuxut-wayland --example uploads
//! ```

use std::sync::Arc;
use std::time::Instant;

use nuxut_core::{Rect, Size};
use nuxut_render::{ImageData, ImageStyle, Texture, TextureOptions};
use nuxut_wayland::{
    Anchor, App, Context, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent,
    SurfaceId,
};

struct Demo {
    /// `NUXUT_OUTPUT` verilirse yalnızca o çıkış; yoksa ilk çıkış.
    target: Option<String>,
    count: usize,
    side: u32,
    surface: Option<SurfaceId>,
    size: Size,
    textures: Vec<Texture>,
    frames: u32,
    start: Instant,
    done_logged: bool,
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.target.as_ref().is_some_and(|t| *t != output.name) || self.surface.is_some() {
            return;
        }
        let spec = LayerSurfaceSpec {
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::LEFT,
            size: (240, 240),
            ..LayerSurfaceSpec::new("nuxut-uploads", output.id)
        };
        self.surface = Some(cx.create_layer_surface(spec));
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        _scale: f32,
    ) {
        self.size = size;
        cx.request_frame(surface);
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn draw(
        &mut self,
        _cx: &mut Context<Self>,
        _surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        self.frames += 1;
        if self.textures.is_empty() {
            // Tek seferde hepsini iste; runtime bütçeye göre karelere böler.
            let bytes = (self.side * self.side * 4) as usize;
            for i in 0..self.count {
                let shade = (40 + 60 * i) as u8;
                let mut pixels = vec![0u8; bytes];
                for px in pixels.chunks_exact_mut(4) {
                    px.copy_from_slice(&[shade, 90, 200 - shade / 2, 255]);
                }
                let image = ImageData {
                    width: self.side,
                    height: self.side,
                    pixels: Arc::from(pixels),
                };
                self.textures
                    .push(painter.upload_texture(image, TextureOptions { mipmaps: false }));
            }
            tracing::info!(
                count = self.count,
                mib_each = bytes >> 20,
                "dokular istendi (kare bütçesi {} MiB)",
                nuxut_render::UPLOAD_BUDGET_PER_FRAME >> 20
            );
        }
        let ready = self.textures.iter().filter(|t| t.is_ready()).count();
        tracing::info!(
            frame = self.frames,
            ready,
            ms = self.start.elapsed().as_millis() as u64,
            "kare"
        );
        if ready == self.textures.len() && !self.done_logged {
            self.done_logged = true;
            tracing::info!(frames = self.frames, "tüm dokular hazır");
        }
        let w = self.size.width / self.count.max(1) as f32;
        for (i, t) in self.textures.iter().enumerate() {
            painter.image(
                Rect::new(i as f32 * w, 0.0, w, self.size.height),
                t,
                &ImageStyle::default(),
            );
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let demo = Demo {
        target: std::env::var("NUXUT_OUTPUT").ok(),
        count: env_or("UPLOAD_COUNT", 3),
        side: env_or("UPLOAD_SIDE", 5120u32).min(nuxut_render::MAX_TEXTURE_SIDE),
        surface: None,
        size: Size::ZERO,
        textures: Vec::new(),
        frames: 0,
        start: Instant::now(),
        done_logged: false,
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        // Son kareden sonra ek kare istenmediğini göstermek için: kapanışta durum.
        let ready = self.textures.iter().filter(|t| t.is_ready()).count();
        tracing::info!(
            frames = self.frames,
            ready,
            total = self.textures.len(),
            "kapanış"
        );
    }
}
