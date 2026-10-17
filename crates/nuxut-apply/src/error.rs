//! Hata tipi. `Display` yol ve değer içermez (loglanabilir); yollar varyantlarda taşınır ve
//! yalnızca kullanıcıya (`~` ile kısaltılarak) gösterilir.

use std::path::PathBuf;

use crate::env::Skip;

#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyError {
    #[error("bu ortamda yazılmaz ({})", .0.name())]
    Skipped(Skip),
    /// Hedef başka bir araç tarafından yönetiliyor (salt okunur, başka sahip, sabit bağ, nix deposu,
    /// kırık symlink). Üstüne yazılmadı, symlink kırılmadı.
    #[error("hedef başka bir araç tarafından yönetiliyor")]
    ExternallyManaged(PathBuf),
    /// Okunduktan sonra dışarıdan değişti; çağıran yeniden okuyup birleştirmeli.
    #[error("hedef okunduktan sonra değişti")]
    Changed,
    /// Başka bir süreç uygulama kilidini tutuyor (5 s).
    #[error("başka bir uygulama sürüyor")]
    Busy,
    #[error("dosya boyut sınırını aşıyor")]
    TooLarge(PathBuf),
    /// Hedef bu sistemde yok (ör. `dconf` kurulu değil). İçerik: hedefin gösterim adı.
    #[error("hedef kullanılamıyor")]
    Unavailable(String),
    /// Snapshot alınamıyor; dış hedefe yazılmadı.
    #[error("snapshot deposu kullanılamıyor")]
    SnapshotUnavailable,
    /// Hedef dosya yok (kullanıcının ana dosyası yaratılmaz).
    #[error("hedef bulunamadı")]
    NotFound(PathBuf),
    /// Hedef dosya çözümlenemedi (TOML/INI sözdizimi, kapanmamış blok, UTF-8 değil).
    #[error("hedef dosya çözümlenemedi")]
    Parse(PathBuf),
    /// Geçersiz girdi (kimlik, yol, değer); statik açıklama.
    #[error("geçersiz girdi: {0}")]
    Invalid(&'static str),
    #[error("G/Ç hatası ({0:?})")]
    Io(std::io::ErrorKind),
}

impl ApplyError {
    /// IPC/JSON için kısa tür adı (ADR-0001 §10.3).
    pub fn kind_name(&self) -> &'static str {
        match self {
            ApplyError::Skipped(_) => "skipped",
            ApplyError::ExternallyManaged(_) => "externally_managed",
            ApplyError::Changed => "changed",
            ApplyError::Busy => "busy",
            ApplyError::TooLarge(_) => "too_large",
            ApplyError::Unavailable(_) => "unavailable",
            ApplyError::SnapshotUnavailable => "snapshot_unavailable",
            ApplyError::NotFound(_) => "not_found",
            ApplyError::Parse(_) => "parse",
            ApplyError::Invalid(_) => "invalid",
            ApplyError::Io(_) => "io",
        }
    }
}

/// Yolları göstermez (değişmez 9): yalnızca tür adı.
impl std::fmt::Debug for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApplyError::Skipped(s) => write!(f, "Skipped({s:?})"),
            ApplyError::Invalid(why) => write!(f, "Invalid({why:?})"),
            ApplyError::Io(k) => write!(f, "Io({k:?})"),
            other => f.write_str(other.kind_name()),
        }
    }
}

impl From<std::io::Error> for ApplyError {
    fn from(e: std::io::Error) -> Self {
        ApplyError::Io(e.kind())
    }
}

impl From<Skip> for ApplyError {
    fn from(s: Skip) -> Self {
        ApplyError::Skipped(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_has_no_paths() {
        let p = PathBuf::from("/home/u/gizli/dosya.ini");
        for e in [
            ApplyError::ExternallyManaged(p.clone()),
            ApplyError::TooLarge(p.clone()),
            ApplyError::NotFound(p.clone()),
            ApplyError::Parse(p.clone()),
            ApplyError::Unavailable("/home/u/gizli".into()),
        ] {
            assert!(!e.to_string().contains("gizli"), "{e}");
            assert!(!format!("{e:?}").contains("gizli"), "{e:?}");
        }
    }
}
