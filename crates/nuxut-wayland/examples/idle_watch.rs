//! Boşta bildirimi demosu (ext-idle-notify-v1). YALNIZCA test compositor'ında çalıştırın
//! (`tools/test-compositor.sh`); girdi o display'e `wtype` ile verilir.
//!
//! Yüzey açmaz. İlk çıkışta izlemeler kurar: kısa 2 sn, uzun 60 sn (Respect) ve sürüm 2 ise
//! inhibitor'ları yok sayan 2 sn (Ignore). Akış:
//! 1. kısa `idle=true` → `idle_set_timeout(kısa, 1 sn)` (ayar değişimi): anında `idle=false`
//!    (yeniden kayıt), girdi yokken 1 sn sonra yine `idle=true` (`REARMED-IDLE` logu).
//! 2. Bu noktada dışarıdan girdi (`wtype`) → kısa `idle=false` → izlemeler bırakılır
//!    (unwatch iki kez — idempotent) ve `DEMO-OK` ile çıkılır.
//!
//! 30 sn içinde bitmezse hata koduyla çıkar.

use std::time::{Duration, Instant};

use nuxut_wayland::{
    App, Context, IdleInhibitors, IdleSupport, IdleWatchId, OutputId, OutputInfo, Painter,
    PointerEvent, SurfaceId, TimerId,
};

#[derive(Default)]
struct Demo {
    started: bool,
    short: Option<IdleWatchId>,
    long: Option<IdleWatchId>,
    input: Option<IdleWatchId>,
    /// 0: ilk boşta bekleniyor, 1: yeniden kayıt sonrası false bekleniyor,
    /// 2: yeniden boşta bekleniyor, 3: girdi bekleniyor.
    phase: u8,
    done: bool,
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.started {
            return;
        }
        self.started = true;
        let support = cx.idle_support();
        tracing::info!(?support, "idle_support");
        let IdleSupport::Supported { ignore_inhibitors } = support else {
            tracing::error!("protokol yok");
            std::process::exit(3);
        };
        self.short = cx.idle_watch(Duration::from_secs(2)).ok();
        self.long = cx.idle_watch(Duration::from_secs(60)).ok();
        let input = cx.idle_watch_with(Duration::from_secs(2), IdleInhibitors::Ignore);
        tracing::info!(ignore_inhibitors, input = ?input, "Ignore izlemesi");
        self.input = input.ok();
        cx.set_timer(Duration::from_secs(30));
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn idle(&mut self, cx: &mut Context<Self>, id: IdleWatchId, idle: bool) {
        let which = if Some(id) == self.short {
            "kısa"
        } else if Some(id) == self.long {
            "uzun"
        } else if Some(id) == self.input {
            "girdi"
        } else {
            "bilinmeyen"
        };
        tracing::info!(which, idle, phase = self.phase, "IDLE-EVENT");
        if self.done {
            // Sözleşme: idle_unwatch sonrası (aynı olay turunda kuyrukta olsa bile) olay gelmez.
            tracing::error!(which, "unwatch sonrası olay");
            std::process::exit(5);
        }
        if Some(id) != self.short {
            return;
        }
        match (self.phase, idle) {
            (0, true) => {
                self.phase = 1;
                let ok = cx.idle_set_timeout(id, Duration::from_secs(1));
                tracing::info!(ok, "kısa izleme 1 sn'ye yeniden kaydedildi");
            }
            (1, false) => self.phase = 2,
            (2, true) => {
                self.phase = 3;
                tracing::info!("REARMED-IDLE");
            }
            (3, false) => {
                cx.idle_unwatch(id);
                cx.idle_unwatch(id);
                for other in [self.long, self.input].into_iter().flatten() {
                    cx.idle_unwatch(other);
                }
                self.done = true;
                tracing::info!("DEMO-OK");
                cx.quit();
            }
            _ => {
                tracing::error!(phase = self.phase, idle, "beklenmeyen sıra");
                std::process::exit(4);
            }
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, _id: TimerId) {
        if !self.done {
            tracing::error!("DEMO-TIMEOUT");
            std::process::exit(2);
        }
        cx.quit();
    }

    fn draw(
        &mut self,
        _cx: &mut Context<Self>,
        _surface: SurfaceId,
        _now: Instant,
        _painter: &mut Painter,
    ) {
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,nuxut_wayland::idle=debug".into()),
        )
        .init();
    if let Err(e) = nuxut_wayland::run(Demo::default()) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
    tracing::info!("run döndü");
}
