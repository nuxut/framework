//! Sentetik dokunma (YALNIZ canlı test, `test-touch` özelliği, debug derlemesi).
//!
//! Başsız sway'de dokunmatik koltuk yok ve Wayland'de sanal dokunma protokolü yok; kullanıcının
//! oturumunu etkilemeden dokunma yolunu uçtan uca denemek için olaylar `wl_touch` yerine bu
//! süreçteki bir FIFO'dan gelir ve `wl_touch` işleyicisiyle aynı yoldan geçer (öykünme, sahiplenme,
//! bekleme tiki, kilit kuralı). `NUXUT_TEST_TOUCH` FIFO'nun yolu; FIFO'yu test betiği yaratır
//! (`mkfifo`), değişken yoksa hiçbir şey kurulmaz.
//!
//! Satır başına bir komut (koordinat yüzey içi, mantıksal piksel; `NS` layer namespace'i, o adla
//! son yaratılan yüzey):
//!
//! ```text
//! down ID NS X Y
//! motion ID X Y
//! up ID
//! cancel
//! ```

#[cfg(not(debug_assertions))]
compile_error!("test-touch yalnızca debug derlemesinde (canlı test) kullanılabilir");

use std::io::Read;
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction};
use nuxut_core::Point;
use nuxut_core::input::TouchId;

use crate::App;
use crate::state::State;

pub const ENV: &str = "NUXUT_TEST_TOUCH";

/// Ayrıştırılmış komut.
#[derive(Debug, Clone, PartialEq)]
enum Command {
    Down { id: i32, namespace: String, at: Point },
    Motion { id: i32, at: Point },
    Up { id: i32 },
    Cancel,
}

fn parse(line: &str) -> Option<Command> {
    let w: Vec<&str> = line.split_whitespace().collect();
    let num = |s: &str| s.parse::<f32>().ok().filter(|v| v.is_finite());
    match w.as_slice() {
        ["down", id, ns, x, y] => {
            Some(Command::Down { id: id.parse().ok()?, namespace: (*ns).to_owned(), at: Point::new(num(x)?, num(y)?) })
        }
        ["motion", id, x, y] => Some(Command::Motion { id: id.parse().ok()?, at: Point::new(num(x)?, num(y)?) }),
        ["up", id] => Some(Command::Up { id: id.parse().ok()? }),
        ["cancel"] => Some(Command::Cancel),
        _ => None,
    }
}

/// `NUXUT_TEST_TOUCH` bir FIFO'yu gösteriyorsa okur. FIFO okuma+yazma açılır: yazan betik her
/// `echo`'da kapatsa da EOF olmaz.
pub(crate) fn install<A: App>(handle: &LoopHandle<'static, State<A>>) {
    let Some(path) = std::env::var_os(ENV) else { return };
    let fifo = std::fs::metadata(&path).is_ok_and(|m| m.file_type().is_fifo());
    if !fifo {
        tracing::warn!("{ENV} bir FIFO değil; sentetik dokunma kapalı");
        return;
    }
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
        .open(&path)
    {
        Ok(f) => f,
        Err(e) => return tracing::warn!("{ENV} açılamadı: {e}"),
    };
    let mut pending = String::new();
    let source = Generic::new(file, Interest::READ, Mode::Level);
    let inserted = handle.insert_source(source, move |_, file, state: &mut State<A>| {
        let mut buf = [0u8; 1024];
        let mut reader: &std::fs::File = file;
        match reader.read(&mut buf) {
            Ok(n) => pending.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(PostAction::Continue),
            Err(e) => return Err(e),
        }
        while let Some(end) = pending.find('\n') {
            let line: String = pending.drain(..=end).collect();
            match parse(&line) {
                Some(cmd) => apply(state, cmd),
                None => tracing::warn!(line = line.trim(), "sentetik dokunma: anlaşılmayan komut"),
            }
        }
        Ok(PostAction::Continue)
    });
    match inserted {
        Ok(_) => tracing::warn!("SENTETİK dokunma açık ({ENV}; yalnızca test derlemesi)"),
        Err(e) => tracing::warn!("{ENV} kaynağı kurulamadı: {}", e.error),
    }
}

fn apply<A: App>(state: &mut State<A>, cmd: Command) {
    match cmd {
        Command::Down { id, namespace, at } => match state.cx.test_layers.get(&namespace).copied() {
            Some(sid) => state.touch_down(sid, TouchId(id), at),
            None => tracing::warn!(namespace, "sentetik dokunma: bu adla yüzey yok"),
        },
        Command::Motion { id, at } => state.touch_motion(TouchId(id), at),
        Command::Up { id } => state.touch_up(TouchId(id)),
        Command::Cancel => state.touch_cancel(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_rejects_garbage() {
        assert_eq!(
            parse("down 0 nuxut-osk 12.5 40\n"),
            Some(Command::Down { id: 0, namespace: "nuxut-osk".into(), at: Point::new(12.5, 40.0) })
        );
        assert_eq!(parse("motion 1 3 4"), Some(Command::Motion { id: 1, at: Point::new(3.0, 4.0) }));
        assert_eq!(parse("up 2"), Some(Command::Up { id: 2 }));
        assert_eq!(parse("cancel"), Some(Command::Cancel));
        for bad in ["", "down 0 ns 1", "down x ns 1 2", "motion 0 NaN 1", "up", "tap 1 2"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
