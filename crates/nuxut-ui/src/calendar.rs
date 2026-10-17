//! Ay görünümü takvim ızgarası: bugün vurgusu, seçim, etkinlik noktaları, ay geçişinde yayla
//! kayan içerik. Hafta başlangıcı ve adlar locale verisinden: [`CalendarGrid::with_locale`] (ör.
//! `nuxut::i18n::time_locale()`, framework.toml `[locale]`) ya da ad dizgesiyle [`CalendarGrid::new`].
//!
//! Tarih hesabı bağımlılıksız (proleptik Gregoryen, Howard Hinnant'ın `days_from_civil`
//! algoritması).

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_locale::{Length, Locale, Weekday};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle};
use nuxut_theme::{STATE_HOVER, Theme};

use crate::button::{IconButton, IconButtonVariant};
use crate::icon_names::IconName;
use crate::icon::Icon;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::label::Label;
use crate::style::{DISABLED_CONTENT, HAIRLINE, accent_on, faded_mark, faded_text, state_layer_under, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Gün başına gösterilen en fazla etkinlik noktası.
pub const MAX_EVENT_DOTS: usize = 3;

/// Ay başlığı: tek satır, sığmazsa üç nokta (genişlik `layout_title`'da).
fn title_style(theme: &Theme) -> nuxut_render::TextStyle {
    text_style(theme, TextRole::Title).with_max_lines(1)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
}

impl Date {
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        let month = month.clamp(1, 12);
        let day = day.clamp(1, days_in_month(year, month));
        Self { year, month, day }
    }

    /// 1970-01-01'den gün sayısı.
    pub fn to_days(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    pub fn from_days(days: i64) -> Self {
        civil_from_days(days)
    }

    /// 0 = Pazartesi … 6 = Pazar.
    pub fn weekday(self) -> u8 {
        // 1970-01-01 Perşembe (3).
        ((self.to_days() + 3).rem_euclid(7)) as u8
    }

    pub fn add_days(self, n: i64) -> Self {
        Self::from_days(self.to_days() + n)
    }

    /// Ay ekler; gün yeni ayın sonuna sınırlanır.
    pub fn add_months(self, n: i32) -> Self {
        let total = self.year * 12 + i32::from(self.month) - 1 + n;
        let (y, m) = (total.div_euclid(12), (total.rem_euclid(12) + 1) as u8);
        Self::new(y, m, self.day)
    }

    /// Yerel saatle bugün (ortam `TZ`'si okunmaz; UTC'ye göre gün + sistem ofseti çağıranın).
    pub fn from_unix_seconds(secs: i64, utc_offset_secs: i64) -> Self {
        Self::from_days((secs + utc_offset_secs).div_euclid(86_400))
    }
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> Date {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    Date { year: (y + i64::from(m <= 2)) as i32, month: m, day: d }
}

/// Locale dizgesinden (`tr_TR.UTF-8`, `en_US`) haftanın ilk günü: 0 = Pazartesi, 5 = Cumartesi,
/// 6 = Pazar (`nuxut_locale::Locale::first_weekday`; çözülemeyen ad → Pazartesi).
pub fn first_weekday_for_locale(locale: &str) -> u8 {
    Locale::parse(locale).map_or(0, |l| first_weekday_of(&l))
}

/// Çözülmüş locale'den haftanın ilk günü (0 = Pazartesi … 6 = Pazar); ör.
/// `nuxut::i18n::time_locale()` (framework.toml `[locale] formats`): tr → 0, en_US → 6.
pub fn first_weekday_of(locale: &Locale) -> u8 {
    locale.first_weekday().num_days_from_monday() as u8
}

/// Ortamdan (`LC_ALL` > `LC_TIME` > `LANG`, ilk boş olmayan karar verir) locale adı; `C`,
/// `POSIX` ya da tanınmayan → `en_US` (`nuxut_locale::Locale::from_env`).
pub fn locale_from_env() -> String {
    Locale::from_env().name().to_owned()
}

/// Ay ve gün adları, locale verisinden (glibc; tanınmayan ad → `en_US`).
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarLabels {
    /// Bağımsız (yalın) ay adları — başlık biçimi ([`Locale::standalone_month_name`]: ru
    /// "Октябрь", pl "październik"; ja "10月").
    pub months: [String; 12],
    /// Pazartesi'den başlayarak kısa gün adları.
    pub weekdays: [String; 7],
}

impl CalendarLabels {
    pub fn for_locale(locale: &str) -> Self {
        Self::from_locale(&Locale::parse(locale).unwrap_or_default())
    }

    /// Çözülmüş locale'den (ör. `nuxut::i18n::time_locale()`).
    pub fn from_locale(l: &Locale) -> Self {
        Self {
            months: std::array::from_fn(|i| l.standalone_month_name(i as u32 + 1)),
            weekdays: std::array::from_fn(|i| l.weekday_name((0..i).fold(Weekday::Mon, |d, _| d.succ()), Length::Short)),
        }
    }
}

/// Dar gün adı: kısa adın (locale verisi) ilk grafemi — "Prş" → "P", "Çar" → "Ç", "火" → "火".
/// glibc'de CLDR "narrow" biçimi yok; CLDR'nin narrow adları da çoğu dilde ilk harftir.
/// Birleşik aksanlı/çok kod noktalı ilk harf bölünmez.
pub fn narrow_weekday(short: &str) -> &str {
    use unicode_segmentation::UnicodeSegmentation;
    short.graphemes(true).next().unwrap_or("")
}

/// Ölçüme dayalı dar genişlik kararları (gün adı biçimi, rakam/nokta yerleşimi). Metin sistemi
/// `set_rect`'te yok: çizimde (`prepare`) hesaplanır, hücre boyu ya da etiketler değişmedikçe
/// yeniden hesaplanmaz.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CellLayout {
    cell: f32,
    /// Kısa adlardan biri hücreye boşluklu sığmıyor: hepsi tek grafeme düşer.
    narrow_weekdays: bool,
    /// Gün rakamının hücre merkezinden düşey kayması (≤ 0; noktalara yer açmak için, tüm
    /// hücrelerde aynı — rakamlar hizalı kalır). Varsayılan genişliklerde 0.
    number_dy: f32,
    /// Nokta satırının hücre üstüne göre y'si.
    dot_y: f32,
    /// Hücreye sığan en fazla nokta (≤ [`MAX_EVENT_DOTS`]; hiç yer yoksa 0).
    dots: usize,
    /// Bugün/seçim/üzerinde dairesi, hücre köşesine göre. Varsayılan: hücre `inset` içeride.
    disc: Rect,
}

impl CellLayout {
    /// Bugün/seçim dairesi. Nokta satırı varsayılan dairenin içinde kalıyorsa (geniş hücre)
    /// daire aynen; dar hücrede noktalar dairenin kenarına binerdi (yarısı dolgu üstünde, yarısı
    /// dışında — seçili günde `primary` nokta `primary` daireye karışıyordu): daire rakamı
    /// ortalayıp noktaların yarım boşluk üstünde biter, noktalar dışarıda kendi renginde kalır.
    /// Daire rakamın satır kutusunun yarısından küçülecekse varsayılana dönülür.
    fn disc(cell: f32, number_h: f32, d: f32, number_dy: f32, dot_y: f32, dots: usize) -> Rect {
        let inset = d;
        let full = Rect::new(inset, inset, cell - 2.0 * inset, cell - 2.0 * inset);
        if dots == 0 || dot_y + d <= full.bottom() {
            return full;
        }
        let cy = cell * 0.5 + number_dy;
        let r = (dot_y - d * 0.25 - cy).min(cy).min(full.width * 0.5);
        if r < number_h * 0.5 {
            return full;
        }
        Rect::new(cell * 0.5 - r, cy - r, r * 2.0, r * 2.0)
    }

    /// `cell`: hücre kenarı, `number_h`: rakam satır kutusu yüksekliği, `d`: nokta çapı (=
    /// `inset`). Varsayılan yer (hücre dairesinin altından `1.75 d` yukarı) rakama değmiyorsa
    /// aynen kalır; değiyorsa noktalar rakamın altına iner (hücre içinde kalarak), yetmezse
    /// rakam yukarı kayar (hücre üstüne kadar), o da yetmezse nokta çizilmez.
    fn dots(cell: f32, number_h: f32, d: f32) -> (f32, f32, usize) {
        let inset = d;
        let gap = d * 0.5;
        let default_y = cell - inset - d * 1.75;
        // Rakamın satır kutusu + yarım çap: iniş payıyla birlikte rakam ile nokta arasında net
        // boşluk (yalnız kutuya değmemek dar hücrede 1–2 px bırakıyordu).
        let number_bottom = (cell + number_h) * 0.5 + gap;
        let fit = ((cell - 2.0 * inset + gap) / (d + gap)).floor().max(0.0) as usize;
        let count = fit.min(MAX_EVENT_DOTS);
        if default_y >= number_bottom {
            return (0.0, default_y, count);
        }
        // Noktanın hücre altıyla payı: yarım çap (komşu satırın dairesi `inset` içeride başlar).
        let lowest = cell - gap - d;
        if number_bottom <= lowest {
            return (0.0, number_bottom, count);
        }
        let need = number_bottom - lowest;
        let room = ((cell - number_h) * 0.5).max(0.0);
        if need <= room {
            (-need, lowest, count)
        } else {
            (0.0, default_y, 0)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarEvent {
    DaySelected(Date),
    /// Görünen ay değişti (etkinlik noktalarını yüklemek için).
    MonthChanged { year: i32, month: u8 },
}

/// Bir ayın çizime hazır hâli.
#[derive(Debug, Clone)]
struct MonthView {
    year: i32,
    month: u8,
    /// 42 hücre (6 hafta): tarih + o ayda mı.
    cells: Vec<(Date, bool)>,
    numbers: Vec<Label>,
}

impl MonthView {
    fn new(theme: &Theme, year: i32, month: u8, first_weekday: u8) -> Self {
        let first = Date::new(year, month, 1);
        let lead = (i64::from(first.weekday()) - i64::from(first_weekday)).rem_euclid(7);
        let start = first.add_days(-lead);
        let cells: Vec<(Date, bool)> = (0..42).map(|i| {
            let d = start.add_days(i);
            (d, d.month == month)
        }).collect();
        let style = text_style(theme, TextRole::Body);
        let numbers = cells.iter().map(|(d, _)| Label::new(d.day.to_string(), style.clone())).collect();
        Self { year, month, cells, numbers }
    }
}

#[derive(Debug, Clone)]
pub struct CalendarGrid {
    rect: Rect,
    first_weekday: u8,
    labels: CalendarLabels,
    /// Başlık sırası ("Ekim 2026" / "2026年10月") için.
    locale: Locale,
    today: Date,
    selected: Option<Date>,
    /// (tarih, nokta renkleri); `None` renk = `primary`.
    events: Vec<(Date, Vec<Option<Color>>)>,
    view: MonthView,
    old: Option<(MonthView, f32)>,
    progress: Animated<f32>,
    title: Label,
    weekday_labels: Vec<Label>,
    /// Dar biçim (ilk grafem), `weekday_labels` ile aynı sıra.
    weekday_narrow: Vec<Label>,
    /// `prepare` sonucu; `None` = yeniden ölçülecek.
    cell_layout: Option<CellLayout>,
    prev: IconButton,
    next: IconButton,
    hovered: Option<usize>,
    damage: DamageState,
    /// Gün ızgarası (dikdörtgen, hücre kenarı); imleç için `set_rect`'te hesaplanır.
    grid_cache: (Rect, f32),
}

impl CalendarGrid {
    /// `locale`: [`locale_from_env`] ya da sabit (`"tr_TR"`).
    pub fn new(theme: &Theme, today: Date, locale: &str) -> Self {
        let mut c = Self::with_locale(theme, today, &Locale::parse(locale).unwrap_or_default());
        // Eski sözleşme: çözülemeyen ad → en_US adları ama Pazartesi başlangıcı.
        let first = first_weekday_for_locale(locale);
        if first != c.first_weekday {
            c.set_first_weekday(theme, first);
        }
        c
    }

    /// Çözülmüş locale ile: ay/gün adları, başlık sırası ve hafta başı oradan. Uygulamalar
    /// `nuxut::i18n::time_locale()` verir (framework.toml `[locale] formats`; `auto` = ortam).
    /// Locale değişince ([`set_locale`](Self::set_locale)) yeniden kurmak gerekmez.
    pub fn with_locale(theme: &Theme, today: Date, locale: &Locale) -> Self {
        let first_weekday = first_weekday_of(locale);
        let labels = CalendarLabels::from_locale(locale);
        let view = MonthView::new(theme, today.year, today.month, first_weekday);
        let mut c = Self {
            rect: Rect::ZERO,
            first_weekday,
            title: Label::new("", title_style(theme)),
            weekday_labels: Vec::new(),
            weekday_narrow: Vec::new(),
            cell_layout: None,
            labels,
            locale: locale.clone(),
            today,
            selected: None,
            events: Vec::new(),
            view,
            old: None,
            progress: Animated::new(1.0, theme.motion.spatial),
            prev: IconButton::new(theme, Icon::named(IconName::ChevronLeft), IconButtonVariant::Standard),
            next: IconButton::new(theme, Icon::named(IconName::ChevronRight), IconButtonVariant::Standard),
            hovered: None,
            damage: DamageState::new(),
            grid_cache: (Rect::ZERO, 0.0),
        };
        c.relabel(theme);
        c
    }

    pub fn first_weekday(&self) -> u8 {
        self.first_weekday
    }

    pub fn set_first_weekday(&mut self, theme: &Theme, first: u8) {
        self.first_weekday = first.min(6);
        self.view = MonthView::new(theme, self.view.year, self.view.month, self.first_weekday);
        self.relabel(theme);
    }

    /// Locale değişimi (`locale_changed`): adlar, başlık sırası ve hafta başı yeni locale'den;
    /// görünen ay, seçim, bugün ve etkinlikler korunur, süren ay geçişi biter. Aynı locale'de
    /// iş yapmaz.
    pub fn set_locale(&mut self, theme: &Theme, locale: &Locale) {
        if &self.locale == locale {
            return;
        }
        self.locale = locale.clone();
        self.labels = CalendarLabels::from_locale(locale);
        self.first_weekday = first_weekday_of(locale);
        self.view = MonthView::new(theme, self.view.year, self.view.month, self.first_weekday);
        self.old = None;
        self.hovered = None;
        self.relabel(theme);
    }

    pub fn set_labels(&mut self, theme: &Theme, labels: CalendarLabels) {
        self.labels = labels;
        self.relabel(theme);
    }

    fn relabel(&mut self, theme: &Theme) {
        let name = &self.labels.months[usize::from(self.view.month.clamp(1, 12) - 1)];
        self.title.set_text(&self.locale.compose_month_year(name, self.view.year));
        let caption = text_style(theme, TextRole::Caption);
        let names: Vec<&str> = (0..7).map(|i| self.labels.weekdays[(usize::from(self.first_weekday) + i) % 7].as_str()).collect();
        // Ay değişiminde de çağrılır: aynı adlar yeniden şekillendirilmez (önbellekli Label).
        let unchanged = self.weekday_labels.len() == 7
            && self.weekday_labels[0].style() == &caption
            && self.weekday_labels.iter().zip(&names).all(|(l, n)| l.text() == *n);
        if !unchanged {
            self.weekday_labels = names.iter().map(|n| Label::new(*n, caption.clone())).collect();
            self.weekday_narrow = names.iter().map(|n| Label::new(narrow_weekday(n), caption.clone())).collect();
            self.cell_layout = None;
        }
        self.damage.mark();
    }

    /// Ölçüme dayalı yerleşimi (dar gün adları, rakam/nokta yeri) hesaplar; hücre boyu ve
    /// etiketler değişmedikçe iş yapmaz. `draw` kendisi çağırır.
    fn prepare(&mut self, theme: &Theme, text: &mut nuxut_render::TextSystem) {
        let cell = self.grid_cache.1;
        if self.cell_layout.is_some_and(|l| l.cell == cell) {
            return;
        }
        // Komşu adlar arasında en az space(2) kalmalı (okunur ayrım); biri sığmazsa hepsi dar
        // (karışık biçim yok).
        let room = cell - theme.space(2);
        let narrow_weekdays = self.weekday_labels.iter_mut().any(|l| l.measure(text).width > room);
        let number_h = self.view.numbers.first_mut().map_or(0.0, |l| l.measure(text).height);
        let d = theme.space(1);
        let (number_dy, dot_y, dots) = CellLayout::dots(cell, number_h, d);
        let disc = CellLayout::disc(cell, number_h, d, number_dy, dot_y, dots);
        self.cell_layout = Some(CellLayout { cell, narrow_weekdays, number_dy, dot_y, dots, disc });
    }

    pub fn month(&self) -> (i32, u8) {
        (self.view.year, self.view.month)
    }

    pub fn selected(&self) -> Option<Date> {
        self.selected
    }

    /// Gün döndü (gece yarısı zamanlayıcısı).
    pub fn set_today(&mut self, today: Date) {
        if self.today != today {
            self.today = today;
            self.damage.mark();
        }
    }

    pub fn set_selected(&mut self, date: Option<Date>) {
        if self.selected != date {
            self.selected = date;
            self.damage.mark();
        }
    }

    /// Etkinlik noktaları (gün başına en fazla [`MAX_EVENT_DOTS`] çizilir).
    pub fn set_events(&mut self, events: Vec<(Date, Vec<Option<Color>>)>) {
        self.events = events;
        self.damage.mark();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.progress.set_motion(theme.motion.spatial, now);
        self.title.set_style(title_style(theme));
        self.prev.set_theme(theme, now);
        self.next.set_theme(theme, now);
        self.view = MonthView::new(theme, self.view.year, self.view.month, self.first_weekday);
        self.cell_layout = None;
        self.relabel(theme);
        self.layout_title(theme);
    }

    /// Başlık önceki-ay düğmesine kadar olan alanla sınırlı (sığmazsa üç nokta).
    fn layout_title(&mut self, theme: &Theme) {
        if self.rect.width <= 0.0 {
            return;
        }
        let left = self.rect.x + theme.space(1);
        let w = (self.prev.rect().x - theme.space(1) - left).max(0.0);
        self.title.set_max_width(Some(w));
    }

    /// Genişlik verilir; hücreler kare.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        let cell = (width / 7.0).floor();
        Size::new(width, theme.space(12) + theme.space(7) + cell * 6.0)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.rect == rect {
            return;
        }
        self.rect = rect;
        let s = theme.space(10);
        let y = rect.y + (theme.space(12) - s) * 0.5;
        self.next.set_rect(Rect::new(rect.right() - s, y, s, s));
        self.prev.set_rect(Rect::new(rect.right() - s * 2.0 - theme.space(1), y, s, s));
        self.grid_cache = self.grid(theme);
        self.layout_title(theme);
        self.damage.mark();
    }

    /// İmleç: günler ve ay düğmeleri `Pointer`, başlık ve boşluk `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if let Some(c) = self.prev.cursor(p).or_else(|| self.next.cursor(p)) {
            return Some(c);
        }
        if self.grid_cache.0.contains(p) {
            return Some(CursorShape::Pointer);
        }
        self.rect.contains(p).then_some(CursorShape::Default)
    }

    fn grid(&self, theme: &Theme) -> (Rect, f32) {
        let cell = (self.rect.width / 7.0).floor();
        let top = self.rect.y + theme.space(12) + theme.space(7);
        let x = self.rect.x + (self.rect.width - cell * 7.0) * 0.5;
        (Rect::new(x, top, cell * 7.0, cell * 6.0), cell)
    }

    fn cell_at(&self, theme: &Theme, p: Point) -> Option<usize> {
        let (g, cell) = self.grid(theme);
        if !g.contains(p) {
            return None;
        }
        let col = ((p.x - g.x) / cell) as usize;
        let row = ((p.y - g.y) / cell) as usize;
        Some((row * 7 + col).min(41))
    }

    /// Aya geç (`dir` = +1 sonraki, −1 önceki); içerik o yönde kayar.
    pub fn show_month(&mut self, theme: &Theme, year: i32, month: u8, now: Instant) -> Option<CalendarEvent> {
        if (year, month) == (self.view.year, self.view.month) {
            return None;
        }
        let dir = if (year, month) > (self.view.year, self.view.month) { 1.0 } else { -1.0 };
        let new = MonthView::new(theme, year, month, self.first_weekday);
        let old = std::mem::replace(&mut self.view, new);
        self.old = Some((old, dir));
        self.progress.jump_to(0.0);
        self.progress.set_target(1.0, now);
        self.hovered = None;
        self.relabel(theme);
        Some(CalendarEvent::MonthChanged { year, month })
    }

    fn step_month(&mut self, theme: &Theme, n: i32, now: Instant) -> Option<CalendarEvent> {
        let d = Date::new(self.view.year, self.view.month, 1).add_months(n);
        self.show_month(theme, d.year, d.month, now)
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<CalendarEvent> {
        let mut out = Response::none();
        let r = self.prev.pointer(event, now);
        out.repaint |= r.repaint;
        if r.event.is_some() {
            return Response::none().with_repaint(true).map_some(self.step_month(theme, -1, now));
        }
        let r = self.next.pointer(event, now);
        out.repaint |= r.repaint;
        if r.event.is_some() {
            return Response::none().with_repaint(true).map_some(self.step_month(theme, 1, now));
        }
        let cell = if event.kind == PointerKind::Leave { None } else { self.cell_at(theme, event.position) };
        if cell != self.hovered {
            self.hovered = cell;
            self.damage.mark();
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Release(Button::Left) => {
                if let Some(i) = cell {
                    let (d, in_month) = self.view.cells[i];
                    self.selected = Some(d);
                    self.damage.mark();
                    if !in_month {
                        let _ = self.show_month(theme, d.year, d.month, now);
                    }
                    return Response::event(CalendarEvent::DaySelected(d));
                }
            }
            PointerKind::Scroll { steps, delta } if self.rect.contains(event.position) => {
                let dy = if steps.y != 0.0 { steps.y } else { delta.y / theme.space(12) };
                if dy.abs() >= 0.5 {
                    return Response::none().with_repaint(true).map_some(self.step_month(theme, dy.signum() as i32, now));
                }
            }
            _ => {}
        }
        out
    }

    /// Ok tuşları seçimi gün/hafta kaydırır (ay sınırında ay değişir), PageUp/PageDown ay,
    /// Enter seçimi bildirir.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<CalendarEvent> {
        if event.state == KeyState::Released {
            return Response::none();
        }
        let base = self.selected.unwrap_or(self.today);
        let next = match event.key {
            Key::Left => base.add_days(-1),
            Key::Right => base.add_days(1),
            Key::Up => base.add_days(-7),
            Key::Down => base.add_days(7),
            Key::PageUp => base.add_months(-1),
            Key::PageDown => base.add_months(1),
            Key::Home => self.today,
            Key::Enter | Key::Space => return Response::event(CalendarEvent::DaySelected(base)),
            _ => return Response::none(),
        };
        self.selected = Some(next);
        self.damage.mark();
        let changed = self.show_month(theme, next.year, next.month, now);
        Response { repaint: true, event: changed }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.rect;
        self.prepare(theme, ui.painter.text_system());
        let layout = self.cell_layout.expect("prepare");
        // Başlık + gezinme.
        let ts = self.title.measure_in(ui.painter);
        self.title.draw(ui.painter, Point::new(rect.x + theme.space(1), rect.y + (theme.space(12) - ts.height) * 0.5), ui.fade(c.on_surface));
        self.prev.draw(ui);
        self.next.draw(ui);
        let (grid, cell) = self.grid(theme);
        let wy = rect.y + theme.space(12);
        let weekdays = if layout.narrow_weekdays { &mut self.weekday_narrow } else { &mut self.weekday_labels };
        for (i, l) in weekdays.iter_mut().enumerate() {
            let m = l.measure_in(ui.painter);
            let cx = grid.x + cell * (i as f32 + 0.5);
            l.draw(ui.painter, Point::new(cx - m.width * 0.5, wy + (theme.space(7) - m.height) * 0.5), ui.fade(c.on_surface_variant));
        }

        let p = self.progress.value(now);
        let slide = cell * 2.0;
        ui.painter.push_clip(grid.expand(theme.space(1)));
        if let Some((old, dir)) = &mut self.old {
            let a = (1.0 - p * 1.6).clamp(0.0, 1.0);
            let off = -*dir * slide * p;
            ui.with_alpha(a, |ui| draw_month(ui, old, grid, layout, off, None, self.today, self.selected, &self.events));
        }
        let (alpha, off) = match &self.old {
            Some((_, dir)) => (((p - 0.25) / 0.75).clamp(0.0, 1.0), *dir * slide * (1.0 - p)),
            None => (1.0, 0.0),
        };
        let hovered = self.hovered;
        ui.with_alpha(alpha, |ui| draw_month(ui, &mut self.view, grid, layout, off, hovered, self.today, self.selected, &self.events));
        ui.painter.pop_clip();

        let animating = !self.progress.is_settled(now);
        if animating {
            ui.request_frame();
        } else {
            self.old = None;
        }
        self.damage.report(ui, rect.expand(theme.space(1)), animating);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_month(
    ui: &mut Ui,
    view: &mut MonthView,
    grid: Rect,
    layout: CellLayout,
    dx: f32,
    hovered: Option<usize>,
    today: Date,
    selected: Option<Date>,
    events: &[(Date, Vec<Option<Color>>)],
) {
    let theme = ui.theme;
    let c = &theme.colors;
    let cell = layout.cell;
    for (i, ((date, in_month), label)) in view.cells.iter().zip(view.numbers.iter_mut()).enumerate() {
        let (row, col) = (i / 7, i % 7);
        let r = Rect::new(grid.x + col as f32 * cell + dx, grid.y + row as f32 * cell, cell, cell);
        let disc = Rect::new(r.x + layout.disc.x, r.y + layout.disc.y, layout.disc.width, layout.disc.height);
        let radii = Corners::uniform(disc.width.min(disc.height) * 0.5);
        let is_today = *date == today;
        let is_selected = selected == Some(*date);
        let colors = DayColors::new(theme, *in_month, is_today, is_selected, hovered == Some(i));
        if let Some(fill) = colors.fill {
            ui.painter.rect(disc, &RectStyle::filled(ui.fade(fill), radii));
        }
        if let Some(ring) = colors.ring {
            let ring = RectStyle { fill: Color::TRANSPARENT, radii, border: Some(Border { width: HAIRLINE, color: ui.fade(ring) }) };
            ui.painter.rect(disc, &ring);
        }
        if colors.layer.a > 0.0 {
            ui.painter.rect(disc, &RectStyle::filled(ui.fade(colors.layer), radii));
        }
        let text = colors.text;
        let m = label.measure_in(ui.painter);
        label.draw(ui.painter, Point::new(r.center().x - m.width * 0.5, r.center().y - m.height * 0.5 + layout.number_dy), ui.fade(text));
        if let Some((_, dots)) = events.iter().find(|(d, _)| d == date) {
            let n = dots.len().min(layout.dots);
            let d = theme.space(1);
            let gap = d * 0.5;
            let total = n as f32 * d + (n.saturating_sub(1)) as f32 * gap;
            let y = r.y + layout.dot_y;
            // Dar hücrede noktalar dairenin altında (`CellLayout::disc`): orada kendi renginde.
            let on_disc = y + d <= disc.bottom();
            for (k, col) in dots.iter().take(n).enumerate() {
                let x = r.center().x - total * 0.5 + k as f32 * (d + gap);
                let color = if is_selected && on_disc { c.on_primary } else { col.unwrap_or(c.primary) };
                let fade = if *in_month { 1.0 } else { DISABLED_CONTENT };
                let under = if on_disc { colors.fill.unwrap_or(Color::TRANSPARENT) } else { Color::TRANSPARENT };
                ui.painter.rect(Rect::new(x, y, d, d), &RectStyle::filled(ui.fade(faded_mark(theme, color, fade, &[under])), Corners::uniform(d * 0.5)));
            }
        }
    }
}

/// Ay görünümünde bir günün renkleri (saf; yüksek karşıtlık testleri bunun üstünden). Tonal
/// dil: seçili gün `primary` dolgu, bugün tonal dolgulu daire (+ `primary` metin); kenar yalnızca
/// kenar kipi açıksa bugünün çevresinde. Ay dışı günler soluk; yüksek karşıtlıkta yine ≥ 4.5
/// (hover katmanı dahil gerçek zemine göre, `style::faded_text`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DayColors {
    pub fill: Option<Color>,
    pub ring: Option<Color>,
    /// Hover state layer (yoksa saydam).
    pub layer: Color,
    pub text: Color,
}

impl DayColors {
    pub(crate) fn new(theme: &Theme, in_month: bool, today: bool, selected: bool, hovered: bool) -> Self {
        let c = &theme.colors;
        let (fill, ring, strong) = if selected {
            (Some(c.primary), None, c.on_primary)
        } else if today {
            let t = tonal_container(theme);
            (Some(t), theme.outline(), accent_on(theme, t))
        } else {
            (None, None, c.on_surface)
        };
        let under = fill.unwrap_or(Color::TRANSPARENT);
        let layer = if hovered {
            let rgb = if selected { c.on_primary } else { c.on_surface };
            state_layer_under(theme, rgb, STATE_HOVER, &[under], &[strong])
        } else {
            Color::TRANSPARENT
        };
        let text = if in_month || selected || today { strong } else { faded_text(theme, c.on_surface, DISABLED_CONTENT, &[under, layer]) };
        Self { fill, ring, layer, text }
    }
}

trait MapSome<T> {
    fn map_some(self, e: Option<T>) -> Response<T>;
}

impl<T> MapSome<T> for Response<T> {
    fn map_some(self, e: Option<T>) -> Response<T> {
        Response { repaint: self.repaint || e.is_some(), event: e }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn date_math() {
        assert_eq!(Date::new(1970, 1, 1).to_days(), 0);
        assert_eq!(Date::from_days(0), Date::new(1970, 1, 1));
        let d = Date::new(2026, 10, 4);
        assert_eq!(Date::from_days(d.to_days()), d);
        assert_eq!(d.weekday(), 6, "4 Ekim 2026 Pazar");
        assert_eq!(Date::new(2024, 2, 29).add_days(1), Date::new(2024, 3, 1));
        assert_eq!(Date::new(2024, 1, 31).add_months(1), Date::new(2024, 2, 29));
        assert_eq!(Date::new(2024, 1, 15).add_months(-13), Date::new(2022, 12, 15));
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(Date::from_unix_seconds(1_791_100_000, 3 * 3600), Date::new(2026, 10, 4));
    }

    #[test]
    fn locale_week_start() {
        assert_eq!(first_weekday_for_locale("tr_TR.UTF-8"), 0);
        assert_eq!(first_weekday_for_locale("en_US.UTF-8"), 6);
        assert_eq!(first_weekday_for_locale("ar_EG"), 5);
        assert_eq!(first_weekday_for_locale("de_DE@euro"), 0);
        assert_eq!(first_weekday_for_locale(""), 0);
        assert_eq!(CalendarLabels::for_locale("tr_TR").months[9], "Ekim");
        assert_eq!(CalendarLabels::for_locale("tr_TR").weekdays[0], "Pzt");
        assert_eq!(CalendarLabels::for_locale("de_DE.UTF-8").months[2], "März");
        assert_eq!(CalendarLabels::for_locale("").weekdays[6], "Sun");
    }

    #[test]
    fn month_title_follows_locale_order() {
        let theme = Theme::default();
        let title = |loc: &str| CalendarGrid::new(&theme, Date::new(2026, 10, 4), loc).title.text().to_owned();
        assert_eq!(title("tr_TR"), "Ekim 2026");
        assert_eq!(title("en_US"), "October 2026");
        assert_eq!(title("ja_JP"), "2026年10月");
        assert_eq!(title("ko_KR"), "2026년 10월");
        // Başlıkta yalın ad (tarih içi -in hâli değil).
        assert_eq!(title("ru_RU"), "Октябрь 2026");
    }

    #[test]
    fn grid_starts_on_locale_weekday() {
        let theme = Theme::default();
        let g = CalendarGrid::new(&theme, Date::new(2026, 10, 4), "tr_TR");
        // Ekim 2026'nın 1'i Perşembe: Pazartesi başlangıçlı ızgarada 3 önceki gün.
        assert_eq!(g.view.cells[0].0, Date::new(2026, 9, 28));
        assert_eq!(g.view.cells[3].0, Date::new(2026, 10, 1));
        let g = CalendarGrid::new(&theme, Date::new(2026, 10, 4), "en_US");
        assert_eq!(g.view.cells[0].0, Date::new(2026, 9, 27));
    }

    #[test]
    fn week_start_from_resolved_locale() {
        let theme = Theme::default();
        let today = Date::new(2026, 10, 4);
        let loc = |s: &str| Locale::parse(s).unwrap();
        let tr = CalendarGrid::with_locale(&theme, today, &loc("tr_TR"));
        assert_eq!(tr.first_weekday(), 0);
        assert_eq!(tr.weekday_labels[0].text(), "Pzt");
        assert_eq!(tr.title.text(), "Ekim 2026");
        let mut g = CalendarGrid::with_locale(&theme, today, &loc("en_US"));
        assert_eq!(g.first_weekday(), 6);
        assert_eq!(g.weekday_labels[0].text(), "Sun");
        assert_eq!(g.view.cells[0].0, Date::new(2026, 9, 27));
        // Canlı locale değişimi: görünen ay, seçim ve etkinlikler korunur.
        let t0 = Instant::now();
        let _ = g.show_month(&theme, 2026, 11, t0);
        g.set_selected(Some(Date::new(2026, 11, 3)));
        g.set_events(vec![(Date::new(2026, 11, 3), vec![None])]);
        g.set_locale(&theme, &loc("tr_TR"));
        assert_eq!(g.first_weekday(), 0);
        assert_eq!(g.month(), (2026, 11));
        assert_eq!(g.selected(), Some(Date::new(2026, 11, 3)));
        assert_eq!(g.events.len(), 1);
        assert!(g.old.is_none());
        assert_eq!(g.title.text(), "Kasım 2026");
        assert_eq!(g.weekday_labels[6].text(), "Paz");
        // 1 Kasım 2026 Pazar: Pazartesi başlangıçlı ızgarada 6 önceki gün.
        assert_eq!(g.view.cells[0].0, Date::new(2026, 10, 26));
        // Cumartesi başlangıcı (ar_EG), hour_cycle geçersiz kılması hafta başını etkilemez.
        g.set_locale(&theme, &loc("ar_EG").with_12_hour(Some(false)));
        assert_eq!(g.first_weekday(), 5);
        // `new` sözleşmesi aynen: çözülemeyen ad → Pazartesi.
        assert_eq!(CalendarGrid::new(&theme, today, "").first_weekday(), 0);
        assert_eq!(CalendarGrid::new(&theme, today, "en_US.UTF-8").first_weekday(), 6);
        assert_eq!(first_weekday_of(&loc("tr_TR")), first_weekday_for_locale("tr_TR"));
    }

    #[test]
    fn navigation_and_selection() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = CalendarGrid::new(&theme, Date::new(2026, 10, 4), "tr_TR");
        let s = g.preferred_size(&theme, 280.0);
        g.set_rect(&theme, Rect::new(0.0, 0.0, s.width, s.height));
        let nr = g.next.rect().center();
        let _ = g.pointer(&theme, &motion(nr.x, nr.y), t0);
        let _ = g.pointer(&theme, &press(nr.x, nr.y), t0);
        let r = g.pointer(&theme, &release(nr.x, nr.y), t0);
        assert_eq!(r.event, Some(CalendarEvent::MonthChanged { year: 2026, month: 11 }));
        // Klavye: PageUp geri, Sol ok ay sınırını geçer.
        let _ = g.key(&theme, &key(Key::Home), t0);
        let r = g.key(&theme, &key(Key::Left), t0);
        assert_eq!(g.selected(), Some(Date::new(2026, 10, 3)));
        assert!(r.repaint);
        // Hücreye tıklama seçer.
        let (grid, cell) = g.grid(&theme);
        let p = Point::new(grid.x + cell * 3.5, grid.y + cell * 0.5);
        let _ = g.pointer(&theme, &press(p.x, p.y), t0);
        assert_eq!(g.pointer(&theme, &release(p.x, p.y), t0).event, Some(CalendarEvent::DaySelected(Date::new(2026, 10, 1))));
    }

    #[test]
    fn narrow_weekday_is_first_grapheme() {
        assert_eq!(narrow_weekday("Prş"), "P");
        assert_eq!(narrow_weekday("Çar"), "Ç");
        // Ayrık aksan bölünmez.
        assert_eq!(narrow_weekday("C\u{327}ar"), "C\u{327}");
        assert_eq!(narrow_weekday("火"), "火");
        assert_eq!(narrow_weekday(""), "");
    }

    fn laid(theme: &Theme, text: &mut nuxut_render::TextSystem, locale: &str, width: f32) -> CalendarGrid {
        let mut g = CalendarGrid::new(theme, Date::new(2026, 10, 4), locale);
        let s = g.preferred_size(theme, width);
        g.set_rect(theme, Rect::new(10.0, 20.0, s.width, s.height));
        g.prepare(theme, text);
        g
    }

    const LOCALES: [&str; 4] = ["tr_TR", "en_US", "de_DE", "ru_RU"];

    #[test]
    fn default_widths_keep_previous_layout() {
        let theme = Theme::default();
        let mut text = nuxut_render::TextSystem::new();
        for loc in LOCALES {
            for w in [340.0, 380.0] {
                let g = laid(&theme, &mut text, loc, w);
                let l = g.cell_layout.unwrap();
                let cell = (w / 7.0).floor();
                let d = theme.space(1);
                assert!(!l.narrow_weekdays, "{loc} {w}: kısa adlar");
                assert_eq!(l.number_dy, 0.0, "{loc} {w}");
                assert_eq!(l.dot_y, cell - d - d * 1.75, "{loc} {w}: eski nokta yeri");
                assert_eq!(l.disc, Rect::new(d, d, cell - 2.0 * d, cell - 2.0 * d), "{loc} {w}: eski daire");
                assert_eq!(l.dots, MAX_EVENT_DOTS);
            }
        }
    }

    #[test]
    fn narrow_widths_do_not_overlap() {
        let theme = Theme::default();
        let mut text = nuxut_render::TextSystem::new();
        let mut went_narrow = false;
        for loc in LOCALES {
            let mut w = 180.0;
            while w <= 400.0 {
                let mut g = laid(&theme, &mut text, loc, w);
                let l = g.cell_layout.unwrap();
                let cell = l.cell;
                // (a) Gün adları: her biri hücresinde, aralarında en az space(2).
                let labels = if l.narrow_weekdays { &mut g.weekday_narrow } else { &mut g.weekday_labels };
                went_narrow |= l.narrow_weekdays;
                for lab in labels.iter_mut() {
                    let lw = lab.measure(&mut text).width;
                    assert!(lw + theme.space(2) <= cell || (l.narrow_weekdays && lw <= cell), "{loc} {w}: '{}' {lw} > {cell}", lab.text());
                }
                // (b) Noktalar rakamın satır kutusuna değmez, hücrede kalır.
                let nh = g.view.numbers[0].measure(&mut text).height;
                let top = (cell - nh) * 0.5 + l.number_dy;
                assert!(top >= -0.01, "{loc} {w}: rakam hücreden taşar");
                if l.dots > 0 {
                    let d = theme.space(1);
                    assert!(l.dot_y >= top + nh - 0.01, "{loc} {w}: nokta {} rakam altı {}", l.dot_y, top + nh);
                    assert!(l.dot_y + d <= cell, "{loc} {w}: nokta hücreden taşar");
                    let row = l.dots as f32 * d + (l.dots - 1) as f32 * d * 0.5;
                    assert!(row <= cell - 2.0 * d, "{loc} {w}: nokta satırı");
                    // Noktalar daire kenarına binmez: ya tamamen içinde ya tamamen altında.
                    let disc = l.disc;
                    let inside = l.dot_y >= disc.y && l.dot_y + d <= disc.bottom();
                    assert!(inside || l.dot_y >= disc.bottom(), "{loc} {w}: nokta {} daire {disc:?}", l.dot_y);
                    assert!(disc.y >= 0.0 && disc.bottom() <= cell && disc.width >= nh, "{loc} {w}: daire {disc:?}");
                    assert!((disc.center().y - (cell * 0.5 + l.number_dy)).abs() < 0.01 || disc.width == cell - 2.0 * d, "{loc} {w}: daire rakamı ortalar");
                }
                // (c) Başlık önceki-ay düğmesine taşmaz.
                let tw = g.title.measure(&mut text).width;
                let left = g.rect.x + theme.space(1);
                assert!(left + tw <= g.prev.rect().x - theme.space(1) + 0.5, "{loc} {w}: başlık {tw}");
                assert!(g.title.measure(&mut text).height <= theme.space(12), "{loc} {w}: tek satır");
                w += 10.0;
            }
        }
        assert!(went_narrow, "dar genişlikte tek harfe düşülmeli");
    }

    #[test]
    fn popout_widths_keep_all_dots() {
        // Shell takvim popout'unun hedef genişlikleri: noktalar düşmez, rakam hücrede kalır.
        let theme = Theme::default();
        let mut text = nuxut_render::TextSystem::new();
        for loc in LOCALES {
            for w in [220.0, 240.0, 280.0, 320.0] {
                let g = laid(&theme, &mut text, loc, w);
                let l = g.cell_layout.unwrap();
                assert_eq!(l.dots, MAX_EVENT_DOTS, "{loc} {w}: {l:?}");
            }
        }
        // 220'de üç harfli adlar sığmaz: tek harf.
        assert!(laid(&theme, &mut text, "tr_TR", 220.0).cell_layout.unwrap().narrow_weekdays);
    }

    #[test]
    fn long_title_is_ellipsized_not_wrapped() {
        let theme = Theme::default();
        let mut text = nuxut_render::TextSystem::new();
        let free = text.layout("September 2026", &title_style(&theme), None).size();
        let mut g = laid(&theme, &mut text, "en_US", 200.0);
        let _ = g.show_month(&theme, 2026, 9, Instant::now());
        let avail = g.prev.rect().x - theme.space(1) - (g.rect.x + theme.space(1));
        assert!(free.width > avail, "senaryo: başlık sığmamalı ({} ≤ {avail})", free.width);
        let s = g.title.measure(&mut text);
        assert!(s.width <= avail + 0.5 && s.height <= free.height + 0.5, "{s:?}");
        // Varsayılan genişlikte aynı başlık kısalmaz.
        let mut g = laid(&theme, &mut text, "en_US", 340.0);
        let _ = g.show_month(&theme, 2026, 9, Instant::now());
        let s = g.title.measure(&mut text);
        assert!((s.height - free.height).abs() < 0.01);
        assert_eq!(g.title.layout(&mut text).line_count(), 1);
        assert!((s.width - free.width).abs() < 0.5, "kısalmadı: {} vs {}", s.width, free.width);
    }

    #[test]
    fn dot_layout_rules() {
        // Varsayılan yer rakama değmiyorsa aynen.
        assert_eq!(CellLayout::dots(48.0, 20.0, 4.0), (0.0, 48.0 - 4.0 - 7.0, 3));
        // Değiyorsa rakamın altına iner.
        let (dy, y, n) = CellLayout::dots(36.0, 20.0, 4.0);
        assert_eq!((dy, n), (0.0, 3));
        assert!(y >= 28.0 && y + 4.0 <= 36.0);
        // Altta yer yoksa rakam yukarı kayar.
        let (dy, y, n) = CellLayout::dots(30.0, 20.0, 4.0);
        assert!(dy < 0.0 && n == 3 && y + 4.0 <= 30.0 && y >= 15.0 + 10.0 + dy, "{dy} {y}");
        // Hiç yer yoksa nokta yok; dar hücrede sayı azalır.
        assert_eq!(CellLayout::dots(22.0, 20.0, 4.0).2, 0);
        assert_eq!(CellLayout::dots(18.0, 8.0, 4.0).2, 2);
        // Daire: noktalar içindeyse varsayılan; kenara biniyorsa rakamı ortalayıp noktaların
        // üstünde biter; rakam kutusunun yarısından küçülecekse varsayılan.
        assert_eq!(CellLayout::disc(48.0, 20.0, 4.0, 0.0, 37.0, 3), Rect::new(4.0, 4.0, 40.0, 40.0));
        assert_eq!(CellLayout::disc(31.0, 20.0, 4.0, -2.5, 25.0, 3), Rect::new(4.5, 2.0, 22.0, 22.0));
        assert_eq!(CellLayout::disc(31.0, 20.0, 4.0, -2.5, 25.0, 0), Rect::new(4.0, 4.0, 23.0, 23.0));
        assert_eq!(CellLayout::disc(26.0, 20.0, 4.0, 0.0, 20.0, 2), Rect::new(4.0, 4.0, 18.0, 18.0));
    }
}
