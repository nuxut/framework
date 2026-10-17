//! Oturum kilidi demosu (ext-session-lock-v1). YALNIZCA test compositor'ında çalıştırın
//! (`tools/test-compositor.sh`); kullanıcının oturumunu kilitlemeyin.
//!
//! `LOCK_DEMO_MODE`:
//! - `unlock` (varsayılan): kilitle → `locked` → 1.5 sn → `unlock_session` → çık.
//! - `hold`: kilitle ve bekle (SIGTERM ile kilitliyken çıkış testi; Enter kilidi açar).
//! - `twice`: iki kez `lock_session` (ikincisi `AlreadyActive` olmalı), sonra `hold`.
//! - `early`: `Pending`'deyken `unlock_session` (`NotLocked` olmalı), sonra `unlock`.
//!
//! Önce klavye-özel bir layer yüzeyi (`lock-demo-guard`) açılır, kilit `LOCK_DEMO_DELAY_MS`
//! (varsayılan 500) sonra istenir; kilit varken guard'a girdi ulaşırsa `SIZINTI` loglanır
//! (runtime savunma derinliği testi).
//!
//! Her yüzeyin ilk sunumu `surface_presented` (lock/guard) olarak loglanır.
//!
//! Kilit yüzeyleri opak zeminle (tema `surface`, alfa 1) boyanır. Loglar yalnızca durum
//! geçişleri ve sayılar (değişmez 9).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    Anchor, App, Context, Key, KeyEvent, KeyState, KeyboardMode, Layer, LayerSurfaceSpec, LockEnd,
    LockState, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId, TimerId,
};

#[derive(Default)]
struct Demo {
    mode: String,
    delay: Duration,
    requested: bool,
    /// Kilit dışı, klavye-özel layer yüzeyi ve boyutu.
    guard: Option<(SurfaceId, Size)>,
    surfaces: HashMap<SurfaceId, Size>,
    lock_timer: Option<TimerId>,
    unlock_timer: Option<TimerId>,
}

impl Demo {
    fn unlock(&mut self, cx: &mut Context<Self>) {
        match cx.unlock_session() {
            Ok(()) => tracing::info!("unlock_session: Ok"),
            Err(e) => tracing::info!("unlock_session: {e:?}"),
        }
    }

    fn lock(&mut self, cx: &mut Context<Self>) {
        tracing::info!(result = ?cx.lock_session(), state = ?cx.lock_state(), "lock_session");
        match self.mode.as_str() {
            "twice" => tracing::info!(result = ?cx.lock_session(), "ikinci lock_session"),
            "early" => self.unlock(cx),
            _ => {}
        }
    }

    /// Guard yüzeyine kilit varken girdi geldiyse bunu yüksek sesle bildirir.
    fn check_guard(&self, cx: &Context<Self>, surface: SurfaceId, what: &str) {
        if self.guard.map(|(id, _)| id) == Some(surface) && cx.lock_state() != LockState::Unlocked {
            tracing::error!(what, state = ?cx.lock_state(), "SIZINTI: kilit dışı yüzeye girdi");
        }
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.requested {
            return;
        }
        self.requested = true;
        let mut spec = LayerSurfaceSpec::new("lock-demo-guard", output.id);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::TOP;
        spec.size = (240, 80);
        spec.keyboard = KeyboardMode::Exclusive;
        self.guard = Some((cx.create_layer_surface(spec), Size::ZERO));
        if self.delay.is_zero() {
            // Aynı olay turunda: başlangıçta var olan diğer çıkışlar Pending'de eklenir.
            self.lock(cx);
        } else {
            self.lock_timer = Some(cx.set_timer(self.delay));
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn lock_surface_added(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _output: OutputId,
    ) {
        self.surfaces.insert(surface, Size::ZERO);
        tracing::info!(count = self.surfaces.len(), state = ?cx.lock_state(), "kilit yüzeyi eklendi");
    }

    fn session_locked(&mut self, cx: &mut Context<Self>) {
        tracing::info!(surfaces = self.surfaces.len(), state = ?cx.lock_state(), "session_locked");
        if matches!(self.mode.as_str(), "unlock" | "early") {
            self.unlock_timer = Some(cx.set_timer(Duration::from_millis(1500)));
        }
    }

    fn lock_finished(&mut self, cx: &mut Context<Self>, end: LockEnd) {
        tracing::info!(?end, state = ?cx.lock_state(), "lock_finished");
        self.surfaces.clear();
        // Kilit açıldıktan hemen sonra çıkmak güvenli olmalı (unlock_session sync bekler).
        cx.quit();
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.lock_timer == Some(id) {
            self.lock(cx);
        } else if self.unlock_timer == Some(id) {
            self.unlock(cx);
        }
    }

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        _scale: f32,
    ) {
        if let Some((id, s)) = self.guard.as_mut()
            && *id == surface
        {
            *s = size;
        } else if let Some(s) = self.surfaces.get_mut(&surface) {
            *s = size;
        } else {
            return;
        }
        cx.request_frame(surface);
    }

    fn surface_presented(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        let guard = self.guard.map(|(id, _)| id) == Some(surface);
        tracing::info!(
            lock = cx.is_lock_surface(surface),
            guard,
            state = ?cx.lock_state(),
            "surface_presented"
        );
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, _event: PointerEvent) {
        self.check_guard(cx, surface, "pointer");
    }

    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        tracing::info!(
            lock = cx.is_lock_surface(surface),
            focused,
            "keyboard_focus"
        );
        if focused {
            self.check_guard(cx, surface, "keyboard_focus");
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        self.check_guard(cx, surface, "keyboard");
        if event.state == KeyState::Pressed && cx.is_lock_surface(surface) {
            tracing::info!("kilit yüzeyine tuş");
            if event.key == Key::Enter {
                tracing::info!("Enter: kilit açılıyor");
                self.unlock(cx);
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
        let theme = cx.theme();
        if let Some((id, size)) = self.guard
            && id == surface
        {
            painter.rect(
                Rect::new(0.0, 0.0, size.width, size.height),
                &RectStyle::filled(theme.colors.primary, theme.shape.large()),
            );
            return;
        }
        let Some(size) = self.surfaces.get(&surface) else {
            return;
        };
        // Opak: kilitli oturumun altı görünmemeli.
        let bg = theme.colors.surface.with_alpha(1.0);
        painter.rect(
            Rect::new(0.0, 0.0, size.width, size.height),
            &RectStyle::filled(bg, Corners::ZERO),
        );
        let w = (size.width * 0.3).min(480.0);
        let card = Rect::new((size.width - w) / 2.0, size.height / 2.0 - 40.0, w, 80.0);
        painter.rect(
            card,
            &RectStyle::filled(
                theme.colors.surface_container.with_alpha(1.0),
                theme.shape.large(),
            ),
        );
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let delay = std::env::var("LOCK_DEMO_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(500);
    let demo = Demo {
        mode: std::env::var("LOCK_DEMO_MODE").unwrap_or_else(|_| "unlock".into()),
        delay: Duration::from_millis(delay),
        ..Default::default()
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
    tracing::info!("run döndü");
}
