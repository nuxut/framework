//! Tek satırlı metin girişi: imleç, seçim (fare/klavye), kelime atlama, kopyala/kes/yapıştır
//! kancaları, yer tutucu, parola modu, IME (text-input-v3) kancaları.
//!
//! - **Pano:** bileşen panoya dokunmaz; `Copy`/`Cut` olayıyla metni verir, `Ctrl+V`'de
//!   `PasteRequested` der; uygulama panodan okuyup [`TextInput::paste`] çağırır.
//! - **IME:** [`TextInput::ime_state`] runtime'ın `TextInputState`'ine birebir eşlenir
//!   (imleç dikdörtgeni + çevre metin + bayt konumları); `ImeEvent`'ler
//!   [`ime_preedit`](TextInput::ime_preedit), [`ime_commit`](TextInput::ime_commit),
//!   [`ime_delete_surrounding`](TextInput::ime_delete_surrounding) ile verilir.
//! - **İmleç:** yanıp sönmez (boşta sıfır iş); yeni konuma `spatial_fast` yayıyla kayar.
//! - **Ölçüm:** `TextLayout::cursor_x` / `hit_byte` (grapheme sınırları); layout yalnızca
//!   metin değişince yeniden şekillendirilir.

use std::fmt;
use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle, TextLayout, TextSystem};
use nuxut_theme::{STATE_HOVER, Theme};

use zeroize::{Zeroize, Zeroizing};

use crate::access::{AccessAction, AccessNode, AccessRequest, AccessTree, Action, NodeId, Role, child_id};
use crate::button::{IconButton, IconButtonVariant};
use crate::interaction::DOUBLE_CLICK;
use crate::icon::Icon;
use crate::icon_names::IconName;
use crate::redact::{Len, OptLen};
use crate::secret::SecretBuffer;
use crate::input::{Button, CursorShape, Key, KeyEvent, KeyState, PointerEvent, PointerKind};
use crate::style::{FOCUS_RING, HAIRLINE, disabled_content, field_fill, focus_ring, pill, state_layer_under};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Response, Ui};

/// Varsayılan en uzun metin (bayt).
pub const DEFAULT_MAX_BYTES: usize = 4096;
/// Parola kipinde en uzun metin (bayt); tampon bu kapasiteyle ayrılır.
pub const PASSWORD_MAX_BYTES: usize = 1024;
/// IME ön düzenlemesi: metin (düşerken sıfırlanır) ve içindeki imleç aralığı.
type Preedit = (Zeroizing<String>, Option<(usize, usize)>);

/// IME'ye gönderilen çevre metin sınırı (protokol: 4000 bayt).
pub const IME_SURROUNDING_MAX: usize = 4000;
/// Parola maskesi.
const MASK: char = '•';
/// Metin seçiminin `primary` opaklığı (yüksek karşıtlıkta seçili metin ≥ 4.5 kalacak kadarla sınırlı).
const SELECTION_TONE: f32 = 0.4;

#[derive(Clone, PartialEq, Eq)]
pub enum TextInputEvent {
    /// Metin değişti (yazma, silme, yapıştırma, IME).
    Changed,
    /// Enter.
    Submit,
    /// Esc (odak bırakılmak isteniyor).
    Cancel,
    /// Ctrl+C: panoya yazılacak metin.
    Copy(String),
    /// Ctrl+X: panoya yazılacak metin (metinden zaten silindi).
    Cut(String),
    /// Ctrl+V: uygulama panoyu okuyup `paste` çağırmalı.
    PasteRequested,
}

/// IME'ye bildirilecek durum (runtime'ın `TextInputState` + `SurroundingText`'i).
#[derive(Clone, PartialEq)]
pub struct ImeState {
    /// Mantıksal, yüzeye göre (aday penceresi buraya açılır).
    pub cursor_rect: Rect,
    pub surrounding: String,
    /// `surrounding` içinde bayt konumları.
    pub cursor: usize,
    pub anchor: usize,
    pub password: bool,
}

/// `Clone` yok: metin sıfırlanan, kopyalanmayan bir tamponda ([`SecretBuffer`]).
/// `Debug` metni, ön düzenlemeyi ve görüntü kopyasını yazmaz (yalnızca uzunluk).
pub struct TextInput {
    text: SecretBuffer,
    cursor: usize,
    anchor: usize,
    /// IME ön düzenlemesi; düşerken sıfırlanır. Parola kipinde hiç tutulmaz (`ime_preedit`).
    /// Kutulu: yalnız IME bileşimi sürerken var; satır içi tutmak her alana ~40 bayt ekler.
    preedit: Option<Box<Preedit>>,
    placeholder: String,
    password: bool,
    max_bytes: usize,
    enabled: bool,
    focused: bool,
    rect: Rect,
    style: nuxut_render::TextStyle,
    version: u64,
    cached_version: u64,
    display: String,
    layout: Option<TextLayout>,
    placeholder_layout: Option<TextLayout>,
    scroll_x: f32,
    caret_x: Animated<f32>,
    focus_anim: Animated<f32>,
    /// Hover tonu 0..1 (`effects`).
    hover: Animated<f32>,
    hovered: bool,
    /// Metnin sol/sağ ek boşluğu (ön/arka ikonlar için; SearchField).
    insets: (f32, f32),
    /// Parola gösteriliyor mu (yalnızca parola kipinde anlamlı; alan yine parola kipidir).
    revealed: bool,
    /// Sondaki göster/gizle düğmesi ([`TextInput::with_reveal_toggle`]). Kutulu: içindeki
    /// `IconButton` ~770 bayt; çoğu alanda yok, satır içi tutmak tüketicilerin enum kollarını
    /// şişiriyordu (clippy `large_enum_variant`, 2026-10-14).
    reveal: Option<Box<RevealToggle>>,
    /// Bir kez parola kipine girmiş alan: layout gizli yoldan şekillenir, bırakılırken silinir.
    secret_layouts: bool,
    pill: bool,
    dragging: bool,
    last_click: Option<(Instant, Point)>,
    last_caret_rect: Rect,
    damage: DamageState,
}


/// Parola göster/gizle düğmesi: alanın sağ ucunda `IconButton` (Standard), erişilebilir adlar
/// çağıranın dilinde (framework gömülü metin taşımaz; boşsa ad yok).
#[derive(Debug)]
struct RevealToggle {
    button: IconButton,
    /// Klavye odağı alandan düğmeye geçti (Tab); metin imleci gizlenir.
    focused: bool,
    /// (göster, gizle) eylem adları.
    labels: (String, String),
    /// Düğme kenarı (`space(8)`) ve alanın sağ kenarına uzaklığı (`space(2)`).
    size: f32,
    edge: f32,
}

impl RevealToggle {
    fn new(theme: &Theme, revealed: bool) -> Self {
        Self {
            button: IconButton::new(theme, reveal_icon(revealed), IconButtonVariant::Standard),
            focused: false,
            labels: (String::new(), String::new()),
            size: theme.space(8),
            edge: theme.space(2),
        }
    }
}

/// Eylemi gösteren ikon: gizliyken "göster" (göz), gösterilirken "gizle" (üstü çizili göz).
fn reveal_icon(revealed: bool) -> Icon {
    Icon::named(if revealed { IconName::VisibilityOff } else { IconName::Visibility })
}

impl fmt::Debug for TextInputEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Changed => f.write_str("Changed"),
            Self::Submit => f.write_str("Submit"),
            Self::Cancel => f.write_str("Cancel"),
            Self::Copy(s) => f.debug_tuple("Copy").field(&Len(s)).finish(),
            Self::Cut(s) => f.debug_tuple("Cut").field(&Len(s)).finish(),
            Self::PasteRequested => f.write_str("PasteRequested"),
        }
    }
}

impl fmt::Debug for ImeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImeState")
            .field("cursor_rect", &self.cursor_rect)
            .field("surrounding", &Len(&self.surrounding))
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .field("password", &self.password)
            .finish()
    }
}

impl fmt::Debug for TextInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextInput")
            .field("text", &self.text)
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .field("preedit", &OptLen(self.preedit.as_deref().map(|(s, _)| s.as_str())))
            .field("password", &self.password)
            .field("revealed", &self.revealed)
            .field("reveal_toggle", &self.reveal.is_some())
            .field("max_bytes", &self.max_bytes)
            .field("enabled", &self.enabled)
            .field("focused", &self.focused)
            .field("rect", &self.rect)
            .finish_non_exhaustive()
    }
}
// Metin `SecretBuffer`'ın, preedit `Zeroizing`'in kendi düşüşünde sıfırlanır; görüntü kopyası
// (parolada yalnızca maske, gösterilmiş parolada düz metin) burada.
impl Drop for TextInput {
    fn drop(&mut self) {
        self.display.zeroize();
        self.discard_layout();
    }
}

impl TextInput {
    pub fn new(theme: &Theme) -> Self {
        Self {
            text: SecretBuffer::with_capacity(DEFAULT_MAX_BYTES),
            cursor: 0,
            anchor: 0,
            preedit: None,
            placeholder: String::new(),
            password: false,
            max_bytes: DEFAULT_MAX_BYTES,
            enabled: true,
            focused: false,
            rect: Rect::ZERO,
            style: text_style(theme, TextRole::Body),
            version: 1,
            cached_version: 0,
            display: String::new(),
            layout: None,
            placeholder_layout: None,
            scroll_x: 0.0,
            caret_x: Animated::new(0.0, theme.motion.spatial_fast),
            focus_anim: Animated::new(0.0, theme.motion.effects),
            hover: Animated::new(0.0, theme.motion.effects),
            hovered: false,
            insets: (0.0, 0.0),
            revealed: false,
            reveal: None,
            secret_layouts: false,
            pill: false,
            dragging: false,
            last_click: None,
            last_caret_rect: Rect::ZERO,
            damage: DamageState::new(),
        }
    }

    pub fn with_placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }

    pub fn with_text(mut self, text: &str) -> Self {
        self.set_text(text);
        self
    }

    pub fn password(mut self, on: bool) -> Self {
        self.password = on;
        if on {
            // Parola kipinde ön düzenleme tutulmaz.
            self.preedit = None;
            // Bundan sonra (password(false) ile düz alana dönse de) her layout gizli yoldan.
            self.secret_layouts = true;
        } else {
            self.revealed = false;
        }
        self.discard_layout();
        if on && self.max_bytes > PASSWORD_MAX_BYTES {
            self = self.with_max_bytes(PASSWORD_MAX_BYTES);
        } else if on && !self.text.is_isolated() {
            // Parola tamponu kendi kilitli eşlemesine taşınır (eski yığın tamponu sıfırlanır).
            let max = self.max_bytes;
            self = self.with_max_bytes(max);
        }
        self.invalidate();
        self
    }

    /// Parola göster/gizle düğmesi (alanın sağ ucunda; **varsayılan kapalı** — mevcut alanlar,
    /// kilit ekranı ve polkit değişmez). Yalnızca parola kipinde görünür ve çalışır. Gösterilen
    /// parola hâlâ parola kipidir: `text()`/`selection()` boş, Copy/Cut olay üretmez, IME'ye
    /// çevre metin ve preedit yok; yalnızca çizim düz metindir. Erişilebilir adlar:
    /// [`set_reveal_labels`](Self::set_reveal_labels). Klavye: [`focus_step`](Self::focus_step).
    pub fn with_reveal_toggle(mut self, theme: &Theme, on: bool) -> Self {
        self.set_reveal_toggle(theme, on);
        self
    }

    /// Düğmeyi açar/kapatır; kapatmak parolayı da gizler.
    pub fn set_reveal_toggle(&mut self, theme: &Theme, on: bool) {
        if on == self.reveal.is_some() {
            return;
        }
        if on {
            let mut t = RevealToggle::new(theme, self.revealed);
            t.button.set_enabled(self.enabled, Instant::now());
            self.reveal = Some(Box::new(t));
        } else {
            self.reveal = None;
            self.set_revealed(false);
        }
        self.layout_rect();
        self.damage.mark();
    }

    pub fn has_reveal_toggle(&self) -> bool {
        self.reveal.is_some()
    }

    /// Parola gösteriliyor mu.
    pub fn is_revealed(&self) -> bool {
        self.revealed
    }

    /// Parolayı gösterir/gizler (düğme olmadan da; ör. uygulamanın kendi denetimi). Parola kipi
    /// dışında etkisiz. Gizlemede düz metnin layout'u sıfırlanarak bırakılır. Değiştiyse `true`.
    pub fn set_revealed(&mut self, on: bool) -> bool {
        let on = on && self.password;
        if self.revealed == on {
            return false;
        }
        self.revealed = on;
        if let Some(t) = &mut self.reveal {
            t.button.set_icon(reveal_icon(on));
        }
        self.discard_layout();
        self.invalidate();
        true
    }

    /// Düğmenin erişilebilir adları (çağıranın dilinde): `show` gizliyken, `hide` gösterilirken.
    pub fn set_reveal_labels(&mut self, show: &str, hide: &str) {
        if let Some(t) = &mut self.reveal {
            show.clone_into(&mut t.labels.0);
            hide.clone_into(&mut t.labels.1);
        }
    }

    /// Düğmenin şu anki erişilebilir adı (yapacağı eylem); düğme yoksa boş.
    pub fn reveal_label(&self) -> &str {
        match &self.reveal {
            Some(t) if self.revealed => &t.labels.1,
            Some(t) => &t.labels.0,
            None => "",
        }
    }

    /// Klavye odağı alanın içinde göster/gizle düğmesinde mi.
    pub fn is_reveal_focused(&self) -> bool {
        self.reveal.as_ref().is_some_and(|t| t.focused)
    }

    /// Alan içi Tab durağı: odaktaki parola alanında Tab metinden düğmeye, düğmedeyken Shift+Tab
    /// metne geçer ve olay tüketilir (`true`). Diğer her durumda `false`: olay uygulamanın
    /// `FocusScope`'una gider. Uygulama Tab'ı önce buraya verir:
    /// `if !input.focus_step(&ev, now) { scope.key(&ev) … }`.
    pub fn focus_step(&mut self, event: &KeyEvent, _now: Instant) -> bool {
        let m = event.modifiers;
        if event.key != Key::Tab || event.state == KeyState::Released || m.ctrl || m.alt || !self.enabled || !self.focused || !self.reveal_active() {
            return false;
        }
        let to_button = !m.shift;
        let Some(t) = &mut self.reveal else { return false };
        if t.focused == to_button {
            return false;
        }
        t.focused = to_button;
        t.button.set_focus_visible(to_button);
        if to_button {
            // Düğmedeyken seçim görünmez; imleç yerinde kalır.
            self.anchor = self.cursor;
        }
        self.damage.mark();
        true
    }

    /// Shift+Tab ile sondan gelinince: odak alana verilir ve (varsa) doğrudan düğmeye konur.
    pub fn focus_from_end(&mut self, now: Instant) {
        self.set_focused(true, now);
        if self.reveal_active()
            && let Some(t) = &mut self.reveal
        {
            t.focused = true;
            t.button.set_focus_visible(true);
            self.damage.mark();
        }
    }

    fn release_reveal_focus(&mut self) {
        if let Some(t) = &mut self.reveal
            && t.focused
        {
            t.focused = false;
            t.button.set_focus_visible(false);
            self.damage.mark();
        }
    }

    /// Düğme görünür ve kullanılabilir mi (parola kipinde).
    fn reveal_active(&self) -> bool {
        self.password && self.reveal.is_some()
    }

    fn toggle_reveal(&mut self) {
        let on = !self.revealed;
        self.set_revealed(on);
    }

    /// Sağ metin boşluğu: çağıranın yuvası + göster/gizle düğmesi.
    fn right_inset(&self, theme: &Theme) -> f32 {
        match &self.reveal {
            Some(t) if self.password => self.insets.1 + (t.edge + t.size - theme.space(4)).max(0.0),
            _ => self.insets.1,
        }
    }

    fn layout_rect(&mut self) {
        let rect = self.rect;
        let ir = self.insets.1;
        if let Some(t) = &mut self.reveal {
            let s = t.size;
            t.button.set_rect(Rect::new(rect.right() - ir - t.edge - s, rect.center().y - s * 0.5, s, s));
        }
    }

    /// Metin alanının sol/sağ ek boşluğu (ön/arka ikon yuvası).
    pub fn with_insets(mut self, left: f32, right: f32) -> Self {
        self.insets = (left.max(0.0), right.max(0.0));
        self.layout_rect();
        self
    }

    /// Hap biçimli alan (arama).
    pub fn pill_shaped(mut self, on: bool) -> Self {
        self.pill = on;
        self
    }

    /// Kapasite (bayt). Tampon yeniden ayrılır; eski tampon düşerken sıfırlanır. Parola kipinde
    /// tampon kilitli belleğe alınır ([`SecretBuffer::locked`]).
    pub fn with_max_bytes(mut self, max: usize) -> Self {
        self.max_bytes = max.max(1);
        let mut buf = if self.password {
            SecretBuffer::locked(self.max_bytes)
        } else {
            SecretBuffer::with_capacity(self.max_bytes)
        };
        let keep = truncate(self.text.as_str(), self.max_bytes);
        buf.set(keep);
        self.text = buf;
        self.cursor = self.cursor.min(self.text.len());
        self.anchor = self.anchor.min(self.text.len());
        self
    }

    /// Metin. **Parola kipinde boş döner**; sır yalnızca [`with_secret`](Self::with_secret) ile.
    pub fn text(&self) -> &str {
        if self.password { "" } else { self.text.as_str() }
    }

    /// Metne kısa ömürlü, ödünç erişim (parola kipinde de). Sonuç dışarı kopyalanmamalı;
    /// kopyalanırsa çağıran sıfırlamakla yükümlüdür.
    pub fn with_secret<R>(&self, f: impl FnOnce(&str) -> R) -> R {
        f(self.text.as_str())
    }

    /// Metin uzunluğu (bayt; parola kipinde de).
    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn is_password(&self) -> bool {
        self.password
    }

    /// Seçili metin (boş olabilir; parola kipinde her zaman boş).
    pub fn selection(&self) -> &str {
        if self.password {
            return "";
        }
        let (a, b) = self.sel_range();
        &self.text.as_str()[a..b]
    }

    /// Metin imleci ve seçim çapası (bayt).
    pub fn caret(&self) -> (usize, usize) {
        (self.cursor, self.anchor)
    }

    /// Seçimi `start..end` yapar (bayt): çapa `start`, imleç `end` (`end < start` = geriye doğru
    /// seçim; eşitse yalnızca imleç). Konumlar metin boyuna kırpılır ve grafem kümesi sınırına
    /// çekilir (kümenin içine düşen konum kümenin başına). Odaktan bağımsızdır; odak kaybı seçimi
    /// yine kaldırır. Örnek: dosya seçicide kaydetme adı `rapor.tar.gz` → `select_range(0, 5)`.
    /// Dönüş: seçim değişti mi.
    pub fn select_range(&mut self, start: usize, end: usize) -> bool {
        let (anchor, cursor) = (self.grapheme_floor(start), self.grapheme_floor(end));
        let changed = self.anchor != anchor || self.cursor != cursor;
        self.anchor = anchor;
        self.move_to(cursor, true);
        if changed {
            self.damage.mark();
        }
        changed
    }

    /// `i`'yi (metne kırpılmış) içinde kaldığı grafem kümesinin başına çeker.
    fn grapheme_floor(&self, i: usize) -> usize {
        use unicode_segmentation::UnicodeSegmentation;
        let s = self.text.as_str();
        if i >= s.len() {
            return s.len();
        }
        s.grapheme_indices(true).map(|(p, _)| p).take_while(|&p| p <= i).last().unwrap_or(0)
    }

    pub fn set_text(&mut self, text: &str) {
        let clean = Zeroizing::new(sanitize(text, self.max_bytes));
        if clean.as_str() != self.text.as_str() {
            self.text.set(&clean);
            self.cursor = self.text.len();
            self.anchor = self.cursor;
            self.preedit = None;
            self.invalidate();
        }
    }

    /// Metni ve IME ön düzenlemesini koşulsuz siler; tampon baytları sıfırlanır (parola
    /// alanı kapanınca/iptalde).
    pub fn clear(&mut self) {
        self.text.clear();
        // Kapanan/iptal edilen alan bir dahaki açılışta gizli başlar.
        self.set_revealed(false);
        self.cursor = 0;
        self.anchor = 0;
        self.preedit = None;
        self.invalidate();
    }

    /// Test kancası: tamponun ham baytları (silinen içerik sıfır olmalı).
    #[cfg(test)]
    pub(crate) fn raw_secret_bytes(&self) -> &[u8] {
        self.text.raw_bytes()
    }

    pub fn placeholder(&self) -> &str {
        &self.placeholder
    }

    pub fn set_placeholder(&mut self, text: &str) {
        if self.placeholder != text {
            text.clone_into(&mut self.placeholder);
            self.placeholder_layout = None;
            self.damage.mark();
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            if let Some(t) = &mut self.reveal {
                t.button.set_enabled(enabled, Instant::now());
                if !enabled {
                    t.focused = false;
                    t.button.set_focus_visible(false);
                }
            }
            self.damage.mark();
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    /// Klavye odağı (uygulama yönetir). Odak kaybında seçim ve preedit kalkar.
    pub fn set_focused(&mut self, focused: bool, now: Instant) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        self.focus_anim.set_target(if focused { 1.0 } else { 0.0 }, now);
        if let Some(t) = &mut self.reveal {
            // Odak her zaman metinden başlar (sondan giriş: `focus_from_end`).
            t.focused = false;
            t.button.set_focus_visible(false);
        }
        if !focused {
            self.anchor = self.cursor;
            self.preedit = None;
            self.dragging = false;
            self.invalidate();
        }
        self.damage.mark();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.style = text_style(theme, TextRole::Body);
        self.caret_x.set_motion(theme.motion.spatial_fast, now);
        self.focus_anim.set_motion(theme.motion.effects, now);
        self.hover.set_motion(theme.motion.effects, now);
        self.placeholder_layout = None;
        if let Some(t) = &mut self.reveal {
            t.button.set_theme(theme, now);
            t.size = theme.space(8);
            t.edge = theme.space(2);
        }
        self.layout_rect();
        self.invalidate();
    }

    /// Yükseklik `space(12)`; genişlik çağıranın.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        Size::new(width, theme.space(12))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.layout_rect();
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn invalidate(&mut self) {
        self.version += 1;
        if self.secret_layouts {
            // Gizli alanda eski görüntü (gösterilmişse düz parola) ve layout'u bir sonraki çizimi
            // beklemeden silinir: gizlenip hiç çizilmeyen alanda düz metin yığında kalmaz
            // (gizleme, clear, set_text, düzenleme, IME — hepsi buradan geçer).
            self.display.zeroize();
            self.cached_version = 0;
            self.discard_layout();
        }
        self.damage.mark();
    }

    fn sel_range(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    // --- Görüntü metni ve ölçüm -----------------------------------------------------

    /// Görüntüde maske mi (parola kipi ve gösterilmiyor).
    fn masked(&self) -> bool {
        self.password && !self.revealed
    }

    /// Görüntü metninin layout'u. Gizli alanın düz metni (gösterilen parola, ya da parola
    /// kipinden `password(false)` ile çıkmış alan) gizli yoldan şekillenir.
    fn shape_display(&self, text: &mut TextSystem) -> TextLayout {
        if self.secret_layouts && !self.masked() {
            text.layout_secret(&self.display, &self.style)
        } else {
            text.layout(&self.display, &self.style, None)
        }
    }

    /// Layout'u bırakır; gizli alanda önce siler (glif kimlikleri metni ele verir).
    fn discard_layout(&mut self) {
        if let Some(mut l) = self.layout.take()
            && self.secret_layouts
        {
            l.wipe();
        }
    }

    /// Görüntülenen metin: parola maskesi ve IME preedit uygulanmış.
    fn rebuild_display(&mut self) {
        if self.cached_version == self.version {
            return;
        }
        self.cached_version = self.version;
        self.discard_layout();
        let masked = self.masked();
        let mut d = String::with_capacity(self.text.len() + 8);
        let (pre, post) = self.text.as_str().split_at(self.cursor);
        let push = |d: &mut String, s: &str, mask: bool| {
            if mask {
                d.extend(std::iter::repeat_n(MASK, s.chars().count()));
            } else {
                d.push_str(s);
            }
        };
        push(&mut d, pre, masked);
        if let Some((p, _)) = self.preedit.as_deref() {
            // Parola kipinde ön düzenleme de yalnızca maske.
            push(&mut d, p, masked);
        }
        push(&mut d, post, masked);
        // Eski görüntü metni (parola gösterilmişse düz metin) bırakılmadan sıfırlanır.
        self.display.zeroize();
        self.display = d;
    }

    /// Metin baytı → görüntü baytı.
    fn to_display(&self, byte: usize) -> usize {
        let masked = self.masked();
        let pre_len = |s: &str| if masked { s.chars().count() * MASK.len_utf8() } else { s.len() };
        let preedit = self.preedit.as_deref().map_or(0, |(p, _)| pre_len(p));
        if byte <= self.cursor {
            pre_len(&self.text.as_str()[..byte])
        } else {
            pre_len(&self.text.as_str()[..self.cursor]) + preedit + pre_len(&self.text.as_str()[self.cursor..byte])
        }
    }

    /// Görüntü metninin layout'u (sürüm değişince yeniden şekillendirilir).
    fn display_layout(&mut self, text: &mut TextSystem) -> &TextLayout {
        self.rebuild_display();
        if self.layout.is_none() {
            self.layout = Some(self.shape_display(text));
        }
        self.layout.as_ref().expect("layout")
    }

    fn width_at(&mut self, text: &mut TextSystem, display_byte: usize) -> f32 {
        self.display_layout(text).cursor_x(display_byte)
    }

    fn caret_offset(&mut self, text: &mut TextSystem) -> f32 {
        let mut d = self.to_display(self.cursor);
        match self.preedit.as_deref() {
            // Parolada maske: imleç ön düzenlemenin sonunda.
            Some((p, _)) if self.masked() => d += p.chars().count() * MASK.len_utf8(),
            Some((_, Some((_, end)))) => d += end,
            Some((p, None)) => d += p.len(),
            None => {}
        }
        self.width_at(text, d)
    }

    /// Metin içi x (kaydırma öncesi) → en yakın sınır (metin baytı). Layout görüntü metninde
    /// (grapheme sınırına) isabet eder; parola/preedit eşlemesi geri çevrilir.
    fn hit(&mut self, text: &mut TextSystem, x: f32) -> usize {
        let d = self.display_layout(text).hit_byte(x);
        let mut best = 0;
        for b in self.text.as_str().char_indices().map(|(i, _)| i).chain(std::iter::once(self.text.len())) {
            if self.to_display(b) <= d {
                best = b;
            } else {
                break;
            }
        }
        best
    }

    fn text_origin_x(&self, theme: &Theme) -> f32 {
        self.rect.x + theme.space(4) + self.insets.0 - self.scroll_x
    }

    // --- Düzenleme ------------------------------------------------------------------

    fn replace_selection(&mut self, s: &str) -> bool {
        let (a, b) = self.sel_range();
        // Ara kopya sıfırlanarak düşer (parola yapıştırma/yazma).
        let s = Zeroizing::new(sanitize(s, usize::MAX));
        let room = self.max_bytes.min(self.text.capacity()).saturating_sub(self.text.len() - (b - a));
        // Parola kipinde sığmayan giriş bütünüyle reddedilir; düz metinde kırpılır.
        if self.password && s.len() > room {
            return false;
        }
        let s = truncate(&s, room);
        if a == b && s.is_empty() {
            return false;
        }
        if !self.text.replace_range(a..b, s) {
            return false;
        }
        self.cursor = a + s.len();
        self.anchor = self.cursor;
        self.invalidate();
        true
    }

    fn prev_boundary(&self, i: usize, word: bool) -> usize {
        if !word {
            return self.text.as_str()[..i].char_indices().next_back().map_or(0, |(p, _)| p);
        }
        let s = &self.text.as_str()[..i];
        let trimmed = s.trim_end_matches(|c: char| !c.is_alphanumeric());
        trimmed.char_indices().rev().find(|(_, c)| !c.is_alphanumeric()).map_or(0, |(p, c)| p + c.len_utf8())
    }

    fn next_boundary(&self, i: usize, word: bool) -> usize {
        if !word {
            return self.text.as_str()[i..].chars().next().map_or(i, |c| i + c.len_utf8());
        }
        let s = &self.text.as_str()[i..];
        let skip = s.len() - s.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
        let rest = &s[skip..];
        let end = rest.find(|c: char| !c.is_alphanumeric()).unwrap_or(rest.len());
        i + skip + end
    }

    fn move_to(&mut self, pos: usize, extend: bool) -> bool {
        let changed = self.cursor != pos || (!extend && self.anchor != pos);
        self.cursor = pos;
        if !extend {
            self.anchor = pos;
        }
        if changed {
            self.damage.mark();
            if self.preedit.is_some() {
                // Preedit imlecin yerinde görünür: görüntü metni değişir.
                self.version += 1;
            }
        }
        changed
    }

    /// Panodan gelen metni imlece (seçimin yerine) yapıştırır. Satır sonları boşluğa döner.
    pub fn paste(&mut self, text: &str) -> Response<TextInputEvent> {
        if !self.enabled || self.is_reveal_focused() {
            return Response::none();
        }
        if self.replace_selection(text) { Response::event(TextInputEvent::Changed) } else { Response::none() }
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<TextInputEvent> {
        if !self.enabled || !self.focused || event.state == KeyState::Released {
            return Response::none();
        }
        if self.is_reveal_focused() {
            // Düğmede: Enter/Space çevirir, Esc alanınki gibi; yazı alana gitmez.
            if event.key == Key::Escape {
                return Response::event(TextInputEvent::Cancel);
            }
            let r = self.reveal.as_mut().map(|t| t.button.key(event, now));
            if r.is_some_and(|r| r.event.is_some()) {
                self.toggle_reveal();
                return Response::repaint();
            }
            return Response::none();
        }
        let m = event.modifiers;
        let shift = m.shift;
        let word = m.ctrl;
        let changed = Response::event(TextInputEvent::Changed);
        match event.key {
            Key::Left => {
                let (a, _) = self.sel_range();
                let target = if !shift && self.cursor != self.anchor && !word { a } else { self.prev_boundary(self.cursor, word) };
                return Response::none().with_repaint(self.move_to(target, shift));
            }
            Key::Right => {
                let (_, b) = self.sel_range();
                let target = if !shift && self.cursor != self.anchor && !word { b } else { self.next_boundary(self.cursor, word) };
                return Response::none().with_repaint(self.move_to(target, shift));
            }
            Key::Home => return Response::none().with_repaint(self.move_to(0, shift)),
            Key::End => return Response::none().with_repaint(self.move_to(self.text.len(), shift)),
            Key::Backspace => {
                if self.cursor == self.anchor {
                    self.anchor = self.prev_boundary(self.cursor, word);
                }
                return if self.replace_selection("") { changed } else { Response::none() };
            }
            Key::Delete => {
                if self.cursor == self.anchor {
                    self.anchor = self.next_boundary(self.cursor, word);
                }
                return if self.replace_selection("") { changed } else { Response::none() };
            }
            Key::Enter => return Response::event(TextInputEvent::Submit),
            Key::Escape => return Response::event(TextInputEvent::Cancel),
            _ => {}
        }
        if m.ctrl && !m.alt {
            let c = match event.key {
                Key::Char(c) => c.to_ascii_lowercase(),
                _ => return Response::none(),
            };
            return match c {
                'a' => {
                    self.anchor = 0;
                    self.cursor = self.text.len();
                    self.invalidate();
                    Response::repaint()
                }
                'c' if !self.password && self.cursor != self.anchor => Response::event(TextInputEvent::Copy(self.selection().to_owned())),
                'x' if !self.password && self.cursor != self.anchor => {
                    let s = self.selection().to_owned();
                    self.replace_selection("");
                    Response::event(TextInputEvent::Cut(s))
                }
                'v' => Response::event(TextInputEvent::PasteRequested),
                _ => Response::none(),
            };
        }
        if m.alt || m.logo {
            return Response::none();
        }
        match &event.text {
            Some(t) if !t.is_empty() => {
                if self.replace_selection(t) {
                    changed
                } else {
                    Response::none()
                }
            }
            _ => Response::none(),
        }
    }

    /// Tıklama imleci koyar (odak isteği uygulamanın: `event` yok, odak dışarıda verilir),
    /// sürükleme seçer, çift tıklama kelimeyi seçer. `text`: ölçüm için.
    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<TextInputEvent> {
        if !self.enabled {
            return Response::none();
        }
        let mut button_repaint = false;
        if self.reveal_active() && !self.dragging {
            // Düğme önce: tıklama çevirir, odağı düğmeye taşımaz (halka yalnız klavyede).
            let on_button = self.reveal.as_ref().is_some_and(|t| t.button.rect().contains(event.position));
            let r = self.reveal.as_mut().map(|t| t.button.pointer(event, now));
            if r.as_ref().is_some_and(|r| r.event.is_some()) {
                self.toggle_reveal();
                return Response::repaint();
            }
            if on_button {
                return r.map_or_else(Response::none, Response::discard);
            }
            button_repaint = r.is_some_and(|r| r.repaint);
        }
        let inside = self.rect.contains(event.position);
        let x = event.position.x - self.text_origin_x(theme);
        let hovered = inside && event.kind != PointerKind::Leave;
        let mut hover_changed = false;
        if hovered != self.hovered {
            self.hovered = hovered;
            self.hover.set_target(if hovered { 1.0 } else { 0.0 }, now);
            self.damage.mark();
            hover_changed = true;
        }
        let r = match event.kind {
            PointerKind::Press(Button::Left) if inside => {
                // Metne tıklama düğme odağını bırakır: yazı yeniden alana gider.
                self.release_reveal_focus();
                let pos = self.hit(text, x);
                let double = self.last_click.is_some_and(|(t, p)| {
                    now.saturating_duration_since(t) < DOUBLE_CLICK && (p.x - event.position.x).abs() < theme.space(1)
                });
                self.last_click = Some((now, event.position));
                if double {
                    self.anchor = self.prev_boundary(self.next_boundary(pos, true), true);
                    self.cursor = self.next_boundary(self.anchor, true);
                    self.last_click = None;
                    self.invalidate();
                } else {
                    self.move_to(pos, false);
                    self.dragging = true;
                }
                Response::repaint()
            }
            PointerKind::Motion if self.dragging => {
                let pos = self.hit(text, x);
                Response::none().with_repaint(self.move_to(pos, true))
            }
            PointerKind::Release(Button::Left) | PointerKind::Leave if self.dragging => {
                self.dragging = false;
                Response::none()
            }
            _ => Response::none(),
        };
        r.with_repaint(hover_changed | button_repaint)
    }

    // --- IME ------------------------------------------------------------------------

    /// IME'ye gönderilecek durum. `cursor_rect` son çizimden.
    pub fn ime_state(&self) -> ImeState {
        // Çevre metin: imleç etrafında ≤ 4000 bayt, karakter sınırına hizalı.
        let half = IME_SURROUNDING_MAX / 2;
        let mut start = self.cursor.saturating_sub(half);
        while !self.text.as_str().is_char_boundary(start) {
            start += 1;
        }
        let mut end = (start + IME_SURROUNDING_MAX).min(self.text.len());
        while !self.text.as_str().is_char_boundary(end) {
            end -= 1;
        }
        let password = self.password;
        let surrounding = if password { String::new() } else { self.text.as_str()[start..end].to_owned() };
        let rel = |b: usize| if password { 0 } else { b.clamp(start, end) - start };
        ImeState { cursor_rect: self.last_caret_rect, surrounding, cursor: rel(self.cursor), anchor: rel(self.anchor), password }
    }

    /// Ön düzenleme (boş metin = bitti). `cursor`: preedit içi bayt aralığı. **Parola kipinde
    /// yok sayılır**: hiçbir kopya tutulmaz, varsa eskisi sıfırlanarak kalkar (runtime gizli alanda
    /// zaten preedit iletmez; bu ikinci kat).
    pub fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) -> Response<TextInputEvent> {
        if self.password {
            if self.preedit.take().is_some() {
                self.invalidate();
                return Response::repaint();
            }
            return Response::none();
        }
        let clean = Zeroizing::new(sanitize(text, usize::MAX));
        let text = Zeroizing::new(truncate(&clean, IME_SURROUNDING_MAX).to_owned());
        let new = if text.is_empty() {
            None
        } else {
            let fix = |b: usize| {
                let mut b = b.min(text.len());
                while !text.is_char_boundary(b) {
                    b -= 1;
                }
                b
            };
            Some((Zeroizing::new(text.as_str().to_owned()), cursor.map(|(a, b)| (fix(a), fix(b)))))
        };
        if new.as_ref() == self.preedit.as_deref() {
            return Response::none();
        }
        // Preedit seçimin yerine geçer: seçim önce silinir.
        if new.is_some() && self.cursor != self.anchor {
            self.replace_selection("");
        }
        self.preedit = new.map(Box::new);
        self.invalidate();
        Response::repaint()
    }

    pub fn ime_commit(&mut self, text: &str) -> Response<TextInputEvent> {
        if self.is_reveal_focused() {
            // key() ile aynı kural: düğmedeyken metin alana gitmez.
            return Response::none();
        }
        self.preedit = None;
        self.invalidate();
        if self.replace_selection(text) { Response::event(TextInputEvent::Changed) } else { Response::repaint() }
    }

    /// İmlecin önünden `before`, arkasından `after` bayt siler (karakter sınırına hizalanır).
    pub fn ime_delete_surrounding(&mut self, before: usize, after: usize) -> Response<TextInputEvent> {
        if self.is_reveal_focused() {
            return Response::none();
        }
        let mut a = self.cursor.saturating_sub(before);
        while !self.text.as_str().is_char_boundary(a) {
            a -= 1;
        }
        let mut b = (self.cursor + after).min(self.text.len());
        while !self.text.as_str().is_char_boundary(b) {
            b += 1;
        }
        if a == b {
            return Response::none();
        }
        if !self.text.replace_range(a..b, "") {
            return Response::none();
        }
        self.cursor = a;
        self.anchor = a;
        self.invalidate();
        Response::event(TextInputEvent::Changed)
    }

    /// İmleç: alan içinde `Text` (parolada da), devre dışı `Default`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if self.reveal_active()
            && !self.dragging
            && let Some(c) = self.reveal.as_ref().and_then(|t| t.button.cursor(p))
        {
            return Some(c);
        }
        (self.rect.contains(p) || self.dragging).then_some(if self.enabled { CursorShape::Text } else { CursorShape::Default })
    }

    // --- Çizim ----------------------------------------------------------------------

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let rect = self.rect;
        let f = self.focus_anim.value(now).clamp(0.0, 1.0);
        let radii = if self.pill { pill(rect) } else { Corners::uniform(theme.shape.radius_small) };

        // Kap: dolgulu alan (kaplardan ayrışan ton), hover'da bir ton açık. Kenar yalnızca kenar
        // kipi açıksa; odak her kipte altta 2px `primary` vurgu (+ imleç).
        let h = self.hover.value(now).clamp(0.0, 1.0);
        // Hover: bir ton açık; yüksek karşıtlıkta metin ve yer tutucu ≥ 4.5 kalacak kadarla sınırlı.
        let field = field_fill(theme);
        let hover = state_layer_under(theme, c.on_surface, STATE_HOVER * h, &[field], &[c.on_surface, c.on_surface_variant]);
        let fill = field.lerp(&c.on_surface, hover.a);
        ui.painter.rect(rect, &RectStyle::filled(ui.fade(fill), radii));
        if let Some(outline) = theme.outline() {
            let style = RectStyle { fill: Color::TRANSPARENT, radii, border: Some(Border { width: HAIRLINE, color: ui.fade(outline) }) };
            ui.painter.rect(rect, &style);
        }
        if f > 0.0 && self.pill {
            // Hap alanda alt çizgi yerine iç halka.
            let style = RectStyle { fill: Color::TRANSPARENT, radii, border: Some(Border { width: FOCUS_RING, color: ui.fade(focus_ring(theme, &[fill]).multiply_alpha(f)) }) };
            ui.painter.rect(rect, &style);
        } else if f > 0.0 {
            // Alt vurgu ortadan iki yana büyür.
            let w = rect.width * f;
            let line = Rect::new(rect.center().x - w * 0.5, rect.bottom() - FOCUS_RING, w, FOCUS_RING);
            ui.painter.rect(line, &RectStyle::filled(ui.fade(focus_ring(theme, &[fill])), Corners::uniform(FOCUS_RING * 0.5)));
        }

        let pad = theme.space(4);
        let (il, ir) = (self.insets.0, self.right_inset(theme));
        let inner = Rect::new(rect.x + pad + il, rect.y, (rect.width - pad * 2.0 - il - ir).max(0.0), rect.height);
        self.rebuild_display();
        let ts = ui.painter.text_system();
        let caret = self.caret_offset(ts);
        // İmleci görünür tut.
        if caret - self.scroll_x > inner.width {
            self.scroll_x = caret - inner.width;
        } else if caret < self.scroll_x {
            self.scroll_x = caret;
        }
        let full_w = self.width_at(ts, self.display.len());
        self.scroll_x = self.scroll_x.min((full_w - inner.width).max(0.0)).max(0.0);
        if self.caret_x.target() != caret {
            if !self.focused {
                self.caret_x.jump_to(caret);
            } else {
                self.caret_x.set_target(caret, now);
            }
        }
        let origin_x = inner.x - self.scroll_x;
        if self.layout.is_none() {
            self.layout = Some(self.shape_display(ts));
        }
        let line_h = self.layout.as_ref().map_or(self.style.size, |l| l.size().height.max(self.style.size));
        let ty = rect.center().y - line_h * 0.5;

        ui.painter.push_clip(inner.expand(1.0));
        // Seçim.
        let (a, b) = self.sel_range();
        if self.focused && a != b {
            let ts = ui.painter.text_system();
            let (da, db) = (self.to_display(a), self.to_display(b));
            let xa = self.width_at(ts, da);
            let xb = self.width_at(ts, db);
            let sel = Rect::new(origin_x + xa, ty, xb - xa, line_h);
            let sel_fill = state_layer_under(theme, c.primary, SELECTION_TONE, &[fill], &[c.on_surface]);
            ui.painter.rect(sel, &RectStyle::filled(ui.fade(sel_fill), Corners::uniform(theme.shape.radius_small * 0.25)));
        }
        let content = if self.enabled { c.on_surface } else { disabled_content(theme, c.on_surface, &[fill]) };
        if self.display.is_empty() {
            if self.placeholder_layout.is_none() {
                let s = self.style.clone();
                self.placeholder_layout = Some(ui.painter.text_system().layout(&self.placeholder, &s, None));
            }
            if let Some(pl) = &self.placeholder_layout {
                ui.painter.text(pl, Point::new(inner.x, ty), ui.fade(c.on_surface_variant));
            }
        } else if let Some(l) = &self.layout {
            ui.painter.text(l, Point::new(origin_x, ty), ui.fade(content));
        }
        // Preedit alt çizgisi.
        if let Some((p, _)) = self.preedit.as_deref() {
            let start = self.to_display(self.cursor);
            let end = start + p.len();
            let ts = ui.painter.text_system();
            let (xa, xb) = (self.width_at(ts, start), self.width_at(ts, end));
            let line = Rect::new(origin_x + xa, ty + line_h - HAIRLINE, xb - xa, HAIRLINE);
            ui.painter.rect(line, &RectStyle::filled(ui.fade(content), Corners::ZERO));
        }
        // İmleç.
        let cx = origin_x + self.caret_x.value(now);
        let caret_rect = Rect::new(cx - HAIRLINE, ty, FOCUS_RING, line_h);
        self.last_caret_rect = caret_rect;
        if self.focused && self.enabled && !self.is_reveal_focused() {
            ui.painter.rect(caret_rect, &RectStyle::filled(ui.fade(c.primary.multiply_alpha(f)), Corners::uniform(HAIRLINE)));
        }
        ui.painter.pop_clip();

        if self.reveal_active()
            && let Some(t) = &mut self.reveal
        {
            t.button.draw(ui);
        }

        let animating = !self.caret_x.is_settled(now) | !self.focus_anim.is_settled(now) | !self.hover.is_settled(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, rect.expand(FOCUS_RING), animating);
    }
}

/// Kontrol karakterlerini atar; satır sonu/sekme boşluğa döner; `max` bayta keser.
fn sanitize(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max));
    for c in s.chars() {
        let c = if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c };
        if c.is_control() {
            continue;
        }
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// Erişilebilirlik (ADR-0011).
impl TextInput {
    /// `Role::TextInput` (parola kipinde `Role::PasswordInput`): metin + imleç/seçim tek
    /// `TextRun` çocuğuyla, yer tutucu `placeholder`. Parola kipinde metin yerine yalnız maske
    /// (`•` × grafem sayısı; gösterilirken de). Göster/gizle düğmesi varsa çocuk `Role::Button`
    /// (adı [`set_reveal_labels`](Self::set_reveal_labels)). Odaktaki alan (ya da düğmesi) ağacın
    /// odağı olur. Ad (alan etiketi) çağıranın: `.label(..)`. AT'den metin yazılamaz (ADR-0011
    /// karar 6): `SetValue`/`ReplaceSelectedText` hiç iletilmez.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash) -> AccessNode<'t> {
        let role = if self.password { Role::PasswordInput } else { Role::TextInput };
        let reveal_focused = self.is_reveal_focused();
        let mut node = tree
            .push(key, role)
            .bounds(self.rect)
            .enabled(self.enabled)
            .action(Action::Focus)
            .focused_if(self.focused && !reveal_focused);
        if !self.placeholder.is_empty() {
            node.node_mut().set_placeholder(self.placeholder.as_str());
        }
        let (caret, anchor) = (self.cursor, self.anchor);
        let node = if self.password {
            self.with_secret(|s| node.text_content(s, caret, anchor, true))
        } else {
            node.text_content(self.text.as_str(), caret, anchor, false)
        };
        let id = node.id();
        if let Some(t) = self.reveal.as_ref().filter(|_| self.reveal_active()) {
            let label = self.reveal_label().to_owned();
            let rect = t.button.rect();
            tree.within(id, |tree| {
                tree.push("reveal", Role::Button)
                    .label(&label)
                    .bounds(rect)
                    .enabled(self.enabled)
                    .toggled(self.revealed)
                    .clickable()
                    .focused_if(self.focused && reveal_focused);
            });
        }
        tree.get(id).expect("az önce eklendi")
    }

    /// AT isteği göster/gizle düğmesine `Click` ise parolayı gösterir/gizler (`true`).
    /// `input`: `access`'in döndürdüğü alanın kimliği. Alana `Focus` çağıranın işidir.
    pub fn access_action(&mut self, input: NodeId, request: AccessRequest) -> bool {
        if request.action != AccessAction::Click || request.node != child_id(input, "reveal") || !self.enabled || !self.reveal_active() {
            return false;
        }
        self.toggle_reveal();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;
    use crate::input::test_util::*;

    fn input(text: &str) -> (TextInput, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = TextInput::new(&theme).with_text(text);
        i.set_focused(true, t0);
        i.set_rect(Rect::new(0.0, 0.0, 300.0, 48.0));
        (i, theme, t0)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent { key: Key::Char(c), state: KeyState::Pressed, text: None, modifiers: Modifiers { ctrl: true, ..Default::default() }, xkb: None }
    }

    fn shift(k: Key) -> KeyEvent {
        KeyEvent { key: k, state: KeyState::Pressed, text: None, modifiers: Modifiers { shift: true, ..Default::default() }, xkb: None }
    }

    #[test]
    fn typing_and_utf8_editing() {
        let (mut i, _t, t0) = input("");
        for c in "Çığ öş".chars() {
            assert_eq!(i.key(&typed(c), t0).event, Some(TextInputEvent::Changed));
        }
        assert_eq!(i.text(), "Çığ öş");
        let _ = i.key(&key(Key::Backspace), t0);
        assert_eq!(i.text(), "Çığ ö");
        let _ = i.key(&key(Key::Home), t0);
        let _ = i.key(&key(Key::Delete), t0);
        assert_eq!(i.text(), "ığ ö");
    }

    #[test]
    fn selection_copy_cut_paste() {
        let (mut i, _t, t0) = input("merhaba dünya");
        let _ = i.key(&ctrl('a'), t0);
        assert_eq!(i.key(&ctrl('c'), t0).event, Some(TextInputEvent::Copy("merhaba dünya".into())));
        let _ = i.key(&key(Key::End), t0);
        for _ in 0..5 {
            let _ = i.key(&shift(Key::Left), t0);
        }
        assert_eq!(i.selection(), "dünya");
        assert_eq!(i.key(&ctrl('x'), t0).event, Some(TextInputEvent::Cut("dünya".into())));
        assert_eq!(i.key(&ctrl('v'), t0).event, Some(TextInputEvent::PasteRequested));
        let _ = i.paste("evren\nve ötesi");
        assert_eq!(i.text(), "merhaba evren ve ötesi");
    }

    #[test]
    fn select_range_snaps_to_graphemes() {
        let (mut i, _t, t0) = input("rapor.tar.gz");
        assert!(i.select_range(0, 5));
        assert_eq!(i.selection(), "rapor");
        assert_eq!(i.caret(), (5, 0));
        assert!(!i.select_range(0, 5));
        // Geriye doğru ve sınır dışı.
        assert!(i.select_range(99, 6));
        assert_eq!(i.caret(), (6, 12));
        assert_eq!(i.selection(), "tar.gz");
        // Grafem kümesinin (e + birleşen akut, 🇹🇷) içine düşen konum kümenin başına çekilir.
        i.set_text("ca\u{301}fe 🇹🇷.txt");
        assert!(i.select_range(3, 9));
        assert_eq!(i.selection(), "a\u{301}fe ");
        let flag = "ca\u{301}fe ".len();
        i.select_range(0, flag + 4);
        assert_eq!(i.selection(), "ca\u{301}fe ");
        // Seçim yazımla değiştirilir; odak kaybı seçimi kaldırır.
        i.select_range(0, 2);
        assert_eq!(i.selection(), "c");
        let _ = i.key(&typed('X'), t0);
        assert_eq!(i.text(), "Xa\u{301}fe 🇹🇷.txt");
        i.select_range(0, 1);
        i.set_focused(false, t0);
        assert_eq!(i.selection(), "");
    }

    #[test]
    fn word_jumps() {
        let (mut i, _t, t0) = input("bir iki  üç");
        let word_left = KeyEvent { key: Key::Left, state: KeyState::Pressed, text: None, modifiers: Modifiers { ctrl: true, ..Default::default() }, xkb: None };
        let _ = i.key(&word_left, t0);
        assert_eq!(&i.text()[i.caret().0..], "üç");
        let _ = i.key(&word_left, t0);
        assert_eq!(&i.text()[i.caret().0..], "iki  üç");
        let ctrl_bs = KeyEvent { key: Key::Backspace, state: KeyState::Pressed, text: None, modifiers: Modifiers { ctrl: true, ..Default::default() }, xkb: None };
        let _ = i.key(&ctrl_bs, t0);
        assert_eq!(i.text(), "iki  üç");
    }

    #[test]
    fn max_bytes_and_control_chars() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = TextInput::new(&theme).with_max_bytes(5);
        i.set_focused(true, t0);
        let _ = i.paste("ab\u{7}cdefgh");
        assert_eq!(i.text(), "abcde");
        let _ = i.key(&typed('x'), t0);
        assert_eq!(i.text(), "abcde");
    }

    #[test]
    fn password_hides_and_refuses_copy() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = TextInput::new(&theme).password(true).with_text("gizli");
        i.set_focused(true, t0);
        i.rebuild_display();
        assert_eq!(i.display, "•••••");
        let _ = i.key(&ctrl('a'), t0);
        assert!(i.key(&ctrl('c'), t0).event.is_none());
        assert!(i.ime_state().surrounding.is_empty());
    }

    #[test]
    fn password_secret_handling() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = TextInput::new(&theme).password(true);
        assert_eq!(i.text.capacity(), PASSWORD_MAX_BYTES, "parola tamponu sabit 1 KiB");
        assert!(i.text.is_isolated(), "parola tamponu kendi (kilitli) eşlemesinde");
        // Kapasitesi zaten küçük alan da parolaya dönünce kilitli tampona taşınır, metin korunur.
        let small = TextInput::new(&theme).with_max_bytes(8).with_text("ab").password(true);
        assert!(small.text.is_isolated() && small.with_secret(|s| s == "ab"));
        assert_eq!(small.text.capacity(), 8);
        let small = small.with_max_bytes(16);
        assert!(small.text.is_isolated(), "parola kipinde yeniden boyut da kilitli");
        i.set_focused(true, t0);
        for ch in "çok-gizli".chars() {
            let _ = i.key(&typed(ch), t0);
        }
        assert_eq!(i.text(), "", "text() parolada boş");
        assert!(i.with_secret(|s| s == "çok-gizli"));
        let dbg = format!("{i:?}");
        assert!(!dbg.contains("gizli"), "Debug sır içermemeli: {dbg}");
        // Pano: kopyala/kes olay üretmez.
        let _ = i.key(&ctrl('a'), t0);
        assert!(i.key(&ctrl('c'), t0).event.is_none());
        assert!(i.key(&ctrl('x'), t0).event.is_none());
        assert!(i.with_secret(|s| s == "çok-gizli"), "kes silmedi");
        // Kapasiteyi aşan yapıştırma bütünüyle reddedilir.
        let _ = i.key(&key(Key::End), t0);
        let big = "x".repeat(PASSWORD_MAX_BYTES);
        assert!(i.paste(&big).event.is_none());
        assert_eq!(i.len(), "çok-gizli".len());
        // Preedit parolada hiç tutulmaz.
        assert!(i.ime_preedit("ab", None).event.is_none());
        assert!(i.preedit.is_none());
        i.rebuild_display();
        assert!(!i.display.contains("ab") && !i.display.contains("gizli"));
        // Düşmeden önceki temizlik tüm baytları sıfırlar.
        assert!(i.text.wipe_and_inspect().iter().all(|b| *b == 0));
    }

    /// Ayrılmış tamponda içerikten sonraki her bayt sıfır mı (silinen/kaydırılan izi yok).
    fn tail_is_zero(i: &TextInput) -> bool {
        i.raw_secret_bytes()[i.len()..].iter().all(|b| *b == 0)
    }

    #[test]
    fn password_buffer_leaves_no_trace_through_edits() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = TextInput::new(&theme).password(true);
        i.set_focused(true, t0);
        // Yazma (SecretText taşıyan KeyEvent'ler) ve silme.
        for ch in "parolaş".chars() {
            let _ = i.key(&typed(ch), t0);
        }
        assert!(tail_is_zero(&i));
        let _ = i.key(&key(Key::Backspace), t0);
        let _ = i.key(&key(Key::Backspace), t0);
        assert!(i.with_secret(|s| s == "parol"));
        assert!(tail_is_zero(&i), "silinen 'aş' baytları sıfır");
        // Ortadan silme kuyruğu kaydırır; boşalan son baytlar sıfır.
        let _ = i.key(&key(Key::Home), t0);
        let _ = i.key(&key(Key::Delete), t0);
        assert!(i.with_secret(|s| s == "arol"));
        assert!(tail_is_zero(&i));
        // IME commit (gizli alanda preedit yok, commit doğrudan tampona).
        let _ = i.key(&key(Key::End), t0);
        assert_eq!(i.ime_commit("çğ").event, Some(TextInputEvent::Changed));
        assert!(i.with_secret(|s| s == "arolçğ"));
        assert!(tail_is_zero(&i));
        let _ = i.ime_delete_surrounding(4, 0);
        assert!(i.with_secret(|s| s == "arol"));
        assert!(tail_is_zero(&i));
        // Tümünü seçip yazmak eskisini yerinde ezer.
        let _ = i.key(&ctrl('a'), t0);
        let _ = i.key(&typed('x'), t0);
        assert!(i.with_secret(|s| s == "x"));
        assert!(tail_is_zero(&i));
        // Odak kaybı: preedit/seçim kalkar; metin uygulamanın kararıyla kalır (kilit ekranında
        // çıkış arası odak değişimi parolayı silmemeli), temizleme `clear()` ile.
        let _ = i.ime_preedit("y", None);
        i.set_focused(false, t0);
        assert!(i.preedit.is_none() && i.caret().0 == i.caret().1);
        assert!(tail_is_zero(&i));
        // Temizleme: ayrılmış kapasitenin tamamı sıfır, kapasite (1 KiB) korunur.
        i.clear();
        assert_eq!(i.raw_secret_bytes().len(), PASSWORD_MAX_BYTES);
        assert!(i.raw_secret_bytes().iter().all(|b| *b == 0));
    }

    fn reveal_input(text: &str) -> (TextInput, Theme, Instant) {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut i = TextInput::new(&theme).password(true).with_reveal_toggle(&theme, true).with_text(text);
        i.set_focused(true, t0);
        i.set_rect(Rect::new(0.0, 0.0, 300.0, 48.0));
        (i, theme, t0)
    }

    fn tab(shift: bool) -> KeyEvent {
        key_mod(Key::Tab, Modifiers { shift, ..Default::default() })
    }

    #[test]
    fn reveal_toggle_is_opt_in_and_password_only() {
        let theme = Theme::default();
        let p = TextInput::new(&theme).password(true);
        assert!(!p.has_reveal_toggle(), "varsayılan kapalı");
        let mut plain = TextInput::new(&theme).with_reveal_toggle(&theme, true).with_text("açık");
        assert!(plain.has_reveal_toggle() && !plain.reveal_active());
        assert!(!plain.set_revealed(true), "parola kipi dışında etkisiz");
        plain.set_focused(true, Instant::now());
        assert!(!plain.focus_step(&tab(false), Instant::now()), "düz alanda Tab durağı yok");
        // Düğme yer kaplar yalnızca parolada.
        assert_eq!(plain.right_inset(&theme), 0.0);
        let (i, ..) = reveal_input("");
        assert!(i.right_inset(&theme) > 0.0);
        let b = i.reveal.as_ref().unwrap().button.rect();
        assert!(b.right() <= i.rect.right() && b.x > i.rect.center().x, "sağ uçta: {b:?}");
    }

    #[test]
    fn revealed_password_keeps_secret_semantics() {
        let (mut i, _t, t0) = reveal_input("gizli");
        assert!(i.set_revealed(true));
        i.rebuild_display();
        assert_eq!(i.display, "gizli", "yalnızca çizim düz metin");
        assert!(i.is_password() && i.text().is_empty() && i.selection().is_empty());
        let _ = i.key(&ctrl('a'), t0);
        assert!(i.key(&ctrl('c'), t0).event.is_none() && i.key(&ctrl('x'), t0).event.is_none());
        let s = i.ime_state();
        assert!(s.password && s.surrounding.is_empty());
        assert!(i.ime_preedit("ön", None).event.is_none() && i.preedit.is_none());
        let dbg = format!("{i:?}");
        assert!(!dbg.contains("gizli") && dbg.contains("revealed: true"), "{dbg}");
        assert!(i.text.is_isolated(), "tampon yine kilitli eşlemede");
        // Gizle: maske geri gelir.
        assert!(i.set_revealed(false));
        i.rebuild_display();
        assert_eq!(i.display, "•••••");
    }

    #[test]
    fn hiding_resets_plaintext_layout() {
        let (mut i, _t, _) = reveal_input("çok-gizli");
        let mut ts = TextSystem::new();
        i.set_revealed(true);
        let plain_w = i.display_layout(&mut ts).size().width;
        assert!(i.layout.is_some() && i.secret_layouts);
        // Gizlemek düz metnin layout'unu hemen bırakır (sonraki karede beklemez).
        i.set_revealed(false);
        assert!(i.layout.is_none(), "gösterilen layout gizlenince bırakıldı");
        let masked_w = i.display_layout(&mut ts).size().width;
        if !ts.families().is_empty() {
            assert_ne!(plain_w, masked_w);
        }
        // Düz alana dönüş (password(false)) de gizli yolda kalır; layout bırakılır.
        let theme = Theme::default();
        let mut j = TextInput::new(&theme).password(true).with_text("x");
        let _ = j.display_layout(&mut ts);
        let j = j.password(false);
        assert!(j.layout.is_none() && j.secret_layouts && !j.is_revealed());
        // Düz alan hiç gizli yola girmez.
        assert!(!TextInput::new(&theme).secret_layouts);
    }

    #[test]
    fn reveal_button_keyboard() {
        let (mut i, _t, t0) = reveal_input("abc");
        // Tab: metinden düğmeye; olay tüketilir.
        assert!(i.focus_step(&tab(false), t0));
        assert!(i.is_reveal_focused());
        // Düğmedeyken Tab uygulamaya gider (sıradaki bileşen).
        assert!(!i.focus_step(&tab(false), t0));
        // Yazı alana gitmez.
        assert!(i.key(&typed('z'), t0).event.is_none());
        assert!(i.with_secret(|s| s == "abc"));
        // Space ve Enter çevirir.
        let _ = i.key(&key(Key::Space), t0);
        assert!(i.is_revealed());
        let _ = i.key(&key(Key::Enter), t0);
        assert!(!i.is_revealed());
        assert_eq!(i.key(&key(Key::Escape), t0).event, Some(TextInputEvent::Cancel));
        // Shift+Tab: metne döner; metinde Shift+Tab uygulamanın.
        assert!(i.focus_step(&tab(true), t0));
        assert!(!i.is_reveal_focused());
        assert!(!i.focus_step(&tab(true), t0));
        // Metindeyken Enter yine Submit.
        assert_eq!(i.key(&key(Key::Enter), t0).event, Some(TextInputEvent::Submit));
        // Sondan giriş düğmeye konur; odak kaybı düğme odağını da kaldırır.
        i.set_focused(false, t0);
        assert!(!i.focus_step(&tab(false), t0), "odaksız alan Tab almaz");
        i.focus_from_end(t0);
        assert!(i.is_focused() && i.is_reveal_focused());
        i.set_focused(false, t0);
        assert!(!i.is_reveal_focused());
        // Devre dışı alan.
        i.set_focused(true, t0);
        i.set_enabled(false);
        assert!(!i.focus_step(&tab(false), t0) && !i.is_reveal_focused());
    }

    #[test]
    fn reveal_button_pointer_and_lifecycle() {
        let (mut i, theme, t0) = reveal_input("abc");
        let mut ts = TextSystem::new();
        let _ = i.key(&key(Key::Home), t0);
        let c = i.reveal.as_ref().unwrap().button.rect().center();
        let _ = i.pointer(&mut ts, &theme, &motion(c.x, c.y), t0);
        assert_eq!(i.cursor(c), Some(CursorShape::Pointer));
        let _ = i.pointer(&mut ts, &theme, &press(c.x, c.y), t0);
        let r = i.pointer(&mut ts, &theme, &release(c.x, c.y), t0);
        assert!(r.repaint && i.is_revealed(), "tıklama gösterir");
        assert_eq!(i.caret().0, 0, "düğmeye tıklama imleci taşımaz");
        assert!(!i.is_reveal_focused(), "fare odağı düğmeye taşımaz");
        // Etiketler eylemi izler.
        i.set_reveal_labels("Parolayı göster", "Parolayı gizle");
        assert_eq!(i.reveal_label(), "Parolayı gizle");
        // clear(): metin silinir, parola yeniden gizlenir.
        i.clear();
        assert!(!i.is_revealed() && i.raw_secret_bytes().iter().all(|b| *b == 0));
        assert_eq!(i.reveal_label(), "Parolayı göster");
        // Düğmeyi kapatmak gizler.
        i.set_revealed(true);
        i.set_reveal_toggle(&theme, false);
        assert!(!i.is_revealed() && i.reveal_label().is_empty());
    }

    #[test]
    fn plaintext_display_is_wiped_without_redraw() {
        let (mut i, _t, t0) = reveal_input("çok-gizli");
        let mut ts = TextSystem::new();
        i.set_revealed(true);
        let _ = i.display_layout(&mut ts);
        assert_eq!(i.display, "çok-gizli");
        // Gizle: çizim beklenmeden düz kopya ve layout gider.
        i.set_revealed(false);
        assert!(i.display.is_empty() && i.layout.is_none(), "gizleme sonrası çizimsiz");
        // Düzenleme (gösterilmişken) de eski düz görüntüyü hemen siler.
        i.set_revealed(true);
        let _ = i.display_layout(&mut ts);
        let _ = i.key(&typed('x'), t0);
        assert!(i.display.is_empty() && i.layout.is_none(), "düzenleme");
        let _ = i.display_layout(&mut ts);
        i.set_text("yeni");
        assert!(i.display.is_empty(), "set_text");
        let _ = i.display_layout(&mut ts);
        i.clear();
        assert!(i.display.is_empty() && i.layout.is_none() && !i.is_revealed(), "clear");
        // Yeniden çizimde maske gelir.
        i.with_secret(|s| assert!(s.is_empty()));
        let _ = i.paste("ab");
        i.rebuild_display();
        assert_eq!(i.display, "••");
    }

    #[test]
    fn button_focus_yields_to_text_click_and_blocks_paste_ime() {
        let (mut i, theme, t0) = reveal_input("abc");
        let mut ts = TextSystem::new();
        assert!(i.focus_step(&tab(false), t0));
        // Düğmedeyken yapıştırma/IME metne girmez (key() ile aynı kural).
        assert!(i.paste("zz").event.is_none());
        assert!(i.ime_commit("zz").event.is_none());
        assert!(i.ime_delete_surrounding(1, 0).event.is_none());
        assert!(i.with_secret(|s| s == "abc"));
        // Metne tıklama düğme odağını bırakır; yazı yeniden alana gider.
        let x = i.text_origin_x(&theme) + 2.0;
        let _ = i.pointer(&mut ts, &theme, &press(x, 24.0), t0);
        let _ = i.pointer(&mut ts, &theme, &release(x, 24.0), t0);
        assert!(!i.is_reveal_focused());
        let _ = i.key(&typed('q'), t0);
        assert_eq!(i.len(), 4);
    }

    #[test]
    fn password_mode_drops_existing_preedit() {
        let (i, _t, t0) = input("");
        let mut i = i;
        let _ = i.ime_preedit("abc", None);
        assert!(i.preedit.is_some(), "normal alanda preedit tutulur");
        let mut i = i.password(true);
        assert!(i.preedit.is_none(), "parolaya dönünce bırakılır");
        i.set_focused(true, t0);
        let _ = i.ime_preedit("def", Some((0, 3)));
        assert!(i.preedit.is_none());
        assert!(i.ime_state().password && i.ime_state().surrounding.is_empty());
    }

    #[test]
    fn ime_flow() {
        let (mut i, _t, t0) = input("ab");
        let _ = i.key(&key(Key::Left), t0);
        let _ = i.ime_preedit("にほ", Some((0, 6)));
        i.rebuild_display();
        assert_eq!(i.display, "aにほb");
        assert_eq!(i.text(), "ab", "preedit metne yazılmaz");
        let _ = i.ime_commit("日本");
        assert_eq!(i.text(), "a日本b");
        let s = i.ime_state();
        assert_eq!(&s.surrounding[..s.cursor], "a日本");
        let _ = i.ime_delete_surrounding(3, 1);
        assert_eq!(i.text(), "a日");
    }

    #[test]
    fn debug_is_redacted() {
        use crate::redact::{SECRET, assert_redacted};
        let (mut i, _t, _) = input(SECRET);
        let _ = i.ime_preedit("gizli-preedit", None);
        i.rebuild_display();
        assert_redacted(&i);
        assert_redacted(&i.ime_state());
        assert_redacted(&TextInputEvent::Copy(SECRET.into()));
        assert_redacted(&TextInputEvent::Cut(SECRET.into()));
        assert!(format!("{:?}", TextInputEvent::Copy(SECRET.into())).contains("15 bayt"));
    }

    #[test]
    fn click_hit_testing() {
        let (mut i, theme, t0) = input("abcdef");
        let mut text = TextSystem::new();
        let origin = i.text_origin_x(&theme);
        let _ = i.pointer(&mut text, &theme, &press(origin - 50.0 + 51.0, 20.0), t0);
        assert!(i.caret().0 <= 1);
        let _ = i.pointer(&mut text, &theme, &release(origin, 20.0), t0);
        let far = Instant::now() + std::time::Duration::from_secs(1);
        let _ = i.pointer(&mut text, &theme, &press(origin + 250.0, 20.0), far);
        assert_eq!(i.caret().0, 6);
        // Sürükleyerek seçim.
        let _ = i.pointer(&mut text, &theme, &motion(origin, 20.0), far);
        assert_eq!(i.selection(), "abcdef");
    }

    /// Boyut kapısı: `TextInput` tüketicilerin durum enum'larında satır içi duruyor; büyürse
    /// clippy `large_enum_variant` onların derlemesini kırar (yaşandı: d576359 sonrası 1648 bayt,
    /// 2026-10-14). Nadir durum (göster/gizle düğmesi, IME ön düzenlemesi)
    /// `Option<Box<..>>` içinde. Sınır, göster/gizle öncesi boyut (5fe1e2e: 872 bayt); aşan yeni
    /// alan ya kutulanır ya da bu sınır gerekçesiyle ve tüketiciler denetlenerek yükseltilir.
    #[test]
    fn size_stays_small() {
        let size = std::mem::size_of::<TextInput>();
        assert!(size <= 872, "TextInput {size} bayt; nadir alanları Option<Box<..>> içine al");
    }
}
