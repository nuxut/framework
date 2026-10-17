//! Sayfa 9, `NUXUT_GALLERY_STATE=mascot`: maskot Fiyu (`Mascot`) — üst yarı etkin tema
//! (varsayılan koyu), alt yarı açık palet (`Palette::light()`); sütunlar yedi ruh hâli, satırlar
//! 160/96/48 px iki ton ve 48 px tek renk (`primary`). Raster fiziksel boyutta bir kez; tema
//! değişince iki tonlu yeniden üretilir.

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_render::RectStyle;
use nuxut_theme::{ColorRole, Palette, Theme, ThemeMode};
use nuxut_ui::{Label, Mascot, Mood, TextRole, Ui, text_style};

const SIZES: [f32; 3] = [160.0, 96.0, 48.0];
/// Son satır: tek renk, bu boyda.
const MONO_SIZE: f32 = 48.0;

struct Half {
    title: Label,
    rect: Rect,
    captions: Vec<(Label, Point)>,
    /// Satır × mood (son satır mono).
    mascots: Vec<Mascot>,
}

pub struct MascotPage {
    halves: [Half; 2],
}

fn light(theme: &Theme) -> Theme {
    let mut t = theme.clone();
    t.colors = Palette::light();
    t
}

fn rows() -> impl Iterator<Item = (f32, bool)> {
    SIZES.iter().map(|s| (*s, false)).chain(std::iter::once((MONO_SIZE, true)))
}

impl MascotPage {
    pub fn new(theme: &Theme) -> Self {
        let half = |title: &str| Half {
            title: Label::new(title, text_style(theme, TextRole::Title)),
            rect: Rect::ZERO,
            captions: Mood::ALL.iter().map(|m| (Label::new(m.name(), text_style(theme, TextRole::Caption)), Point::ZERO)).collect(),
            mascots: rows()
                .flat_map(|(size, mono)| {
                    Mood::ALL.into_iter().map(move |mood| {
                        let m = Mascot::new(theme, mood, size);
                        if mono { m.mono(ColorRole::Primary) } else { m }
                    })
                })
                .collect(),
        };
        Self { halves: [half(&active_title(theme)), half("Fiyu · açık palet (Palette::light)")] }
    }

    pub fn layout(&mut self, theme: &Theme, size: Size) {
        let pad = theme.space(4);
        let gap = theme.space(2);
        let h = size.height * 0.5;
        let col_w = ((size.width - pad * 2.0) / Mood::ALL.len() as f32).min(SIZES[0] + theme.space(4));
        let header = theme.space(18);
        for (i, half) in self.halves.iter_mut().enumerate() {
            half.rect = Rect::new(0.0, h * i as f32, size.width, h);
            let top = half.rect.y + header;
            for (col, (_, at)) in half.captions.iter_mut().enumerate() {
                *at = Point::new(pad + col_w * col as f32, top - theme.space(6));
            }
            let mut y = top;
            for (row, (side, _)) in rows().enumerate() {
                for col in 0..Mood::ALL.len() {
                    let m = &mut half.mascots[row * Mood::ALL.len() + col];
                    m.set_rect(Rect::new(pad + col_w * col as f32, y, side.min(col_w), side));
                }
                y += side + gap;
            }
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        let light = light(theme);
        for (i, h) in self.halves.iter_mut().enumerate() {
            let t = if i == 0 { theme } else { &light };
            if i == 0 {
                h.title.set_text(&active_title(theme));
            }
            h.title.set_style(text_style(t, TextRole::Title));
            for (c, _) in &mut h.captions {
                c.set_style(text_style(t, TextRole::Caption));
            }
            for m in &mut h.mascots {
                m.set_theme(t, now);
            }
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let light = light(ui.theme);
        let (now, track) = (ui.now, ui.tracks_damage());
        let pad = ui.theme.space(4);
        for (i, h) in self.halves.iter_mut().enumerate() {
            let theme = if i == 0 { ui.theme } else { &light };
            let mut sub = Ui::new(&mut *ui.painter, theme, now).with_damage_tracking(track);
            let c = &theme.colors;
            sub.painter.rect(h.rect, &RectStyle::filled(c.surface, Corners::ZERO));
            h.title.draw(sub.painter, Point::new(pad, h.rect.y + pad), c.on_surface);
            for (label, at) in &mut h.captions {
                label.draw(sub.painter, *at, c.on_surface_variant);
            }
            for m in &mut h.mascots {
                m.draw(&mut sub);
            }
            if sub.wants_frame() {
                ui.request_frame();
            }
        }
    }
}

/// Üst yarının başlığı: etkin temanın kipi ve karşıtlığı.
fn active_title(theme: &Theme) -> String {
    let mode = if theme.mode == ThemeMode::Dark { "koyu" } else { "açık" };
    let contrast = if theme.contrast.is_high() { ", yüksek karşıtlık" } else { "" };
    let hidden = if theme.appearance.mascot { "" } else { " — [appearance] mascot = false: gizli" };
    format!("Fiyu · etkin tema ({mode}{contrast}){hidden}")
}
