//! Optik ortalama görsel denetimi (ADR-0009): bar benzeri hücrelerde Nerd/Material ikonları ve
//! saat etiketleri; her hücrenin merkezinden ince artı çizgisi geçer. Çağıran yalnızca layout
//! kutusunu hücrede ortalar (shell'in `center_in`'i gibi) — ek ofset yok.
//!
//! ```sh
//! cargo run -p nuxut --example optical
//! ```
//! Escape (yüzey odaktayken) çıkar.

use std::collections::HashMap;
use std::time::Instant;

use nuxut::ui::{Icon, IconName, IconView, Label, TextRole, text_style};
use nuxut::{
    Anchor, App, Context, Corners, Key, KeyEvent, KeyState, KeyboardMode, Layer, LayerSurfaceSpec, OutputId,
    OutputInfo, Painter, Point, Rect, RectStyle, Size, SurfaceId, TextStyle, Theme,
};

/// Hücre kenarı (shell'in `cx.item`'ı gibi) ve ikon boyutu (shell: `item * 0.62`).
const ITEM: f32 = 32.0;
// font-istisnası: görsel denetim, shell'in workspace glifleriyle aynı ham Nerd kod noktaları.
const NERD_GLYPHS: [&str; 6] = ["\u{f0239}", "\u{f018d}", "\u{f0a1e}", "\u{f024b}", "\u{f066f}", "\u{e6ae}"];

struct Row {
    cells: Vec<Label>,
}

struct Screen {
    size: Size,
    scale: f32,
    rows: Vec<Row>,
    icons: Vec<IconView>,
}

impl Screen {
    fn new(theme: &Theme) -> Self {
        let glyph = |family: &str| Row {
            cells: NERD_GLYPHS
                .iter()
                .map(|g| Label::new(*g, TextStyle::new(family, (ITEM * 0.62).round(), 400)))
                .collect(),
        };
        let clock = |family: Option<&str>| {
            let mut style = text_style(theme, TextRole::Body);
            if let Some(f) = family {
                style.family = f.to_owned();
            }
            Row { cells: ["14:05", "09:41", "Sal 14 Eki"].iter().map(|t| Label::new(*t, style.clone())).collect() }
        };
        let rows = vec![
            // font-istisnası: belirli ikon fontunun hücre metriği denetleniyor.
            glyph("JetBrainsMono Nerd Font"),
            // font-istisnası: belirli ikon fontunun hücre metriği denetleniyor.
            glyph("Symbols Nerd Font"),
            clock(None),
            // font-istisnası: eş aralıklı font metriği denetleniyor.
            clock(Some("JetBrainsMono Nerd Font")),
            clock(Some("Google Sans Flex")),
            Row { cells: (1..=6).map(|i| Label::new(i.to_string(), text_style(theme, TextRole::Body))).collect() },
        ];
        let icons = [IconName::Wifi, IconName::VolumeHigh, IconName::Bluetooth, IconName::Settings, IconName::Search, IconName::Close]
            .into_iter()
            .map(|n| IconView::new(theme, Icon::named(n)))
            .collect();
        Self { size: Size::ZERO, scale: 1.0, rows, icons }
    }

    fn draw(&mut self, theme: &Theme, painter: &mut Painter) {
        let c = &theme.colors;
        painter.rect(Rect::new(0.0, 0.0, self.size.width, self.size.height), &RectStyle::filled(c.surface, Corners::ZERO));
        let hair = 1.0 / self.scale;
        let guide = c.error;
        let gap = 8.0;
        let mut y = gap;
        let n_rows = self.rows.len() + 1;
        for r in 0..n_rows {
            let mut x = gap;
            let count = if r < self.rows.len() { self.rows[r].cells.len() } else { self.icons.len() };
            for i in 0..count {
                let w = if (2..=4).contains(&r) { ITEM * 3.2 } else { ITEM };
                let cell = Rect::new(x, y, w, ITEM);
                painter.rect(cell, &RectStyle::filled(c.surface_bright, Corners::uniform(ITEM * 0.5)));
                let center = cell.center();
                painter.rect(Rect::new(cell.x, center.y - hair * 0.5, cell.width, hair), &RectStyle::filled(guide, Corners::ZERO));
                painter.rect(Rect::new(center.x - hair * 0.5, cell.y, hair, cell.height), &RectStyle::filled(guide, Corners::ZERO));
                if r < self.rows.len() {
                    let label = &mut self.rows[r].cells[i];
                    let s = label.measure_in(painter);
                    label.draw(painter, Point::new(center.x - s.width * 0.5, center.y - s.height * 0.5), c.on_surface);
                } else {
                    self.icons[i].draw_centered(painter, center, c.on_surface);
                }
                x += w + gap;
            }
            y += ITEM + gap;
        }
    }
}

#[derive(Default)]
struct Demo {
    screens: HashMap<SurfaceId, (OutputId, Screen)>,
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        let spec = LayerSurfaceSpec {
            layer: Layer::Top,
            anchor: Anchor::TOP | Anchor::LEFT,
            size: (420, (7.0 * (ITEM + 8.0) + 8.0) as u32),
            exclusive_zone: 0,
            keyboard: KeyboardMode::OnDemand,
            ..LayerSurfaceSpec::new("nuxut-optical-demo", output.id)
        };
        let id = cx.create_layer_surface(spec);
        self.screens.insert(id, (output.id, Screen::new(cx.theme())));
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.screens.retain(|&id, (o, _)| {
            let keep = *o != output;
            if !keep {
                cx.destroy_surface(id);
            }
            keep
        });
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        if let Some((_, s)) = self.screens.get_mut(&surface) {
            s.size = size;
            s.scale = scale;
            cx.request_frame(surface);
        }
    }

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: nuxut::PointerEvent) {}

    fn keyboard(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: KeyEvent) {
        if event.state == KeyState::Pressed && event.key == Key::Escape {
            cx.quit();
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, _now: Instant, painter: &mut Painter) {
        let theme = cx.theme().clone();
        if let Some((_, s)) = self.screens.get_mut(&surface) {
            s.draw(&theme, painter);
        }
    }
}

fn main() {
    if let Err(e) = nuxut::run(Demo::default()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
