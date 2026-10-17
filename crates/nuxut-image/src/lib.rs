//! Nuxut görselleri.
//!
//! - [`decode`]: PNG, JPEG, WebP, GIF (animasyonlu), JPEG XL (`jxl`), AVIF (`avif`, sistem
//!   dav1d) ve SVG çözme; boyut/piksel/kare sınırları (dekompresyon bombası koruması) ve hedef
//!   boyuta yüksek kaliteli küçültme.
//! - [`cursor`]: imleç temaları (XCursor + hyprcursor), sınırlı ayrıştırıcılar (önizleme).
//! - [`preview`]: tema önizleme rolleri, istek kurucuları, sınırlı LRU önbellek.
//! - [`app_icon`]: `app_id` → `.desktop` → tema ikonu ([`AppIcons`]; arka planda, sınırlı LRU).
//! - [`icon`]: freedesktop ikon teması araması (Icon Theme Spec), önbellekli; StatusNotifierItem
//!   ARGB32 pixmap dönüşümü.
//! - [`Player`]: animasyon karesi zamanlaması (yalnızca görünürken ilerler, boşta iş yok).
//! - [`Loader`]: arka plan thread'lerinde asenkron yükleme; sonuç callback ile gelir.
//! - [`IsolatedDecoder`]: güvenilmeyen görselin yalıtılmış çözümü (ADR-0007; işçi `nuxut-image-sandbox`).
//! - `render` özelliğiyle: [`Wallpaper`] bileşeni ve [`Image`] → doku dönüşümü.
//!
//! Tüm pikseller **premultiplied RGBA8**'dir (renderer'ın beklediği biçim).

#![forbid(unsafe_code)]

pub mod app_icon;
pub mod cursor;
mod decode;
mod error;
mod indexed;
mod isolation;
pub mod icon;
mod limits;
mod loader;
mod pixels;
mod player;
pub mod preview;
mod raw;
mod resize;
mod svg;
#[cfg(feature = "render")]
mod wallpaper;
mod wallpaper_decode;

pub use decode::{Animation, AnimationInfo, AnimationStream, Decoded, Frame, MAX_FRAME_DELAY, animation_info, decode_bytes, decode_file, is_animated, is_image};
pub use error::ImageError;
pub use cursor::{CursorImage, CursorRole, CursorSizes, CursorTheme};
pub use isolation::{ANON_KEY, CancelFlag, DecodedFile, IsolatedDecoder, Isolation, NO_KEY, SandboxFailure, SniffedFormat, decode_isolated, derive_key, enter_worker_mode, file_key, in_worker_mode, open_user_file, sniff_format};
pub use icon::IconTheme;
pub use app_icon::{AppIcon, AppIconDirs, AppIconOptions, AppIcons};
#[cfg(feature = "render")]
pub use app_icon::AppIconTextures;
pub use limits::{DecodeOptions, Fit, JXL_BYTES_PER_PIXEL, Limits, TargetSize, Trust};
pub use loader::{LoadHandle, LoadKeys, LoadRequest, Loader, LoaderOptions, Source};
pub use pixels::Image;
pub use player::Player;
pub use preview::{IconRole, PreviewCache, PreviewImage, PreviewKey};
pub use raw::{downscale_argb32, downscale_rgba};
pub use resize::fit_size;
pub use svg::render_svg;
pub use wallpaper_decode::{WallpaperDecodeStats, WallpaperDecoder, WallpaperDecoderOptions, WallpaperFrame, WallpaperRequest, WallpaperTicket};
#[cfg(feature = "render")]
pub use wallpaper::{Wallpaper, WallpaperBackdrop, WallpaperSource, WallpaperStatus};
