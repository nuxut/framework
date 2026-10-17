//! Güvenilmeyen medya kaynağı: yalnızca yerel, normal dosya.
//!
//! mpv'ye **hiçbir zaman yol ya da URL verilmez**. Dosyayı biz açarız: mutlak yol, son bileşen
//! sembolik bağ olamaz (`O_NOFOLLOW`), açtıktan sonra `fstat` ile normal dosya olduğu
//! doğrulanır (FIFO/aygıt/dizin yok; `O_NONBLOCK` FIFO'da asılı kalmayı önler). mpv'ye
//! `fdclose://N` ile tanıtıcı verilir. Böylece `http://`, `rtsp://`, `ytdl://`, `edl://`,
//! `av://` gibi protokoller ve kaynağın yanındaki dosyalar (alt yazı, ses, kapak) açılamaz.
//! Kalan iç başvurular (playlist, EDL, HLS/concat bölümleri) `ffi` sertleştirme
//! seçenekleriyle kapalıdır.

use std::fs::File;
use std::path::Path;

/// mpv'nin açabileceği tek kaynak türleri.
#[derive(Debug)]
pub(crate) enum MediaSource {
    /// Güvenli açılmış yerel dosya; mpv'ye sahipliğiyle (`fdclose://`) verilir.
    File(File),
    /// Kodda sabit, güvenilen üreteç (VA-API yoklamasının `lavfi` test klibi).
    Builtin(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    #[error("yalnızca mutlak yerel dosya yolu kabul edilir")]
    NotLocal,
    #[error("sembolik bağ kabul edilmez")]
    Symlink,
    #[error("normal dosya değil")]
    NotRegular,
    #[error("G/Ç hatası: {0}")]
    Io(std::io::ErrorKind),
}

/// `path`'i güvenilmeyen medya olarak açar (yukarıdaki kurallar).
pub fn open_media(path: &Path) -> Result<File, SourceError> {
    if !path.is_absolute() {
        return Err(SourceError::NotLocal);
    }
    let file = open_nofollow(path)?;
    let meta = file.metadata().map_err(|e| SourceError::Io(e.kind()))?;
    if !meta.file_type().is_file() {
        return Err(SourceError::NotRegular);
    }
    Ok(file)
}

#[cfg(target_os = "linux")]
fn open_nofollow(path: &Path) -> Result<File, SourceError> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY;
    match rustix::fs::open(path, flags, Mode::empty()) {
        Ok(fd) => Ok(File::from(fd)),
        // Son bileşen sembolik bağsa O_NOFOLLOW ELOOP döner.
        Err(rustix::io::Errno::LOOP) => Err(SourceError::Symlink),
        Err(e) => Err(SourceError::Io(std::io::Error::from(e).kind())),
    }
}

/// Linux dışı (yalnızca durağan görsel içe aktarma): bağ denetimi + açma (TOCTOU penceresi
/// kabul; video yolu bu platformlarda yok).
#[cfg(not(target_os = "linux"))]
fn open_nofollow(path: &Path) -> Result<File, SourceError> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| SourceError::Io(e.kind()))?;
    if meta.file_type().is_symlink() {
        return Err(SourceError::Symlink);
    }
    File::open(path).map_err(|e| SourceError::Io(e.kind()))
}
