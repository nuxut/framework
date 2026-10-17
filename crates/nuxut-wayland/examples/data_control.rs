//! Pano izleme (data-control) demosu. YALNIZCA test compositor'ında çalıştırın
//! (`tools/test-compositor.sh`): gerçek oturumda kullanıcının panosunu okur ve değiştirir.
//!
//! Yüzey açmaz. Başlarken `data_control_start` + `data_control_watch_primary(true)`; her
//! yabancı (`own = false`) metin teklifinde:
//! 1. `cx.data_control_read` (kısa ömürlü thread, `App::data_control_read`) ve aynı anda bir
//!    thread'de `DataControlHandle::read` (shell servisinin yolu) — iki sonuç aynı olmalı;
//! 2. `max_bytes = 1` ile okuma → `TooLarge` (içerik 1 bayttan uzunsa);
//! 3. ana thread'den bloklayan okuma → `MainThread`;
//! 4. içerik `nuxut:` (birincil seçimde `nuxutp:`) önekiyle thread'den `handle.set_selection`
//!    ile aynı seçime geri konur; sonraki teklif `own = true` gelmeli ve önceki kimlikle okuma
//!    `Gone` dönmeli.
//!
//! Hassas (`sensitive`) teklifte yalnızca okuma denenir: `Sensitive` beklenir, geri konmaz.
//!
//! `NUXUT_DATA_CONTROL=wlr` her ikisi de varken wlr yedeğini seçtirir. `NUXUT_DC_QUIT_AFTER=N`:
//! N. kendi teklifinden sonra çıkış. İçerik loglanmaz: yalnızca uzunluk ve FNV özeti.

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut_wayland::{
    App, Context, DataControlEvent, DataControlHandle, DataOfferId, DataReadError, DataReadId,
    OutputId, OutputInfo, Painter, PointerEvent, Selection, SurfaceId, Zeroizing,
};

const MAX: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(2);
const TEXT_MIMES: [&str; 2] = ["text/plain;charset=utf-8", "text/plain"];

fn prefix(selection: Selection) -> &'static [u8] {
    match selection {
        Selection::Clipboard => b"nuxut:",
        Selection::Primary => b"nuxutp:",
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

enum Msg {
    ThreadRead(DataOfferId, Result<(usize, u64), DataReadError>),
}

#[derive(Default)]
struct Demo {
    started: bool,
    handle: Option<DataControlHandle>,
    last_foreign: Option<DataOfferId>,
    own_seen: u32,
    quit_after: u32,
}

impl Demo {
    fn on_offer(
        &mut self,
        cx: &mut Context<Self>,
        selection: Selection,
        id: DataOfferId,
        mimes: &[String],
    ) {
        let Some(handle) = self.handle.clone() else {
            return;
        };
        let Some(mime) = TEXT_MIMES
            .iter()
            .find(|m| mimes.iter().any(|x| x == *m))
            .map(|m| (*m).to_owned())
        else {
            tracing::info!(id = ?id, "metin türü yok; okunmadı");
            return;
        };
        let main = handle.read(id, &mime, MAX, TIMEOUT);
        tracing::info!(result = ?main.as_ref().map(|b| b.len()), "ana thread okuması");
        let rid = cx.data_control_read(id, &mime, MAX, TIMEOUT);
        tracing::info!(read = ?rid, "olaylı okuma istendi");
        let sender = cx.sender();
        std::thread::spawn(move || {
            let tiny = handle.read(id, &mime, 1, TIMEOUT);
            tracing::info!(result = ?tiny.as_ref().map(|b| b.len()), "1 bayt sınırlı okuma");
            let result = handle.read(id, &mime, MAX, TIMEOUT);
            let summary = result.as_ref().map(|b| (b.len(), fnv(b))).map_err(|e| *e);
            let _ = sender.send(Msg::ThreadRead(id, summary));
            if let Ok(bytes) = result {
                let mut next = prefix(selection).to_vec();
                next.extend_from_slice(&bytes);
                let next: Arc<[u8]> = Arc::from(next);
                let data = TEXT_MIMES
                    .iter()
                    .map(|m| ((*m).to_owned(), next.clone()))
                    .collect();
                let set = handle.set_selection(selection, data);
                tracing::info!(
                    ?selection,
                    ?set,
                    bytes = next.len(),
                    "seçim thread'den kondu"
                );
            }
        });
        if selection == Selection::Clipboard {
            self.last_foreign = Some(id);
        }
    }

    fn on_own(&mut self, cx: &mut Context<Self>, selection: Selection) {
        self.own_seen += 1;
        if selection == Selection::Clipboard
            && let (Some(old), Some(handle)) = (self.last_foreign, self.handle.clone())
        {
            std::thread::spawn(move || {
                let r = handle.read(old, TEXT_MIMES[0], MAX, TIMEOUT);
                tracing::info!(result = ?r.as_ref().map(|b| b.len()), "eski kimlikle okuma");
            });
        }
        if self.quit_after > 0 && self.own_seen >= self.quit_after {
            // Eski kimlik okumasına süre tanı, sonra çık.
            let _ = cx.set_timer(Duration::from_millis(300));
        }
    }
}

impl App for Demo {
    type Message = Msg;

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.started {
            return;
        }
        self.started = true;
        self.quit_after = std::env::var("NUXUT_DC_QUIT_AFTER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        tracing::info!(
            protocol = ?cx.data_control_protocol(),
            primary = cx.data_control_primary_supported(),
            "data-control"
        );
        match cx.data_control_start() {
            Ok(h) => {
                self.handle = Some(h);
                cx.data_control_watch_primary(true);
                tracing::info!("izleme başladı");
            }
            Err(e) => {
                tracing::error!(error = ?e, "izleme başlamadı");
                cx.quit();
            }
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn data_control(&mut self, cx: &mut Context<Self>, event: DataControlEvent) {
        match event {
            DataControlEvent::SelectionOffered {
                selection,
                id,
                mimes,
                own,
                sensitive,
                truncated,
            } => {
                tracing::info!(
                    ?selection,
                    id = ?id,
                    mimes = mimes.len(),
                    own,
                    sensitive,
                    truncated,
                    "teklif"
                );
                if own {
                    self.on_own(cx, selection);
                } else if sensitive {
                    if let Some(handle) = self.handle.clone() {
                        std::thread::spawn(move || {
                            let r = handle.read(id, TEXT_MIMES[1], MAX, TIMEOUT);
                            tracing::info!(result = ?r.as_ref().map(|b| b.len()), "hassas okuma");
                        });
                    }
                } else {
                    self.on_offer(cx, selection, id, &mimes);
                }
            }
            other => tracing::info!(event = ?other, "olay"),
        }
    }

    fn data_control_read(
        &mut self,
        _cx: &mut Context<Self>,
        id: DataReadId,
        result: Result<Zeroizing<Vec<u8>>, DataReadError>,
    ) {
        let summary = result.as_ref().map(|b| (b.len(), fnv(b)));
        tracing::info!(read = ?id, result = ?summary, "olaylı okuma sonucu");
    }

    fn message(&mut self, _cx: &mut Context<Self>, message: Msg) {
        let Msg::ThreadRead(id, summary) = message;
        tracing::info!(id = ?id, result = ?summary, "thread okuma sonucu");
    }

    fn timer(&mut self, cx: &mut Context<Self>, _id: nuxut_wayland::TimerId) {
        tracing::info!("çıkılıyor");
        cx.quit();
    }

    fn draw(&mut self, _: &mut Context<Self>, _: SurfaceId, _: Instant, _: &mut Painter) {}
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,nuxut_wayland=debug".into()),
        )
        .init();
    if let Err(e) = nuxut_wayland::run(Demo::default()) {
        eprintln!("hata: {e}");
        std::process::exit(1);
    }
}
