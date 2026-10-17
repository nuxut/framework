//! Tek süreçte iki `xdg_toplevel`: ana pencere + sabit boyutlu "Hoş geldin" penceresi.
//!
//! - Açılışta ana pencere (`nuxut-two-windows`) ve hoş geldin penceresi (`nuxut-welcome`,
//!   en küçük = en büyük = 720×640; ipuçları yaratıldıktan sonra `set_min_size`/`set_max_size`
//!   ile verilir) açılır.
//! - Hoş geldin penceresinde Enter ya da "Bitti" düğmesi yalnızca o pencereyi kapatır.
//! - Ana pencerede `w` hoş geldin penceresini yeniden açar, `a` app-id'yi
//!   `nuxut-two-windows-renamed` yapar (`set_app_id`), `m` en büyük boyutu 600×400 ile
//!   sınırsız arasında değiştirir (çizilmiş pencerede `set_max_size`).
//! - Compositor kapatma isteği (`killactive`, sway `kill`) yalnızca o pencereyi kapatır;
//!   son pencere kapanınca uygulama kendisi çıkar (`cx.window_count() == 0` → `quit`).
//! - `DEMO_CYCLE_MS=<ms>`: hoş geldin penceresi bu aralıkla zamanlayıcıyla kapanıp yeniden açılır
//!   (`DEMO_CYCLES=<n>` tur sonra durur; varsayılan sınırsız). Girdisiz canlı test için.
//! - Her olay (boyut/ölçek, klavye odağı, tuş, tıklama) pencere adı + `SurfaceId` ile loglanır.
//!
//! ```sh
//! RUST_LOG=info cargo run -p nuxut-wayland --example two_windows
//! ```

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_render::{RectStyle, TextStyle};
use nuxut_wayland::{
    App, Button, Context, Key, KeyEvent, KeyState, OutputId, OutputInfo, Painter, PointerEvent,
    PointerKind, SurfaceId, TimerId, WindowSpec, WindowState,
};

/// Zamanlayıcıyla aç/kapa turu (`DEMO_CYCLE_MS`, `DEMO_CYCLES`).
struct Cycle {
    every: Duration,
    left: Option<u32>,
}

impl Cycle {
    fn from_env() -> Option<Self> {
        let ms = std::env::var("DEMO_CYCLE_MS").ok()?.parse::<u64>().ok()?;
        let left = std::env::var("DEMO_CYCLES")
            .ok()
            .and_then(|n| n.parse().ok());
        Some(Self {
            every: Duration::from_millis(ms.max(1)),
            left,
        })
    }
}

const WELCOME: (u32, u32) = (720, 640);
const MAIN_MAX: (u32, u32) = (600, 400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Main,
    Welcome,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Main => "ana",
            Self::Welcome => "hoş geldin",
        }
    }
}

struct Win {
    kind: Kind,
    size: Size,
    scale: f32,
    focused: bool,
    state: WindowState,
    typed: String,
}

impl Win {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            size: Size::new(0.0, 0.0),
            scale: 1.0,
            focused: false,
            state: WindowState::default(),
            typed: String::new(),
        }
    }

    /// Hoş geldin penceresindeki "Bitti" düğmesi.
    fn done_rect(&self) -> Rect {
        Rect::new(
            self.size.width - 160.0,
            self.size.height - 72.0,
            136.0,
            48.0,
        )
    }
}

#[derive(Default)]
struct Demo {
    started: bool,
    windows: HashMap<SurfaceId, Win>,
    main_limited: bool,
    cycle: Option<Cycle>,
}

impl Demo {
    fn welcome_id(&self) -> Option<SurfaceId> {
        self.windows
            .iter()
            .find(|(_, w)| w.kind == Kind::Welcome)
            .map(|(id, _)| *id)
    }

    fn arm_cycle(&mut self, cx: &mut Context<Self>) {
        if let Some(cycle) = &self.cycle
            && cycle.left != Some(0)
        {
            cx.set_timer(cycle.every);
        }
    }
}

impl Demo {
    fn open_main(&mut self, cx: &mut Context<Self>) {
        let spec = WindowSpec {
            size: (900, 600),
            min_size: Some((360, 240)),
            ..WindowSpec::new("Nuxut ana pencere", "nuxut-two-windows")
        };
        let id = cx.create_window(spec);
        tracing::info!(?id, "ana pencere yaratıldı");
        self.windows.insert(id, Win::new(Kind::Main));
    }

    fn open_welcome(&mut self, cx: &mut Context<Self>) {
        if self.windows.values().any(|w| w.kind == Kind::Welcome) {
            tracing::info!("hoş geldin penceresi zaten açık");
            return;
        }
        let spec = WindowSpec {
            size: WELCOME,
            ..WindowSpec::new("Nuxut'a hoş geldin", "nuxut-welcome")
        };
        let id = cx.create_window(spec);
        // Çalışma zamanı ipuçları (ilk tampondan önce: ilk karede geçerli).
        cx.set_min_size(id, Some(WELCOME));
        cx.set_max_size(id, Some(WELCOME));
        tracing::info!(?id, "hoş geldin penceresi yaratıldı");
        self.windows.insert(id, Win::new(Kind::Welcome));
    }

    fn close(&mut self, cx: &mut Context<Self>, id: SurfaceId, why: &str) {
        let kind = self.windows.remove(&id).map(|w| w.kind);
        cx.destroy_surface(id);
        let left = cx.window_count();
        tracing::info!(?id, ?kind, why, left, "pencere kapandı");
        if left == 0 {
            tracing::info!("son pencere kapandı; çıkılıyor");
            cx.quit();
        }
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if !self.started {
            self.started = true;
            self.open_main(cx);
            self.open_welcome(cx);
            self.arm_cycle(cx);
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, _id: TimerId) {
        if let Some(left) = self.cycle.as_mut().and_then(|c| c.left.as_mut()) {
            *left = left.saturating_sub(1);
        }
        match self.welcome_id() {
            Some(id) => self.close(cx, id, "zamanlayıcı"),
            None => self.open_welcome(cx),
        }
        self.arm_cycle(cx);
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        scale: f32,
    ) {
        if let Some(win) = self.windows.get_mut(&surface) {
            tracing::info!(
                ?surface,
                window = win.kind.name(),
                w = size.width,
                h = size.height,
                scale,
                "boyut"
            );
            win.size = size;
            win.scale = scale;
        }
        cx.request_frame(surface);
    }

    fn window_state_changed(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        state: WindowState,
    ) {
        if let Some(win) = self.windows.get_mut(&surface) {
            tracing::info!(
                ?surface,
                window = win.kind.name(),
                activated = state.activated,
                "durum"
            );
            win.state = state;
        }
        cx.request_frame(surface);
    }

    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        if let Some(win) = self.windows.get_mut(&surface) {
            tracing::info!(?surface, window = win.kind.name(), focused, "klavye odağı");
            win.focused = focused;
        }
        cx.request_frame(surface);
    }

    fn close_requested(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        self.close(cx, surface, "compositor kapatma isteği");
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        let Some(win) = self.windows.get(&surface) else {
            return;
        };
        if let PointerKind::Press(Button::Left) = event.kind {
            tracing::info!(
                ?surface,
                window = win.kind.name(),
                x = event.position.x,
                y = event.position.y,
                "tıklama"
            );
            if win.kind == Kind::Welcome && win.done_rect().contains(event.position) {
                self.close(cx, surface, "Bitti düğmesi");
            }
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if event.state == KeyState::Released {
            return;
        }
        let Some(win) = self.windows.get_mut(&surface) else {
            return;
        };
        // Değişmez 9: tuş/metin içeriği loglanmaz; yalnızca yönlendirme (hangi pencere).
        tracing::info!(?surface, window = win.kind.name(), "tuş");
        match (win.kind, &event.key) {
            (Kind::Welcome, Key::Enter) => self.close(cx, surface, "Enter"),
            (Kind::Main, Key::Char('w')) => self.open_welcome(cx),
            (Kind::Main, Key::Char('a')) => {
                cx.set_app_id(surface, "nuxut-two-windows-renamed");
                tracing::info!(?surface, "app-id değişti");
            }
            (Kind::Main, Key::Char('m')) => {
                self.main_limited = !self.main_limited;
                let max = self.main_limited.then_some(MAIN_MAX);
                cx.set_max_size(surface, max);
                tracing::info!(?surface, ?max, "ana pencere en büyük boyutu");
            }
            _ => {
                if let Some(text) = event.text {
                    win.typed.push_str(&text);
                    cx.request_frame(surface);
                }
            }
        }
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let Some(win) = self.windows.get(&surface) else {
            return;
        };
        let theme = cx.theme();
        let c = &theme.colors;
        let (w, h) = (win.size.width, win.size.height);
        let bg = match win.kind {
            Kind::Main => c.surface,
            Kind::Welcome => c.surface_container,
        };
        painter.rect(
            Rect::new(0.0, 0.0, w, h),
            &RectStyle::filled(bg, Corners::ZERO),
        );
        // Klavye odağı: üstte vurgu şeridi.
        let strip = if win.focused { c.primary } else { c.outline };
        painter.rect(
            Rect::new(0.0, 0.0, w, 8.0),
            &RectStyle::filled(strip, Corners::ZERO),
        );

        let title_style = TextStyle::new(
            theme.typography.ui.family.clone(),
            theme.typography.size * 1.6,
            600,
        );
        let body = TextStyle::new(
            theme.typography.ui.family.clone(),
            theme.typography.size,
            theme.typography.ui.weight,
        );
        let title = match win.kind {
            Kind::Main => "Ana pencere",
            Kind::Welcome => "Hoş geldin",
        };
        let layout = painter.text_system().layout(title, &title_style, None);
        painter.text(&layout, Point::new(24.0, 28.0), c.on_surface);

        let info = format!(
            "{surface:?}   {w} × {h}   ölçek {:.2}\nklavye odağı: {}   etkin: {}\nyazılan: {}",
            win.scale,
            if win.focused { "evet" } else { "hayır" },
            win.state.activated,
            win.typed,
        );
        let layout = painter.text_system().layout(&info, &body, Some(w - 48.0));
        painter.text(&layout, Point::new(24.0, 84.0), c.on_surface_variant);

        if win.kind == Kind::Welcome {
            let done = win.done_rect();
            painter.rect(done, &RectStyle::filled(c.primary, theme.shape.small()));
            let label = painter.text_system().layout("Bitti", &body, None);
            let size = label.size();
            painter.text(
                &label,
                Point::new(
                    done.x + (done.width - size.width) / 2.0,
                    done.y + (done.height - size.height) / 2.0,
                ),
                c.on_primary,
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
        cycle: Cycle::from_env(),
        ..Demo::default()
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
