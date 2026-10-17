//! `nuxut-image-sandbox` bütünleşik testleri: tek test ikilisi (`cargo test -p nuxut-image-sandbox --test it`),
//! gerçek (kancalı) işçi ikilisiyle. Süreç/fd sayan testler birbirini bozmasın diye modüller
//! arası TEK kilit ([`serial`]) altında sırayla çalışır.
#![cfg(target_os = "linux")]

use std::sync::{Mutex, MutexGuard};

mod adversarial;
mod preview;
mod wallpaper;

static SERIAL: Mutex<()> = Mutex::new(());

pub(crate) fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) const WORKER: &str = env!("CARGO_BIN_EXE_nuxut-image-worker-test");
