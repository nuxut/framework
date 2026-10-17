//! Tam satır parçası: `source = ~/.config/hypr/nuxut.conf`, `@import 'nuxut.css';`.
//!
//! Eşleşme: satırın baştaki/sondaki boşluk atılmış hâli `marker`'a eşit. Değer var/yok'tur:
//! `set(Some)` satır yoksa ekler ([`Position::Start`] en başa — CSS `@import` için —, `End`
//! sona), varsa dokunmaz; `set(None)` eşleşen bütün satırları siler. Ekle → sil bayt bayt
//! eski hâli verir (dosya satır sonusuz bitse de).

use super::{EditError, Lines};
use crate::model::Position;

/// Satır varsa `Some(marker)`.
pub fn get(text: &str, marker: &str) -> Option<String> {
    Lines::parse(text)
        .lines
        .iter()
        .any(|(l, _)| l.trim() == marker)
        .then(|| marker.to_owned())
}

pub fn set(text: &str, marker: &str, present: bool, at: Position) -> Result<String, EditError> {
    if marker.is_empty() || marker.contains(['\n', '\r', '\0']) || marker.trim() != marker {
        return Err(EditError::InvalidValue);
    }
    let mut lines = Lines::parse(text);
    let found: Vec<usize> = lines
        .lines
        .iter()
        .enumerate()
        .filter(|(_, (l, _))| l.trim() == marker)
        .map(|(i, _)| i)
        .collect();
    match (present, found.is_empty()) {
        (true, false) | (false, true) => return Ok(text.to_owned()),
        (true, true) => {
            let idx = match at {
                Position::Start => 0,
                Position::End => lines.lines.len(),
            };
            lines.insert(idx, marker.to_owned());
        }
        (false, false) => {
            for i in found.into_iter().rev() {
                lines.remove(i);
            }
        }
    }
    Ok(lines.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_roundtrip() {
        let m = "source = ~/.config/hypr/nuxut.conf";
        for original in ["", "a = 1", "a = 1\n", "# x\r\nb\r\n", "\n\n"] {
            for at in [Position::Start, Position::End] {
                let added = set(original, m, true, at).unwrap();
                assert_eq!(get(&added, m).as_deref(), Some(m));
                assert!(added.contains(original.trim_end_matches(['\n', '\r'])));
                assert_eq!(
                    set(&added, m, false, at).unwrap(),
                    original,
                    "{original:?} {at:?}"
                );
                // İdempotent.
                assert_eq!(set(&added, m, true, at).unwrap(), added);
            }
        }
        assert_eq!(
            set("a\n", m, true, Position::End).unwrap(),
            format!("a\n{m}\n")
        );
        assert_eq!(
            set("a {}\n", "@import 'nuxut.css';", true, Position::Start).unwrap(),
            "@import 'nuxut.css';\na {}\n"
        );
    }

    #[test]
    fn indented_match_and_removal_of_duplicates() {
        let m = "source = x";
        let t = "a\n  source = x  \nb\nsource = x\n";
        assert_eq!(get(t, m).as_deref(), Some(m));
        assert_eq!(set(t, m, false, Position::End).unwrap(), "a\nb\n");
        // Yorumlanmış satır eşleşmez.
        assert_eq!(get("# source = x\n", m), None);
    }

    #[test]
    fn rejects_bad_marker() {
        assert_eq!(
            set("", "a\nb", true, Position::End),
            Err(EditError::InvalidValue)
        );
        assert_eq!(
            set("", " a", true, Position::End),
            Err(EditError::InvalidValue)
        );
    }
}
