//! Tuş olaylarının XKB ayrıntısı ([`XkbKey`]): üretilen keysym'in yanında aynı tuşun
//! değiştiricisiz (seviye 0) keysym'leri ve adları — kısayol yakalama için.
//!
//! sctk yalnızca üretilen keysym'i verir ve kendi `xkb_state`'ini açmaz; bu yüzden compositor'ın
//! keymap'i (sctk'nin `update_keymap`'te verdiği metin) burada ikinci kez derlenir — keymap
//! değişiminde bir kez, birkaç ms. Hyprland'in bind çözümüyle birebir: iki değiştiricisiz durum,
//! biri etkin düzende (`xkb_state_update_mask(0,0,0,0,0,group)`, `resolve_binds_by_sym`),
//! biri ilk düzende (yeni durum, grup 0; varsayılan). Tek simge üretmeyen tuşta `NoSymbol`
//! (Hyprland da `xkb_state_key_get_one_sym` kullanır).

use nuxut_core::input::{NamedKeysym, XkbKey};
use xkbcommon::xkb;

/// Derlenmiş keymap'in değiştiricisiz durumları.
pub(crate) struct KeysymResolver {
    /// Etkin düzen (grup) uygulanmış, değiştiricisiz.
    active: xkb::State,
    /// İlk düzen (grup 0), değiştiricisiz.
    first: xkb::State,
    layout: u32,
}

impl KeysymResolver {
    /// XKB metin keymap'inden. Derlenemezse `None` (olaylar `xkb: None` taşır).
    pub fn from_keymap_text(text: &str) -> Option<Self> {
        // Metin keymap tümüyle çözülmüştür: include yolu ve ortam değişkeni gerekmez.
        let context = xkb::Context::new(
            xkb::CONTEXT_NO_DEFAULT_INCLUDES | xkb::CONTEXT_NO_ENVIRONMENT_NAMES,
        );
        let keymap = xkb::Keymap::new_from_string(
            &context,
            text.to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::COMPILE_NO_FLAGS,
        )?;
        Some(Self {
            active: xkb::State::new(&keymap),
            first: xkb::State::new(&keymap),
            layout: 0,
        })
    }

    /// Etkin düzen değişti (`wl_keyboard.modifiers`'ın `group`'u).
    pub fn set_layout(&mut self, layout: u32) {
        if layout != self.layout {
            self.layout = layout;
            self.active.update_mask(0, 0, 0, 0, 0, layout);
        }
    }

    /// `evdev`: `wl_keyboard.key` kodu; `produced`: geçerli durumla üretilen keysym.
    pub fn resolve(&self, evdev: u32, produced: u32) -> XkbKey {
        let keycode = evdev.saturating_add(8);
        let code = xkb::Keycode::new(keycode);
        XkbKey {
            keycode,
            sym: named(produced),
            base: named(self.active.key_get_one_sym(code).raw()),
            first_layout_base: named(self.first.key_get_one_sym(code).raw()),
        }
    }
}

fn named(raw: u32) -> NamedKeysym {
    NamedKeysym { raw, name: xkb::keysym_get_name(xkb::Keysym::new(raw)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // evdev kodları (input-event-codes.h).
    const KEY_1: u32 = 2;
    const KEY_I: u32 = 23; // Türkçe Q: ı / I
    const KEY_APOSTROPHE: u32 = 40; // Türkçe Q: i / İ
    const KEY_COMMA: u32 = 51; // Türkçe Q: ö / Ö
    const KEY_SLASH: u32 = 53; // Türkçe Q: . / :
    const KEY_MINUS: u32 = 12; // Türkçe Q: * / ?
    const KEY_ENTER: u32 = 28;
    const KEY_A: u32 = 30;

    /// Gerçek yoldaki gibi: RMLVO'dan derlenip metne dökülen keymap. xkeyboard-config yoksa
    /// `None` (test atlanır).
    fn keymap_text(layout: &str) -> Option<String> {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap =
            xkb::Keymap::new_from_names(&context, "", "", layout, "", None, xkb::COMPILE_NO_FLAGS)?;
        Some(keymap.get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1))
    }

    /// Tam (değiştiricili) durumla üretilen keysym: sctk'nin `KeyEvent::keysym`'i.
    fn produced(text: &str, evdev: u32, shift: bool, layout: u32) -> u32 {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_string(
            &context,
            text.to_owned(),
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::COMPILE_NO_FLAGS,
        )
        .unwrap();
        let mut state = xkb::State::new(&keymap);
        let shift_mask = if shift { 1 << keymap.mod_get_index(xkb::MOD_NAME_SHIFT) } else { 0 };
        state.update_mask(shift_mask, 0, 0, 0, 0, layout);
        state.key_get_one_sym(xkb::Keycode::new(evdev + 8)).raw()
    }

    fn names(k: &XkbKey) -> (&str, &str, &str) {
        (&k.sym.name, &k.base.name, &k.first_layout_base.name)
    }

    #[test]
    fn turkish_q_dotted_and_dotless_i() {
        let Some(text) = keymap_text("tr") else { return };
        let r = KeysymResolver::from_keymap_text(&text).unwrap();
        let k = |code, shift| r.resolve(code, produced(&text, code, shift, 0));

        let dotless = k(KEY_I, false);
        assert_eq!(names(&dotless), ("idotless", "idotless", "idotless"));
        assert_eq!(dotless.keycode, KEY_I + 8);
        // Shift+ı → I üretir ama bind'i `SHIFT, idotless` (seviye 0).
        assert_eq!(names(&k(KEY_I, true)), ("I", "idotless", "idotless"));
        assert_eq!(names(&k(KEY_APOSTROPHE, false)), ("i", "i", "i"));
        assert_eq!(names(&k(KEY_APOSTROPHE, true)), ("Iabovedot", "i", "i"));
        assert_eq!(names(&k(KEY_COMMA, true)), ("Odiaeresis", "odiaeresis", "odiaeresis"));
        // Adlar xkb_keysym_from_name ile geri çözülür (Hyprland'in yaptığı).
        for name in ["idotless", "Iabovedot", "odiaeresis"] {
            assert_ne!(xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS).raw(), 0, "{name}");
        }
    }

    #[test]
    fn turkish_q_shifted_digits_and_punctuation() {
        let Some(text) = keymap_text("tr") else { return };
        let r = KeysymResolver::from_keymap_text(&text).unwrap();
        let k = |code, shift| r.resolve(code, produced(&text, code, shift, 0));

        assert_eq!(names(&k(KEY_1, false)), ("1", "1", "1"));
        assert_eq!(names(&k(KEY_1, true)), ("exclam", "1", "1"));
        assert_eq!(k(KEY_1, true).hyprland(false).raw, u32::from('1'));
        assert_eq!(names(&k(KEY_SLASH, false)), ("period", "period", "period"));
        assert_eq!(names(&k(KEY_SLASH, true)), ("colon", "period", "period"));
        assert_eq!(names(&k(KEY_MINUS, true)), ("question", "asterisk", "asterisk"));
        assert_eq!(names(&k(KEY_ENTER, false)), ("Return", "Return", "Return"));
    }

    #[test]
    fn second_layout_base_follows_active_group_first_stays() {
        // us,tr: grup 1 (tr) etkinken `base` Türkçe, `first_layout_base` ABD düzenidir.
        let Some(text) = keymap_text("us,tr") else { return };
        let mut r = KeysymResolver::from_keymap_text(&text).unwrap();
        r.set_layout(1);
        let dotless = r.resolve(KEY_I, produced(&text, KEY_I, true, 1));
        assert_eq!(names(&dotless), ("I", "idotless", "i"));
        assert_eq!(dotless.hyprland(true).name, "idotless");
        assert_eq!(dotless.hyprland(false).name, "i");
        r.set_layout(0);
        assert_eq!(names(&r.resolve(KEY_A, u32::from('a'))), ("a", "a", "a"));
        assert_eq!(r.resolve(KEY_I, u32::from('i')).base.name, "i");
    }

    #[test]
    fn invalid_keymap_and_unknown_keys() {
        assert!(KeysymResolver::from_keymap_text("bozuk").is_none());
        let Some(text) = keymap_text("us") else { return };
        let r = KeysymResolver::from_keymap_text(&text).unwrap();
        // Keymap'te olmayan kod: NoSymbol (panik yok, taşma yok).
        let k = r.resolve(u32::MAX, 0);
        assert!(k.base.is_none() && k.first_layout_base.is_none());
        assert_eq!(k.base.name, "NoSymbol");
        assert_eq!(k.keycode, u32::MAX);
    }
}
