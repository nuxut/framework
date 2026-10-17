//! Odak kapma demosu (`hyprland_focus_grab_v1`). Kısa ve kendi kendine yeten bir senaryo;
//! girdi gerekmez, kullanıcının pencerelerine hiçbir şey gönderilmez.
//!
//! Küçük bir overlay layer yüzeyi (klavye `None`) açılır ve hemen grab istenir — yüzey henüz
//! çizilmediği için grab ilk tampondan sonra kurulur (ertelenmiş kurulum). Sonra aynı yüzeyle
//! ikinci bir grab kurulur: compositor yeni grab başlayınca eskisini bitirir, birinci grab için
//! `focus_grab_cleared` gelmeli. Ardından yüzey yok edilir (ikinci grab kendiliğinden
//! bırakılır, `cleared` GELMEMELİ) ve demo çıkar. Grab toplam ~0.6 s sürer; bu sürede klavye
//! ve işaretçi odağı demo yüzeyindedir (protokol gereği), sonra compositor normal odağa döner.
//!
//! Protokolü sunmayan compositor'da (sway) `focus_grab` `None` döner ve demo hemen çıkar.

use std::time::{Duration, Instant};

use nuxut_core::{Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    Anchor, App, Context, FocusGrabId, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo,
    Painter, PointerEvent, SurfaceId, TimerId,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Birinci grab kuruldu (ilk tamponu bekliyor olabilir).
    First,
    /// İkinci grab kuruldu; birincinin `cleared`'ı bekleniyor.
    Second,
    /// Yüzey yok edildi; `cleared` beklenmiyor.
    Destroyed,
}

#[derive(Default)]
struct Demo {
    surface: Option<(SurfaceId, Size)>,
    first: Option<FocusGrabId>,
    second: Option<FocusGrabId>,
    step: Option<Step>,
    timer: Option<TimerId>,
    cleared: Vec<FocusGrabId>,
}

impl Demo {
    fn after(&mut self, cx: &mut Context<Self>, ms: u64) {
        self.timer = Some(cx.set_timer(Duration::from_millis(ms)));
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        let mut spec = LayerSurfaceSpec::new("nuxut-focus-grab-demo", output.id);
        spec.layer = Layer::Overlay;
        spec.anchor = Anchor::TOP | Anchor::RIGHT;
        spec.size = (96, 32);
        spec.keyboard = KeyboardMode::None;
        let surface = cx.create_layer_surface(spec);
        self.surface = Some((surface, Size::ZERO));
        tracing::info!(supported = cx.focus_grab_supported(), "odak kapma desteği");
        self.first = cx.focus_grab(&[surface]);
        match self.first {
            Some(id) => {
                tracing::info!(
                    ?id,
                    active = cx.focus_grab_active(id),
                    "birinci grab istendi (çizimden önce: etkin olmamalı)"
                );
                self.step = Some(Step::First);
                self.after(cx, 300);
            }
            None => {
                tracing::info!("focus_grab None (protokol yok); çıkılıyor");
                cx.quit();
            }
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if Some(id) != self.timer {
            return;
        }
        let Some((surface, _)) = self.surface else {
            return;
        };
        match self.step {
            Some(Step::First) => {
                let first = self.first.expect("birinci grab");
                tracing::info!(
                    active = cx.focus_grab_active(first),
                    "birinci grab (çizimden sonra)"
                );
                self.second = cx.focus_grab(&[surface]);
                tracing::info!(
                    second = ?self.second,
                    active = self.second.is_some_and(|g| cx.focus_grab_active(g)),
                    "ikinci grab kuruldu; birincinin cleared'ı bekleniyor"
                );
                self.step = Some(Step::Second);
                self.after(cx, 300);
            }
            Some(Step::Second) => {
                let first = self.first.expect("birinci grab");
                tracing::info!(
                    first_cleared = self.cleared.contains(&first),
                    first_active = cx.focus_grab_active(first),
                    "ikinci aşama sonu"
                );
                cx.destroy_surface(surface);
                let second = self.second.expect("ikinci grab");
                tracing::info!(
                    second_active = cx.focus_grab_active(second),
                    "yüzey yok edildi (ikinci grab bırakılmış olmalı)"
                );
                self.step = Some(Step::Destroyed);
                self.after(cx, 300);
            }
            Some(Step::Destroyed) => {
                let second_cleared = self.second.is_some_and(|g| self.cleared.contains(&g));
                tracing::info!(
                    cleared = self.cleared.len(),
                    second_cleared,
                    "bitti (beklenen: cleared = 1, second_cleared = false)"
                );
                for id in [self.first, self.second].into_iter().flatten() {
                    cx.focus_grab_release(id);
                }
                cx.quit();
            }
            None => {}
        }
    }

    fn focus_grab_cleared(&mut self, _cx: &mut Context<Self>, id: FocusGrabId) {
        let which = if Some(id) == self.first {
            "birinci"
        } else if Some(id) == self.second {
            "ikinci"
        } else {
            "bilinmeyen"
        };
        tracing::info!(?id, which, step = ?self.step, "focus_grab_cleared");
        self.cleared.push(id);
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
            cx.request_frame(surface);
        }
    }

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
    tracing::info!("run döndü");
}
