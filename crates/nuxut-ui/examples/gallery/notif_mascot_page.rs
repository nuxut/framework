//! Sayfa 9, `NUXUT_GALLERY_STATE=notif-mascot`: bildirim kartında maskot yuvası
//! (`NotificationCard::with_mascot`).
//!
//! Dört kart: kısa iç bildirim (maskot kartı uzatır), uzun gövdeli iç bildirim (metin maskottan
//! uzun), yığın giriş solması ortasında (%50; maskot kartla birlikte solar) ve karşılaştırma için
//! dış uygulama bildirimi (maskot yok). `framework.toml [appearance] mascot = false` ile
//! çalıştırılınca yuva kapanır: metin sola yaslanır, kartlar kısalır.

use std::time::Instant;

use nuxut_core::{Point, Rect};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;
use nuxut_ui::{BodySpan, Icon, IconName, Label, Mood, NotificationCard, NotificationContent, TextRole, Ui, text_style};

/// Kart genişliği (shell bildirim yığını genişliği).
const CARD_WIDTH: f32 = 380.0;

pub struct NotifMascotPage {
    title: Label,
    /// (kart, başlık, dış alfa).
    cards: Vec<(NotificationCard, Label, f32)>,
    /// Kartların ve başlıklarının yeri.
    slots: Vec<(Rect, Point)>,
}

fn internal(title: &str, body: &str, actions: &[&str]) -> NotificationContent {
    NotificationContent {
        app_name: "Nuxut".into(),
        app_icon: Some(Icon::named(IconName::Settings)),
        time: "şimdi".into(),
        title: title.into(),
        body: vec![BodySpan::plain(body)],
        image: None,
        actions: actions.iter().map(|a| (*a).to_owned()).collect(),
    }
}

impl NotifMascotPage {
    pub fn new(theme: &Theme) -> Self {
        let caption = |s: &str| Label::new(s, text_style(theme, TextRole::Caption));
        let updated = internal("Nuxut güncellendi", "Sürüm 0.4 kuruldu.", &["Yenilikler"]);
        let long = internal(
            "Güncelleme tamamlandı",
            "Kabuk, Ayarlar ve tema paketleri yeni sürüme geçti. Değişiklikler bir sonraki oturumda da korunur; \
             ayrıntılar Yenilikler sayfasında.",
            &[],
        );
        let external = NotificationContent {
            app_name: "Mesajlar".into(),
            app_icon: Some(Icon::named(IconName::Chat)),
            time: "2 dk".into(),
            title: "Ayşe Yılmaz".into(),
            body: vec![BodySpan::plain("Akşam yemeğine geliyor musun?")],
            image: None,
            actions: vec!["Yanıtla".into()],
        };
        let cards = vec![
            (NotificationCard::new(theme, updated.clone()).with_mascot(theme, Mood::Celebrate), caption("iç bildirim: maskot 64, karosuz"), 1.0),
            (NotificationCard::new(theme, long).with_mascot(theme, Mood::Celebrate), caption("uzun gövde: metin maskottan uzun"), 1.0),
            (NotificationCard::new(theme, updated).with_mascot(theme, Mood::Celebrate), caption("giriş solması %50: maskot da solar"), 0.5),
            (NotificationCard::new(theme, external), caption("dış uygulama: maskot yok"), 1.0),
        ];
        let n = cards.len();
        Self {
            title: Label::new("Bildirim kartında maskot (NotificationCard::with_mascot)", text_style(theme, TextRole::Title)),
            cards,
            slots: vec![(Rect::ZERO, Point::ZERO); n],
        }
    }

    /// İki sütun × iki satır; satır yüksekliği o satırın en uzun kartı.
    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme) {
        let pad = theme.space(4);
        let gap = theme.space(6);
        let cap_h = theme.space(6);
        let top = pad + theme.space(12);
        let heights: Vec<f32> = self.cards.iter_mut().map(|(c, _, _)| c.measure(text, theme, CARD_WIDTH)).collect();
        let mut y = top;
        for row in 0..heights.len().div_ceil(2) {
            let row_h = heights[row * 2..(row * 2 + 2).min(heights.len())].iter().copied().fold(0.0, f32::max);
            for col in 0..2 {
                let i = row * 2 + col;
                if i >= heights.len() {
                    break;
                }
                let x = pad + col as f32 * (CARD_WIDTH + gap);
                let rect = Rect::new(x, y + cap_h, CARD_WIDTH, heights[i]);
                self.cards[i].0.set_rect(text, theme, rect);
                self.slots[i] = (rect, Point::new(x, y));
            }
            y += cap_h + row_h + gap;
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.title.set_style(text_style(theme, TextRole::Title));
        for (c, l, _) in &mut self.cards {
            c.set_theme(theme, now);
            l.set_style(text_style(theme, TextRole::Caption));
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let c = &theme.colors;
        let pad = theme.space(4);
        let state = if theme.appearance.mascot { "maskot açık" } else { "maskot kapalı ([appearance] mascot = false)" };
        self.title.draw(ui.painter, Point::new(pad, pad), c.on_surface);
        let mut note = Label::new(state, text_style(theme, TextRole::Caption));
        note.draw(ui.painter, Point::new(pad, pad + theme.space(7)), c.on_surface_variant);
        for ((card, label, alpha), (_, at)) in self.cards.iter_mut().zip(&self.slots) {
            label.draw(ui.painter, *at, c.on_surface_variant);
            ui.with_alpha(*alpha, |ui| card.draw(ui));
        }
    }
}
