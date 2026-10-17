//! Sayfa 4: shell'in günlük bileşenleri — hızlı ayar kutucukları + alt sayfalar (cihaz
//! satırları), OSD kapsülü, başlatıcı (arama + 5000 öğelik sanal sonuç listesi).

use std::ops::Range;
use std::time::{Duration, Instant};

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{
    Button, ButtonVariant, DeviceEvent, DeviceKind, DeviceRow, DeviceState, Icon, Label, MenuAlign, MenuAnchor, MenuSide, Osd,
    PageHeader, PageStack, ResultEvent, ResultItem, ResultList, SearchField, TextInputEvent, TextRole, TileEvent, TileGrid,
    TileItem, Ui, IconName, FontEvent, FontPicker, FontPickerLabels, text_style,
};

use super::Card;

/// Ses OSD aşımı (%150).
const VOLUME_MAX: f32 = 1.5;
/// OSD görünür kalma süresi (çağıranın zamanlayıcısı).
pub const OSD_TIMEOUT: Duration = Duration::from_millis(1500);
/// Başlatıcıdaki öğe sayısı (ölçüm için).
pub const LAUNCHER_ITEMS: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QsPage {
    Root,
    Wifi,
    Bluetooth,
}

const TILES: [(IconName, &str, Option<&str>, bool, bool); 6] = [
    (IconName::Wifi, "Wi-Fi", Some("ornek-ag"), true, true),
    (IconName::Bluetooth, "Bluetooth", Some("2 cihaz"), true, true),
    (IconName::DarkMode, "Karanlık tema", None, true, false),
    (IconName::DoNotDisturb, "Rahatsız etme", None, false, false),
    (IconName::Flight, "Uçak modu", None, false, false),
    (IconName::NightLight, "Gece ışığı", Some("22:00'a kadar"), false, false),
];

/// Kurgusal uygulama adları: hece birleşimleri (5000 benzersiz ad).
fn app_names() -> Vec<String> {
    const A: [&str; 10] = ["Nova", "Lira", "Mavi", "Pusula", "Kıvılcım", "Orman", "Deniz", "Yıldız", "Kuzey", "Bulut"];
    const B: [&str; 10] = ["Not", "Takvim", "Müzik", "Harita", "Posta", "Foto", "Kod", "Hesap", "Okur", "Sohbet"];
    const C: [&str; 50] = [
        "", " Pro", " Lite", " 2", " Plus", " Studio", " Mini", " Go", " Max", " One", " X", " Air", " Edu", " Dev", " Beta", " Nightly",
        " Classic", " Next", " Neo", " Prime", " Flow", " Box", " Hub", " Kit", " Lab", " Desk", " Pad", " Cloud", " Sync", " Link",
        " Zen", " Pop", " Dash", " Snap", " Wave", " Spark", " Core", " Base", " Home", " Work", " Play", " Note", " Book", " Cast",
        " View", " Draw", " Mail", " Chat", " Map", " Tune",
    ];
    let mut out = Vec::with_capacity(LAUNCHER_ITEMS);
    for c in C {
        for a in A {
            for b in B {
                out.push(format!("{a}{b}{c}"));
            }
        }
    }
    out
}

/// Sırayla (büyük/küçük harf duyarsız) alt dizi eşleşmesi; eşleşen bayt aralıkları.
fn subsequence(hay: &str, needle: &str) -> Option<Vec<Range<usize>>> {
    let mut want = needle.chars().flat_map(char::to_lowercase).peekable();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (i, ch) in hay.char_indices() {
        let Some(&w) = want.peek() else { break };
        if ch.to_lowercase().eq(std::iter::once(w)) {
            want.next();
            let end = i + ch.len_utf8();
            match ranges.last_mut() {
                Some(r) if r.end == i => r.end = end,
                _ => ranges.push(i..end),
            }
        }
    }
    want.peek().is_none().then_some(ranges)
}

/// Hızlı ayar sayfalarının içeriği (kökte kutucuklar, alt sayfalarda başlık + cihaz satırları).
struct QsContent {
    tiles: TileGrid,
    header: PageHeader,
    wifi: Vec<DeviceRow>,
    bluetooth: Vec<DeviceRow>,
}

impl QsContent {
    fn rows(&self, page: QsPage) -> &[DeviceRow] {
        match page {
            QsPage::Wifi => &self.wifi,
            QsPage::Bluetooth => &self.bluetooth,
            QsPage::Root => &[],
        }
    }

    fn rows_mut(&mut self, page: QsPage) -> &mut [DeviceRow] {
        match page {
            QsPage::Wifi => &mut self.wifi,
            QsPage::Bluetooth => &mut self.bluetooth,
            QsPage::Root => &mut [],
        }
    }

    /// Sayfanın boyutu; `target`: satırların yay hedefi (yoksa bu karedeki yükseklik).
    fn size(&self, theme: &Theme, page: QsPage, width: f32, target: bool, now: Instant) -> Size {
        match page {
            QsPage::Root => TileGrid::size_for(theme, TILES.len(), width),
            p => {
                let h: f32 = self
                    .rows(p)
                    .iter()
                    .map(|r| if target { r.target_height(theme) } else { r.height(theme, now) } + theme.space(1))
                    .sum();
                Size::new(width, theme.space(14) + h)
            }
        }
    }

    /// Sayfayı `origin`e yerleştirir (geçişte eski ve yeni sayfa ayrı kökenlerde).
    fn place(&mut self, text: &mut TextSystem, theme: &Theme, page: QsPage, origin: Point, width: f32, now: Instant) {
        match page {
            QsPage::Root => {
                self.tiles.layout(theme, origin, width);
            }
            p => {
                let hs = self.header.preferred_size(text, theme);
                self.header.set_rect(Rect::new(origin.x, origin.y, hs.width, hs.height));
                let mut y = origin.y + theme.space(14);
                for r in self.rows_mut(p) {
                    let h = r.height(theme, now);
                    r.set_rect(text, theme, Rect::new(origin.x, y, width, h));
                    y += h + theme.space(1);
                }
            }
        }
    }

    fn draw(&mut self, ui: &mut Ui, page: QsPage) {
        match page {
            QsPage::Root => self.tiles.draw(ui),
            p => {
                self.header.draw(ui);
                for r in self.rows_mut(p) {
                    r.draw(ui);
                }
            }
        }
    }
}

pub struct ShellPage {
    cards: Vec<Card>,
    qs: PageStack<QsPage>,
    qc: QsContent,
    osd: Osd,
    osd_buttons: Vec<Button>,
    volume: f32,
    brightness: f32,
    /// Çağıran (galeri) bu süreyle zamanlayıcı kurar; dolunca `osd_timeout`.
    pub osd_timer: Option<Duration>,
    search: SearchField,
    /// Font seçici (aileler ilk yerleşimde `TextSystem::families()`'ten).
    fonts: Option<FontPicker>,
    /// Klavye font seçiciye mi gidiyor (son tıklanan arama alanı).
    font_focus: bool,
    results: ResultList,
    names: Vec<String>,
    last_event: Label,
    size: Size,
}

impl ShellPage {
    pub fn new(theme: &Theme) -> Self {
        let tiles = TILES
            .iter()
            .map(|(i, l, s, on, ex)| {
                let t = TileItem::new(Icon::named(*i), *l).on(*on).expandable(*ex);
                match s {
                    Some(s) => t.subtitle(*s),
                    None => t,
                }
            })
            .collect();
        let wifi = [
            ("ornek-ag", 92, true, DeviceState::Connected, "Bağlı"),
            ("Kafe Misafir", 64, false, DeviceState::Available, "Açık ağ"),
            ("Ev-5G", 40, true, DeviceState::Saved, "Kayıtlı"),
            ("Komşu ağı", 18, true, DeviceState::Available, "Şifreli"),
            ("Eski yönlendirici", 5, true, DeviceState::Unavailable, "Kapsama dışında"),
        ]
        .into_iter()
        .map(|(n, s, sec, st, label)| {
            let mut r = DeviceRow::new(theme, n, DeviceKind::Wifi { strength: s, secured: sec }, st);
            r.set_status_text(label);
            r.set_password_placeholder("Parola");
            r
        })
        .collect();
        let bluetooth = [
            // Cihaz türü ikonu `DeviceRow::with_icon` ile; `None` varsayılan Bluetooth ikonu.
            ("Kulaklık", Some(80), DeviceState::Connected, "Bağlı · %80", Some(IconName::Headphones)),
            ("Klavye", Some(15), DeviceState::Connecting, "Bağlanıyor…", Some(IconName::Keyboard)),
            ("Telefon", Some(55), DeviceState::Saved, "Eşleşmiş", Some(IconName::Phone)),
            ("Saat", None, DeviceState::Available, "Yakında", Some(IconName::Watch)),
            ("Oyun kolu", None, DeviceState::Available, "Yakında", Some(IconName::Gamepad)),
            ("Bilinmeyen cihaz", None, DeviceState::Saved, "Eşleşmiş", None),
        ]
        .into_iter()
        .map(|(n, b, st, label, icon)| {
            let mut r = DeviceRow::new(theme, n, DeviceKind::Bluetooth { battery: b }, st);
            r.set_icon(icon.map(Icon::named));
            r.set_status_text(label);
            r
        })
        .collect();
        let names = app_names();
        let mut page = Self {
            cards: ["Hızlı ayarlar", "OSD · Font seçici", "Başlatıcı"].into_iter().map(|t| Card::new(theme, t)).collect(),
            qs: PageStack::new(theme, QsPage::Root, Size::ZERO),
            qc: QsContent { tiles: TileGrid::new(theme, tiles), header: PageHeader::new(theme, "Wi-Fi"), wifi, bluetooth },
            osd: Osd::new(theme),
            osd_buttons: [(IconName::VolumeMedium, "Ses −"), (IconName::VolumeHigh, "Ses +"), (IconName::Brightness, "Parlaklık")]
                .into_iter()
                .map(|(i, l)| Button::new(theme, ButtonVariant::Tonal).with_icon(theme, Icon::named(i)).with_label(theme, l))
                .collect(),
            volume: 0.5,
            brightness: 0.3,
            osd_timer: None,
            search: SearchField::new(theme, "Uygulama ara…"),
            fonts: None,
            font_focus: false,
            results: ResultList::new(theme),
            names,
            last_event: Label::new("Son olay: —", text_style(theme, TextRole::Caption)),
            size: Size::ZERO,
        };
        page.filter(theme, Instant::now());
        page
    }

    fn filter(&mut self, theme: &Theme, now: Instant) {
        let q = self.search.text().trim().to_owned();
        // Emoji seçici örneği: ikon yuvasında glif (renkli emoji ve düz harf).
        let glyphs = [("Gülen yüz", "😀"), ("Kalp", "❤️"), ("Türkiye bayrağı", "🇹🇷"), ("Omega", "Ω")];
        let glyph_rows = glyphs.into_iter().filter_map(|(name, g)| {
            let m = if q.is_empty() { Some(Vec::new()) } else { subsequence(name, &q) }?;
            Some(ResultItem::new(name).subtitle("Emoji").glyph(g).matches(m))
        });
        let items = glyph_rows
            .chain(self.names.iter().filter_map(|n| {
                let m = if q.is_empty() { Some(Vec::new()) } else { subsequence(n, &q) }?;
                let item = ResultItem::new(n.clone()).subtitle("Uygulama").icon(Icon::named(IconName::Apps)).matches(m);
                // Sağ yuva örneği (pano öğesi gibi): her üçüncü satırda.
                Some(if n.len() % 3 == 0 { item.trailing("12:30 · 4 KB") } else { item })
            }))
            .collect();
        self.results.set_items(theme, items, now);
    }

    fn page_size(&self, theme: &Theme, page: QsPage, target: bool) -> Size {
        let w = self.cards[0].body(theme).width;
        self.qc.size(theme, page, w, target, Instant::now())
    }

    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let s = |n| theme.space(n);
        self.size = size;
        let m = s(5);
        let col_w = ((size.width - m * 4.0) / 3.0).floor();
        let h = size.height - m * 2.0;
        for (i, c) in self.cards.iter_mut().enumerate() {
            c.rect = Rect::new(m + (col_w + m) * i as f32, m, col_w, h);
        }
        let target = self.page_size(theme, *self.qs.current(), true);
        self.qs.set_size(target, Instant::now());
        let body = self.cards[0].body(theme);
        self.qc.place(text, theme, *self.qs.current(), body.origin(), body.width, Instant::now());

        let body = self.cards[1].body(theme);
        let mut x = body.x;
        for b in &mut self.osd_buttons {
            let bs = b.preferred_size(text, theme);
            b.set_rect(Rect::new(x, body.y, bs.width, bs.height));
            x += bs.width + s(2);
        }
        let fonts = self.fonts.get_or_insert_with(|| {
            let mut p = FontPicker::new(theme, text.families(), font_labels(), None);
            let fam = text.effective_family(&theme.typography.ui.family, nuxut_render::GenericFamily::SansSerif);
            p.set_selected(theme, &fam, 400, Instant::now());
            p
        });
        let top = body.y + s(16);
        fonts.set_rect(text, theme, Rect::new(body.x, top, body.width, body.bottom() - top - s(8)), Instant::now());

        let body = self.cards[2].body(theme);
        let ss = self.search.preferred_size(theme, body.width);
        self.search.set_rect(theme, Rect::new(body.x, body.y, ss.width, ss.height));
        let ly = body.y + ss.height + s(3);
        self.results.set_rect(theme, Rect::new(body.x, ly, body.width, body.bottom() - ly - s(8)), Instant::now());
    }

    fn event(&mut self, t: String) {
        self.last_event.set_text(&t);
    }

    fn show_osd(&mut self, theme: &Theme, i: usize, now: Instant) {
        // Ses %150'ye kadar (aşım), parlaklık %100.
        self.osd.set_max(if i < 2 { VOLUME_MAX } else { 1.0 }, now);
        let (icon, v) = match i {
            0 | 1 => {
                self.volume = (self.volume + if i == 0 { -0.1 } else { 0.1 }).clamp(0.0, VOLUME_MAX);
                (Icon::named(IconName::volume(pct(self.volume), false)), self.volume)
            }
            _ => {
                self.brightness = if self.brightness >= 0.95 { 0.1 } else { (self.brightness + 0.2).min(1.0) };
                (Icon::named(IconName::brightness(pct(self.brightness))), self.brightness)
            }
        };
        // Kapsül kartın alt kenarından (kenara yapışık) büyür.
        let card = self.cards[1].rect;
        let edge = Rect::new(card.center().x, card.bottom(), 0.0, 0.0);
        let anchor = MenuAnchor { rect: edge, side: MenuSide::Above, align: MenuAlign::Center, gap: 0.0 };
        self.osd.show(theme, icon, v, anchor, card, now);
        self.osd_timer = Some(OSD_TIMEOUT);
    }

    pub fn osd_timeout(&mut self, now: Instant) {
        self.osd.hide(now);
    }

    fn open_sub(&mut self, theme: &Theme, page: QsPage, now: Instant) {
        self.qc.header.set_title(if page == QsPage::Wifi { "Wi-Fi" } else { "Bluetooth" });
        let size = self.page_size(theme, page, true);
        self.qs.push(page, size, now);
    }

    fn back(&mut self, theme: &Theme, now: Instant) {
        let size = self.page_size(theme, QsPage::Root, true);
        let _ = self.qs.pop(size, now);
    }

    /// Senaryo durumları (ekran görüntüsü).
    pub fn apply_state(&mut self, text: &mut TextSystem, theme: &Theme, state: &str, now: Instant) {
        match state {
            "wifi" | "password" => {
                self.open_sub(theme, QsPage::Wifi, now);
                if state == "password" {
                    let _ = self.qc.wifi[3].key(&super::press_key(ui_input::Key::Enter), now);
                }
            }
            "bluetooth" => self.open_sub(theme, QsPage::Bluetooth, now),
            "osd" | "osd-overflow" => {
                // Ekran görüntüsü için açık kalır (zamanlayıcı kurulmaz).
                if state == "osd-overflow" {
                    self.volume = 1.25;
                }
                self.show_osd(theme, 1, now);
                self.osd_timer = None;
            }
            "search" => {
                self.search.set_focused(true, now);
                for ch in "dehar".chars() {
                    let e = ui_input::KeyEvent { text: Some(ch.into()), ..super::press_key(ui_input::Key::Other(0)) };
                    let _ = self.search.key(&e, now);
                }
                self.filter(theme, now);
            }
            "fonts" | "fonts-mono" | "fonts-var" => {
                self.font_focus = true;
                if let Some(f) = &mut self.fonts {
                    let q = match state {
                        "fonts" => "sans",
                        "fonts-mono" => "mono",
                        _ => "google sans flex",
                    };
                    for ch in q.chars() {
                        let e = ui_input::KeyEvent { text: Some(ch.into()), ..super::press_key(ui_input::Key::Other(0)) };
                        let _ = f.key(text, theme, &e, now);
                    }
                    // Seçimi bir aşağı taşı ve seç (ağırlık çipleri yeni aileye göre kurulur).
                    if state != "fonts-var" {
                        let _ = f.key(text, theme, &super::press_key(ui_input::Key::Down), now);
                    }
                    let _ = f.key(text, theme, &super::press_key(ui_input::Key::Enter), now);
                }
            }
            _ => {}
        }
        let body = self.cards[0].body(theme);
        self.qc.place(text, theme, *self.qs.current(), body.origin(), body.width, now);
    }

    /// Ölçüm: başlatıcıyı bir sayfa aşağı/yukarı kaydırır (hızlı kaydırma senaryosu).
    /// `line`: satır satır (önbellek isabeti ağırlıklı), değilse sayfa sayfa (her karede yeni satırlar).
    pub fn scroll_step(&mut self, theme: &Theme, step: usize, line: bool, now: Instant) {
        let key = if line {
            if (step / 200).is_multiple_of(2) { ui_input::Key::Down } else { ui_input::Key::Up }
        } else if (step / 40).is_multiple_of(2) { ui_input::Key::PageDown } else { ui_input::Key::PageUp };
        let _ = self.results.key(theme, &super::press_key(key), now);
    }

    pub fn shaped_rows(&self) -> u64 {
        self.results.shaped_rows()
    }

    pub fn key(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::KeyEvent, now: Instant) -> bool {
        let _ = text;
        if let Some(r) = self.qc.wifi.iter_mut().chain(self.qc.bluetooth.iter_mut()).find(|r| r.is_expanded()) {
            if let Some(ev) = r.key(e, now).event {
                self.last_event.set_text(&format!("Son olay: {ev:?}"));
            }
            return true;
        }
        use ui_input::Key;
        if self.font_focus
            && e.key != Key::Escape
            && let Some(f) = &mut self.fonts
        {
            if let Some(FontEvent::Changed { family, weight }) = f.key(text, theme, e, now).event {
                self.last_event.set_text(&format!("Son olay: font {family} {weight}"));
            }
            return true;
        }
        // Arama alanı tuşları alır; Yukarı/Aşağı/Sayfa/Enter listeye gider.
        let list_key = matches!(e.key, Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Enter)
            || (matches!(e.key, Key::Home | Key::End) && e.modifiers.ctrl);
        if list_key {
            if let Some(ResultEvent::Activated(i)) = self.results.key(theme, e, now).event {
                self.event(format!("Son olay: başlat #{i}"));
            }
            return true;
        }
        if e.key == Key::Escape {
            return false;
        }
        if let Some(TextInputEvent::Changed) = self.search.key(e, now).event {
            self.filter(theme, now);
        }
        true
    }

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        match *self.qs.current() {
            QsPage::Root => {
                if let Some((i, ev)) = self.qc.tiles.pointer(theme, e, now).event {
                    match ev {
                        TileEvent::Expand if i < 2 => self.open_sub(theme, if i == 0 { QsPage::Wifi } else { QsPage::Bluetooth }, now),
                        _ => {}
                    }
                    self.event(format!("Son olay: kutucuk {i} {ev:?}"));
                }
            }
            page => {
                if self.qc.header.pointer(e, now).event.is_some() {
                    self.back(theme, now);
                } else {
                    let rows = self.qc.rows_mut(page);
                    let mut out = None;
                    for (i, r) in rows.iter_mut().enumerate() {
                        if let Some(ev) = r.pointer(text, theme, e, now).event {
                            out = Some((i, ev));
                        }
                    }
                    if let Some((i, ev)) = out {
                        let st = rows[i].state();
                        match ev {
                            DeviceEvent::Connect | DeviceEvent::Password(_) => rows[i].set_state(DeviceState::Connecting, now),
                            DeviceEvent::Disconnect => rows[i].set_state(DeviceState::Saved, now),
                            DeviceEvent::Forget if st == DeviceState::Saved => rows[i].set_state(DeviceState::Available, now),
                            DeviceEvent::Forget => {}
                        }
                        // Sır olay metnine yazılmaz.
                        let shown = match &ev {
                            DeviceEvent::Password(p) => format!("Password({} bayt)", p.len()),
                            other => format!("{other:?}"),
                        };
                        self.event(format!("Son olay: cihaz {i} {shown}"));
                    }
                }
            }
        }
        for (i, b) in self.osd_buttons.iter_mut().enumerate() {
            if b.pointer(e, now).event.is_some() {
                self.show_osd(theme, i, now);
                break;
            }
        }
        if let ui_input::PointerKind::Press(_) = e.kind {
            if self.search.rect().contains(e.position) {
                self.font_focus = false;
            } else if self.fonts.as_ref().is_some_and(|f| f.rect().contains(e.position)) {
                self.font_focus = true;
            }
        }
        if let Some(f) = &mut self.fonts
            && let Some(FontEvent::Changed { family, weight }) = f.pointer(text, theme, e, now).event
        {
            self.last_event.set_text(&format!("Son olay: font {family} {weight}"));
        }
        if let Some(TextInputEvent::Changed) = self.search.pointer(text, theme, e, now).event {
            self.filter(theme, now);
        }
        if let Some(ResultEvent::Activated(i)) = self.results.pointer(theme, e, now).event {
            self.event(format!("Son olay: başlat #{i}"));
        }
        true
    }

    pub fn cursor_at(&self, p: Point) -> CursorShape {
        let qs = match self.qs.current() {
            QsPage::Root => self.qc.tiles.cursor(p),
            &page => self.qc.header.cursor(p).or_else(|| self.qc.rows(page).iter().find_map(|r| r.cursor(p))),
        };
        CursorShape::pick(
            [qs, self.search.cursor(p), self.results.cursor(p), self.fonts.as_ref().and_then(|f| f.cursor(p))]
                .into_iter()
                .chain(self.osd_buttons.iter().map(|b| b.cursor(p))),
        )
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let now = ui.now;
        for c in &mut self.cards {
            c.draw(ui);
        }
        // Hızlı ayarlar: kap boyutu yayla değişir; satır yükseklikleri her karede yerleşir.
        // Satır açılıp kapanınca (klavye ya da fare) kap hedefi izler.
        let target = self.page_size(theme, *self.qs.current(), true);
        if target != self.qs.target_size() {
            self.qs.set_size(target, now);
        }
        let body = self.cards[0].body(theme);
        let size = self.qs.current_size(now);
        let clip = Rect::new(body.x, body.y, body.width, size.height.max(0.0) + theme.space(1));
        let qc = &mut self.qc;
        self.qs.draw(ui, clip, |ui, page, origin| {
            qc.place(ui.painter.text_system(), theme, *page, origin, body.width, now);
            qc.draw(ui, *page);
        });
        for b in &mut self.osd_buttons {
            b.draw(ui);
        }
        if let Some(f) = &mut self.fonts {
            f.draw(ui);
        }
        self.osd.draw(ui);
        self.search.draw(ui);
        self.results.draw(ui);
        let at = Point::new(self.cards[2].body(theme).x, self.cards[2].rect.bottom() - theme.space(10));
        ui.damage(Rect::new(at.x, at.y, theme.space(110), theme.space(6)));
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_matching() {
        assert_eq!(app_names().len(), LAUNCHER_ITEMS);
        assert_eq!(subsequence("NovaMüzik", "nomü"), Some(vec![0..2, 4..7]));
        assert_eq!(subsequence("Nova", "x"), None);
    }
}

/// 0..1 → yüzde.
fn pct(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 100.0).round() as u8
}

fn font_labels() -> FontPickerLabels {
    FontPickerLabels {
        search: "Font ara…".into(),
        all: "Tümü".into(),
        monospace: "Eş aralıklı".into(),
        sample: "Pijamalı hasta yağız şoföre çabucak güvendi 0123".into(),
        weight_names: ["İnce", "Çok hafif", "Hafif", "Normal", "Orta", "Yarı kalın", "Kalın", "Çok kalın", "Siyah"].map(String::from),
    }
}
