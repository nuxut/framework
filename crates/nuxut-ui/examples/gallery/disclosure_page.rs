//! Sayfa 6: açılır satır (`Disclosure`) — kapalı, rozetli, açık (içerikte anahtarlı satırlar),
//! klavye odaklı. Durumlar (`NUXUT_GALLERY_PAGE=6 NUXUT_GALLERY_STATE=…`):
//! - `flash`: açık satırın ikinci içerik satırı parlatılır (arama sonucu; vurgunun içinde donuk);
//! - `mid`: açık satır açılma animasyonunun ortasında dondurulur (ekran görüntüsü için).

use std::time::{Duration, Instant};

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{Disclosure, DisclosureEvent, Label, TextRole, Toggle, Ui, text_style};

use super::Card;

/// İçerikteki bir ayar satırı (başlık + anahtar).
struct Row {
    label: Label,
    toggle: Toggle,
    /// İçerik koordinatında (içeriğin sol-üstü 0,0).
    local: Rect,
}

struct Item {
    d: Disclosure,
    rows: Vec<Row>,
}

pub struct DisclosurePage {
    cards: Vec<Card>,
    items: Vec<Item>,
    /// Klavye odağındaki açılır satır.
    focus: usize,
    /// `mid` durumu: açık satır bu ana dondurulur.
    freeze: Option<Instant>,
    last_event: Label,
    size: Size,
}

const ROW_TITLES: [&str; 3] = ["Gölge", "Bulanıklık", "Kenarlık"];

impl DisclosurePage {
    pub fn new(theme: &Theme) -> Self {
        let now = Instant::now();
        let rows = |n: usize| -> Vec<Row> {
            ROW_TITLES
                .iter()
                .take(n)
                .enumerate()
                .map(|(i, t)| Row { label: Label::new(*t, text_style(theme, TextRole::Body)), toggle: Toggle::new(theme, i != 1), local: Rect::ZERO })
                .collect()
        };
        let mut closed = Disclosure::new(theme, "Renkleri tek tek ayarla").with_description(theme, "Duvar kağıdından üretilen renkleri ezer");
        closed.set_summary("11 renk");
        let mut badged = Disclosure::new(theme, "Gelişmiş bağlantı ayarları");
        badged.set_summary("4 ayar · 2'si elle ayarlı");
        badged.set_badge(true, now);
        let mut open = Disclosure::new(theme, "Efekt ayrıntıları").with_description(theme, "Panellerin derinliği");
        open.set_summary("3 ayar");
        open.set_open(true, false, now);
        let mut focused = Disclosure::new(theme, "Uzman ayarları");
        focused.set_summary("6 ayar");
        focused.set_focus_visible(true);
        Self {
            cards: vec![Card::new(theme, "Açılır satır"), Card::new(theme, "Dar")],
            items: vec![
                Item { d: closed, rows: rows(3) },
                Item { d: badged, rows: rows(2) },
                Item { d: open, rows: rows(3) },
                Item { d: focused, rows: rows(3) },
            ],
            focus: 3,
            freeze: None,
            last_event: Label::new("", text_style(theme, TextRole::Caption)),
            size: Size::ZERO,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for it in &mut self.items {
            it.d.set_theme(theme, now);
            for r in &mut it.rows {
                r.label.set_style(text_style(theme, TextRole::Body));
                r.toggle.set_theme(theme, now);
            }
        }
    }

    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size, now: Instant) {
        let s = |n| theme.space(n);
        self.size = size;
        let m = s(5);
        let narrow_w = s(80).min((size.width - m * 3.0) * 0.35).floor();
        let main_w = (size.width - narrow_w - m * 3.0).min(s(160));
        self.cards[0].rect = Rect::new(m, m, main_w, size.height - m * 2.0);
        self.cards[1].rect = Rect::new(m * 2.0 + main_w, m, narrow_w, size.height - m * 2.0);
        // Ana kart: dört durum alt alta; dar kart: ilk satırın dar genişlikte kopyası değil,
        // aynı bileşen (özet başlığın altına iner) — rozetli satır orada.
        let body = self.cards[0].body(theme);
        let mut y = body.y;
        for (i, it) in self.items.iter_mut().enumerate() {
            let (x, w, top) = if i == 1 {
                let b = self.cards[1].body(theme);
                (b.x - s(2), b.width + s(4), b.y)
            } else {
                (body.x - s(2), body.width + s(4), y)
            };
            let row_h = s(14);
            let mut cy = 0.0;
            for r in &mut it.rows {
                r.local = Rect::new(0.0, cy, w, row_h);
                cy += row_h;
            }
            it.d.set_content_height(cy);
            let _ = it.d.header_height(text, theme, w);
            let t = if i == 2 { self.freeze.unwrap_or(now) } else { now };
            let h = it.d.height(t);
            it.d.set_rect(Rect::new(x, top, w, h));
            let content = it.d.content_rect();
            let pad = s(4);
            for r in &mut it.rows {
                let ts = r.toggle.preferred_size(theme);
                let rr = r.local.translate(Point::new(content.x, content.y));
                r.toggle.set_rect(Rect::new(rr.right() - pad - ts.width, rr.center().y - ts.height * 0.5, ts.width, ts.height));
            }
            if i != 1 {
                y += h + s(2);
            }
        }
    }

    pub fn apply_state(&mut self, state: &str, now: Instant) {
        match state {
            "flash" => {
                let it = &mut self.items[2];
                let r = it.rows[1].local;
                it.d.flash_content(r, now);
                // Ekran görüntüsü zamanlamasından bağımsız: vurgu bekleyişinin içinde donar.
                self.freeze = Some(now + Duration::from_millis(200));
            }
            "mid" => {
                let it = &mut self.items[2];
                it.d.set_open(false, false, now);
                it.d.set_open(true, true, now);
                self.freeze = Some(now + Duration::from_millis(70));
            }
            _ => {}
        }
    }


    fn note(&mut self, s: &str) {
        self.last_event.set_text(s);
    }

    pub fn key(&mut self, e: &ui_input::KeyEvent, now: Instant) -> bool {
        if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Tab {
            let n = self.items.len();
            self.focus = if e.modifiers.shift { (self.focus + n - 1) % n } else { (self.focus + 1) % n };
            for (i, it) in self.items.iter_mut().enumerate() {
                it.d.set_focus_visible(i == self.focus);
            }
            return true;
        }
        let i = self.focus;
        let r = self.items[i].d.key(e, now);
        if let Some(DisclosureEvent::Toggled(open)) = r.event {
            self.note(&format!("Son olay: #{i} {}", if open { "açıldı" } else { "kapandı" }));
            return true;
        }
        r.repaint
    }

    pub fn pointer(&mut self, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        let mut note = None;
        for (i, it) in self.items.iter_mut().enumerate() {
            let r = it.d.pointer(e, now);
            repaint |= r.repaint;
            if let Some(DisclosureEvent::Toggled(open)) = r.event {
                note = Some((i, open));
                repaint = true;
            }
            if it.d.is_open() {
                for row in &mut it.rows {
                    repaint |= row.toggle.pointer(e, now).repaint;
                }
            }
        }
        if let Some((i, open)) = note {
            self.focus = i;
            for it in &mut self.items {
                it.d.set_focus_visible(false);
            }
            self.note(&format!("Son olay: #{i} {}", if open { "açıldı" } else { "kapandı" }));
        }
        repaint
    }

    pub fn cursor_at(&self, p: Point) -> CursorShape {
        CursorShape::pick(self.items.iter().flat_map(|it| {
            let open = it.d.is_open();
            std::iter::once(it.d.cursor(p)).chain(it.rows.iter().map(move |r| if open { r.toggle.cursor(p) } else { None }))
        }))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        for c in &mut self.cards {
            c.draw(ui);
        }
        let real_now = ui.now;
        for (i, it) in self.items.iter_mut().enumerate() {
            // `mid`: yalnızca açık örnek dondurulmuş anda çizilir.
            if i == 2
                && let Some(t) = self.freeze
            {
                ui.now = t;
            }
            it.d.draw(ui);
            let rows = &mut it.rows;
            it.d.draw_content(ui, |ui, content| {
                let pad = ui.theme.space(4);
                for r in rows.iter_mut() {
                    let rr = r.local.translate(Point::new(content.x, content.y));
                    let ls = r.label.measure_in(ui.painter);
                    let color = ui.fade(ui.theme.colors.on_surface);
                    r.label.draw(ui.painter, Point::new(rr.x + pad, rr.center().y - ls.height * 0.5), color);
                    r.toggle.draw(ui);
                }
            });
            ui.now = real_now;
        }
        let at = Point::new(self.cards[0].body(theme).x, self.cards[0].rect.bottom() - theme.space(10));
        ui.damage(Rect::new(at.x, at.y, theme.space(110), theme.space(6)));
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}
