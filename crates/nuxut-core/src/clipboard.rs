//! Platformdan bağımsız pano tipleri. Platform crate'leri (nuxut-wayland, ileride diğerleri)
//! panoyu bunlarla sunar; MIME seçimi ve çözme kuralları burada, tek yerde ve testli.
//!
//! Yalnız metin: `text/plain;charset=utf-8` (tercih) ve `text/plain`. İçerik kullanıcı
//! verisidir: [`ClipboardText`]'in `Debug`'ı yalnız uzunluk yazar ve hata tipleri içerik taşımaz.

use std::fmt;
use std::time::Duration;

/// Hangi seçim: sistem panosu (Ctrl+C/V) ya da birincil seçim (seç + orta tık).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Selection {
    Clipboard,
    Primary,
}

/// Tercih edilen metin türü.
pub const MIME_TEXT_UTF8: &str = "text/plain;charset=utf-8";
/// Yedek metin türü (karakter kümesi belirtilmemiş; UTF-8 olarak doğrulanır).
pub const MIME_TEXT: &str = "text/plain";
/// Kopyalarken sunulan türler (tercih sırasıyla).
pub const OFFERED_MIMES: [&str; 2] = [MIME_TEXT_UTF8, MIME_TEXT];

/// Yapıştırmada okunacak en çok bayt; aşılırsa [`PasteError::TooLarge`].
pub const MAX_PASTE_BYTES: usize = 4 << 20;
/// Yapıştırma okumasının tamamlanması için süre; aşılırsa [`PasteError::Timeout`].
pub const PASTE_TIMEOUT: Duration = Duration::from_secs(2);
/// Başka bir istemciye kendi seçimimizi yazma süresi; aşılırsa boru kapatılır.
pub const COPY_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Sunulan türlerden okunacak olanı seçer; metin yoksa `None`. Büyük/küçük harf ve
/// `charset` yazımı farklarına toleranslıdır (`text/plain; charset=UTF-8`).
pub fn preferred_mime(offered: &[String]) -> Option<&str> {
    let normalized = |m: &str| m.to_ascii_lowercase().replace(' ', "");
    offered
        .iter()
        .find(|m| normalized(m) == MIME_TEXT_UTF8)
        .or_else(|| offered.iter().find(|m| normalized(m) == MIME_TEXT))
        .map(String::as_str)
}

/// Okunan baytları metne çevirir. Geçersiz UTF-8 reddedilir (kayıplı dönüştürme yok:
/// sessizce bozulmuş metin yapıştırılmaz).
pub fn decode(bytes: Vec<u8>) -> Result<ClipboardText, PasteError> {
    if bytes.len() > MAX_PASTE_BYTES {
        return Err(PasteError::TooLarge);
    }
    String::from_utf8(bytes)
        .map(ClipboardText)
        .map_err(|_| PasteError::InvalidUtf8)
}

/// Panodan okunmuş metin. `Clone` yok (kopyası sızmaz); `Debug` yalnız uzunluk yazar.
#[derive(PartialEq, Eq)]
pub struct ClipboardText(String);

impl ClipboardText {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for ClipboardText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClipboardText(<{} bayt>)", self.0.len())
    }
}

/// Yapıştırma başarısız. İçerik taşımaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteError {
    /// Platform/compositor bu seçimi desteklemiyor (ya da koltuk yok).
    Unsupported,
    /// Seçim boş.
    Empty,
    /// Seçimde metin türü yok.
    NotText,
    /// Oturum kilidi istenmiş/kurulu: pano kullanılmaz.
    Locked,
    /// [`MAX_PASTE_BYTES`] aşıldı.
    TooLarge,
    /// [`PASTE_TIMEOUT`] içinde bitmedi.
    Timeout,
    /// Geçerli UTF-8 değil.
    InvalidUtf8,
    /// Boru/okuma hatası.
    Io,
}

impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "pano desteklenmiyor",
            Self::Empty => "pano boş",
            Self::NotText => "panoda metin yok",
            Self::Locked => "oturum kilitliyken pano kullanılmaz",
            Self::TooLarge => "pano içeriği çok büyük",
            Self::Timeout => "pano okuması zaman aşımına uğradı",
            Self::InvalidUtf8 => "pano içeriği geçerli UTF-8 değil",
            Self::Io => "pano okunamadı",
        })
    }
}

impl std::error::Error for PasteError {}

/// Kopyalama başarısız. İçerik taşımaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyError {
    /// Platform/compositor bu seçimi desteklemiyor (ya da koltuk yok).
    Unsupported,
    /// Oturum kilidi istenmiş/kurulu: pano kullanılmaz.
    Locked,
    /// Seçimi sahiplenmek için geçerli bir girdi olayı (tuş ya da tıklama) yok.
    NoInput,
}

impl fmt::Display for CopyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "pano desteklenmiyor",
            Self::Locked => "oturum kilitliyken pano kullanılmaz",
            Self::NoInput => "panoya yazmak için girdi olayı gerekli",
        })
    }
}

impl std::error::Error for CopyError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn mimes(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn prefers_utf8_then_plain_and_ignores_others() {
        let offered = mimes(&["image/png", "text/plain", "text/plain;charset=utf-8"]);
        assert_eq!(preferred_mime(&offered), Some("text/plain;charset=utf-8"));
        let offered = mimes(&["text/html", "text/plain"]);
        assert_eq!(preferred_mime(&offered), Some("text/plain"));
        let offered = mimes(&["text/plain; charset=UTF-8"]);
        assert_eq!(preferred_mime(&offered), Some("text/plain; charset=UTF-8"));
        // Benzer ama farklı türler metin sayılmaz.
        let offered = mimes(&["text/html", "UTF8_STRING", "text/plain-x", "text/uri-list"]);
        assert_eq!(preferred_mime(&offered), None);
        assert_eq!(preferred_mime(&[]), None);
    }

    #[test]
    fn decode_validates_utf8_and_size() {
        assert_eq!(
            decode("merhaba dünya".into()).unwrap().as_str(),
            "merhaba dünya"
        );
        assert_eq!(decode(vec![0xff, 0xfe]), Err(PasteError::InvalidUtf8));
        assert_eq!(
            decode(vec![b'a'; MAX_PASTE_BYTES]).map(|t| t.len()),
            Ok(MAX_PASTE_BYTES)
        );
        assert_eq!(
            decode(vec![b'a'; MAX_PASTE_BYTES + 1]),
            Err(PasteError::TooLarge)
        );
    }

    #[test]
    fn debug_and_errors_hide_content() {
        let text = ClipboardText::new("gizli-parola".into());
        let shown = format!("{text:?}");
        assert!(!shown.contains("gizli"), "{shown}");
        assert!(shown.contains("12 bayt"), "{shown}");
        assert_eq!(text.into_string(), "gizli-parola");
    }
}
