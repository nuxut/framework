//! Metin girişi / IME (`zwp_text_input_v3`) ve klavye düzeni adı.
//!
//! IME yoksa ya da compositor protokolü sunmuyorsa her şey sessizce no-op'tur; tuşlar normal
//! `KeyEvent.text` ile gelir.
//!
//! **Gizli alan** (`TextPurpose::Password` odakta): compositor'a `purpose = password` +
//! `sensitive_data | hidden_text` bildirilir, çevre metin gönderilmez; gelen preedit hiç
//! saklanmaz (sıfırlanıp atılır), commit [`SecretText`] olarak verilir (tüketilince sıfırlanır).

use std::collections::HashMap;

use nuxut_core::input::Redacted;
use nuxut_core::{Rect, SecretText};
use smithay_client_toolkit::compositor::SurfaceData;
use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_manager_v3::ZwpTextInputManagerV3;
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    self, ContentHint, ContentPurpose, ZwpTextInputV3,
};

use crate::App;
use crate::state::State;
use crate::types::SurfaceId;

/// Çevre metin sınırı (protokolün önerdiği 4000 bayt).
pub const MAX_SURROUNDING: usize = 4000;
/// IME'den gelen preedit/commit metni sınırı; fazlası kesilir.
pub const MAX_IME_TEXT: usize = 4096;

/// Odaktaki metin alanının IME'ye bildirilen durumu.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputState {
    /// İmleç dikdörtgeni, **yüzey koordinatında** (mantıksal, yüzeyin sol-üstüne göre); aday
    /// penceresi buna göre konumlanır. Bileşen/layout koordinatından çevirmek uygulamanın işi.
    pub cursor_rect: Rect,
    pub surrounding: Option<SurroundingText>,
    pub purpose: TextPurpose,
    pub multiline: bool,
}

impl Default for TextInputState {
    fn default() -> Self {
        Self {
            cursor_rect: Rect::ZERO,
            surrounding: None,
            purpose: TextPurpose::Normal,
            multiline: false,
        }
    }
}

/// İmleç çevresindeki metin; `cursor`/`anchor` bayt ofsetidir (seçim yoksa eşit).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct SurroundingText {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextPurpose {
    #[default]
    Normal,
    Password,
    Email,
    Url,
    Number,
    Phone,
    Terminal,
}

/// IME'den gelen düzenleme. Bir `done` içindekiler bu sırayla verilir:
/// `DeleteSurrounding` → `Commit` → `Preedit`. Metinler [`SecretText`]: düşerken sıfırlanır,
/// `&str` gibi kullanılır (`input.ime_commit(&text)`).
#[derive(Clone, PartialEq, Eq)]
pub enum ImeEvent {
    /// Oluşturulmakta olan metin; boş `text` = preedit bitti. `cursor` bayt aralığıdır,
    /// `None` = imleç gizli. Gizli alan odaktayken yalnızca boş (temizleme) preedit gelir.
    Preedit {
        text: SecretText,
        cursor: Option<(usize, usize)>,
    },
    /// İmlece eklenecek metin.
    Commit(SecretText),
    /// İmlecin önünden/arkasından silinecek bayt sayıları.
    DeleteSurrounding { before: usize, after: usize },
}

#[derive(Default)]
struct Pending {
    preedit: Option<(SecretText, i32, i32)>,
    commit: Option<SecretText>,
    delete: Option<(u32, u32)>,
}

// Kullanıcı içeriği loglanmaz (ARCHITECTURE değişmez 9): metin alanları Debug'da yalnızca
// karakter sayısıyla görünür; ofsetler ve tür serbesttir.
impl std::fmt::Debug for SurroundingText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurroundingText")
            .field("text", &Redacted(self.text.chars().count()))
            .field("cursor", &self.cursor)
            .field("anchor", &self.anchor)
            .finish()
    }
}

impl std::fmt::Debug for ImeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // SecretText'in Debug'ı yalnızca karakter sayısını yazar.
            Self::Preedit { text, cursor } => f
                .debug_struct("Preedit")
                .field("text", text)
                .field("cursor", cursor)
                .finish(),
            Self::Commit(text) => f.debug_tuple("Commit").field(text).finish(),
            Self::DeleteSurrounding { before, after } => f
                .debug_struct("DeleteSurrounding")
                .field("before", before)
                .field("after", after)
                .finish(),
        }
    }
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("preedit", &self.preedit)
            .field("commit", &self.commit)
            .field("delete", &self.delete)
            .finish()
    }
}

/// Context'te tutulan IME durumu.
#[derive(Debug, Default)]
pub(crate) struct Ime {
    pub manager: Option<ZwpTextInputManagerV3>,
    pub text_input: Option<ZwpTextInputV3>,
    /// text-input'un kendi `enter`'ıyla bildirilen yüzey.
    pub focus: Option<SurfaceId>,
    enabled: bool,
    /// Uygulamanın `set_text_input` ile verdiği durumlar.
    pub states: HashMap<SurfaceId, TextInputState>,
    pending: Pending,
    /// Odakta preedit gösteriliyor mu (leave'de temizlemek için).
    preedit_active: bool,
    /// xkb düzen adları ve etkin grup.
    pub layouts: Vec<String>,
    pub active_layout: usize,
}

impl Ime {
    /// Odaktaki yüzeyin durumunu compositor'a gönderir (etkinleştirir/kapatır).
    pub fn sync(&mut self) {
        let Some(ti) = self.text_input.as_ref() else {
            return;
        };
        let state = self.focus.and_then(|id| self.states.get(&id));
        match state {
            Some(state) => {
                if !self.enabled {
                    ti.enable();
                    self.enabled = true;
                }
                let plan = SyncPlan::of(state);
                if let Some((text, cursor, anchor)) = plan.surrounding {
                    ti.set_surrounding_text(text.to_owned(), cursor as i32, anchor as i32);
                }
                ti.set_content_type(plan.hint, plan.purpose);
                let r = plan.cursor_rect;
                ti.set_cursor_rectangle(r.x as i32, r.y as i32, r.width as i32, r.height as i32);
                ti.commit();
            }
            None if self.enabled => {
                ti.disable();
                ti.commit();
                self.enabled = false;
            }
            None => {}
        }
    }

    /// Yüzey yok edildi: durumunu ve odağını unut.
    pub fn forget(&mut self, id: SurfaceId) {
        self.states.remove(&id);
        if self.focus == Some(id) {
            self.focus = None;
            self.preedit_active = false;
            self.sync();
        }
    }

    pub fn layout_name(&self) -> Option<&str> {
        self.layouts.get(self.active_layout).map(String::as_str)
    }

    /// Odaktaki yüzeyin bildirdiği alan gizli mi (`TextPurpose::Password`).
    fn secret_focus(&self) -> bool {
        self.focus
            .and_then(|id| self.states.get(&id))
            .is_some_and(|s| s.purpose == TextPurpose::Password)
    }

    /// `preedit_string`: gizli alanda saklanmaz (metin burada düşer ve sıfırlanır).
    fn preedit_string(&mut self, text: SecretText, begin: i32, end: i32) {
        if self.secret_focus() {
            self.pending.preedit = None;
            return;
        }
        self.pending.preedit = Some((truncate_text(text), begin, end));
    }

    /// `done`: bekleyen değişiklikleri protokol sırasıyla olaylara çevirir.
    fn take_events(&mut self) -> Vec<ImeEvent> {
        let mut p = std::mem::take(&mut self.pending);
        if self.secret_focus() {
            // Durum preedit geldikten sonra gizliye dönmüş olabilir: yine de tutulmaz.
            p.preedit = None;
        }
        let mut events = Vec::new();
        if let Some((before, after)) = p.delete
            && (before, after) != (0, 0)
        {
            events.push(ImeEvent::DeleteSurrounding {
                before: before as usize,
                after: after as usize,
            });
        }
        if let Some(text) = p.commit
            && !text.is_empty()
        {
            events.push(ImeEvent::Commit(text));
        }
        // Preedit her `done`'da yenisiyle değişir; gelmediyse boştur.
        let (text, begin, end) = p.preedit.unwrap_or_default();
        if !text.is_empty() || self.preedit_active {
            self.preedit_active = !text.is_empty();
            let cursor = (!text.is_empty() && begin >= 0 && end >= 0)
                .then_some((begin as usize, end as usize))
                .filter(|&(b, e)| {
                    b <= e
                        && e <= text.len()
                        && text.is_char_boundary(b)
                        && text.is_char_boundary(e)
                });
            events.push(ImeEvent::Preedit { text, cursor });
        }
        events
    }
}

/// `sync`'in compositor'a göndereceği istekler (saf; testli).
#[derive(Debug, PartialEq)]
struct SyncPlan<'a> {
    /// `set_surrounding_text` (gizli alanda hiç gönderilmez).
    surrounding: Option<(&'a str, usize, usize)>,
    hint: ContentHint,
    purpose: ContentPurpose,
    cursor_rect: Rect,
}

impl<'a> SyncPlan<'a> {
    fn of(state: &'a TextInputState) -> Self {
        let secret = state.purpose == TextPurpose::Password;
        let surrounding = state
            .surrounding
            .as_ref()
            .filter(|_| !secret)
            .map(|s| clamp_surrounding(&s.text, s.cursor, s.anchor));
        let (hint, purpose) = content_type(state.purpose, state.multiline);
        Self {
            surrounding,
            hint,
            purpose,
            cursor_rect: state.cursor_rect.round_out(),
        }
    }
}

fn content_type(purpose: TextPurpose, multiline: bool) -> (ContentHint, ContentPurpose) {
    let mut hint = ContentHint::None;
    if multiline {
        hint |= ContentHint::Multiline;
    }
    let purpose = match purpose {
        TextPurpose::Normal => ContentPurpose::Normal,
        TextPurpose::Password => {
            hint |= ContentHint::SensitiveData | ContentHint::HiddenText;
            ContentPurpose::Password
        }
        TextPurpose::Email => ContentPurpose::Email,
        TextPurpose::Url => ContentPurpose::Url,
        TextPurpose::Number => ContentPurpose::Number,
        TextPurpose::Phone => ContentPurpose::Phone,
        TextPurpose::Terminal => ContentPurpose::Terminal,
    };
    (hint, purpose)
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Metni imleç/çapa etrafında `MAX_SURROUNDING` bayta indirger; ofsetleri yeni dilime göre verir.
fn clamp_surrounding(text: &str, cursor: usize, anchor: usize) -> (&str, usize, usize) {
    let cursor = floor_boundary(text, cursor);
    let anchor = floor_boundary(text, anchor);
    if text.len() <= MAX_SURROUNDING {
        return (text, cursor, anchor);
    }
    let (lo, hi) = (cursor.min(anchor), cursor.max(anchor));
    // Seçimin kendisi sığmıyorsa imleç tarafını tut.
    let (lo, hi) = if hi - lo > MAX_SURROUNDING {
        (cursor, cursor)
    } else {
        (lo, hi)
    };
    let spare = MAX_SURROUNDING - (hi - lo);
    let mut start = lo.saturating_sub(spare / 2);
    let end = (start + MAX_SURROUNDING).min(text.len());
    start = end.saturating_sub(MAX_SURROUNDING).min(start);
    let start = ceil_boundary(text, start);
    let end = floor_boundary(text, end);
    let clamp = |i: usize| i.clamp(start, end) - start;
    (&text[start..end], clamp(cursor), clamp(anchor))
}

/// `MAX_IME_TEXT` bayta keser; atılan kuyruk sıfırlanır.
fn truncate_text(mut text: SecretText) -> SecretText {
    text.truncate_zeroized(MAX_IME_TEXT);
    text
}

/// xkb keymap metninden düzen adları (`name[Group1]="Turkish";` ya da `name[1]=…`), grup sırasıyla.
pub(crate) fn parse_layout_names(keymap: &str) -> Vec<String> {
    let mut names: Vec<(usize, String)> = Vec::new();
    for line in keymap.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("name[") else {
            continue;
        };
        let Some((group, value)) = rest.split_once(']') else {
            continue;
        };
        let Some(index) = group
            .strip_prefix("Group")
            .or_else(|| group.strip_prefix("group"))
            // Yeni libxkbcommon (1.13.2’de gözlendi) grubu yalnızca sayıyla yazar: `name[1]="Turkish";`.
            .or(Some(group))
            .and_then(|n| n.parse::<usize>().ok())
        else {
            continue;
        };
        let Some(start) = value.find('"') else {
            continue;
        };
        let Some(len) = value[start + 1..].find('"') else {
            continue;
        };
        let name = &value[start + 1..start + 1 + len];
        if index >= 1 && !names.iter().any(|(i, _)| *i == index) {
            names.push((index, name.to_owned()));
        }
    }
    names.sort_by_key(|(i, _)| *i);
    names.into_iter().map(|(_, n)| n).collect()
}

/// `zwp_text_input_v3` kullanıcı verisi.
#[derive(Debug)]
pub(crate) struct TextInputData;

impl<A: App> Dispatch2<ZwpTextInputV3, State<A>> for TextInputData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ZwpTextInputV3,
        event: zwp_text_input_v3::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        use zwp_text_input_v3::Event;
        let ime = &mut state.cx.ime;
        match event {
            Event::Enter { surface } => {
                let id = surface.data::<SurfaceData<SurfaceId>>().map(|d| *d.data());
                ime.focus = id.filter(|id| state.cx.surfaces.contains_key(id));
                ime.preedit_active = false;
                ime.sync();
            }
            Event::Leave { .. } => {
                let had_preedit = std::mem::take(&mut ime.preedit_active);
                let old = ime.focus.take();
                ime.pending = Pending::default();
                ime.sync();
                // Yarım kalan preedit'i uygulamaya temizlet.
                if had_preedit && let Some(id) = old.filter(|id| state.cx.surfaces.contains_key(id))
                {
                    state.app.ime(
                        &mut state.cx,
                        id,
                        ImeEvent::Preedit {
                            text: SecretText::new(),
                            cursor: None,
                        },
                    );
                }
            }
            // Protokolden gelen `String` kopyalanmadan SecretText'e taşınır.
            Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                let text = SecretText::from(text.unwrap_or_default());
                ime.preedit_string(text, cursor_begin, cursor_end);
            }
            Event::CommitString { text } => {
                let text = SecretText::from(text.unwrap_or_default());
                ime.pending.commit = Some(truncate_text(text));
            }
            Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                ime.pending.delete = Some((before_length, after_length));
            }
            Event::Done { .. } => {
                let events = ime.take_events();
                let focus = ime.focus;
                let Some(id) = focus
                    .filter(|id| state.cx.surfaces.contains_key(id) && state.cx.input_allowed(*id))
                else {
                    return;
                };
                for event in events {
                    state.app.ime(&mut state.cx, id, event);
                }
            }
            _ => {}
        }
    }
}

impl<D> Dispatch2<ZwpTextInputManagerV3, D> for crate::state::NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &ZwpTextInputManagerV3,
        _: <ZwpTextInputManagerV3 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_names_from_keymap() {
        let keymap = "xkb_keymap {\nxkb_keycodes \"evdev\" { };\nxkb_types \"x\" { level_name[Level1]=\"Base\"; };\n\
                      xkb_symbols \"pc+tr+us:2\" {\n\tname[Group1]=\"Turkish\";\n\tname[Group2]=\"English (US)\";\n};\n};";
        assert_eq!(parse_layout_names(keymap), vec!["Turkish", "English (US)"]);
        assert!(parse_layout_names("").is_empty());
        let new_format =
            "xkb_symbols \"pc_tr_us_2\" {\n\tname[1]=\"Turkish\";\n\tname[2]=\"English (US)\";\n};";
        assert_eq!(
            parse_layout_names(new_format),
            vec!["Turkish", "English (US)"]
        );
    }

    #[test]
    fn surrounding_is_clamped_around_cursor() {
        let short = "merhaba";
        assert_eq!(clamp_surrounding(short, 3, 3), (short, 3, 3));
        let long = "ş".repeat(3000); // 6000 bayt
        let (slice, c, a) = clamp_surrounding(&long, 3000, 3000);
        assert!(slice.len() <= MAX_SURROUNDING);
        assert_eq!(c, a);
        assert!(slice.is_char_boundary(c));
        assert_eq!(&slice[..c], &long[3000 - c..3000]);
        // Çok baytlı karakterin ortası aşağı yuvarlanır.
        assert_eq!(clamp_surrounding("aş", 2, 2).1, 1);
    }

    #[test]
    fn done_orders_events_and_clears_preedit() {
        let mut ime = Ime::default();
        ime.pending.delete = Some((2, 0));
        ime.pending.commit = Some("ğ".into());
        ime.pending.preedit = Some(("ab".into(), 1, 1));
        assert_eq!(
            ime.take_events(),
            vec![
                ImeEvent::DeleteSurrounding {
                    before: 2,
                    after: 0
                },
                ImeEvent::Commit("ğ".into()),
                ImeEvent::Preedit {
                    text: "ab".into(),
                    cursor: Some((1, 1))
                },
            ]
        );
        // Sonraki done'da preedit yoksa: boş preedit ile temizlenir, sonra sessiz.
        assert_eq!(
            ime.take_events(),
            vec![ImeEvent::Preedit {
                text: SecretText::new(),
                cursor: None
            }]
        );
        assert!(ime.take_events().is_empty());
        assert_eq!(
            truncate_text(SecretText::from("ş".repeat(3000))).len(),
            MAX_IME_TEXT
        );
    }

    #[test]
    fn password_hints() {
        let (hint, purpose) = content_type(TextPurpose::Password, false);
        assert!(hint.contains(ContentHint::HiddenText | ContentHint::SensitiveData));
        assert_eq!(purpose, ContentPurpose::Password);
    }

    fn state(purpose: TextPurpose) -> TextInputState {
        TextInputState {
            surrounding: Some(SurroundingText {
                text: "çevre".into(),
                cursor: 2,
                anchor: 2,
            }),
            purpose,
            ..Default::default()
        }
    }

    #[test]
    fn sync_plan_for_secret_field() {
        // Gizli alan: purpose=password + sensitive_data|hidden_text, çevre metin gönderilmez.
        let secret = state(TextPurpose::Password);
        let plan = SyncPlan::of(&secret);
        assert_eq!(plan.purpose, ContentPurpose::Password);
        assert!(plan.hint.contains(ContentHint::SensitiveData | ContentHint::HiddenText));
        assert_eq!(plan.surrounding, None);
        // Normal alan değişmedi: çevre metin gider, hassas ipucu yok.
        let normal = state(TextPurpose::Normal);
        let plan = SyncPlan::of(&normal);
        assert_eq!(plan.purpose, ContentPurpose::Normal);
        assert!(!plan.hint.intersects(ContentHint::SensitiveData | ContentHint::HiddenText));
        assert_eq!(plan.surrounding, Some(("çevre", 2, 2)));
    }

    fn focused(purpose: TextPurpose) -> Ime {
        let id = SurfaceId(1);
        let mut ime = Ime {
            focus: Some(id),
            ..Default::default()
        };
        ime.states.insert(id, state(purpose));
        ime
    }

    #[test]
    fn secret_field_never_keeps_preedit() {
        let mut ime = focused(TextPurpose::Password);
        ime.preedit_string("gizli".into(), 0, 5);
        assert!(ime.pending.preedit.is_none(), "preedit saklanmadı");
        ime.pending.commit = Some("ş".into());
        // Yalnızca commit çıkar; preedit olayı yok, preedit etkin sayılmaz.
        assert_eq!(ime.take_events(), vec![ImeEvent::Commit("ş".into())]);
        assert!(!ime.preedit_active);
        assert!(ime.take_events().is_empty());
    }

    #[test]
    fn preedit_shown_before_field_turned_secret_is_cleared() {
        let mut ime = focused(TextPurpose::Normal);
        ime.preedit_string("ab".into(), 1, 1);
        assert_eq!(ime.take_events().len(), 1);
        assert!(ime.preedit_active);
        // Alan gizliye döndü; o arada gelen preedit tutulmaz, gösterilen boşla temizlenir.
        ime.states.insert(SurfaceId(1), state(TextPurpose::Password));
        ime.pending.preedit = Some(("cd".into(), 0, 0));
        assert_eq!(
            ime.take_events(),
            vec![ImeEvent::Preedit {
                text: SecretText::new(),
                cursor: None
            }]
        );
        assert!(!ime.preedit_active);
    }

    #[test]
    fn normal_field_preedit_unchanged() {
        let mut ime = focused(TextPurpose::Normal);
        ime.preedit_string("にほ".into(), 0, 6);
        assert_eq!(
            ime.take_events(),
            vec![ImeEvent::Preedit {
                text: "にほ".into(),
                cursor: Some((0, 6))
            }]
        );
    }

    #[test]
    fn debug_redacts_user_text() {
        let secret = "gizli-parola";
        let outputs = [
            format!(
                "{:?}",
                SurroundingText {
                    text: secret.into(),
                    cursor: 3,
                    anchor: 3,
                }
            ),
            format!(
                "{:?}",
                ImeEvent::Preedit {
                    text: secret.into(),
                    cursor: Some((0, 1)),
                }
            ),
            format!("{:?}", ImeEvent::Commit(secret.into())),
            format!(
                "{:?}",
                Pending {
                    preedit: Some((secret.into(), 0, 0)),
                    commit: Some(secret.into()),
                    delete: None,
                }
            ),
            format!(
                "{:?}",
                TextInputState {
                    surrounding: Some(SurroundingText {
                        text: secret.into(),
                        cursor: 0,
                        anchor: 0,
                    }),
                    purpose: TextPurpose::Password,
                    ..Default::default()
                }
            ),
        ];
        for out in &outputs {
            assert!(!out.contains(secret), "sızıntı: {out}");
            assert!(out.contains("12 karakter gizli"), "{out}");
        }
    }
}
