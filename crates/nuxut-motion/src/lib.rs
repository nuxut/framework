//! Nuxut hareket sistemi.
//!
//! Temel ilke: **mekânsal hareket yaydır, süre değil.** Yay animasyonları kesilebilir; yeni
//! hedef geldiğinde mevcut konum *ve hız* korunur, bu yüzden hareket hiçbir zaman "kırılmaz".
//! Az sönümlü (underdamped) bir yay, hedefin biraz ötesine taşıp geri oturur; puding hissi
//! buradan gelir.
//!
//! Yaylar analitik olarak çözülür (sayısal entegrasyon yok). Sonuç kare hızından tamamen
//! bağımsızdır: 30 Hz bir VM'de ve 165 Hz bir monitörde aynı eğri çizilir.
//!
//! Efektler (opaklık, renk) için süre + bezier eğrisi kullanan [`Tween`] de vardır.

#![forbid(unsafe_code)]

mod animated;
mod bezier;
mod deform;
mod motion;
mod spring;

pub use animated::{Animatable, Animated};
pub use bezier::{CubicBezier, MAX_SLOPE};
pub use deform::{Deform, Stretch};
pub use motion::{Curve, Motion};
pub use spring::{Spring, SpringState};
