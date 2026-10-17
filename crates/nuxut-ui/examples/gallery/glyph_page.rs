//! Sayfa 10: glif ızgarası (`GlyphGrid`) — emoji seçici (bölüm başlıklı, ~500 öğe, sanal) ve
//! sabit 6 sütunlu sembol ızgarası (tek renk glifler). Tab ızgaralar arasında odak değiştirir.
//! Durumlar (`NUXUT_GALLERY_PAGE=10 NUXUT_GALLERY_STATE=…`):
//! - (yok): emoji ızgarasında klavye odağı (halka) + seçim, sembol ızgarasında fare seçimi;
//! - `section`: emoji ızgarası "Hayvanlar" bölümüne atlamış (başlık tepede).

use std::time::Instant;

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{GlyphColumns, GlyphGrid, GlyphGridEvent, GlyphItem, GlyphSection, Label, TextRole, Ui, text_style};

use super::Card;

/// Kod noktası aralığından öğeler (ad: "U+XXXX"; gerçek uygulama CLDR adlarını verir).
fn range(from: u32, to: u32) -> Vec<GlyphItem> {
    (from..=to).filter_map(char::from_u32).map(|c| GlyphItem::new(c.to_string(), format!("U+{:04X}", c as u32))).collect()
}

fn emoji_sections() -> Vec<GlyphSection> {
    let recent = ["👍🏽", "👩‍💻", "🏳️‍🌈", "❤️", "🎉", "🐧", "🚀", "☕", "✅"]
        .iter()
        .map(|s| GlyphItem::new(*s, format!("{} kod noktası", s.chars().count())))
        .collect();
    vec![
        GlyphSection::new("Son kullanılanlar", recent),
        GlyphSection::new("Yüzler", range(0x1F600, 0x1F64F)),
        GlyphSection::new("Hayvanlar", range(0x1F400, 0x1F43F)),
        GlyphSection::new("Yiyecek", range(0x1F345, 0x1F37F)),
        GlyphSection::new("Nesneler ve simgeler", range(0x1F380, 0x1F3FA)),
        GlyphSection::new("Çeşitli", range(0x1F4A0, 0x1F4FC)),
        GlyphSection::new("Ulaşım", range(0x1F680, 0x1F6C5)),
    ]
}

fn symbols() -> Vec<GlyphItem> {
    let mut v = range(0x2190, 0x21FF); // oklar
    v.extend(range(0x2200, 0x22FF)); // matematik
    v
}

pub struct GlyphPage {
    cards: Vec<Card>,
    grids: [GlyphGrid; 2],
    focus: usize,
    status: Label,
    size: Size,
}

impl GlyphPage {
    pub fn new(theme: &Theme) -> Self {
        let now = Instant::now();
        let mut emoji = GlyphGrid::new(theme);
        emoji.set_sections(theme, emoji_sections(), now);
        let mut sym = GlyphGrid::new(theme);
        sym.set_columns(theme, GlyphColumns::Fixed(6), now);
        sym.set_items(theme, symbols(), now);
        Self {
            cards: vec![Card::new(theme, "Emoji (bölümlü, sanal)"), Card::new(theme, "Semboller (6 sütun)")],
            grids: [emoji, sym],
            focus: 0,
            status: Label::new("", text_style(theme, TextRole::Caption)),
            size: Size::ZERO,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for g in &mut self.grids {
            g.set_theme(theme, now);
        }
        self.status.set_style(text_style(theme, TextRole::Caption));
    }

    pub fn layout(&mut self, theme: &Theme, size: Size, now: Instant) {
        let m = theme.space(5);
        self.size = size;
        let h = (size.height - m * 2.0).min(theme.space(150));
        let w0 = theme.space(130).min((size.width - m * 3.0) * 0.6).floor();
        let w1 = theme.space(80);
        self.cards[0].rect = Rect::new(m, m, w0, h);
        self.cards[1].rect = Rect::new(m * 2.0 + w0, m, w1, h * 0.6);
        for (card, g) in self.cards.iter().zip(&mut self.grids) {
            let b = card.body(theme);
            g.set_rect(theme, Rect::new(b.x, b.y, b.width, b.height - theme.space(8)), now);
        }
    }

    pub fn apply_state(&mut self, state: &str, now: Instant) {
        self.grids[0].set_selected(13, now);
        self.grids[0].set_focus_visible(true);
        self.grids[1].set_selected(8, now);
        if state == "section" {
            let _ = self.grids[0].reveal_section(2, now);
        }
        self.describe();
    }

    fn describe(&mut self) {
        let g = &self.grids[self.focus];
        let s = g.selected().and_then(|i| Some((g.item(i)?.glyph.clone(), g.label(i)?.to_owned())));
        let text = match s {
            Some((glyph, label)) => format!("Seçili: {glyph}  {label}  ·  {} öğe, {} sütun, {} şekillendirildi", g.len(), g.columns(), g.shaped_cells()),
            None => String::new(),
        };
        self.status.set_text(&text);
    }

    pub fn key(&mut self, e: &ui_input::KeyEvent, now: Instant) -> bool {
        if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Tab {
            self.focus = 1 - self.focus;
            for (i, g) in self.grids.iter_mut().enumerate() {
                g.set_focus_visible(i == self.focus);
            }
            self.describe();
            return true;
        }
        let r = self.grids[self.focus].key(e, now);
        if let Some(GlyphGridEvent::Selected(_) | GlyphGridEvent::Activated(_)) = r.event {
            self.describe();
            return true;
        }
        r.repaint
    }

    pub fn pointer(&mut self, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        for i in 0..self.grids.len() {
            let r = self.grids[i].pointer(theme, e, now);
            repaint |= r.repaint;
            if let Some(GlyphGridEvent::Activated(_)) = r.event {
                // Fareyle odak: halka gizlenir (DESIGN "Odak").
                self.focus = i;
                for g in &mut self.grids {
                    g.set_focus_visible(false);
                }
                self.describe();
                repaint = true;
            }
        }
        repaint
    }

    pub fn cursor_at(&self, p: Point) -> CursorShape {
        CursorShape::pick(self.grids.iter().map(|g| g.cursor(p)))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        for c in &mut self.cards {
            c.draw(ui);
        }
        for g in &mut self.grids {
            g.draw(ui);
        }
        let b = self.cards[0].body(theme);
        let at = Point::new(b.x, self.cards[0].rect.bottom() - theme.space(10));
        ui.damage(Rect::new(at.x, at.y, b.width, theme.space(6)));
        self.status.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}
