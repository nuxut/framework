//! Sanal klavye demosu (zwp-virtual-keyboard-v1). YALNIZCA test compositor'ında çalıştırın
//! (`tools/test-compositor.sh`): odaktaki pencereye — testte kendi açtığınız
//! `kitty --class nuxut-test` — yazı yazar.
//!
//! Yüzey açmaz. Açılıştan 300 ms sonra compositor'ın keymap'ini (yoksa yerleşik "us")
//! `virtual_keyboard_set_keymap`'e verir, `NUXUT_VKBD_TEXT` metnini (varsayılan
//! "Nuxut vkbd") 25 ms aralıklarla yazar — büyük harfte Shift tuşu + değiştirici maskesiyle —
//! ve Enter'a basar. Ardından: aynı keymap yeniden (no-op), `release`, sonra `key` →
//! `NoKeymap` beklenir; `DEMO-OK` ile çıkar. 10 sn içinde bitmezse hata koduyla çıkar.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use nuxut_wayland::{
    App, Context, OutputId, OutputInfo, Painter, PointerEvent, SurfaceId, TimerId,
    VirtualKeyboardError,
};
use xkbcommon::xkb;

const KEY_LEFTSHIFT: u32 = 42;
const KEY_ENTER: u32 = 28;
/// us düzeninde Shift'in değiştirici maskesi (mod indeksi 0).
const SHIFT_MASK: u32 = 1;

/// Küçük harf ve boşluk için evdev kodu (us düzeni).
fn evdev(c: char) -> Option<u32> {
    const ROWS: [(&str, u32); 3] = [("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)];
    if c == ' ' {
        return Some(57);
    }
    ROWS.iter().find_map(|(row, base)| {
        row.find(c.to_ascii_lowercase())
            .map(|i| base + i as u32)
    })
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Key(u32, bool),
    Mods(u32),
}

#[derive(Default)]
struct Demo {
    started: bool,
    steps: VecDeque<Step>,
    keymap: Option<String>,
    deadline: Option<TimerId>,
    done: bool,
}

impl Demo {
    fn plan(&mut self, text: &str) {
        for c in text.chars() {
            let Some(code) = evdev(c) else {
                tracing::warn!("desteklenmeyen karakter atlandı");
                continue;
            };
            let upper = c.is_ascii_uppercase();
            if upper {
                self.steps.push_back(Step::Key(KEY_LEFTSHIFT, true));
                self.steps.push_back(Step::Mods(SHIFT_MASK));
            }
            self.steps.push_back(Step::Key(code, true));
            self.steps.push_back(Step::Key(code, false));
            if upper {
                self.steps.push_back(Step::Key(KEY_LEFTSHIFT, false));
                self.steps.push_back(Step::Mods(0));
            }
        }
        self.steps.push_back(Step::Key(KEY_ENTER, true));
        self.steps.push_back(Step::Key(KEY_ENTER, false));
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        tracing::info!(supported = cx.virtual_keyboard_supported(), "sanal klavye");
        if !cx.virtual_keyboard_supported() {
            std::process::exit(3);
        }
        let keymap = match cx.keyboard_keymap() {
            Some(k) => k.to_owned(),
            None => {
                tracing::info!("compositor keymap'i yok; yerleşik us");
                let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                xkb::Keymap::new_from_names(&context, "", "", "us", "", None, 0)
                    .expect("us keymap")
                    .get_as_string(xkb::KEYMAP_FORMAT_TEXT_V1)
            }
        };
        // Keymap'ten önce tuş: protokol hatası değil, açık hata.
        assert_eq!(
            cx.virtual_keyboard_key(KEY_ENTER, true),
            Err(VirtualKeyboardError::NoKeymap)
        );
        if let Err(e) = cx.virtual_keyboard_set_keymap(&keymap) {
            tracing::error!(%e, "set_keymap");
            std::process::exit(4);
        }
        self.keymap = Some(keymap);
        let text = std::env::var("NUXUT_VKBD_TEXT").unwrap_or_else(|_| "Nuxut vkbd".into());
        self.plan(&text);
        cx.set_timer(Duration::from_millis(25));
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        let keymap = self.keymap.take().expect("keymap");
        // Aynı keymap: no-op.
        cx.virtual_keyboard_set_keymap(&keymap).expect("aynı keymap");
        cx.virtual_keyboard_release();
        cx.virtual_keyboard_release();
        assert_eq!(
            cx.virtual_keyboard_key(KEY_ENTER, true),
            Err(VirtualKeyboardError::NoKeymap)
        );
        self.done = true;
        tracing::info!("DEMO-OK");
        cx.quit();
    }
}

impl App for Demo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if self.deadline.is_none() {
            self.deadline = Some(cx.set_timer(Duration::from_secs(10)));
            // Compositor keymap'i gelsin.
            cx.set_timer(Duration::from_millis(300));
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn pointer(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, _event: PointerEvent) {}

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if Some(id) == self.deadline {
            if !self.done {
                tracing::error!("DEMO-TIMEOUT");
                std::process::exit(2);
            }
            return;
        }
        if !self.started {
            self.started = true;
            self.start(cx);
            return;
        }
        match self.steps.pop_front() {
            Some(Step::Key(code, pressed)) => {
                cx.virtual_keyboard_key(code, pressed).expect("key");
            }
            Some(Step::Mods(depressed)) => {
                cx.virtual_keyboard_modifiers(depressed, 0, 0, 0).expect("modifiers");
            }
            None => return self.finish(cx),
        }
        cx.set_timer(Duration::from_millis(25));
    }

    fn draw(
        &mut self,
        _cx: &mut Context<Self>,
        _surface: SurfaceId,
        _now: Instant,
        _painter: &mut Painter,
    ) {
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,nuxut_wayland=debug".into()),
        )
        .init();
    nuxut_wayland::run(Demo::default()).expect("runtime");
}
