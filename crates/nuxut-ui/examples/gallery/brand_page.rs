//! Sayfa 9: marka işareti (`BrandMark`, Kara Sağan) — sol yarı etkin tema (varsayılan koyu),
//! sağ yarı aynı temanın açık paleti (`Palette::light()`); her satır bir biçim (tek renk
//! `on_surface`, vurgu `primary`, iki ton, karo) 16/24/48/128 px. Maskeler bir kez
//! rasterleşir; renk çizimde verilir (kip değişiminde yeniden raster yok).

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_theme::{Palette, Theme};
use nuxut_ui::{BrandMark, Label, TextRole, Ui, text_style};

const SIZES: [f32; 4] = [16.0, 24.0, 48.0, 128.0];
const ROWS: [&str; 4] = ["Tek renk · on_surface", "Vurgu · primary", "İki ton", "Karo (uygulama ikonu)"];
const SLOGAN: &str = "The penguin can't fly. This one can.";

fn mark(row: usize, size: f32) -> BrandMark {
    let m = BrandMark::new().with_size(size);
    match row {
        0 => m,
        1 => m.accent(),
        2 => m.duotone(),
        _ => m.tile(),
    }
}

struct Half {
    title: Label,
    rect: Rect,
    /// Satır başlıkları ve konumları.
    captions: Vec<(Label, Point)>,
    /// Satır × boy.
    marks: Vec<BrandMark>,
    /// Karo satırının arkasındaki kart (karo `surface`, kart üstünde görünür).
    band: Rect,
}

pub struct BrandPage {
    halves: [Half; 2],
    slogan: Label,
    slogan_at: Point,
}

fn light(theme: &Theme) -> Theme {
    let mut t = theme.clone();
    t.colors = Palette::light();
    t
}

impl BrandPage {
    pub fn new(theme: &Theme) -> Self {
        let half = |title: &str| Half {
            title: Label::new(title, text_style(theme, TextRole::Title)),
            rect: Rect::ZERO,
            captions: ROWS.iter().map(|r| (Label::new(*r, text_style(theme, TextRole::Caption)), Point::ZERO)).collect(),
            band: Rect::ZERO,
            marks: (0..ROWS.len()).flat_map(|row| SIZES.iter().map(move |s| mark(row, *s))).collect(),
        };
        Self {
            halves: [half("Koyu (etkin tema)"), half("Açık (Palette::light)")],
            slogan: Label::new(SLOGAN, text_style(theme, TextRole::Body)),
            slogan_at: Point::ZERO,
        }
    }

    pub fn layout(&mut self, theme: &Theme, size: Size) {
        let pad = theme.space(6);
        let gap = theme.space(6);
        let caption_w = theme.space(44);
        let w = size.width * 0.5;
        let header = theme.space(14);
        let row_h = ((size.height - header - pad * 2.0) / ROWS.len() as f32).min(SIZES[3] + theme.space(8));
        for (i, h) in self.halves.iter_mut().enumerate() {
            h.rect = Rect::new(w * i as f32, 0.0, w, size.height);
            for (row, (_, at)) in h.captions.iter_mut().enumerate() {
                *at = Point::new(h.rect.x + pad, header + row_h * row as f32 + row_h * 0.5 - theme.space(2));
            }
            for (k, m) in h.marks.iter_mut().enumerate() {
                let (row, col) = (k / SIZES.len(), k % SIZES.len());
                let x0 = h.rect.x + pad + caption_w + SIZES[..col].iter().map(|s| s + gap).sum::<f32>();
                let y = header + row_h * row as f32 + (row_h - SIZES[col]) * 0.5;
                m.set_rect(Rect::new(x0, y, SIZES[col], SIZES[col]));
            }
            let last = ROWS.len() - 1;
            let marks_w: f32 = SIZES.iter().map(|s| s + gap).sum::<f32>() + gap;
            h.band = Rect::new(h.rect.x + pad * 0.5, header + row_h * last as f32 + theme.space(1), caption_w + marks_w, row_h - theme.space(2));
        }
        self.slogan_at = Point::new(pad, pad + theme.space(8));
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for h in &mut self.halves {
            h.title.set_style(text_style(theme, TextRole::Title));
            for (c, _) in &mut h.captions {
                c.set_style(text_style(theme, TextRole::Caption));
            }
            for m in &mut h.marks {
                m.set_theme(theme, now);
            }
        }
        self.slogan.set_style(text_style(theme, TextRole::Body));
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let light = light(ui.theme);
        let (now, track) = (ui.now, ui.tracks_damage());
        let pad = ui.theme.space(6);
        for (i, h) in self.halves.iter_mut().enumerate() {
            let theme = if i == 0 { ui.theme } else { &light };
            let mut sub = Ui::new(&mut *ui.painter, theme, now).with_damage_tracking(track);
            let c = &theme.colors;
            sub.painter.rect(h.rect, &RectStyle::filled(c.surface, Corners::ZERO));
            sub.painter.rect(h.band, &RectStyle::filled(c.surface_container, theme.shape.medium()));
            h.title.draw(sub.painter, Point::new(h.rect.x + pad, pad), c.on_surface);
            for (label, at) in &mut h.captions {
                label.draw(sub.painter, *at, c.on_surface_variant);
            }
            for m in &mut h.marks {
                m.draw(&mut sub);
            }
            if sub.wants_frame() {
                ui.request_frame();
            }
        }
        let c = ui.theme.colors.on_surface_variant;
        self.slogan.draw(ui.painter, self.slogan_at, c);
    }
}
