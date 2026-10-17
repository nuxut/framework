//! Bileşen galerisi: tüm `nuxut-ui` bileşenleri tüm durumlarıyla (normal, hover, basılı,
//! devre dışı, açık/kapalı) tek bir layer yüzeyinde.
//!
//! ```sh
//! cargo run -p nuxut-ui --example gallery
//! NUXUT_GALLERY_STATE=menu     …   # bar menüsü açık başlar
//! NUXUT_GALLERY_STATE=submenu  …   # bar menüsü alt sayfada başlar
//! NUXUT_GALLERY_STATE=context  …   # imleç menüsü açık başlar
//! NUXUT_GALLERY_STATE=tooltip  …   # ipucu sabit gösterilir
//! NUXUT_GALLERY_STATE=reveal   …   # parola alanı gösterilmiş, odak göster/gizle düğmesinde
//! NUXUT_GALLERY_PAGE=2 NUXUT_GALLERY_STATE=overlay …   # kırpılı karttan taşan popover + ipucu (kaplama katmanı)
//! NUXUT_GALLERY_PAGE=2 NUXUT_GALLERY_STATE=select|select-icons …   # açık seçim listesi; select-icons: bölüm başlıkları, ayraç, uygulama ikonları, yer tutucu
//! NUXUT_GALLERY_PAGE=3 NUXUT_GALLERY_STATE=dialog|date|time|time-12|snackbar|confirm …   # uygulama bileşenleri; time: 24 s, 1 dk adım (60 hücre); time-12: hour_cycle=h12 geçersiz kılması (ÖÖ/ÖS); confirm: yerinde onay düğmeleri kurulu başlar
//! NUXUT_GALLERY_PAGE=4 NUXUT_GALLERY_STATE=wifi|password|bluetooth|osd|osd-overflow|search|fonts|fonts-mono|fonts-var …   # shell bileşenleri, font seçici
//! NUXUT_GALLERY_PAGE=4 NUXUT_GALLERY_STATE=scroll|scroll-line …   # 5000 öğe hızlı kaydırma ölçümü (stdout: MEASURE)
//! NUXUT_GALLERY_PAGE=5 [NUXUT_GALLERY_STATE=week-3|cal-narrow] …   # takvim: etkinlik satırı/kartı, palet seçici, haftalık ızgara; cal-narrow: 220/240/280/320 genişlikte tr_TR/en_US ay ızgaraları + sağ metinli sonuçlar
//! NUXUT_GALLERY_PAGE=6 [NUXUT_GALLERY_STATE=flash|mid] …   # açılır satır: kapalı/rozetli/açık/odaklı; flash = içerik satırı parlar, mid = açılma ortasında donuk
//! NUXUT_GALLERY_PAGE=7 [NUXUT_GALLERY_STATE=rail-focus|focus|rail|rail-mid] …   # gezinme listesi: gruplar, yoğun satır, ikon şeridi (R: geniş ↔ dar)
//! NUXUT_GALLERY_PAGE=8 …   # imleç ve ikon teması önizlemeleri (kurulu temalar; tembel, önbellekli yükleme)
//! NUXUT_GALLERY_PAGE=9 …   # marka işareti (BrandMark): tek renk / vurgu / iki ton / karo, 16–128 px, koyu + açık palet
//! NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=mascot …   # maskot Fiyu (Mascot): 7 ruh hâli × 160/96/48 iki ton + 48 mono, koyu + açık
//! NUXUT_GALLERY_PAGE=10 [NUXUT_GALLERY_STATE=section] …   # glif ızgarası (GlyphGrid): bölümlü emoji (sanal, klavye odaklı) + sabit 6 sütun semboller; Tab odak değiştirir
//! NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=empty …   # boş durum (EmptyState: geniş/kısa/çok kısa/ikonlu) + metinli rozet (Pill: satır içi, köşe, liste ve sonuç sağ ucu)
//! NUXUT_GALLERY_PAGE=11 [NUXUT_GALLERY_STATE=edit] …   # dosya seçici: yol çubuğu (PathBar: geniş, dar/katlanmış, düzenleme kipi); Tab odak, Ctrl+L düzenle
//! NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=notif-mascot …   # bildirim kartında maskot (NotificationCard::with_mascot): kısa/uzun iç bildirim, %50 solma, dış uygulama (maskotsuz)
//! ```
//! Senaryo adımı çalışınca stdout'a `GALLERY_STEP <n>` yazar (ekran görüntüsü zamanlaması).
//! Sağ tık boş alanda imleç menüsü açar. Escape (hiçbir şey odakta değilken) çıkar.

use std::io::Write as _;
use std::time::{Duration, Instant};

use nuxut_core::{Corners, Insets, Point, Rect, Size};
use nuxut_render::{BlobField, BlobShape, ImageData, Painter, RectStyle, ShadowStyle, TextSystem, Texture, TextureOptions};
use nuxut_theme::{ColorRole, Theme};
use nuxut_ui::input as ui_input;
use nuxut_ui::{
    Avatar, AvatarSize, Badge, BodySpan, Button, ButtonVariant, CalendarEvent, CalendarGrid, ColorEvent, ColorPicker, Date, Icon,
    IconButton, IconButtonVariant, IconName, IconView, Label, List, ListEvent, ListItem, ListTrailing, Menu, MenuAnchor, MenuEvent,
    MenuItem, MenuModel, MenuSide, NotificationCard, NotificationContent, NotificationEvent, NotificationStack, Orientation,
    Separator, Slider, TextInput, TextInputEvent, TextRole, Toggle, Tooltip, Ui, text_style,
    CurveEditor, CurveEvent, CurveValue, MenuAlign, Popover, Progress, ReorderEvent, ReorderItem, ReorderList, Segment,
    SegmentedButton, Select, SelectEntry, SelectEvent, SelectOption, Spinner, Gauge, SparkScale, Sparkline, Threshold,
    Chip, ChipGroup, ChipKind, DateEvent, DatePicker, Dialog, DialogAction, DialogActionKind, FocusScope, NavItem,
    NavigationRail, ScreenEdge, SearchField, Snackbar, SnackbarEvent, Time, TimeEvent, TimePicker, format_time, ConfirmButton, Notice, NoticeKind, NoticeSize,
    ConfirmEvent,
};
use nuxut_motion::{CubicBezier, Spring};
use nuxut_wayland as wl;
use nuxut_core::input::CursorShape;
use nuxut_wayland::{App, Context, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, SurfaceId, TimerId};

#[path = "gallery/calendar_page.rs"]
mod calendar_page;
#[path = "gallery/disclosure_page.rs"]
mod disclosure_page;
#[path = "gallery/brand_page.rs"]
mod brand_page;
#[path = "gallery/mascot_page.rs"]
mod mascot_page;
#[path = "gallery/empty_page.rs"]
mod empty_page;
#[path = "gallery/notif_mascot_page.rs"]
mod notif_mascot_page;
#[path = "gallery/nav_page.rs"]
mod nav_page;
#[path = "gallery/theme_page.rs"]
mod theme_page;
#[path = "gallery/glyph_page.rs"]
mod glyph_page;
#[path = "gallery/files_page.rs"]
mod files_page;
#[path = "gallery/shell.rs"]
mod shell;

fn press_key(key: ui_input::Key) -> ui_input::KeyEvent {
    ui_input::KeyEvent { key, state: ui_input::KeyState::Pressed, text: None, modifiers: Default::default(), xkb: None }
}

// --- Yerleşim yardımcıları ----------------------------------------------------------------

const SURFACE_SIZE: (u32, u32) = (1880, 1040);

/// Bar başlığı ipucunun tam metni (bar'da kısaltılmış görünür; ipucunda sarılır, satır sınırında "…").
const LONG_TITLE: &str = "Belgeler — nuxut-framework/docs/ARCHITECTURE.md · Tooltip API ve uyanma sözleşmesi: fare öğenin üstüne gelince gecikmeli belirir, ısınmış pencerede beklemesiz yer değiştirir, bar kenarından ekranın içine açılır, uzun metin satırlara sarılır ve satır sınırını aşınca kesilir; bu cümle o sınırı aşmak için bilerek uzatıldı.";

struct Card {
    title: Label,
    rect: Rect,
}

impl Card {
    fn new(theme: &Theme, title: &str) -> Self {
        Self { title: Label::new(title, text_style(theme, TextRole::Title)), rect: Rect::ZERO }
    }

    fn body(&self, theme: &Theme) -> Rect {
        let pad = theme.space(5);
        let top = pad + theme.space(8);
        Rect::new(self.rect.x + pad, self.rect.y + top, self.rect.width - pad * 2.0, self.rect.height - top - pad)
    }

    fn draw(&mut self, ui: &mut Ui) {
        let t = ui.theme;
        ui.painter.rect(self.rect, &RectStyle::filled(t.colors.surface_container, t.shape.medium()));
        let pad = t.space(5);
        self.title.draw(ui.painter, Point::new(self.rect.x + pad, self.rect.y + pad), t.colors.on_surface);
    }
}

struct Caption {
    label: Label,
    at: Point,
}

/// Soldan sağa, sığmayınca alt satıra dizer; son satırın altını döner.
fn flow(area: Rect, sizes: &[Size], gap: f32, row_gap: f32) -> (Vec<Rect>, f32) {
    let (mut x, mut y, mut row_h) = (area.x, area.y, 0.0f32);
    let mut out = Vec::with_capacity(sizes.len());
    for s in sizes {
        if x > area.x && x + s.width > area.right() {
            x = area.x;
            y += row_h + row_gap;
            row_h = 0.0;
        }
        out.push(Rect::new(x, y, s.width, s.height));
        x += s.width + gap;
        row_h = row_h.max(s.height);
    }
    (out, y + row_h)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    None,
    Input(usize),
    Hex,
    Calendar,
    List,
}

// --- Galeri ------------------------------------------------------------------------------

struct Gallery {
    surface: Option<SurfaceId>,
    output: Option<OutputId>,
    size: Size,
    damage_all: bool,
    state: String,
    cards: Vec<Card>,
    captions: Vec<Caption>,
    // Düğmeler.
    buttons: Vec<Button>,
    /// Görünüşü sabit tutulan (hover/basılı) düğmelerin indisleri.
    pinned: Vec<usize>,
    // Kontroller.
    icon_buttons: Vec<IconButton>,
    badges: Vec<(IconView, Badge, Rect)>,
    badge_bump: Button,
    toggles: Vec<Toggle>,
    sliders: Vec<Slider>,
    avatars: Vec<Avatar>,
    separators: Vec<(Separator, Rect)>,
    // Menü.
    bar: Rect,
    bar_label: Label,
    tray: IconButton,
    menu: Menu,
    context: Menu,
    last_event: Label,
    last_event_at: Point,
    last_menu_damage: Option<Rect>,
    tooltip: Tooltip,
    /// Senaryo görüntüsü için ikinci ipucu (uzun, sarılmış).
    tooltip_long: Tooltip,
    /// `Ui::wake_deadline` zamanlayıcısı (tüm sayfalar, tek zamanlayıcı).
    wake_timer: Option<TimerId>,
    /// Bar başlığının son çizildiği yer (ipucu bağlantısı).
    bar_title: Rect,
    // Metin girişi.
    inputs: Vec<TextInput>,
    /// Süren yapıştırma: istek ve hedef giriş.
    paste: Option<(wl::PasteRequestId, usize)>,
    // Liste.
    list: List,
    // Bildirimler.
    notifications: Vec<(u32, NotificationCard)>,
    stack: NotificationStack<u32>,
    stack_area: Rect,
    add_notification: Button,
    next_notification: u32,
    // Renk ve takvim.
    picker: ColorPicker,
    calendar: CalendarGrid,
    focus: Focus,
    assets: Assets,
    /// 0 = temel bileşenler, 1 = Ayarlar bileşenleri (Tab ile geçilir).
    page: usize,
    settings: SettingsPage,
    apps: AppPage,
    shell: shell::ShellPage,
    cal: calendar_page::CalendarPage,
    disc: disclosure_page::DisclosurePage,
    nav: nav_page::NavPage,
    brand: brand_page::BrandPage,
    mascot: mascot_page::MascotPage,
    empty: empty_page::EmptyPage,
    notif_mascot: notif_mascot_page::NotifMascotPage,
    themes: theme_page::ThemePage,
    glyphs: glyph_page::GlyphPage,
    files: files_page::FilesPage,
    osd_timer: Option<TimerId>,
    /// Ölçüm senaryosu (`NUXUT_GALLERY_STATE=scroll`, sayfa 4): adım ve başlangıç CPU tıkı.
    measure: Option<(usize, u64, Instant)>,
    script: Option<TimerId>,
    script_step: u32,
}

fn menu_model(app: Option<&Texture>) -> MenuModel {
    // Tepsi menüsü gibi: ilk öğe uygulamanın kendi (renkli) ikonuyla.
    let app_icon = app.map_or_else(|| Icon::named(IconName::OpenInNew), |t| Icon::image(t.clone()));
    MenuModel::new(vec![
        MenuItem::action(1, "Pencereyi göster").with_icon(app_icon).with_shortcut("Ctrl+O"),
        MenuItem::check(2, "Bildirimleri sustur", true).with_shortcut("Ctrl+M"),
        MenuItem::check(3, "Başlangıçta çalıştır", false),
        MenuItem::separator(),
        MenuItem::submenu(
            10,
            "Ses çıkışı",
            vec![
                MenuItem::radio(11, "Hoparlörler", true),
                MenuItem::radio(12, "Kulaklık", false),
                MenuItem::radio(13, "HDMI — LG UltraFine 27\" ekran hoparlörü", false),
                MenuItem::separator(),
                MenuItem::submenu(
                    20,
                    "Gelişmiş",
                    vec![
                        MenuItem::action(21, "Örnekleme hızı").with_icon(Icon::named(IconName::Tune)),
                        MenuItem::action(22, "Gecikme ölçümü").with_icon(Icon::named(IconName::Timer)),
                        MenuItem::action(23, "Sıfırla").enabled(false),
                    ],
                ),
            ],
        )
        .with_icon(Icon::named(IconName::VolumeHigh)),
        MenuItem::action(4, "Güncellemeleri denetle").enabled(false).with_icon(Icon::named(IconName::Update)),
        MenuItem::separator(),
        MenuItem::action(5, "Çık").with_icon(Icon::named(IconName::Logout)).with_shortcut("Ctrl+Q"),
    ])
}

fn context_model() -> MenuModel {
    MenuModel::new(vec![
        MenuItem::action(101, "Kes").with_icon(Icon::named(IconName::Cut)).with_shortcut("Ctrl+X"),
        MenuItem::action(102, "Kopyala").with_icon(Icon::named(IconName::Copy)).with_shortcut("Ctrl+C"),
        MenuItem::action(103, "Yapıştır").with_icon(Icon::named(IconName::Paste)).with_shortcut("Ctrl+V"),
        MenuItem::separator(),
        MenuItem::submenu(110, "Sırala", vec![MenuItem::radio(111, "Ada göre", true), MenuItem::radio(112, "Tarihe göre", false)])
            .with_icon(Icon::named(IconName::Sort)),
    ])
}

fn list_items(theme: &Theme) -> Vec<ListItem> {
    let mut items = vec![
        ListItem::new(theme, "Wi-Fi")
            .with_subtitle(theme, "nuxut-5G · güçlü")
            .with_leading(theme, Icon::named(IconName::Wifi))
            .with_trailing(theme, ListTrailing::Toggle(true)),
        ListItem::new(theme, "Bluetooth").with_leading(theme, Icon::named(IconName::Bluetooth)).with_trailing(theme, ListTrailing::Toggle(false)),
        ListItem::new(theme, "Ses").with_leading(theme, Icon::named(IconName::VolumeHigh)).with_trailing(theme, ListTrailing::Text("%64".into())),
        ListItem::new(theme, "Ekran")
            .with_subtitle(theme, "1920×1080 · 60 Hz")
            .with_leading(theme, Icon::named(IconName::Desktop))
            .with_trailing(theme, ListTrailing::Chevron),
        ListItem::new(theme, "Uçak modu (devre dışı)").with_leading(theme, Icon::named(IconName::Flight)).with_trailing(theme, ListTrailing::Toggle(false)),
        // Glif yuvası (emoji seçici): renkli emoji ve düz harf ikon boyunda, yuva ortasında.
        ListItem::new(theme, "Gülen yüz")
            .with_subtitle(theme, "Emoji · glif yuvası")
            .with_leading(theme, Icon::glyph("😀"))
            .with_trailing(theme, ListTrailing::Icon(Icon::named(IconName::Emoji))),
        ListItem::new(theme, "Omega").with_leading(theme, Icon::glyph("Ω")).with_trailing(theme, ListTrailing::Text("U+03A9".into())),
    ];
    items[3].set_selected(true);
    items[4].set_enabled(false, Instant::now());
    for i in 0..60 {
        items.push(
            ListItem::new(theme, format!("Uygulama {i:02} — uzun bir başlık kısalarak sığmalı"))
                .with_subtitle(theme, "Kaydırma ve sanallaştırma için doldurma öğesi")
                .with_leading(theme, Icon::named(IconName::Apps))
                .with_trailing(theme, ListTrailing::Text(format!("{} MB", 12 + i * 7))),
        );
    }
    items
}

fn notification(theme: &Theme, n: u32) -> NotificationCard {
    let samples = [
        NotificationContent {
            app_name: "Mesajlar".into(),
            app_icon: Some(Icon::named(IconName::Chat)),
            time: "şimdi".into(),
            title: "Ayşe Yılmaz".into(),
            body: vec![
                BodySpan::plain("Akşam yemeğine geliyor musun? Saat "),
                BodySpan { text: "sekizde".into(), bold: true, ..BodySpan::default() },
                BodySpan::plain(" Kadıköy'de buluşuyoruz; "),
                BodySpan { text: "konum".into(), link: Some("https://nuxut.dev/harita".into()), ..BodySpan::default() },
                BodySpan::plain(". Geç kalırsan haber ver, "),
                BodySpan { text: "kitabı da getir".into(), italic: true, ..BodySpan::default() },
                BodySpan::plain(" lütfen."),
            ],
            image: None,
            actions: vec!["Yanıtla".into(), "Okundu say".into()],
        },
        NotificationContent {
            app_name: "Ekran görüntüsü".into(),
            app_icon: Some(Icon::named(IconName::Screenshot)),
            time: "2 dk".into(),
            title: "Görüntü kaydedildi".into(),
            body: vec![BodySpan::plain("~/Resimler/ekran-2026-10-04.png")],
            image: None,
            actions: vec!["Aç".into(), "Düzenle".into(), "Sil".into()],
        },
        NotificationContent {
            app_name: "Pil".into(),
            app_icon: Some(Icon::named(IconName::BatteryAlert)),
            time: "14 dk".into(),
            title: "Pil %15".into(),
            body: vec![BodySpan::plain("Güç tasarrufu "), BodySpan { text: "açıldı".into(), underline: true, ..BodySpan::default() }, BodySpan::plain(".")],
            image: None,
            actions: vec![],
        },
    ];
    NotificationCard::new(theme, samples[(n as usize) % samples.len()].clone())
}

impl Gallery {
    fn new(theme: &Theme) -> Self {
        let now = Instant::now();
        let mut buttons = Vec::new();
        let mut pinned = Vec::new();
        for variant in [ButtonVariant::Filled, ButtonVariant::Tonal, ButtonVariant::Outlined, ButtonVariant::Text] {
            for (i, text) in ["Normal", "Hover", "Basılı", "Kapalı"].into_iter().enumerate() {
                let mut b = Button::new(theme, variant).with_label(theme, text);
                match i {
                    1 => b.set_visual_state(true, false, now),
                    2 => b.set_visual_state(true, true, now),
                    3 => {
                        b.set_enabled(false, now);
                    }
                    _ => {}
                }
                if matches!(i, 1 | 2) {
                    pinned.push(buttons.len());
                }
                buttons.push(b);
            }
        }
        for variant in [ButtonVariant::Filled, ButtonVariant::Tonal, ButtonVariant::Outlined, ButtonVariant::Text] {
            buttons.push(Button::new(theme, variant).with_icon(theme, Icon::named(IconName::Add)).with_label(theme, "İkonlu"));
        }
        let mut icon_buttons = Vec::new();
        for variant in [IconButtonVariant::Standard, IconButtonVariant::Filled, IconButtonVariant::Tonal] {
            icon_buttons.push(IconButton::new(theme, Icon::named(IconName::Settings), variant));
            icon_buttons.push(IconButton::new(theme, Icon::named(IconName::Bluetooth), variant).toggleable(true));
            icon_buttons.push(IconButton::new(theme, Icon::named(IconName::Bluetooth), variant).toggleable(false));
            let mut d = IconButton::new(theme, Icon::named(IconName::Wifi), variant);
            d.set_enabled(false, now);
            icon_buttons.push(d);
        }
        let badges = [None, Some(3), Some(128)]
            .into_iter()
            .map(|c| (IconView::new(theme, Icon::named(IconName::Notifications)), Badge::new(theme, c), Rect::ZERO))
            .collect();
        // İlk ikisi etkileşimli; diğerleri sabit durum (kapalı/açık çiftleri): hover, basılı,
        // klavye odağı, devre dışı.
        let mut toggles: Vec<Toggle> = (0..10).map(|i| Toggle::new(theme, i % 2 == 1)).collect();
        for t in &mut toggles[2..4] {
            t.set_visual_state(true, false, now);
        }
        for t in &mut toggles[4..6] {
            t.set_visual_state(true, true, now);
        }
        for t in &mut toggles[6..8] {
            t.set_focus_visible(true);
        }
        for t in &mut toggles[8..10] {
            t.set_enabled(false, now);
        }
        // Yatay: sol sütun etkileşimli (normal · adımlı · devre dışı), sağ sütun sabit durum
        // (hover · sürükleme + değer balonu · klavye odağı); dikey: normal + sürükleme.
        let mut sliders = vec![
            Slider::new(theme, Orientation::Horizontal, 0.35),
            Slider::new(theme, Orientation::Horizontal, 0.6).with_steps(10),
            Slider::new(theme, Orientation::Horizontal, 0.5),
            Slider::new(theme, Orientation::Horizontal, 0.5),
            Slider::new(theme, Orientation::Horizontal, 0.7),
            Slider::new(theme, Orientation::Horizontal, 0.42),
            Slider::new(theme, Orientation::Vertical, 0.7),
            Slider::new(theme, Orientation::Vertical, 0.25).with_steps(4),
        ];
        sliders[2].set_enabled(false, now);
        sliders[3].set_visual_state(true, false, now);
        sliders[4].set_visual_state(true, true, now);
        sliders[5].set_focus_visible(true);
        sliders[7].set_visual_state(true, true, now);
        let mut avatars = vec![
            Avatar::new(theme, "Ayşe Yılmaz", AvatarSize::Small),
            Avatar::new(theme, "Mehmet Öz", AvatarSize::Medium),
            Avatar::new(theme, "Deniz Kaya", AvatarSize::Large),
            Avatar::new(theme, "Zeynep", AvatarSize::Medium),
            Avatar::new(theme, "Ada Lovelace", AvatarSize::XLarge),
        ];
        avatars[2].set_presence(true, now);
        let mut inputs = vec![
            TextInput::new(theme).with_placeholder("Ara…"),
            TextInput::new(theme).with_text("Merhaba dünya"),
            TextInput::new(theme).with_text("Odakta, seçili metin"),
            // Göster/gizle düğmeli parola (Tab: metin → düğme; Space/Enter çevirir).
            TextInput::new(theme).password(true).with_reveal_toggle(theme, true).with_text("gizli-parola"),
            TextInput::new(theme).with_text("Devre dışı alan"),
        ];
        inputs[2].set_focused(true, now);
        let ctrl_a = ui_input::KeyEvent {
            key: ui_input::Key::Char('a'),
            state: ui_input::KeyState::Pressed,
            text: None,
            modifiers: ui_input::Modifiers { ctrl: true, ..Default::default() },
            xkb: None,
        };
        let _ = inputs[2].key(&ctrl_a, now);
        inputs[4].set_enabled(false);
        inputs[3].set_reveal_labels("Parolayı göster", "Parolayı gizle");
        let now_secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let today = Date::from_unix_seconds(now_secs, 3 * 3600);
        let mut calendar = CalendarGrid::new(theme, today, &nuxut_ui::locale_from_env());
        calendar.set_events(vec![
            (today.add_days(2), vec![None]),
            (today.add_days(5), vec![None, None]),
            (today.add_days(-3), vec![None, None, None]),
            (today, vec![None]),
        ]);
        calendar.set_selected(Some(today.add_days(5)));
        let mut picker = ColorPicker::new(theme, theme.colors.primary);
        picker.set_recent(theme, &[theme.colors.on_surface_variant, theme.colors.surface_bright]);
        let stack = NotificationStack::new(theme);
        let caption = text_style(theme, TextRole::Caption);
        Self {
            surface: None,
            output: None,
            size: Size::ZERO,
            damage_all: true,
            state: std::env::var("NUXUT_GALLERY_STATE").unwrap_or_default(),
            cards: ["Düğmeler", "Kontroller", "Menü · ipucu", "Metin girişi", "Liste", "Bildirimler", "Renk seçici", "Takvim"]
                .into_iter()
                .map(|t| Card::new(theme, t))
                .collect(),
            captions: Vec::new(),
            buttons,
            pinned,
            icon_buttons,
            badges,
            badge_bump: Button::new(theme, ButtonVariant::Tonal).with_icon(theme, Icon::named(IconName::Add)).with_label(theme, "Sayaç"),
            toggles,
            sliders,
            avatars,
            separators: Vec::new(),
            bar: Rect::ZERO,
            bar_label: Label::new("Belgeler — nuxut-framework/docs/ARCHITE…", text_style(theme, TextRole::Label)),
            tray: IconButton::new(theme, Icon::named(IconName::MoreHoriz), IconButtonVariant::Standard),
            menu: Menu::new(theme),
            context: Menu::new(theme),
            last_event: Label::new("Son olay: —", caption),
            last_event_at: Point::ZERO,
            last_menu_damage: None,
            tooltip: Tooltip::new(theme),
            tooltip_long: Tooltip::new(theme),
            wake_timer: None,
            bar_title: Rect::ZERO,
            inputs,
            paste: None,
            list: List::new(theme).with_items(list_items(theme)),
            notifications: (0..3).map(|n| (n, notification(theme, n))).collect(),
            stack,
            stack_area: Rect::ZERO,
            add_notification: Button::new(theme, ButtonVariant::Tonal)
                .with_icon(theme, Icon::named(IconName::NotificationsAdd))
                .with_label(theme, "Bildirim ekle"),
            next_notification: 3,
            picker,
            calendar,
            focus: Focus::Input(2),
            assets: Assets::default(),
            page: match std::env::var("NUXUT_GALLERY_PAGE").as_deref() {
                Ok("2") => 1,
                Ok("3") => 2,
                Ok("4") => 3,
                Ok("5") => 4,
                Ok("6") => 5,
                Ok("7") => 6,
                Ok("8") => 7,
                Ok("9") => 8,
                Ok("10") => 9,
                Ok("11") => 10,
                _ => 0,
            },
            apps: AppPage::new(theme),
            shell: shell::ShellPage::new(theme),
            cal: calendar_page::CalendarPage::new(theme, today, now_secs + 3 * 3600),
            disc: disclosure_page::DisclosurePage::new(theme),
            nav: nav_page::NavPage::new(theme),
            brand: brand_page::BrandPage::new(theme),
            mascot: mascot_page::MascotPage::new(theme),
            empty: empty_page::EmptyPage::new(theme),
            notif_mascot: notif_mascot_page::NotifMascotPage::new(theme),
            themes: theme_page::ThemePage::new(theme),
            glyphs: glyph_page::GlyphPage::new(theme),
            files: files_page::FilesPage::new(theme),
            osd_timer: None,
            measure: None,
            settings: SettingsPage::new(theme),
            script: None,
            script_step: 0,
        }
    }

    fn caption(&mut self, theme: &Theme, text: &str, at: Point) {
        self.captions.push(Caption { label: Label::new(text, text_style(theme, TextRole::Caption)), at });
    }

    fn layout(&mut self, text: &mut TextSystem, theme: &Theme) {
        let s = |n| theme.space(n);
        let (w, h) = (self.size.width, self.size.height);
        let m = s(5);
        let col_w = ((w - m * 5.0) / 4.0).floor();
        let col_x = |i: f32| m + (col_w + m) * i;
        let now = Instant::now();
        self.captions.clear();
        self.separators.clear();

        // Sütun 1: düğmeler + kontroller.
        let sizes: Vec<Size> = self.buttons.iter_mut().map(|b| b.preferred_size(text, theme)).collect();
        self.cards[0].rect = Rect::new(col_x(0.0), m, col_w, 0.0);
        let body = self.cards[0].body(theme);
        let (rects, bottom) = flow(body, &sizes, s(2), s(3));
        for (b, r) in self.buttons.iter_mut().zip(rects) {
            b.set_rect(r);
        }
        self.caption(theme, "filled · tonal · outlined · text — normal, hover, basılı, kapalı", Point::new(body.x, bottom + s(3)));
        self.cards[0].rect.height = bottom + s(9) - m;

        let top = self.cards[0].rect.bottom() + m;
        self.cards[1].rect = Rect::new(col_x(0.0), top, col_w, h - top - m);
        let body = self.cards[1].body(theme);
        let mut y = body.y;
        for row in self.icon_buttons.chunks_mut(4) {
            let mut x = body.x;
            for b in row.iter_mut() {
                b.set_rect(Rect::new(x, y, s(10), s(10)));
                x += s(12);
            }
            y += s(11);
        }
        // Rozetler, sayaç ve avatarlar ikon düğmelerinin sağında.
        let right_x = body.x + s(52);
        let mut bx = right_x;
        for (icon, badge, rect) in &mut self.badges {
            let sz = icon.measure(text);
            *rect = Rect::new(bx, body.y + s(2), sz.width, sz.height);
            badge.set_anchor(Point::new(rect.right(), rect.y));
            bx += s(10);
        }
        let bs = self.badge_bump.preferred_size(text, theme);

        let mut ax = right_x;
        for a in &mut self.avatars {
            let side = a.size().side(theme);
            // XLarge (profil kartı) satıra sığmaz: Sayaç düğmesinin sağına yerleşir (aşağıda).
            if a.size() == AvatarSize::XLarge {
                continue;
            }
            a.set_center(Point::new(ax + side * 0.5, body.y + s(19)));
            ax += side + s(2);
        }
        self.caption(theme, "ikon düğmeleri: standart · dolu · tonal — rozetler, avatarlar", Point::new(body.x, y));
        y += s(6);
        self.separators.push((Separator::horizontal(0.0), Rect::new(body.x, y, body.width, s(2))));
        y += s(4);
        // İki satır: normal · hover · basılı / odak · devre dışı (her biri kapalı + açık).
        let mut x = body.x;
        for (i, t) in self.toggles.iter_mut().enumerate() {
            if i == 6 {
                x = body.x;
                y += s(11);
            }
            let sz = t.preferred_size(theme);
            t.set_rect(Rect::new(x, y, sz.width, sz.height));
            x += sz.width + s(if i % 2 == 1 { 6 } else { 3 });
        }
        self.caption(theme, "anahtar (kapalı/açık): normal · hover · basılı / odak · devre dışı", Point::new(body.x, y + s(9)));
        y += s(16);
        let hw = body.width - s(26);
        let cw = (hw - s(4)) * 0.5;
        let top = y;
        for (i, sl) in self.sliders.iter_mut().take(6).enumerate() {
            let sz = sl.preferred_size(theme, cw);
            let x = if i < 3 { body.x } else { body.x + cw + s(4) };
            if i == 3 {
                y = top;
            }
            sl.set_rect(Rect::new(x, y, sz.width, sz.height));
            y += sz.height + s(1);
        }
        // Sayaç düğmesi kaydırıcıların altında (rozetlere tıklanınca sayılar artar).
        self.badge_bump.set_rect(Rect::new(body.x, y + s(2), bs.width, bs.height));
        // XLarge avatar: Sayaç'ın sağında, üst kenarı düğmeyle hizalı.
        let bump = Rect::new(body.x, y + s(2), bs.width, bs.height);
        for a in self.avatars.iter_mut().filter(|a| a.size() == AvatarSize::XLarge) {
            let side = a.size().side(theme);
            a.set_center(Point::new(bump.right() + s(10) + side * 0.5, bump.y + side * 0.5));
        }
        let mut vx = body.x + hw + s(6);
        for sl in self.sliders.iter_mut().skip(6) {
            let sz = sl.preferred_size(theme, s(34));
            sl.set_rect(Rect::new(vx, y - s(36), sz.width, sz.height));
            vx += sz.width + s(1);
        }

        // Sütun 2: menü + metin girişi.
        self.cards[2].rect = Rect::new(col_x(1.0), m, col_w, s(132));
        let body = self.cards[2].body(theme);
        self.bar = Rect::new(body.x, body.y, body.width, s(10));
        self.tray.set_rect(Rect::new(self.bar.right() - s(10), self.bar.y + s(1), s(8), s(8)));
        let title_w = self.bar_label.measure(text).width;
        self.bar_title = Rect::new(self.bar.x + s(4), self.bar.y, title_w, self.bar.height);
        self.last_event_at = Point::new(body.x, body.bottom() - s(5));
        let top = self.cards[2].rect.bottom() + m;
        self.cards[3].rect = Rect::new(col_x(1.0), top, col_w, h - top - m);
        let body = self.cards[3].body(theme);
        let mut y = body.y;
        for i in &mut self.inputs {
            let sz = i.preferred_size(theme, body.width);
            i.set_rect(Rect::new(body.x, y, sz.width, sz.height));
            y += sz.height + s(3);
        }

        // Sütun 3: liste + bildirimler.
        self.cards[4].rect = Rect::new(col_x(2.0), m, col_w, s(100));
        let body = self.cards[4].body(theme);
        self.list.set_rect(theme, body, now);
        let top = self.cards[4].rect.bottom() + m;
        self.cards[5].rect = Rect::new(col_x(2.0), top, col_w, h - top - m);
        let body = self.cards[5].body(theme);
        let bs = self.add_notification.preferred_size(text, theme);
        self.add_notification.set_rect(Rect::new(body.x, body.y - s(1), bs.width, bs.height));
        self.stack_area = Rect::new(body.x, body.y + bs.height + s(2), body.width, body.bottom() - body.y - bs.height - s(2));

        // Sütun 4: renk seçici + takvim.
        let pw = col_w - s(10);
        let ps = self.picker.preferred_size(theme, pw);
        self.cards[6].rect = Rect::new(col_x(3.0), m, col_w, ps.height + s(23));
        let body = self.cards[6].body(theme);
        self.picker.set_rect(theme, Rect::new(body.x, body.y, ps.width, ps.height));
        let top = self.cards[6].rect.bottom() + m;
        self.cards[7].rect = Rect::new(col_x(3.0), top, col_w, h - top - m);
        let body = self.cards[7].body(theme);
        let cs = self.calendar.preferred_size(theme, body.width);
        self.calendar.set_rect(theme, Rect::new(body.x, body.y - s(4), cs.width, cs.height));
    }

    fn open_bar_menu(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        let anchor = MenuAnchor::below(Rect::new(self.tray.rect().x, self.bar.y, self.tray.rect().width, self.bar.height));
        // Menü kartların içinde kalır: bar'ın sağ ucuna dayanınca kenarı bar'la hizalanır.
        let bounds = self.cards[2].body(theme).union(&self.cards[3].body(theme));
        self.menu.open(text, theme, menu_model(self.assets.app.as_ref()), anchor, bounds, now);
    }

    /// Bileşen uyanma zamanlayıcısını (`Ui::wake_deadline`: ipucu gecikmesi, kaydırıcı balonu,
    /// eğri önizleme beklemesi) `at`e kurar; öncekini iptal eder (tek zamanlayıcı).
    fn arm_wake(&mut self, cx: &mut Context<Self>, at: Instant, now: Instant) {
        if let Some(old) = self.wake_timer.take() {
            cx.cancel_timer(old);
        }
        self.wake_timer = Some(cx.set_timer(at.saturating_duration_since(now)));
    }

    /// Kare sonu sözleşmesi (her sayfa): canlanan varsa sonraki kare, durağan bekleyen varsa
    /// uyanma zamanlayıcısı. İkincisi atlanırsa zamanlı bileşenler (balon, ipucu) donar.
    fn finish_frame(&mut self, cx: &mut Context<Self>, surface: SurfaceId, wants_frame: bool, wake: Option<Instant>, now: Instant) {
        if wants_frame {
            cx.request_frame(surface);
        }
        if let Some(at) = wake {
            self.arm_wake(cx, at, now);
        }
    }

    fn surface_bounds(&self, theme: &Theme) -> Rect {
        Rect::new(0.0, 0.0, self.size.width, self.size.height).inset(Insets::uniform(theme.space(2)))
    }

    fn set_event(&mut self, text: String) {
        self.last_event.set_text(&text);
    }

    fn report_menu(&mut self, e: &MenuEvent) {
        let text = match e {
            MenuEvent::Activated(id) => format!("Son olay: etkinleştirildi #{id}"),
            MenuEvent::SubmenuOpened(id) => format!("Son olay: alt sayfa #{id}"),
            MenuEvent::Dismissed => "Son olay: kapatıldı".to_owned(),
        };
        self.set_event(text);
    }

    fn set_focus(&mut self, focus: Focus, now: Instant) {
        if self.focus == focus {
            return;
        }
        if let Focus::Input(i) = self.focus {
            self.inputs[i].set_focused(false, now);
        }
        if self.focus == Focus::Hex {
            self.picker.set_hex_focused(false, now);
        }
        self.focus = focus;
        match focus {
            Focus::Input(i) => self.inputs[i].set_focused(true, now),
            Focus::Hex => self.picker.set_hex_focused(true, now),
            _ => {}
        }
    }

    fn step(&mut self) {
        self.script_step += 1;
        println!("GALLERY_STEP {}", self.script_step);
        let _ = std::io::stdout().flush();
    }
}

impl App for Gallery {
    type Message = theme_page::Msg;

    fn message(&mut self, cx: &mut Context<Self>, message: theme_page::Msg) {
        if let theme_page::Msg::Loaded(loaded) = message {
            self.themes.loaded(loaded);
        }
        if self.page == 7
            && let Some(s) = self.surface
        {
            cx.request_frame(s);
        }
    }

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        if self.surface.is_some() {
            return;
        }
        // Canlı testler görünmez NUXUT-TEST çıkışında: NUXUT_GALLERY_OUTPUT=NUXUT-TEST.
        if let Ok(want) = std::env::var("NUXUT_GALLERY_OUTPUT")
            && output.name != want
        {
            return;
        }
        let spec = LayerSurfaceSpec {
            layer: Layer::Overlay,
            size: SURFACE_SIZE,
            keyboard: KeyboardMode::OnDemand,
            ..LayerSurfaceSpec::new("nuxut-gallery", output.id)
        };
        self.surface = Some(cx.create_layer_surface(spec));
        self.output = Some(output.id);
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        if self.output == Some(output) {
            cx.quit();
        }
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        let theme = cx.theme().clone();
        self.themes.set_scale(scale);
        self.themes.layout(&theme, size);
        self.size = size;
        self.damage_all = true;
        self.layout(cx.text(), &theme);
        self.settings.layout(cx.text(), &theme, size);
        self.apps.layout(cx.text(), &theme, size);
        self.shell.layout(cx.text(), &theme, size);
        self.cal.layout(cx.text(), &theme, size);
        self.disc.layout(cx.text(), &theme, size, Instant::now());
        if self.page == 5 && self.script_step == 0 {
            let now = Instant::now();
            self.disc.apply_state(&self.state, now);
            self.disc.layout(cx.text(), &theme, size, now);
        }
        self.nav.layout(cx.text(), &theme, size);
        self.brand.layout(&theme, size);
        self.mascot.layout(&theme, size);
        self.glyphs.layout(&theme, size, Instant::now());
        if self.page == 9 && self.script_step == 0 {
            self.glyphs.apply_state(&self.state, Instant::now());
        }
        self.files.layout(cx.text(), &theme, size);
        if self.page == 10 && self.script_step == 0 {
            self.files.apply_state(&theme, &self.state, Instant::now());
        }
        self.empty.layout(cx.text(), &theme, size);
        self.notif_mascot.layout(cx.text(), &theme);
        if self.page == 6 && self.script_step == 0 {
            self.nav.apply_state(&theme, &self.state, Instant::now());
        }
        if self.page == 4 && self.script_step == 0 {
            self.cal.apply_state(&theme, &self.state, Instant::now());
        }
        if self.page == 3 && self.script_step == 0 {
            let now = Instant::now();
            self.shell.apply_state(cx.text(), &theme, &self.state, now);
            if self.state.starts_with("scroll") {
                self.measure = Some((0, cpu_ticks(), Instant::now()));
            }
            self.arm_osd_timer(cx);
        }
        if self.page == 2 && self.script_step == 0 {
            match self.state.as_str() {
                "dialog" => self.apps.open_dialog(cx.text(), &theme, Instant::now()),
                "date" => self.apps.open_date(&theme, Instant::now()),
                "time" => self.apps.open_time(&theme, false, Instant::now()),
                "time-12" => self.apps.open_time(&theme, true, Instant::now()),
                "snackbar" => self.apps.show_snackbar(cx.text(), &theme, Instant::now()),
                "confirm" => self.apps.arm_confirms(cx.text(), &theme, Instant::now()),
                _ => {}
            }
        }
        if self.assets.loaded.is_none() {
            self.assets.load(&theme);
        }
        if self.page == 1 && self.script_step == 0 {
            match self.state.as_str() {
                "popover" => self.settings.open_popover(&theme, Instant::now()),
                // Kaplama: kırpılı karttaki tetikleyicilerden açılan popover ve ipucu kart dışına taşar.
                "overlay" => {
                    let now = Instant::now();
                    self.settings.open_popover(&theme, now);
                    let anchor = self.settings.color_button.rect();
                    self.settings.tip.show("Kırpılı kartın dışına taşan ipucu", anchor, MenuSide::Left, Rect::ZERO, now);
                }
                "select" => {
                    let down = press_key(ui_input::Key::Down);
                    let _ = self.settings.selects[1].key(cx.text(), &theme, &down, Instant::now());
                }
                // Başlıklı + ikonlu seçim açık başlar.
                "select-icons" => {
                    let down = press_key(ui_input::Key::Down);
                    let _ = self.settings.selects[2].key(cx.text(), &theme, &down, Instant::now());
                }
                _ => {}
            }
        }
        if self.script_step == 0 {
            let now = Instant::now();
            match self.state.as_str() {
                "menu" | "submenu" => {
                    self.open_bar_menu(cx.text(), &theme, now);
                    self.script = Some(cx.set_timer(Duration::from_millis(1200)));
                }
                "context" => {
                    let at = Point::new(self.cards[4].rect.x + theme.space(30), self.cards[4].rect.y + theme.space(40));
                    let bounds = self.surface_bounds(&theme);
                    self.context.open(cx.text(), &theme, context_model(), MenuAnchor::at_point(at, theme.space(1)), bounds, now);
                }
                "tooltip" => {
                    let bounds = self.surface_bounds(&theme);
                    self.tooltip.show("Bluetooth (açık)", self.icon_buttons[5].rect(), MenuSide::Below, bounds, now);
                    self.tooltip_long.show(LONG_TITLE, self.bar_title, MenuSide::away_from(ScreenEdge::Top), bounds, now);
                }
                // Parola gösterilmiş, klavye odağı göster/gizle düğmesinde (odak halkası).
                "reveal" => {
                    self.set_focus(Focus::Input(3), now);
                    self.inputs[3].set_revealed(true);
                    let _ = self.inputs[3].focus_step(&press_key(ui_input::Key::Tab), now);
                }
                _ => {}
            }
            self.step();
        }
        cx.request_frame(surface);
    }

    fn paste(
        &mut self,
        cx: &mut Context<Self>,
        id: wl::PasteRequestId,
        result: Result<wl::ClipboardText, wl::PasteError>,
    ) {
        let Some((_, i)) = self.paste.take_if(|(pending, _)| *pending == id) else {
            return;
        };
        match result {
            Ok(text) => {
                let _ = self.inputs[i].paste(text.as_str());
                if let Some(s) = self.surface {
                    cx.request_frame(s);
                }
            }
            Err(e) => eprintln!("galeri: yapıştırılamadı: {e}"),
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.wake_timer == Some(id) {
            // Uyanma anı geldi (ipucu gecikmesi, balon sönmesi…): kare çizilir, bileşenler ilerler.
            self.wake_timer = None;
            if let Some(s) = self.surface {
                cx.request_frame(s);
            }
            return;
        }
        if self.osd_timer == Some(id) {
            self.osd_timer = None;
            self.shell.osd_timeout(Instant::now());
            if let Some(s) = self.surface {
                cx.request_frame(s);
            }
            return;
        }
        if self.script != Some(id) {
            return;
        }
        self.script = None;
        let theme = cx.theme().clone();
        let now = Instant::now();
        for _ in 0..2 {
            let _ = self.menu.key(cx.text(), &theme, &press_key(ui_input::Key::Down), now);
        }
        if self.state == "submenu" {
            for _ in 0..2 {
                let _ = self.menu.key(cx.text(), &theme, &press_key(ui_input::Key::Down), now);
            }
            let r = self.menu.key(cx.text(), &theme, &press_key(ui_input::Key::Right), now);
            if let Some(e) = &r.event {
                self.report_menu(e);
            }
            let _ = self.menu.key(cx.text(), &theme, &press_key(ui_input::Key::Down), now);
        }
        self.step();
        if let Some(s) = self.surface {
            cx.request_frame(s);
        }
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: wl::PointerEvent) {
        let pos = event.position;
        self.on_pointer(cx, surface, event);
        // Bağlama göre imleç: en üstteki isabet önce (runtime aynı şekli tekrar göndermez).
        let shape = self.cursor_at(pos);
        cx.set_cursor(surface, shape);
    }

    fn modifiers_changed(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, modifiers: wl::Modifiers) {
        self.files.modifiers_changed(modifiers);
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: wl::KeyEvent) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        let e = event;
        // Ctrl+Tab sayfa değiştirir (Tab sayfa 3'te odak sırasıdır).
        if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Tab && e.modifiers.ctrl && !self.menu.is_open() && !self.settings.captures_keys() {
            self.page = (self.page + 1) % 11;
            self.damage_all = true;
            cx.request_frame(surface);
            return;
        }
        if self.page == 10 {
            if self.files.key(&theme, &e, now) {
                cx.request_frame(surface);
            } else if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
            }
            return;
        }
        if self.page == 9 {
            if self.glyphs.key(&e, now) {
                cx.request_frame(surface);
            } else if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
            }
            return;
        }
        if self.page == 8 {
            if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
            }
            return;
        }
        if self.page == 5 {
            if !self.disc.key(&e, now) && e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
                return;
            }
            cx.request_frame(surface);
            return;
        }
        if self.page == 6 {
            if self.nav.key(&theme, &e, now) {
                cx.request_frame(surface);
            } else if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
            }
            return;
        }
        if self.page == 4 {
            if !self.cal.key(&theme, &e, now) && e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
                return;
            }
            cx.request_frame(surface);
            return;
        }
        if self.page == 3 {
            if !self.shell.key(cx.text(), &theme, &e, now) && e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
                return;
            }
            self.arm_osd_timer(cx);
            cx.request_frame(surface);
            return;
        }
        if self.page == 2 {
            if !self.apps.key(cx.text(), &theme, &e, now) && e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                cx.quit();
                return;
            }
            cx.request_frame(surface);
            return;
        }
        if self.page == 1 {
            if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape && !self.settings.captures_keys() {
                cx.quit();
                return;
            }
            self.settings.key(cx.text(), &theme, &e, now);
            cx.request_frame(surface);
            return;
        }
        for which in 0..2 {
            let menu = if which == 0 { &mut self.menu } else { &mut self.context };
            if menu.is_open() {
                let r = menu.key(cx.text(), &theme, &e, now);
                if let Some(ev) = r.event {
                    self.report_menu(&ev);
                }
                cx.request_frame(surface);
                return;
            }
        }
        match self.focus {
            Focus::Input(i) => {
                // Alan içi Tab durağı (parola göster/gizle düğmesi).
                if self.inputs[i].focus_step(&e, now) {
                    cx.request_frame(surface);
                    return;
                }
                let r = self.inputs[i].key(&e, now);
                match r.event {
                    Some(TextInputEvent::Copy(s) | TextInputEvent::Cut(s)) => {
                        if let Err(e) = cx.set_clipboard(wl::Selection::Clipboard, &s) {
                            eprintln!("galeri: panoya yazılamadı: {e}");
                        }
                    }
                    Some(TextInputEvent::PasteRequested) => {
                        self.paste = Some((cx.request_paste(wl::Selection::Clipboard), i));
                    }
                    Some(TextInputEvent::Cancel) => self.set_focus(Focus::None, now),
                    _ => {}
                }
            }
            Focus::Hex => {
                let _ = self.picker.key(&theme, &e, now);
            }
            Focus::Calendar => {
                let _ = self.calendar.key(&theme, &e, now);
            }
            Focus::List => {
                let _ = self.list.key(&theme, &e, now);
            }
            Focus::None => {
                if e.state == ui_input::KeyState::Pressed && e.key == ui_input::Key::Escape {
                    cx.quit();
                }
            }
        }
        cx.request_frame(surface);
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        for b in &mut self.buttons {
            b.set_theme(&theme, now);
        }
        for b in &mut self.icon_buttons {
            b.set_theme(&theme, now);
        }
        for t in &mut self.toggles {
            t.set_theme(&theme, now);
        }
        for s in &mut self.sliders {
            s.set_theme(&theme, now);
        }
        for a in &mut self.avatars {
            a.set_theme(&theme, now);
        }
        for i in &mut self.inputs {
            i.set_theme(&theme, now);
        }
        for (_, n) in &mut self.notifications {
            n.set_theme(&theme, now);
        }
        self.stack.set_theme(&theme, now);
        self.list.set_theme(&theme, now);
        self.tooltip.set_theme(&theme, now);
        self.tooltip_long.set_theme(&theme, now);
        self.picker.set_theme(&theme, now);
        self.calendar.set_theme(&theme, now);
        self.menu.set_theme(cx.text(), &theme, now);
        self.context.set_theme(cx.text(), &theme, now);
        self.settings.set_theme(cx.text(), &theme, now);
        self.disc.set_theme(&theme, now);
        self.nav.set_theme(&theme, now);
        self.brand.set_theme(&theme, now);
        self.mascot.set_theme(&theme, now);
        self.empty.set_theme(&theme, now);
        self.empty.layout(cx.text(), &theme, self.size);
        self.notif_mascot.set_theme(&theme, now);
        self.notif_mascot.layout(cx.text(), &theme);
        self.themes.set_theme(&theme, now);
        self.glyphs.set_theme(&theme, now);
        self.files.set_theme(&theme, now);
        self.damage_all = true;
        self.layout(cx.text(), &theme);
        if let Some(s) = self.surface {
            cx.request_frame(s);
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let theme = cx.theme().clone();
        if std::mem::take(&mut self.damage_all) {
            painter.damage_all();
        }
        if !self.assets.pending.is_empty() {
            self.assets.upload(painter);
            self.apply_assets(painter.text_system(), &theme, now);
        }
        if self.page == 5 {
            // Açılır satır yüksekliği canlanır: her karede yeniden yerleşim (etiketler önbellekli).
            let size = self.size;
            self.disc.layout(painter.text_system(), &theme, size, now);
        }
        let sender = cx.sender();
        let mut ui = Ui::new(painter, &theme, now).with_damage_tracking(true);
        let full = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        ui.painter.rect(full, &RectStyle::filled(theme.colors.surface, theme.shape.large()));
        if self.page == 10 {
            self.files.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 9 {
            self.glyphs.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 8 {
            if self.state == "empty" {
                self.empty.draw(&mut ui);
            } else if self.state == "notif-mascot" {
                self.notif_mascot.draw(&mut ui);
            } else if self.state == "mascot" {
                self.mascot.draw(&mut ui);
            } else {
                self.brand.draw(&mut ui);
            }
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 7 {
            self.themes.draw(&mut ui, &sender);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 5 {
            self.disc.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 6 {
            self.nav.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 4 {
            self.cal.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 3 {
            if let Some((step, start, t0)) = self.measure {
                if step < MEASURE_STEPS {
                    self.shell.scroll_step(&theme, step, self.state == "scroll-line", now);
                    self.measure = Some((step + 1, start, t0));
                    ui.request_frame();
                } else {
                    self.measure = None;
                    println!(
                        "MEASURE frames={MEASURE_STEPS} wall_ms={} cpu_ticks={} shaped_rows={} rss_kb={}",
                        t0.elapsed().as_millis(),
                        cpu_ticks() - start,
                        self.shell.shaped_rows(),
                        rss_kb()
                    );
                    let _ = std::io::stdout().flush();
                }
            }
            self.shell.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 2 {
            self.apps.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        if self.page == 1 {
            self.settings.draw(&mut ui);
            self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
            return;
        }
        for card in &mut self.cards {
            card.draw(&mut ui);
        }
        for c in &mut self.captions {
            c.label.draw(ui.painter, c.at, theme.colors.on_surface_variant);
        }
        for (sep, r) in &self.separators {
            sep.draw(&mut ui, *r);
        }
        for b in &mut self.buttons {
            b.draw(&mut ui);
        }
        for b in &mut self.icon_buttons {
            b.draw(&mut ui);
        }
        for (icon, badge, rect) in &mut self.badges {
            icon.draw_centered(ui.painter, rect.center(), theme.colors.on_surface);
            badge.draw(&mut ui);
        }
        self.badge_bump.draw(&mut ui);
        for a in &mut self.avatars {
            a.draw(&mut ui);
        }
        for t in &mut self.toggles {
            t.draw(&mut ui);
        }
        for s in &mut self.sliders {
            s.draw(&mut ui);
        }
        for i in &mut self.inputs {
            i.draw(&mut ui);
        }
        self.list.draw(&mut ui);

        // Bildirim yığını: kartlar ölçülür, yığın konumları yayla canlandırır.
        self.add_notification.draw(&mut ui);
        let area = self.stack_area;
        // Bildirimler normalde yüzey zemininde durur; galeride kartın içinde zemin çiz.
        ui.painter.rect(area.expand(theme.space(2)), &RectStyle::filled(theme.colors.surface, theme.shape.medium()));
        let mut heights = Vec::with_capacity(self.notifications.len());
        for (n, card) in &mut self.notifications {
            let h = card.measure(ui.painter.text_system(), &theme, area.width);
            heights.push((*n, h));
        }
        self.stack.layout(&heights, theme.space(3), now);
        // Yığın bölgesi her karede hasarlı: kartlar kayarken eski yerler silinsin.
        ui.damage(area.expand(theme.space(2)));
        ui.painter.push_clip(area.expand(theme.space(1)));
        for (n, card) in &mut self.notifications {
            let Some(slot) = self.stack.slot(n, now) else { continue };
            let h = heights.iter().find(|(k, _)| k == n).map_or(0.0, |(_, h)| *h);
            card.set_rect(ui.painter.text_system(), &theme, Rect::new(area.x, area.y + slot.y, area.width, h));
            ui.with_alpha(slot.alpha, |ui| card.draw(ui));
        }
        ui.painter.pop_clip();
        let before = self.notifications.len();
        self.notifications.retain_mut(|(_, c)| !c.is_gone(now));
        if self.stack.is_animating(now) || before != self.notifications.len() {
            ui.request_frame();
        }

        self.picker.draw(&mut ui);
        self.calendar.draw(&mut ui);

        let at = self.last_event_at;
        ui.damage(Rect::new(at.x, at.y, theme.space(80), theme.space(6)));
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);

        // Bar + menü tek blob alanı: menü bar'dan büyür, birleşim filetoyla yumuşar.
        let bar_shape = BlobShape::new(self.bar, Corners::uniform(theme.shape.radius_medium));
        let mut shapes = vec![bar_shape];
        if let Some(sh) = self.menu.shape(now, &theme) {
            shapes.push(sh);
        }
        let field = BlobField {
            frame: None,
            shapes,
            smoothing: theme.blob.smoothing,
            corner_radius: theme.shape.radius_large,
            fill: theme.colors.surface,
            shadow: theme.shadow.enabled.then(|| ShadowStyle {
                color: theme.shadow.color,
                blur: theme.shadow.blur,
                offset: Point::new(0.0, theme.shadow.offset_y),
                spread: 0.0,
            }),
        };
        let menu_damage = field.shapes.get(1).map(|sh| field.damage_rect(sh));
        for r in [self.last_menu_damage, menu_damage].into_iter().flatten() {
            ui.damage(r);
        }
        self.last_menu_damage = menu_damage;
        ui.painter.blob(&field);
        let bl = self.bar_label.measure_in(ui.painter);
        self.bar_label.draw(
            ui.painter,
            Point::new(self.bar.x + theme.space(4), self.bar.center().y - bl.height * 0.5),
            theme.colors.on_surface_variant,
        );
        self.tray.draw(&mut ui);
        self.menu.draw_content(&mut ui);

        self.tooltip.draw(&mut ui);
        self.tooltip_long.draw(&mut ui);
        self.context.draw(&mut ui);

        self.finish_frame(cx, surface, ui.wants_frame(), ui.wake_deadline(), now);
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    // Bileşenler runtime'ın yükleyeceği temayla kurulsun (framework.toml).
    let theme = nuxut_theme::ThemeLoader::new().load().map_or_else(|_| nuxut_theme::Theme::broken_config(), |l| l.value);
    if let Err(e) = wl::run(Gallery::new(&theme)) {
        eprintln!("galeri: {e}");
        std::process::exit(1);
    }
}

// --- Sayfa 2: Ayarlar uygulamasının bileşenleri --------------------------------------------

struct SettingsPage {
    /// Son olay etiketi / renk örneği değişti (yalnızca o zaman hasar).
    label_dirty: bool,
    last_swatch: Option<nuxut_core::Color>,
    cards: Vec<Card>,
    captions: Vec<Caption>,
    segments: Vec<SegmentedButton>,
    selects: Vec<Select>,
    color_button: Button,
    swatch: Rect,
    popover: Popover,
    picker: ColorPicker,
    /// Kırpılı kartın içindeki seçimin ipucu (kaplama örneği).
    tip: Tooltip,
    spinners: Vec<Spinner>,
    progress: Vec<Progress>,
    /// Dairesel göstergeler: düz ve eşikli (açık gösterge).
    gauges: Vec<Gauge>,
    /// Geçmiş grafikleri: CPU (primary, 0..100) ve eşiği aşmış (error).
    sparks: Vec<Sparkline>,
    bump: Button,
    bezier: CurveEditor,
    spring: CurveEditor,
    reorder: ReorderList,
    hidden: ReorderList,
    last_event: Label,
    last_event_at: Point,
    size: Size,
}

impl SettingsPage {
    fn new(theme: &Theme) -> Self {
        let now = Instant::now();
        let mut disabled_seg = SegmentedButton::new(theme, vec![Segment::new("Açık"), Segment::new("Kapalı")], 0);
        disabled_seg.set_enabled(false, now);
        let mut spinners = vec![Spinner::new(theme), Spinner::new(theme)];
        spinners[1].set_progress(theme, Some(0.65), now);
        let mut bezier = CurveEditor::new(CurveValue::Bezier(CubicBezier::STANDARD));
        bezier.set_preview(Some(Duration::from_millis(600)));
        let mut spring = CurveEditor::new(CurveValue::Spring(Spring::new(0.42, 0.7)));
        spring.set_preview(Some(Duration::ZERO));
        // Ölçüm: NUXUT_GALLERY_ANIM=curve|spinner yalnızca o animasyonu oynatır (CPU karşılaştırması).
        match std::env::var("NUXUT_GALLERY_ANIM").as_deref() {
            Ok("curve") => {
                spinners[0].set_progress(theme, Some(0.3), now);
                spring.set_preview(None);
            }
            Ok("none") => {
                spinners[0].set_progress(theme, Some(0.3), now);
                bezier.set_preview(None);
                spring.set_preview(None);
            }
            Ok("spinner") => {
                spinners[1] = Spinner::new(theme);
                spinners[1].set_progress(theme, Some(0.65), now);
                bezier.set_preview(None);
                spring.set_preview(None);
            }
            _ => {}
        }
        let modules = ["Saat", "Çalışma alanları", "Etkin pencere", "Tepsi", "Ses", "Pil"];
        Self {
            cards: ["Bölümlü düğme · açılır seçim", "Popover · ilerleme", "Hareket eğrisi", "Sıralama"]
                .into_iter()
                .map(|t| Card::new(theme, t))
                .collect(),
            captions: Vec::new(),
            segments: vec![
                SegmentedButton::new(
                    theme,
                    vec![
                        Segment::new("Gün").with_icon(Icon::named(IconName::ViewDay)),
                        Segment::new("Hafta").with_icon(Icon::named(IconName::ViewWeek)),
                        Segment::new("Ay").with_icon(Icon::named(IconName::CalendarMonth)),
                    ],
                    1,
                ),
                SegmentedButton::new(theme, (1..=5).map(|i| Segment::new(format!("{i}x"))).collect(), 2),
                SegmentedButton::new(
                    theme,
                    vec![Segment::new("Çok uzun bir seçenek etiketi"), Segment::new("Kısa")],
                    0,
                ),
                disabled_seg,
            ],
            selects: vec![
                Select::new(theme, vec!["Açık".into(), "Koyu".into(), "Sistem".into()], 1),
                Select::new(
                    theme,
                    // font-istisnası: seçim listesi için örnek veri (font seçici değil)
                    ["Noto Sans", "Inter", "Cantarell", "DejaVu Sans", "Ubuntu", "Roboto", "Source Sans 3", "IBM Plex Sans"]
                        .map(String::from)
                        .to_vec(),
                    0,
                ),
                // Bölüm başlıkları + ayraç + seçenek ikonları (varsayılan uygulama seçimi örneği).
                // Uygulama ikonu dokusu yüklenince `apply_assets` verir; o zamana dek yer tutucu.
                Select::with_entries(
                    theme,
                    vec![
                        SelectEntry::header("Önerilen"),
                        SelectEntry::option("Terminal"),
                        SelectOption::new("Web tarayıcısı").with_icon(IconName::Language).into(),
                        SelectEntry::Separator,
                        SelectEntry::header("Diğer"),
                        SelectOption::new("Dosyalar").with_icon(IconName::Folder).into(),
                        SelectEntry::option("İkonu yüklenemeyen uygulama"),
                    ],
                    0,
                )
                .with_icon_slot(true),
            ],
            color_button: Button::new(theme, ButtonVariant::Tonal).with_icon(theme, Icon::named(IconName::Palette)).with_label(theme, "Vurgu rengi"),
            swatch: Rect::ZERO,
            popover: Popover::new(theme),
            picker: ColorPicker::new(theme, theme.colors.primary),
            tip: Tooltip::new(theme),
            spinners,
            progress: vec![
                Progress::new(theme, 0.3),
                Progress::new(theme, 0.8),
                // Renk rolü ve eşik: %85'i aşan değer error'a geçer.
                Progress::new(theme, 0.55).with_color(ColorRole::OnSurfaceVariant),
                Progress::new(theme, 0.92).with_threshold(Threshold::new(0.85)),
            ],
            gauges: vec![
                Gauge::new(theme, 0.42).with_label(theme, "42%"),
                Gauge::new(theme, 0.93).with_threshold(Threshold::new(0.85)).with_sweep(0.75).with_label(theme, "93%"),
            ],
            sparks: {
                // Belirlenimci örnek geçmiş (ekran görüntüsü tekrarlanabilir).
                let cpu = |i: usize| 35.0 + 20.0 * (i as f32 * 0.31).sin() + 9.0 * (i as f32 * 0.83).sin();
                let mut a = Sparkline::new(theme, 60).with_scale(SparkScale::Fixed { min: 0.0, max: 100.0 });
                a.set_samples((0..60).map(cpu));
                let mut b = Sparkline::new(theme, 60)
                    .with_scale(SparkScale::AutoMax { min: 0.0, min_max: 1.0 })
                    .with_threshold(Threshold::new(60.0).with_hysteresis(5.0));
                b.set_samples((0..60).map(|i| cpu(i) + i as f32 * 0.9));
                vec![a, b]
            },
            bump: Button::new(theme, ButtonVariant::Tonal).with_icon(theme, Icon::named(IconName::Refresh)).with_label(theme, "İlerlet"),
            bezier,
            spring,
            reorder: ReorderList::new(theme, modules.iter().map(|m| ReorderItem::new(*m)).collect()),
            hidden: ReorderList::new(theme, vec![ReorderItem::new("Bluetooth").with_icon(Icon::named(IconName::Bluetooth))]),
            last_event: Label::new("Son olay: —", text_style(theme, TextRole::Caption)),
            last_event_at: Point::ZERO,
            size: Size::ZERO,
            label_dirty: true,
            last_swatch: None,
        }
    }

    fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let s = |n| theme.space(n);
        self.size = size;
        let now = Instant::now();
        let m = s(5);
        let col_w = ((size.width - m * 5.0) / 4.0).floor();
        let col_x = |i: f32| m + (col_w + m) * i;
        let h = size.height - m * 2.0;
        self.captions.clear();
        for (i, c) in self.cards.iter_mut().enumerate() {
            c.rect = Rect::new(col_x(i as f32), m, col_w, h);
        }
        // Sütun 1.
        let body = self.cards[0].body(theme);
        let mut y = body.y;
        for (i, seg) in self.segments.iter_mut().enumerate() {
            let sz = seg.preferred_size(text, theme);
            let w = if i == 2 { body.width * 0.6 } else { sz.width.min(body.width) };
            seg.set_rect(theme, Rect::new(body.x, y, w, sz.height));
            y += sz.height + s(4);
        }
        self.captions.push(Caption {
            label: Label::new("gün/hafta/ay · 5 seçenek · sığmayan etiket kırpılır · devre dışı", text_style(theme, TextRole::Caption)),
            at: Point::new(body.x, y - s(2)),
        });
        y += s(6);
        let bounds = Rect::new(0.0, 0.0, size.width, size.height).inset(Insets::uniform(s(2)));
        for (i, sel) in self.selects.iter_mut().enumerate() {
            let sz = sel.preferred_size(theme, body.width);
            // İkinci seçim kartın dibinde: menü sığmayınca yukarı açılır.
            let sy = if i == 1 { body.bottom() - sz.height - s(10) } else { y };
            sel.set_rect(theme, Rect::new(body.x, sy, sz.width, sz.height));
            sel.set_bounds(bounds);
            y += sz.height + s(3);
        }
        self.last_event_at = Point::new(body.x, body.bottom() - s(4));

        // Sütun 2.
        let body = self.cards[1].body(theme);
        let bs = self.color_button.preferred_size(text, theme);
        self.color_button.set_rect(Rect::new(body.x, body.y, bs.width, bs.height));
        self.swatch = Rect::new(body.x + bs.width + s(3), body.y + s(1), s(8), s(8));
        let mut y = body.y + s(16);
        let sp = self.spinners[0].preferred_size(theme);
        for (i, sn) in self.spinners.iter_mut().enumerate() {
            sn.set_rect(Rect::new(body.x + i as f32 * (sp.width + s(6)), y, sp.width, sp.height));
        }
        y += sp.height + s(5);
        for p in &mut self.progress {
            let ps = p.preferred_size(theme, body.width);
            p.set_rect(Rect::new(body.x, y, ps.width, ps.height));
            y += s(5);
        }
        let bs = self.bump.preferred_size(text, theme);
        self.bump.set_rect(Rect::new(body.x, y, bs.width, bs.height));
        y += bs.height + s(5);
        let gs = self.gauges[0].preferred_size(theme);
        for (i, g) in self.gauges.iter_mut().enumerate() {
            g.set_rect(Rect::new(body.x + i as f32 * (gs.width + s(6)), y, gs.width, gs.height));
        }
        y += gs.height + s(5);
        for sp in &mut self.sparks {
            let ss = sp.preferred_size(theme, body.width);
            sp.set_rect(Rect::new(body.x, y, ss.width, ss.height));
            y += ss.height + s(4);
        }

        // Sütun 3.
        let body = self.cards[2].body(theme);
        let es = self.bezier.preferred_size(theme, body.width);
        self.bezier.set_rect(Rect::new(body.x, body.y, es.width, es.height));
        let es2 = self.spring.preferred_size(theme, body.width);
        self.spring.set_rect(Rect::new(body.x, body.y + es.height + s(8), es2.width, es2.height));

        // Sütun 4.
        let body = self.cards[3].body(theme);
        let rh = self.reorder.preferred_height(theme);
        self.reorder.set_rect(theme, Rect::new(body.x, body.y, body.width, rh), now);
        let hy = body.y + rh + s(14);
        self.captions.push(Caption {
            label: Label::new("Gizli modüller (sürükleyip bırakın)", text_style(theme, TextRole::Caption)),
            at: Point::new(body.x, hy - s(6)),
        });
        let hh = self.hidden.preferred_height(theme).max(s(12));
        self.hidden.set_rect(theme, Rect::new(body.x, hy, body.width, hh.max(s(40))), now);
    }

    fn open_popover(&mut self, theme: &Theme, now: Instant) {
        let pw = theme.space(70);
        let ps = self.picker.preferred_size(theme, pw);
        let anchor = MenuAnchor { rect: self.color_button.rect(), side: MenuSide::Below, align: MenuAlign::Start, gap: theme.space(2) };
        let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height).inset(Insets::uniform(theme.space(2)));
        self.popover.open(theme, anchor, ps, bounds, now);
        let r = self.popover.content_target(theme);
        self.picker.set_rect(theme, r);
    }

    /// Klavye bir açılır öğede mi (Tab/Esc galeriye gitmesin).
    fn captures_keys(&self) -> bool {
        self.popover.is_open() || self.selects.iter().any(Select::is_open)
    }

    fn event(&mut self, text: String) {
        self.last_event.set_text(&text);
        self.label_dirty = true;
    }

    fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        if self.popover.is_open() {
            if self.popover.pointer(e, now).event.is_some() {
                return true;
            }
            let r = self.picker.pointer(text, theme, e, now);
            if let Some(ColorEvent::Changed(c) | ColorEvent::Committed(c)) = r.event {
                self.event(format!("Son olay: renk {}", c.to_hex()));
            }
            return true;
        }
        for i in 0..self.selects.len() {
            if self.selects[i].is_open() {
                let r = self.selects[i].pointer(text, theme, e, now);
                if let Some(SelectEvent::Changed(v)) = r.event {
                    self.event(format!("Son olay: seçim {i} → {v}"));
                }
                return true;
            }
        }
        for (i, seg) in self.segments.iter_mut().enumerate() {
            let r = seg.pointer(e, now);
            repaint |= r.repaint;
            if let Some(v) = r.event {
                self.last_event.set_text(&format!("Son olay: bölüm {i} → {v}"));
                self.label_dirty = true;
            }
        }
        for sel in &mut self.selects {
            repaint |= sel.pointer(text, theme, e, now).repaint;
        }
        let r = self.color_button.pointer(e, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            self.open_popover(theme, now);
            repaint = true;
        }
        let r = self.bump.pointer(e, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            for p in &mut self.progress {
                let v = p.value() + 0.15;
                p.set_value(if v > 1.0 { 0.05 } else { v }, now);
            }
            let v = (self.progress[0].value() + 0.2) % 1.0;
            self.spinners[1].set_progress(theme, Some(v), now);
            for g in &mut self.gauges {
                let v = (g.value() + 0.15) % 1.0;
                g.set_value(v, now);
                g.set_label(&format!("{:.0}%", v * 100.0));
            }
            for (i, sp) in self.sparks.iter_mut().enumerate() {
                let last = sp.history().latest().unwrap_or(0.0);
                let next = if i == 0 { (last + 17.0) % 100.0 } else { (last + 23.0) % 110.0 };
                sp.push(next, now);
            }
        }
        for ed in [&mut self.bezier, &mut self.spring] {
            let r = ed.pointer(theme, e);
            repaint |= r.repaint;
            if let Some(ev) = r.event {
                let text = match ev {
                    CurveEvent::Bezier(b) => format!("Son olay: cubic-bezier({:.2}, {:.2}, {:.2}, {:.2})", b.x1, b.y1, b.x2, b.y2),
                    CurveEvent::Spring(s) => format!("Son olay: yay tepki {:.2} sönüm {:.2}", s.response, s.damping),
                };
                self.last_event.set_text(&text);
                self.label_dirty = true;
            }
        }
        for li in [0usize, 1] {
            let r = if li == 0 { self.reorder.pointer(theme, e, now) } else { self.hidden.pointer(theme, e, now) };
            repaint |= r.repaint;
            match r.event {
                Some(ReorderEvent::DraggedOut { index, position }) => {
                    // Diğer listenin üstüne bırakıldıysa taşı.
                    let (src, dst) = if li == 0 { (&mut self.reorder, &mut self.hidden) } else { (&mut self.hidden, &mut self.reorder) };
                    if dst.rect().expand(theme.space(6)).contains(position)
                        && let Some(item) = src.remove(theme, index, now)
                    {
                        let at = dst.len();
                        dst.insert(theme, at, item, now);
                    }
                    self.event(format!("Son olay: liste {li} öğe {index} dışarı"));
                    repaint = true;
                }
                Some(ev) => self.event(format!("Son olay: liste {li} {ev:?}")),
                None => {}
            }
        }
        repaint
    }

    fn key(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::KeyEvent, now: Instant) {
        if self.popover.is_open() {
            if self.popover.key(e, now).event.is_none() {
                let _ = self.picker.key(theme, e, now);
            }
            return;
        }
        for sel in &mut self.selects {
            if sel.is_open() {
                let _ = sel.key(text, theme, e, now);
                return;
            }
        }
        let _ = self.reorder.key(theme, e, now);
    }

    fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        for c in &mut self.cards {
            c.draw(ui);
        }
        for c in &mut self.captions {
            c.label.draw(ui.painter, c.at, theme.colors.on_surface_variant);
        }
        // İlk iki kartın gövdesi kırpılı (Ayarlar'daki blok/kaydırma alanı gibi). Açılır menü,
        // ipucu ve popover tetikleyicinin hemen ardından, kırpmanın içinden çizilir; kaplama
        // katmanında oldukları için kırpılmaz, kare sonunda en üstte çizilirler.
        ui.painter.push_clip(self.cards[0].body(theme));
        for s in &mut self.segments {
            s.draw(ui);
        }
        for s in &mut self.selects {
            s.draw(ui);
            s.draw_overlay(ui);
        }
        ui.painter.pop_clip();
        ui.painter.push_clip(self.cards[1].body(theme));
        self.color_button.draw(ui);
        let picker = &mut self.picker;
        self.popover.draw_overlay(ui, |ui, _rect| picker.draw(ui));
        // Kaplamalar kendi aralarında çizim sırasıyla üst üste: ipucu en son.
        self.tip.draw(ui);
        let sw = self.swatch;
        ui.painter.rect(sw, &RectStyle::filled(self.picker.color(), Corners::uniform(sw.height * 0.5)));
        if self.last_swatch != Some(self.picker.color()) {
            self.last_swatch = Some(self.picker.color());
            ui.damage(sw);
        }
        for s in &mut self.spinners {
            s.draw(ui);
        }
        for p in &mut self.progress {
            p.draw(ui);
        }
        for g in &mut self.gauges {
            g.draw(ui);
        }
        for sp in &mut self.sparks {
            sp.draw(ui);
        }
        self.bump.draw(ui);
        ui.painter.pop_clip();
        self.bezier.draw(ui);
        self.spring.draw(ui);
        self.reorder.draw(ui);
        self.hidden.draw(ui);
        let at = self.last_event_at;
        if std::mem::take(&mut self.label_dirty) {
            ui.damage(Rect::new(at.x, at.y, theme.space(90), theme.space(6)));
        }
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}

impl SettingsPage {
    fn set_theme(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        for s in &mut self.segments {
            s.set_theme(theme, now);
        }
        for s in &mut self.selects {
            s.set_theme(theme, now);
        }
        self.color_button.set_theme(theme, now);
        self.bump.set_theme(theme, now);
        self.popover.set_theme(theme, now);
        self.picker.set_theme(theme, now);
        self.tip.set_theme(theme, now);
        for s in &mut self.spinners {
            s.set_theme(theme, now);
        }
        for p in &mut self.progress {
            p.set_theme(theme, now);
        }
        for g in &mut self.gauges {
            g.set_theme(theme, now);
        }
        for sp in &mut self.sparks {
            sp.set_theme(theme, now);
        }
        self.reorder.set_theme(theme, now);
        self.hidden.set_theme(theme, now);
        let size = self.size;
        self.layout(text, theme, size);
    }
}

// --- Örnek görseller (ikon teması + üretilmiş görseller) ------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    App,
    Symbolic,
    Photo,
    Thumb,
}

#[derive(Default)]
struct Assets {
    loaded: Option<()>,
    pending: Vec<(Slot, ImageData)>,
    app: Option<Texture>,
    symbolic: Option<Texture>,
    photo: Option<Texture>,
    thumb: Option<Texture>,
}

impl Assets {
    /// Olay işleyicisinde (çizim dışı) çözer; yükleme ilk karede `Painter` ile yapılır.
    fn load(&mut self, theme: &Theme) {
        self.loaded = Some(());
        let icons = nuxut_image::IconTheme::system();
        let first = |names: &[&str]| names.iter().find_map(|n| icons.load_as(n, 48, 1, nuxut_image::Trust::System).ok());
        if let Some(img) = first(&["utilities-terminal", "org.gnome.Terminal", "kitty", "system-file-manager", "applications-system"]) {
            self.pending.push((Slot::App, to_data(img)));
        }
        if let Some(img) = first(&["audio-volume-high-symbolic", "audio-speakers-symbolic", "emblem-system-symbolic"]) {
            self.pending.push((Slot::Symbolic, to_data(img)));
        }
        // Kurgusal "fotoğraf" ve küçük resim: tema renklerinden yumuşak gradyan.
        let c = &theme.colors;
        self.pending.push((Slot::Photo, gradient(96, 96, c.primary, c.surface_bright)));
        self.pending.push((Slot::Thumb, gradient(112, 112, c.surface_bright, c.primary)));
    }

    fn upload(&mut self, painter: &mut Painter) {
        for (slot, data) in self.pending.drain(..) {
            let t = Some(painter.upload_texture(data, TextureOptions::default()));
            match slot {
                Slot::App => self.app = t,
                Slot::Symbolic => self.symbolic = t,
                Slot::Photo => self.photo = t,
                Slot::Thumb => self.thumb = t,
            }
        }
    }
}

fn to_data(img: nuxut_image::Image) -> ImageData {
    let (width, height) = img.size();
    ImageData { width, height, pixels: std::sync::Arc::from(img.into_pixels()) }
}

/// Köşegen gradyan (premultiplied, opak).
fn gradient(w: u32, h: u32, a: nuxut_core::Color, b: nuxut_core::Color) -> ImageData {
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let t = (x + y) as f32 / (w + h) as f32;
            let c = a.lerp(&b, t);
            px.extend([c.r, c.g, c.b].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            px.push(255);
        }
    }
    ImageData { width: w, height: h, pixels: std::sync::Arc::from(px) }
}

impl Gallery {
    /// Dokular hazır olunca bileşenlere verilir.
    fn apply_assets(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        // Açık menü varsa uygulama ikonlu modelle yenilenir.
        if self.menu.is_open() {
            self.menu.set_model(text, theme, menu_model(self.assets.app.as_ref()), now);
        }
        if let Some(app) = &self.assets.app {
            self.settings.selects[2].set_option_icon(0, Some(Icon::image(app.clone())));
        }
        self.files.set_assets(self.assets.photo.as_ref(), self.assets.thumb.as_ref());
        if let Some(t) = &self.assets.photo {
            self.avatars[3].set_image(Some(t.clone()));
        }
        if let Some(t) = &self.assets.symbolic {
            self.list.items_mut()[2].set_leading(theme, Some(Icon::symbolic(t.clone())));
        }
        for (n, card) in &mut self.notifications {
            if *n % 3 == 1 {
                let mut content = card.content().clone();
                content.image = self.assets.thumb.clone();
                if let Some(app) = &self.assets.app {
                    content.app_icon = Some(Icon::image(app.clone()));
                }
                *card = NotificationCard::new(theme, content);
            }
        }
    }
}

impl Gallery {
    fn on_pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: wl::PointerEvent) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        let e = event;
        if self.page == 10 {
            if self.files.pointer(cx.text(), &theme, &e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 9 {
            if self.glyphs.pointer(&theme, &e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 8 {
            return;
        }
        if self.page == 7 {
            if self.themes.pointer(&e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 5 {
            if self.disc.pointer(&e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 6 {
            if self.nav.pointer(&theme, &e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 4 {
            if self.cal.pointer(&theme, &e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 3 {
            if self.shell.pointer(cx.text(), &theme, &e, now) {
                cx.request_frame(surface);
            }
            self.arm_osd_timer(cx);
            return;
        }
        if self.page == 2 {
            if self.apps.pointer(cx.text(), &theme, &e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        if self.page == 1 {
            if self.settings.pointer(cx.text(), &theme, &e, now) {
                cx.request_frame(surface);
            }
            return;
        }
        let mut repaint = false;

        for which in 0..2 {
            let menu = if which == 0 { &mut self.menu } else { &mut self.context };
            if menu.is_open() {
                let r = menu.pointer(cx.text(), &theme, &e, now);
                if let Some(ev) = r.event {
                    self.report_menu(&ev);
                }
                cx.request_frame(surface);
                return;
            }
        }

        for (i, b) in self.buttons.iter_mut().enumerate() {
            if !self.pinned.contains(&i) {
                repaint |= b.pointer(&e, now).repaint;
            }
        }
        for b in &mut self.icon_buttons {
            repaint |= b.pointer(&e, now).repaint;
        }
        let r = self.badge_bump.pointer(&e, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            for (_, badge, _) in &mut self.badges {
                let next = badge.count().map(|c| c + 1);
                badge.set_count(next, now);
            }
        }
        // Sabit görünüm durumundakiler (hover/basılı/odak) olay almaz.
        for (i, t) in self.toggles.iter_mut().enumerate() {
            if !(2..8).contains(&i) {
                repaint |= t.pointer(&e, now).repaint;
            }
        }
        // Sabit görünüm durumundakiler (hover/sürükleme/odak) olay almaz.
        for (i, s) in self.sliders.iter_mut().enumerate() {
            if !matches!(i, 3..=5 | 7) {
                repaint |= s.pointer(&e, now).repaint;
            }
        }
        let r = self.tray.pointer(&e, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            self.open_bar_menu(cx.text(), &theme, now);
        }

        // Tıklama klavye odağını taşır.
        if let ui_input::PointerKind::Press(ui_input::Button::Left) = e.kind {
            let hit = self.inputs.iter().position(|i| i.rect().contains(e.position));
            let focus = match hit {
                Some(i) => Focus::Input(i),
                None if self.cards[6].rect.contains(e.position) => Focus::Hex,
                None if self.cards[7].rect.contains(e.position) => Focus::Calendar,
                None if self.list.rect().contains(e.position) => Focus::List,
                None => Focus::None,
            };
            self.set_focus(focus, now);
            repaint = true;
        }
        for input in &mut self.inputs {
            repaint |= input.pointer(cx.text(), &theme, &e, now).repaint;
        }

        let r = self.list.pointer(&theme, &e, now);
        repaint |= r.repaint;
        if let Some((i, ev)) = r.event {
            self.set_event(match ev {
                ListEvent::Clicked => format!("Son olay: liste #{i}"),
                ListEvent::Toggled(on) => format!("Son olay: liste #{i} → {on}"),
            });
        }

        let r = self.add_notification.pointer(&e, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            let n = self.next_notification;
            self.next_notification += 1;
            self.notifications.insert(0, (n, notification(&theme, n)));
        }
        let mut note = None;
        for (n, card) in &mut self.notifications {
            let r = card.pointer(&theme, &e, now);
            repaint |= r.repaint;
            if let Some(ev) = r.event {
                note = Some(match ev {
                    NotificationEvent::Activated => format!("Son olay: bildirim {n} açıldı"),
                    NotificationEvent::Action(a) => format!("Son olay: bildirim {n} eylem {a}"),
                    NotificationEvent::Dismissed => format!("Son olay: bildirim {n} kapatıldı"),
                    NotificationEvent::Link(url) => format!("Son olay: bağlantı {url}"),
                });
            }
        }
        if let Some(t) = note {
            self.set_event(t);
        }

        let r = self.picker.pointer(cx.text(), &theme, &e, now);
        repaint |= r.repaint;
        if let Some(ColorEvent::Committed(c)) = r.event {
            self.set_event(format!("Son olay: renk {}", c.to_hex()));
        }
        let r = self.calendar.pointer(&theme, &e, now);
        repaint |= r.repaint;
        if let Some(ev) = r.event {
            self.set_event(match ev {
                CalendarEvent::DaySelected(d) => format!("Son olay: {:04}-{:02}-{:02}", d.year, d.month, d.day),
                CalendarEvent::MonthChanged { year, month } => format!("Son olay: ay {year}-{month:02}"),
            });
        }

        // İpucu: ikon düğmeleri ve bar başlığı. Gecikmeyi bileşen yönetir; hover uyanma anı
        // döndürürse tek zamanlayıcı kurulur (beklerken kare çizilmez).
        let names = ["Ayarlar", "Bluetooth (açık)", "Bluetooth (kapalı)", "Wi-Fi (devre dışı)"];
        let target = if e.kind == ui_input::PointerKind::Leave {
            None
        } else if let Some(i) = self.icon_buttons.iter().position(|b| b.rect().contains(e.position)) {
            Some((names[i % 4], self.icon_buttons[i].rect(), MenuSide::Below))
        } else if self.bar_title.contains(e.position) {
            Some((LONG_TITLE, self.bar_title, MenuSide::away_from(ScreenEdge::Top)))
        } else {
            None
        };
        if self.state != "tooltip" {
            match target {
                Some((text, anchor, side)) => {
                    let was = self.tooltip.is_visible();
                    let bounds = self.surface_bounds(&theme);
                    match self.tooltip.hover(text, anchor, side, bounds, now) {
                        Some(at) => self.arm_wake(cx, at, now),
                        None => repaint |= !was || self.tooltip.is_visible(),
                    }
                }
                None if self.tooltip.is_visible() || self.tooltip.deadline().is_some() => {
                    self.tooltip.unhover(now);
                    repaint = true;
                }
                None => {}
            }
        }

        if let ui_input::PointerKind::Press(ui_input::Button::Right) = e.kind {
            let bounds = self.surface_bounds(&theme);
            self.context.open(cx.text(), &theme, context_model(), MenuAnchor::at_point(e.position, theme.space(1)), bounds, now);
            repaint = true;
        }
        if repaint {
            cx.request_frame(surface);
        }
    }
}

// --- Bağlama göre imleç -------------------------------------------------------------------

impl Gallery {
    /// En üstteki isabetten başlayarak imleç (açık menüler en üstte).
    fn cursor_at(&self, p: Point) -> CursorShape {
        if self.page == 10 {
            return self.files.cursor_at(p);
        }
        if self.page == 9 {
            return self.glyphs.cursor_at(p);
        }
        if self.page == 8 {
            return CursorShape::Default;
        }
        if self.page == 7 {
            return self.themes.cursor_at(p);
        }
        if self.page == 5 {
            return self.disc.cursor_at(p);
        }
        if self.page == 6 {
            return self.nav.cursor_at(p);
        }
        if self.page == 4 {
            return self.cal.cursor_at(p);
        }
        if self.page == 3 {
            return self.shell.cursor_at(p);
        }
        if self.page == 2 {
            return self.apps.cursor_at(p);
        }
        if self.page == 1 {
            return self.settings.cursor_at(p);
        }
        CursorShape::pick(
            [self.context.cursor(p), self.menu.cursor(p), self.tray.cursor(p)]
                .into_iter()
                .chain(self.notifications.iter().map(|(_, c)| c.cursor(p)))
                .chain([self.add_notification.cursor(p), self.list.cursor(p), self.picker.cursor(p), self.calendar.cursor(p)])
                .chain(self.inputs.iter().map(|i| i.cursor(p)))
                .chain(self.buttons.iter().map(|b| b.cursor(p)))
                .chain(self.icon_buttons.iter().map(|b| b.cursor(p)))
                .chain(self.toggles.iter().map(|t| t.cursor(p)))
                .chain(self.sliders.iter().map(|s| s.cursor(p)))
                .chain([self.badge_bump.cursor(p)]),
        )
    }
}

impl SettingsPage {
    fn cursor_at(&self, p: Point) -> CursorShape {
        // Açık popover: içerik (renk seçici) önce, sonra kabın kendisi.
        if self.popover.is_open() {
            return CursorShape::pick([self.picker.cursor(p), self.popover.cursor(p)]);
        }
        CursorShape::pick(
            self.selects
                .iter()
                .map(|s| s.cursor(p))
                .chain(self.segments.iter().map(|s| s.cursor(p)))
                .chain([
                    self.color_button.cursor(p),
                    self.bump.cursor(p),
                    self.bezier.cursor(p),
                    self.spring.cursor(p),
                    self.reorder.cursor(p),
                    self.hidden.cursor(p),
                ]),
        )
    }
}

// --- Sayfa 3: uygulama düzeyi bileşenler ---------------------------------------------------

/// Odak sırası: 0 ray, 1 arama, 2 tarih, 3 saat, 4 saat (12), 5 diyalog düğmesi, 6 snackbar düğmesi,
/// 7–8 yerinde onay düğmeleri.
const APP_FOCUS: usize = 10;

struct AppPage {
    rail: NavigationRail,
    collapse: IconButton,
    title: Label,
    cards: Vec<Card>,
    search: SearchField,
    filters: ChipGroup,
    inputs: ChipGroup,
    date: DatePicker,
    time24: TimePicker,
    time12: TimePicker,
    /// Not satırları: bilgi (yumuşak), uyarı, başarı; ardından sıkı (Caption) uyarı ve başarı.
    notices: [Notice; 5],
    dialog_button: Button,
    snack_button: Button,
    /// Yerinde onay: yalnız ikon (kapat), etiketli (sıfırla) ve büyük ikonlu (oturum kartı gibi).
    confirms: [ConfirmButton; 3],
    dialog: Dialog,
    dialog_body: Label,
    snackbar: Snackbar,
    focus: FocusScope,
    last_event: Label,
    size: Size,
}

const RAIL_ITEMS: [(IconName, &str); 5] =
    [(IconName::Palette, "Görünüm"), (IconName::Wifi, "Ağ"), (IconName::VolumeHigh, "Ses"), (IconName::Notifications, "Bildirimler"), (IconName::Info, "Hakkında")];

impl AppPage {
    fn new(theme: &Theme) -> Self {
        let caption = text_style(theme, TextRole::Caption);
        let today = Date::new(2026, 10, 4);
        let filters = ["Okunmamış", "Bugün", "Ekli dosya", "Önemli"]
            .iter()
            .enumerate()
            .map(|(i, l)| Chip::new(theme, ChipKind::Filter, l).selected(i == 1))
            .collect();
        let inputs = ["ayse@ornek.org", "mehmet@ornek.org", "Proje toplantısı"]
            .iter()
            .map(|l| Chip::new(theme, ChipKind::Input, l).with_icon(theme, Icon::named(IconName::Person)))
            .collect();
        Self {
            rail: NavigationRail::new(theme, RAIL_ITEMS.iter().map(|(i, l)| NavItem::new(Icon::named(*i), *l)).collect(), 0),
            collapse: IconButton::new(theme, Icon::named(IconName::Menu), IconButtonVariant::Standard),
            title: Label::new("Görünüm", text_style(theme, TextRole::Display)),
            cards: ["Arama · chip'ler", "Tarih · saat", "Diyalog · snackbar · odak"].into_iter().map(|t| Card::new(theme, t)).collect(),
            search: SearchField::new(theme, "Ayarlarda ara…"),
            filters: ChipGroup::new(filters, false),
            inputs: ChipGroup::new(inputs, false),
            date: DatePicker::new(theme, Date::new(2026, 10, 9), today, "tr_TR"),
            // Uygulamalar `nuxut_i18n::time_locale()` verir; galeri sabit locale ile iki döngüyü gösterir.
            time24: TimePicker::with_locale(theme, Time::new(14, 30), &gallery_locale(None)).with_minute_step(1),
            time12: TimePicker::with_locale(theme, Time::new(9, 15), &gallery_locale(Some(true))),
            notices: [
                Notice::new(theme, NoticeKind::Info, "Saat ağdan eşitleniyor; elle ayar için otomatik eşitlemeyi kapatın."),
                Notice::new(theme, NoticeKind::Warning, "Saat dilimi bulunamadı; UTC kullanılıyor."),
                Notice::new(theme, NoticeKind::Success, "Ayarlar kaydedildi."),
                // Sıkı boyut: form satırının altındaki geçersiz kılma uyarısı (Ayarlar).
                Notice::new(theme, NoticeKind::Warning, "hyprland.conf bu değeri geçersiz kılıyor (input:repeat_rate).").compact(theme),
                Notice::new(theme, NoticeKind::Success, "Eşitlendi: 2 dk önce.").compact(theme),
            ],
            dialog_button: Button::new(theme, ButtonVariant::Destructive).with_icon(theme, Icon::named(IconName::Delete)).with_label(theme, "Hesabı kaldır…"),
            snack_button: Button::new(theme, ButtonVariant::Tonal).with_icon(theme, Icon::named(IconName::Save)).with_label(theme, "Kaydet"),
            confirms: [
                ConfirmButton::new(theme, ButtonVariant::Text, Icon::named(IconName::Power), "Kapat · tekrar bas"),
                ConfirmButton::new(theme, ButtonVariant::Tonal, Icon::named(IconName::Refresh), "Sıfırla · tekrar bas")
                    .with_label(theme, "Ayarları sıfırla"),
                ConfirmButton::new(theme, ButtonVariant::Tonal, Icon::named(IconName::Power), "Kapat · tekrar bas")
                    .with_armed_variant(ButtonVariant::Destructive)
                    .with_icon_role(theme, TextRole::IconLarge),
            ],
            dialog: Dialog::new(theme),
            dialog_body: Label::new(
                "Bu hesabın takvimleri ve etkinlikleri bu cihazdan kaldırılacak. Sunucudaki veriler etkilenmez.",
                text_style(theme, TextRole::Body),
            ),
            snackbar: Snackbar::new(theme),
            focus: FocusScope::new(APP_FOCUS),
            last_event: Label::new("Son olay: —  (Tab: odak sırası, Ctrl+Tab: sayfa)", caption),
            size: Size::ZERO,
        }
    }

    fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let s = |n| theme.space(n);
        self.size = size;
        let now = Instant::now();
        let m = s(5);
        let rail_w = self.rail.current_width(text, theme, now);
        self.rail.set_rect(theme, Rect::new(m, m + s(14), rail_w, size.height - m * 2.0 - s(14)));
        self.collapse.set_rect(Rect::new(m + s(5), m + s(2), s(10), s(10)));
        let x0 = m * 2.0 + rail_w;
        let col_w = ((size.width - x0 - m * 3.0) / 3.0).floor();
        let top = m + s(16);
        let h = size.height - top - m;
        for (i, c) in self.cards.iter_mut().enumerate() {
            c.rect = Rect::new(x0 + (col_w + m) * i as f32, top, col_w, h);
        }
        let body = self.cards[0].body(theme);
        let ss = self.search.preferred_size(theme, body.width);
        self.search.set_rect(theme, Rect::new(body.x, body.y, ss.width, ss.height));
        let fh = self.filters.layout(text, theme, Point::new(body.x, body.y + ss.height + s(6)), body.width);
        self.inputs.layout(text, theme, Point::new(body.x, body.y + ss.height + s(12) + fh), body.width);

        let body = self.cards[1].body(theme);
        let bounds = Rect::new(0.0, 0.0, size.width, size.height).inset(Insets::uniform(s(2)));
        let mut y = body.y;
        let ds = self.date.preferred_size(theme, body.width);
        self.date.set_rect(Rect::new(body.x, y, ds.width, ds.height));
        self.date.set_bounds(bounds);
        y += ds.height + s(4);
        for t in [&mut self.time24, &mut self.time12] {
            let ts = t.preferred_size(theme, body.width * 0.6);
            t.set_rect(Rect::new(body.x, y, ts.width, ts.height));
            t.set_bounds(bounds);
            y += ts.height + s(4);
        }
        for n in &mut self.notices {
            let ns = n.measure(text, theme, body.width);
            n.set_rect(Rect::new(body.x, y, ns.width, ns.height));
            y += ns.height + if n.size() == NoticeSize::Compact { s(2) } else { s(3) };
        }

        let body = self.cards[2].body(theme);
        let bs = self.dialog_button.preferred_size(text, theme);
        self.dialog_button.set_rect(Rect::new(body.x, body.y, bs.width, bs.height));
        let ks = self.snack_button.preferred_size(text, theme);
        self.snack_button.set_rect(Rect::new(body.x, body.y + bs.height + s(3), ks.width, ks.height));
        self.layout_confirms(text, theme);
        self.dialog_body.set_max_width(Some(s(90)));
    }

    /// Onay düğmeleri geçerli durumlarının boyutunda, sola hizalı (onayda sağa doğru açılır).
    fn layout_confirms(&mut self, text: &mut TextSystem, theme: &Theme) {
        let mut y = self.snack_button.rect().bottom() + theme.space(3);
        let x = self.snack_button.rect().x;
        for (i, c) in self.confirms.iter_mut().enumerate() {
            let mut cs = c.preferred_size(text, theme);
            if i == 2 {
                // Büyük ikonlu düğme oturum kartındaki gibi daha büyük karede (halka ona sığar).
                cs = Size::new(cs.width.max(theme.space(16)), theme.space(16));
            }
            c.set_rect(Rect::new(x, y, cs.width, cs.height));
            y += cs.height + theme.space(3);
        }
    }

    /// Onay düğmesi olayı: içerik değişti → yerleşim; son olay satırı.
    fn confirm_event(&mut self, text: &mut TextSystem, theme: &Theme, i: usize, ev: Option<ConfirmEvent>) {
        if let Some(ev) = ev {
            self.layout_confirms(text, theme);
            self.event(format!("Son olay: onay {i} {ev:?}"));
        }
    }

    /// `NUXUT_GALLERY_STATE=confirm`: iki düğme de onay bekler.
    fn arm_confirms(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        for i in 0..self.confirms.len() {
            let ev = self.confirms[i].key(&press_key(ui_input::Key::Enter), now).event;
            self.confirm_event(text, theme, i, ev);
        }
    }

    fn surface(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.width, self.size.height)
    }

    fn open_dialog(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        let body = self.dialog_body.measure(text);
        let actions = vec![
            DialogAction::new("Vazgeç", DialogActionKind::Default),
            DialogAction::new("Kaldır", DialogActionKind::Destructive),
        ];
        let bounds = self.surface();
        self.dialog.open(text, theme, "Hesap kaldırılsın mı?", actions, body, bounds, now);
    }

    fn open_date(&mut self, theme: &Theme, now: Instant) {
        let _ = self.date.key(theme, &press_key(ui_input::Key::Enter), now);
    }

    fn open_time(&mut self, theme: &Theme, twelve: bool, now: Instant) {
        let t = if twelve { &mut self.time12 } else { &mut self.time24 };
        let _ = t.key(theme, &press_key(ui_input::Key::Enter), now);
    }

    fn show_snackbar(&mut self, text: &mut TextSystem, theme: &Theme, now: Instant) {
        let area = self.surface();
        self.snackbar.show(text, theme, "Ayarlar kaydedildi", Some("Geri al"), area, now);
    }

    fn event(&mut self, t: String) {
        self.last_event.set_text(&t);
    }

    /// Odak halkalarını odak durumuna göre günceller.
    fn sync_focus(&mut self, now: Instant) {
        let f = &self.focus;
        self.rail.set_focus_visible(f.ring_visible_for(0));
        self.search.set_focused(f.focused() == Some(1), now);
        self.date.set_focus_visible(f.ring_visible_for(2));
        self.time24.set_focus_visible(f.ring_visible_for(3));
        self.time12.set_focus_visible(f.ring_visible_for(4));
        // Odak giden saat alanı yarım serbest girişi bırakır.
        if f.focused() != Some(3) {
            self.time24.cancel_entry();
        }
        if f.focused() != Some(4) {
            self.time12.cancel_entry();
        }
        self.dialog_button.set_focus_visible(f.ring_visible_for(5));
        self.snack_button.set_focus_visible(f.ring_visible_for(6));
        for (i, c) in self.confirms.iter_mut().enumerate() {
            c.set_focus_visible(f.ring_visible_for(7 + i));
        }
    }

    /// Klavye; tüketildiyse `true`.
    fn key(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::KeyEvent, now: Instant) -> bool {
        if self.dialog.is_open() {
            if let Some(ev) = self.dialog.key(e, now).event {
                self.event(format!("Son olay: diyalog {ev:?}"));
            }
            return true;
        }
        for (i, open) in [(2, self.date.is_open()), (3, self.time24.is_open()), (4, self.time12.is_open())] {
            if open {
                self.focus_key(text, theme, i, e, now);
                return true;
            }
        }
        // Bekleyen onay varsa Escape önce ondan vazgeçer (odak ve sayfa yerinde kalır).
        if e.key == ui_input::Key::Escape {
            for i in 0..self.confirms.len() {
                if self.confirms[i].is_armed(now) {
                    let ev = self.confirms[i].key(e, now).event;
                    self.confirm_event(text, theme, i, ev);
                    return true;
                }
            }
        }
        if self.focus.key(e).is_some() {
            self.sync_focus(now);
            return true;
        }
        match self.focus.focused() {
            Some(i) => {
                self.focus_key(text, theme, i, e, now);
                // Esc odağı bırakır (sonraki Esc çıkar).
                if e.key == ui_input::Key::Escape {
                    self.focus.clear();
                    self.sync_focus(now);
                }
                true
            }
            None => false,
        }
    }

    fn focus_key(&mut self, text: &mut TextSystem, theme: &Theme, i: usize, e: &ui_input::KeyEvent, now: Instant) {
        match i {
            0 => {
                if let Some(v) = self.rail.key(theme, e, now).event {
                    self.title.set_text(RAIL_ITEMS[v].1);
                }
            }
            1 => {
                let _ = self.search.key(e, now);
            }
            2 => {
                if let Some(DateEvent::Changed(d)) = self.date.key(theme, e, now).event {
                    self.event(format!("Son olay: tarih {:04}-{:02}-{:02}", d.year, d.month, d.day));
                }
            }
            3 | 4 => {
                let t = if i == 3 { &mut self.time24 } else { &mut self.time12 };
                if let Some(TimeEvent::Changed(v)) = t.key(theme, e, now).event {
                    let s = format_time(v, t.day_periods());
                    self.event(format!("Son olay: saat {s}"));
                }
            }
            5 if self.dialog_button.key(e, now).event.is_some() => self.open_dialog(text, theme, now),
            6 if self.snack_button.key(e, now).event.is_some() => self.show_snackbar(text, theme, now),
            7..=9 => {
                let ev = self.confirms[i - 7].key(e, now).event;
                self.confirm_event(text, theme, i - 7, ev);
            }
            _ => {}
        }
    }

    fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        // Modal diyalog her şeyi tüketir.
        if self.dialog.is_open() {
            if let Some(ev) = self.dialog.pointer(e, now).event {
                self.event(format!("Son olay: diyalog {ev:?}"));
            }
            return true;
        }
        if let Some(SnackbarEvent::Action) = self.snackbar.pointer(e, now).event {
            self.event("Son olay: geri alındı".into());
        }
        // Açık açılır seçiciler en üstte.
        if self.date.is_open() {
            if let Some(DateEvent::Changed(d)) = self.date.pointer(theme, e, now).event {
                self.event(format!("Son olay: tarih {:04}-{:02}-{:02}", d.year, d.month, d.day));
            }
            return true;
        }
        for t in [&mut self.time24, &mut self.time12] {
            if t.is_open() {
                if let Some(TimeEvent::Changed(v)) = t.pointer(theme, e, now).event {
                    let s = format_time(v, t.day_periods());
                    self.last_event.set_text(&format!("Son olay: saat {s}"));
                }
                return true;
            }
        }
        if let ui_input::PointerKind::Press(ui_input::Button::Left) = e.kind {
            let hit = [
                self.rail.rect().contains(e.position),
                self.search.rect().contains(e.position),
                self.date.rect().contains(e.position),
                self.time24.rect().contains(e.position),
                self.time12.rect().contains(e.position),
                self.dialog_button.rect().contains(e.position),
                self.snack_button.rect().contains(e.position),
                self.confirms[0].rect().contains(e.position),
                self.confirms[1].rect().contains(e.position),
                self.confirms[2].rect().contains(e.position),
            ]
            .iter()
            .position(|h| *h);
            match hit {
                Some(i) => {
                    let _ = self.focus.focus_pointer(i);
                }
                None => {
                    self.focus.clear();
                }
            }
            self.sync_focus(now);
        }
        if self.collapse.pointer(e, now).event.is_some() {
            let c = !self.rail.is_collapsed();
            self.rail.set_collapsed(theme, c, now);
        }
        if let Some(v) = self.rail.pointer(theme, e, now).event {
            self.title.set_text(RAIL_ITEMS[v].1);
        }
        let _ = self.search.pointer(text, theme, e, now);
        for (g, name) in [(&mut self.filters, "filtre"), (&mut self.inputs, "giriş")] {
            if let Some((i, ev)) = g.pointer(text, theme, e, now).event {
                self.last_event.set_text(&format!("Son olay: {name} chip {i} {ev:?}"));
            }
        }
        let _ = self.date.pointer(theme, e, now);
        let _ = self.time24.pointer(theme, e, now);
        let _ = self.time12.pointer(theme, e, now);
        if self.dialog_button.pointer(e, now).event.is_some() {
            self.open_dialog(text, theme, now);
        }
        if self.snack_button.pointer(e, now).event.is_some() {
            self.show_snackbar(text, theme, now);
        }
        // Onay düğmeleri tüm basışları görür (dışarı basış vazgeçirir).
        for i in 0..self.confirms.len() {
            let ev = self.confirms[i].pointer(e, now).event;
            self.confirm_event(text, theme, i, ev);
        }
        true
    }

    fn cursor_at(&self, p: Point) -> CursorShape {
        if self.dialog.is_open() {
            return self.dialog.cursor(p).unwrap_or_default();
        }
        CursorShape::pick([
            self.date.cursor(p).filter(|_| self.date.is_open()),
            self.time24.cursor(p).filter(|_| self.time24.is_open()),
            self.time12.cursor(p).filter(|_| self.time12.is_open()),
            self.snackbar.cursor(p),
            self.collapse.cursor(p),
            self.rail.cursor(p),
            self.search.cursor(p),
            self.filters.cursor(p),
            self.inputs.cursor(p),
            self.date.cursor(p),
            self.time24.cursor(p),
            self.time12.cursor(p),
            self.dialog_button.cursor(p),
            self.snack_button.cursor(p),
            self.confirms[0].cursor(p),
            self.confirms[1].cursor(p),
            self.confirms[2].cursor(p),
        ])
    }

    fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let now = ui.now;
        // Ray genişliği yayla değişiyorsa yerleşim izler.
        let w = self.rail.current_width(ui.painter.text_system(), theme, now);
        if (w - self.rail.rect().width).abs() > 0.25 {
            let size = self.size;
            self.layout(ui.painter.text_system(), theme, size);
            ui.painter.damage_all();
        }
        self.collapse.draw(ui);
        let tx = self.rail.rect().right() + theme.space(5);
        self.title.draw(ui.painter, Point::new(tx, theme.space(6)), theme.colors.on_surface);
        self.rail.draw(ui);
        for c in &mut self.cards {
            c.draw(ui);
        }
        self.search.draw(ui);
        self.filters.draw(ui);
        self.inputs.draw(ui);
        self.date.draw(ui);
        self.time24.draw(ui);
        self.time12.draw(ui);
        for n in &mut self.notices {
            n.draw(ui);
        }
        self.dialog_button.draw(ui);
        self.snack_button.draw(ui);
        // Süre dolduysa içerik boşa döner: yerleşim önce (çizim yeni boyutla).
        let mut expired = false;
        for c in &mut self.confirms {
            expired |= c.expire(now);
        }
        if expired {
            self.layout_confirms(ui.painter.text_system(), theme);
            self.event("Son olay: onay süresi doldu".into());
        }
        for c in &mut self.confirms {
            c.draw(ui);
        }
        let at =Point::new(self.cards[2].body(theme).x, self.cards[2].rect.bottom() - theme.space(10));
        ui.damage(Rect::new(at.x, at.y, theme.space(110), theme.space(6)));
        self.last_event.draw(ui.painter, at, theme.colors.on_surface_variant);
        // Üst katmanlar: açılır seçiciler, snackbar, modal diyalog.
        self.date.draw_overlay(ui);
        self.time24.draw_overlay(ui);
        self.time12.draw_overlay(ui);
        self.snackbar.draw(ui);
        let body = &mut self.dialog_body;
        self.dialog.draw(ui, |ui, rect| body.draw(ui.painter, rect.origin(), ui.fade(ui.theme.colors.on_surface_variant)));
    }
}

// --- Sayfa 4 yardımcıları: OSD zamanlayıcısı ve ölçüm --------------------------------------

/// Hızlı kaydırma ölçümünün kare sayısı.
const MEASURE_STEPS: usize = 400;

impl Gallery {
    /// Sayfa 4 OSD istediyse zamanlayıcıyı (yeniden) kurar: her gösterim süreyi uzatır.
    fn arm_osd_timer(&mut self, cx: &mut Context<Self>) {
        if let Some(after) = self.shell.osd_timer.take() {
            if let Some(old) = self.osd_timer.take() {
                cx.cancel_timer(old);
            }
            self.osd_timer = Some(cx.set_timer(after));
        }
    }
}

/// Bu sürecin kullanıcı + sistem CPU tıkı (`/proc/self/stat`, alan 14–15).
fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let rest = stat.rsplit_once(')').map_or("", |(_, r)| r);
    rest.split_whitespace().skip(11).take(2).filter_map(|f| f.parse::<u64>().ok()).sum()
}

/// Yerleşik bellek (VmRSS, KiB).
fn rss_kb() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// Galerinin saat locale'i: `tr_TR` + `[locale] hour_cycle` geçersiz kılması (`None` = auto).
fn gallery_locale(twelve: Option<bool>) -> nuxut_locale::Locale {
    nuxut_locale::Locale::parse("tr_TR").unwrap_or_default().with_12_hour(twelve)
}
