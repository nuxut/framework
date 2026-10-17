//! Klavye eşlemi (keymap) gelmeden önceki odak ve tuşlar.
//!
//! sctk 0.21 `xkb_state` yokken gelen `wl_keyboard.enter`'ı ve tuşları sessizce yutar
//! (`seat/keyboard/mod.rs`); compositor keymap'i ilk tuşa kadar göndermezse odak hiç kurulmaz ve
//! kilit ekranı tuş alamaz. Çözüm: aynı koltuktan ikinci, yalnızca gözlemci bir `wl_keyboard`
//! (`RawKeyboard`) ham enter/leave/tuşları izler. Keymap gelince yutulan odak yeniden uygulanır;
//! keymap hiç gelmeden tuş basılırsa sctk'ye yerleşik yedek keymap (`us`) verilir ve yutulan
//! tuşlar yeniden oynatılır — yalnızca basıldıkları yüzey hâlâ odaktaysa ve girdi kabul ediyorsa
//! (odak değişince bekleyen tuşlar silinir; parola başka yüzeye gitmez). Mantık `FocusGate`'te
//! (testli); protokol kısmı `state.rs`'te.

use std::io::Write;
use std::os::fd::OwnedFd;

use rustix::fs::{MemfdFlags, SealFlags, fcntl_add_seals, memfd_create};

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::seat::keyboard::KeyboardData;
use wayland_client::protocol::wl_keyboard::{self, KeymapFormat, WlKeyboard};
use wayland_client::{Connection, Proxy, QueueHandle, WEnum};
use xkbcommon::xkb;
use zeroize::{Zeroize, Zeroizing};

use crate::App;
use crate::state::State;
use crate::types::SurfaceId;

/// Compositor keymap göndermezse kullanılan düzen (en azından ASCII parola yazılabilsin).
pub(crate) const FALLBACK_LAYOUT: &str = "us";

/// Keymap yokken gelen ham tuş; sctk bunu yutmuş olabilir.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct RawKey {
    pub serial: u32,
    pub time: u32,
    pub key: u32,
    pub pressed: bool,
}

// Tuş kodu parolayı açığa çıkarır: biriken kuyruk bırakılırken alanlar sıfırlanır.
impl Zeroize for RawKey {
    fn zeroize(&mut self) {
        self.serial.zeroize();
        self.time.zeroize();
        self.key.zeroize();
        self.pressed.zeroize();
    }
}

// Tuş kodu kullanıcı girdisidir (parola): Debug'da gizlenir (değişmez 9).
impl std::fmt::Debug for RawKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawKey")
            .field("serial", &self.serial)
            .field("pressed", &self.pressed)
            .finish_non_exhaustive()
    }
}

/// Ham (gözlemci klavyedeki) odak ile sctk'nin teslim ettiği odak arasındaki uzlaştırma.
#[derive(Debug, Default)]
pub(crate) struct FocusGate {
    /// sctk'nin klavyesi geçerli bir keymap aldı (`update_keymap`).
    pub has_keymap: bool,
    raw_focus: Option<SurfaceId>,
    /// Keymap yokken gelen tuşlar (sırayla), basıldıkları anki ham odakla. Odak değişince
    /// silinir: bir yüzeye yazılan tuş (parola) asla başka yüzeye oynatılmaz.
    unkeyed: Vec<(SurfaceId, RawKey)>,
    /// sctk yolundan geçen tuş serial'leri (yalnızca `unkeyed` boş değilken tutulur).
    seen: Vec<u32>,
    fallback_used: bool,
}

/// Keymap'siz tuş birikimi sınırı (yedek keymap ilk tuşta kurulur; bu yalnızca bir güvenlik).
const MAX_UNKEYED: usize = 64;

impl FocusGate {
    pub fn raw_enter(&mut self, id: Option<SurfaceId>) {
        // Protokol enter'dan önce leave gönderir; göndermese de başka yüzeyin tuşları silinir.
        if id != self.raw_focus {
            self.drop_pending();
        }
        self.raw_focus = id;
    }

    pub fn raw_leave(&mut self) {
        self.raw_focus = None;
        self.drop_pending();
    }

    fn drop_pending(&mut self) {
        self.wipe_unkeyed();
        self.seen.clear();
    }

    /// Biriken ham tuşları yerinde sıfırlayıp boşaltır (tampon serbest bırakılmadan önce).
    fn wipe_unkeyed(&mut self) {
        self.zero_unkeyed();
        self.unkeyed.clear();
    }

    fn zero_unkeyed(&mut self) {
        for (_, key) in &mut self.unkeyed {
            key.zeroize();
        }
    }

    /// Ham tuş geldi. Keymap yoksa odaktaki yüzeyle saklanır (odak yoksa atılır); yedek keymap
    /// kurulmalıysa `true` (bir kez).
    pub fn raw_key(&mut self, key: RawKey) -> bool {
        if self.has_keymap {
            return false;
        }
        if let Some(id) = self.raw_focus
            && self.unkeyed.len() < MAX_UNKEYED
        {
            self.unkeyed.push((id, key));
        }
        !std::mem::replace(&mut self.fallback_used, true)
    }

    /// sctk bu tuşu işledi (`press_key`/`release_key`).
    pub fn seen_key(&mut self, serial: u32) {
        if !self.unkeyed.is_empty() {
            self.seen.push(serial);
        }
    }

    /// Keymap varken ham odak sctk'nin teslim ettiğinden farklıysa yeniden uygulanacak yüzey.
    /// `alive`: yüzey hâlâ var mı.
    pub fn focus_to_replay(
        &self,
        delivered: Option<SurfaceId>,
        alive: impl Fn(SurfaceId) -> bool,
    ) -> Option<SurfaceId> {
        let raw = self.raw_focus.filter(|id| alive(*id))?;
        (self.has_keymap && delivered != Some(raw)).then_some(raw)
    }

    /// Keymap kurulduktan sonra sctk'nin hiç işlemediği (yuttuğu) tuşlar; kuyruk her durumda
    /// boşaltılır. Yalnızca basıldığı yüzey hem ham odakta hem sctk'nin teslim ettiği odakta
    /// (`delivered`) olan ve `allowed` (yaşıyor + girdi kabul ediyor) tuşlar döner; gerisi atılır.
    pub fn take_swallowed(
        &mut self,
        delivered: Option<SurfaceId>,
        allowed: impl Fn(SurfaceId) -> bool,
    ) -> Zeroizing<Vec<RawKey>> {
        let mut out = Zeroizing::new(Vec::new());
        if !self.has_keymap || self.unkeyed.is_empty() {
            return out;
        }
        let seen = std::mem::take(&mut self.seen);
        if let Some(target) =
            delivered.filter(|id| self.raw_focus == Some(*id) && allowed(*id))
        {
            out.extend(
                self.unkeyed
                    .iter()
                    .filter(|(id, k)| *id == target && !seen.contains(&k.serial))
                    .map(|(_, k)| *k),
            );
        }
        // Kuyruk her durumda yerinde sıfırlanır; dönen kopya da düşerken sıfırlanır.
        self.wipe_unkeyed();
        out
    }

    /// Klavye kalktı: her şey sıfırlanır (yedek keymap uyarısı yeniden verilebilir).
    pub fn reset(&mut self) {
        self.wipe_unkeyed();
        *self = Self::default();
    }
}

/// Yerleşik yedek keymap'i (`FALLBACK_LAYOUT`) sctk'nin okuyacağı biçimde (fd + NUL dahil boyut)
/// hazırlar. Sistemde xkeyboard-config yoksa `None`.
pub(crate) fn fallback_keymap() -> Option<(OwnedFd, u32)> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = xkb::Keymap::new_from_names(
        &context,
        "",
        "",
        FALLBACK_LAYOUT,
        "",
        None,
        xkb::COMPILE_NO_FLAGS,
    )?;
    let mut text = keymap
        .get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
        .into_bytes();
    text.push(0);
    let size = u32::try_from(text.len()).ok()?;
    // Bellekte adsız dosya (memfd): dosya sisteminde iz yok; yazılıp mühürlenir.
    let fd = memfd_create(
        "nuxut-keymap",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )
    .ok()?;
    let mut file = std::fs::File::from(fd);
    file.write_all(&text).ok()?;
    fcntl_add_seals(
        &file,
        SealFlags::SHRINK | SealFlags::GROW | SealFlags::WRITE | SealFlags::SEAL,
    )
    .ok()?;
    Some((file.into(), size))
}

/// Gözlemci klavyenin kullanıcı verisi. Olayları yalnızca kaydeder; uygulamaya hiçbir şey
/// doğrudan gitmez (teslim her zaman sctk'nin klavyesi üzerinden).
#[derive(Debug)]
pub(crate) struct RawKeyboard;

impl<A: App> Dispatch2<WlKeyboard, State<A>> for RawKeyboard {
    fn event(
        &self,
        state: &mut State<A>,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        conn: &Connection,
        qh: &QueueHandle<State<A>>,
    ) {
        match event {
            wl_keyboard::Event::Enter { surface, .. } => state.raw_keyboard_enter(&surface),
            wl_keyboard::Event::Leave { .. } => state.focus_gate.raw_leave(),
            wl_keyboard::Event::Key {
                serial,
                time,
                key,
                state: key_state,
            } => {
                let pressed = key_state == WEnum::Value(wl_keyboard::KeyState::Pressed);
                let raw = RawKey {
                    serial,
                    time,
                    key,
                    pressed,
                };
                if state.focus_gate.raw_key(raw) {
                    state.install_fallback_keymap(conn, qh);
                }
            }
            _ => {}
        }
    }
}

/// sctk'nin klavyesine sentetik bir olay verir (yedek keymap ya da yutulan tuşun yeniden
/// oynatılması). sctk klavyesi yoksa ya da veri türü beklenmedikse no-op.
pub(crate) fn feed_sctk<A: App>(
    state: &mut State<A>,
    keyboard: &WlKeyboard,
    event: wl_keyboard::Event,
    conn: &Connection,
    qh: &QueueHandle<State<A>>,
) {
    if let Some(data) = keyboard.data::<KeyboardData<State<A>, ()>>() {
        data.event(state, keyboard, event, conn, qh);
    }
}

pub(crate) fn keymap_event(fd: OwnedFd, size: u32) -> wl_keyboard::Event {
    wl_keyboard::Event::Keymap {
        format: WEnum::Value(KeymapFormat::XkbV1),
        fd,
        size,
    }
}

pub(crate) fn key_event(key: RawKey) -> wl_keyboard::Event {
    wl_keyboard::Event::Key {
        serial: key.serial,
        time: key.time,
        key: key.key,
        state: WEnum::Value(if key.pressed {
            wl_keyboard::KeyState::Pressed
        } else {
            wl_keyboard::KeyState::Released
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(serial: u32) -> RawKey {
        RawKey {
            serial,
            time: 0,
            key: 30,
            pressed: true,
        }
    }

    #[test]
    fn swallowed_enter_is_replayed_once_keymap_arrives() {
        let mut gate = FocusGate::default();
        let s = SurfaceId(7);
        gate.raw_enter(Some(s));
        // Keymap yokken sctk teslim etmedi: henüz yeniden uygulanmaz.
        assert_eq!(gate.focus_to_replay(None, |_| true), None);
        gate.has_keymap = true;
        assert_eq!(gate.focus_to_replay(None, |_| true), Some(s));
        // Teslim edildiyse ya da yüzey öldüyse yok.
        assert_eq!(gate.focus_to_replay(Some(s), |_| true), None);
        assert_eq!(gate.focus_to_replay(None, |_| false), None);
        gate.raw_leave();
        assert_eq!(gate.focus_to_replay(None, |_| true), None);
    }

    #[test]
    fn normal_path_needs_no_replay() {
        // Keymap önce geldi: sctk enter'ı teslim eder, kapı hiçbir şey yapmaz.
        let mut gate = FocusGate {
            has_keymap: true,
            ..Default::default()
        };
        let s = SurfaceId(1);
        gate.raw_enter(Some(s));
        assert_eq!(gate.focus_to_replay(Some(s), |_| true), None);
        assert!(!gate.raw_key(key(1)));
        assert!(gate.take_swallowed(Some(s), |_| true).is_empty());
    }

    #[test]
    fn unkeyed_keycodes_are_zeroed_before_release() {
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(SurfaceId(1)));
        gate.raw_key(key(1));
        gate.raw_key(key(2));
        // Boşaltmadan önceki yerinde sıfırlama: tuş kodları (parola) tamponda kalmaz.
        gate.zero_unkeyed();
        assert_eq!(gate.unkeyed.len(), 2);
        assert!(gate.unkeyed.iter().all(|(_, k)| k.key == 0 && k.serial == 0 && !k.pressed));
        gate.raw_leave();
        assert!(gate.unkeyed.is_empty());
        let mut k = key(9);
        k.zeroize();
        assert_eq!(k, RawKey { serial: 0, time: 0, key: 0, pressed: false });
    }

    #[test]
    fn first_unkeyed_key_requests_fallback_once() {
        let mut gate = FocusGate::default();
        assert!(gate.raw_key(key(1)));
        assert!(!gate.raw_key(key(2)));
    }

    fn serials(keys: &[RawKey]) -> Vec<u32> {
        keys.iter().map(|k| k.serial).collect()
    }

    #[test]
    fn only_keys_sctk_did_not_see_are_replayed() {
        // Gözlemci önce: yedek kurulur, sctk 1'i kendisi işler; 2'yi yutmuştu.
        let mut gate = FocusGate::default();
        let s = SurfaceId(3);
        gate.raw_enter(Some(s));
        gate.raw_key(key(2));
        gate.raw_key(key(1));
        gate.has_keymap = true;
        gate.seen_key(1);
        assert_eq!(serials(&gate.take_swallowed(Some(s), |_| true)), vec![2]);
        assert!(gate.take_swallowed(Some(s), |_| true).is_empty());
    }

    #[test]
    fn keys_never_replayed_to_another_surface() {
        let (a, b) = (SurfaceId(1), SurfaceId(2));

        // A'ya yazıldı, aynı turda odak B'ye geçti (leave + enter): A'nın tuşları silinir.
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(a));
        gate.raw_key(key(1));
        gate.raw_leave();
        gate.raw_enter(Some(b));
        gate.has_keymap = true;
        assert!(gate.take_swallowed(Some(b), |_| true).is_empty());

        // Leave'siz doğrudan enter (protokol dışı) da siler.
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(a));
        gate.raw_key(key(1));
        gate.raw_enter(Some(b));
        gate.has_keymap = true;
        assert!(gate.take_swallowed(Some(b), |_| true).is_empty());

        // Ham odak A ama teslim edilen odak B: oynatılmaz, kuyruk yine boşalır.
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(a));
        gate.raw_key(key(1));
        gate.has_keymap = true;
        assert!(gate.take_swallowed(Some(b), |_| true).is_empty());
        assert!(gate.take_swallowed(Some(a), |_| true).is_empty());

        // Teslim edilen odak yok: oynatılmaz.
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(a));
        gate.raw_key(key(1));
        gate.has_keymap = true;
        assert!(gate.take_swallowed(None, |_| true).is_empty());

        // Odak yokken gelen tuş saklanmaz (yedek keymap yine istenir).
        let mut gate = FocusGate::default();
        assert!(gate.raw_key(key(1)));
        gate.raw_enter(Some(a));
        gate.has_keymap = true;
        assert!(gate.take_swallowed(Some(a), |_| true).is_empty());
    }

    #[test]
    fn keys_dropped_when_surface_no_longer_accepts_input() {
        // Ör. tuşlar basıldıktan sonra oturum kilidi istendi: kilit dışı yüzeye oynatılmaz.
        let a = SurfaceId(1);
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(a));
        gate.raw_key(key(1));
        gate.has_keymap = true;
        assert!(gate.take_swallowed(Some(a), |_| false).is_empty());
        assert!(gate.take_swallowed(Some(a), |_| true).is_empty());
    }

    #[test]
    fn leave_drops_pending_keys() {
        let a = SurfaceId(1);
        let mut gate = FocusGate::default();
        gate.raw_enter(Some(a));
        gate.raw_key(key(1));
        gate.raw_leave();
        gate.raw_enter(Some(a));
        gate.raw_key(key(2));
        gate.has_keymap = true;
        assert_eq!(serials(&gate.take_swallowed(Some(a), |_| true)), vec![2]);
    }

    #[test]
    fn raw_key_debug_hides_keycode() {
        let out = format!("{:?}", key(5));
        assert!(!out.contains("30"), "{out}");
    }

    #[test]
    fn fallback_keymap_compiles() {
        // xkeyboard-config kurulu değilse (CI) atlanır.
        if let Some((fd, size)) = fallback_keymap() {
            assert!(size > 1000);
            // memfd: dosya sisteminde adı yok, boyutu sabit (mühürlü).
            let file = std::fs::File::from(fd);
            assert_eq!(file.metadata().unwrap().len(), u64::from(size));
            assert!(file.set_len(1).is_err());
        }
    }
}
