//! Nuxut video duvar kağıdı ve duvar kağıdı içe aktarma.
//!
//! - [`VideoPlayer`]: libmpv render API'siyle bizim GL context'imize çizen oynatıcı (donanım
//!   çözme, döngü, ses aç/kapat, görünmezken duraklatma; yeni kare yoksa iş yok).
//! - [`Importer`]: eklenen duvar kağıdını en ucuz oynatılan biçime dönüştürüp önbelleğe alır
//!   (durağan → QOI, animasyon → H.264 video, video → gerekirse yeniden kodlama; ADR-0004).
//!
//! libmpv derlemede gerekmez: `libmpv.so.2` ilk kullanımda `dlopen` edilir; yoksa
//! [`VideoError::Unavailable`] döner ve çağıran statik duvar kağıdına düşer.

#![deny(unsafe_code)]

#[cfg(target_os = "linux")]
mod ffi;
#[cfg(not(target_os = "linux"))]
#[path = "ffi_stub.rs"]
mod ffi;
mod budget;
mod hwcaps;
mod import;
mod player;
mod source;

pub use budget::{BudgetExceeded, ImportLimits};
pub use hwcaps::{Codec, DecodeCaps};
pub use import::{ImportError, ImportEvent, ImportJob, ImportRequest, Imported, Importer};
pub use player::{VideoInfo, VideoPlayer, VideoStatus};
pub use source::{SourceError, open_media};

/// libmpv bu sistemde yüklenebiliyor mu.
pub fn available() -> bool {
    ffi::available()
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum VideoError {
    #[error("video desteği yok: {0}")]
    Unavailable(String),
    #[error("mpv: {0}")]
    Mpv(String),
    #[error("kaynak reddedildi: {0}")]
    Source(#[from] SourceError),
}
