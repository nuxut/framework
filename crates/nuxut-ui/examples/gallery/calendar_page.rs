//! Sayfa 5: takvim bileşenleri — etkinlik satırları ve ayrıntı kartı, küçük palet renk seçici,
//! haftalık zaman ızgarası. Takvim renkleri örnek veridir: temadan üretilen paletten.

use std::time::Instant;

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{
    AgendaItem, CalendarGrid, Date, EventCard, EventRow, Icon, IconName, Label, ResultItem, ResultList, SwatchEvent, SwatchPicker, TextRole, Ui,
    WeekEvent, WeekGrid, WeekGridEvent,
    swatch_palette, text_style,
};

use super::Card;

const DAY: i64 = 86_400;
/// Dar genişlik denemesi (shell Takvim kartı ~260–280; 220/240 tek harfli gün adları için).
const MINI_WIDTHS: [f32; 4] = [220.0, 240.0, 280.0, 320.0];
/// Satır başına locale: tr pazartesi, en_US pazar başlangıçlı ([`CalendarGrid::with_locale`]).
const MINI_LOCALES: [&str; 2] = ["tr_TR", "en_US"];
/// Sağ metinli sonuç listesi genişlikleri.
const RESULT_WIDTHS: [f32; 2] = [280.0, 420.0];

pub struct CalendarPage {
    cards: Vec<Card>,
    rows: Vec<EventRow>,
    card: EventCard,
    swatches: SwatchPicker,
    week: WeekGrid,
    /// `NUXUT_GALLERY_STATE=cal-narrow`: haftalık ızgara yerine MINI_WIDTHS × MINI_LOCALES ay
    /// ızgaraları (satır öncelikli) ve sağ metinli sonuç listeleri.
    minis: Vec<CalendarGrid>,
    mini_card: Card,
    mini_results: Vec<ResultList>,
    today: Date,
    /// Klavye ızgaraya mı gidiyor (son tıklanan bölge).
    week_focus: bool,
    last_event: Label,
    size: Size,
    /// Son yerleşimin teması (imleç sorgusu için).
    theme: Theme,
}

impl CalendarPage {
    /// `now_local`: yerel saniye (Unix + ofset).
    pub fn new(theme: &Theme, today: Date, now_local: i64) -> Self {
        let colors = swatch_palette(theme, 12);
        let (work, home, sport) = (Some(colors[0]), Some(colors[4]), Some(colors[8]));
        let items = [
            AgendaItem::new("Ekip toplantısı", "09:30 – 10:00").location("Oda 3").color(work).past(true),
            AgendaItem::new("Tasarım incelemesi", "11:00 – 12:30").location("Çevrimiçi").color(work).ongoing(true),
            AgendaItem::new("Yüzme", "18:00 – 19:00").color(sport),
            AgendaItem::new("Annemi ara", "Tüm gün").color(home),
        ];
        let rows = items.iter().cloned().map(|i| EventRow::new(theme, i)).collect();
        let card = EventCard::new(
            theme,
            items[1]
                .clone()
                .calendar("İş")
                .description("Yeni ayarlar uygulamasının ekranları: hesaplar, görünüm, klavye. Taslaklar paylaşılan klasörde; yorumlar toplantıdan önce eklensin."),
        );
        let swatches = SwatchPicker::new(theme, colors.clone(), work).with_custom(theme);
        let day = |d: i64, h: i64, m: i64| (today.to_days() + d) * DAY + h * 3600 + m * 60;
        let events = vec![
            WeekEvent::timed(1, "Ekip toplantısı", day(0, 9, 30), day(0, 10, 0)).with_color(work),
            WeekEvent::timed(2, "Tasarım incelemesi", day(0, 11, 0), day(0, 12, 30)).with_color(work),
            WeekEvent::timed(3, "Öğle yemeği", day(0, 12, 0), day(0, 13, 0)).with_color(home),
            WeekEvent::timed(4, "Yüzme", day(0, 18, 0), day(0, 19, 0)).with_color(sport),
            WeekEvent::timed(5, "Sunum provası", day(1, 14, 0), day(1, 16, 0)),
            WeekEvent::timed(6, "Gece yayını", day(2, 22, 0), day(3, 1, 30)).with_color(colors[6].into()),
            WeekEvent::timed(7, "Hatırlatma", day(-1, 8, 0), day(-1, 8, 0)).with_color(home),
            WeekEvent::all_day(8, "Konferans", today.add_days(-1), today.add_days(1)).with_color(colors[2].into()),
            WeekEvent::all_day(9, "Annemi ara", today, today).with_color(home),
        ];
        let mut week = WeekGrid::new(theme, today, &nuxut_ui::locale_from_env());
        week.set_events(theme, events);
        week.set_now(Some(now_local));
        Self {
            cards: vec![Card::new(theme, "Etkinlikler"), Card::new(theme, "")],
            rows,
            card,
            swatches,
            week,
            minis: Vec::new(),
            mini_card: Card::new(theme, "Dar takvim 220 · 240 · 280 · 320 (tr_TR, en_US) · sağ metinli sonuçlar 280 · 420"),
            mini_results: Vec::new(),
            today,
            week_focus: true,
            last_event: Label::new("", text_style(theme, TextRole::Caption)),
            size: Size::ZERO,
            theme: theme.clone(),
        }
    }

    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let s = |n| theme.space(n);
        self.size = size;
        self.theme = theme.clone();
        let m = s(5);
        let left_w = s(96).min((size.width - m * 3.0) * 0.4).floor();
        let h = size.height - m * 2.0;
        self.cards[0].rect = Rect::new(m, m, left_w, h);
        self.cards[1].rect = Rect::new(m * 2.0 + left_w, m, size.width - left_w - m * 3.0, h);
        let body = self.cards[0].body(theme);
        let mut y = body.y;
        let ss = self.swatches.preferred_size(theme, body.width);
        self.swatches.set_rect(Rect::new(body.x, y, ss.width, ss.height));
        y += ss.height + s(4);
        let rh = EventRow::height(theme);
        for r in &mut self.rows {
            r.set_rect(theme, Rect::new(body.x - s(2), y, body.width + s(4), rh));
            y += rh;
        }
        y += s(4);
        let ch = self.card.preferred_height(text, theme, body.width);
        self.card.set_rect(theme, Rect::new(body.x, y, body.width, ch));
        let wb = self.cards[1].body(theme);
        // Kart başlığı yerine ızgaranın kendi başlığı (ay + gezinme) kullanılır.
        let top = self.cards[1].rect.y + s(4);
        self.week.set_rect(theme, Rect::new(wb.x, top, wb.width, wb.bottom() - top - s(8)), Instant::now());
        self.layout_minis(theme);
    }

    fn layout_minis(&mut self, theme: &Theme) {
        let m = theme.space(5);
        self.mini_card.rect = Rect::new(m, m, self.size.width - m * 2.0, self.size.height - m * 2.0);
        let body = self.mini_card.body(theme);
        let gap = theme.space(10);
        let mut y = body.y;
        for row in self.minis.chunks_mut(MINI_WIDTHS.len()) {
            let mut x = body.x;
            let mut row_h: f32 = 0.0;
            for (g, w) in row.iter_mut().zip(MINI_WIDTHS) {
                let gs = g.preferred_size(theme, w);
                g.set_rect(theme, Rect::new(x, y, gs.width, gs.height));
                x += w + gap;
                row_h = row_h.max(gs.height);
            }
            y += row_h + theme.space(8);
        }
        let x = body.x + MINI_WIDTHS.iter().sum::<f32>() + gap * MINI_WIDTHS.len() as f32;
        let mut y = body.y;
        let h = ResultList::row_height(theme) * 5.0;
        for (l, w) in self.mini_results.iter_mut().zip(RESULT_WIDTHS) {
            l.set_rect(theme, Rect::new(x, y, w, h), Instant::now());
            y += h + theme.space(8);
        }
    }

    pub fn apply_state(&mut self, theme: &Theme, state: &str, now: Instant) {
        if state == "week-3" {
            self.week.set_days(theme, 3);
            self.week.scroll_to_minute(theme, 7 * 60, now);
        }
        if state == "cal-narrow" && self.minis.is_empty() {
            let colors = swatch_palette(theme, 12);
            let t = self.today;
            self.minis = MINI_LOCALES
                .iter()
                .flat_map(|loc| MINI_WIDTHS.map(|_| *loc))
                .map(|loc| {
                    let locale = nuxut_locale::Locale::parse(loc).unwrap_or_default();
                    let mut g = CalendarGrid::with_locale(theme, t, &locale);
                    g.set_events(vec![
                        (t, vec![None, Some(colors[4]), Some(colors[8])]),
                        (t.add_days(2), vec![Some(colors[0])]),
                        (t.add_days(5), vec![None, Some(colors[2])]),
                        (t.add_months(-1), vec![None, Some(colors[4])]),
                    ]);
                    g.set_selected(Some(t.add_days(5)));
                    g
                })
                .collect();
            // En dar en_US örneğinde uzun ay adı ("September"): başlık üç noktayla kısalır.
            let _ = self.minis[MINI_WIDTHS.len()].show_month(theme, t.year, 9, now);
            let items = || {
                vec![
                    ResultItem::new("Komut paleti: son açılan dosyalar ve çalışma alanları")
                        .subtitle("Düzenleyici")
                        .icon(Icon::named(IconName::Apps))
                        .trailing("Ctrl+Shift+P"),
                    ResultItem::new("Ekran görüntüsü 2026-10-13 14.02.11 çok uzun dosya adı.png")
                        .subtitle("~/Resimler/Ekran görüntüleri")
                        .icon(Icon::named(IconName::Apps))
                        .trailing("12:30 · 4 KB"),
                    ResultItem::new("Ayarlar").subtitle("Sistem").icon(Icon::named(IconName::Apps)).trailing("Uygulama"),
                    ResultItem::new("Yuvasız uzun başlık: sağ metin olmadan tüm genişlik kullanılır")
                        .subtitle("Karşılaştırma")
                        .icon(Icon::named(IconName::Apps)),
                    ResultItem::new("Takvim").icon(Icon::named(IconName::Apps)).trailing("Çok uzun bir kategori adı, yuvanın sınırını aşar"),
                ]
            };
            self.mini_results = RESULT_WIDTHS
                .iter()
                .map(|_| {
                    let mut l = ResultList::new(theme);
                    l.set_items(theme, items(), now);
                    l
                })
                .collect();
            self.layout_minis(theme);
        }
    }

    fn note(&mut self, s: &str) {
        self.last_event.set_text(s);
    }

    pub fn key(&mut self, theme: &Theme, e: &ui_input::KeyEvent, now: Instant) -> bool {
        if self.week_focus {
            let r = self.week.key(theme, e, now);
            if let Some(ev) = r.event {
                self.on_week(ev);
            }
            return r.repaint;
        }
        let r = self.swatches.key(theme, e, now);
        if let Some(ev) = r.event {
            self.on_swatch(ev);
        }
        r.repaint || r.event.is_some()
    }

    fn on_week(&mut self, ev: WeekGridEvent) {
        let s = match ev {
            WeekGridEvent::EventActivated(k) => format!("etkinlik #{k}"),
            WeekGridEvent::SlotActivated { date, minute } => format!("yeni: {}-{:02}-{:02} {:02}:{:02}", date.year, date.month, date.day, minute / 60, minute % 60),
            WeekGridEvent::DayActivated(d) => format!("gün: {}-{:02}-{:02}", d.year, d.month, d.day),
            WeekGridEvent::RangeChanged { start, days } => format!("aralık: {}-{:02}-{:02} +{days}", start.year, start.month, start.day),
        };
        self.note(&s);
    }

    fn on_swatch(&mut self, ev: SwatchEvent) {
        let s = match ev {
            SwatchEvent::Selected(c) => format!("renk {}", c.to_hex()),
            SwatchEvent::Custom => "özel renk".to_owned(),
        };
        self.note(&s);
    }

    pub fn pointer(&mut self, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        if matches!(e.kind, ui_input::PointerKind::Press(_)) {
            self.week_focus = self.week.rect().contains(e.position);
        }
        let r = self.week.pointer(theme, e, now);
        repaint |= r.repaint;
        if let Some(ev) = r.event {
            self.on_week(ev);
            repaint = true;
        }
        let r = self.swatches.pointer(theme, e, now);
        repaint |= r.repaint;
        if let Some(ev) = r.event {
            self.on_swatch(ev);
            repaint = true;
        }
        for i in 0..self.rows.len() {
            let r = self.rows[i].pointer(e, now);
            repaint |= r.repaint;
            if r.event.is_some() {
                let item = self.rows[i].item().clone();
                self.card.set_item(theme, item);
                self.note(&format!("satır {i}"));
                repaint = true;
            }
        }
        repaint
    }

    pub fn cursor_at(&self, p: Point) -> CursorShape {
        let theme = &self.theme;
        CursorShape::pick(
            [self.week.cursor(theme, p, Instant::now()), self.swatches.cursor(theme, p)].into_iter().chain(self.rows.iter().map(|r| r.cursor(p))),
        )
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        if !self.minis.is_empty() {
            self.mini_card.draw(ui);
            for g in &mut self.minis {
                g.draw(ui);
            }
            for l in &mut self.mini_results {
                l.draw(ui);
            }
            return;
        }
        for c in &mut self.cards {
            c.draw(ui);
        }
        self.swatches.draw(ui);
        for r in &mut self.rows {
            r.draw(ui);
        }
        self.card.draw(ui);
        self.week.draw(ui);
        let at = Point::new(self.cards[1].body(theme).x, self.cards[1].rect.bottom() - theme.space(10));
        ui.damage(Rect::new(at.x, at.y, theme.space(110), theme.space(6)));
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}
