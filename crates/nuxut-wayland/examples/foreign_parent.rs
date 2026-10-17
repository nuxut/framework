//! xdg-foreign-v2 içe aktarma: tek süreçte iki pencere. Ebeveyn pencere dışa aktarılır, çocuk
//! pencere tanıtıcıyı `set_window_parent` ile içe aktarıp ebeveyninin çocuğu olur (portal
//! dosya seçicisinin kendi diyaloğunu uygulamanın penceresine bağladığı yol).
//!
//! Akış (girdisiz): çocuk ebeveynlenir → 500 ms sonra `has_window_parent` loglanır → ebeveyn
//! kapatılır → 500 ms sonra yeniden loglanır (`destroyed` geldiyse `false`) → çıkış.
//!
//! ```sh
//! RUST_LOG=info cargo run -p nuxut-wayland --example foreign_parent
//! ```

use std::time::{Duration, Instant};

use nuxut_core::{Corners, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    App, Context, ExportRequestId, KeyEvent, OutputId, OutputInfo, Painter, PointerEvent,
    SurfaceId, TimerId, ToplevelHandle, WindowSpec,
};

#[derive(Default)]
struct Demo {
    parent: Option<SurfaceId>,
    child: Option<SurfaceId>,
    exported: bool,
    child_ready: bool,
    handle: Option<ToplevelHandle>,
    step: u8,
    sizes: Vec<(SurfaceId, Size)>,
}

impl Demo {
    fn try_link(&mut self, cx: &mut Context<Self>) {
        let (Some(child), Some(handle)) = (self.child, self.handle.as_ref()) else {
            return;
        };
        if !self.child_ready || self.step != 0 {
            return;
        }
        let ok = cx.set_window_parent(child, handle.as_str());
        tracing::info!(ok, "çocuk ebeveynlendi");
        self.step = 1;
        cx.set_timer(Duration::from_millis(500));
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.parent.is_none() {
            self.parent = Some(cx.create_window(WindowSpec {
                size: (480, 320),
                ..WindowSpec::new("Ebeveyn", "nuxut-foreign-parent")
            }));
            self.child = Some(cx.create_window(WindowSpec {
                size: (240, 160),
                ..WindowSpec::new("Çocuk", "nuxut-foreign-child")
            }));
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, _: f32) {
        self.sizes.retain(|(id, _)| *id != surface);
        self.sizes.push((surface, size));
        if Some(surface) == self.parent && !self.exported {
            self.exported = true;
            cx.export_toplevel(surface);
        }
        if Some(surface) == self.child {
            self.child_ready = true;
            self.try_link(cx);
        }
        cx.request_frame(surface);
    }

    fn toplevel_exported(
        &mut self,
        cx: &mut Context<Self>,
        _id: ExportRequestId,
        _surface: SurfaceId,
        handle: Option<ToplevelHandle>,
    ) {
        tracing::info!(var = handle.is_some(), "ebeveyn dışa aktarıldı");
        match handle {
            Some(handle) => {
                self.handle = Some(handle);
                self.try_link(cx);
            }
            None => cx.quit(),
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, _id: TimerId) {
        let Some(child) = self.child else { return };
        tracing::info!(step = self.step, parent = cx.has_window_parent(child), "durum");
        match self.step {
            1 => {
                if let Some(parent) = self.parent.take() {
                    cx.destroy_surface(parent);
                }
                self.step = 2;
                cx.set_timer(Duration::from_millis(500));
            }
            _ => cx.quit(),
        }
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn keyboard(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: KeyEvent) {}

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, _now: Instant, painter: &mut Painter) {
        let theme = cx.theme();
        let color = if Some(surface) == self.child {
            theme.colors.primary
        } else {
            theme.colors.surface.with_alpha(1.0)
        };
        let size = self
            .sizes
            .iter()
            .find(|(id, _)| *id == surface)
            .map_or(Size::ZERO, |(_, s)| *s);
        painter.rect(
            Rect::new(0.0, 0.0, size.width, size.height),
            &RectStyle::filled(color, Corners::ZERO),
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
        tracing::error!(error = %e, "çalışma zamanı hatası");
        std::process::exit(1);
    }
}
