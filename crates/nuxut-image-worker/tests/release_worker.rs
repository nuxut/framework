//! Dağıtılan işçi uçtan uca: kancasız ebeveynle el sıkışır (kilit beyanı kabul edilir) ve JPEG XL
//! çözer (jxl yalnızca bu ikilide). `cargo test -p nuxut-image-worker` tek başına çalıştırılınca
//! iki taraf da gerçekten kancasızdır.
#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::time::Duration;

use nuxut_image::{DecodeOptions, Fit, Isolation, Limits, TargetSize};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};

const WORKER: &str = env!("CARGO_BIN_EXE_nuxut-image-worker");
const RED4_JXL: &[u8] = include_bytes!("fixtures/red4.jxl");

#[test]
fn shipped_worker_decodes_jxl_and_svg_in_isolation() {
    let sb = Sandbox::new(SandboxConfig { worker: Some(WORKER.into()), timeout: Duration::from_secs(10), spawn_timeout: Duration::from_secs(10), ..SandboxConfig::default() });
    assert_eq!(sb.isolation(), Isolation::Process);
    let opts = DecodeOptions { limits: Limits::ICON, target: Some(TargetSize::new(16, 16, Fit::Contain)), animate: false };
    let img = sb.decode(1, Arc::from(RED4_JXL), &opts).expect("işçi JXL çözmeli");
    assert_eq!(img.size(), (4, 4));
    assert_eq!(&img.pixels()[..4], &[255, 0, 0, 255]);
    let svg: Arc<[u8]> = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/></svg>"#.to_vec().into();
    assert_eq!(sb.decode(2, svg, &opts).unwrap().size(), (16, 16));
    assert_eq!(sb.stats().decoded, 2);
    assert_eq!(sb.isolation(), Isolation::Process, "el sıkışma kabul edildi");
}
