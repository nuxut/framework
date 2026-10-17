//! Yalıtılmış çözümün maliyeti (ADR-0007 §Havuz): tek kullanımlık işçilerle soğuk başlangıç
//! (süreç + kilitlenme + el sıkışma + çözüm), sıkı döngü (her istek yeni süreç; önceden ısıtılan
//! işçi kilitlenmeyi henüz bitirmemiş olabilir) ve aralıklı istek (önceden ısıtılmış işçi
//! hazır; tepsi/bildirim tipik durumu), süreç içi çözümle karşılaştırmalı.
//!
//! `cargo build -p nuxut-image-worker && cargo run -p nuxut-image-sandbox --example sandbox_bench`
//! (işçi yolu: `NUXUT_IMAGE_WORKER` ya da örneğin yanındaki `../nuxut-image-worker`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use nuxut_image::{DecodeOptions, Fit, Limits, TargetSize, Trust};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};

fn median(mut v: Vec<Duration>) -> (Duration, Duration) {
    v.sort();
    (v[v.len() / 2], v[v.len() - 1])
}

fn main() {
    let worker = std::env::var_os("NUXUT_IMAGE_WORKER").map(Into::into).or_else(|| {
        let exe = std::env::current_exe().ok()?;
        Some(exe.parent()?.parent()?.join("nuxut-image-worker"))
    });
    let svg: Arc<[u8]> = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" rx="12" fill="royalblue"/><circle cx="32" cy="32" r="18" fill="white" opacity="0.7"/></svg>"##.to_vec().into();
    let opts = DecodeOptions { limits: Limits::ICON, target: Some(TargetSize::new(64, 64, Fit::Contain)), animate: false };
    let cfg = SandboxConfig { worker, rate_per_sec: 1_000_000, burst: 1_000_000, ..SandboxConfig::default() };

    let mut cold = Vec::new();
    for _ in 0..20 {
        let sb = Sandbox::new(cfg.clone());
        let t = Instant::now();
        sb.decode(0, svg.clone(), &opts).expect("işçi çalışmalı");
        cold.push(t.elapsed());
    }
    let sb = Sandbox::new(cfg);
    sb.decode(0, svg.clone(), &opts).unwrap();
    let n = 300;
    let t = Instant::now();
    for _ in 0..n {
        sb.decode(0, svg.clone(), &opts).unwrap();
    }
    let tight = t.elapsed() / n;
    let mut spaced = Vec::new();
    for _ in 0..100 {
        std::thread::sleep(Duration::from_millis(20));
        let t = Instant::now();
        sb.decode(0, svg.clone(), &opts).unwrap();
        spaced.push(t.elapsed());
    }
    let t = Instant::now();
    for _ in 0..n {
        nuxut_image::decode_bytes(&svg, &opts.with_trust(Trust::System)).unwrap();
    }
    let local = t.elapsed() / n;
    let (cm, cw) = median(cold);
    let (sm, sw) = median(spaced);
    println!("soğuk (süreç + kilit + el sıkışma + çözüm): medyan {cm:?}, en kötü {cw:?}");
    println!("aralıklı istek (önceden ısıtılmış taze işçi, 64² SVG): medyan {sm:?}, en kötü {sw:?}");
    println!("sıkı döngü (istek başına yeni süreç): ortalama {tight:?}; süreç içi aynı çözüm: {local:?}");
}
