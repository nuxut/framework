//! Çıkışı compositor'a bırakılmış layer yüzeyi demosu (`LayerSurfaceSpec::on_focused_output`).
//! YALNIZCA test compositor'ında çalıştırın (`tools/test-compositor.sh`).
//!
//! İlk çıkış gelince ortada bir kart açılır; `get_layer_surface`'e çıkış verilmez, compositor
//! odaklı çıkışı seçer. `surface_output_changed` (wl_surface.enter) ile öğrenilen çıkış ve
//! ölçek loglanır. Yüzeyin çıkışı kaldırılırsa compositor yüzeyi kapatır; demo bunu
//! `output_removed`'da fark edip yeni odaklı çıkışta yeniden açar (OSD/diyalog kalıbı).
//! Her `output_added`/`output_changed` mantıksal boyutu, geçerli kipin fiziksel boyutunu ve
//! yenileme hızını loglar.

use std::collections::HashMap;
use std::time::Instant;

use nuxut_core::{Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    App, Context, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId,
};

const NAMESPACE: &str = "nuxut-focused-output-demo";

#[derive(Default)]
struct Demo {
    surface: Option<(SurfaceId, Size)>,
    /// Bağlayıcı adları (log için).
    names: HashMap<OutputId, String>,
}

impl Demo {
    fn open(&mut self, cx: &mut Context<Self>) {
        let mut spec = LayerSurfaceSpec::on_focused_output(NAMESPACE);
        spec.layer = Layer::Overlay;
        spec.size = (320, 160);
        let id = cx.create_layer_surface(spec);
        tracing::info!(?id, "yüzey açıldı (çıkış: compositor seçer)");
        self.surface = Some((id, Size::ZERO));
    }

    fn name(&self, output: Option<OutputId>) -> &str {
        output
            .and_then(|o| self.names.get(&o))
            .map_or("-", String::as_str)
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        log_output("output_added", output);
        self.names.insert(output.id, output.name.clone());
        if self.surface.is_none() {
            self.open(cx);
        }
    }

    fn output_changed(&mut self, _cx: &mut Context<Self>, output: &OutputInfo) {
        log_output("output_changed", output);
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        let name = self.names.remove(&output);
        tracing::info!(output = name.as_deref().unwrap_or("-"), "çıkış kaldırıldı");
        // Compositor çıkışla birlikte yüzeyi kapattıysa (closed) yüzey artık yok.
        if let Some((id, _)) = self.surface
            && cx.surface_scale(id).is_none()
        {
            tracing::info!(
                ?id,
                "yüzey compositor tarafından kapatıldı; yeniden açılıyor"
            );
            self.surface = None;
            if !self.names.is_empty() {
                self.open(cx);
            }
        }
    }

    fn surface_output_changed(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        output: Option<OutputId>,
    ) {
        tracing::info!(
            ?surface,
            output = self.name(output),
            scale = ?cx.surface_scale(surface),
            matches_query = (cx.surface_output(surface) == output),
            "surface_output_changed"
        );
    }

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        scale: f32,
    ) {
        let Some((id, s)) = self.surface.as_mut() else {
            return;
        };
        if *id != surface {
            return;
        }
        *s = size;
        let output = cx.surface_output(surface);
        tracing::info!(?size, scale, output = self.name(output), "surface_resized");
        cx.request_frame(surface);
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let Some((id, size)) = self.surface else {
            return;
        };
        if id != surface {
            return;
        }
        tracing::info!(scale = ?cx.surface_scale(surface), "draw");
        let theme = cx.theme();
        painter.rect(
            Rect::new(0.0, 0.0, size.width, size.height),
            &RectStyle::filled(theme.colors.primary, theme.shape.large()),
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
}

/// Mantıksal (`size`, xdg-output) ile fiziksel (`physical_size`, geçerli kip) boyutu yan yana
/// loglar; test compositor'ında `swaymsg output HEADLESS-1 scale 1.5` farkı gösterir.
fn log_output(what: &str, o: &OutputInfo) {
    tracing::info!(
        name = %o.name,
        logical = ?(o.size.width, o.size.height),
        physical = ?o.physical_size,
        mode = ?o.mode_size,
        scale = o.scale,
        refresh = o.refresh,
        "{what}"
    );
}
