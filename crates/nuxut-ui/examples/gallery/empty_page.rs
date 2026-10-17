//! Sayfa 9, `NUXUT_GALLERY_STATE=empty`: boş durum (`EmptyState`) ve metinli rozet (`Pill`).
//!
//! Üst sıra: aynı bileşen dört alanda — geniş (maskot 96 + gövde + düğme), kısa (maskot
//! küçülür), çok kısa (maskot düşer, yalnız metin ortalı) ve ikonlu (maskotsuz). Geniş alanın yedek
//! ikonu var: `[appearance] mascot = false` ile çalıştırılınca maskotun yerine sessiz zil çizilir
//! (`with_fallback_icon`); diğerlerinde yer kapanır. Alt sıra: satır
//! içi ve köşeye yaslı hap, hap sağ uçlu liste satırları (seçili dahil; düz metin sağ uç
//! karşılaştırma için) ve hap yuvalı sonuç listesi.

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_render::{RectStyle, TextSystem};
use nuxut_theme::Theme;
use nuxut_ui::{
    EmptyState, Icon, IconName, IconView, Label, List, ListItem, ListTrailing, Mood, Pill, ResultItem, ResultList, TextRole, Ui, text_style,
};

/// Üst sıradaki alanlar (genişlik, yükseklik): geniş, kısa, çok kısa, ikonlu.
const PANELS: [(f32, f32); 4] = [(380.0, 340.0), (260.0, 200.0), (260.0, 110.0), (280.0, 240.0)];

pub struct EmptyPage {
    title: Label,
    panels: Vec<(EmptyState, Rect)>,
    captions: Vec<(Label, Point)>,
    inline_label: Label,
    inline_at: Point,
    inline_pill: Pill,
    corner_icon: IconView,
    corner_box: Rect,
    corner_pill: Pill,
    list: List,
    results: ResultList,
}

impl EmptyPage {
    pub fn new(theme: &Theme) -> Self {
        let body = "Yeni bildirimler geldiğinde burada görünür. Rahatsız etmeyin açıkken sessizce birikir.";
        let panels = vec![
            EmptyState::new(theme, "Bildirim yok")
                .with_mascot(theme, Mood::Empty)
                .with_fallback_icon(theme, Icon::named(IconName::NotificationsOff))
                .with_body(theme, body)
                .with_action(theme, "Ayarları aç"),
            EmptyState::new(theme, "Sonuç bulunamadı").with_mascot(theme, Mood::NoResults).with_body(theme, "Başka bir arama deneyin."),
            EmptyState::new(theme, "Bağlantı yok").with_mascot(theme, Mood::Error).with_body(theme, "Ağ ayarlarını denetleyin."),
            EmptyState::new(theme, "Pano boş").with_icon(theme, Icon::named(IconName::Paste)).with_body(theme, "Kopyaladıkların burada listelenir."),
        ];
        let caption = |s: &str| (Label::new(s, text_style(theme, TextRole::Caption)), Point::ZERO);
        let items = vec![
            ListItem::new(theme, "Duvar kağıdı").with_leading(theme, Icon::named(IconName::Wallpaper)).with_trailing(theme, ListTrailing::Pill("Yeni".into())),
            ListItem::new(theme, "Pil").with_subtitle(theme, "Şarj oluyor").with_leading(theme, Icon::named(IconName::Battery60)).with_trailing(theme, ListTrailing::Text("%64".into())),
            {
                let mut it = ListItem::new(theme, "Masaüstü efektleri — uzun başlık hapın soluna sığar")
                    .with_leading(theme, Icon::named(IconName::Tune))
                    .with_trailing(theme, ListTrailing::Pill("Beta".into()));
                it.set_selected(true);
                it
            },
        ];
        Self {
            title: Label::new("Boş durum (EmptyState) · metinli rozet (Pill)", text_style(theme, TextRole::Title)),
            panels: panels.into_iter().map(|p| (p, Rect::ZERO)).collect(),
            captions: vec![caption("geniş: maskot 96"), caption("kısa: maskot küçülür"), caption("çok kısa: maskot düşer"), caption("ikonlu")],
            inline_label: Label::new("Kilit ekranı", text_style(theme, TextRole::Body)),
            inline_at: Point::ZERO,
            inline_pill: Pill::new(theme, "Yeni"),
            corner_icon: IconView::with_role(theme, Icon::named(IconName::Widgets), TextRole::IconLarge),
            corner_box: Rect::ZERO,
            corner_pill: Pill::new(theme, "Yeni"),
            list: List::new(theme).with_items(items),
            results: ResultList::new(theme),
        }
    }

    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let now = Instant::now();
        let pad = theme.space(4);
        let gap = theme.space(4);
        let top = pad + theme.space(12);
        let panel_top = top + theme.space(6);
        let mut x = pad;
        for (((panel, rect), (_, at)), (w, h)) in self.panels.iter_mut().zip(&mut self.captions).zip(PANELS) {
            *rect = Rect::new(x, panel_top, w, h);
            panel.set_rect(*rect);
            *at = Point::new(x, top);
            x += w + gap;
        }
        let y = panel_top + PANELS[0].1 + theme.space(10);
        let col_w = ((size.width - pad * 2.0 - gap) * 0.5).clamp(0.0, 440.0);
        let m = self.inline_label.measure(text);
        self.inline_at = Point::new(pad + theme.space(2), y + theme.space(4));
        self.inline_pill.place_inline(text, theme, self.inline_at.x + m.width + theme.space(2), self.inline_at.y + m.height * 0.5);
        let side = self.corner_icon.size().width + theme.space(4);
        self.corner_box = Rect::new(pad + theme.space(80), y + theme.space(2), side, side);
        self.corner_pill.place_corner(text, theme, Point::new(self.corner_box.right(), self.corner_box.y));
        let ly = y + theme.space(22);
        self.list.set_rect(theme, Rect::new(pad, ly, col_w, theme.space(60)), now);
        self.results.set_rect(theme, Rect::new(pad + col_w + gap, ly, col_w, theme.space(60)), now);
        if self.results.is_empty() {
            let items = vec![
                ResultItem::new("Ayarlar").subtitle("Sistem tercihleri").icon(Icon::named(IconName::Settings)).trailing_pill("Yeni"),
                ResultItem::new("Pano geçmişi").subtitle("Metin · 4 KB").icon(Icon::named(IconName::Paste)).trailing("12:30"),
                ResultItem::new("Ekran görüntüsü").icon(Icon::named(IconName::Screenshot)).trailing_pill("Beta"),
            ];
            self.results.set_items(theme, items, now);
            self.results.set_selected(theme, 2, now);
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.title.set_style(text_style(theme, TextRole::Title));
        for (p, _) in &mut self.panels {
            p.set_theme(theme, now);
        }
        for (l, _) in &mut self.captions {
            l.set_style(text_style(theme, TextRole::Caption));
        }
        self.inline_label.set_style(text_style(theme, TextRole::Body));
        self.inline_pill.set_theme(theme, now);
        self.corner_icon.set_theme(theme, now);
        self.corner_pill.set_theme(theme, now);
        self.list.set_theme(theme, now);
        self.results.set_theme(theme, now);
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let c = &theme.colors;
        let pad = theme.space(4);
        self.title.draw(ui.painter, Point::new(pad, pad), c.on_surface);
        for (label, at) in &mut self.captions {
            label.draw(ui.painter, *at, c.on_surface_variant);
        }
        for (panel, rect) in &mut self.panels {
            ui.painter.rect(*rect, &RectStyle::filled(c.surface_container, theme.shape.large()));
            panel.draw(ui);
        }
        self.inline_label.draw(ui.painter, self.inline_at, c.on_surface);
        self.inline_pill.draw(ui);
        ui.painter.rect(self.corner_box, &RectStyle::filled(c.surface_container, theme.shape.large()));
        self.corner_icon.draw_centered(ui.painter, self.corner_box.center(), c.on_surface_variant);
        self.corner_pill.draw(ui);
        self.list.draw(ui);
        self.results.draw(ui);
    }
}
