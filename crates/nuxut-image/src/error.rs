use std::path::PathBuf;

/// Görsel hataları. `Display` ve `Debug` kullanıcı içeriği taşımaz (ARCHITECTURE değişmez 9):
/// dosya yolu ve ikon adı alanlarda durur ama yazdırılmaz; ayrıştırıcı iletileri yalnızca
/// konum/tür içerir.
#[derive(thiserror::Error)]
pub enum ImageError {
    #[error("G/Ç hatası: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("dosya çok büyük: {size} bayt (sınır {limit})")]
    FileTooLarge { size: u64, limit: u64 },
    #[error("görsel boyutu sınırı aşıyor: {width}x{height}")]
    TooLarge { width: u32, height: u32 },
    #[error("boş görsel")]
    Empty,
    #[error("bilinmeyen ya da desteklenmeyen biçim")]
    UnknownFormat,
    #[error("ikon bulunamadı ({} baytlık ad)", .0.len())]
    NotFound(String),
    #[error("çözme hatası: {0}")]
    Decode(String),
    #[error("SVG hatası: {0}")]
    Svg(String),
    #[error("küçültme hatası: {0}")]
    Resize(String),
    #[error("geçersiz piksel verisi: {0}")]
    InvalidData(String),
    #[error("güvenilmeyen kaynakta desteklenmeyen biçim: {0}")]
    Unsupported(&'static str),
    #[error("iptal edildi")]
    Cancelled,
    /// Yalıtılmış çözüm başarısız (ADR-0007); bayt/yol taşımaz. Çağıran yedek ikonu gösterir.
    #[error("yalıtılmış çözüm: {0}")]
    Sandbox(crate::SandboxFailure),
}

impl std::fmt::Debug for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { source, .. } => f.debug_struct("Io").field("path", &format_args!("<gizli>")).field("source", source).finish(),
            Self::NotFound(name) => write!(f, "NotFound(<{} bayt gizli>)", name.len()),
            other => write!(f, "{other}"),
        }
    }
}

impl From<image::ImageError> for ImageError {
    fn from(e: image::ImageError) -> Self {
        match e {
            image::ImageError::Limits(l) => Self::Decode(format!("sınır aşıldı: {l}")),
            image::ImageError::Unsupported(u) => Self::Decode(format!("desteklenmiyor: {u}")),
            other => Self::Decode(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_debug_hide_paths_and_names() {
        let io = ImageError::Io { path: "/home/kişi/gizli-tatil.png".into(), source: std::io::ErrorKind::NotFound.into() };
        let name = ImageError::NotFound("file:///home/kişi/x.svg".into());
        for e in [io, name] {
            for s in [e.to_string(), format!("{e:?}")] {
                assert!(!s.contains("kişi") && !s.contains("tatil"), "{s}");
            }
        }
    }
}
