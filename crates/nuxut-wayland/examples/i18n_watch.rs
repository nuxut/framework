//! Kullanıcı çeviri dizini izleme demosu (yüzey açmaz). YALNIZCA test compositor'ında ve
//! geçici `XDG_DATA_HOME` ile çalıştırın: `$XDG_DATA_HOME/nuxut/locales/demo/en.ftl` yazılınca
//! runtime katalogu yeniden kurar ve `locale_changed` loglanır (`demo-hello` metniyle; demo
//! kendi sabit metnini loglar, kullanıcı içeriği değil).

use std::time::Instant;

use nuxut_wayland::{App, Context, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId};

const SRC: nuxut_i18n::Source = nuxut_i18n::Source {
    domain: "demo",
    embedded: &[("en", "demo-hello = Hello\n")],
};

struct Demo {
    changes: u32,
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, _cx: &mut Context<Self>, _output: &OutputInfo) {}
    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}
    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn locale_changed(&mut self, _cx: &mut Context<Self>) {
        self.changes += 1;
        let text = nuxut_i18n::current().text("demo-hello").into_owned();
        tracing::info!(changes = self.changes, text, "locale_changed");
    }

    fn draw(&mut self, _cx: &mut Context<Self>, _s: SurfaceId, _now: Instant, _p: &mut Painter) {}
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    nuxut_i18n::init(&[SRC]);
    tracing::info!(text = %nuxut_i18n::current().text("demo-hello"), "başladı");
    if let Err(e) = nuxut_wayland::run(Demo { changes: 0 }) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
