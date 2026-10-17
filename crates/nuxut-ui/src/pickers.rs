//! Tarih ve saat seçme alanları: [`DatePicker`] (alan + Popover içinde [`CalendarGrid`]) ve
//! [`TimePicker`] (alan + Popover içinde saat ve dakika ızgaraları; 12/24 saat locale'den,
//! yedek 24 saat; kurulu `Locale` ile `[locale] hour_cycle`), dakika adımı ve serbest giriş.

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_render::{RectStyle, TextStyle};
use nuxut_theme::{STATE_HOVER, Theme};

use crate::calendar::{CalendarEvent, CalendarGrid, CalendarLabels, Date};
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::label::Label;
use crate::menu::{MenuAlign, MenuAnchor, MenuSide};
use crate::popover::Popover;
use crate::pressable::Pressable;
use crate::segmented::{Segment, SegmentedButton};
use crate::select::draw_field;
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// Yerel uzun tarih metni ([`nuxut_locale::DateStyle::Long`]; sıra locale'in `%x` kalıbından):
/// tr "4 Ekim 2026", en_US "October 4, 2026", de "4. Oktober 2026", ja "2026年10月4日".
/// Çözülemeyen locale → `en_US`. `labels` yalnızca geçersiz tarihte (ay 13, gün 0) yedek ad
/// için kullanılır (imza geriye uyumlu).
pub fn format_date(date: Date, labels: &CalendarLabels, locale: &str) -> String {
    format_date_in(date, labels, &nuxut_locale::Locale::parse(locale).unwrap_or_default())
}

/// [`format_date`]'in çözülmüş locale sürümü (`DatePicker::with_locale` alan metni).
fn format_date_in(date: Date, labels: &CalendarLabels, locale: &nuxut_locale::Locale) -> String {
    locale
        .format_ymd(date.year, u32::from(date.month), u32::from(date.day), nuxut_locale::DateStyle::Long)
        .unwrap_or_else(|_| {
            let month = &labels.months[usize::from(date.month.clamp(1, 12) - 1)];
            format!("{} {month} {}", date.day, date.year)
        })
}

fn anchor_below(field: Rect, theme: &Theme) -> MenuAnchor {
    MenuAnchor { rect: field, side: MenuSide::Below, align: MenuAlign::Start, gap: theme.space(1) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateEvent {
    Changed(Date),
}

/// Tarih alanı + açılır takvim. Locale: [`DatePicker::with_locale`] (ör.
/// `nuxut_i18n::time_locale()`) alan metnini (`DateStyle::Long`), takvimin ay/gün adlarını ve hafta
/// başını verir; `locale_changed`'de [`DatePicker::set_locale`]. [`DatePicker::new`] ad dizgesiyle
/// (geriye uyumlu; çözülemeyen ad → en_US adları, Pazartesi başlangıcı).
#[derive(Debug, Clone)]
pub struct DatePicker {
    core: Pressable,
    label: Label,
    icon: IconView,
    date: Date,
    locale: nuxut_locale::Locale,
    labels: CalendarLabels,
    popover: Popover,
    calendar: CalendarGrid,
    bounds: Rect,
}

impl DatePicker {
    pub fn new(theme: &Theme, date: Date, today: Date, locale: &str) -> Self {
        let calendar = CalendarGrid::new(theme, today, locale);
        Self::build(theme, date, nuxut_locale::Locale::parse(locale).unwrap_or_default(), calendar)
    }

    /// Kurulu locale'den (`nuxut_i18n::time_locale()`; framework.toml `[locale] formats`).
    pub fn with_locale(theme: &Theme, date: Date, today: Date, locale: &nuxut_locale::Locale) -> Self {
        let calendar = CalendarGrid::with_locale(theme, today, locale);
        Self::build(theme, date, locale.clone(), calendar)
    }

    fn build(theme: &Theme, date: Date, locale: nuxut_locale::Locale, mut calendar: CalendarGrid) -> Self {
        calendar.set_selected(Some(date));
        let labels = CalendarLabels::from_locale(&locale);
        Self {
            core: Pressable::new(theme),
            label: Label::new(format_date_in(date, &labels, &locale), text_style(theme, TextRole::Body).with_max_lines(1)),
            icon: IconView::with_role(theme, Icon::named(IconName::CalendarToday), TextRole::IconSmall),
            date,
            locale,
            labels,
            popover: Popover::new(theme),
            calendar,
            bounds: Rect::ZERO,
        }
    }

    /// Locale değişti: alan metni, takvimin ay/gün adları ve hafta başı yeni locale'den
    /// ([`CalendarGrid::set_locale`]); seçili tarih ve görünen ay korunur, açık popover yeni
    /// boyuta yeniden kurulur. Değişiklik yoksa iş yapmaz (`false`).
    pub fn set_locale(&mut self, theme: &Theme, locale: &nuxut_locale::Locale, now: Instant) -> bool {
        if *locale == self.locale {
            return false;
        }
        self.locale = locale.clone();
        self.labels = CalendarLabels::from_locale(locale);
        self.label.set_text(&format_date_in(self.date, &self.labels, locale));
        self.calendar.set_locale(theme, locale);
        if self.popover.is_open() {
            let size = self.calendar.preferred_size(theme, theme.space(70));
            self.popover.set_content_size(theme, size, now);
            self.calendar.set_rect(theme, self.popover.content_target(theme));
        }
        self.core.damage.mark();
        true
    }

    /// Alanın kullandığı locale.
    pub fn locale(&self) -> &nuxut_locale::Locale {
        &self.locale
    }

    /// Açılır takvim (salt okunur; ör. hafta başı ve görünen ay).
    pub fn calendar(&self) -> &CalendarGrid {
        &self.calendar
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    pub fn date(&self) -> Date {
        self.date
    }

    pub fn set_date(&mut self, date: Date) {
        if self.date != date {
            self.date = date;
            self.label.set_text(&format_date_in(date, &self.labels, &self.locale));
            self.calendar.set_selected(Some(date));
            self.core.damage.mark();
        }
    }

    /// Alan metni (ör. "4 Ekim 2026").
    pub fn text(&self) -> &str {
        self.label.text()
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_open()
    }

    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.label.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
        self.icon.set_theme(theme, now);
        self.popover.set_theme(theme, now);
        self.calendar.set_theme(theme, now);
    }

    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(12))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    fn open(&mut self, theme: &Theme, now: Instant) {
        let size = self.calendar.preferred_size(theme, theme.space(70));
        self.calendar.show_month(theme, self.date.year, self.date.month, now);
        self.popover.open(theme, anchor_below(self.core.rect, theme), size, self.bounds, now);
        self.calendar.set_rect(theme, self.popover.content_target(theme));
        self.core.damage.mark();
    }

    fn pick(&mut self, d: Date, now: Instant) -> Response<DateEvent> {
        self.set_date(d);
        self.popover.close(now);
        Response::event(DateEvent::Changed(d))
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<DateEvent> {
        if self.popover.is_open() {
            if self.popover.pointer(event, now).event.is_some() {
                return Response::repaint();
            }
            let r = self.calendar.pointer(theme, event, now);
            if let Some(CalendarEvent::DaySelected(d)) = r.event {
                return self.pick(d, now);
            }
            return r.discard();
        }
        let r = self.core.pointer(event, now);
        if r.event.is_some() {
            self.open(theme, now);
            return Response::repaint();
        }
        r.discard()
    }

    /// Kapalıyken Enter/Space/Aşağı açar; açıkken oklar günü, Enter seçer, Esc kapatır.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<DateEvent> {
        if event.state == KeyState::Released {
            return Response::none();
        }
        if self.popover.is_open() {
            if self.popover.key(event, now).event.is_some() {
                return Response::repaint();
            }
            let r = self.calendar.key(theme, event, now);
            if let Some(CalendarEvent::DaySelected(d)) = r.event {
                return self.pick(d, now);
            }
            return r.discard();
        }
        if self.core.enabled && matches!(event.key, Key::Enter | Key::Space | Key::Down) {
            self.open(theme, now);
            return Response::repaint();
        }
        Response::none()
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.popover.is_open()
            && let Some(c) = self.calendar.cursor(p).or_else(|| self.popover.cursor(p))
        {
            return Some(c);
        }
        self.core.cursor(p)
    }

    pub fn input_rect(&self) -> Option<Rect> {
        self.popover.input_rect()
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let open = self.popover.is_open();
        draw_field(ui, &mut self.core, &mut self.label, &mut self.icon, open);
    }

    /// Açılır takvim: kaplama katmanında (en üstte, kapsayıcı kırpmasından bağımsız; alanın
    /// hemen ardından kırpılı kapsayıcının içinden çağrılabilir).
    pub fn draw_overlay(&mut self, ui: &mut Ui) {
        let cal = &mut self.calendar;
        self.popover.draw_overlay(ui, |ui, _| cal.draw(ui));
    }
}

/// Günün saati.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
}

impl Time {
    pub fn new(hour: u8, minute: u8) -> Self {
        Self { hour: hour.min(23), minute: minute.min(59) }
    }

    /// Dakika ekler (gün içinde dolanır).
    pub fn add_minutes(self, m: i32) -> Self {
        let t = (i32::from(self.hour) * 60 + i32::from(self.minute) + m).rem_euclid(24 * 60);
        Self { hour: (t / 60) as u8, minute: (t % 60) as u8 }
    }
}

/// Locale 12 saat mı kullanıyor (locale verisinin saat kalıbından, `nuxut_locale`); çözülemeyen
/// ad → 24 saat. `[locale] hour_cycle` geçersiz kılmasını görmez: onun için
/// [`nuxut_locale::Locale::uses_12_hour`] (`nuxut_i18n::time_locale()` ile).
pub fn uses_12_hour(locale: &str) -> bool {
    nuxut_locale::Locale::parse(locale).is_some_and(|l| l.uses_12_hour())
}

/// Locale 12 saatlikse gün dilimi adları (`[ÖÖ, ÖS]`, `[AM, PM]`; `nuxut_locale`), değilse `None`.
/// Addan çözer (geçersiz kılma yok); kurulu locale için [`day_periods_for`].
pub fn day_periods(locale: &str) -> Option<[String; 2]> {
    nuxut_locale::Locale::parse(locale).as_ref().and_then(day_periods_for)
}

/// [`day_periods`]'in kurulu `Locale` sürümü: `with_12_hour` geçersiz kılmasına uyar
/// (`hour_cycle = "h12"` + `de_DE` → `[AM, PM]`, `"h23"` + `en_US` → `None`).
pub fn day_periods_for(locale: &nuxut_locale::Locale) -> Option<[String; 2]> {
    locale.uses_12_hour().then(|| locale.day_periods())
}

/// "14:05" (`periods` yok: 24 saat) ya da "2:05 PM" (gün dilimi adları locale'den:
/// [`day_periods`]).
pub fn format_time(t: Time, periods: Option<&[String; 2]>) -> String {
    if let Some([am, pm]) = periods {
        let h = match t.hour % 12 {
            0 => 12,
            h => h,
        };
        format!("{h}:{:02} {}", t.minute, if t.hour < 12 { am } else { pm })
    } else {
        format!("{:02}:{:02}", t.hour, t.minute)
    }
}

/// Saat alanının metni: locale'in saat:dakika kalıbı (`Locale::format_hm`, tek kaynak);
/// biçimlenemezse (beklenmez) [`format_time`].
fn field_text(locale: &nuxut_locale::Locale, t: Time, periods: Option<&[String; 2]>) -> String {
    locale.format_hm(u32::from(t.hour), u32::from(t.minute)).unwrap_or_else(|_| format_time(t, periods))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeEvent {
    Changed(Time),
}

/// Varsayılan dakika adımı (ızgara, oklar, tekerlek).
pub const MINUTE_STEP: u8 = 5;

/// Geçerli dakika adımları: 60'ı bölenler (ızgara saat başına dolanır).
pub const MINUTE_STEPS: [u8; 11] = [1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30];

/// İstenen adımı geçerli bir adıma indirir: ondan büyük olmayan en büyük bölen (0 → 1, 59 → 30).
pub fn normalize_minute_step(step: u8) -> u8 {
    MINUTE_STEPS.iter().copied().rfind(|&s| s <= step).unwrap_or(1)
}

/// `t`'yi `step` dakikalık ızgarada `dir` yönünde bir adım götürür: hizalı değilse önce en
/// yakın ızgara çizgisine (14:07, adım 5: ↑ 14:10, ↓ 14:05). Gün içinde dolanır.
pub fn step_time(t: Time, step: u8, dir: i32) -> Time {
    let step = i32::from(normalize_minute_step(step));
    let m = i32::from(t.hour) * 60 + i32::from(t.minute);
    let rem = m.rem_euclid(step);
    let next = match dir.signum() {
        1 => m - rem + step,
        -1 if rem != 0 => m - rem,
        -1 => m - step,
        _ => m,
    };
    Time::new(0, 0).add_minutes(next)
}

/// Klavyeden yazılan saatin ayrıştırılmış hâli ([`parse_entry`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct EntryState {
    /// Tamamlanmış saat (24 saatlikte 0–23, 12 saatlikte 1–12).
    hour: Option<u8>,
    /// İkinci rakamı beklenen saatin ilk rakamı.
    first: Option<u8>,
    /// Dakikanın onlar basamağı.
    tens: Option<u8>,
    /// Tamamlanmış dakika.
    minute: Option<u8>,
}

/// Serbest girişte saat/dakika ayırıcısı (`:` ya da `.`); rakamlarla aynı dizide tutulur.
const ENTRY_SEP: u8 = 10;

/// Yazılan rakamları sırayla ayrıştırır; geçerli bir saate götürmeyen dizi → `None` (o rakam
/// reddedilir). Saatin ilk rakamı tek başına saat olamıyorsa (24 s: 3–9, 12 s: 2–9) saat
/// hemen tamamlanır ("7" → 07, dakikaya geçer). Bekleyen ilk rakamdan sonra iki basamaklı saat
/// yapamayan rakam dakikanın onlar basamağıdır (12 s "13" → 1:3_, 24 s "25" → 02:5_); ayırıcı
/// ([`ENTRY_SEP`]) bekleyen rakamı saat yapar ("1:30" → 01:30), saatten sonra iş yapmaz.
fn parse_entry(digits: &[u8], twelve: bool) -> Option<EntryState> {
    let hour_ok = |h: u8| if twelve { (1..=12).contains(&h) } else { h <= 23 };
    let mut s = EntryState::default();
    let mut sep = false;
    for &d in digits {
        match (s.hour, s.first) {
            _ if d > ENTRY_SEP => return None,
            (None, None) if d == ENTRY_SEP => return None,
            (None, None) => {
                if d >= if twelve { 2 } else { 3 } {
                    s.hour = Some(d);
                } else {
                    s.first = Some(d);
                }
            }
            (None, Some(p)) if d == ENTRY_SEP => {
                if !hour_ok(p) {
                    return None;
                }
                s.hour = Some(p);
                s.first = None;
                sep = true;
            }
            (None, Some(p)) => {
                let h = p * 10 + d;
                if hour_ok(h) {
                    s.hour = Some(h);
                } else if hour_ok(p) && d <= 5 {
                    s.hour = Some(p);
                    s.tens = Some(d);
                } else {
                    return None;
                }
                s.first = None;
            }
            // Saatten sonra tek ayırıcı (dakikadan önce) kabul, iş yapmaz.
            (Some(_), _) if d == ENTRY_SEP => {
                if sep || s.tens.is_some() {
                    return None;
                }
                sep = true;
            }
            (Some(_), _) => match (s.tens, s.minute) {
                (None, _) if d <= 5 => s.tens = Some(d),
                (Some(t), None) => s.minute = Some(t * 10 + d),
                _ => return None,
            },
        }
    }
    Some(s)
}

#[derive(Debug, Clone)]
struct Cell {
    label: Label,
    rect: Rect,
    value: u8,
}

fn hour_cells(style: &TextStyle, twelve: bool) -> Vec<Cell> {
    let cell = |v: u8, s: String| Cell { label: Label::new(s, style.clone()), rect: Rect::ZERO, value: v };
    if twelve { (1..=12u8).map(|h| cell(h % 12, h.to_string())).collect() } else { (0..24u8).map(|h| cell(h, format!("{h:02}"))).collect() }
}

fn minute_cells(style: &TextStyle, step: u8) -> Vec<Cell> {
    (0..60u8).step_by(usize::from(step)).map(|m| Cell { label: Label::new(format!("{m:02}"), style.clone()), rect: Rect::ZERO, value: m }).collect()
}

/// Saat alanı + Popover içinde saat ve dakika ızgaraları.
///
/// - 12/24 saat: [`TimePicker::with_locale`] / [`TimePicker::set_locale`] kurulu `Locale`'in
///   `uses_12_hour`'una uyar (`[locale] hour_cycle` dahil; uygulamalar
///   `nuxut_i18n::time_locale()` verir, `locale_changed`'de yeniden). [`TimePicker::new`]
///   locale adından çözer (geçersiz kılma yok; geriye uyumlu).
/// - Dakika adımı: [`TimePicker::with_minute_step`] (60'ı bölen; varsayılan [`MINUTE_STEP`]).
///   Izgara, ↑/↓ ve tekerlek adımı izler; 30'dan çok hücrede ızgara 10 sütun (1 dk: 6 satır).
/// - Serbest giriş: odaktayken rakam yazmak saati doğrudan girer (alan altı çizili, metin
///   "14:3_"; `:`/`.` saat/dakika ayırıcısı); geçersiz rakam reddedilir, dördüncü rakam
///   (ya da Enter) uygular, Backspace siler,
///   Esc vazgeçer. 12 saatlikte `a`/`p` gün dilimini seçer. Odak giderken çağıran
///   [`TimePicker::cancel_entry`] çağırır.
#[derive(Debug, Clone)]
pub struct TimePicker {
    core: Pressable,
    label: Label,
    icon: IconView,
    /// Saat locale'i (`hour_cycle` geçersiz kılması dahil): 12/24 saat, gün dilimi, alan metni.
    locale: nuxut_locale::Locale,
    time: Time,
    twelve: bool,
    /// 12 saatlikte gün dilimi adları (locale verisinden).
    periods: Option<[String; 2]>,
    step: u8,
    popover: Popover,
    hours: Vec<Cell>,
    minutes: Vec<Cell>,
    ampm: SegmentedButton,
    hovered: Option<(bool, usize)>,
    bounds: Rect,
    /// Serbest girişte kabul edilmiş rakamlar (boş = giriş yok).
    entry: Vec<u8>,
    /// Serbest girişte seçilen gün dilimi (`a`/`p`; `None` = şu anki saatinki).
    entry_pm: Option<bool>,
    /// Dokunmatik yüzey kaydırmasının henüz adıma dönüşmemiş kısmı.
    scroll_acc: f32,
}

impl TimePicker {
    /// Locale adından (ör. `LC_TIME`) 12/24 saat; `[locale] hour_cycle`'ı görmez — uygulamalar
    /// [`with_locale`](Self::with_locale) kullanmalı.
    pub fn new(theme: &Theme, time: Time, locale: &str) -> Self {
        // Çözülemeyen ad: 24 saat (eski davranış; en_US + h23 → "%H:%M" yedeği).
        let locale = nuxut_locale::Locale::parse(locale).unwrap_or_else(|| nuxut_locale::Locale::en_us().with_12_hour(Some(false)));
        Self::with_locale(theme, time, &locale)
    }

    /// Kurulu locale'den (`nuxut_i18n::time_locale()`): `hour_cycle` geçersiz kılmasına uyar.
    /// Alan metni `Locale::format_hm` ile (bar saatiyle aynı kalıp: tr "14:05", en_US "2:05 PM",
    /// fi "14.05").
    pub fn with_locale(theme: &Theme, time: Time, locale: &nuxut_locale::Locale) -> Self {
        let time = Time::new(time.hour, time.minute);
        let periods = day_periods_for(locale);
        let twelve = periods.is_some();
        let style = text_style(theme, TextRole::Label);
        Self {
            core: Pressable::new(theme),
            label: Label::new(field_text(locale, time, periods.as_ref()), text_style(theme, TextRole::Body).with_max_lines(1)),
            icon: IconView::with_role(theme, Icon::named(IconName::Schedule), TextRole::IconSmall),
            locale: locale.clone(),
            time,
            twelve,
            ampm: Self::ampm_button(theme, periods.as_ref(), time),
            periods,
            step: MINUTE_STEP,
            popover: Popover::new(theme),
            hours: hour_cells(&style, twelve),
            minutes: minute_cells(&style, MINUTE_STEP),
            hovered: None,
            bounds: Rect::ZERO,
            entry: Vec::new(),
            entry_pm: None,
            scroll_acc: 0.0,
        }
    }

    fn ampm_button(theme: &Theme, periods: Option<&[String; 2]>, time: Time) -> SegmentedButton {
        let [am, pm] = periods.cloned().unwrap_or_else(|| ["AM".to_owned(), "PM".to_owned()]);
        SegmentedButton::new(theme, vec![Segment::new(am), Segment::new(pm)], usize::from(time.hour >= 12))
    }

    /// Dakika adımı (ızgara, ↑/↓, tekerlek). 60'ı bölmeyen değer [`normalize_minute_step`] ile
    /// küçük bölene iner. Serbest giriş adımdan bağımsızdır (tam dakika girilebilir).
    pub fn with_minute_step(mut self, step: u8) -> Self {
        self.apply_minute_step(step);
        self
    }

    /// Bkz. [`with_minute_step`](Self::with_minute_step); açık ızgara yeni boyuta yayla morf olur.
    pub fn set_minute_step(&mut self, theme: &Theme, step: u8, now: Instant) -> bool {
        let changed = self.apply_minute_step(step);
        if changed {
            self.relayout(theme, now);
        }
        changed
    }

    fn apply_minute_step(&mut self, step: u8) -> bool {
        let step = normalize_minute_step(step);
        if step == self.step {
            return false;
        }
        self.step = step;
        // Saat ızgarası hiç boş değil (12 ya da 24 hücre); stil ondan (tema değişince ikisi birlikte).
        let style = self.hours[0].label.style().clone();
        self.minutes = minute_cells(&style, step);
        self.hovered = None;
        self.core.damage.mark();
        true
    }

    pub fn minute_step(&self) -> u8 {
        self.step
    }

    /// Locale (ya da `hour_cycle`) değişti: 12/24 saat, gün dilimi adları, saat ızgarası ve
    /// alan metni yeniden kurulur. Değişiklik yoksa iş yapmaz (`false`).
    pub fn set_locale(&mut self, theme: &Theme, locale: &nuxut_locale::Locale, now: Instant) -> bool {
        // Döngü aynı kalsa da kalıp değişebilir (tr → fi: "14:05" → "14.05").
        if *locale == self.locale {
            return false;
        }
        self.locale = locale.clone();
        let periods = day_periods_for(locale);
        self.twelve = periods.is_some();
        self.ampm = Self::ampm_button(theme, periods.as_ref(), self.time);
        self.periods = periods;
        self.hours = hour_cells(&text_style(theme, TextRole::Label), self.twelve);
        self.hovered = None;
        self.entry.clear();
        self.entry_pm = None;
        self.refresh_label();
        self.relayout(theme, now);
        true
    }

    fn relayout(&mut self, theme: &Theme, now: Instant) {
        if self.popover.is_open() {
            let size = self.content_size(theme);
            self.popover.set_content_size(theme, size, now);
            let area = self.popover.content_target(theme);
            self.layout_cells(theme, area);
        }
        self.core.damage.mark();
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    pub fn time(&self) -> Time {
        self.time
    }

    /// 12 saatlikte gün dilimi adları (`format_time` ile aynı metin için).
    pub fn day_periods(&self) -> Option<&[String; 2]> {
        self.periods.as_ref()
    }

    pub fn is_twelve_hour(&self) -> bool {
        self.twelve
    }

    /// Dışarıdan saat verir; süren serbest girişi bırakır.
    pub fn set_time(&mut self, time: Time, now: Instant) {
        self.entry.clear();
        self.entry_pm = None;
        self.apply_time(time, now);
        self.refresh_label();
    }

    fn apply_time(&mut self, time: Time, now: Instant) {
        let time = Time::new(time.hour, time.minute);
        if self.time != time {
            self.time = time;
            self.ampm.set_selected(usize::from(time.hour >= 12), now);
            self.core.damage.mark();
        }
    }

    fn refresh_label(&mut self) {
        let text = match self.entry_text() {
            Some(t) => t,
            None => field_text(&self.locale, self.time, self.periods.as_ref()),
        };
        if self.label.set_text(&text) {
            self.core.damage.mark();
        }
    }

    /// Serbest giriş sürüyor mu.
    pub fn is_entering(&self) -> bool {
        !self.entry.is_empty() || self.entry_pm.is_some()
    }

    /// Süren serbest girişi bırakır (odak kaybında çağıran çağırır); alan saati yeniden gösterir.
    pub fn cancel_entry(&mut self) -> bool {
        if !self.is_entering() {
            return false;
        }
        self.entry.clear();
        self.entry_pm = None;
        self.refresh_label();
        self.core.damage.mark();
        true
    }

    fn entry_pm(&self) -> bool {
        self.entry_pm.unwrap_or(self.time.hour >= 12)
    }

    /// Girişin alan metni: "1_:__", "14:3_", "9:__ PM"; giriş yoksa `None`.
    fn entry_text(&self) -> Option<String> {
        if !self.is_entering() {
            return None;
        }
        let s = parse_entry(&self.entry, self.twelve).unwrap_or_default();
        let hour = match (s.hour, s.first) {
            (Some(h), _) if self.twelve => h.to_string(),
            (Some(h), _) => format!("{h:02}"),
            (None, Some(p)) => format!("{p}_"),
            (None, None) if self.twelve => format!("{}", match self.time.hour % 12 {
                0 => 12,
                h => h,
            }),
            (None, None) => format!("{:02}", self.time.hour),
        };
        let minute = match (s.minute, s.tens, s.hour) {
            (Some(m), _, _) => format!("{m:02}"),
            (None, Some(t), _) => format!("{t}_"),
            (None, None, Some(_)) => "__".to_owned(),
            (None, None, None) if self.entry.is_empty() => format!("{:02}", self.time.minute),
            (None, None, None) => "__".to_owned(),
        };
        Some(match &self.periods {
            Some([am, pm]) => format!("{hour}:{minute} {}", if self.entry_pm() { pm } else { am }),
            None => format!("{hour}:{minute}"),
        })
    }

    /// Girişten saat: tek rakamlı bekleyen saat de saat sayılır (12 s'de "0" değil), eksik
    /// dakika basamakları 0 (alan metnindeki "_" gibi). Saat yoksa yalnız gün dilimi değişir.
    fn entry_time(&self) -> Option<Time> {
        let s = parse_entry(&self.entry, self.twelve)?;
        let hour = match (s.hour, s.first) {
            (Some(h), _) => h,
            (None, Some(p)) if !self.twelve || p >= 1 => p,
            (None, Some(_)) => return None,
            (None, None) => self.time.hour % if self.twelve { 12 } else { 24 },
        };
        let minute = s.minute.or(s.tens.map(|t| t * 10)).unwrap_or(if self.entry.is_empty() { self.time.minute } else { 0 });
        let hour = if self.twelve { hour % 12 + if self.entry_pm() { 12 } else { 0 } } else { hour };
        Some(Time::new(hour, minute))
    }

    fn commit_entry(&mut self, now: Instant) -> Response<TimeEvent> {
        let Some(t) = self.entry_time() else {
            return Response::none();
        };
        let had_digits = !self.entry.is_empty();
        self.entry.clear();
        self.entry_pm = None;
        let r = self.change(t, now);
        self.refresh_label();
        if had_digits {
            // Yazılan saat işi bitirir (dakika seçimi gibi); yalnız gün dilimi açık bırakır.
            self.popover.close(now);
        }
        r.with_repaint(true)
    }

    /// Serbest giriş tuşları; ilgisiz tuşta `None`.
    fn entry_key(&mut self, event: &KeyEvent, now: Instant) -> Option<Response<TimeEvent>> {
        let m = event.modifiers;
        if m.ctrl || m.alt || m.logo {
            return None;
        }
        match event.key {
            Key::Char(c) if c.is_ascii_digit() || (matches!(c, ':' | '.') && !self.entry.is_empty()) => {
                let d = if c.is_ascii_digit() { c as u8 - b'0' } else { ENTRY_SEP };
                let mut next = self.entry.clone();
                next.push(d);
                let Some(s) = parse_entry(&next, self.twelve) else {
                    // Geçersiz rakam: yutulur (başka bileşene gitmez), durum değişmez.
                    return Some(Response::none());
                };
                self.entry = next;
                if s.minute.is_some() {
                    return Some(self.commit_entry(now));
                }
                self.refresh_label();
                Some(Response::repaint())
            }
            Key::Char(c) if self.twelve && matches!(c.to_ascii_lowercase(), 'a' | 'p') => {
                self.entry_pm = Some(c.eq_ignore_ascii_case(&'p'));
                if self.entry.is_empty() {
                    // Yalnız gün dilimi: hemen uygulanır.
                    return Some(self.commit_entry(now));
                }
                self.refresh_label();
                Some(Response::repaint())
            }
            Key::Backspace if self.is_entering() => {
                if self.entry.pop().is_none() {
                    self.entry_pm = None;
                }
                self.refresh_label();
                self.core.damage.mark();
                Some(Response::repaint())
            }
            Key::Escape if self.is_entering() => {
                self.cancel_entry();
                Some(Response::repaint())
            }
            Key::Enter if self.is_entering() => Some(self.commit_entry(now)),
            _ => None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.popover.is_open()
    }

    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.label.set_style(text_style(theme, TextRole::Body).with_max_lines(1));
        self.icon.set_theme(theme, now);
        self.popover.set_theme(theme, now);
        self.ampm.set_theme(theme, now);
        let style = text_style(theme, TextRole::Label);
        for c in self.hours.iter_mut().chain(&mut self.minutes) {
            c.label.set_style(style.clone());
        }
    }

    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(12))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    fn cell_size(theme: &Theme) -> Size {
        Size::new(theme.space(11), theme.space(9))
    }

    /// Dakika ızgarasının sütun sayısı ve hücre boyu: 30'dan çok hücrede (1–2 dk adım) 10 sütun,
    /// daha alçak hücre; genişlik saat ızgarasıyla aynı.
    fn minute_grid(&self, theme: &Theme) -> (usize, Size) {
        let c = Self::cell_size(theme);
        if self.minutes.len() > 30 {
            let gap = theme.space(1);
            let w = (c.width * 6.0 + gap * 5.0 - gap * 9.0) / 10.0;
            (10, Size::new(w, theme.space(7)))
        } else {
            (6, c)
        }
    }

    /// Popover içeriği: saat ızgarası (+ AM/PM), dakika ızgarası; 6 (dakikada 6 ya da 10) sütun.
    fn content_size(&self, theme: &Theme) -> Size {
        let c = Self::cell_size(theme);
        let gap = theme.space(1);
        let w = c.width * 6.0 + gap * 5.0;
        let hour_rows = (self.hours.len() as f32 / 6.0).ceil();
        let (cols, mc) = self.minute_grid(theme);
        let minute_rows = (self.minutes.len() as f32 / cols as f32).ceil();
        let ampm = if self.twelve { theme.space(10) + theme.space(2) } else { 0.0 };
        let h = ampm + hour_rows * (c.height + gap) + theme.space(3) + minute_rows * (mc.height + gap) - gap;
        Size::new(w, h)
    }

    fn layout_cells(&mut self, theme: &Theme, area: Rect) {
        let c = Self::cell_size(theme);
        let gap = theme.space(1);
        let mut y = area.y;
        if self.twelve {
            self.ampm.set_rect(theme, Rect::new(area.x, y, area.width, theme.space(10)));
            y += theme.space(10) + theme.space(2);
        }
        let place = |cells: &mut Vec<Cell>, y0: f32, cols: usize, c: Size| -> f32 {
            for (i, cell) in cells.iter_mut().enumerate() {
                let (row, col) = (i / cols, i % cols);
                cell.rect = Rect::new(area.x + col as f32 * (c.width + gap), y0 + row as f32 * (c.height + gap), c.width, c.height);
            }
            y0 + (cells.len() as f32 / cols as f32).ceil() * (c.height + gap)
        };
        y = place(&mut self.hours, y, 6, c) + theme.space(3);
        let (cols, mc) = self.minute_grid(theme);
        place(&mut self.minutes, y, cols, mc);
    }

    fn open(&mut self, theme: &Theme, now: Instant) {
        let size = self.content_size(theme);
        self.popover.open(theme, anchor_below(self.core.rect, theme), size, self.bounds, now);
        let area = self.popover.content_target(theme);
        self.layout_cells(theme, area);
        self.core.damage.mark();
    }

    fn hour_value(&self, cell_value: u8) -> u8 {
        if self.twelve {
            cell_value % 12 + if self.time.hour >= 12 { 12 } else { 0 }
        } else {
            cell_value
        }
    }

    fn change(&mut self, t: Time, now: Instant) -> Response<TimeEvent> {
        let before = self.time;
        self.apply_time(t, now);
        self.refresh_label();
        if self.time != before { Response::event(TimeEvent::Changed(self.time)) } else { Response::repaint() }
    }

    /// Saat ızgarasının kapladığı bölge (açıkken; tekerlek orada saat değiştirir).
    fn hours_rect(&self) -> Option<Rect> {
        let first = self.hours.first()?.rect;
        Some(self.hours.iter().fold(first, |r, c| r.union(&c.rect)))
    }

    /// Tekerlek/dokunmatik yüzey: alanın üstünde ya da açık içerikte; yukarı = ileri. Saat
    /// ızgarasının üstünde saat, başka yerde dakika adımı.
    fn scroll(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Option<Response<TimeEvent>> {
        let PointerKind::Scroll { delta, steps } = event.kind else {
            return None;
        };
        let p = event.position;
        let in_content = self.popover.is_open() && self.popover.content_target(theme).contains(p);
        if !self.core.enabled || !(self.core.rect.contains(p) || in_content) {
            return None;
        }
        self.scroll_acc += if steps.y != 0.0 { steps.y } else { delta.y / theme.space(6) };
        let n = self.scroll_acc.trunc().clamp(-24.0, 24.0) as i32;
        self.scroll_acc -= self.scroll_acc.trunc();
        if n == 0 {
            return Some(Response::none());
        }
        self.cancel_entry();
        let over_hours = in_content && self.hours_rect().is_some_and(|r| r.contains(p));
        let mut t = self.time;
        for _ in 0..n.abs() {
            t = if over_hours { t.add_minutes(-60 * n.signum()) } else { step_time(t, self.step, -n.signum()) };
        }
        Some(self.change(t, now))
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<TimeEvent> {
        if let Some(r) = self.scroll(theme, event, now) {
            return r;
        }
        if matches!(event.kind, PointerKind::Press(_)) {
            self.cancel_entry();
        }
        if self.popover.is_open() {
            if self.popover.pointer(event, now).event.is_some() {
                return Response::repaint();
            }
            if self.twelve {
                let r = self.ampm.pointer(event, now);
                if let Some(i) = r.event {
                    let h = self.time.hour % 12 + if i == 1 { 12 } else { 0 };
                    return self.change(Time::new(h, self.time.minute), now);
                }
            }
            let p = event.position;
            let hit = self.hours.iter().position(|c| c.rect.contains(p)).map(|i| (true, i))
                .or_else(|| self.minutes.iter().position(|c| c.rect.contains(p)).map(|i| (false, i)));
            let mut out = Response::none();
            if hit != self.hovered && event.kind != PointerKind::Leave {
                self.hovered = hit;
                self.core.damage.mark();
                out.repaint = true;
            }
            if let (PointerKind::Release(Button::Left), Some((is_hour, i))) = (event.kind, hit) {
                if is_hour {
                    let h = self.hour_value(self.hours[i].value);
                    return self.change(Time::new(h, self.time.minute), now);
                }
                let m = self.minutes[i].value;
                let r = self.change(Time::new(self.time.hour, m), now);
                // Dakika seçimi işi bitirir.
                self.popover.close(now);
                return r.with_repaint(true);
            }
            return out;
        }
        let r = self.core.pointer(event, now);
        if r.event.is_some() {
            self.open(theme, now);
            return Response::repaint();
        }
        r.discard()
    }

    /// Rakamlar serbest giriş (bkz. tür belgesi); ↑/↓ dakika adımı, PageUp/PageDown bir saat;
    /// kapalıyken Enter/Space açar; açıkken Esc kapatır.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<TimeEvent> {
        if event.state == KeyState::Released || !self.core.enabled {
            return Response::none();
        }
        if let Some(r) = self.entry_key(event, now) {
            return r;
        }
        // Adım tuşları süren girişi bırakır: yoksa alan saat değişse de yarım girişi gösterirdi.
        if matches!(event.key, Key::Up | Key::Down | Key::PageUp | Key::PageDown) {
            self.cancel_entry();
        }
        match event.key {
            Key::Up => self.change(step_time(self.time, self.step, 1), now),
            Key::Down => self.change(step_time(self.time, self.step, -1), now),
            Key::PageUp => self.change(self.time.add_minutes(60), now),
            Key::PageDown => self.change(self.time.add_minutes(-60), now),
            _ if self.popover.is_open() => self.popover.key(event, now).discard(),
            Key::Enter | Key::Space => {
                self.open(theme, now);
                Response::repaint()
            }
            _ => Response::none(),
        }
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.popover.is_open() {
            if self.hours.iter().chain(&self.minutes).any(|c| c.rect.contains(p)) {
                return Some(CursorShape::Pointer);
            }
            if let Some(c) = (if self.twelve { self.ampm.cursor(p) } else { None }).or_else(|| self.popover.cursor(p)) {
                return Some(c);
            }
        }
        self.core.cursor(p)
    }

    pub fn input_rect(&self) -> Option<Rect> {
        self.popover.input_rect()
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let active = self.popover.is_open() || self.is_entering();
        draw_field(ui, &mut self.core, &mut self.label, &mut self.icon, active);
    }

    /// Açılır saat seçici: kaplama katmanında (bkz. [`DatePicker::draw_overlay`]).
    pub fn draw_overlay(&mut self, ui: &mut Ui) {
        let hour = self.time.hour;
        let minute = self.time.minute;
        let step = self.step;
        let twelve = self.twelve;
        let hovered = self.hovered;
        let (hours, minutes, ampm) = (&mut self.hours, &mut self.minutes, &mut self.ampm);
        self.popover.draw_overlay(ui, |ui, _| {
            let theme = ui.theme;
            let c = &theme.colors;
            if twelve {
                ampm.draw(ui);
            }
            let radii = Corners::uniform(theme.shape.radius_small);
            let sel_hour = if twelve { hour % 12 } else { hour };
            for (is_hour, cells) in [(true, &mut *hours), (false, &mut *minutes)] {
                for (i, cell) in cells.iter_mut().enumerate() {
                    let selected = if is_hour { cell.value == sel_hour } else { cell.value == minute - minute % step };
                    let (fill, text) = if selected { (c.primary, c.on_primary) } else { (c.surface_bright, c.on_surface) };
                    ui.painter.rect(cell.rect, &RectStyle::filled(ui.fade(fill), radii));
                    if hovered == Some((is_hour, i)) {
                        let layer = if selected { c.on_primary } else { c.on_surface };
                        let layer = crate::style::state_layer_under(theme, layer, STATE_HOVER, &[fill], &[text]);
                        ui.painter.rect(cell.rect, &RectStyle::filled(ui.fade(layer), radii));
                    }
                    let m = cell.label.measure_in(ui.painter);
                    let cc = cell.rect.center();
                    cell.label.draw(ui.painter, Point::new(cc.x - m.width * 0.5, cc.y - m.height * 0.5), ui.fade(text));
                }
            }
        });
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn date_formatting_and_pick() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let labels = CalendarLabels::for_locale("tr_TR");
        assert_eq!(format_date(Date::new(2026, 10, 4), &labels, "tr_TR"), "4 Ekim 2026");
        assert_eq!(format_date(Date::new(2026, 10, 4), &CalendarLabels::for_locale("en_US"), "en_US"), "October 4, 2026");
        let fd = |loc: &str| format_date(Date::new(2026, 10, 4), &CalendarLabels::for_locale(loc), loc);
        assert_eq!(fd("de_DE.UTF-8"), "4. Oktober 2026");
        assert_eq!(fd("ja_JP"), "2026年10月4日");
        assert_eq!(fd("en_GB"), "4 October 2026");
        assert_eq!(fd("bozuk"), "October 4, 2026");
        // Geçersiz tarih panik etmez (yedek ad tablosu).
        assert_eq!(format_date(Date { year: 2026, month: 13, day: 0 }, &labels, "tr_TR"), "0 Aralık 2026");
        let mut d = DatePicker::new(&theme, Date::new(2026, 10, 4), Date::new(2026, 10, 4), "tr_TR");
        d.set_rect(Rect::new(10.0, 10.0, 240.0, 48.0));
        d.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = d.key(&theme, &key(Key::Enter), t0);
        assert!(d.is_open());
        assert!(d.input_rect().unwrap().y >= 58.0, "alanın altında");
        let _ = d.key(&theme, &key(Key::Right), t0);
        let r = d.key(&theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(DateEvent::Changed(Date::new(2026, 10, 5))));
        assert!(!d.is_open());
    }

    #[test]
    fn date_picker_locale_change() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let loc = |n: &str| nuxut_locale::Locale::parse(n).unwrap();
        let day = Date::new(2026, 10, 4);
        let mut d = DatePicker::with_locale(&theme, day, day, &loc("tr_TR"));
        assert_eq!(d.text(), "4 Ekim 2026");
        assert_eq!(d.calendar().first_weekday(), 0, "tr: Pazartesi");
        // new(ad) ile with_locale aynı sonucu verir.
        assert_eq!(DatePicker::new(&theme, day, day, "tr_TR").text(), d.text());
        d.set_rect(Rect::new(10.0, 10.0, 240.0, 48.0));
        d.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = d.key(&theme, &key(Key::Enter), t0);
        assert!(d.is_open());
        assert!(!d.set_locale(&theme, &loc("tr_TR"), t0), "aynı locale iş yapmaz");
        // Açıkken locale değişir: popover açık kalır, seçim ve görünen ay korunur.
        assert!(d.set_locale(&theme, &loc("en_US"), t0));
        assert_eq!(d.text(), "October 4, 2026");
        assert_eq!(d.calendar().first_weekday(), 6, "en_US: Pazar");
        assert_eq!(d.date(), day);
        assert_eq!(d.calendar().selected(), Some(day));
        assert_eq!(d.calendar().month(), (2026, 10));
        assert!(d.is_open());
        assert!(d.input_rect().unwrap().y >= 58.0, "yeniden kurulan popover yine alanın altında");
        assert_eq!(d.locale(), &loc("en_US"));
        // Yeni locale ile seçim de yeni metni üretir.
        let _ = d.key(&theme, &key(Key::Right), t0);
        let r = d.key(&theme, &key(Key::Enter), t0);
        assert_eq!(r.event, Some(DateEvent::Changed(Date::new(2026, 10, 5))));
        assert_eq!(d.text(), "October 5, 2026");
        assert!(d.set_locale(&theme, &loc("de_DE"), t0));
        assert_eq!(d.text(), "5. Oktober 2026");
        assert_eq!(d.calendar().first_weekday(), 0);
    }

    #[test]
    fn time_locale_format_and_grid() {
        let theme = Theme::default();
        let t0 = Instant::now();
        assert!(uses_12_hour("en_US.UTF-8") && !uses_12_hour("tr_TR") && !uses_12_hour(""));
        assert_eq!(format_time(Time::new(14, 5), None), "14:05");
        let en = day_periods("en_US.UTF-8");
        assert_eq!(format_time(Time::new(0, 30), en.as_ref()), "12:30 AM");
        assert_eq!(day_periods("tr_TR"), None);
        assert_eq!(Time::new(23, 58).add_minutes(5), Time::new(0, 3));
        let mut t = TimePicker::new(&theme, Time::new(9, 0), "tr_TR");
        t.set_rect(Rect::new(10.0, 10.0, 200.0, 48.0));
        t.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        assert_eq!(t.key(&theme, &key(Key::Up), t0).event, Some(TimeEvent::Changed(Time::new(9, 5))));
        let _ = t.key(&theme, &key(Key::Enter), t0);
        assert!(t.is_open());
        let h14 = t.hours[14].rect.center();
        let _ = t.pointer(&theme, &press(h14.x, h14.y), t0);
        assert_eq!(t.pointer(&theme, &release(h14.x, h14.y), t0).event, Some(TimeEvent::Changed(Time::new(14, 5))));
        assert_eq!(t.cursor(h14), Some(CursorShape::Pointer));
        let m30 = t.minutes[6].rect.center();
        let _ = t.pointer(&theme, &press(m30.x, m30.y), t0);
        assert_eq!(t.pointer(&theme, &release(m30.x, m30.y), t0).event, Some(TimeEvent::Changed(Time::new(14, 30))));
        assert!(!t.is_open(), "dakika seçimi kapatır");
    }

    #[test]
    fn twelve_hour_ampm() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = TimePicker::new(&theme, Time::new(9, 0), "en_US");
        t.set_rect(Rect::new(10.0, 10.0, 200.0, 48.0));
        t.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
        let _ = t.key(&theme, &key(Key::Enter), t0);
        let pm = t.ampm.rect();
        let p = Point::new(pm.right() - 10.0, pm.center().y);
        let _ = t.pointer(&theme, &press(p.x, p.y), t0);
        assert_eq!(t.pointer(&theme, &release(p.x, p.y), t0).event, Some(TimeEvent::Changed(Time::new(21, 0))));
    }

    fn loc(name: &str, twelve: Option<bool>) -> nuxut_locale::Locale {
        nuxut_locale::Locale::parse(name).unwrap().with_12_hour(twelve)
    }

    fn picker(theme: &Theme, time: Time, locale: &nuxut_locale::Locale) -> TimePicker {
        let mut t = TimePicker::with_locale(theme, time, locale);
        t.set_rect(Rect::new(10.0, 10.0, 200.0, 48.0));
        t.set_bounds(Rect::new(0.0, 0.0, 800.0, 900.0));
        t
    }

    fn typed_all(t: &mut TimePicker, theme: &Theme, s: &str, now: Instant) -> Option<TimeEvent> {
        let mut last = None;
        for c in s.chars() {
            if let Some(e) = t.key(theme, &typed(c), now).event {
                last = Some(e);
            }
        }
        last
    }

    #[test]
    fn hour_cycle_override_from_locale() {
        let theme = Theme::default();
        let t0 = Instant::now();
        // h12 tr'de: ÖÖ/ÖS, 12 saat ızgarası; h23 en_US'de: 24 saat, gün dilimi yok.
        let tr12 = picker(&theme, Time::new(14, 5), &loc("tr_TR", Some(true)));
        assert!(tr12.is_twelve_hour());
        assert_eq!(tr12.hours.len(), 12);
        assert_eq!(tr12.label.text(), format_time(Time::new(14, 5), tr12.day_periods()));
        let en23 = picker(&theme, Time::new(14, 5), &loc("en_US", Some(false)));
        assert!(!en23.is_twelve_hour() && en23.day_periods().is_none());
        assert_eq!(en23.label.text(), "14:05");
        assert_eq!(en23.hours.len(), 24);
        // Geçersiz kılma yok: locale verisi.
        assert!(picker(&theme, Time::new(1, 0), &loc("en_US", None)).is_twelve_hour());
        assert!(!picker(&theme, Time::new(1, 0), &loc("tr_TR", None)).is_twelve_hour());
        // %p'si boş locale + h12: AM/PM yedeği.
        assert_eq!(day_periods_for(&loc("de_DE", Some(true))), Some(["AM".to_owned(), "PM".to_owned()]));
        assert_eq!(day_periods_for(&loc("en_US", Some(false))), None);
        // Ad tabanlı API değişmedi (geçersiz kılma görmez).
        assert_eq!(day_periods("tr_TR"), None);
        assert!(!uses_12_hour("bozuk") && TimePicker::new(&theme, Time::new(9, 0), "bozuk").day_periods().is_none());

        // locale_changed: set_locale yeniden kurar; açık popover yeni boyuta morf olur.
        let mut t = picker(&theme, Time::new(21, 30), &loc("tr_TR", None));
        let _ = t.key(&theme, &key(Key::Enter), t0);
        let h24 = t.content_size(&theme).height;
        assert!(t.set_locale(&theme, &loc("tr_TR", Some(true)), t0));
        assert!(!t.set_locale(&theme, &loc("tr_TR", Some(true)), t0), "aynı döngü iş yapmaz");
        assert!(t.is_twelve_hour() && t.hours.len() == 12);
        assert_eq!(t.time(), Time::new(21, 30), "saat korunur");
        assert!(t.label.text().starts_with("9:30"), "{}", t.label.text());
        assert_ne!(t.content_size(&theme).height, h24);
        assert!(t.hours.iter().all(|c| c.rect.width > 0.0), "açıkken yeniden yerleşir");
        assert!(t.set_locale(&theme, &loc("tr_TR", Some(false)), t0));
        assert_eq!(t.label.text(), "21:30");
        // Alan metni locale kalıbından (Locale::format_hm, bar saatiyle aynı): döngü aynı kalsa
        // da locale değişimi metni yeniler (fi "21.30").
        assert!(t.set_locale(&theme, &loc("fi_FI", None), t0), "24 → 24 saat locale değişimi");
        assert_eq!(t.label.text(), loc("fi_FI", None).format_hm(21, 30).unwrap());
        assert_eq!(t.label.text(), "21.30");
        let ko = picker(&theme, Time::new(13, 5), &loc("ko_KR", Some(true)));
        assert_eq!(ko.label.text(), loc("ko_KR", Some(true)).format_hm(13, 5).unwrap());
    }

    #[test]
    fn minute_step_one_and_five() {
        let theme = Theme::default();
        let t0 = Instant::now();
        assert_eq!(normalize_minute_step(0), 1);
        assert_eq!(normalize_minute_step(7), 6);
        assert_eq!(normalize_minute_step(59), 30);
        assert_eq!(step_time(Time::new(14, 7), 5, 1), Time::new(14, 10));
        assert_eq!(step_time(Time::new(14, 7), 5, -1), Time::new(14, 5));
        assert_eq!(step_time(Time::new(14, 10), 5, -1), Time::new(14, 5));
        assert_eq!(step_time(Time::new(23, 59), 1, 1), Time::new(0, 0));
        assert_eq!(step_time(Time::new(0, 0), 15, -1), Time::new(23, 45));

        // Varsayılan 5 dk: ızgara 12 hücre, ok tuşu 5 dk.
        let mut t5 = picker(&theme, Time::new(9, 0), &loc("tr_TR", None));
        assert_eq!(t5.minute_step(), MINUTE_STEP);
        assert_eq!(t5.minutes.len(), 12);
        assert_eq!(t5.key(&theme, &key(Key::Down), t0).event, Some(TimeEvent::Changed(Time::new(8, 55))));
        assert_eq!(t5.key(&theme, &key(Key::PageUp), t0).event, Some(TimeEvent::Changed(Time::new(9, 55))));

        // 1 dk: 60 hücre, 10 sütun × 6 satır; ok ve tekerlek 1 dk.
        let mut t1 = picker(&theme, Time::new(9, 0), &loc("tr_TR", None)).with_minute_step(1);
        assert_eq!(t1.minutes.len(), 60);
        assert_eq!(t1.key(&theme, &key(Key::Up), t0).event, Some(TimeEvent::Changed(Time::new(9, 1))));
        let f = t1.rect().center();
        let wheel = |dy: f32| PointerEvent::new(f, PointerKind::Scroll { delta: Point::new(0.0, dy * 15.0), steps: Point::new(0.0, dy) });
        assert_eq!(t1.pointer(&theme, &wheel(-1.0), t0).event, Some(TimeEvent::Changed(Time::new(9, 2))), "yukarı = ileri");
        assert_eq!(t1.pointer(&theme, &wheel(3.0), t0).event, Some(TimeEvent::Changed(Time::new(8, 59))));
        // Dokunmatik yüzey: küçük deltalar birikir.
        let pad = |dy: f32| PointerEvent::new(f, PointerKind::Scroll { delta: Point::new(0.0, dy), steps: Point::ZERO });
        assert_eq!(t1.pointer(&theme, &pad(-theme.space(6) * 0.6), t0).event, None);
        assert_eq!(t1.pointer(&theme, &pad(-theme.space(6) * 0.6), t0).event, Some(TimeEvent::Changed(Time::new(9, 0))));
        let _ = t1.key(&theme, &key(Key::Enter), t0);
        let area = t1.popover.content_target(&theme);
        let last = t1.minutes[59].rect;
        assert!(last.right() <= area.right() + 0.5 && last.bottom() <= area.bottom() + 0.5, "{last:?} ⊄ {area:?}");
        assert_eq!(t1.minutes[10].rect.y, t1.minutes[19].rect.y, "10 sütun");
        assert!(t1.minutes[9].rect.width > 0.0 && t1.minutes[9].rect.right() <= area.right() + 0.5);
        let m37 = t1.minutes[37].rect.center();
        let _ = t1.pointer(&theme, &press(m37.x, m37.y), t0);
        assert_eq!(t1.pointer(&theme, &release(m37.x, m37.y), t0).event, Some(TimeEvent::Changed(Time::new(9, 37))));

        // Açıkken adım değişimi: ızgara yeniden kurulur ve yerleşir.
        let mut t = picker(&theme, Time::new(9, 0), &loc("tr_TR", None));
        let _ = t.key(&theme, &key(Key::Enter), t0);
        assert!(t.set_minute_step(&theme, 1, t0));
        assert!(!t.set_minute_step(&theme, 1, t0));
        assert_eq!(t.minutes.len(), 60);
        assert!(t.minutes.iter().all(|c| c.rect.width > 0.0));
        assert!(t.set_minute_step(&theme, 15, t0));
        assert_eq!(t.minutes.iter().map(|c| c.value).collect::<Vec<_>>(), vec![0, 15, 30, 45]);
    }

    #[test]
    fn free_entry_parses_and_rejects() {
        // Saf ayrıştırıcı.
        assert_eq!(parse_entry(&[7], false).unwrap().hour, Some(7), "3–9 saat tek rakam");
        assert_eq!(parse_entry(&[2], false).unwrap().first, Some(2));
        // 24 saat yapamayan ikinci rakam dakika onlar basamağı: "24" → 02:4_; "26" yok.
        let s = parse_entry(&[2, 4], false).unwrap();
        assert_eq!((s.hour, s.tens), (Some(2), Some(4)));
        assert!(parse_entry(&[2, 6], false).is_none(), "02:6_ yok");
        // 12 saat: "13" → 1:3_, "0" + ayırıcı yok (saat 0), ayırıcı ile tek basamak saat.
        let s = parse_entry(&[1, 3], true).unwrap();
        assert_eq!((s.hour, s.tens), (Some(1), Some(3)));
        assert!(parse_entry(&[0, ENTRY_SEP], true).is_none());
        let s = parse_entry(&[1, ENTRY_SEP, 3, 0], true).unwrap();
        assert_eq!((s.hour, s.minute), (Some(1), Some(30)), "1:30");
        assert!(parse_entry(&[ENTRY_SEP], false).is_none(), "başta ayırıcı yok");
        assert!(parse_entry(&[7, ENTRY_SEP, ENTRY_SEP], false).is_none(), "tek ayırıcı");
        assert!(parse_entry(&[7, 3, ENTRY_SEP], false).is_none(), "dakikadan sonra ayırıcı yok");
        assert_eq!(parse_entry(&[7, ENTRY_SEP, 4, 5], false).unwrap().minute, Some(45));
        assert!(parse_entry(&[1, 4, 6], false).is_none(), "dakika onlar 0–5");
        assert!(parse_entry(&[0, 0], true).is_none(), "12 saatlikte 00 yok");
        assert_eq!(parse_entry(&[1, 4, 3, 5], false).unwrap().minute, Some(35));
        assert!(parse_entry(&[1, 4, 3, 5, 1], false).is_none());

        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = picker(&theme, Time::new(9, 0), &loc("tr_TR", None)).with_minute_step(15);
        assert_eq!(t.key(&theme, &typed('1'), t0).event, None);
        assert!(t.is_entering());
        assert_eq!(t.label.text(), "1_:__");
        // '9' → 19 geçerli; ardından '7' dakika onlar basamağı olamaz → reddedilir.
        assert_eq!(typed_all(&mut t, &theme, "97", t0), None);
        assert_eq!(t.label.text(), "19:__");
        assert_eq!(t.key(&theme, &typed('4'), t0).event, None);
        assert_eq!(t.label.text(), "19:4_");
        // Adımdan bağımsız tam dakika; dördüncü rakam uygular.
        assert_eq!(t.key(&theme, &typed('3'), t0).event, Some(TimeEvent::Changed(Time::new(19, 43))));
        assert!(!t.is_entering());
        assert_eq!(t.label.text(), "19:43");

        // Enter eksik dakikayı 0'la tamamlar; Backspace siler; Esc vazgeçer.
        assert_eq!(typed_all(&mut t, &theme, "83", t0), None);
        assert_eq!(t.label.text(), "08:3_");
        let _ = t.key(&theme, &key(Key::Backspace), t0);
        assert_eq!(t.label.text(), "08:__");
        assert_eq!(t.key(&theme, &key(Key::Enter), t0).event, Some(TimeEvent::Changed(Time::new(8, 0))));
        let _ = typed_all(&mut t, &theme, "12", t0);
        let _ = t.key(&theme, &key(Key::Escape), t0);
        assert!(!t.is_entering());
        assert_eq!(t.time(), Time::new(8, 0));
        assert_eq!(t.label.text(), "08:00");
        // Ctrl+rakam giriş değildir.
        let mut ctrl = typed('5');
        ctrl.modifiers.ctrl = true;
        let _ = t.key(&theme, &ctrl, t0);
        assert!(!t.is_entering());
        // cancel_entry girişi bırakır (odak kaybı).
        let _ = t.key(&theme, &typed('2'), t0);
        assert!(t.cancel_entry() && !t.cancel_entry());
        assert_eq!(t.label.text(), "08:00");
        // Giriş sürerken ↑: giriş bırakılır, alan yeni saati gösterir (yarım girişi değil).
        let _ = t.key(&theme, &typed('2'), t0);
        assert_eq!(t.key(&theme, &key(Key::Up), t0).event, Some(TimeEvent::Changed(Time::new(8, 15))));
        assert!(!t.is_entering());
        assert_eq!(t.label.text(), "08:15");

        // 12 saat: gün dilimi a/p, 12 s geçersiz rakam reddi.
        let mut e = picker(&theme, Time::new(9, 0), &loc("en_US", None));
        assert_eq!(typed_all(&mut e, &theme, "1", t0), None);
        assert_eq!(e.label.text(), "1_:__ AM");
        assert_eq!(e.key(&theme, &typed('p'), t0).event, None);
        assert_eq!(e.label.text(), "1_:__ PM");
        assert_eq!(e.key(&theme, &key(Key::Enter), t0).event, Some(TimeEvent::Changed(Time::new(13, 0))));
        // "13" 12 saatlikte 1:3_; "1:30" ayırıcıyla aynı saat (eskiden ':' yok sayılıp 10 oluyordu).
        assert_eq!(typed_all(&mut e, &theme, "13", t0), None);
        assert_eq!(e.label.text(), "1:3_ PM");
        assert_eq!(e.key(&theme, &key(Key::Escape), t0).event, None);
        assert_eq!(typed_all(&mut e, &theme, "1:30", t0), Some(TimeEvent::Changed(Time::new(13, 30))));
        assert_eq!(e.key(&theme, &typed(':'), t0).event, None);
        assert!(!e.is_entering(), "girişsiz ayırıcı giriş başlatmaz");
        assert_eq!(typed_all(&mut e, &theme, "1205", t0), Some(TimeEvent::Changed(Time::new(12, 5))), "12 PM öğle");
        assert_eq!(e.key(&theme, &typed('a'), t0).event, Some(TimeEvent::Changed(Time::new(0, 5))), "yalnız gün dilimi hemen");
        // Açıkken yazılan saat popover'ı kapatır.
        let _ = e.key(&theme, &key(Key::Enter), t0);
        assert!(e.is_open());
        assert_eq!(typed_all(&mut e, &theme, "745", t0), Some(TimeEvent::Changed(Time::new(7, 45))));
        assert!(!e.is_open());
    }
}
