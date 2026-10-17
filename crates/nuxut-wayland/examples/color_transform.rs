//! Renk dönüşümü (`hyprland-ctm-control-v1`) yoklaması.
//!
//! Varsayılan (salt-okur): global ilan edildi mi, durum ne — yönetici BAĞLANMAZ, CTM'e
//! dokunulmaz (yalnızca birim matris istenir; yönetici yokken bu hiçbir şey göndermez).
//! Destek yoksa (sway) `set_color_transforms` `Unsupported` döner. Yüzey açmaz, hemen çıkar.
//!
//! `--apply`: tüm çıkışlara 3 sn sıcak matris uygular, sonra yöneticiyi bırakır (Hyprland
//! birime döndürür). Kullanıcının ekranını değiştirir; yalnızca elle, bilerek.

use std::time::{Duration, Instant};

use nuxut_wayland::{
    App, ColorMatrix, Context, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId, TimerId,
};

struct Probe {
    apply: bool,
    started: bool,
    outputs: Vec<OutputId>,
    start_timer: Option<TimerId>,
    release_timer: Option<TimerId>,
}

impl Probe {
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.started {
            return;
        }
        self.started = true;
        println!(
            "supported={} state={:?}",
            cx.color_transform_supported(),
            cx.color_transform_state()
        );
        let identity: Vec<_> = self
            .outputs
            .iter()
            .map(|&o| (o, ColorMatrix::IDENTITY))
            .collect();
        println!(
            "set(identity)={:?} state={:?}",
            cx.set_color_transforms(&identity),
            cx.color_transform_state()
        );
        if !self.apply {
            cx.quit();
            return;
        }
        let warm: Vec<_> = self
            .outputs
            .iter()
            .map(|&o| (o, ColorMatrix::diagonal(1.0, 0.78, 0.56)))
            .collect();
        println!(
            "set(warm)={:?} state={:?}",
            cx.set_color_transforms(&warm),
            cx.color_transform_state()
        );
        self.release_timer = Some(cx.set_timer(Duration::from_secs(3)));
    }
}

impl App for Probe {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        self.outputs.push(output.id);
        // Açılıştaki tüm çıkışlar aynı turda gelir; kısa gecikmeyle hepsini topla.
        if !self.started {
            self.start_timer
                .get_or_insert_with(|| cx.set_timer(Duration::from_millis(100)));
        }
    }

    fn output_removed(&mut self, _: &mut Context<Self>, output: OutputId) {
        self.outputs.retain(|&o| o != output);
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if Some(id) == self.start_timer {
            self.start(cx);
        } else if Some(id) == self.release_timer {
            cx.color_transform_release();
            println!("release state={:?}", cx.color_transform_state());
            cx.quit();
        }
    }

    fn color_transform_blocked(&mut self, cx: &mut Context<Self>) {
        println!("blocked state={:?}", cx.color_transform_state());
        cx.quit();
    }

    fn pointer(&mut self, _: &mut Context<Self>, _: SurfaceId, _: PointerEvent) {}

    fn draw(&mut self, _: &mut Context<Self>, _: SurfaceId, _: Instant, _: &mut Painter) {}
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let apply = std::env::args().any(|a| a == "--apply");
    let probe = Probe {
        apply,
        started: false,
        outputs: Vec::new(),
        start_timer: None,
        release_timer: None,
    };
    if let Err(e) = nuxut_wayland::run(probe) {
        eprintln!("hata: {e}");
        std::process::exit(1);
    }
}
