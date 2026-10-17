//! Kısmi yeniden çizim ölçümü: shell benzeri tam ekran katman yüzeyi (blob çerçeve + üst bar,
//! gölge), bar'da saniyede bir (ya da `DEMO_TICK_MS`) değişen küçük bir saat metni.
//! YALNIZCA test compositor'ında çalıştırın (`tools/test-compositor.sh`).
//!
//! Ortam:
//! - `DEMO_DAMAGE=clock|two|bar|full` (varsayılan `clock`): tikte bildirilen hasar — yalnız saat
//!   kutusu; `two`: ortadaki saat ve sağ uçtaki sistem izleyici hapı aynı karede değişir (iki
//!   uzak parça); tüm bar şeridi ya da hiç (tüm yüzey).
//! - `DEMO_UNION=1` (`two` ile): iki kutunun sınırlayıcı kutusu bildirilir — çoklu parçalı
//!   hasardan önceki tek kutunun benzetimi (önce/sonra ölçümü aynı ikiliyle).
//! - `DEMO_TICK_MS` (varsayılan 1000), `DEMO_TICKS` (varsayılan 0 = sonsuz): bu kadar tikten
//!   sonra ölçüm satırı (`cpu …`) yazılır ve tik durur; süreç açık kalır (ardından `full`
//!   aynı durumu yeniden çizebilsin), `q` ile ya da öldürülerek kapanır.
//! - `DEMO_BLOB=0`: blob çerçeve yerine düz bar; `DEMO_SURFACE=strip`: yalnızca bar yüksekliğinde
//!   yüzey (swap maliyetinin yüzey alanına bağlılığı).
//! - `NUXUT_SOFTWARE_PRESENT=0|1`: sunum yolunu zorla (varsayılan: llvmpipe ise shm).
//! - `DEMO_WANDER=1`: saat her tikte bar boyunca başka bir yere atlar (eski ∪ yeni bölge
//!   hasarlı); artık piksel testi için.
//!
//! stdin: `full` tüm yüzeyi bir kez yeniden çizer (piksel karşılaştırması için), `q` çıkar.
//! Her karenin ardından stdout'a `drawn <n>` yazılır.
//!
//! Kare zamanlaması: `RUST_LOG=nuxut::frame=trace` (yaş, bölge, kayıt/GL/swap µs).
//! Süreç CPU'su (tüm iş parçacıkları, llvmpipe dahil) çıkışta `cpu` satırında.

use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_render::{BlobField, FrameShape, RectStyle, ShadowStyle, TextStyle};
use nuxut_wayland::{
    Anchor, App, Context, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent,
    SurfaceId, TimerId,
};

const BAR: f32 = 40.0;
const EDGE: f32 = 8.0;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Clock,
    /// Ortada saat + sağda sistem izleyici (shell bar'ı gibi iki uzak parça).
    Two,
    Bar,
    Full,
}

enum Message {
    Full,
    Quit,
}

struct Demo {
    mode: Mode,
    tick_every: Duration,
    ticks_max: u64,
    wander: bool,
    /// `DEMO_BLOB=0`: blob çerçeve yerine düz bar dikdörtgeni (blob gölgelendiricisinin payı).
    blob: bool,
    /// `DEMO_SURFACE=strip`: tam ekran yerine yalnızca bar yüksekliğinde yüzey (blob yok).
    strip: bool,
    surface: Option<(SurfaceId, Size)>,
    ticks: u64,
    drawn: u64,
    full_next: bool,
    /// Önceki karede saatin kapladığı alan (eski ∪ yeni hasar için).
    last_clock: Option<Rect>,
    /// `two`: önceki karede sistem izleyici hapı.
    last_sysmon: Option<Rect>,
    /// `DEMO_UNION=1`: `two` hasarını tek sınırlayıcı kutu olarak bildir.
    union: bool,
    timer: Option<TimerId>,
    /// İlk tikte (ısınma sonrası) CPU ve zaman.
    start: Option<(u64, Instant, u64)>,
}

/// Süreç CPU'su (utime + stime, tüm iş parçacıkları), saat tıkı (Linux'ta 100 Hz).
fn process_cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    // comm parantez içinde boşluk içerebilir: son ')' sonrasından say.
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    let f: Vec<&str> = rest.split_whitespace().collect();
    // ')' sonrası 0: state (alan 3) → utime alan 14 = indeks 11, stime 12.
    let get = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    get(11) + get(12)
}

impl Demo {
    fn clock_text(&self) -> String {
        let s = self.ticks;
        format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
    }

    fn clock_rect(&self, size: Size, text_w: f32) -> Rect {
        let w = text_w + 16.0;
        let x = if self.mode == Mode::Two {
            (size.width - w) / 2.0
        } else if self.wander {
            // Bar boyunca sözde rastgele (deterministik) konum.
            let span = (size.width - 2.0 * EDGE - w).max(1.0);
            EDGE + ((self.ticks.wrapping_mul(2_654_435_761) % 1000) as f32 / 1000.0) * span
        } else {
            size.width - EDGE - 12.0 - w
        };
        Rect::new(x.round(), 6.0, w, BAR - 12.0)
    }

    /// `two`: sağ uçtaki sistem izleyici metni (her tikte değişir).
    fn sysmon_text(&self) -> String {
        format!("CPU {:>2}%  {:.1} GB", self.ticks * 37 % 100, 3.0 + (self.ticks % 9) as f32 / 10.0)
    }

    fn sysmon_rect(size: Size, text_w: f32) -> Rect {
        let w = text_w + 16.0;
        Rect::new((size.width - EDGE - 12.0 - w).round(), 6.0, w, BAR - 12.0)
    }
}

impl App for Demo {
    type Message = Message;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        let mut spec = LayerSurfaceSpec::new("nuxut-damage-bench", output.id);
        spec.layer = Layer::Bottom;
        if self.strip {
            // Yalnızca bar şeridi: swap maliyetinin yüzey alanıyla ölçeklenip ölçeklenmediği.
            spec.anchor = Anchor::TOP | Anchor::LEFT | Anchor::RIGHT;
            spec.size = (0, BAR as u32);
        } else {
            spec.anchor = Anchor::ALL;
        }
        spec.exclusive_zone = -1;
        self.surface = Some((cx.create_layer_surface(spec), Size::ZERO));
        let sender = cx.sender();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                let Ok(line) = line else { break };
                let m = match line.trim() {
                    "full" => Message::Full,
                    "q" => Message::Quit,
                    _ => continue,
                };
                if sender.send(m).is_err() {
                    break;
                }
            }
        });
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn message(&mut self, cx: &mut Context<Self>, message: Message) {
        match message {
            Message::Full => {
                self.full_next = true;
                if let Some((id, _)) = self.surface {
                    cx.request_frame(id);
                }
            }
            Message::Quit => cx.quit(),
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.timer != Some(id) {
            return;
        }
        if self.start.is_none() {
            self.start = Some((process_cpu_ticks(), Instant::now(), self.drawn));
        }
        // Son tik saati ilerletmez: ardından gelen `full` aynı durumu yeniden çizmeli (piksel
        // karşılaştırması).
        if self.ticks_max > 0 && self.ticks >= self.ticks_max {
            if let Some((cpu0, t0, d0)) = self.start {
                let cpu_ms = (process_cpu_ticks() - cpu0) * 10;
                let frames = (self.drawn - d0).max(1);
                println!(
                    "cpu frames={frames} wall_ms={} cpu_ms={cpu_ms} cpu_ms_per_frame={:.2}",
                    t0.elapsed().as_millis(),
                    cpu_ms as f64 / frames as f64
                );
                let _ = std::io::stdout().flush();
            }
            self.timer = None;
            return;
        }
        self.ticks += 1;
        if let Some((id, _)) = self.surface {
            cx.request_frame(id);
        }
        self.timer = Some(cx.set_timer(self.tick_every));
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
            if self.timer.is_none() && self.ticks == 0 {
                self.timer = Some(cx.set_timer(self.tick_every));
            }
        }
    }

    fn draw(
        &mut self,
        cx: &mut Context<Self>,
        surface: SurfaceId,
        _now: Instant,
        painter: &mut Painter,
    ) {
        let Some((_, size)) = self.surface.filter(|(id, _)| *id == surface) else {
            return;
        };
        let theme = cx.theme();
        let colors = &theme.colors;
        let bar = Rect::new(0.0, 0.0, size.width, BAR);
        let style = TextStyle::new(
            theme.typography.ui.family.clone(),
            theme.typography.size,
            500,
        );
        let clock = painter
            .text_system()
            .layout(&self.clock_text(), &style, None);
        let clock_rect = self.clock_rect(size, clock.size().width);
        let two = self.mode == Mode::Two;
        let sysmon = two.then(|| {
            let l = painter
                .text_system()
                .layout(&self.sysmon_text(), &style, None);
            let r = Self::sysmon_rect(size, l.size().width);
            (l, r)
        });

        // Hasar çizimden önce: kip ve önceki/yeni saat (ve sistem izleyici) kutusu.
        if std::mem::take(&mut self.full_next) {
            painter.damage_all();
        } else {
            match self.mode {
                Mode::Full => {}
                Mode::Bar => painter.damage(bar),
                Mode::Clock | Mode::Two => {
                    let rects = self
                        .last_clock
                        .iter()
                        .chain(std::iter::once(&clock_rect))
                        .chain(&self.last_sysmon)
                        .chain(sysmon.as_ref().map(|(_, r)| r));
                    if self.union {
                        painter.damage(rects.fold(Rect::ZERO, |a, b| {
                            if a.is_empty() { *b } else { a.union(b) }
                        }));
                    } else {
                        for r in rects {
                            painter.damage(*r);
                        }
                    }
                }
            }
        }
        self.last_clock = Some(clock_rect);
        self.last_sysmon = sysmon.as_ref().map(|(_, r)| *r);

        let shadow = theme.shadow.enabled.then(|| ShadowStyle {
            color: theme.shadow.color,
            blur: theme.shadow.blur,
            offset: Point::new(0.0, theme.shadow.offset_y),
            spread: 0.0,
        });
        if !self.blob || self.strip {
            painter.rect(bar, &RectStyle::filled(colors.surface, Corners::ZERO));
        } else {
            painter.blob(&BlobField {
                frame: Some(FrameShape {
                    outer: Rect::new(0.0, 0.0, size.width, size.height),
                    inner: Rect::new(
                        EDGE,
                        BAR,
                        (size.width - 2.0 * EDGE).max(0.0),
                        (size.height - BAR - EDGE).max(0.0),
                    ),
                    inner_radius: theme.blob.smoothing,
                }),
                shapes: Vec::new(),
                smoothing: theme.blob.smoothing,
                corner_radius: theme.shape.radius_large,
                fill: colors.surface,
                shadow,
            });
        }

        // Bar içeriği: çalışma alanı hapları, başlık, simgeler, saat.
        painter.push_clip(bar);
        let pill = RectStyle::filled(
            colors.surface_container,
            Corners::uniform(theme.shape.radius_small),
        );
        let active = RectStyle::filled(colors.primary, Corners::uniform(theme.shape.radius_small));
        for i in 0..9 {
            let r = Rect::new(EDGE + 12.0 + i as f32 * 34.0, 6.0, 28.0, BAR - 12.0);
            painter.rect(r, if i == 2 { &active } else { &pill });
            let n = painter
                .text_system()
                .layout(&(i + 1).to_string(), &style, None);
            let c = if i == 2 {
                colors.on_primary
            } else {
                colors.on_surface
            };
            painter.text(
                &n,
                Point::new(
                    r.x + (r.width - n.size().width) / 2.0,
                    r.y + (r.height - n.size().height) / 2.0,
                ),
                c,
            );
        }
        let title = painter
            .text_system()
            .layout("nuxut — damage bench", &style, None);
        if !two {
            painter.text(
                &title,
                Point::new(
                    (size.width - title.size().width) / 2.0,
                    (BAR - title.size().height) / 2.0,
                ),
                colors.on_surface,
            );
        }
        if let Some((l, r)) = &sysmon {
            painter.rect(*r, &pill);
            painter.text(
                l,
                Point::new(r.x + 8.0, r.y + (r.height - l.size().height) / 2.0),
                colors.on_surface,
            );
        }
        for i in 0..6 {
            let r = Rect::new(
                size.width - EDGE - 140.0 - 12.0 - (i + 1) as f32 * 30.0,
                10.0,
                20.0,
                20.0,
            );
            painter.rect(
                r,
                &RectStyle::filled(colors.on_surface_variant, Corners::uniform(10.0)),
            );
        }
        painter.rect(clock_rect, &pill);
        painter.text(
            &clock,
            Point::new(
                clock_rect.x + 8.0,
                clock_rect.y + (clock_rect.height - clock.size().height) / 2.0,
            ),
            colors.on_surface,
        );
        painter.pop_clip();

        self.drawn += 1;
        println!("drawn {}", self.drawn);
        let _ = std::io::stdout().flush();
    }
}

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let mode = match std::env::var("DEMO_DAMAGE").as_deref() {
        Ok("full") => Mode::Full,
        Ok("bar") => Mode::Bar,
        Ok("two") => Mode::Two,
        _ => Mode::Clock,
    };
    let demo = Demo {
        mode,
        tick_every: Duration::from_millis(env("DEMO_TICK_MS", 1000)),
        ticks_max: env("DEMO_TICKS", 0),
        wander: std::env::var_os("DEMO_WANDER").is_some_and(|v| v != "0"),
        blob: std::env::var_os("DEMO_BLOB").is_none_or(|v| v != "0"),
        strip: std::env::var("DEMO_SURFACE").is_ok_and(|v| v == "strip"),
        surface: None,
        ticks: 0,
        drawn: 0,
        full_next: false,
        last_clock: None,
        last_sysmon: None,
        union: std::env::var_os("DEMO_UNION").is_some_and(|v| v != "0"),
        timer: None,
        start: None,
    };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
