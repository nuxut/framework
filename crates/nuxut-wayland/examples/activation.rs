//! Etkinleştirme belirteci demosu (xdg-activation-v1). YALNIZCA test compositor'ında
//! çalıştırın (`tools/test-compositor.sh`): demo oturumu kısa süre kilitler.
//!
//! Klavye-özel bir layer yüzeyi açılır; ilk tuş basışında belirteç istenir (gelmeli). Sonra
//! oturum kilitlenir, kilitliyken istenen belirteç `None` olmalı; kilit açılır ve demo çıkar.
//! Belirteç loglanmaz (değişmez 9): yalnızca var/yok ve uzunluk.

use std::collections::HashMap;
use std::time::Instant;

use nuxut_core::{Corners, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    ActivationRequestId, ActivationToken, Anchor, App, Context, KeyEvent, KeyState, KeyboardMode,
    Layer, LayerSurfaceSpec, LockEnd, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId,
};

#[derive(Default)]
struct Demo {
    surface: Option<(SurfaceId, Size)>,
    lock_surfaces: HashMap<SurfaceId, Size>,
    /// Kilitsiz istek (belirteç beklenir) ve kilitliyken istek (`None` beklenir).
    unlocked_request: Option<ActivationRequestId>,
    locked_request: Option<ActivationRequestId>,
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        let mut spec = LayerSurfaceSpec::new("activation-demo", output.id);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::TOP;
        spec.size = (240, 80);
        spec.keyboard = KeyboardMode::Exclusive;
        self.surface = Some((cx.create_layer_surface(spec), Size::ZERO));
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if event.state == KeyState::Pressed && self.unlocked_request.is_none() {
            let id = cx.request_activation_token(surface, Some("nuxut-activation-demo"));
            self.unlocked_request = Some(id);
            tracing::info!("belirteç istendi (kilitsiz)");
        }
    }

    fn activation_token(
        &mut self,
        cx: &mut Context<Self>,
        id: ActivationRequestId,
        token: Option<ActivationToken>,
    ) {
        let len = token.as_ref().map(|t| t.as_str().len());
        if Some(id) == self.unlocked_request {
            tracing::info!(var = token.is_some(), ?len, "kilitsiz istek sonucu");
            tracing::info!(result = ?cx.lock_session(), "lock_session");
        } else if Some(id) == self.locked_request {
            tracing::info!(var = token.is_some(), "kilitliyken istek sonucu");
            if let Err(e) = cx.unlock_session() {
                tracing::info!("unlock_session: {e:?}");
            }
        } else {
            tracing::error!("bilinmeyen istek kimliği");
        }
    }

    fn lock_surface_added(
        &mut self,
        _cx: &mut Context<Self>,
        surface: SurfaceId,
        _output: OutputId,
    ) {
        self.lock_surfaces.insert(surface, Size::ZERO);
    }

    fn session_locked(&mut self, cx: &mut Context<Self>) {
        if let Some((surface, _)) = self.surface {
            self.locked_request = Some(cx.request_activation_token(surface, None));
        }
    }

    fn lock_finished(&mut self, cx: &mut Context<Self>, end: LockEnd) {
        self.lock_surfaces.clear();
        tracing::info!(?end, "lock_finished");
        cx.quit();
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
        } else if let Some(s) = self.lock_surfaces.get_mut(&surface) {
            *s = size;
        } else {
            return;
        }
        cx.request_frame(surface);
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let theme = cx.theme();
        // Kilit yüzeyleri opak zeminle; demo yüzeyi birincil renkle.
        let (color, corners, size) = match self.surface {
            Some((id, size)) if id == surface => (theme.colors.primary, theme.shape.large(), size),
            _ => match self.lock_surfaces.get(&surface) {
                Some(size) => (theme.colors.surface.with_alpha(1.0), Corners::ZERO, *size),
                None => return,
            },
        };
        painter.rect(
            Rect::new(0.0, 0.0, size.width, size.height),
            &RectStyle::filled(color, corners),
        );
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
    tracing::info!("run döndü");
}
