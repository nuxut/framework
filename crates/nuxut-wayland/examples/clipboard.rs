//! Pano demosu (wl_data_device + primary selection). YALNIZCA test compositor'ında
//! çalıştırın (`tools/test-compositor.sh`): `l` oturumu kısa süre kilitler.
//!
//! Klavye-özel bir layer yüzeyi açılır. Tuşlar:
//! - `c`: `NUXUT_CLIP_COPY` (yoksa sabit örnek) panoya ve birincil seçime yazılır.
//! - `v` / `p`: panodan / birincil seçimden yapıştırma istenir; sonuç `NUXUT_CLIP_EXPECT` ile
//!   karşılaştırılır.
//! - `l`: kilitlenir; kilitliyken kopyalama ve yapıştırma `Locked` olmalı; kilit açılır.
//! - `q`: çıkış.
//!
//! İçerik loglanmaz (değişmez 9): yalnızca uzunluk, hata türü ve beklenenle eşleşme.

use std::collections::HashMap;
use std::time::Instant;

use nuxut_core::{Corners, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    Anchor, App, ClipboardText, Context, Key, KeyEvent, KeyState, KeyboardMode, Layer,
    LayerSurfaceSpec, LockEnd, OutputId, OutputInfo, Painter, PasteError, PasteRequestId,
    PointerEvent, Selection, SurfaceId,
};

const SAMPLE: &str = "nuxut pano örneği ✓";

#[derive(Default)]
struct Demo {
    surface: Option<(SurfaceId, Size)>,
    lock_surfaces: HashMap<SurfaceId, Size>,
    requests: HashMap<PasteRequestId, Selection>,
    locked_request: Option<PasteRequestId>,
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        let mut spec = LayerSurfaceSpec::new("clipboard-demo", output.id);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::TOP;
        spec.size = (240, 80);
        spec.keyboard = KeyboardMode::Exclusive;
        self.surface = Some((cx.create_layer_surface(spec), Size::ZERO));
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn keyboard(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: KeyEvent) {
        if event.state != KeyState::Pressed {
            return;
        }
        let Key::Char(c) = event.key else {
            return;
        };
        // Tuşun kendisi değil, yalnız değiştirici durumu (sanal klavye testleri için).
        tracing::info!(ctrl = event.modifiers.ctrl, shift = event.modifiers.shift, "tuş");
        match c {
            'c' => {
                let text = env_text("NUXUT_CLIP_COPY").unwrap_or_else(|| SAMPLE.to_owned());
                let clipboard = cx.set_clipboard(Selection::Clipboard, &text);
                let primary = cx.set_clipboard(Selection::Primary, &text);
                tracing::info!(?clipboard, ?primary, bytes = text.len(), "kopyalandı");
            }
            'v' | 'p' => {
                let selection = if c == 'v' {
                    Selection::Clipboard
                } else {
                    Selection::Primary
                };
                let id = cx.request_paste(selection);
                self.requests.insert(id, selection);
            }
            'l' => tracing::info!(result = ?cx.lock_session(), "lock_session"),
            'q' => cx.quit(),
            _ => {}
        }
    }

    fn paste(
        &mut self,
        cx: &mut Context<Self>,
        id: PasteRequestId,
        result: Result<ClipboardText, PasteError>,
    ) {
        if Some(id) == self.locked_request {
            tracing::info!(error = ?result.as_ref().err(), "kilitliyken yapıştırma sonucu");
            if let Err(e) = cx.unlock_session() {
                tracing::info!("unlock_session: {e:?}");
            }
            return;
        }
        let Some(selection) = self.requests.remove(&id) else {
            tracing::error!("bilinmeyen yapıştırma kimliği");
            return;
        };
        match result {
            Ok(text) => {
                let matches = env_text("NUXUT_CLIP_EXPECT").map(|expect| expect == text.as_str());
                tracing::info!(?selection, bytes = text.len(), ?matches, "yapıştırıldı");
            }
            Err(error) => tracing::info!(?selection, ?error, "yapıştırılamadı"),
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
        let copy = cx.set_clipboard(Selection::Clipboard, SAMPLE);
        tracing::info!(?copy, "kilitliyken kopyalama sonucu");
        self.locked_request = Some(cx.request_paste(Selection::Clipboard));
    }

    fn lock_finished(&mut self, _cx: &mut Context<Self>, end: LockEnd) {
        self.lock_surfaces.clear();
        tracing::info!(?end, "lock_finished");
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
