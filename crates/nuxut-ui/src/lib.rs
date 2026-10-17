//! Nuxut bileşenleri. Wayland bilmez; yalnızca `Painter`, `TextSystem` ve `Theme` kullanır.
//! Hiçbir bileşen sabit renk/yarıçap/süre içermez — her şey `Theme`'den okunur.
//!
//! Ortak kalıp (her bileşen):
//! - `new(theme, …)` → model; `preferred_size(…)` ölçüm; `set_rect(rect)` yerleşim.
//! - `pointer(&PointerEvent, now)` / `key(&KeyEvent, now)` → [`Response`]: `repaint` (kare
//!   iste) ve varsa bileşenin olayı (`Some(())` tıklama, `Some(değer)` …).
//! - `draw(&mut Ui)`: çizer, canlanıyorsa [`Ui::request_frame`], hasar takibi açıksa değişen
//!   bölgesini (eski ∪ yeni) bildirir.
//! - `set_theme(theme, now)`: token değişimi (yaylar devam eder).

#![forbid(unsafe_code)]

pub use nuxut_access as access;
#[cfg(test)]
mod access_tests;
mod agenda;
mod app_glyph;
mod avatar;
mod badge;
mod bar;
#[cfg(feature = "brand")]
mod brand;
#[cfg(feature = "brand")]
mod mascot;
mod button;
mod curve_editor;
#[cfg(test)]
mod cursor_tests;
#[cfg(test)]
mod overlay_tests;
#[cfg(test)]
mod reduced_motion_tests;
#[cfg(test)]
mod contrast_tests;
mod calendar;
mod chip;
mod color_picker;
mod confirm;
mod device;
mod dialog;
mod disclosure;
mod empty_state;
mod file_grid;
mod focus;
mod glyph_grid;
mod font_picker;
mod grow;
mod icon;
mod icon_names;
pub mod input;
mod interaction;
mod label;
mod menu;
mod layout;
mod nav;
mod notice;
mod notification;
mod list;
mod osd;
mod page_stack;
mod path_bar;
mod pickers;
mod popover;
mod pressable;
mod progress;
mod sparkline;
mod redact;
mod reorder;
mod search;
mod results;
mod scroll;
mod secret;
mod selection;
mod segmented;
mod select;
mod slider;
mod snackbar;
mod swatch;
mod text_input;
pub mod style;
mod theme_preview;
mod tile;
mod tint;
mod toggle;
mod tooltip;
mod typography;
mod ui;
mod week;

pub use access::{AccessAction, AccessNode, AccessRequest, AccessTree};
pub use app_glyph::{AppGlyphParseError, AppGlyphs, MAX_APP_CLASS, NERD_SYMBOLS_FAMILY, NerdFont, app_glyph, find_nerd_font, is_nerd_font_family};
pub use agenda::{AgendaItem, CARD_DESCRIPTION_LINES, EventCard, EventRow, MAX_AGENDA_DESCRIPTION, MAX_AGENDA_TEXT};
pub use avatar::{Avatar, AvatarSize, initials};
#[cfg(feature = "brand")]
pub use brand::{BRAND_MASK_CACHE, BRAND_SVG, BRAND_SYMBOLIC_SVG, BrandMark, BrandStyle, DEFAULT_BRAND_SIZE, MAX_BRAND_MASK_SIDE, MaskPart, brand_mask, brand_shade};
#[cfg(feature = "brand")]
pub use mascot::{
    DEFAULT_MASCOT_SIZE, MASCOT_CACHE, MASCOT_MAX_SIZE, MASCOT_MIN_SIZE, Mascot, MascotColors, MascotStyle, Mood, RasterKey, mascot_colors, mascot_image,
    paint_svg,
};
pub use badge::{BADGE_MAX, Badge, MAX_PILL_TEXT, Pill, Separator, pill_colors};
pub use button::{Button, ButtonVariant, IconButton, IconButtonVariant};
pub use calendar::{
    CalendarEvent, CalendarGrid, CalendarLabels, Date, MAX_EVENT_DOTS, days_in_month, first_weekday_for_locale, is_leap,
    first_weekday_of, locale_from_env, narrow_weekday,
};
pub use chip::{Chip, ChipEvent, ChipGroup, ChipKind, MAX_CHIP_LABEL};
pub use color_picker::{ColorEvent, ColorPicker, MAX_RECENT};
pub use confirm::{ConfirmButton, ConfirmEvent};
pub use curve_editor::{BEZIER_Y_MAX, BEZIER_Y_MIN, CurveEditor, CurveEvent, CurveValue, SPRING_DAMPING, SPRING_RESPONSE};
pub use device::{DeviceEvent, DeviceKind, DeviceRow, DeviceState};
pub use dialog::{Dialog, DialogAction, DialogActionKind, DialogEvent, MAX_DIALOG_ACTIONS};
pub use disclosure::{Disclosure, DisclosureEvent, MAX_DISCLOSURE_TEXT};
pub use empty_state::{EMPTY_BODY_LINES, EMPTY_MASCOT_SIZE, EMPTY_TITLE_LINES, EmptyLayout, EmptyMetrics, EmptyState, MAX_EMPTY_TEXT, empty_layout, fit_art};
pub use file_grid::{FILE_CACHE_CELLS, FILE_LABEL_LINES, FileGrid, FileGridEvent, FileItem, MAX_FILE_ITEMS, MAX_FILE_LABEL};
pub use focus::{FocusChange, FocusScope};
pub use glyph_grid::{GLYPH_CACHE_CELLS, GlyphColumns, GlyphGrid, GlyphGridEvent, GlyphItem, GlyphSection, MAX_GLYPH_ITEMS, MAX_GLYPH_LABEL, MAX_GLYPH_TEXT};
pub use font_picker::{FontEvent, FontPicker, FontPickerLabels};
pub use icon::{Icon, IconView, MAX_GLYPH_BYTES};
pub use icon_names::{IconName, UnknownIcon, glyph};
pub use interaction::{Interaction, Tap};
pub use label::Label;
pub use menu::{MAX_MENU_ITEMS, MAX_MENU_LABEL, MAX_MENU_SHORTCUT, Menu, MenuAlign, MenuAnchor, MenuEvent, MenuItem, MenuItemId, MenuItemKind, MenuModel, MenuSide, ScreenEdge, strip_mnemonic};
pub use layout::{Align, Row};
pub use nav::{NavItem, NavigationRail};
pub use notification::{
    BodySpan, MAX_ACTION_LABEL, MAX_NOTIFICATION_ACTIONS, MAX_NOTIFICATION_BODY, MAX_NOTIFICATION_META, MAX_NOTIFICATION_TITLE, NOTIFICATION_MASCOT_SIZE,
    NotificationCard, NotificationContent, NotificationEvent, NotificationStack, StackSlot};
pub use osd::{OSD_MAX_OVERFLOW, OSD_MAX_VALUE_TEXT, Osd};
pub use page_stack::{PageHeader, PageStack};
pub use path_bar::{MAX_PATH_SEGMENT, MAX_PATH_SEGMENTS, PathBar, PathBarEvent, PathSegment};
pub use notice::{Notice, NoticeKind, NoticeSize};
pub use pickers::{DateEvent, DatePicker, MINUTE_STEP, MINUTE_STEPS, Time, TimeEvent, TimePicker, day_periods, day_periods_for, format_date, format_time, normalize_minute_step, step_time, uses_12_hour};
pub use popover::{Popover, PopoverEvent};
pub use list::{List, ListDensity, ListEvent, ListItem, ListSelectEvent, ListTrailing};
pub use progress::{Gauge, Progress, Spinner};
pub use sparkline::{MAX_SPARK_SAMPLES, SampleHistory, SparkScale, Sparkline};
pub use tint::{Threshold, Tint};
pub use reorder::{ReorderEvent, ReorderItem, ReorderList};
pub use search::SearchField;
pub use results::{MAX_RESULT_TEXT, RESULT_CACHE_ROWS, ResultEvent, ResultItem, ResultList, TRAILING_MAX_FRACTION};
pub use scroll::{ScrollView, visible_range_uniform};
pub use secret::SecretBuffer;
pub use selection::Selection;
/// [`SecretBuffer::to_zeroizing`] dönüş tipi (çağıranın `zeroize` bağımlılığı gerekmesin).
pub use zeroize::Zeroizing;
pub use segmented::{MAX_SEGMENTS, MIN_SEGMENTS, Segment, SegmentedButton};
pub use select::{MAX_SELECT_LABEL, MAX_SELECT_OPTIONS, Select, SelectEntry, SelectEvent, SelectOption};
pub use snackbar::{MAX_SNACKBAR_TEXT, Snackbar, SnackbarEvent};
pub use slider::{Orientation, Slider};
pub use swatch::{MAX_SWATCHES, SwatchEvent, SwatchPicker, swatch_palette};
pub use text_input::{DEFAULT_MAX_BYTES, PASSWORD_MAX_BYTES, IME_SURROUNDING_MAX, ImeState, TextInput, TextInputEvent};
pub use theme_preview::{CursorPreview, Cursors, IconThemePreview, Icons, MAX_PREVIEW_SLOTS, PreviewFrame, PreviewKind, PreviewSlot, ThemePreview};
pub use tile::{MAX_TILE_TEXT, Tile, TileEvent, TileGrid, TileItem};
pub use toggle::Toggle;
pub use tooltip::{TOOLTIP_MAX_LINES, Tooltip};
pub use typography::{TextRole, text_style};
pub use ui::{DamageState, OverlayRegions, Response, Ui};
pub use week::{MAX_ALL_DAY_LANES, MAX_WEEK_EVENTS, MAX_WEEK_TITLE, SLOT_MINUTES, WeekEvent, WeekGrid, WeekGridEvent, week_start};
