//! Sayfa 6: gezinme listesi — `List` grup başlıkları, yoğun satır ve ikon şeridi kipi
//! (Ayarlar kenar çubuğu biçimi). Son kart canlı: düğme ya da `R` geniş ↔ dar geçişi yayla
//! oynatır; şeritte üzerine gelince/klavye odağında başlık ipucu olarak görünür.
//!
//! Durumlar: `rail-focus` (şerit listesinde klavye odağı + ipucu), `focus` (normal listede
//! klavye odak halkası), `rail` (canlı kart dar başlar), `rail-mid` (canlı kart geçişin
//! ortasında donmuş: ilerleme ≈ 0,5; ekran görüntüsü için).

use std::time::Instant;

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{Button, ButtonVariant, Icon, IconName, Label, List, ListDensity, ListItem, TextRole, Ui, text_style};

use super::{Card, press_key};

/// Ayarlar kenar çubuğu (settings-ux §2.2): üç grup, 13 sayfa.
const PAGES: [(IconName, &str); 13] = [
    (IconName::Palette, "Görünüm"),
    (IconName::Wallpaper, "Duvar kağıdı"),
    (IconName::Font, "Yazı"),
    (IconName::Animation, "Hareket"),
    (IconName::Panel, "Bar"),
    (IconName::Panel, "Paneller"),
    (IconName::Window, "Pencereler"),
    (IconName::Desktop, "Ekranlar"),
    (IconName::Keyboard, "Kısayollar"),
    (IconName::Notifications, "Bildirimler"),
    (IconName::VolumeHigh, "Ses"),
    (IconName::AccountCircle, "Hesaplar"),
    (IconName::Info, "Hakkında"),
];
const GROUPS: [(usize, &str); 3] = [(0, "Kişisel"), (4, "Masaüstü"), (9, "Sistem")];

fn nav(theme: &Theme, density: ListDensity, rail: bool) -> List {
    let items = PAGES.iter().map(|(icon, title)| ListItem::new(theme, *title).with_leading(theme, Icon::named(*icon))).collect();
    let mut list = List::new(theme).with_items(items).with_groups(theme, &GROUPS).with_density(density).with_rail(rail);
    list.items_mut()[0].set_selected(true);
    list
}

pub struct NavPage {
    cards: Vec<Card>,
    /// Normal, yoğun, şerit, canlı (geniş ↔ dar).
    lists: Vec<List>,
    /// Canlı kartın geniş hâldeki genişliği.
    live_full: f32,
    toggle: Button,
    /// Klavye hangi listeye gidiyor (son tıklanan).
    focus: usize,
    last_event: Label,
    /// `rail-mid`: canlı liste bu anda donmuş çizilir (geçiş ortası; kare istenmez).
    frozen: Option<Instant>,
    theme: Theme,
}

impl NavPage {
    pub fn new(theme: &Theme) -> Self {
        let toggle = Button::new(theme, ButtonVariant::Tonal).with_label(theme, "Daralt (R)");
        Self {
            cards: vec![Card::new(theme, "Normal"), Card::new(theme, "Yoğun"), Card::new(theme, "Şerit"), Card::new(theme, "Canlı")],
            lists: vec![
                nav(theme, ListDensity::Regular, false),
                nav(theme, ListDensity::Compact, false),
                nav(theme, ListDensity::Compact, true),
                nav(theme, ListDensity::Compact, false),
            ],
            live_full: 0.0,
            toggle,
            focus: 3,
            frozen: None,
            last_event: Label::new("", text_style(theme, TextRole::Caption)),
            theme: theme.clone(),
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.theme = theme.clone();
        for l in &mut self.lists {
            l.set_theme(theme, now);
        }
    }

    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let s = |n| theme.space(n);
        self.theme = theme.clone();
        let m = s(5);
        let h = size.height - m * 2.0;
        let pad = s(5) * 2.0;
        let rail = List::rail_width(theme);
        let widths = [s(64), s(56), rail + pad, s(56)];
        let mut x = m;
        for (card, w) in self.cards.iter_mut().zip(widths) {
            card.rect = Rect::new(x, m, w, h);
            x += w + m;
        }
        let now = Instant::now();
        for i in 0..3 {
            let body = self.cards[i].body(theme);
            self.lists[i].set_rect(theme, body, now);
        }
        // Düğme uzun etiketine göre boyutlanır (etiket kipe göre değişir).
        self.toggle.set_label("Genişlet (R)");
        let ts = self.toggle.preferred_size(text, theme);
        self.toggle.set_label(if self.lists[3].is_rail() { "Genişlet (R)" } else { "Daralt (R)" });
        let body = self.cards[3].body(theme);
        self.toggle.set_rect(Rect::new(body.x, body.y, ts.width, ts.height));
        self.live_full = body.width;
        self.layout_live(theme, now);
    }

    fn live_rect(&self, theme: &Theme, now: Instant) -> Rect {
        let body = self.cards[3].body(theme);
        let top = self.toggle.rect().bottom() + theme.space(4);
        let w = self.lists[3].animated_width(theme, self.live_full, now);
        Rect::new(body.x, top, w, body.bottom() - top - theme.space(8))
    }

    fn layout_live(&mut self, theme: &Theme, now: Instant) {
        let now = self.frozen.unwrap_or(now);
        let r = self.live_rect(theme, now);
        self.lists[3].set_rect(theme, r, now);
    }

    pub fn apply_state(&mut self, theme: &Theme, state: &str, now: Instant) {
        let down = press_key(ui_input::Key::Down);
        match state {
            "rail-focus" => {
                for _ in 0..4 {
                    let _ = self.lists[2].key(theme, &down, now);
                }
            }
            "focus" => {
                for _ in 0..3 {
                    let _ = self.lists[0].key(theme, &down, now);
                }
            }
            "rail" => {
                self.lists[3] = nav(theme, ListDensity::Compact, true);
                self.toggle.set_label("Genişlet (R)");
                self.layout_live(theme, now);
            }
            "rail-mid" if self.frozen.is_none() => {
                // Yayın ilerlemesi 0,5'i geçtiği ilk milisaniyede dondur.
                self.lists[3].set_rail(true, now);
                self.toggle.set_label("Genişlet (R)");
                let at = (0..2000u64)
                    .map(|ms| now + std::time::Duration::from_millis(ms))
                    .find(|&t| self.lists[3].rail_progress(t) >= 0.5)
                    .unwrap_or(now);
                self.frozen = Some(at);
                self.layout_live(theme, now);
            }
            _ => {}
        }
    }

    fn flip(&mut self, now: Instant) {
        // Donmuş geçiş (`rail-mid`) gerçek zamana döner.
        self.frozen = None;
        let rail = !self.lists[3].is_rail();
        self.lists[3].set_rail(rail, now);
        self.toggle.set_label(if rail { "Genişlet (R)" } else { "Daralt (R)" });
    }

    fn select(&mut self, list: usize, item: usize) {
        for (i, it) in self.lists[list].items_mut().iter_mut().enumerate() {
            it.set_selected(i == item);
        }
        self.last_event.set_text(&format!("Seçildi: {}", PAGES[item].1));
    }

    pub fn pointer(&mut self, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        let r = self.toggle.pointer(e, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            self.flip(now);
            repaint = true;
        }
        for i in 0..self.lists.len() {
            let r = self.lists[i].pointer(theme, e, now);
            repaint |= r.repaint;
            if let Some((item, _)) = r.event {
                self.focus = i;
                self.select(i, item);
            }
        }
        repaint
    }

    /// `false`: tuş kullanılmadı.
    pub fn key(&mut self, theme: &Theme, e: &ui_input::KeyEvent, now: Instant) -> bool {
        if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Char('r') {
            self.flip(now);
            return true;
        }
        let f = self.focus;
        let r = self.lists[f].key(theme, e, now);
        if let Some((item, _)) = r.event {
            self.select(f, item);
        }
        r.repaint
    }

    pub fn cursor_at(&self, p: Point) -> CursorShape {
        CursorShape::pick(self.lists.iter().map(|l| l.cursor(p)).chain([self.toggle.cursor(p)]))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let now = self.frozen.unwrap_or(ui.now);
        if self.lists[3].is_rail_animating(now) || self.lists[3].rect().width != self.lists[3].animated_width(theme, self.live_full, now) {
            self.layout_live(theme, now);
        }
        for c in &mut self.cards {
            c.draw(ui);
        }
        self.toggle.draw(ui);
        let (fixed, live) = self.lists.split_at_mut(3);
        for l in fixed {
            l.draw(ui);
        }
        match self.frozen {
            // Donmuş an: alt Ui'nin kare isteği yok sayılır (görüntü durağan).
            Some(t) => {
                let mut frozen = Ui::new(ui.painter, theme, t);
                live[0].draw(&mut frozen);
            }
            None => live[0].draw(ui),
        }
        let body = self.cards[3].body(theme);
        let at = Point::new(body.x, body.bottom() - theme.space(5));
        ui.damage(Rect::new(at.x, at.y, body.width, theme.space(6)));
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}

