//! Haftalık zaman ızgarası ([`WeekGrid`]): gün başlıkları, tüm gün şeridi, saat oluğu, dikey
//! kaydırılan 24 saatlik gövde, çakışan etkinlikler yan yana sütunlarda, "şimdi" çizgisi.
//! Hafta değişiminde içerik o yönde yayla kayar (ay ızgarasıyla aynı dil).
//!
//! Zamanlar **yerel saniye**dir (Unix saniyesi + çağıranın UTC ofseti); bileşen saat dilimi
//! bilmez. Tüm gün etkinliklerinde `start`/`end` yerel gece yarısıdır (`end` hariç).
//! Gün sayısı 1–7 (gün / 3 gün / hafta görünümü).
//!
//! Etkinlik kutusu tonaldir: takvim rengi zemine karışır, solda tam renkli şerit, metin tema
//! token'ıyla — her takvim renginde okunur.

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect};
use nuxut_locale::Locale;
use nuxut_motion::Animated;
use nuxut_render::RectStyle;
use nuxut_theme::{STATE_HOVER, STATE_PRESSED, Theme};

use crate::button::{IconButton, IconButtonVariant};
use crate::calendar::{CalendarLabels, Date, first_weekday_for_locale};
use crate::icon::Icon;
use crate::icon_names::IconName;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::label::Label;
use crate::pickers::{Time, day_periods, format_time};
use crate::scroll::ScrollView;
use crate::style::{clip_text, divider, pill, state_layer_under, tinted, tonal_container};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Tüm gün şeridinde görünen en fazla şerit (fazlası gün başına "+N").
pub const MAX_ALL_DAY_LANES: usize = 3;
/// Etkinlik başlığının en uzun hâli (bayt).
pub const MAX_WEEK_TITLE: usize = 256;
/// Boş alana tıklamada dakika yuvarlaması.
pub const SLOT_MINUTES: u16 = 30;
/// Gösterilen en fazla etkinlik (sınırsız veri yerleşimi boğmasın).
pub const MAX_WEEK_EVENTS: usize = 2000;
const DAY: i64 = 86_400;
/// Süresiz / çok kısa etkinliğin çizimdeki en kısa süresi (dk).
const MIN_BLOCK_MINUTES: f32 = 15.0;
/// Etkinlik kutusunda takvim renginin zemine karışım oranı.
const EVENT_TINT: f32 = 0.28;

/// Etkinlik kutusunun zemini: `surface_bright` üstünde takvim rengi. Yüksek karşıtlıkta
/// karışım üstündeki metin (`on_surface`, `on_surface_variant`) ≥ 4.5 kalacak kadarla sınırlı
/// (`style::tinted`; takvim rengi rastgele olabilir).
pub(crate) fn event_fill(theme: &Theme, color: Color) -> Color {
    let c = &theme.colors;
    tinted(theme, c.surface_bright, color, EVENT_TINT, &[c.on_surface, c.on_surface_variant])
}

/// Izgaraya verilen etkinlik.
#[derive(Clone, PartialEq)]
pub struct WeekEvent {
    /// Çağıranın kimliği (tıklamada geri gelir).
    pub key: u64,
    pub title: String,
    /// Yerel saniye (Unix + UTC ofseti).
    pub start: i64,
    /// Yerel saniye, hariç.
    pub end: i64,
    pub all_day: bool,
    /// Takvim rengi; `None` = `primary`.
    pub color: Option<Color>,
}

/// Başlık kullanıcı verisidir: `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for WeekEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeekEvent")
            .field("key", &self.key)
            .field("title", &crate::redact::Len(&self.title))
            .field("start", &self.start)
            .field("end", &self.end)
            .field("all_day", &self.all_day)
            .field("color", &self.color.is_some())
            .finish()
    }
}

impl WeekEvent {
    pub fn timed(key: u64, title: impl Into<String>, start: i64, end: i64) -> Self {
        Self { key, title: title.into(), start, end, all_day: false, color: None }
    }

    /// `first`..=`last` günlerini kaplayan tüm gün etkinliği.
    pub fn all_day(key: u64, title: impl Into<String>, first: Date, last: Date) -> Self {
        let start = first.to_days() * DAY;
        let end = (last.to_days() + 1) * DAY;
        Self { key, title: title.into(), start, end: end.max(start + DAY), all_day: true, color: None }
    }

    pub fn with_color(mut self, color: Option<Color>) -> Self {
        self.color = color;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekGridEvent {
    /// Etkinliğe tıklandı.
    EventActivated(u64),
    /// Boş zamana tıklandı (yeni etkinlik): gün + [`SLOT_MINUTES`]'a yuvarlanmış dakika.
    SlotActivated { date: Date, minute: u16 },
    /// Gün başlığına tıklandı (gün görünümüne geçiş için).
    DayActivated(Date),
    /// Gösterilen aralık değişti: yeni etkinlikleri yükle ([`WeekGrid::range`]).
    RangeChanged { start: Date, days: u8 },
}

/// Gün sütunundaki zamanlı parça.
#[derive(Debug, Clone)]
struct Block {
    key: u64,
    day: usize,
    start_min: f32,
    end_min: f32,
    col: usize,
    cols: usize,
    color: Option<Color>,
    title: Label,
    time: Label,
}

#[derive(Debug, Clone)]
struct Bar {
    key: u64,
    first: usize,
    last: usize,
    lane: usize,
    color: Option<Color>,
    title: Label,
}

/// Bir aralığın çizime hazır hâli.
#[derive(Debug, Clone)]
struct WeekView {
    start: Date,
    heads: Vec<(Label, Label)>,
    blocks: Vec<Block>,
    bars: Vec<Bar>,
    /// Gün başına görünmeyen tüm gün etkinliği ("+N").
    overflow: Vec<(usize, Label)>,
    /// Görünen şerit sayısı.
    lanes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Block(usize),
    Bar(usize),
    Head(usize),
    Slot(usize, u16),
}

#[derive(Debug, Clone)]
pub struct WeekGrid {
    rect: Rect,
    days: u8,
    first_weekday: u8,
    labels: CalendarLabels,
    /// Başlık sırası ("Ekim 2026" / "2026年10月") için.
    locale: Locale,
    /// 12 saatlikte gün dilimi adları (locale verisinden); `None` = 24 saat.
    periods: Option<[String; 2]>,
    today: Date,
    /// Yerel saniye ("şimdi" çizgisi).
    now_local: Option<i64>,
    events: Vec<WeekEvent>,
    view: WeekView,
    old: Option<(WeekView, f32)>,
    progress: Animated<f32>,
    scroll: ScrollView,
    hour_labels: Vec<Label>,
    title: Label,
    prev: IconButton,
    next: IconButton,
    hovered: Option<Hit>,
    pressed: Option<Hit>,
    /// Dokunmatik yüzeyde yatay kaydırma birikimi (hafta geçişi).
    hscroll: f32,
    damage: DamageState,
    /// İlk yerleşimde gövde bu dakikaya kaydırılır.
    initial_minute: Option<u16>,
}

impl WeekGrid {
    /// `locale`: hafta başı, gün adları, 12/24 saat ([`locale_from_env`](crate::locale_from_env)).
    pub fn new(theme: &Theme, today: Date, locale: &str) -> Self {
        let first_weekday = first_weekday_for_locale(locale);
        let labels = CalendarLabels::for_locale(locale);
        let periods = day_periods(locale);
        let start = week_start(today, first_weekday);
        let mut g = Self {
            rect: Rect::ZERO,
            days: 7,
            first_weekday,
            labels,
            locale: Locale::parse(locale).unwrap_or_default(),
            periods,
            today,
            now_local: None,
            events: Vec::new(),
            view: WeekView { start, heads: Vec::new(), blocks: Vec::new(), bars: Vec::new(), overflow: Vec::new(), lanes: 0 },
            old: None,
            progress: Animated::new(1.0, theme.motion.spatial),
            scroll: ScrollView::new(theme),
            hour_labels: Vec::new(),
            title: Label::new("", text_style(theme, TextRole::Title).with_max_lines(1)),
            prev: IconButton::new(theme, Icon::named(IconName::ChevronLeft), IconButtonVariant::Standard),
            next: IconButton::new(theme, Icon::named(IconName::ChevronRight), IconButtonVariant::Standard),
            hovered: None,
            pressed: None,
            hscroll: 0.0,
            damage: DamageState::new(),
            initial_minute: Some(8 * 60),
        };
        g.relabel_hours(theme);
        g.rebuild(theme);
        g
    }

    /// Gün sayısı (1–7). 7'de aralık hafta başına hizalanır, değilse başlangıç korunur.
    pub fn with_days(mut self, theme: &Theme, days: u8) -> Self {
        self.set_days(theme, days);
        self
    }

    pub fn set_days(&mut self, theme: &Theme, days: u8) {
        let days = days.clamp(1, 7);
        if days == self.days {
            return;
        }
        self.days = days;
        let anchor = if (self.view.start..self.view.start.add_days(7)).contains(&self.today) { self.today } else { self.view.start };
        self.view.start = if days == 7 { week_start(anchor, self.first_weekday) } else { anchor };
        self.old = None;
        self.rebuild(theme);
    }

    pub fn days(&self) -> u8 {
        self.days
    }

    /// Gösterilen aralık: (ilk gün, gün sayısı).
    pub fn range(&self) -> (Date, u8) {
        (self.view.start, self.days)
    }

    /// Aralığın yerel saniye sınırları `[from, to)` (sorgu için; UTC'ye çevirmek çağıranın).
    pub fn local_range(&self) -> (i64, i64) {
        let from = self.view.start.to_days() * DAY;
        (from, from + i64::from(self.days) * DAY)
    }

    pub fn set_labels(&mut self, theme: &Theme, labels: CalendarLabels) {
        self.labels = labels;
        self.rebuild(theme);
    }

    pub fn set_today(&mut self, theme: &Theme, today: Date) {
        if self.today != today {
            self.today = today;
            self.rebuild(theme);
        }
    }

    /// "Şimdi" çizgisi (yerel saniye); `None` gizler. Dakikada bir çağrılması yeterli.
    pub fn set_now(&mut self, now_local: Option<i64>) {
        if self.now_local.map(|s| s / 60) != now_local.map(|s| s / 60) {
            self.now_local = now_local;
            self.damage.mark();
        }
    }

    pub fn set_events(&mut self, theme: &Theme, mut events: Vec<WeekEvent>) {
        events.truncate(MAX_WEEK_EVENTS);
        if self.events != events {
            self.events = events;
            self.rebuild(theme);
        }
    }

    /// Gövdeyi günün bir dakikasına kaydırır (ör. ilk etkinlik ya da şimdi).
    pub fn scroll_to_minute(&mut self, theme: &Theme, minute: u16, now: Instant) {
        let y = (Self::hour_height(theme) * f32::from(minute.min(24 * 60)) / 60.0 - theme.space(4)).max(0.0);
        self.scroll.scroll_to(y, now);
        self.initial_minute = None;
        self.damage.mark();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.progress.set_motion(theme.motion.spatial, now);
        self.scroll.set_theme(theme, now);
        self.prev.set_theme(theme, now);
        self.next.set_theme(theme, now);
        self.relabel_hours(theme);
        self.title.set_style(text_style(theme, TextRole::Title).with_max_lines(1));
        self.rebuild(theme);
        let r = self.rect;
        self.rect = Rect::ZERO;
        self.set_rect(theme, r, now);
    }

    fn relabel_hours(&mut self, theme: &Theme) {
        let style = text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1);
        self.hour_labels = (1..24).map(|h| Label::new(format_time(Time::new(h, 0), self.periods.as_ref()), style.clone())).collect();
        self.damage.mark();
    }

    fn rebuild(&mut self, theme: &Theme) {
        self.view = self.build_view(theme, self.view.start);
        self.title.set_text(&self.title_text());
        self.hovered = None;
        self.pressed = None;
        self.damage.mark();
    }

    fn build_view(&self, theme: &Theme, start: Date) -> WeekView {
        let days = usize::from(self.days);
        let caption = text_style(theme, TextRole::Caption).with_max_lines(1);
        let number = text_style(theme, TextRole::Title).with_tabular_numbers().with_max_lines(1);
        let heads = (0..days)
            .map(|i| {
                let d = start.add_days(i as i64);
                (Label::new(self.labels.weekdays[usize::from(d.weekday())].clone(), caption.clone()), Label::new(d.day.to_string(), number.clone()))
            })
            .collect();
        let from = start.to_days() * DAY;
        let to = from + days as i64 * DAY;
        let label_style = text_style(theme, TextRole::Caption).with_max_lines(2);
        let time_style = text_style(theme, TextRole::Caption).with_tabular_numbers().with_max_lines(1);
        let bar_style = text_style(theme, TextRole::Caption).with_max_lines(1);

        // Tüm gün şeritleri: uzun olan önce, ilk boş şeride.
        let mut spans: Vec<(usize, usize, &WeekEvent)> = self
            .events
            .iter()
            .filter(|e| e.all_day && e.end > from && e.start < to)
            .map(|e| {
                let a = ((e.start.max(from) - from) / DAY) as usize;
                let b = (((e.end.min(to) - 1).max(e.start.max(from)) - from) / DAY) as usize;
                (a, b.min(days - 1), e)
            })
            .collect();
        spans.sort_by_key(|(a, b, e)| (*a, std::cmp::Reverse(*b), e.start));
        let mut lane_end: Vec<usize> = Vec::new();
        let mut placed: Vec<(usize, usize, usize, &WeekEvent)> = Vec::new();
        for (a, b, e) in spans {
            let lane = match lane_end.iter().position(|&end| end < a) {
                Some(l) => l,
                None => {
                    lane_end.push(0);
                    lane_end.len() - 1
                }
            };
            lane_end[lane] = b;
            placed.push((a, b, lane, e));
        }
        let total_lanes = lane_end.len();
        // Taşmada son görünen şerit "+N"e ayrılır.
        let shown = if total_lanes > MAX_ALL_DAY_LANES { MAX_ALL_DAY_LANES - 1 } else { total_lanes };
        let mut hidden = vec![0usize; days];
        let mut bars = Vec::new();
        for (a, b, lane, e) in placed {
            if lane < shown {
                bars.push(Bar { key: e.key, first: a, last: b, lane, color: e.color, title: Label::new(clip_text(&e.title, MAX_WEEK_TITLE), bar_style.clone()) });
            } else {
                for h in &mut hidden[a..=b] {
                    *h += 1;
                }
            }
        }
        let overflow: Vec<(usize, Label)> =
            hidden.iter().enumerate().filter(|(_, n)| **n > 0).map(|(d, n)| (d, Label::new(format!("+{n}"), bar_style.clone()))).collect();
        let lanes = shown + usize::from(!overflow.is_empty());

        // Zamanlı parçalar: gün sınırında bölünür, çakışma kümelerinde sütunlara dağıtılır.
        let mut segs: Vec<(usize, f32, f32, &WeekEvent)> = Vec::new();
        for e in self.events.iter().filter(|e| !e.all_day) {
            let end = e.end.max(e.start);
            for d in 0..days {
                let ds = from + d as i64 * DAY;
                let (s, t) = (e.start.max(ds), end.min(ds + DAY));
                let zero_len_here = e.start == end && (ds..ds + DAY).contains(&e.start);
                if t > s || zero_len_here {
                    let sm = (s - ds) as f32 / 60.0;
                    let em = ((t - ds) as f32 / 60.0).max(sm + MIN_BLOCK_MINUTES).min(24.0 * 60.0);
                    segs.push((d, sm.min(24.0 * 60.0 - MIN_BLOCK_MINUTES), em, e));
                }
            }
        }
        segs.sort_by(|a, b| (a.0, a.1).partial_cmp(&(b.0, b.1)).unwrap_or(std::cmp::Ordering::Equal).then(b.2.total_cmp(&a.2)));
        let mut blocks: Vec<Block> = Vec::with_capacity(segs.len());
        let mut cluster_start = 0;
        let mut cluster_end = f32::MIN;
        let mut cluster_day = usize::MAX;
        let mut col_end: Vec<f32> = Vec::new();
        let close = |blocks: &mut Vec<Block>, from: usize, cols: usize| {
            for b in &mut blocks[from..] {
                b.cols = cols.max(1);
            }
        };
        for (d, sm, em, e) in segs {
            if d != cluster_day || sm >= cluster_end {
                close(&mut blocks, cluster_start, col_end.len());
                cluster_start = blocks.len();
                cluster_day = d;
                cluster_end = f32::MIN;
                col_end.clear();
            }
            let col = match col_end.iter().position(|&end| end <= sm) {
                Some(c) => c,
                None => {
                    col_end.push(0.0);
                    col_end.len() - 1
                }
            };
            col_end[col] = em;
            cluster_end = cluster_end.max(em);
            let t0 = Time::new((sm / 60.0) as u8, (sm % 60.0) as u8);
            blocks.push(Block {
                key: e.key,
                day: d,
                start_min: sm,
                end_min: em,
                col,
                cols: 1,
                color: e.color,
                title: Label::new(clip_text(&e.title, MAX_WEEK_TITLE), label_style.clone()),
                time: Label::new(format_time(t0, self.periods.as_ref()), time_style.clone()),
            });
        }
        close(&mut blocks, cluster_start, col_end.len());
        WeekView { start, heads, blocks, bars, overflow, lanes }
    }

    pub fn preferred_height(theme: &Theme) -> f32 {
        theme.space(12) + theme.space(16) + Self::hour_height(theme) * 10.0
    }

    fn hour_height(theme: &Theme) -> f32 {
        theme.space(12)
    }

    fn nav_height(theme: &Theme) -> f32 {
        theme.space(12)
    }

    fn head_height(theme: &Theme) -> f32 {
        theme.space(14)
    }

    fn lane_height(theme: &Theme) -> f32 {
        theme.space(6)
    }

    fn gutter(theme: &Theme) -> f32 {
        theme.space(14)
    }

    fn all_day_height(&self, theme: &Theme) -> f32 {
        if self.view.lanes == 0 { 0.0 } else { self.view.lanes as f32 * Self::lane_height(theme) + theme.space(2) }
    }

    fn col_width(&self, theme: &Theme) -> f32 {
        ((self.rect.width - Self::gutter(theme)) / f32::from(self.days)).max(1.0)
    }

    fn columns_x(&self, theme: &Theme) -> f32 {
        self.rect.x + Self::gutter(theme)
    }

    fn head_rect(&self, theme: &Theme) -> Rect {
        Rect::new(self.columns_x(theme), self.rect.y + Self::nav_height(theme), self.rect.width - Self::gutter(theme), Self::head_height(theme))
    }

    fn all_day_rect(&self, theme: &Theme) -> Rect {
        let h = self.head_rect(theme);
        Rect::new(h.x, h.bottom(), h.width, self.all_day_height(theme))
    }

    fn body_rect(&self, theme: &Theme) -> Rect {
        let top = self.all_day_rect(theme).bottom();
        Rect::new(self.rect.x, top, self.rect.width, (self.rect.bottom() - top).max(0.0))
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect, now: Instant) {
        let s = theme.space(10);
        let y = rect.y + (Self::nav_height(theme) - s) * 0.5;
        self.next.set_rect(Rect::new(rect.right() - s, y, s, s));
        self.prev.set_rect(Rect::new(rect.right() - s * 2.0 - theme.space(1), y, s, s));
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
        self.sync_scroll(theme, now);
    }

    fn sync_scroll(&mut self, theme: &Theme, now: Instant) {
        let body = self.body_rect(theme);
        self.scroll.set_geometry(body, Self::hour_height(theme) * 24.0, now);
        if let Some(m) = self.initial_minute
            && body.height > 0.0
        {
            let y = (Self::hour_height(theme) * f32::from(m) / 60.0 - theme.space(4)).clamp(0.0, self.scroll.max_offset());
            self.scroll.scroll_to(y, now);
            self.initial_minute = None;
        }
    }

    /// Aralığı `n` sayfa (gün sayısı kadar) kaydırır.
    fn step(&mut self, theme: &Theme, n: i64, now: Instant) -> Option<WeekGridEvent> {
        let start = self.view.start.add_days(n * i64::from(self.days));
        self.show(theme, start, now)
    }

    /// Bir güne git (7 günlükte haftasına).
    pub fn show_date(&mut self, theme: &Theme, date: Date, now: Instant) -> Option<WeekGridEvent> {
        let start = if self.days == 7 { week_start(date, self.first_weekday) } else { date };
        self.show(theme, start, now)
    }

    fn show(&mut self, theme: &Theme, start: Date, now: Instant) -> Option<WeekGridEvent> {
        if start == self.view.start {
            return None;
        }
        let dir = if start > self.view.start { 1.0 } else { -1.0 };
        let new = self.build_view(theme, start);
        let old = std::mem::replace(&mut self.view, new);
        self.old = Some((old, dir));
        self.title.set_text(&self.title_text());
        self.progress.jump_to(0.0);
        self.progress.set_target(1.0, now);
        self.hovered = None;
        self.pressed = None;
        self.damage.mark();
        self.sync_scroll(theme, now);
        Some(WeekGridEvent::RangeChanged { start, days: self.days })
    }

    fn block_rect(&self, theme: &Theme, b: &Block, origin_y: f32) -> Rect {
        let cw = self.col_width(theme);
        let hh = Self::hour_height(theme);
        let inset = theme.space(1) * 0.5;
        let x0 = self.columns_x(theme) + cw * b.day as f32;
        let w = (cw - theme.space(1)) / b.cols as f32;
        let y0 = origin_y + hh * b.start_min / 60.0;
        let y1 = origin_y + hh * b.end_min / 60.0;
        Rect::new(x0 + w * b.col as f32 + inset, y0 + inset, (w - inset * 2.0).max(1.0), (y1 - y0 - inset * 2.0).max(theme.space(1)))
    }

    fn bar_rect(&self, theme: &Theme, b: &Bar) -> Rect {
        let a = self.all_day_rect(theme);
        let cw = self.col_width(theme);
        let lh = Self::lane_height(theme);
        let inset = theme.space(1) * 0.5;
        Rect::new(a.x + cw * b.first as f32 + inset, a.y + lh * b.lane as f32 + inset, cw * (b.last - b.first + 1) as f32 - inset * 2.0, lh - inset * 2.0)
    }

    fn hit(&self, theme: &Theme, p: Point, now: Instant) -> Option<Hit> {
        if !self.rect.contains(p) {
            return None;
        }
        let head = self.head_rect(theme);
        let cw = self.col_width(theme);
        let col = |x: f32| (((x - head.x) / cw).floor().max(0.0) as usize).min(usize::from(self.days) - 1);
        if head.contains(p) {
            return Some(Hit::Head(col(p.x)));
        }
        let all = self.all_day_rect(theme);
        if all.contains(p) {
            return self.view.bars.iter().position(|b| self.bar_rect(theme, b).contains(p)).map(Hit::Bar);
        }
        let body = self.body_rect(theme);
        if !body.contains(p) || p.x < head.x || self.scroll.cursor(p).is_some() {
            return None;
        }
        let oy = self.scroll.content_origin(now).y;
        // Üstteki (sonra çizilen) kutu önce.
        if let Some(i) = self.view.blocks.iter().rposition(|b| self.block_rect(theme, b, oy).contains(p)) {
            return Some(Hit::Block(i));
        }
        let minute = ((p.y - oy) / Self::hour_height(theme) * 60.0).clamp(0.0, 24.0 * 60.0 - 1.0) as u16;
        Some(Hit::Slot(col(p.x), minute - minute % SLOT_MINUTES))
    }

    pub fn pointer(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<WeekGridEvent> {
        let mut out = Response::none();
        for prev in [true, false] {
            let btn = if prev { &mut self.prev } else { &mut self.next };
            let r = btn.pointer(event, now);
            out.repaint |= r.repaint;
            if r.event.is_some() {
                let e = self.step(theme, if prev { -1 } else { 1 }, now);
                return Response { repaint: true, event: e };
            }
        }
        if let PointerKind::Scroll { delta, steps } = event.kind
            && self.rect.contains(event.position)
            && (steps.x.abs() > steps.y.abs() || delta.x.abs() > delta.y.abs())
        {
            // Yatay kaydırma hafta değiştirir; dikey gövdeye gider.
            let dir = if steps.x != 0.0 {
                steps.x.signum()
            } else {
                self.hscroll += delta.x;
                if self.hscroll.abs() < theme.space(24) {
                    return out;
                }
                std::mem::take(&mut self.hscroll).signum()
            };
            let e = self.step(theme, dir as i64, now);
            return Response { repaint: true, event: e };
        }
        let r = self.scroll.pointer(theme, event, now);
        if r.repaint || r.event.is_some() {
            self.damage.mark();
            out.repaint = true;
        }
        if r.event.is_some() {
            return out;
        }
        for ev in self.scroll.forward(event) {
            let r = self.pointer_content(theme, &ev, now);
            if r.event.is_some() {
                return r.with_repaint(out.repaint);
            }
            out.repaint |= r.repaint;
        }
        out
    }

    fn pointer_content(&mut self, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<WeekGridEvent> {
        let mut out = Response::none();
        let hit = if event.kind == PointerKind::Leave { None } else { self.hit(theme, event.position, now) };
        let hover = hit.filter(|h| !matches!(h, Hit::Slot(..)));
        if hover != self.hovered {
            self.hovered = hover;
            self.damage.mark();
            out.repaint = true;
        }
        match event.kind {
            PointerKind::Press(Button::Left) => {
                self.pressed = hit;
                self.damage.mark();
                out.repaint = true;
            }
            PointerKind::Release(Button::Left) => {
                let pressed = self.pressed.take();
                self.damage.mark();
                out.repaint = true;
                let same = match (pressed, hit) {
                    // Boş zamanda basıp başka yarım saatte bırakmak da aynı güne yeni etkinliktir.
                    (Some(Hit::Slot(a, _)), Some(Hit::Slot(b, _))) => a == b,
                    (a, b) => a.is_some() && a == b,
                };
                if same && let Some(h) = hit {
                    let e = match h {
                        Hit::Block(i) => WeekGridEvent::EventActivated(self.view.blocks[i].key),
                        Hit::Bar(i) => WeekGridEvent::EventActivated(self.view.bars[i].key),
                        Hit::Head(d) => WeekGridEvent::DayActivated(self.view.start.add_days(d as i64)),
                        Hit::Slot(d, minute) => WeekGridEvent::SlotActivated { date: self.view.start.add_days(d as i64), minute },
                    };
                    return Response::event(e);
                }
            }
            _ => {}
        }
        out
    }

    /// PageUp/PageDown ya da Sol/Sağ önceki/sonraki aralık, Home bugün; Yukarı/Aşağı
    /// gövdeyi bir saat kaydırır.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<WeekGridEvent> {
        if event.state == KeyState::Released {
            return Response::none();
        }
        let e = match event.key {
            Key::PageUp | Key::Left => self.step(theme, -1, now),
            Key::PageDown | Key::Right => self.step(theme, 1, now),
            Key::Home => self.show_date(theme, self.today, now),
            Key::Up | Key::Down => {
                let dy = if event.key == Key::Up { -1.0 } else { 1.0 } * Self::hour_height(theme);
                let target = self.scroll.offset(now) + dy;
                self.scroll.scroll_to(target, now);
                self.damage.mark();
                return Response::repaint();
            }
            _ => return Response::none(),
        };
        Response { repaint: true, event: e }
    }

    pub fn cursor(&self, theme: &Theme, p: Point, now: Instant) -> Option<CursorShape> {
        if let Some(c) = self.prev.cursor(p).or_else(|| self.next.cursor(p)).or_else(|| self.scroll.cursor(p)) {
            return Some(c);
        }
        if !self.rect.contains(p) {
            return None;
        }
        Some(if self.hit(theme, p, now).is_some() { CursorShape::Pointer } else { CursorShape::Default })
    }

    fn title_text(&self) -> String {
        let s = self.view.start;
        let e = s.add_days(i64::from(self.days) - 1);
        let m = |d: Date| &self.labels.months[usize::from(d.month.clamp(1, 12) - 1)];
        // Ay–yıl sırası locale'den ("Ekim 2026", "2026年10月"); aralık aynı yıldaysa yıl bir kez.
        let my = |month: &str, year: i32| self.locale.compose_month_year(month, year);
        if s.month == e.month && s.year == e.year {
            my(m(s), s.year)
        } else if s.year == e.year {
            my(&format!("{} – {}", m(s), m(e)), e.year)
        } else {
            format!("{} – {}", my(m(s), s.year), my(m(e), e.year))
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.rect;
        // Başlık (ay/yıl) + gezinme.
        let ts = self.title.measure_in(ui.painter);
        self.title.draw(ui.painter, Point::new(rect.x + theme.space(1), rect.y + (Self::nav_height(theme) - ts.height) * 0.5), ui.fade(c.on_surface));
        self.prev.draw(ui);
        self.next.draw(ui);

        let p = self.progress.value(now);
        let slide = self.col_width(theme) * 2.0;
        let cols = Rect::new(self.columns_x(theme), rect.y + Self::nav_height(theme), rect.width - Self::gutter(theme), rect.height - Self::nav_height(theme));
        let body = self.body_rect(theme);
        let line = divider(theme);

        // Saat oluğu ve yatay saat çizgileri (kaymaz).
        let hh = Self::hour_height(theme);
        let hours = &mut self.hour_labels;
        let gutter = Self::gutter(theme);
        let grid_x = self.rect.x + gutter;
        self.scroll.draw(ui, |ui, origin, _| {
            for (i, l) in hours.iter_mut().enumerate() {
                let y = origin.y + hh * (i + 1) as f32;
                let m = l.measure_in(ui.painter);
                l.draw(ui.painter, Point::new(grid_x - theme.space(2) - m.width, y - m.height * 0.5), ui.fade(c.on_surface_variant));
                ui.painter.rect(Rect::new(grid_x, y, rect.right() - grid_x, crate::style::HAIRLINE), &RectStyle::filled(ui.fade(line), Corners::ZERO));
            }
        });

        let origin_y = self.scroll.content_origin(now).y;
        ui.painter.push_clip(cols);
        if let Some((mut old, dir)) = self.old.take() {
            let a = (1.0 - p * 1.6).clamp(0.0, 1.0);
            let dx = -dir * slide * p;
            ui.with_alpha(a, |ui| self.draw_view(ui, &mut old, dx, origin_y, body, None, None));
            self.old = Some((old, dir));
        }
        let (alpha, dx) = match &self.old {
            Some((_, dir)) => (((p - 0.25) / 0.75).clamp(0.0, 1.0), *dir * slide * (1.0 - p)),
            None => (1.0, 0.0),
        };
        let mut view = std::mem::replace(&mut self.view, WeekView { start: Date::new(1970, 1, 1), heads: Vec::new(), blocks: Vec::new(), bars: Vec::new(), overflow: Vec::new(), lanes: 0 });
        let (hovered, pressed) = (self.hovered, self.pressed);
        ui.with_alpha(alpha, |ui| self.draw_view(ui, &mut view, dx, origin_y, body, hovered, pressed));
        self.view = view;
        ui.painter.pop_clip();

        let animating = !self.progress.is_settled(now);
        if animating {
            ui.request_frame();
        } else {
            self.old = None;
        }
        self.damage.report(ui, rect, animating);
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_view(&self, ui: &mut Ui, view: &mut WeekView, dx: f32, origin_y: f32, body: Rect, hovered: Option<Hit>, pressed: Option<Hit>) {
        let theme = ui.theme;
        let c = &theme.colors;
        let cw = self.col_width(theme);
        let head = self.head_rect(theme);
        let x0 = head.x + dx;
        let layer_of = |h: Hit| {
            if pressed == Some(h) {
                STATE_PRESSED
            } else if hovered == Some(h) {
                STATE_HOVER
            } else {
                0.0
            }
        };

        // Gün başlıkları: gün adı + numara; bugün numarası `primary` dolgulu daire.
        for (i, (name, num)) in view.heads.iter_mut().enumerate() {
            let d = view.start.add_days(i as i64);
            let cx = x0 + cw * (i as f32 + 0.5);
            let nm = name.measure_in(ui.painter);
            let ns = num.measure_in(ui.painter);
            let top = head.y + (head.height - nm.height - ns.height - theme.space(1)) * 0.5;
            let is_today = d == self.today;
            name.draw(ui.painter, Point::new(cx - nm.width * 0.5, top), ui.fade(if is_today { c.primary } else { c.on_surface_variant }));
            let ny = top + nm.height + theme.space(1);
            let disc = ns.height.max(ns.width) + theme.space(2);
            let dr = Rect::new(cx - disc * 0.5, ny + ns.height * 0.5 - disc * 0.5, disc, disc);
            let mut text = c.on_surface;
            if is_today {
                ui.painter.rect(dr, &RectStyle::filled(ui.fade(c.primary), pill(dr)));
                text = c.on_primary;
            }
            let l = layer_of(Hit::Head(i));
            if l > 0.0 {
                let layer = if is_today { c.on_primary } else { c.on_surface };
                ui.painter.rect(dr, &RectStyle::filled(ui.fade(state_layer_under(theme, layer, l, &[if is_today { c.primary } else { Color::TRANSPARENT }], &[text])), pill(dr)));
            }
            num.draw(ui.painter, Point::new(cx - ns.width * 0.5, ny), ui.fade(text));
        }

        // Tüm gün şeritleri.
        let pad = theme.space(2);
        for (i, bar) in view.bars.iter_mut().enumerate() {
            let mut r = self.bar_rect(theme, bar);
            r.x += dx;
            let color = bar.color.unwrap_or(c.primary);
            let radii = Corners::uniform(theme.shape.radius_small);
            let fill = event_fill(theme, color);
            ui.painter.rect(r, &RectStyle::filled(ui.fade(fill), radii));
            let l = layer_of(Hit::Bar(i));
            if l > 0.0 {
                ui.painter.rect(r, &RectStyle::filled(ui.fade(state_layer_under(theme, c.on_surface, l, &[fill], &[c.on_surface])), radii));
            }
            let stripe = Rect::new(r.x, r.y, theme.space(1), r.height);
            ui.painter.rect(stripe, &RectStyle::filled(ui.fade(color), Corners::new(radii.top_left, 0.0, 0.0, radii.bottom_left)));
            bar.title.set_max_width(Some((r.width - pad * 2.0).max(0.0)));
            let m = bar.title.measure_in(ui.painter);
            bar.title.draw(ui.painter, Point::new(r.x + pad, r.center().y - m.height * 0.5), ui.fade(c.on_surface));
        }
        let all = self.all_day_rect(theme);
        for (d, l) in &mut view.overflow {
            let lh = Self::lane_height(theme);
            let y = all.y + lh * (view.lanes - 1) as f32;
            let m = l.measure_in(ui.painter);
            l.draw(ui.painter, Point::new(x0 + cw * *d as f32 + pad, y + (lh - m.height) * 0.5), ui.fade(c.on_surface_variant));
        }

        // Gövde: gün ayırıcıları, bugün sütunu tonu, etkinlikler, şimdi çizgisi.
        ui.painter.push_clip(Rect::new(head.x, body.y, head.width, body.height));
        let line = divider(theme);
        for i in 0..view.heads.len() {
            let x = x0 + cw * i as f32;
            if view.start.add_days(i as i64) == self.today {
                let tint = tonal_container(theme).multiply_alpha(0.35);
                ui.painter.rect(Rect::new(x, body.y, cw, body.height), &RectStyle::filled(ui.fade(tint), Corners::ZERO));
            }
            ui.painter.rect(Rect::new(x, body.y, crate::style::HAIRLINE, body.height), &RectStyle::filled(ui.fade(line), Corners::ZERO));
        }
        let small = theme.space(1) * 0.5;
        for (i, b) in view.blocks.iter_mut().enumerate() {
            let mut r = self.block_rect(theme, b, origin_y);
            r.x += dx;
            if r.bottom() < body.y || r.y > body.bottom() {
                continue;
            }
            let color = b.color.unwrap_or(c.primary);
            let radii = Corners::uniform(theme.shape.radius_small.min(r.height * 0.5));
            // Arkadaki kutudan ayrışsın: zemin rengiyle ince boşluk.
            ui.painter.rect(r.expand(small), &RectStyle::filled(ui.fade(c.surface), Corners::uniform(radii.top_left + small)));
            let fill = event_fill(theme, color);
            ui.painter.rect(r, &RectStyle::filled(ui.fade(fill), radii));
            let l = layer_of(Hit::Block(i));
            if l > 0.0 {
                ui.painter.rect(r, &RectStyle::filled(ui.fade(state_layer_under(theme, c.on_surface, l, &[fill], &[c.on_surface, c.on_surface_variant])), radii));
            }
            let stripe = Rect::new(r.x, r.y, theme.space(1), r.height);
            ui.painter.rect(stripe, &RectStyle::filled(ui.fade(color), Corners::new(radii.top_left, 0.0, 0.0, radii.bottom_left)));
            let tx = r.x + theme.space(1) + pad;
            let tw = (r.right() - pad - tx).max(0.0);
            ui.painter.push_clip(r);
            b.title.set_max_width(Some(tw));
            b.time.set_max_width(Some(tw));
            let mm = b.time.measure_in(ui.painter);
            // Kısa kutuda (ör. 30 dk) başlık tek satır ve dikeyde ortalı; kenara yapışmaz.
            let short = r.height < mm.height * 2.0 + small * 4.0;
            let lines = if short { 1 } else { 2 };
            if b.title.style().max_lines != Some(lines) {
                let style = b.title.style().clone().with_max_lines(lines);
                b.title.set_style(style);
            }
            let tm = b.title.measure_in(ui.painter);
            let top = if short { r.y + (r.height - tm.height) * 0.5 } else { r.y + small * 2.0 };
            b.title.draw(ui.painter, Point::new(tx, top), ui.fade(c.on_surface));
            if top + tm.height + mm.height <= r.bottom() {
                b.time.draw(ui.painter, Point::new(tx, top + tm.height), ui.fade(c.on_surface_variant));
            }
            ui.painter.pop_clip();
        }
        if let Some(now_s) = self.now_local {
            let day = now_s.div_euclid(DAY) - view.start.to_days();
            if (0..view.heads.len() as i64).contains(&day) {
                let minute = now_s.rem_euclid(DAY) as f32 / 60.0;
                let y = origin_y + Self::hour_height(theme) * minute / 60.0;
                let x = x0 + cw * day as f32;
                let thick = crate::style::FOCUS_RING;
                ui.painter.rect(Rect::new(x, y - thick * 0.5, cw, thick), &RectStyle::filled(ui.fade(c.primary), Corners::ZERO));
                let dot = theme.space(2);
                let dr = Rect::new(x - dot * 0.5, y - dot * 0.5, dot, dot);
                ui.painter.rect(dr, &RectStyle::filled(ui.fade(c.primary), pill(dr)));
            }
        }
        ui.painter.pop_clip();
    }
}

/// `date`'i içeren haftanın ilk günü.
pub fn week_start(date: Date, first_weekday: u8) -> Date {
    let lead = (i64::from(date.weekday()) - i64::from(first_weekday)).rem_euclid(7);
    date.add_days(-lead)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    const H: i64 = 3600;

    fn at(d: Date, h: i64, m: i64) -> i64 {
        d.to_days() * DAY + h * H + m * 60
    }

    fn grid() -> (WeekGrid, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = WeekGrid::new(&theme, Date::new(2026, 10, 7), "tr_TR");
        g.set_rect(&theme, Rect::new(0.0, 0.0, 800.0, 700.0), t0);
        (g, theme, t0)
    }

    #[test]
    fn range_title_follows_locale_order() {
        let theme = Theme::default();
        let title = |loc: &str, d: Date| WeekGrid::new(&theme, d, loc).title_text();
        assert_eq!(title("tr_TR", Date::new(2026, 10, 7)), "Ekim 2026");
        assert_eq!(title("tr_TR", Date::new(2026, 10, 29)), "Ekim – Kasım 2026");
        // ja haftası Pazar başlar: 29 Kasım – 5 Aralık.
        assert_eq!(title("ja_JP", Date::new(2026, 11, 30)), "2026年11月 – 12月");
        assert_eq!(title("tr_TR", Date::new(2026, 12, 31)), "Aralık 2026 – Ocak 2027");
        assert_eq!(title("ja_JP", Date::new(2026, 12, 31)), "2026年12月 – 2027年1月");
    }

    #[test]
    fn week_aligns_to_locale_start() {
        let (g, _, _) = grid();
        assert_eq!(g.range(), (Date::new(2026, 10, 5), 7), "Pazartesi başlangıç");
        let theme = Theme::default();
        let us = WeekGrid::new(&theme, Date::new(2026, 10, 7), "en_US");
        assert_eq!(us.range().0, Date::new(2026, 10, 4), "Pazar başlangıç");
        assert_eq!(week_start(Date::new(2026, 10, 5), 0), Date::new(2026, 10, 5));
    }

    #[test]
    fn overlapping_events_share_columns() {
        let (mut g, theme, _) = grid();
        let d = Date::new(2026, 10, 6);
        g.set_events(
            &theme,
            vec![
                WeekEvent::timed(1, "a", at(d, 9, 0), at(d, 11, 0)),
                WeekEvent::timed(2, "b", at(d, 10, 0), at(d, 12, 0)),
                WeekEvent::timed(3, "c", at(d, 11, 0), at(d, 11, 30)),
                WeekEvent::timed(4, "d", at(d, 14, 0), at(d, 15, 0)),
            ],
        );
        let get = |k| g.view.blocks.iter().find(|b| b.key == k).map(|b| (b.day, b.col, b.cols)).unwrap();
        assert_eq!(get(1), (1, 0, 2));
        assert_eq!(get(2), (1, 1, 2));
        assert_eq!(get(3), (1, 0, 2), "11:00'de a bitti: sütun 0 boşaldı");
        assert_eq!(get(4), (1, 0, 1), "ayrı küme");
    }

    #[test]
    fn midnight_crossing_splits_and_zero_length_is_visible() {
        let (mut g, theme, _) = grid();
        let d = Date::new(2026, 10, 7);
        g.set_events(&theme, vec![WeekEvent::timed(1, "gece", at(d, 22, 0), at(d.add_days(1), 2, 0)), WeekEvent::timed(2, "an", at(d, 8, 0), at(d, 8, 0))]);
        let parts: Vec<_> = g.view.blocks.iter().filter(|b| b.key == 1).map(|b| (b.day, b.start_min, b.end_min)).collect();
        assert_eq!(parts, vec![(2, 22.0 * 60.0, 24.0 * 60.0), (3, 0.0, 120.0)]);
        let z = g.view.blocks.iter().find(|b| b.key == 2).unwrap();
        assert_eq!(z.end_min - z.start_min, MIN_BLOCK_MINUTES);
    }

    #[test]
    fn all_day_lanes_and_overflow() {
        let (mut g, theme, _) = grid();
        let mon = Date::new(2026, 10, 5);
        let mut ev = vec![WeekEvent::all_day(1, "uzun", mon, mon.add_days(3)), WeekEvent::all_day(2, "kısa", mon.add_days(4), mon.add_days(4))];
        g.set_events(&theme, ev.clone());
        assert_eq!(g.view.lanes, 1, "çakışmayan şeritler aynı satırda");
        // Pazartesi'ye 3 tane daha: 4 şerit → 2 görünür + "+N" satırı.
        for k in 10..13 {
            ev.push(WeekEvent::all_day(k, "x", mon, mon));
        }
        g.set_events(&theme, ev);
        assert_eq!(g.view.lanes, MAX_ALL_DAY_LANES);
        assert_eq!(g.view.overflow.len(), 1);
        assert_eq!(g.view.overflow[0].0, 0);
        assert_eq!(g.view.overflow[0].1.text(), "+2");
    }

    #[test]
    fn click_block_slot_and_head() {
        let (mut g, theme, t0) = grid();
        let d = Date::new(2026, 10, 6);
        g.set_events(&theme, vec![WeekEvent::timed(7, "a", at(d, 9, 0), at(d, 10, 0))]);
        g.set_rect(&theme, Rect::new(0.0, 0.0, 800.0, 700.0), t0);
        g.scroll_to_minute(&theme, 0, t0);
        let t = t0 + std::time::Duration::from_secs(5);
        let oy = g.scroll.content_origin(t).y;
        let r = g.block_rect(&theme, &g.view.blocks[0].clone(), oy).center();
        let click = |g: &mut WeekGrid, p: Point| {
            let _ = g.pointer(&theme, &press(p.x, p.y), t);
            g.pointer(&theme, &release(p.x, p.y), t).event
        };
        assert_eq!(click(&mut g, r), Some(WeekGridEvent::EventActivated(7)));
        // Aynı gün 05:10 → 05:00 yuvası.
        let y = oy + WeekGrid::hour_height(&theme) * (5.0 + 10.0 / 60.0);
        assert_eq!(click(&mut g, Point::new(r.x, y)), Some(WeekGridEvent::SlotActivated { date: d, minute: 5 * 60 }));
        let h = g.head_rect(&theme);
        assert_eq!(click(&mut g, Point::new(h.x + 5.0, h.center().y)), Some(WeekGridEvent::DayActivated(Date::new(2026, 10, 5))));
    }

    #[test]
    fn navigation_reports_range() {
        let (mut g, theme, t0) = grid();
        let r = g.key(&theme, &key(Key::PageDown), t0);
        assert_eq!(r.event, Some(WeekGridEvent::RangeChanged { start: Date::new(2026, 10, 12), days: 7 }));
        let r = g.key(&theme, &key(Key::Home), t0);
        assert_eq!(r.event, Some(WeekGridEvent::RangeChanged { start: Date::new(2026, 10, 5), days: 7 }));
        g.set_days(&theme, 3);
        assert_eq!(g.range(), (Date::new(2026, 10, 7), 3), "bugünden başlar");
        assert_eq!(g.local_range().1 - g.local_range().0, 3 * DAY);
    }

    #[test]
    fn debug_is_redacted() {
        crate::redact::assert_redacted(&WeekEvent::timed(1, crate::redact::SECRET, 0, 60));
        let (mut g, theme, _) = grid();
        g.set_events(&theme, vec![WeekEvent::timed(1, crate::redact::SECRET, 0, 60)]);
        crate::redact::assert_redacted(&g);
    }
}
