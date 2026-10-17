//! Çözme süresi ve tepe bellek ölçümü.
//!
//! ```sh
//! cargo run --release -p nuxut-image --example decode_bench -- <dosya> [genişlik yükseklik]
//! ```
//! Hedef verilirse görsel o boyuta (cover) küçültülür. Tepe RSS `/proc/self/status` VmHWM'den.

use std::time::Instant;

use nuxut_image::{DecodeOptions, Decoded, Fit, TargetSize, decode_file};

fn status(key: &str) -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with(key)).map(|l| l[key.len()..].trim().to_owned()))
        .unwrap_or_default()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("kullanım: decode_bench <dosya> [genişlik yükseklik]");
        std::process::exit(2);
    };
    let target = match (args.get(1).and_then(|v| v.parse().ok()), args.get(2).and_then(|v| v.parse().ok())) {
        (Some(w), Some(h)) => Some(TargetSize::new(w, h, Fit::Cover)),
        _ => None,
    };
    let start = Instant::now();
    let decoded = match decode_file(path.as_ref(), &DecodeOptions { target, ..Default::default() }) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hata: {e}");
            std::process::exit(1);
        }
    };
    let elapsed = start.elapsed();
    let kind = match &decoded {
        Decoded::Still(_) => "durağan".to_owned(),
        Decoded::Animated(a) => format!("{} kare, döngü {:?}", a.frames.len(), a.duration()),
        Decoded::Streamed(_) => "akış (kareler arka planda çözülür)".to_owned(),
    };
    let (w, h) = decoded.size();
    println!(
        "{w}x{h} {kind}; {:.1} MiB piksel; {:.0} ms; tepe RSS {}; şu an RSS {}",
        decoded.byte_size() as f64 / (1 << 20) as f64,
        elapsed.as_secs_f64() * 1000.0,
        status("VmHWM:"),
        status("VmRSS:"),
    );
}
