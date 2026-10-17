//! `nuxut-image` bütünleşik testleri: tek test ikilisi (`cargo test -p nuxut-image --test it`).

mod app_icon;
mod decode;
mod decode_gate;
mod icon_theme;
mod trust;
#[cfg(feature = "render")]
mod wallpaper_blur;
