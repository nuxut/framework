//! Pencere listesi + tek kare yakalama doğrulaması (yalnızca İZOLE test compositor'ında).
//!
//! Kendi penceresini açar (sol yarı `primary`, sağ yarı `error` tema rolü), pencere listesinden
//! kendi pencere kimliğini bulur, sonra şunları yakalar ve pikselleri BELLEKTE denetler (PNG ya
//! da başka dosya yazılmaz, piksel loglanmaz):
//! 1. kendi penceresi, tam boyut — boyut = pencerenin fiziksel boyutu, dört çeyrek rengi;
//! 2. kendi penceresi, 64×64 önizleme — en-boy korunmuş, çeyrek renkleri;
//! 3. çıkış, 256×256 önizleme (pencere çıkışı kaplar: `default_border none`) — çeyrek renkleri
//!    (çıkış döndürülmüşse görüntü dik gelmeli);
//! 4. iptal edilen yakalama — sonucu gelmemeli;
//! 5. kilit kapısı (`set_session_locked_hint`): yürüyen yakalama `Locked` ile biter, kilitliyken
//!    yeni istek `Locked`, kilit kalkınca yeni istek yine kare verir.
//!
//! Kullanıcının oturumunu korumak için `HYPRLAND_INSTANCE_SIGNATURE` tanımlıysa çalışmaz.
//!
//! ```sh
//! eval "$(tools/test-compositor.sh env)"
//! cargo run -p nuxut-wayland --example capture -- --globals   # ilgili global'leri dök
//! cargo run -p nuxut-wayland --example capture                # ext yolu
//! NUXUT_CAPTURE=wlr cargo run -p nuxut-wayland --example capture  # çıkış için wlr yedeği
//! ```

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nuxut_core::{Color, Corners, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_wayland::{
    App, CaptureError, CaptureId, CaptureOptions, CaptureSource, CapturedImage, Context, OutputId,
    OutputInfo, Painter, PointerEvent, SurfaceId, TimerId, ToplevelEvent, ToplevelId, WindowSpec,
};

const APP_ID: &str = "nuxut-capture-test";

static PASSES: AtomicUsize = AtomicUsize::new(0);
static FAILURES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    WindowFull,
    WindowPreview,
    OutputPreview,
    Cancelled,
    LockAborted,
    LockRefused,
    AfterUnlock,
}

#[derive(Default)]
struct Probe {
    window: Option<SurfaceId>,
    output: Option<OutputId>,
    size: Size,
    scale: f32,
    presented: bool,
    toplevel: Option<ToplevelId>,
    started: bool,
    start_timer: Option<TimerId>,
    deadline: Option<TimerId>,
    pending: Vec<(CaptureId, Check)>,
    colors: Option<([u8; 3], [u8; 3])>,
    lock_phase: bool,
}

fn rgb8(c: Color) -> [u8; 3] {
    [c.r, c.g, c.b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl Probe {
    fn maybe_start(&mut self, cx: &mut Context<Self>) {
        if self.started || !self.presented || self.toplevel.is_none() || self.output.is_none() {
            return;
        }
        self.started = true;
        // Kare compositor'da gerçekten ekranda olsun diye kısa bekleme.
        self.start_timer = Some(cx.set_timer(Duration::from_millis(300)));
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let (Some(toplevel), Some(output)) = (self.toplevel, self.output) else {
            return;
        };
        println!("support={:?}", cx.capture_support());
        let t = Instant::now();
        let full = cx.capture(CaptureSource::Toplevel(toplevel), CaptureOptions::default());
        let preview = cx.capture(
            CaptureSource::Toplevel(toplevel),
            CaptureOptions::preview(64, 64),
        );
        let out = cx.capture(
            CaptureSource::Output(output),
            CaptureOptions::preview(256, 256),
        );
        let cancelled = cx.capture(
            CaptureSource::Output(output),
            CaptureOptions::preview(32, 32),
        );
        cx.capture_cancel(cancelled);
        self.pending = vec![
            (full, Check::WindowFull),
            (preview, Check::WindowPreview),
            (out, Check::OutputPreview),
            (cancelled, Check::Cancelled),
        ];
        println!("istekler gönderildi ({:?})", t.elapsed());
        self.deadline = Some(cx.set_timer(Duration::from_secs(10)));
    }

    /// Kilit kapısı: yürüyen iş kilitle düşer, kilitliyken yeni istek reddedilir, kilit kalkınca
    /// yakalama yeniden çalışır.
    fn start_lock_phase(&mut self, cx: &mut Context<Self>) {
        let Some(output) = self.output else {
            return;
        };
        self.lock_phase = true;
        let preview = CaptureOptions::preview(32, 32);
        let aborted = cx.capture(CaptureSource::Output(output), preview);
        cx.set_session_locked_hint(true);
        let refused = cx.capture(CaptureSource::Output(output), preview);
        cx.set_session_locked_hint(false);
        let after = cx.capture(CaptureSource::Output(output), preview);
        self.pending = vec![
            (aborted, Check::LockAborted),
            (refused, Check::LockRefused),
            (after, Check::AfterUnlock),
        ];
        if let Some(t) = self.deadline.take() {
            cx.cancel_timer(t);
        }
        self.deadline = Some(cx.set_timer(Duration::from_secs(10)));
    }

    fn verdict(&mut self, ok: bool, what: &str) {
        if ok {
            PASSES.fetch_add(1, Ordering::Relaxed);
            println!("PASS {what}");
        } else {
            FAILURES.fetch_add(1, Ordering::Relaxed);
            println!("FAIL {what}");
        }
    }

    /// Dört çeyreğin merkezi: sol = primary, sağ = error.
    fn check_quadrants(&mut self, img: &CapturedImage, what: &str) {
        let Some((left, right)) = self.colors else {
            return;
        };
        let (w, h) = (img.image.width, img.image.height);
        let px = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            let p = &img.image.pixels[i..i + 4];
            [p[0], p[1], p[2], p[3]]
        };
        let close = |p: [u8; 4], c: [u8; 3]| {
            p[3] == 255 && (0..3).all(|i| p[i].abs_diff(c[i]) <= 3)
        };
        let points = [
            (w / 4, h / 4, left, "sol üst"),
            (w / 4, 3 * h / 4, left, "sol alt"),
            (3 * w / 4, h / 4, right, "sağ üst"),
            (3 * w / 4, 3 * h / 4, right, "sağ alt"),
        ];
        for (x, y, want, name) in points {
            let got = px(x, y);
            let ok = close(got, want);
            if !ok {
                println!("  {what} {name} ({x},{y}): {got:?} ≠ {want:?}");
            }
            self.verdict(ok, &format!("{what} {name}"));
        }
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        println!(
            "sonuç: {} geçti, {} kaldı",
            PASSES.load(Ordering::Relaxed),
            FAILURES.load(Ordering::Relaxed)
        );
        cx.toplevels_stop();
        cx.quit();
    }
}

impl App for Probe {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.window.is_some() {
            return;
        }
        println!(
            "çıkış {}x{} ölçek {}",
            output.size.width, output.size.height, output.scale
        );
        self.output = Some(output.id);
        let theme = cx.theme();
        let c = &theme.colors;
        self.colors = Some((
            rgb8(c.primary.with_alpha(1.0)),
            rgb8(c.error.with_alpha(1.0)),
        ));
        self.window = Some(cx.create_window(WindowSpec {
            size: (480, 320),
            ..WindowSpec::new("Nuxut yakalama testi", APP_ID)
        }));
        match cx.toplevels_start() {
            Ok(()) => println!("pencere listesi başladı"),
            Err(e) => {
                println!("FAIL pencere listesi: {e}");
                FAILURES.fetch_add(1, Ordering::Relaxed);
                cx.quit();
            }
        }
    }

    fn output_removed(&mut self, _: &mut Context<Self>, _: OutputId) {}

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        self.size = size;
        self.scale = scale;
        cx.request_frame(surface);
    }

    fn surface_presented(&mut self, cx: &mut Context<Self>, _: SurfaceId) {
        self.presented = true;
        self.maybe_start(cx);
    }

    fn toplevel(&mut self, cx: &mut Context<Self>, event: ToplevelEvent) {
        if let ToplevelEvent::Added(id) = event {
            // Yalnızca kendi penceremiz (uygulama kimliği yazdırılmaz).
            let mine = cx.toplevel(id).is_some_and(|t| t.app_id == APP_ID);
            println!(
                "pencere eklendi (kendi: {mine}, toplam {})",
                cx.toplevels().count()
            );
            if mine && self.toplevel.is_none() {
                self.toplevel = Some(id);
                self.maybe_start(cx);
            }
        }
    }

    fn captured(
        &mut self,
        cx: &mut Context<Self>,
        id: CaptureId,
        result: Result<CapturedImage, CaptureError>,
    ) {
        let Some(pos) = self.pending.iter().position(|(p, _)| *p == id) else {
            self.verdict(false, "bilinmeyen yakalama sonucu");
            return;
        };
        let (_, check) = self.pending.remove(pos);
        println!("{check:?}: {result:?}");
        match (check, result) {
            (Check::Cancelled, _) => self.verdict(false, "iptal edilen yakalamanın sonucu geldi"),
            (Check::LockAborted | Check::LockRefused, r) => self.verdict(
                matches!(r, Err(CaptureError::Locked)),
                &format!("{check:?}: kilitliyken Locked"),
            ),
            (_, Err(e)) => self.verdict(false, &format!("{check:?} hata: {e}")),
            (Check::WindowFull, Ok(img)) => {
                // Pencere tamponunun ölçeği compositor'a bağlı: fiziksel (yüzeyin tamponu) ya da
                // mantıksal (sway 1.12 kesirli ölçekte mantıksal veriyor). Tam boyutta küçültme yok.
                let physical = (
                    (self.size.width * self.scale).round() as u32,
                    (self.size.height * self.scale).round() as u32,
                );
                let logical = (self.size.width as u32, self.size.height as u32);
                let got = (img.image.width, img.image.height);
                self.verdict(
                    (got == physical || got == logical) && img.source_size == got,
                    &format!("pencere tam boyut {got:?} ∈ {{{physical:?}, {logical:?}}}"),
                );
                self.check_quadrants(&img, "pencere");
            }
            (Check::WindowPreview, Ok(img)) => {
                let (w, h) = (img.image.width, img.image.height);
                let ratio = |a: u32, b: u32| a as f32 / b.max(1) as f32;
                let aspect_ok =
                    (ratio(w, h) - ratio(img.source_size.0, img.source_size.1)).abs() < 0.05;
                self.verdict(
                    w <= 64 && h <= 64 && w.max(h) == 64 && aspect_ok,
                    &format!("önizleme {w}x{h} ≤ 64×64, oran korunmuş"),
                );
                self.check_quadrants(&img, "önizleme");
            }
            (Check::OutputPreview, Ok(img)) => {
                let (w, h) = (img.image.width, img.image.height);
                self.verdict(
                    w.max(h) == 256,
                    &format!("çıkış önizleme {w}x{h} (tampon {:?})", img.source_size),
                );
                self.check_quadrants(&img, "çıkış");
            }
            (Check::AfterUnlock, Ok(img)) => {
                self.verdict(true, "kilit kalkınca yakalama yeniden çalışıyor");
                self.check_quadrants(&img, "kilit sonrası çıkış");
            }
        }
        let only_cancelled = self.pending.iter().all(|(_, c)| *c == Check::Cancelled);
        if only_cancelled && !self.lock_phase {
            self.start_lock_phase(cx);
        } else if only_cancelled {
            // İptalin sonucu gelmesin diye kısa bekle.
            if let Some(t) = self.deadline.take() {
                cx.cancel_timer(t);
            }
            self.deadline = Some(cx.set_timer(Duration::from_millis(500)));
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.start_timer == Some(id) {
            self.start(cx);
        } else if self.deadline == Some(id) {
            for (_, check) in std::mem::take(&mut self.pending) {
                let ok = check == Check::Cancelled;
                self.verdict(ok, &format!("{check:?} sonuçsuz"));
            }
            self.finish(cx);
        }
    }

    fn pointer(&mut self, _: &mut Context<Self>, _: SurfaceId, _: PointerEvent) {}

    fn draw(&mut self, cx: &mut Context<Self>, _: SurfaceId, _: Instant, painter: &mut Painter) {
        let c = &cx.theme().colors;
        let (w, h) = (self.size.width, self.size.height);
        painter.rect(
            Rect::new(0.0, 0.0, w / 2.0, h),
            &RectStyle::filled(c.primary.with_alpha(1.0), Corners::ZERO),
        );
        painter.rect(
            Rect::new(w / 2.0, 0.0, w - w / 2.0, h),
            &RectStyle::filled(c.error.with_alpha(1.0), Corners::ZERO),
        );
    }
}

/// Yakalamayla ilgili global'leri döker (wayland-info yok).
fn dump_globals() -> ExitCode {
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::{Connection, Dispatch, QueueHandle};
    struct S;
    impl Dispatch<WlRegistry, GlobalListContents> for S {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    let Ok(conn) = Connection::connect_to_env() else {
        eprintln!("bağlanılamadı");
        return ExitCode::FAILURE;
    };
    let Ok((globals, _queue)) = registry_queue_init::<S>(&conn) else {
        eprintln!("registry alınamadı");
        return ExitCode::FAILURE;
    };
    let wanted = [
        "ext_image_copy_capture_manager_v1",
        "ext_output_image_capture_source_manager_v1",
        "ext_foreign_toplevel_image_capture_source_manager_v1",
        "ext_foreign_toplevel_list_v1",
        "zwlr_screencopy_manager_v1",
        "hyprland_toplevel_export_manager_v1",
        "hyprland_toplevel_mapping_manager_v1",
        "wl_shm",
    ];
    let list = globals.contents().clone_list();
    for name in wanted {
        match list.iter().find(|g| g.interface == name) {
            Some(g) => println!("{name} v{}", g.version),
            None => println!("{name} YOK"),
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        eprintln!(
            "HYPRLAND_INSTANCE_SIGNATURE tanımlı: kullanıcının oturumunda yakalama yapılmaz. \
             İzole test compositor'ının ortamıyla çalıştırın (tools/test-compositor.sh env)."
        );
        return ExitCode::from(3);
    }
    if std::env::args().any(|a| a == "--globals") {
        return dump_globals();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();
    if let Err(e) = nuxut_wayland::run(Probe::default()) {
        eprintln!("runtime: {e}");
        return ExitCode::FAILURE;
    }
    // `run` App'i tüketir: sonuç sayaçları süreç geneli.
    if FAILURES.load(Ordering::Relaxed) == 0 && PASSES.load(Ordering::Relaxed) > 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
