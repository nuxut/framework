//! Parça düzenleyicileri: saf metin → metin. Yönetilen parça dışındaki her bayt korunur
//! (yorumlar, sıra, boş satırlar, satır sonları, `[$e]` ekleri).
//!
//! Hepsi aynı biçimde: `get(metin, …) -> Option<değer>` ve `set(metin, …, Option<değer>) ->
//! yeni metin` (`None` = parçayı kaldır). `set` ardından aynı parçanın eski değeriyle `set`
//! çağrısı, dosyayı bayt bayt eski hâline döndürür (testli; istisnalar her modülde yazılı).

pub mod block;
pub mod gvariant;
pub mod ini;
pub mod line;
pub mod managed;
pub mod toml;

/// Düzenleyici hatası (içerik taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    /// Dosya sözdizimi çözümlenemedi.
    #[error("dosya çözümlenemedi")]
    Parse,
    /// Yol bir değere değil tabloya/diziye işaret ediyor (ya da ara bölüt tablo değil).
    #[error("anahtar bir değer değil")]
    NotAValue,
    /// İşaretli blok kapanmamış ya da iç içe.
    #[error("işaretli blok bozuk")]
    Malformed,
    /// Yazılacak değer geçersiz (çok satırlı, işaret içeriyor, TOML ifadesi değil).
    #[error("geçersiz değer")]
    InvalidValue,
}

/// Metni satırlara böler: (içerik, satır sonu) — satır sonu `"\n"`, `"\r\n"` ya da son satırda `""`.
pub(crate) fn split_lines(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(i) => {
                let (line, eol) = if i > 0 && rest.as_bytes()[i - 1] == b'\r' {
                    (&rest[..i - 1], &rest[i - 1..=i])
                } else {
                    (&rest[..i], &rest[i..=i])
                };
                out.push((line, eol));
                rest = &rest[i + 1..];
            }
            None => {
                out.push((rest, ""));
                rest = "";
            }
        }
    }
    out
}

/// Dosyanın satır sonu biçimi (ilk satıra göre).
pub(crate) fn detect_eol(lines: &[(&str, &str)]) -> &'static str {
    match lines.first() {
        Some((_, "\r\n")) => "\r\n",
        _ => "\n",
    }
}

/// Satır listesi (sahipli) üzerinde ekleme/silme; son satırın satır sonu kuralıyla:
/// dosya `\n` ile bitmiyorsa sona eklenen satır önceki satıra satır sonu verir, kendisi
/// satır sonusuz kalır; o satır silinince önceki satırın satır sonu geri alınır.
#[derive(Debug, Clone)]
pub(crate) struct Lines {
    pub lines: Vec<(String, String)>,
    pub eol: &'static str,
}

impl Lines {
    pub fn parse(text: &str) -> Self {
        let raw = split_lines(text);
        let eol = detect_eol(&raw);
        Self {
            lines: raw
                .into_iter()
                .map(|(l, e)| (l.to_owned(), e.to_owned()))
                .collect(),
            eol,
        }
    }

    pub fn render(&self) -> String {
        let mut s = String::with_capacity(self.lines.iter().map(|(l, e)| l.len() + e.len()).sum());
        for (l, e) in &self.lines {
            s.push_str(l);
            s.push_str(e);
        }
        s
    }

    /// `idx` konumuna satır ekler (`idx == len` → sona).
    pub fn insert(&mut self, idx: usize, content: String) {
        if idx == self.lines.len() {
            match self.lines.last_mut() {
                Some((_, e)) if e.is_empty() => {
                    // Dosya satır sonusuz bitiyordu: önceki satıra satır sonu, yenisi sonusuz.
                    *e = self.eol.to_owned();
                    self.lines.push((content, String::new()));
                }
                _ => self.lines.push((content, self.eol.to_owned())),
            }
        } else {
            self.lines.insert(idx, (content, self.eol.to_owned()));
        }
    }

    /// `idx`'teki satırı siler; son satır satır sonusuzsa önceki satırın satır sonu da kalkar.
    pub fn remove(&mut self, idx: usize) {
        let (_, eol) = self.lines.remove(idx);
        if eol.is_empty()
            && idx == self.lines.len()
            && let Some((_, prev)) = self.lines.last_mut()
        {
            prev.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_and_render_roundtrip() {
        for t in ["", "a", "a\n", "a\r\nb", "\n\n", "a\nb\r\n\r\nc", "x\r"] {
            assert_eq!(Lines::parse(t).render(), t, "{t:?}");
        }
        assert_eq!(split_lines("a\r\nb"), vec![("a", "\r\n"), ("b", "")]);
    }

    #[test]
    fn insert_remove_at_eof_is_exact() {
        for t in ["a", "a\n", ""] {
            let mut l = Lines::parse(t);
            let n = l.lines.len();
            l.insert(n, "x".into());
            l.remove(n);
            assert_eq!(l.render(), t, "{t:?}");
        }
    }
}
