//! Nuxut renderer'ı: OpenGL ES 3.0 (`glow`) üzerinde SDF tabanlı yuvarlatılmış dikdörtgen,
//! gölge, blob alanı ve `cosmic-text` metni. Wayland bilmez; GL context'i dışarıdan alır.
//!
//! Tüm renkler premultiplied alpha ile karıştırılır (`ONE, ONE_MINUS_SRC_ALPHA`).

pub mod gl;
mod atlas;
mod blob;
mod blur;
mod damage;
mod gradient;
mod stroke;
mod error;
mod font_cache;
mod painter;
mod record;
mod renderer;
mod style;
mod text;
#[cfg(test)]
mod optical_tests;
mod texture;

pub use blob::{BlobField, BlobShape, DEFORM_IDENTITY, FrameShape, MAX_BLOB_SHAPES};
pub use damage::{DamageHistory, MAX_RECTS as MAX_DAMAGE_RECTS, PxRect, Region, Repaint};
pub use error::RenderError;
pub use glow;
pub use painter::Painter;
pub use record::{DrawKind, RecordedDraw, Recorder};
pub use renderer::{DEFAULT_TEXT_GAMMA, FrameTarget, Renderer};
pub use style::{Border, RectStyle, ShadowStyle};
pub use text::{FamilyInfo, GenericFamily, MAX_TEXT_BYTES, OpticalCenter, SpanStyle, TextAlign, TextLayout, TextStyle, TextSystem};
pub use texture::{ImageData, ImageFit, ImageStyle, MAX_TEXTURE_SIDE, Texture, TextureOptions, UPLOAD_BUDGET_PER_FRAME};
