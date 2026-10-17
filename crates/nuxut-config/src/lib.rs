//! Nuxut config altyapısı.
//!
//! Kurallar (eski projeden öğrenilenler dahil):
//! - Dosya yoksa derlenmiş varsayılanlar kullanılır; **dosya asla otomatik yazılmaz.**
//!   Okurken yazmak + dosya izleme = sonsuz yeniden yükleme döngüsü.
//! - Kısmi dosya geçerlidir: yazılmayan her anahtar varsayılanından gelir.
//! - Bilinmeyen anahtarlar hata değil uyarıdır (yazım hatası yakalanır, eski sürüm
//!   config'i kırılmaz).
//! - Bozuk dosya yüklenmez; çağıran son geçerli config'i korur.
//! - Boyut sınırı vardır; config dosyası bir saldırı yüzeyi olmamalı.

#![forbid(unsafe_code)]

pub mod locale;
#[cfg(target_os = "linux")]
mod watch;
#[cfg(not(target_os = "linux"))]
#[path = "watch_unsupported.rs"]
mod watch;

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

pub use watch::ConfigWatcher;

/// Bir config dosyasının kabul edilen en büyük boyutu.
pub const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: okunamadı: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: dosya çok büyük ({size} bayt, sınır {MAX_CONFIG_BYTES})")]
    TooLarge { path: PathBuf, size: u64 },
    #[error("{path}: {message}")]
    Parse { path: PathBuf, message: String },
}

impl ConfigError {
    /// Loglanabilir özet: yalnızca hata türü. Display/Debug dosya yolunu ve ayrıştırıcının
    /// alıntıladığı config satırını taşır; onlar yalnızca kullanıcıya (UI) gösterilir, loga
    /// bu yazılır (ARCHITECTURE değişmez 9).
    pub fn log_summary(&self) -> String {
        match self {
            Self::Io { source, .. } => format!("okunamadı ({:?})", source.kind()),
            Self::TooLarge { size, .. } => format!("dosya çok büyük ({size} bayt)"),
            Self::Parse { .. } => "ayrıştırılamadı".into(),
        }
    }
}

/// Yükleme sonucu: değer + kullanıcıya gösterilecek uyarılar.
#[derive(Debug, Clone)]
pub struct Loaded<T> {
    pub value: T,
    pub warnings: Vec<String>,
    /// Dosya gerçekten var mıydı (yoksa varsayılanlar kullanıldı).
    pub from_file: bool,
}

/// `$XDG_CONFIG_HOME/nuxut` (genelde `~/.config/nuxut`).
pub fn config_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("nuxut"))
}

/// `<config_dir>/<name>.toml`.
pub fn config_path(name: &str) -> Option<PathBuf> {
    Some(config_dir()?.join(format!("{name}.toml")))
}

/// Dosyayı yükler. Tip `#[serde(default)]` ile işaretlenmiş olmalı ki kısmi dosyalar
/// geçerli olsun.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<Loaded<T>, ConfigError> {
    let text = match read_limited(path)? {
        Some(text) => text,
        None => return Ok(Loaded { value: T::default(), warnings: Vec::new(), from_file: false }),
    };
    let (value, warnings) = parse_str(&text).map_err(|message| ConfigError::Parse {
        path: path.to_owned(),
        message,
    })?;
    Ok(Loaded { value, warnings, from_file: true })
}

/// Metinden ayrıştırır. Bilinmeyen anahtarları uyarı olarak döndürür.
pub fn parse_str<T: DeserializeOwned>(text: &str) -> Result<(T, Vec<String>), String> {
    let mut unknown = Vec::new();
    let de = toml::Deserializer::parse(text).map_err(|e| e.to_string())?;
    let value = serde_ignored::deserialize(de, |path| unknown.push(path.to_string()))
        .map_err(|e: toml::de::Error| e.to_string())?;
    let warnings = unknown.into_iter().map(|key| format!("bilinmeyen anahtar yok sayıldı: {key}")).collect();
    Ok((value, warnings))
}

fn read_limited(path: &Path) -> Result<Option<String>, ConfigError> {
    let io = |source| ConfigError::Io { path: path.to_owned(), source };
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    let size = file.metadata().map_err(io)?.len();
    if size > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge { path: path.to_owned(), size });
    }
    let mut text = String::new();
    // Boyut ölçümü ile okuma arasında dosya büyüyebilir; okumayı da sınırla.
    file.take(MAX_CONFIG_BYTES).read_to_string(&mut text).map_err(io)?;
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Default, Deserialize, PartialEq)]
    #[serde(default)]
    struct Sample {
        size: f32,
        inner: Inner,
    }

    #[derive(Debug, Default, Deserialize, PartialEq)]
    #[serde(default)]
    struct Inner {
        enabled: bool,
    }

    #[test]
    fn log_summary_has_no_path_or_content() {
        let dir = std::env::temp_dir().join(format!("nuxut-config-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("gizli-dosya.toml");
        std::fs::write(&path, "size = \"GizliDeger\n").unwrap();
        let err = load::<Sample>(&path).unwrap_err();
        let _ = std::fs::remove_dir_all(&dir);
        // Display UI içindir ve ayrıntı taşır; özet taşımaz.
        assert!(err.to_string().contains("gizli-dosya"));
        let summary = err.log_summary();
        assert!(!summary.contains("gizli") && !summary.contains("Gizli"), "{summary}");
        assert_eq!(summary, "ayrıştırılamadı");
    }

    #[test]
    fn partial_file_uses_defaults_and_reports_unknown_keys() {
        let (v, warnings): (Sample, _) = parse_str("size = 3.0\n[inner]\nenabld = true\n").unwrap();
        assert_eq!(v, Sample { size: 3.0, inner: Inner { enabled: false } });
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("inner.enabld"), "{warnings:?}");
    }

    #[test]
    fn parse_error_has_location() {
        let err = parse_str::<Sample>("size = \n").unwrap_err();
        assert!(err.contains("line 1") || err.contains("1:"), "{err}");
    }

    #[test]
    fn missing_file_is_default() {
        let loaded: Loaded<Sample> = load(Path::new("/nonexistent/nuxut/x.toml")).unwrap();
        assert!(!loaded.from_file);
        assert_eq!(loaded.value, Sample::default());
    }
}
