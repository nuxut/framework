//! Platformdan bağımsız girdi olayları. Platform crate'leri (nuxut-wayland, ileride
//! nuxut-winit) bunları üretir; nuxut-ui bunları platform bilmeden tüketir.

use crate::geometry::Point;
pub use crate::secret::SecretText;

use zeroize::Zeroize;

use std::time::Duration;

/// Dokunmada basma ile sürükleme arasındaki eşik (mantıksal piksel). Parmak bundan az kaydıysa
/// dokunuş "yerinde" sayılır: kaydırma görünümü kaydırmaz, dokunuş tıklama olur.
pub const TOUCH_SLOP: f32 = 8.0;

/// Kaydırma görünümünde dokunmanın çocuklara basma olarak iletilmeden önce beklediği süre: bu
/// sürede parmak eşiği aşarsa hareket kaydırmadır ve çocuk basmayı hiç görmez. Daha önce kalkan
/// parmak (hızlı dokunuş) beklemeden basma + bırakma olarak iletilir. Runtime basılı ve
/// kımıldamayan birincil temas için bu süre dolunca aynı konumda bir `Motion` üretir.
pub const TOUCH_TAP_DELAY: Duration = Duration::from_millis(100);

/// İşaretçi olayı. `position` yüzeye göre mantıksal koordinattır.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEvent {
    pub position: Point,
    pub kind: PointerKind,
    /// Olayı üreten aygıt. Dokunmatik ekranın birincil teması işaretçi olarak öykünülür
    /// (`Touch`); bileşenlerin çoğu ayrımı umursamaz.
    pub source: PointerSource,
}

impl PointerEvent {
    /// Fare/dokunmatik yüzey olayı (`PointerSource::Mouse`).
    pub const fn new(position: Point, kind: PointerKind) -> Self {
        Self { position, kind, source: PointerSource::Mouse }
    }

    /// Dokunmatik ekran öykünmesi olayı (`PointerSource::Touch`).
    pub const fn touch(position: Point, kind: PointerKind) -> Self {
        Self { position, kind, source: PointerSource::Touch }
    }

    /// Aynı kaynaktan başka bir olay (bileşenlerin ürettiği türev olaylar kaynağı korur).
    pub const fn with_kind(&self, position: Point, kind: PointerKind) -> Self {
        Self { position, kind, source: self.source }
    }

    pub const fn is_touch(&self) -> bool {
        matches!(self.source, PointerSource::Touch)
    }

    /// Aynı olay, konumu `-offset` kaydırılmış (kaydırılan içerik koordinatına çevirmek için).
    pub fn translated(&self, offset: Point) -> Self {
        Self { position: self.position - offset, ..*self }
    }
}

/// İşaretçi olayının kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PointerSource {
    /// Fare, dokunmatik yüzey, tablet kalemi (wl_pointer).
    #[default]
    Mouse,
    /// Dokunmatik ekranın birincil teması (wl_touch öykünmesi): Enter → Motion → Press(Left) …
    /// Release(Left) → Leave; iptalde Release olmadan Leave. Hover yalnızca parmak basılıyken.
    Touch,
}

/// Bir dokunma temasının kimliği (`wl_touch` id'si; temas bitince yeniden kullanılabilir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TouchId(pub i32);

/// Ham dokunma olayı (tüm temaslar; çoklu dokunma bilen bileşenler için). Konum yüzeye göre
/// mantıksal koordinattır.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TouchEvent {
    pub id: TouchId,
    pub position: Point,
    pub phase: TouchPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchPhase {
    Down,
    Motion,
    Up,
    /// Compositor dizinin denetimini aldı (ör. hareket tanıdı): temas eylemsiz biter. `position`
    /// temasın son konumudur.
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PointerKind {
    Enter,
    Leave,
    Motion,
    Press(Button),
    Release(Button),
    /// `delta` mantıksal piksel (dokunmatik yüzey), `steps` tekerlek çentiği (kesirli olabilir).
    /// Pozitif y aşağı kaydırmadır.
    Scroll {
        delta: Point,
        steps: Point,
    },
    /// Dokunmatik yüzeyde parmaklar kalktı (`wl_pointer.axis_stop`): kaydırma bitti,
    /// fırlatma/geri sekme başlayabilir. Tekerlek için gelmez.
    ScrollEnd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    Left,
    Right,
    Middle,
    /// Linux `input-event-codes.h` kodu.
    Other(u32),
}

impl Button {
    /// Linux `input-event-codes.h` düğme kodundan (BTN_LEFT = 0x110 …).
    pub fn from_code(code: u32) -> Self {
        match code {
            0x110 => Self::Left,
            0x111 => Self::Right,
            0x112 => Self::Middle,
            other => Self::Other(other),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: Key,
    pub state: KeyState,
    /// Basılan tuşun ürettiği metin (yazdırılabilir karakterler; kontrol karakteri yok).
    /// Bırakmada her zaman `None`. Parola olabilir: [`SecretText`] düşerken sıfırlanır,
    /// `Debug`'da görünmez; `&str` gibi kullanılır (`text.as_deref()`, `&t`).
    pub text: Option<SecretText>,
    pub modifiers: Modifiers,
    /// Tuşun XKB çözümü (tuş kodu, keysym'ler ve adları). Platform klavyesinden gelen olaylarda
    /// `Some`; sentetik olaylarda (testler, yapıştırma, yeniden üretilen olaylar) `None`.
    pub xkb: Option<XkbKey>,
}

/// Bir tuş olayının XKB ayrıntısı: kısayol yakalama (Hyprland bind'leri keysym adıyla yazılır)
/// içindir; metin girişi `KeyEvent::key`/`text` kullanır.
///
/// Hyprland (0.56, `KeybindManager::onKeyEvent`) bind'i basılan tuşun **değiştiricisiz
/// (seviye 0)** keysym'iyle eşler: varsayılan olarak ilk düzenin (grup 0) — `input:kb_layout`'tan
/// derlediği değiştiricisiz durum —, `input:resolve_binds_by_sym = true` iken etkin düzenin.
/// Değiştiriciler (Shift dahil) ayrıca, maske olarak birebir eşlenir. Bu yüzden Shift+1 bind'i
/// `SHIFT, 1` yazılır (`exclam` değil): [`XkbKey::hyprland`].
#[derive(Clone, PartialEq, Eq)]
pub struct XkbKey {
    /// XKB tuş kodu (evdev kodu + 8). Hyprland `code:<n>` bind'i bu değeri bekler.
    pub keycode: u32,
    /// Geçerli değiştiriciler ve düzenle üretilen keysym (Türkçe Q'da Shift+1 → `exclam`,
    /// Shift+ı → `I`).
    pub sym: NamedKeysym,
    /// Aynı tuşun etkin düzende değiştiricisiz (seviye 0) keysym'i (Shift+1 → `1`,
    /// Shift+ı → `idotless`). Hyprland `resolve_binds_by_sym = true` iken eşlenen budur.
    pub base: NamedKeysym,
    /// Aynı tuşun ilk düzende (grup 0) seviye-0 keysym'i: Hyprland'in varsayılan eşlemesi.
    /// Tek düzende `base` ile aynıdır.
    pub first_layout_base: NamedKeysym,
}

impl XkbKey {
    /// Hyprland bind'ine yazılacak keysym (adı `name`). `resolve_binds_by_sym`: Hyprland
    /// `input:resolve_binds_by_sym` ayarı (varsayılan `false`).
    pub fn hyprland(&self, resolve_binds_by_sym: bool) -> &NamedKeysym {
        if resolve_binds_by_sym { &self.base } else { &self.first_layout_base }
    }
}

/// XKB keysym değeri ve `xkb_keysym_get_name` adı (`exclam`, `idotless`, `Return`; adı olmayan
/// Unicode keysym'lerde `U0130` biçimi — `xkb_keysym_from_name` bunu geri çözer). Simge yoksa
/// `raw == 0`, ad `NoSymbol`.
#[derive(Clone, PartialEq, Eq)]
pub struct NamedKeysym {
    pub raw: u32,
    pub name: String,
}

// Keysym adı ve değeri basılan tuşu (parola karakterini) açığa çıkarır: düşerken sıfırlanır.
impl Drop for NamedKeysym {
    fn drop(&mut self) {
        self.raw.zeroize();
        self.name.zeroize();
    }
}

impl NamedKeysym {
    /// Tuş simge üretmiyor (`NoSymbol`).
    pub fn is_none(&self) -> bool {
        self.raw == 0
    }
}

impl KeyEvent {
    /// Sentetik olay (`xkb: None`): testler ve yeniden üretilen olaylar için. `text` kopyalanmadan
    /// [`SecretText`]'e taşınır.
    pub fn new(key: Key, state: KeyState, text: Option<String>, modifiers: Modifiers) -> Self {
        Self { key, state, text: text.map(SecretText::from), modifiers, xkb: None }
    }

    /// Basma ya da tekrar (bırakma değil).
    pub fn is_down(&self) -> bool {
        self.state != KeyState::Released
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyState {
    Pressed,
    Repeated,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub logo: bool,
    /// Caps Lock kilidi açık (basılı tutulan bir değiştirici değil, durum). Kısayol eşlerken
    /// "değiştirici yok" denetimi bunu ve `num_lock`'u yok saymalı.
    pub caps_lock: bool,
    /// Num Lock kilidi açık (durum).
    pub num_lock: bool,
}

/// Mantıksal tuş. Düzene göre çözülmüştür (Q klavyede `Char('ş')` gelebilir).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Escape,
    Enter,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Space,
    /// Yazdırılabilir tek karakter (küçük/büyük harf düzene ve shift'e göre).
    Char(char),
    /// Eşlenmemiş XKB keysym değeri.
    Other(u32),
}

impl Key {
    /// XKB keysym → `Key`. Değerler `xkbcommon-keysyms.h`'den.
    pub fn from_keysym(sym: u32) -> Self {
        match sym {
            0xff1b => Self::Escape,
            0xff0d | 0xff8d => Self::Enter, // Return, KP_Enter
            0xff09 | 0xfe20 => Self::Tab,   // Tab, ISO_Left_Tab
            0xff08 => Self::Backspace,
            0xffff | 0xff9f => Self::Delete,
            0xff51 | 0xff96 => Self::Left,
            0xff53 | 0xff98 => Self::Right,
            0xff52 | 0xff97 => Self::Up,
            0xff54 | 0xff99 => Self::Down,
            0xff50 | 0xff95 => Self::Home,
            0xff57 | 0xff9c => Self::End,
            0xff55 | 0xff9a => Self::PageUp,
            0xff56 | 0xff9b => Self::PageDown,
            0x20 => Self::Space,
            // Latin-1 doğrudan Unicode'dur.
            0x21..=0x7e | 0xa0..=0xff => char::from_u32(sym).map_or(Self::Other(sym), Self::Char),
            // Unicode keysym'leri: 0x0100_0000 + kod noktası.
            0x0100_0100..=0x0110_ffff => {
                char::from_u32(sym - 0x0100_0000).map_or(Self::Other(sym), Self::Char)
            }
            other => Self::Other(other),
        }
    }
}

/// İmleç şekli (CSS / `wp_cursor_shape_v1` adlarıyla). Hangi şeklin ne zaman kullanılacağı
/// DESIGN.md "İmleç" bölümündedir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorShape {
    #[default]
    Default,
    /// Tıklanabilir öğe (el).
    Pointer,
    /// Metin girişi / seçilebilir metin (I).
    Text,
    /// Tutulabilir (açık el).
    Grab,
    /// Sürükleniyor (kapalı el).
    Grabbing,
    /// Açık bir eylem reddediliyor (ör. buraya bırakılamaz).
    NotAllowed,
    Move,
    /// Hassas nokta seçimi (renk alanı).
    Crosshair,
    /// Arka planda iş sürüyor, arayüz kullanılabilir.
    Progress,
    /// Arayüz bekliyor (kullanılmamalı; Progress tercih edilir).
    Wait,
    Help,
    ColResize,
    RowResize,
    ResizeN,
    ResizeS,
    ResizeE,
    ResizeW,
    ResizeNe,
    ResizeNw,
    ResizeSe,
    ResizeSw,
}

impl CursorShape {
    /// İlk `Some`'u seçer (en üstteki isabet önce gelmeli); yoksa `Default`.
    pub fn pick(candidates: impl IntoIterator<Item = Option<CursorShape>>) -> CursorShape {
        candidates.into_iter().flatten().next().unwrap_or_default()
    }

    /// Freedesktop/xcursor tema adı (protokol yoksa tema yedeği için).
    pub fn xcursor_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Pointer => "pointer",
            Self::Text => "text",
            Self::Grab => "grab",
            Self::Grabbing => "grabbing",
            Self::NotAllowed => "not-allowed",
            Self::Move => "move",
            Self::Crosshair => "crosshair",
            Self::Progress => "progress",
            Self::Wait => "wait",
            Self::Help => "help",
            Self::ColResize => "col-resize",
            Self::RowResize => "row-resize",
            Self::ResizeN => "n-resize",
            Self::ResizeS => "s-resize",
            Self::ResizeE => "e-resize",
            Self::ResizeW => "w-resize",
            Self::ResizeNe => "ne-resize",
            Self::ResizeNw => "nw-resize",
            Self::ResizeSe => "se-resize",
            Self::ResizeSw => "sw-resize",
        }
    }
}

/// Kontrol karakteri içeren metni (Escape'in `\x1b`'si, Enter'ın `\r`'si…) eler. Gelen tampon
/// kopyalanmadan [`SecretText`]'e taşınır; elenen metin de düşerken sıfırlanır.
pub fn printable(text: Option<String>) -> Option<SecretText> {
    printable_secret(text.map(SecretText::from))
}

/// [`printable`]'ın zaten [`SecretText`]'e taşınmış metin için olanı.
pub fn printable_secret(text: Option<SecretText>) -> Option<SecretText> {
    text.filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
}

// Kullanıcı içeriği loglanmaz (ARCHITECTURE değişmez 9): yazılan metin ve karakter tuşları
// Debug çıktısında gizlenir; yalnızca uzunluk/tür görünür.
impl std::fmt::Debug for KeyEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyEvent")
            .field("key", &self.key)
            .field("state", &self.state)
            .field("text", &self.text)
            .field("modifiers", &self.modifiers)
            .field("xkb", &self.xkb)
            .finish()
    }
}

// Tuş kodu ve keysym'ler basılan tuşu (parolayı) açığa çıkarır: tümü gizlenir.
impl std::fmt::Debug for XkbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("XkbKey(…)")
    }
}

impl std::fmt::Debug for NamedKeysym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NamedKeysym(…)")
    }
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Escape => f.write_str("Escape"),
            Self::Enter => f.write_str("Enter"),
            Self::Tab => f.write_str("Tab"),
            Self::Backspace => f.write_str("Backspace"),
            Self::Delete => f.write_str("Delete"),
            Self::Left => f.write_str("Left"),
            Self::Right => f.write_str("Right"),
            Self::Up => f.write_str("Up"),
            Self::Down => f.write_str("Down"),
            Self::Home => f.write_str("Home"),
            Self::End => f.write_str("End"),
            Self::PageUp => f.write_str("PageUp"),
            Self::PageDown => f.write_str("PageDown"),
            Self::Space => f.write_str("Space"),
            Self::Char(_) => f.write_str("Char(…)"),
            Self::Other(sym) => write!(f, "Other({sym:#x})"),
        }
    }
}

/// Gizlenmiş içerik: yalnızca karakter sayısı.
pub struct Redacted(pub usize);

impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} karakter gizli>", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms_map_to_keys() {
        assert_eq!(Key::from_keysym(0xff1b), Key::Escape);
        assert_eq!(Key::from_keysym(0xff0d), Key::Enter);
        assert_eq!(Key::from_keysym(0xff8d), Key::Enter);
        assert_eq!(Key::from_keysym(u32::from('a')), Key::Char('a'));
        assert_eq!(Key::from_keysym(0xfc), Key::Char('ü'));
        assert_eq!(Key::from_keysym(0x0100_015f), Key::Char('ş'));
        assert_eq!(Key::from_keysym(0xffe1), Key::Other(0xffe1)); // Shift_L
    }

    #[test]
    fn control_text_is_dropped() {
        assert_eq!(printable(Some("\u{1b}".into())), None);
        assert_eq!(printable(Some("\r".into())), None);
        assert_eq!(printable(Some(String::new())), None);
        assert_eq!(printable(Some("ğ".into())), Some("ğ".into()));
    }

    /// Tüketicilerin (shell, settings, calendar) `KeyEvent.text` kullanım kalıpları derlenmeye
    /// devam eder: `String`'den `SecretText`'e geçiş kaynak uyumlu.
    #[test]
    fn key_text_consumer_patterns_still_compile() {
        fn takes(s: &str) -> usize {
            s.len()
        }
        let ev = KeyEvent::new(Key::Char('ş'), KeyState::Pressed, Some('ş'.to_string()), Modifiers::default());
        assert_eq!(ev.text.as_deref(), Some("ş"));
        assert!(ev.text.is_some());
        if let Some(t) = &ev.text {
            assert_eq!(takes(t), 2);
            assert!(!t.is_empty() && t.chars().count() == 1 && t == "ş");
        }
        match &ev.text {
            Some(t) if !t.is_empty() => assert_eq!(takes(t), 2),
            _ => unreachable!(),
        }
        let mut typed = String::new();
        if let Some(text) = ev.clone().text {
            typed.push_str(&text);
        }
        assert_eq!(typed, "ş");
        let lit = KeyEvent { key: Key::Escape, state: KeyState::Pressed, text: None, modifiers: Modifiers::default(), xkb: None };
        assert_eq!(lit.text, None);
        let lit = KeyEvent { text: Some("a".into()), ..lit };
        assert_eq!(lit.text, Some("a".into()));
    }

    #[test]
    fn buttons() {
        assert_eq!(Button::from_code(0x110), Button::Left);
        assert_eq!(Button::from_code(0x113), Button::Other(0x113));
    }

    #[test]
    fn debug_hides_typed_content() {
        let ev = KeyEvent {
            key: Key::Char('ş'),
            state: KeyState::Pressed,
            text: Some("parola123 şifre".into()),
            modifiers: Modifiers::default(),
            xkb: Some(XkbKey {
                keycode: 47,
                sym: NamedKeysym { raw: 0x0100_015f, name: "scedilla".into() },
                base: NamedKeysym { raw: 0x0100_015f, name: "scedilla".into() },
                first_layout_base: NamedKeysym { raw: 0x3b, name: "semicolon".into() },
            }),
        };
        let s = format!("{ev:?}");
        assert!(!s.contains("parola") && !s.contains('ş') && !s.contains("şifre"), "{s}");
        assert!(!s.contains("scedilla") && !s.contains("semicolon") && !s.contains("47"), "{s}");
        assert!(s.contains("15 karakter gizli") && s.contains("Char(…)"), "{s}");
        assert_eq!(format!("{:?}", Key::Enter), "Enter");
    }

    #[test]
    fn cursor_pick_takes_topmost() {
        assert_eq!(CursorShape::pick([None, Some(CursorShape::Text), Some(CursorShape::Pointer)]), CursorShape::Text);
        assert_eq!(CursorShape::pick([None, None]), CursorShape::Default);
        assert_eq!(CursorShape::ResizeNe.xcursor_name(), "ne-resize");
    }
}
