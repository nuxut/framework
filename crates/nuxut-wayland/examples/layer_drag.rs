//! Sürüklenerek taşınan layer yüzeyi (`Context::set_layer_placement`). YALNIZCA test
//! compositor'ında çalıştırın (`tools/test-compositor.sh`); işaretçi o display'e
//! `test-compositor.sh pointer ...` ile verilir.
//!
//! 200×120 yüzey, `TOP | LEFT`, margin (100, 100). Sol tuşla basılıp sürüklenince her
//! hareket olayında yeni margin istenir; runtime çağrıları kareye birleştirir.
//!
//! Basılıyken (örtük kapma) yüzey içi konumun tabanı compositor'a bağlıdır: sway 1.12 basma
//! anındaki yüzey konumunu tutar (varsayılan: ekran konumu = basmadaki margin + yerel);
//! `NUXUT_DRAG_BASE=current` tabanı yüzeyin son *gönderilen* konumu sayar (draw'da kaydedilir:
//! yerleşim o karenin commit'iyle gider). Bırakınca `DRAG-DONE margin=(x,y) calls=N frames=M` yazar ve
//! 2 sn sonra çıkar. 30 sn içinde bitmezse hata koduyla çıkar.

use std::time::{Duration, Instant};

use nuxut_core::{Corners, Insets, Point, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    Anchor, App, Button, Context, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo,
    Painter, PointerEvent, PointerKind, SurfaceId, TimerId,
};

const W: u32 = 200;
const H: u32 = 120;

#[derive(Default)]
struct Demo {
    surface: Option<SurfaceId>,
    size: Size,
    /// İstenen (en son) sol-üst margin.
    margin: Point,
    /// Son çizimde (o karenin commit'iyle) gönderilen margin.
    committed: Point,
    /// Basma anındaki margin (işaretçi yerelinin tabanı sway gibi basma anında sabitse).
    press_margin: Point,
    /// Basma anında yüzey içi tutma noktası.
    grab: Option<Point>,
    /// `NUXUT_DRAG_BASE=current`: yerel konum yüzeyin *şu anki* (son gönderilen) konumuna göre.
    base_current: bool,
    calls: u32,
    frames: u32,
    deadline: Option<TimerId>,
    done: bool,
}

impl Demo {
    fn place(&mut self, cx: &mut Context<Self>, id: SurfaceId) {
        let m = Insets::new(self.margin.y, 0.0, 0.0, self.margin.x);
        cx.set_layer_placement(id, Anchor::TOP | Anchor::LEFT, m, None);
        self.calls += 1;
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        self.margin = Point::new(100.0, 100.0);
        self.committed = self.margin;
        self.base_current = std::env::var("NUXUT_DRAG_BASE").is_ok_and(|v| v == "current");
        let spec = LayerSurfaceSpec {
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::LEFT,
            size: (W, H),
            margin: Insets::new(100.0, 0.0, 0.0, 100.0),
            keyboard: KeyboardMode::None,
            ..LayerSurfaceSpec::new("nuxut-layer-drag", output.id)
        };
        let id = cx.create_layer_surface(spec);
        // İlk tampondan önce aynı yerleşim: istek üretmez.
        cx.set_layer_placement(id, Anchor::TOP | Anchor::LEFT, Insets::new(100.0, 0.0, 0.0, 100.0), Some((W, H)));
        self.surface = Some(id);
        self.deadline = Some(cx.set_timer(Duration::from_secs(30)));
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, size: Size, _scale: f32) {
        self.size = size;
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        match event.kind {
            PointerKind::Press(Button::Left) => {
                self.grab = Some(event.position);
                self.press_margin = self.committed;
                tracing::info!("sürükleme başladı");
            }
            PointerKind::Motion => {
                if let Some(grab) = self.grab {
                    // Ekran konumu, işaretçinin göreli olduğu (gönderilmiş) yerleşimden.
                    let base = if self.base_current { self.committed } else { self.press_margin };
                    let global = Point::new(base.x + event.position.x, base.y + event.position.y);
                    self.margin = Point::new((global.x - grab.x).max(0.0), (global.y - grab.y).max(0.0));
                    tracing::debug!(local = ?event.position, committed = ?self.committed, margin = ?self.margin, "sürükleme");
                    self.place(cx, surface);
                }
            }
            PointerKind::Release(Button::Left) if self.grab.take().is_some() => {
                println!(
                    "DRAG-DONE margin=({},{}) calls={} frames={}",
                    self.margin.x as i32, self.margin.y as i32, self.calls, self.frames
                );
                self.done = true;
                cx.set_timer(Duration::from_secs(2));
            }
            _ => {}
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if Some(id) == self.deadline && !self.done {
            eprintln!("DEMO-TIMEOUT");
            std::process::exit(2);
        }
        if self.done {
            cx.quit();
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, _now: Instant, painter: &mut Painter) {
        self.frames += 1;
        self.committed = self.margin;
        let theme = cx.theme();
        let rect = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        painter.rect(rect, &RectStyle::filled(theme.colors.primary, Corners::uniform(theme.shape.radius_large)));
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,layer_drag=debug".into()),
        )
        .init();
    nuxut_wayland::run(Demo::default()).expect("runtime");
}
