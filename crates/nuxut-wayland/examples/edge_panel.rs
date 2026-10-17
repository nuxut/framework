//! Runtime demosu: her çıkışın sağ kenarında yarı saydam bir panel.
//!
//! - Tıklama (veya Enter) panel rengini değiştirir.
//! - Saniye sınırına hizalı bir timer her saniye alttaki çubuğu büyütür (60'ta sıfırlanır).
//! - Panelin yalnızca kendisi tıklama alır; çevresindeki şeffaf pay tıklamayı geçirir.
//! - Escape veya SIGTERM/SIGINT/SIGHUP temiz çıkar (Drop loglanır).
//! - Klavye odağı değişimleri loglanır.
//! - Tek örnek: ikinci çalıştırma argümanlarını IPC ile birinciye iletir
//!   (`edge_panel ping`, `edge_panel color`, `edge_panel echo a "b c"`, `edge_panel quit`).
//! - Tekerlek olayları loglanır.
//! - Saniye tikinde yalnızca çubuk sütunu hasarlı bildirilir (kısmi yeniden çizim).
//! - `DEMO_NO_TICK=1`: timer kurulmaz; boşta sıfır CPU ölçümü için.
//!
//! ```sh
//! RUST_LOG=debug cargo run -p nuxut-wayland --example edge_panel
//! NUXUT_DEBUG_SURFACES=1 cargo run -p nuxut-wayland --example edge_panel   # yüzey sınırları
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nuxut_core::{Color, Corners, Insets, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    Anchor, App, Context, CursorShape, Key, KeyEvent, KeyState, KeyboardMode, Layer,
    LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent, PointerKind, RunOptions,
    SurfaceId, TimerId, ipc,
};

/// Yüzey genişliği; panel bunun içinde `PAD` kadar boşlukla durur.
const WIDTH: u32 = 96;
const PAD: f32 = 12.0;

struct Panel {
    output: OutputId,
    size: Size,
    color: usize,
    /// Tüm panel yeniden çizilmeli (renk/boyut/tema); değilse yalnızca saniye çubuğu.
    repaint_all: bool,
}

#[derive(Default)]
struct Demo {
    panels: HashMap<SurfaceId, Panel>,
    seconds: u32,
    tick: Option<TimerId>,
}

impl Demo {
    fn panel_rect(size: Size) -> Rect {
        Rect::new(0.0, 0.0, size.width, size.height).inset(Insets::uniform(PAD))
    }

    fn palette(cx: &Context<Self>) -> [Color; 3] {
        let c = &cx.theme().colors;
        [c.surface, c.primary, c.surface_bright]
    }

    /// Bir sonraki saniye sınırına hizalı timer (boşta uyanma yalnızca saniyede bir).
    fn schedule_tick(&mut self, cx: &mut Context<Self>) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let until_next =
            Duration::from_secs(1) - Duration::from_nanos(u64::from(now.subsec_nanos()));
        self.tick = Some(cx.set_timer(until_next));
    }

    fn redraw_all(&self, cx: &mut Context<Self>) {
        for &id in self.panels.keys() {
            cx.request_frame(id);
        }
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        tracing::info!(?output, "çıkış");
        let spec = LayerSurfaceSpec {
            layer: Layer::Top,
            anchor: Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
            size: (WIDTH, 0),
            keyboard: KeyboardMode::OnDemand,
            ..LayerSurfaceSpec::new("nuxut-demo", output.id)
        };
        let id = cx.create_layer_surface(spec);
        self.panels.insert(
            id,
            Panel {
                output: output.id,
                size: Size::ZERO,
                color: 0,
                repaint_all: true,
            },
        );
        if self.tick.is_none() && std::env::var_os("DEMO_NO_TICK").is_none() {
            self.schedule_tick(cx);
        }
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.panels.retain(|&id, panel| {
            let keep = panel.output != output;
            if !keep {
                cx.destroy_surface(id);
            }
            keep
        });
    }

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        scale: f32,
    ) {
        tracing::info!(?surface, ?size, scale, "yüzey boyutlandı");
        if let Some(panel) = self.panels.get_mut(&surface) {
            panel.size = size;
            panel.repaint_all = true;
            cx.set_input_region(surface, &[Self::panel_rect(size)]);
        }
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        // Bağlama göre imleç: üst üçte bir düğme gibi (Pointer), orta metin (Text), alt
        // tutamaç (Grab); panel dışındaki şeffaf pay Default. Runtime aynı şekli tekrarlamaz.
        if let Some(panel) = self.panels.get(&surface)
            && matches!(event.kind, PointerKind::Enter | PointerKind::Motion)
        {
            let rect = Self::panel_rect(panel.size);
            let y = (event.position.y - rect.y) / rect.height.max(1.0);
            let shape = if !rect.contains(event.position) {
                CursorShape::Default
            } else if y < 1.0 / 3.0 {
                CursorShape::Pointer
            } else if y < 2.0 / 3.0 {
                CursorShape::Text
            } else {
                CursorShape::Grab
            };
            cx.set_cursor(surface, shape);
        }
        match event.kind {
            PointerKind::Press(button) => {
                tracing::info!(?button, pos = ?event.position, "tıklama");
                if let Some(panel) = self.panels.get_mut(&surface) {
                    panel.color = (panel.color + 1) % 3;
                    panel.repaint_all = true;
                    cx.request_frame(surface);
                }
            }
            PointerKind::Scroll { delta, steps } => tracing::info!(?delta, ?steps, "kaydırma"),
            PointerKind::ScrollEnd => tracing::info!("kaydırma bitti"),
            PointerKind::Enter | PointerKind::Leave => {
                tracing::debug!(kind = ?event.kind, "işaretçi")
            }
            _ => {}
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        tracing::info!(?event, "tuş");
        if event.state == KeyState::Released {
            return;
        }
        match event.key {
            Key::Escape => cx.quit(),
            Key::Enter => {
                if let Some(panel) = self.panels.get_mut(&surface) {
                    panel.color = (panel.color + 1) % 3;
                    panel.repaint_all = true;
                    cx.request_frame(surface);
                }
            }
            _ => {}
        }
    }

    fn command(&mut self, cx: &mut Context<Self>, args: Vec<String>, reply: ipc::Reply) {
        tracing::info!(?args, "komut");
        match args
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["ping"] => reply.send("pong"),
            ["color"] => {
                for panel in self.panels.values_mut() {
                    panel.color = (panel.color + 1) % 3;
                    panel.repaint_all = true;
                }
                self.redraw_all(cx);
                reply.send("ok");
            }
            ["echo", rest @ ..] => reply.send(&rest.join(" | ")),
            ["quit"] => {
                reply.send("ok");
                cx.quit();
            }
            _ => reply.send("error: komutlar: ping, color, echo <arg>…, quit"),
        }
    }

    fn keyboard_focus(&mut self, _cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        tracing::info!(?surface, focused, "klavye odağı");
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.tick == Some(id) {
            self.seconds = (self.seconds + 1) % 60;
            self.redraw_all(cx);
            self.schedule_tick(cx);
        }
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        tracing::info!("tema değişti");
        for panel in self.panels.values_mut() {
            panel.repaint_all = true;
        }
        self.redraw_all(cx);
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let Some(panel) = self.panels.get_mut(&surface) else {
            return;
        };
        let theme = cx.theme();
        let rect = Self::panel_rect(panel.size);
        let fill = Self::palette(cx)[panel.color].with_alpha(0.72);
        painter.rect(
            rect,
            &RectStyle::filled(fill, Corners::uniform(theme.shape.radius_large)),
        );

        // Saniye çubuğu: panelin altından yukarı büyür.
        let inner = rect.inset(Insets::uniform(PAD));
        let h = inner.height * (self.seconds as f32 + 1.0) / 60.0;
        let bar = Rect::new(inner.x, inner.bottom() - h, inner.width, h);
        // Hasar: saniye tikinde yalnızca çubuk sütunu (60'ta sıfırlanınca eski uzun çubuk da
        // bu sütunda kalır), diğer değişikliklerde tüm yüzey.
        if std::mem::take(&mut panel.repaint_all) {
            painter.damage_all();
        } else {
            painter.damage(inner);
        }
        painter.rect(
            bar,
            &RectStyle::filled(
                theme.colors.on_surface.with_alpha(0.85),
                theme.shape.small(),
            ),
        );
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        tracing::info!(panels = self.panels.len(), "Demo düşürüldü");
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    // Tek örnek: ikinci çalıştırma argümanlarını birinciye iletir (`edge_panel color`).
    let listener = match ipc::try_single_instance("nuxut-demo") {
        Ok(ipc::Instance::Primary(listener)) => Some(listener),
        Ok(ipc::Instance::Secondary(client)) => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            match client.send_args(&args) {
                Ok(reply) => {
                    println!("{reply}");
                    return;
                }
                Err(e) => {
                    eprintln!("komut iletilemedi: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            tracing::warn!("tek örnek IPC kullanılamıyor: {e}");
            None
        }
    };
    if let Err(e) = nuxut_wayland::run_with(Demo::default(), RunOptions { ipc: listener }) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
    tracing::info!("run döndü");
}
