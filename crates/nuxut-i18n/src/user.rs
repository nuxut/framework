//! Kullanıcı çeviri dizini: `<kök>/<domain>/<dil>.ftl` gömülünün **üstüne** yazar (anahtar
//! başına). Güvenilmeyen girdi (ADR-0010 §3):
//! - yalnızca düz dosya; symlink izlenmez (`O_NOFOLLOW`), FIFO/aygıtta takılmaz
//!   (`O_NONBLOCK` + `fstat` ile tür denetimi);
//! - en çok [`MAX_USER_FILE_BYTES`], en çok [`MAX_USER_MESSAGES`] mesaj/terim (aşan dosya
//!   tümüyle yok sayılır);
//! - ayrıştırma hatalı girdiler atlanır, geri kalan kullanılır; dosya başına tek uyarı;
//!   içerik ve yol loga yazılmaz (değişmez 9).

use std::io::Read;
use std::path::Path;
use std::str::FromStr;

use fluent_bundle::FluentResource;
use fluent_bundle::concurrent::FluentBundle;
use fluent_syntax::ast;
use unic_langid::LanguageIdentifier;

/// Kullanıcı dosyasının en büyük boyu.
pub const MAX_USER_FILE_BYTES: u64 = 1024 * 1024;
/// Kullanıcı dosyasındaki en çok mesaj + terim.
pub const MAX_USER_MESSAGES: usize = 10_000;
/// Domain dizininde bakılan en çok girdi (dev dizin taraması sınırlı).
const MAX_DIR_ENTRIES: usize = 512;

/// Okuma reddinin nedeni (loga yalnızca bu yazılır).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Reject {
    NotRegular,
    Symlink,
    TooLarge,
    NotUtf8,
    TooManyMessages,
    Io,
}

/// Log alanı: neden reddedildi (tanı metni; arayüzde gösterilmez).
impl std::fmt::Display for Reject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotRegular => "düz dosya değil",
            Self::Symlink => "sembolik bağ",
            Self::TooLarge => "çok büyük",
            Self::NotUtf8 => "UTF-8 değil",
            Self::TooManyMessages => "çok fazla mesaj",
            Self::Io => "okunamadı",
        })
    }
}

/// `read`/`load`'un hata türü (`Err(Reject)`); metni yalnızca loga gider.
impl std::error::Error for Reject {}

/// Domain dizinindeki dil etiketleri (`<dil>.ftl` düz dosyalar; geçersiz adlar ve
/// symlink/dizin/FIFO atlanır).
pub(crate) fn languages(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .take(MAX_DIR_ENTRIES)
        .flatten()
        .filter_map(|e| {
            // Yalnızca düz dosya (symlink izlenmez: `file_type` bağın kendisini verir); yüklemede
            // zaten reddedilecek ad dil listesine girmesin.
            if !e.file_type().is_ok_and(|t| t.is_file()) {
                return None;
            }
            let name = e.file_name();
            let lang = name.to_str()?.strip_suffix(".ftl")?;
            LanguageIdentifier::from_str(lang).ok().filter(|l| !l.language.is_empty())?;
            Some(lang.to_owned())
        })
        .collect()
}

/// `<root>/<domain>/<lang>.ftl` varsa pakete üstüne yazarak ekler.
pub(crate) fn apply(bundle: &mut FluentBundle<FluentResource>, root: &Path, domain: &str, lang: &LanguageIdentifier) {
    let dir = root.join(domain);
    // Etiket yazımı farklı olabilir (`pt-br.ftl`): eşleşen ilk ad.
    let Some(file) = languages(&dir)
        .into_iter()
        .find(|name| LanguageIdentifier::from_str(name).is_ok_and(|l| &l == lang))
    else {
        return;
    };
    match load(&dir.join(format!("{file}.ftl"))) {
        Ok(Some((res, skipped))) => {
            if skipped > 0 {
                tracing::warn!(domain, %lang, skipped, "kullanıcı çeviri dosyasında hatalı girdiler atlandı");
            }
            bundle.add_resource_overriding(res);
        }
        Ok(None) => {}
        Err(reason) => {
            tracing::warn!(domain, %lang, reason = %reason, "kullanıcı çeviri dosyası yok sayıldı");
        }
    }
}

/// Dosyayı güvenle okur ve ayrıştırır: `(kaynak, atlanan hatalı girdi sayısı)`; dosya yoksa
/// `None`.
pub(crate) fn load(path: &Path) -> Result<Option<(FluentResource, usize)>, Reject> {
    let Some(text) = read(path)? else { return Ok(None) };
    let (res, skipped) = match FluentResource::try_new(text) {
        Ok(res) => (res, 0),
        Err((res, errors)) => (res, errors.len()),
    };
    let count = res
        .entries()
        .filter(|e| matches!(e, ast::Entry::Message(_) | ast::Entry::Term(_)))
        .count();
    if count > MAX_USER_MESSAGES {
        return Err(Reject::TooManyMessages);
    }
    Ok(Some((res, skipped)))
}

#[cfg(unix)]
fn open(path: &Path) -> Result<Option<std::fs::File>, Reject> {
    use rustix::fs::{FileType, Mode, OFlags};
    use rustix::io::Errno;

    let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOCTTY;
    let fd = match rustix::fs::open(path, flags, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        // O_NOFOLLOW son bileşen symlink ise ELOOP verir.
        Err(Errno::LOOP) => return Err(Reject::Symlink),
        Err(_) => return Err(Reject::Io),
    };
    let st = rustix::fs::fstat(&fd).map_err(|_| Reject::Io)?;
    if FileType::from_raw_mode(st.st_mode) != FileType::RegularFile {
        return Err(Reject::NotRegular);
    }
    if u64::try_from(st.st_size).unwrap_or(u64::MAX) > MAX_USER_FILE_BYTES {
        return Err(Reject::TooLarge);
    }
    Ok(Some(std::fs::File::from(fd)))
}

#[cfg(not(unix))]
fn open(path: &Path) -> Result<Option<std::fs::File>, Reject> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Reject::Io),
    };
    if meta.file_type().is_symlink() {
        return Err(Reject::Symlink);
    }
    if !meta.is_file() {
        return Err(Reject::NotRegular);
    }
    if meta.len() > MAX_USER_FILE_BYTES {
        return Err(Reject::TooLarge);
    }
    std::fs::File::open(path).map(Some).map_err(|_| Reject::Io)
}

pub(crate) fn read(path: &Path) -> Result<Option<String>, Reject> {
    let Some(file) = open(path)? else { return Ok(None) };
    let mut bytes = Vec::new();
    // fstat ile okuma arasında dosya büyüyebilir: okuma da sınırlı.
    file.take(MAX_USER_FILE_BYTES + 1).read_to_end(&mut bytes).map_err(|_| Reject::Io)?;
    if bytes.len() as u64 > MAX_USER_FILE_BYTES {
        return Err(Reject::TooLarge);
    }
    String::from_utf8(bytes).map(Some).map_err(|_| Reject::NotUtf8)
}
