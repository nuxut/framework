//! `nuxut-i18n` bütünleşik testleri: tek test ikilisi (`cargo test -p nuxut-i18n --test it`).
//!
//! Ayrı kalanlar (süreç geneli durum değiştirir, ortak ikiliye girmez):
//! - `tests/global.rs`: süreç geneli katalog (`set_current`, `configure`, `register`).
//! - `tests/watch_reload.rs`: `XDG_*` ve `LANGUAGE` ortam değişkenlerini ayarlar.

mod check;
mod framework_guard;
mod runtime;
#[cfg(target_os = "linux")]
mod watch;
