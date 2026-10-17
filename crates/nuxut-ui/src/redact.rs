//! Kullanıcı içeriği taşıyan tiplerin `Debug`'ı (ARCHITECTURE değişmez 9): metin yerine
//! yalnızca bayt uzunluğu yazılır. Bileşenler `#[derive(Debug)]` yerine bunlarla elle
//! `Debug` yazar; böylece `?event` / `{:?}` hiçbir log seviyesinde içerik sızdırmaz.

use std::fmt;

/// Metnin yerine `<N bayt>`.
pub(crate) struct Len<'a>(pub &'a str);

impl fmt::Debug for Len<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} bayt>", self.0.len())
    }
}

/// İsteğe bağlı metin: `None` ya da `Some(<N bayt>)`.
pub(crate) struct OptLen<'a>(pub Option<&'a str>);

impl fmt::Debug for OptLen<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            None => f.write_str("None"),
            Some(s) => write!(f, "Some(<{} bayt>)", s.len()),
        }
    }
}

/// Metin listesi: yalnızca öğe sayısı.
pub(crate) struct Count(pub usize);

impl fmt::Debug for Count {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} öğe>", self.0)
    }
}

#[cfg(test)]
pub(crate) const SECRET: &str = "gizli-icerik-42";

/// Testler: `Debug` çıktısında sızdırılmaması gereken işaret metni yok mu.
#[cfg(test)]
pub(crate) fn assert_redacted(value: &impl fmt::Debug) {
    let s = format!("{value:?}");
    let alt = format!("{value:#?}");
    assert!(!s.contains("gizli") && !alt.contains("gizli"), "Debug içerik sızdırıyor: {s}");
}
