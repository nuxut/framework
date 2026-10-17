//! Nuxut renk temaları karşılaştırması (docs/design/color-themes.md): her tema × kip için bir kart —
//! yeniden boyanmış yerleşik duvar kağıdı, üstünde temanın kendi token'larıyla küçük bir arayüz
//! (bar, panel kartı, metin, vurgu düğmesi, kaydırıcı) ve dış uygulamalara giden tonal paletlerden
//! şerit (birincil, ikincil, üçüncül kap, hata). Tüm renkler temadan; görsel doğrulama içindir.
//!
//! ```sh
//! cargo run -p nuxut-ui --example color_themes -- <shell>/crates/nuxut-wallpapers/assets [duvar-kağıdı-kimliği]
//! ```
//!
//! Dizin verilmezse duvar kağıdı yerine temanın yüzey gradyanı çizilir.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_image::{Fit, TargetSize, render_svg};
use nuxut_palette::{ContrastMode, Mode, Palette as Export};
use nuxut_render::{ImageData, ImageFit, ImageStyle, RectStyle, TextStyle, Texture, TextureOptions};
use nuxut_theme::color_theme::BLUE;
use nuxut_theme::{ColorThemeId, Palette, ThemeMode, recolor_svg};
use nuxut_wayland::{App, Context, Key, KeyEvent, KeyState, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId, WindowSpec};

const COLS: usize = 4;
const GAP: f32 = 12.0;

struct Card {
    id: ColorThemeId,
    mode: ThemeMode,
    palette: Palette,
    export: Export,
    wallpaper: Option<Texture>,
}

struct Demo {
    window: Option<SurfaceId>,
    size: Size,
    dir: Option<PathBuf>,
    wallpaper: String,
    cards: Vec<Card>,
    uploaded: bool,
}

/// Görünen ad (örnek; ürün adları uygulama kataloğunda `color-theme-<ad>`).
fn title(id: ColorThemeId) -> &'static str {
    match id {
        ColorThemeId::Blue => "Nuxut mavisi",   // i18n: örnek etiketi (galeri dışı tanı aracı)
        ColorThemeId::Neutral => "Nötr",        // i18n: örnek etiketi
        ColorThemeId::Sage => "Adaçayı",        // i18n: örnek etiketi
        ColorThemeId::Rose => "Gül kurusu",     // i18n: örnek etiketi
        ColorThemeId::Amber => "Kehribar",      // i18n: örnek etiketi
        ColorThemeId::Graphite => "Grafit",     // i18n: örnek etiketi
    }
}

impl Demo {
    fn render_wallpaper(&self, card: &Card, w: u32, h: u32) -> Option<ImageData> {
        let dir = self.dir.as_ref()?;
        let path = dir.join(format!("{}-{}.svg", self.wallpaper, card.mode.name()));
        let svg = std::fs::read(path).ok()?;
        let theme = card.id.theme();
        let svg = recolor_svg(&svg, |c| theme.recolor(&BLUE, c));
        let img = render_svg(&svg, TargetSize { crop: true, ..TargetSize::new(w, h, Fit::Cover) }).ok()?;
        Some(ImageData { width: img.width(), height: img.height(), pixels: Arc::from(img.pixels()) })
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.window.is_none() {
            let spec = WindowSpec { size: (1880, 1040), ..WindowSpec::new("Nuxut renk temaları", "nuxut-color-themes") }; // i18n: örnek pencere başlığı
            self.window = Some(cx.create_window(spec));
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, _scale: f32) {
        self.size = size;
        cx.request_frame(surface);
    }

    fn close_requested(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {
        cx.destroy_surface(surface);
        cx.quit();
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if event.state == KeyState::Pressed && event.key == Key::Escape {
            cx.destroy_surface(surface);
            cx.quit();
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, _now: Instant, painter: &mut Painter) {
        let theme = cx.theme().clone();
        let rows = self.cards.len().div_ceil(COLS);
        let cw = (self.size.width - GAP * (COLS as f32 + 1.0)) / COLS as f32;
        let ch = (self.size.height - GAP * (rows as f32 + 1.0)) / rows as f32;
        let wall_h = (ch * 0.62).round();
        if !self.uploaded {
            self.uploaded = true;
            let scale = painter.scale();
            for i in 0..self.cards.len() {
                let data = self.render_wallpaper(&self.cards[i], (cw * scale) as u32, (wall_h * scale) as u32);
                self.cards[i].wallpaper = data.map(|d| painter.upload_texture(d, TextureOptions::default()));
            }
        }
        let backdrop = theme.colors.shadow;
        painter.rect(Rect::new(0.0, 0.0, self.size.width, self.size.height), &RectStyle::filled(backdrop, Corners::ZERO));
        let family = theme.typography.ui.family.clone();
        let size = theme.typography.size;
        let text = |painter: &mut Painter, s: &str, px: f32, weight: u16, at: Point, color| {
            let layout = painter.text_system().layout(s, &TextStyle::new(family.clone(), px, weight), None);
            painter.text(&layout, at, color);
            layout.size()
        };
        for (i, card) in self.cards.iter().enumerate() {
            let (col, row) = (i % COLS, i / COLS);
            let r = Rect::new(GAP + col as f32 * (cw + GAP), GAP + row as f32 * (ch + GAP), cw, ch);
            let p = &card.palette;
            let radius = theme.shape.large();
            painter.rect(r, &RectStyle::filled(p.surface, radius));
            // Duvar kağıdı (üst bölüm) ya da yüzey.
            let wall = Rect::new(r.x, r.y, r.width, wall_h);
            let top = Corners { bottom_left: 0.0, bottom_right: 0.0, ..radius };
            match &card.wallpaper {
                Some(t) => painter.image(wall, t, &ImageStyle { fit: ImageFit::Cover, radii: top, ..ImageStyle::default() }),
                None => painter.rect(wall, &RectStyle::filled(p.surface_container, top)),
            }
            // Bar şeridi: duvar kağıdının üstünde, yüzey renginde; etkin workspace vurguda.
            let bar = Rect::new(wall.x + 10.0, wall.y + 10.0, wall.width - 20.0, 26.0);
            painter.rect(bar, &RectStyle::filled(p.surface, theme.shape.small()));
            for k in 0..4 {
                let dot = Rect::new(bar.x + 10.0 + k as f32 * 22.0, bar.y + 8.0, if k == 1 { 18.0 } else { 10.0 }, 10.0);
                let c = if k == 1 { p.primary } else { p.on_surface_variant.multiply_alpha(0.5) };
                painter.rect(dot, &RectStyle::filled(c, Corners::uniform(5.0)));
            }
            let clock = format!("{} · {}", title(card.id), if card.mode == ThemeMode::Dark { "koyu" } else { "açık" }); // i18n: örnek etiketi
            text(painter, &clock, size, 600, Point::new(bar.x + 110.0, bar.y + 5.0), p.on_surface);
            // Panel kartı: duvar kağıdının sağ altında.
            let panel = Rect::new(wall.x + wall.width * 0.42, wall.y + 48.0, wall.width * 0.58 - 12.0, wall_h - 60.0);
            painter.rect(panel, &RectStyle::filled(p.surface, theme.shape.medium()));
            let inner = Rect::new(panel.x + 10.0, panel.y + 10.0, panel.width - 20.0, panel.height - 20.0);
            painter.rect(inner, &RectStyle::filled(p.surface_container, theme.shape.small()));
            let s = text(painter, "Bildirim başlığı", size, 600, Point::new(inner.x + 12.0, inner.y + 10.0), p.on_surface); // i18n: örnek etiketi
            text(painter, "İkincil metin, ayrıntı", size - 1.0, 400, Point::new(inner.x + 12.0, inner.y + 14.0 + s.height), p.on_surface_variant); // i18n: örnek etiketi
            let btn = Rect::new(inner.x + 12.0, inner.y + inner.height - 40.0, 96.0, 28.0);
            painter.rect(btn, &RectStyle::filled(p.primary, Corners::uniform(14.0)));
            text(painter, "Tamam", size, 600, Point::new(btn.x + 24.0, btn.y + 6.0), p.on_primary); // i18n: örnek etiketi
            let btn2 = Rect::new(btn.x + btn.width + 8.0, btn.y, 96.0, 28.0);
            painter.rect(btn2, &RectStyle::filled(p.surface_bright, Corners::uniform(14.0)));
            text(painter, "Vazgeç", size, 500, Point::new(btn2.x + 22.0, btn2.y + 6.0), p.on_surface); // i18n: örnek etiketi
            // Alt bölüm: kaydırıcı + palet şeridi.
            let y = r.y + wall_h + 14.0;
            let track = Rect::new(r.x + 16.0, y + 4.0, r.width - 32.0, 6.0);
            painter.rect(track, &RectStyle::filled(p.outline, Corners::uniform(3.0)));
            painter.rect(Rect::new(track.x, track.y, track.width * 0.62, 6.0), &RectStyle::filled(p.primary, Corners::uniform(3.0)));
            let s = card.export.scheme(if card.mode == ThemeMode::Dark { Mode::Dark } else { Mode::Light });
            let swatches = [
                s.primary,
                s.primary_container,
                s.secondary,
                s.secondary_container,
                s.tertiary,
                s.tertiary_container,
                s.surface_container_highest,
                s.error,
            ];
            let sw = (r.width - 32.0 - 7.0 * 6.0) / 8.0;
            let sy = y + 22.0;
            let sh = (r.y + r.height - 14.0 - sy).max(8.0);
            for (k, c) in swatches.iter().enumerate() {
                let x = r.x + 16.0 + k as f32 * (sw + 6.0);
                painter.rect(Rect::new(x, sy, sw, sh), &RectStyle::filled(*c, theme.shape.small()));
            }
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let mut args = std::env::args().skip(1);
    let dir = args.next().map(PathBuf::from);
    let wallpaper = args.next().unwrap_or_else(|| "pair-flight".into());
    let mut cards = Vec::new();
    for id in ColorThemeId::ALL {
        for mode in ThemeMode::ALL {
            let export = Export::from_color_theme(id.theme(), mode.into(), ContrastMode::Standard);
            cards.push(Card { id, mode, palette: id.theme().palette(mode), export, wallpaper: None });
        }
    }
    let demo = Demo { window: None, size: Size::ZERO, dir, wallpaper, cards, uploaded: false };
    if let Err(e) = nuxut_wayland::run(demo) {
        tracing::error!("{e}");
        std::process::exit(1);
    }
}
