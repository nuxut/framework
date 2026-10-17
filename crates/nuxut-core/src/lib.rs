//! Nuxut temel tipleri. Bağımlılıkları yalnızca `zeroize` (sır metni) ve opsiyonel `serde`; `unsafe` içermez;
//! framework'ün her katmanı ve gelecekteki her platform bu crate'i paylaşır.

#![forbid(unsafe_code)]

pub mod clipboard;
mod color;
mod geometry;
pub mod input;
mod secret;

pub use color::{Color, ColorParseError};
pub use geometry::{Corners, Insets, Point, Rect, Size};
pub use secret::SecretText;
