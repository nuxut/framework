//! Dosya seçici portalı demosu: pencere açılır, xdg-foreign-v2 ile dışa aktarılır ve
//! xdg-desktop-portal FileChooser diyaloğu pencereye ebeveynli açılır.
//!
//! - `o` aç (çoklu, resim filtresi), `d` dizin seç, `s` kaydet, `c` süren isteği iptal et,
//!   Escape çık.
//! - `NUXUT_FILE_CHOOSER_AUTO=1` (ya da saniye sayısı): pencere hazır olunca açma
//!   diyaloğu kendiliğinden açılır, 3 sn (ya da verilen süre) sonra iptal edilir (Request.Close) ve sonuç gelince demo çıkar (test akışı).
//!
//! Seçilen yollar loglanmaz (değişmez 9): yalnız sayı ve hata türü.
//!
//! ```sh
//! RUST_LOG=info cargo run -p nuxut-portal --example file_chooser
//! ```

use std::time::{Duration, Instant};

use nuxut_core::{Corners, Rect, Size};
use nuxut_portal::{
    Filter, OpenFileOptions, ParentWindow, Portal, PortalRequestId, PortalResponse, SaveFileOptions,
};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    App, Context, ExportRequestId, Key, KeyEvent, KeyState, OutputId, OutputInfo, Painter,
    PointerEvent, SurfaceId, TimerId, ToplevelHandle, WindowSpec,
};

struct Demo {
    window: Option<SurfaceId>,
    size: Size,
    portal: Option<Portal>,
    parent: Option<ParentWindow>,
    export: Option<ExportRequestId>,
    pending: Option<PortalRequestId>,
    /// `Some(sn)`: otomatik aç, `sn` saniye sonra iptal et, sonuçta çık.
    auto: Option<u64>,
    auto_cancel: Option<TimerId>,
}

impl Demo {
    fn open(&mut self, directory: bool) {
        let (Some(portal), Some(parent)) = (&self.portal, &self.parent) else {
            return;
        };
        let mut options = OpenFileOptions::new(if directory {
            "Klasör seç"
        } else {
            "Resim seç"
        });
        options.directory = directory;
        options.multiple = !directory;
        if !directory {
            options.filters = vec![
                Filter::new("Resimler").mime("image/*"),
                Filter::new("Tümü").glob("*"),
            ];
        }
        self.pending = Some(portal.open_file(parent, options));
        tracing::info!("açma diyaloğu istendi");
    }

    fn save(&mut self) {
        let (Some(portal), Some(parent)) = (&self.portal, &self.parent) else {
            return;
        };
        let mut options = SaveFileOptions::new("Yedeği kaydet");
        options.current_name = Some("nuxut-yedek.toml".into());
        self.pending = Some(portal.save_file(parent, options));
        tracing::info!("kaydetme diyaloğu istendi");
    }
}

impl App for Demo {
    type Message = PortalResponse;

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.window.is_none() {
            let spec = WindowSpec {
                size: (420, 240),
                ..WindowSpec::new("Nuxut dosya seçici demosu", "nuxut-file-chooser-demo")
            };
            self.window = Some(cx.create_window(spec));
            // Sonuçlar calloop kanalıyla `App::message`'a döner.
            let sender = cx.sender();
            self.portal = Some(Portal::new(move |response| {
                let _ = sender.send(response);
            }));
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        size: Size,
        _scale: f32,
    ) {
        self.size = size;
        if self.export.is_none() {
            self.export = Some(cx.export_toplevel(surface));
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
        tracing::info!(var = handle.is_some(), "pencere dışa aktarıldı");
        self.parent = Some(
            handle
                .map(|h| ParentWindow::wayland(h.as_str()))
                .unwrap_or_else(ParentWindow::none),
        );
        if let Some(secs) = self.auto {
            self.open(false);
            self.auto_cancel = Some(cx.set_timer(Duration::from_secs(secs)));
        }
    }

    fn timer(&mut self, _cx: &mut Context<Self>, id: TimerId) {
        if Some(id) == self.auto_cancel
            && let (Some(portal), Some(pending)) = (&self.portal, self.pending)
        {
            tracing::info!("otomatik iptal");
            portal.cancel(pending);
        }
    }

    fn message(&mut self, cx: &mut Context<Self>, response: PortalResponse) {
        match &response.result {
            Ok(paths) => tracing::info!(count = paths.len(), "seçildi"),
            Err(e) => tracing::info!(error = ?e, "seçim yok"),
        }
        if Some(response.id) == self.pending {
            self.pending = None;
        }
        if self.auto.is_some() {
            cx.quit();
        }
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn keyboard(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: KeyEvent) {
        if event.state != KeyState::Pressed {
            return;
        }
        match event.key {
            Key::Escape => cx.quit(),
            Key::Char('o') => self.open(false),
            Key::Char('d') => self.open(true),
            Key::Char('s') => self.save(),
            Key::Char('c') => {
                if let (Some(portal), Some(pending)) = (&self.portal, self.pending) {
                    portal.cancel(pending);
                }
            }
            _ => {}
        }
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        _surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let theme = cx.theme();
        let color = if self.pending.is_some() {
            theme.colors.primary
        } else {
            theme.colors.surface.with_alpha(1.0)
        };
        painter.rect(
            Rect::new(0.0, 0.0, self.size.width, self.size.height),
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
    // "1" → 3 sn; başka bir tamsayı → o kadar saniye.
    let auto = std::env::var("NUXUT_FILE_CHOOSER_AUTO")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(|n| if n == 1 { 3 } else { n });
    let demo = Demo {
        window: None,
        size: Size::ZERO,
        portal: None,
        parent: None,
        export: None,
        pending: None,
        auto,
        auto_cancel: None,
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
