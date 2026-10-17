//! Duvar kağıdından Material You paleti (matugen tarzı).
//!
//! 1. `source_color_from_file(yol, çözücü, anahtar)` / `source_color_from_frame` (`image` özelliği,
//!    varsayılan açık): görsel yalıtılmış işçide 128² `Contain` çözülür (ya da önbellekteki kare
//!    küçültülür), Celebi niceleme + Material puanlamayla kaynak renk seçilir.
//! 2. [`Palette::generate`]: `material-colors` (material-color-utilities'in saf Rust portu) ile
//!    seçilen varyantta açık ve koyu şema; Nuxut `[colors]` rollerine [`THEME_ROLES`] eşlemesi.
//! 3. [`export::palette_toml`] → `$XDG_STATE_HOME/nuxut/palette.toml` (tema katmanı;
//!    `framework.toml` asla yazılmaz) ve [`export::export_templates`] → GTK/Qt/kitty/foot/
//!    Hyprland dosyaları `$XDG_STATE_HOME/nuxut/exports/` altına.
//!
//! Tüm üretim deterministiktir: aynı görsel ve seçenekler aynı baytları verir.

#![forbid(unsafe_code)]

pub mod export;
#[cfg(feature = "image")]
mod extract;
pub mod look;
mod roles;
mod scheme;
pub mod template;
pub mod text_scale;

use std::path::PathBuf;

#[cfg(feature = "image")]
pub use extract::{SAMPLE_TARGET, sample_options, source_color, source_color_from_file, source_color_from_frame};
pub use nuxut_theme::ContrastMode;
pub use roles::{ON_PAIRS, Role, SUCCESS_SEED, SURFACE_ROLES, Scheme, WARNING_SEED};
pub use scheme::{Mode, Palette, Roles, THEME_ROLES, Variant};

/// `Display`/`Debug` yol ve reddedilen adı yazmaz (loglarda kullanıcı içeriği yok); CLI yolu
/// kendi bağlamından gösterir. Şablon adı doğrulanmış düz ASCII dosya adıdır.
#[derive(thiserror::Error)]
pub enum PaletteError {
    #[cfg(feature = "image")]
    #[error("görsel: {0}")]
    Image(#[from] nuxut_image::ImageError),
    #[error("G/Ç hatası: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("güvensiz dosya adı ({} bayt)", .0.len())]
    BadName(String),
    #[error("şablon {0}: {1}")]
    Template(String, template::TemplateError),
}

impl std::fmt::Debug for PaletteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(feature = "image")]
            Self::Image(e) => f.debug_tuple("Image").field(e).finish(),
            Self::Io { source, .. } => f.debug_struct("Io").field("path", &format_args!("<gizli>")).field("source", source).finish(),
            other => write!(f, "{other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_hide_paths_and_rejected_names() {
        let io = PaletteError::Io { path: "/home/kişi/duvar.png".into(), source: std::io::ErrorKind::NotFound.into() };
        let bad = PaletteError::BadName("../kişi".into());
        for e in [io, bad] {
            for s in [e.to_string(), format!("{e:?}")] {
                assert!(!s.contains("kişi"), "{s}");
            }
        }
    }
}
