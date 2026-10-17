//! Test compositor'ı için sanal işaretçi (`zwlr_virtual_pointer_v1`). Yalnızca
//! `WAYLAND_DISPLAY`'in gösterdiği compositor'a gider; `tools/test-compositor.sh` bunu test
//! display'iyle `serve` modunda başlatır. Kullanıcının oturumunda çalıştırmayın.
//!
//! Sanal aygıt her komutta yaratılıp yok edilirse istemciler işaretçi yeteneğini kaybeder
//! (enter gelmez); bu yüzden `serve` tek bir aygıtı açık tutar ve FIFO'dan satır okur:
//!
//! ```text
//! move X Y W H          # düzen koordinatı (X, Y), düzen boyutu (W, H)
//! click [left|right|middle]
//! press|release [düğme]
//! scroll DY             # piksel; pozitif aşağı
//! wheel N               # tekerlek çentiği (kaynak Wheel); pozitif aşağı
//! lock caps|num on|off    # kalıcı klavyenin kilit durumu
//! key KOMBİNASYON         # kalıcı klavyeden bas-bırak: ör. `ctrl+v`, `Return`, `shift+Tab`
//! ```
//!
//! `key` adları xkb keysym adlarıdır (`a`, `Return`, `Escape`…) ve `ctrl`/`shift`/`alt`/
//! `super`; keymap `NUXUT_TEST_KEYMAP`'ten. Tuş içeriği hiçbir hata/logda yazılmaz.
//!
//! Güvenlik: Hyprland'e (kullanıcının oturumu) bağlanırsa ya da `WAYLAND_DISPLAY`
//! `NUXUT_USER_WAYLAND_DISPLAY` ile aynıysa hiçbir şey göndermeden çıkar.

use std::io::BufRead;
use std::os::fd::AsFd;
use std::time::Instant;

use smithay_client_toolkit::reexports::protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
    zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
};
use smithay_client_toolkit::reexports::protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
use xkbcommon::xkb;

struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore WlSeat);
delegate_noop!(State: ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ZwlrVirtualPointerV1);
delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
delegate_noop!(State: ZwpVirtualKeyboardV1);

fn button(name: Option<&str>) -> Option<u32> {
    match name {
        None | Some("left") => Some(0x110),
        Some("right") => Some(0x111),
        Some("middle") => Some(0x112),
        _ => None,
    }
}

/// Açık tutulan sanal aygıtlar.
struct Devices {
    pointer: ZwlrVirtualPointerV1,
    keyboard: Option<ZwpVirtualKeyboardV1>,
    /// Klavyeye verilen keymap ve basılı değiştiricilerin durumu (`key` için).
    xkb: Option<(xkb::Keymap, xkb::State)>,
    /// Kilitli değiştirici maskesi (standart xkb: Lock = 1<<1, NumLock = Mod2 = 1<<4).
    locked: u32,
}

/// Keysym adının (ya da `ctrl`/`shift`/`alt`/`super`) ilk seviyede üretildiği keycode.
fn keycode_for(keymap: &xkb::Keymap, name: &str) -> Option<xkb::Keycode> {
    let name = match name {
        "ctrl" => "Control_L",
        "shift" => "Shift_L",
        "alt" => "Alt_L",
        "super" => "Super_L",
        other => other,
    };
    let sym = xkb::keysym_from_name(name, xkb::KEYSYM_NO_FLAGS);
    if sym.raw() == 0 {
        return None;
    }
    (keymap.min_keycode().raw()..=keymap.max_keycode().raw())
        .map(xkb::Keycode::new)
        .find(|&kc| keymap.key_get_syms_by_level(kc, 0, 0).contains(&sym))
}

/// Kombinasyonu kalıcı klavyeden basar (sırayla) ve bırakır (ters sırayla); her adımda
/// değiştirici durumu da gönderilir.
fn press_combo(dev: &mut Devices, time: u32, combo: &str) -> Result<(), String> {
    let Devices {
        keyboard,
        xkb,
        locked,
        ..
    } = dev;
    let keyboard = keyboard
        .as_ref()
        .ok_or("sanal klavye yok (NUXUT_TEST_KEYMAP)")?;
    let (keymap, state) = xkb.as_mut().ok_or("keymap yok")?;
    let codes = combo
        .split('+')
        .map(|name| keycode_for(keymap, name))
        .collect::<Option<Vec<_>>>()
        .ok_or("key: bilinmeyen tuş adı")?;
    if codes.is_empty() {
        return Err("key: kombinasyon boş".into());
    }
    let mut send = |kc: xkb::Keycode, down: bool, t: u32| {
        // evdev kodu = xkb keycode − 8.
        keyboard.key(t, kc.raw() - 8, u32::from(down));
        let dir = if down {
            xkb::KeyDirection::Down
        } else {
            xkb::KeyDirection::Up
        };
        state.update_key(kc, dir);
        keyboard.modifiers(
            state.serialize_mods(xkb::STATE_MODS_DEPRESSED),
            state.serialize_mods(xkb::STATE_MODS_LATCHED),
            state.serialize_mods(xkb::STATE_MODS_LOCKED) | *locked,
            state.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE),
        );
    };
    for &kc in &codes {
        send(kc, true, time);
    }
    for &kc in codes.iter().rev() {
        send(kc, false, time + 1);
    }
    Ok(())
}

/// Tek komut satırını uygular; geçersizse hata metni.
fn apply(dev: &mut Devices, start: Instant, line: &str) -> Result<(), String> {
    let pointer = &dev.pointer;
    let mut words = line.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let rest: Vec<&str> = words.collect();
    let num = |i: usize| -> Result<u32, String> {
        rest.get(i)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("sayı bekleniyor: {line}"))
    };
    let time = start.elapsed().as_millis() as u32;
    match cmd {
        "move" => pointer.motion_absolute(time, num(0)?, num(1)?, num(2)?, num(3)?),
        "click" | "press" | "release" => {
            let b =
                button(rest.first().copied()).ok_or_else(|| format!("bilinmeyen düğme: {line}"))?;
            if cmd != "release" {
                pointer.button(time, b, ButtonState::Pressed);
            }
            if cmd == "click" {
                pointer.frame();
            }
            if cmd != "press" {
                pointer.button(time + 1, b, ButtonState::Released);
            }
        }
        "scroll" => {
            let dy: f64 = rest
                .first()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("sayı bekleniyor: {line}"))?;
            pointer.axis(time, Axis::VerticalScroll, dy);
        }
        // wheel N: fare tekerleği çentikleri (kaynak Wheel + discrete; pozitif aşağı).
        "wheel" => {
            let n: i32 = rest
                .first()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("tam sayı bekleniyor: {line}"))?;
            pointer.axis_source(AxisSource::Wheel);
            pointer.axis_discrete(time, Axis::VerticalScroll, f64::from(n) * 15.0, n);
        }
        // lock caps|num on|off: kalıcı klavyenin kilit durumu (Caps/Num Lock göstergesi testleri).
        "lock" => {
            let bit = match rest.first().copied() {
                Some("caps") => 1 << 1,
                Some("num") => 1 << 4,
                _ => return Err(format!("lock caps|num on|off: {line}")),
            };
            match rest.get(1).copied() {
                Some("on") => dev.locked |= bit,
                Some("off") => dev.locked &= !bit,
                _ => return Err(format!("lock caps|num on|off: {line}")),
            }
            let keyboard = dev
                .keyboard
                .as_ref()
                .ok_or("sanal klavye yok (NUXUT_TEST_KEYMAP)")?;
            keyboard.modifiers(0, 0, dev.locked, 0);
            return Ok(());
        }
        // Hata metinleri satırı (tuş içeriğini) yazmaz.
        "key" => {
            let combo = rest.first().ok_or("key KOMBİNASYON")?;
            return press_combo(dev, time, combo);
        }
        "" => return Ok(()),
        _ => return Err("bilinmeyen komut".into()),
    }
    pointer.frame();
    Ok(())
}

fn connect() -> (EventQueue<State>, Devices) {
    // Kullanıcının display'ine asla: betik onu NUXUT_USER_WAYLAND_DISPLAY ile verir.
    let display = std::env::var_os("WAYLAND_DISPLAY");
    if display.is_none() || display == std::env::var_os("NUXUT_USER_WAYLAND_DISPLAY") {
        eprintln!("WAYLAND_DISPLAY test compositor'ınınki değil; hiçbir şey gönderilmedi");
        std::process::exit(1);
    }
    let conn = Connection::connect_to_env().unwrap_or_else(|e| {
        eprintln!("bağlanılamadı: {e}");
        std::process::exit(1)
    });
    let (globals, queue) = registry_queue_init::<State>(&conn).expect("registry");
    // Hyprland = kullanıcının oturumu (test compositor'ı sway): aygıt yaratmadan çık.
    let hyprland = globals
        .contents()
        .with_list(|list| list.iter().any(|g| g.interface.starts_with("hyprland_")));
    if hyprland {
        eprintln!("Hyprland'e bağlanıldı (kullanıcının oturumu); hiçbir şey gönderilmedi");
        std::process::exit(1);
    }
    let qh = queue.handle();
    let seat: WlSeat = globals.bind(&qh, 1..=1, ()).expect("wl_seat yok");
    let manager: ZwlrVirtualPointerManagerV1 = globals.bind(&qh, 1..=2, ()).unwrap_or_else(|_| {
        eprintln!("zwlr_virtual_pointer_manager_v1 yok (test compositor'ı değil mi?)");
        std::process::exit(1)
    });
    let pointer = manager.create_virtual_pointer(Some(&seat), &qh, ());
    // Kalıcı sanal klavye: koltuk baştan klavye yeteneği taşısın ki istemciler wl_keyboard
    // alsın; `wtype`'ın kısa ömürlü aygıtının tuşları da koltuk üzerinden onlara ulaşır.
    let mut keyboard = None;
    let mut xkb_state = None;
    if let Some(path) = std::env::var_os("NUXUT_TEST_KEYMAP") {
        match (
            std::fs::File::open(&path),
            globals.bind::<ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ()),
        ) {
            (Ok(file), Ok(kbd_manager)) => {
                let size = file.metadata().map_or(0, |m| m.len()) as u32;
                let kbd = kbd_manager.create_virtual_keyboard(&seat, &qh, ());
                // XKB_V1 = 1. Aygıt süreç ömrü boyunca açık kalır.
                kbd.keymap(1, file.as_fd(), size);
                keyboard = Some(kbd);
                // `key` için aynı keymap'in yerel kopyası (keysym → keycode, değiştiriciler).
                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                xkb_state = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| {
                        xkb::Keymap::new_from_string(
                            &context,
                            text,
                            xkb::KEYMAP_FORMAT_TEXT_V1,
                            xkb::KEYMAP_COMPILE_NO_FLAGS,
                        )
                    })
                    .map(|keymap| {
                        let state = xkb::State::new(&keymap);
                        (keymap, state)
                    });
                if xkb_state.is_none() {
                    eprintln!("keymap derlenemedi; `key` kullanılamaz");
                }
            }
            (Err(e), _) => eprintln!("keymap açılamadı: {e}"),
            (_, Err(_)) => eprintln!("zwp_virtual_keyboard_manager_v1 yok"),
        }
    }
    (
        queue,
        Devices {
            pointer,
            keyboard,
            xkb: xkb_state,
            locked: 0,
        },
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut queue, mut dev) = connect();
    queue.roundtrip(&mut State).expect("roundtrip");
    let start = Instant::now();

    if args.first().map(String::as_str) == Some("serve") {
        let fifo = args.get(1).unwrap_or_else(|| {
            eprintln!("kullanım: test_pointer serve FIFO");
            std::process::exit(2)
        });
        // Yazan taraf kapanınca EOF gelir; FIFO yeniden açılıp beklenir.
        loop {
            let file = match std::fs::File::open(fifo) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("FIFO açılamadı: {e}");
                    std::process::exit(1)
                }
            };
            for line in std::io::BufReader::new(file).lines() {
                let Ok(line) = line else { break };
                if let Err(e) = apply(&mut dev, start, &line) {
                    eprintln!("{e}");
                }
                if let Err(e) = queue.roundtrip(&mut State) {
                    eprintln!("bağlantı koptu: {e}");
                    std::process::exit(1);
                }
            }
        }
    }

    // Tek atımlık kullanım (aygıt hemen yok edilir; istemciler enter almayabilir).
    if let Err(e) = apply(&mut dev, start, &args.join(" ")) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    dev.pointer.destroy();
    queue.roundtrip(&mut State).expect("roundtrip");
}
