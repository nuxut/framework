//! İşaretli blok: `# >>> nuxut:<id>` … `# <<< nuxut:<id>` (satırlar boşluk atılarak eşleşir).
//!
//! Değer, işaretler arasındaki metindir ve satır sonuyla biter ([`normalize`]). Eksik blok sona
//! eklenir; silinince işaretlerle birlikte kalkar (ekle → sil bayt bayt eski hâl). Açılıp
//! kapanmayan, ters sıralı ya da yinelenen işaretler [`EditError::Malformed`]: dosyaya dokunulmaz.

use super::{EditError, Lines};

pub fn begin_marker(id: &str) -> String {
    format!("# >>> nuxut:{id}")
}

pub fn end_marker(id: &str) -> String {
    format!("# <<< nuxut:{id}")
}

/// Değerin saklanan biçimi: boş değilse `\n` ile biter.
pub fn normalize(inner: &str) -> String {
    if inner.is_empty() || inner.ends_with('\n') {
        inner.to_owned()
    } else {
        format!("{inner}\n")
    }
}

/// (başlangıç satırı, bitiş satırı) ya da `None`.
fn find(lines: &Lines, id: &str) -> Result<Option<(usize, usize)>, EditError> {
    let (b, e) = (begin_marker(id), end_marker(id));
    let begins: Vec<usize> = lines
        .lines
        .iter()
        .enumerate()
        .filter(|(_, (l, _))| l.trim() == b)
        .map(|(i, _)| i)
        .collect();
    let ends: Vec<usize> = lines
        .lines
        .iter()
        .enumerate()
        .filter(|(_, (l, _))| l.trim() == e)
        .map(|(i, _)| i)
        .collect();
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([s], [t]) if s < t => Ok(Some((*s, *t))),
        _ => Err(EditError::Malformed),
    }
}

/// Bloğun iç metni; satır sonları `\n`'e normalleşir (yazım dosyanın satır sonunu kullanır).
pub fn get(text: &str, id: &str) -> Result<Option<String>, EditError> {
    let lines = Lines::parse(text);
    Ok(find(&lines, id)?.map(|(s, t)| {
        lines.lines[s + 1..t]
            .iter()
            .map(|(l, _)| format!("{l}\n"))
            .collect()
    }))
}

pub fn set(text: &str, id: &str, inner: Option<&str>) -> Result<String, EditError> {
    let mut lines = Lines::parse(text);
    let found = find(&lines, id)?;
    let inner = inner.map(normalize);
    if let Some(v) = &inner {
        let (b, e) = (begin_marker(id), end_marker(id));
        if v.contains('\0') || v.lines().any(|l| l.trim() == b || l.trim() == e) {
            return Err(EditError::InvalidValue);
        }
    }
    let inner_lines = |v: &str| -> Vec<String> {
        v.lines()
            .map(|l| l.trim_end_matches('\r').to_owned())
            .collect()
    };
    match (found, inner) {
        (None, None) => return Ok(text.to_owned()),
        (Some((s, t)), None) => {
            for i in (s..=t).rev() {
                lines.remove(i);
            }
        }
        (Some((s, t)), Some(v)) => {
            for i in (s + 1..t).rev() {
                lines.remove(i);
            }
            for (k, l) in inner_lines(&v).into_iter().enumerate() {
                lines.insert(s + 1 + k, l);
            }
        }
        (None, Some(v)) => {
            let n = lines.lines.len();
            lines.insert(n, begin_marker(id));
            for l in inner_lines(&v) {
                let n = lines.lines.len();
                lines.insert(n, l);
            }
            let n = lines.lines.len();
            lines.insert(n, end_marker(id));
        }
    }
    Ok(lines.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_replace_remove_roundtrip() {
        for original in ["", "a = 1", "a = 1\n", "x\r\ny\r\n"] {
            let added = set(original, "look", Some("k = 1\nj = 2")).unwrap();
            assert_eq!(
                get(&added, "look").unwrap().as_deref(),
                Some("k = 1\nj = 2\n"),
                "{added:?}"
            );
            let replaced = set(&added, "look", Some("k = 3\n")).unwrap();
            assert_eq!(get(&replaced, "look").unwrap().as_deref(), Some("k = 3\n"));
            assert_eq!(
                set(&replaced, "look", Some("k = 1\nj = 2\n")).unwrap(),
                added
            );
            assert_eq!(
                set(&replaced, "look", None).unwrap(),
                original,
                "{original:?}"
            );
        }
        assert_eq!(
            set("a\n", "x", Some("b")).unwrap(),
            "a\n# >>> nuxut:x\nb\n# <<< nuxut:x\n"
        );
        // Boş iç metin.
        let e = set("a\n", "x", Some("")).unwrap();
        assert_eq!(get(&e, "x").unwrap().as_deref(), Some(""));
        assert_eq!(set(&e, "x", None).unwrap(), "a\n");
    }

    #[test]
    fn surrounding_bytes_preserved_on_replace() {
        let t = "pre\r\n  # >>> nuxut:x\r\nold\r\n# <<< nuxut:x  \r\npost";
        let out = set(t, "x", Some("new")).unwrap();
        assert_eq!(
            out,
            "pre\r\n  # >>> nuxut:x\r\nnew\r\n# <<< nuxut:x  \r\npost"
        );
    }

    #[test]
    fn malformed_refused() {
        for bad in [
            "# >>> nuxut:x\n",
            "# <<< nuxut:x\n# >>> nuxut:x\n",
            "# >>> nuxut:x\n# <<< nuxut:x\n# >>> nuxut:x\n# <<< nuxut:x\n",
        ] {
            assert_eq!(set(bad, "x", Some("a")), Err(EditError::Malformed));
            assert_eq!(get(bad, "x"), Err(EditError::Malformed));
        }
        // Başka kimlikli blok etkilemez.
        assert_eq!(get("# >>> nuxut:y\n", "x"), Ok(None));
        // Değer işaret içeremez.
        assert_eq!(
            set("", "x", Some("# <<< nuxut:x\nexec = evil")),
            Err(EditError::InvalidValue)
        );
    }
}
