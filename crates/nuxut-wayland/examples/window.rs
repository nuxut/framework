//! Pencere demosu: `xdg_toplevel` + uygulamanın çizdiği başlık çubuğu.
//!
//! - Başlık çubuğunda sol tık → taşıma (`begin_move`), sağ tık → büyüt/geri al.
//! - Kenarlardan 6 px içeride sol tık → yeniden boyutlandırma (`begin_resize`).
//! - Sağ üstteki düğme ya da Escape kapatır; compositor'ın kapatma isteği de çıkarır.
//! - Etkin/pasif durum başlık rengini değiştirir; boyut ve durum loglanır.
//!
//! ```sh
//! RUST_LOG=info cargo run -p nuxut-wayland --example window
//! ```

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_render::{RectStyle, TextStyle};
use nuxut_wayland::{
    App, Button, Context, ImeEvent, Key, KeyEvent, KeyState, Modifiers, OutputId, OutputInfo,
    Painter, PointerEvent, PointerKind, ResizeEdge, SurfaceId, SurroundingText, TextInputState,
    WindowSpec, WindowState,
};

const TITLE_H: f32 = 36.0;
const EDGE: f32 = 6.0;
const CLOSE: f32 = 24.0;

#[derive(Default)]
struct Demo {
    window: Option<SurfaceId>,
    size: Size,
    state: WindowState,
    pointer: Point,
    /// Klavye/IME ile yazılan metin.
    typed: String,
}

impl Demo {
    fn close_rect(&self) -> Rect {
        Rect::new(
            self.size.width - CLOSE - 6.0,
            (TITLE_H - CLOSE) / 2.0,
            CLOSE,
            CLOSE,
        )
    }

    fn edge_at(&self, p: Point) -> Option<ResizeEdge> {
        // Tam ekran pencere kenardan boyutlandırılmaz. (Hyprland yüzen pencerelerde de
        // `tiled`/`maximized` bildirir; CSD gölgesi çizilmesin diye. O yüzden onlara bakılmaz.)
        if self.state.fullscreen {
            return None;
        }
        let (l, r) = (p.x < EDGE, p.x > self.size.width - EDGE);
        let (t, b) = (p.y < EDGE, p.y > self.size.height - EDGE);
        Some(match (t, b, l, r) {
            (true, _, true, _) => ResizeEdge::TopLeft,
            (true, _, _, true) => ResizeEdge::TopRight,
            (_, true, true, _) => ResizeEdge::BottomLeft,
            (_, true, _, true) => ResizeEdge::BottomRight,
            (true, ..) => ResizeEdge::Top,
            (_, true, ..) => ResizeEdge::Bottom,
            (_, _, true, _) => ResizeEdge::Left,
            (_, _, _, true) => ResizeEdge::Right,
            _ => return None,
        })
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.window.is_none() {
            let spec = WindowSpec {
                size: (640, 420),
                min_size: Some((320, 200)),
                ..WindowSpec::new("Nuxut pencere demosu", "nuxut-window-demo")
            };
            self.window = Some(cx.create_window(spec));
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        scale: f32,
    ) {
        tracing::info!(?size, scale, "pencere boyutlandı");
        self.size = size;
        cx.request_frame(surface);
    }

    fn window_state_changed(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        state: WindowState,
    ) {
        tracing::info!(?state, "pencere durumu");
        self.state = state;
        // Metin girişi: içerikteki satır gibi davran (IME aday penceresi buraya konumlanır).
        cx.set_text_input(
            surface,
            Some(TextInputState {
                cursor_rect: Rect::new(
                    14.0 + self.typed.len() as f32 * 7.0,
                    TITLE_H + 72.0,
                    2.0,
                    18.0,
                ),
                surrounding: Some(SurroundingText {
                    text: self.typed.clone(),
                    cursor: self.typed.len(),
                    anchor: self.typed.len(),
                }),
                ..TextInputState::default()
            }),
        );
        cx.request_frame(surface);
    }

    fn ime(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: ImeEvent) {
        tracing::info!(?event, "IME");
        match event {
            ImeEvent::Commit(text) => self.typed.push_str(&text),
            ImeEvent::DeleteSurrounding { before, .. } => {
                let keep = self.typed.len().saturating_sub(before);
                let keep = (0..=keep)
                    .rev()
                    .find(|&i| self.typed.is_char_boundary(i))
                    .unwrap_or(0);
                self.typed.truncate(keep);
            }
            ImeEvent::Preedit { .. } => {}
        }
        cx.request_frame(surface);
    }

    fn modifiers_changed(
        &mut self,
        _cx: &mut Context<Self>,
        _surface: SurfaceId,
        modifiers: Modifiers,
    ) {
        tracing::info!(
            caps = modifiers.caps_lock,
            num = modifiers.num_lock,
            "değiştiriciler"
        );
    }

    fn keyboard_layout_changed(&mut self, cx: &mut Context<Self>) {
        tracing::info!(layout = cx.keyboard_layout(), "klavye düzeni");
    }

    fn close_requested(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        tracing::info!("kapatma isteği");
        cx.destroy_surface(surface);
        cx.quit();
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        self.pointer = event.position;
        match event.kind {
            PointerKind::Press(Button::Left) => {
                if self.close_rect().contains(event.position) {
                    cx.destroy_surface(surface);
                    cx.quit();
                } else if let Some(edge) = self.edge_at(event.position) {
                    tracing::info!(?edge, "boyutlandırma");
                    cx.begin_resize(surface, edge);
                } else if event.position.y < TITLE_H {
                    tracing::info!("taşıma");
                    cx.begin_move(surface);
                }
            }
            PointerKind::Press(Button::Right) if event.position.y < TITLE_H => {
                cx.set_maximized(surface, !self.state.maximized);
            }
            _ => {}
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if event.state == KeyState::Released {
            return;
        }
        match event.key {
            Key::Escape => {
                cx.destroy_surface(surface);
                cx.quit();
            }
            Key::Backspace => {
                self.typed.pop();
                cx.request_frame(surface);
            }
            _ => {
                if let Some(text) = event.text {
                    // Metnin kendisi loglanmaz (değişmez 9; SecretText `Value` değil).
                    tracing::info!(chars = text.chars().count(), "tuş metni");
                    self.typed.push_str(&text);
                    cx.request_frame(surface);
                }
            }
        }
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        _surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let theme = cx.theme();
        let c = &theme.colors;
        let full = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        painter.rect(full, &RectStyle::filled(c.surface, Corners::ZERO));

        let title_bg = if self.state.activated {
            c.surface_bright
        } else {
            c.surface_container
        };
        painter.rect(
            Rect::new(0.0, 0.0, self.size.width, TITLE_H),
            &RectStyle::filled(title_bg, Corners::ZERO),
        );
        let title_fg = if self.state.activated {
            c.on_surface
        } else {
            c.on_surface_variant
        };
        let style = TextStyle::new(
            theme.typography.ui.family.clone(),
            theme.typography.size,
            600,
        );
        let layout = painter
            .text_system()
            .layout("Nuxut pencere demosu", &style, None);
        let origin = Point::new(14.0, (TITLE_H - layout.size().height) / 2.0);
        painter.text(&layout, origin, title_fg);

        let close = self.close_rect();
        painter.rect(
            close,
            &RectStyle::filled(c.primary.with_alpha(0.85), theme.shape.small()),
        );

        // İçerik: durum bilgisi.
        let info = format!(
            "{} × {}   etkin: {}   büyütülmüş: {}   döşenmiş: {}",
            self.size.width,
            self.size.height,
            self.state.activated,
            self.state.maximized,
            self.state.tiled
        );
        let body = TextStyle::new(
            theme.typography.ui.family.clone(),
            theme.typography.size,
            theme.typography.ui.weight,
        );
        let layout = painter
            .text_system()
            .layout(&info, &body, Some(self.size.width - 28.0));
        painter.text(
            &layout,
            Point::new(14.0, TITLE_H + 16.0),
            c.on_surface_variant,
        );
        if !self.typed.is_empty() {
            let layout =
                painter
                    .text_system()
                    .layout(&self.typed, &body, Some(self.size.width - 28.0));
            painter.text(&layout, Point::new(14.0, TITLE_H + 72.0), c.on_surface);
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    if let Err(e) = nuxut_wayland::run(Demo::default()) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
