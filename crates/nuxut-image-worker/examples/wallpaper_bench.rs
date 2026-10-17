//! Duvar kağıdı profilinde yalıtılmış çözümün süresi ve belleği (ARCHITECTURE "Duvar kağıdı
//! yalıtımı" ölçüm tablosu).
//!
//! ```sh
//! cargo build --release -p nuxut-image-worker
//! NUXUT_IMAGE_WORKER=target/release/nuxut-image-worker \
//!   cargo run --release -p nuxut-image-worker --example wallpaper_bench -- <dosya> <genişlik> <yükseklik> [cover|contain]
//! ```
//! Ölçülen: uçtan uca süre (`WallpaperDecoder` isteği → kare), işçinin tepe RSS'i (çalışırken
//! `/proc/<pid>/status` VmHWM, 2 ms aralıkla), ebeveynin VmRSS artışı (kare tutulurken ve
//! bırakıldıktan sonra: belleğin işletim sistemine dönüp dönmediği).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use nuxut_image::{Fit, WallpaperDecoder, WallpaperDecoderOptions, WallpaperRequest};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};

fn status_kib(pid: &str, key: &str) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    s.lines().find_map(|l| l.strip_prefix(key)).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok())
}

/// Bu sürecin çocuklarının en büyük VmHWM'i (KiB).
fn children_hwm() -> u64 {
    let me = std::process::id().to_string();
    let mut best = 0;
    let Ok(dir) = std::fs::read_dir("/proc") else { return 0 };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.parse::<u32>().is_err() {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { continue };
        if rest.split_whitespace().nth(1) == Some(me.as_str()) {
            best = best.max(status_kib(&name, "VmHWM:").unwrap_or(0));
        }
    }
    best
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(path), Some(w), Some(h)) = (args.first(), args.get(1).and_then(|v| v.parse().ok()), args.get(2).and_then(|v| v.parse().ok())) else {
        eprintln!("kullanım: wallpaper_bench <dosya> <genişlik> <yükseklik> [cover|contain]");
        std::process::exit(2);
    };
    let fit = if args.get(3).map(String::as_str) == Some("contain") { Fit::Contain } else { Fit::Cover };
    let sb = Arc::new(Sandbox::new(SandboxConfig::wallpaper()));
    eprintln!("yalıtım: {:?}", sb.isolation());
    let dec = WallpaperDecoder::new(sb.clone(), WallpaperDecoderOptions { linger: Duration::ZERO, ..WallpaperDecoderOptions::default() });
    let base = status_kib("self", "VmRSS:").unwrap_or(0);
    let peak = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let (peak, stop) = (peak.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                peak.fetch_max(children_hwm(), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let ticket = dec.request(WallpaperRequest::new(path, (w, h), fit), move |r| {
        let _ = tx.send(r);
    });
    let result = rx.recv_timeout(Duration::from_secs(120)).expect("sonuç gelmedi");
    let elapsed = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    let _ = sampler.join();
    drop(ticket);
    let frame = match result {
        Ok(f) => f,
        Err(e) => {
            println!("hata: {e} ({:.0} ms; işçi tepe RSS {} MiB)", elapsed.as_secs_f64() * 1000.0, peak.load(Ordering::Relaxed) >> 10);
            std::process::exit(1);
        }
    };
    let held = status_kib("self", "VmRSS:").unwrap_or(0).saturating_sub(base);
    let (fw, fh) = frame.size();
    let bytes = frame.byte_size();
    drop(frame);
    // Kare bırakıldı; çözüm thread'i linger 0 ile çıkar.
    let until = Instant::now() + Duration::from_secs(5);
    while dec.stats().thread_alive && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    let after = status_kib("self", "VmRSS:").unwrap_or(0).saturating_sub(base);
    println!(
        "{fw}x{fh} ({:.1} MiB); {:.0} ms; işçi tepe RSS {} MiB; ebeveyn +{} MiB (kare tutulurken), +{} MiB (bırakınca); önbellek {} B; işçi {}",
        bytes as f64 / f64::from(1 << 20),
        elapsed.as_secs_f64() * 1000.0,
        peak.load(Ordering::Relaxed) >> 10,
        held >> 10,
        after >> 10,
        dec.stats().resident_bytes,
        sb.stats().live,
    );
}
