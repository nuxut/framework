//! Satır tabanlı INI düzenleyici: GTK `settings.ini`, qt5ct/qt6ct, `kdeglobals`, Kvantum,
//! imleç `index.theme`.
//!
//! - Yorum (`#`, `;`), boş satır, sıra, girinti, satır sonları ve bilinmeyen satırlar dokunulmadan
//!   kalır. Değiştirilen satırda yalnızca `=`'den sonraki değer değişir (anahtar ve boşluk biçimi
//!   korunur).
//! - Anahtar eşleşmesi `kdeglobals` bayrak eklerini tanır: `key[$e]=…`, `key[$i]=…` `key` ile
//!   eşleşir ve ek korunur. Yerel ekleri (`Name[de]`) ayrı anahtardır.
//! - `section = ""` ilk başlıktan önceki alan. Bölüm adı başlığın köşeli parantezler arası
//!   metnidir (`[Colors:Window]` → `Colors:Window`; iç içe `[A][B]` → `A][B`).
//! - Aynı anahtar birden çok kez varsa: `get` sonuncuyu döndürür, `set` hepsini günceller/siler.
//! - Eksik anahtar bölümün son anahtarından sonra `key=value` olarak eklenir; eksik bölüm dosya
//!   sonuna eklenir; dosya boş değilse önüne her zaman tam bir boş satır konur (dosya boş satırla
//!   bitse de — silmenin bayt bayt tersi olabilsin diye). Anahtar silinince bölüm dosyanın **son** bölümüyse ve
//!   gövdesinde yalnızca boş satır kaldıysa başlık ve önündeki tek boş satır da silinir (eklemenin
//!   tam tersi). İstisna: kullanıcının dosya sonunda boş bıraktığı bölüm de bu durumda kalkar
//!   (INI'de anlamsız, kabul edildi). `set` ortadaki boş bölümlere dokunmaz; Nuxut'un yarattığı
//!   (depoda `section_created`) ortadaki bölümleri applier [`remove_empty_section`] ile kaldırır.

use super::{EditError, Lines};

#[derive(Debug, PartialEq, Eq)]
enum Kind<'a> {
    Section(&'a str),
    /// Anahtar (bayrak ekleri ayıklanmış) ve değerin satırdaki başlangıç baytı.
    Key {
        key: &'a str,
        value_at: usize,
    },
    Other,
}

fn classify(line: &str) -> Kind<'_> {
    let t = line.trim_start();
    if t.is_empty() || t.starts_with('#') || t.starts_with(';') {
        return Kind::Other;
    }
    if t.starts_with('[') {
        let te = t.trim_end();
        if te.ends_with(']') && te.len() >= 2 {
            return Kind::Section(&te[1..te.len() - 1]);
        }
        return Kind::Other;
    }
    let Some(eq) = line.find('=') else {
        return Kind::Other;
    };
    let mut key = line[..eq].trim();
    // kdeglobals bayrakları: `key[$e]`, `key[$i]`, `key[$ei]` (birden çok olabilir).
    while key.ends_with(']') {
        match key.rfind("[$") {
            Some(i) if !key[i + 2..key.len() - 1].contains(['[', ']']) => key = key[..i].trim_end(),
            _ => break,
        }
    }
    let after = &line[eq + 1..];
    let value_at = eq + 1 + (after.len() - after.trim_start().len());
    Kind::Key { key, value_at }
}

fn value_of(line: &str, value_at: usize) -> &str {
    line[value_at..].trim_end()
}

/// Bölümün satır aralığı: (başlık indeksi ya da `None` üst alan için, gövde başlangıcı, bitiş).
fn section_range(lines: &Lines, section: &str) -> Option<(Option<usize>, usize, usize)> {
    let mut start = if section.is_empty() {
        Some((None, 0))
    } else {
        None
    };
    for (i, (l, _)) in lines.lines.iter().enumerate() {
        if let Kind::Section(name) = classify(l) {
            if let Some((h, body)) = start {
                return Some((h, body, i));
            }
            if name == section {
                start = Some((Some(i), i + 1));
            }
        }
    }
    start.map(|(h, body)| (h, body, lines.lines.len()))
}

/// `[section]` başlığı var mı? (`""` üst alan her zaman var sayılır.)
pub fn has_section(text: &str, section: &str) -> bool {
    section_range(&Lines::parse(text), section).is_some()
}

/// Boş kalmış bölümü kaldırır: gövdesinde yalnızca boş satır varsa (anahtar, yorum ya da tanınmayan
/// satır yoksa) başlık ve gövdesi silinir. Bölüm dosyanın sonundaysa önündeki tek boş satır da
/// gider (eklemenin tam tersi); ortadaysa önündeki boş satır sonraki bölümün ayırıcısı olarak kalır.
/// Üst alan (`""`), olmayan ya da dolu bölüm: metin aynen döner. Kimin yarattığına çağıran karar
/// verir (yalnızca Nuxut'un yarattığı bölüm için çağrılır).
pub fn remove_empty_section(text: &str, section: &str) -> String {
    if section.is_empty() {
        return text.to_owned();
    }
    let mut lines = Lines::parse(text);
    let Some((Some(h), body, end)) = section_range(&lines, section) else {
        return text.to_owned();
    };
    if !lines.lines[body..end]
        .iter()
        .all(|(l, _)| l.trim().is_empty())
    {
        return text.to_owned();
    }
    let last = end == lines.lines.len();
    for i in (h..end).rev() {
        lines.remove(i);
    }
    if last && h > 0 && h == lines.lines.len() && lines.lines[h - 1].0.trim().is_empty() {
        lines.remove(h - 1);
    }
    lines.render()
}

/// `[section] key` değeri (son eşleşme).
pub fn get(text: &str, section: &str, key: &str) -> Option<String> {
    let lines = Lines::parse(text);
    let (_, body, end) = section_range(&lines, section)?;
    lines.lines[body..end]
        .iter()
        .rev()
        .find_map(|(l, _)| match classify(l) {
            Kind::Key { key: k, value_at } if k == key => Some(value_of(l, value_at).to_owned()),
            _ => None,
        })
}

/// `[section] key`'i `value`'ya ayarlar (`None` = sil).
pub fn set(text: &str, section: &str, key: &str, value: Option<&str>) -> Result<String, EditError> {
    if let Some(v) = value
        && v.chars().any(|c| c == '\n' || c == '\r' || c == '\0')
    {
        return Err(EditError::InvalidValue);
    }
    let mut lines = Lines::parse(text);
    let range = section_range(&lines, section);
    let matches: Vec<(usize, usize)> = match range {
        Some((_, body, end)) => (body..end)
            .filter_map(|i| match classify(&lines.lines[i].0) {
                Kind::Key { key: k, value_at } if k == key => Some((i, value_at)),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    };

    match value {
        Some(v) => {
            if !matches.is_empty() {
                for (i, at) in matches {
                    let line = &mut lines.lines[i].0;
                    // Değerin ardındaki boşluk korunur.
                    let old_len = value_of(line, at).len();
                    let tail = line[at + old_len..].to_owned();
                    line.truncate(at);
                    line.push_str(v);
                    line.push_str(&tail);
                }
            } else if let Some((h, body, end)) = range {
                // Bölümün son anahtarından (yoksa başlıktan) sonra.
                let last_key = (body..end)
                    .rev()
                    .find(|&i| matches!(classify(&lines.lines[i].0), Kind::Key { .. }));
                let at = match (last_key, h) {
                    (Some(i), _) => i + 1,
                    (None, Some(h)) => h + 1,
                    (None, None) => 0,
                };
                lines.insert(at, format!("{key}={v}"));
            } else {
                // Bölüm yok: sona, tam olarak bir boş satırla ayrılarak — dosya zaten boş satırla
                // bitse de (silmede yalnızca bu satır gider: sondaki kullanıcı boş satırı kalır).
                let n = lines.lines.len();
                if n > 0 {
                    lines.insert(n, String::new());
                }
                let n = lines.lines.len();
                lines.insert(n, format!("[{section}]"));
                let n = lines.lines.len();
                lines.insert(n, format!("{key}={v}"));
            }
        }
        None => {
            if matches.is_empty() {
                return Ok(text.to_owned());
            }
            for (i, _) in matches.iter().rev() {
                lines.remove(*i);
            }
            // Dosyanın son bölümü boş kaldıysa başlığı da kaldır (eklemenin tersi).
            if let Some((Some(h), body, end)) = section_range(&lines, section) {
                let last = end == lines.lines.len();
                let only_blank = lines.lines[body..end]
                    .iter()
                    .all(|(l, _)| l.trim().is_empty());
                if last && only_blank {
                    for i in (h..end).rev() {
                        lines.remove(i);
                    }
                    if h > 0 && lines.lines[h - 1].0.trim().is_empty() && h == lines.lines.len() {
                        lines.remove(h - 1);
                    }
                }
            }
        }
    }
    Ok(lines.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GTK: &str = "# kullanıcı yorumu\r\n[Settings]\r\ngtk-theme-name = Adwaita-dark   \r\n; noktalı virgül yorumu\r\ngtk-icon-theme-name=Papirus\r\n\r\n[Other]\r\nx=1";

    #[test]
    fn get_values() {
        assert_eq!(
            get(GTK, "Settings", "gtk-theme-name").as_deref(),
            Some("Adwaita-dark")
        );
        assert_eq!(
            get(GTK, "Settings", "gtk-icon-theme-name").as_deref(),
            Some("Papirus")
        );
        assert_eq!(get(GTK, "Other", "x").as_deref(), Some("1"));
        assert_eq!(get(GTK, "Settings", "x"), None);
        assert_eq!(get(GTK, "Yok", "x"), None);
        assert_eq!(get("a=1\n[S]\na=2\n", "", "a").as_deref(), Some("1"));
        assert_eq!(get("[S]\na=1\na=2\n", "S", "a").as_deref(), Some("2"));
    }

    #[test]
    fn replace_keeps_everything_else_bytewise() {
        let out = set(GTK, "Settings", "gtk-theme-name", Some("Nuxut")).unwrap();
        assert_eq!(out, GTK.replace("= Adwaita-dark   ", "= Nuxut   "));
        // Geri: bayt bayt aynı.
        assert_eq!(
            set(&out, "Settings", "gtk-theme-name", Some("Adwaita-dark")).unwrap(),
            GTK
        );
    }

    #[test]
    fn insert_and_remove_roundtrip() {
        for original in [
            GTK,
            "",
            "a=1",
            "a=1\n",
            "[Settings]\n",
            "[Settings]\n\n[B]\nk=v\n",
            "# yalnız yorum",
            "a=1\n\n",
            "\n",
            "[Settings]\nx=1\n\n\n",
            "a=1\r\n\r\n",
        ] {
            for (sec, key) in [
                ("Settings", "gtk-cursor-theme-name"),
                ("New", "k"),
                ("Other", "y"),
                ("", "top"),
            ] {
                let added = set(original, sec, key, Some("v")).unwrap();
                assert_eq!(
                    get(&added, sec, key).as_deref(),
                    Some("v"),
                    "{original:?} {sec} {key}"
                );
                let back = set(&added, sec, key, None).unwrap();
                if original == "[Settings]\n" && sec == "Settings" {
                    // Kullanıcının boş bölümü de kalkar (belgelenmiş istisna).
                    assert_eq!(back, "", "{added:?}");
                } else {
                    assert_eq!(back, original, "{original:?} [{sec}] {key} → {added:?}");
                }
            }
        }
    }

    #[test]
    fn insert_position_and_style() {
        let out = set(GTK, "Settings", "gtk-cursor-theme-name", Some("Bibata")).unwrap();
        assert!(
            out.contains(
                "gtk-icon-theme-name=Papirus\r\ngtk-cursor-theme-name=Bibata\r\n\r\n[Other]"
            ),
            "{out:?}"
        );
        let out = set("[A]\nk=1\n\n# B'nin yorumu\n[B]\n", "A", "n", Some("2")).unwrap();
        assert_eq!(out, "[A]\nk=1\nn=2\n\n# B'nin yorumu\n[B]\n");
        let out = set("x=1\n", "New", "k", Some("v")).unwrap();
        assert_eq!(out, "x=1\n\n[New]\nk=v\n");
        let out = set("[A]\n", "A", "k", Some("v")).unwrap();
        assert_eq!(out, "[A]\nk=v\n");
    }

    #[test]
    fn kdeglobals_flags_preserved() {
        let kde = "[General]\nColorScheme[$e]=BreezeDark\nName[de]=Dunkel\nName=Dark\nwidgetStyle[$i]=Breeze\n[Colors:Window]\nBackgroundNormal=1,2,3\n";
        assert_eq!(
            get(kde, "General", "ColorScheme").as_deref(),
            Some("BreezeDark")
        );
        assert_eq!(get(kde, "General", "Name").as_deref(), Some("Dark"));
        assert_eq!(
            get(kde, "General", "widgetStyle").as_deref(),
            Some("Breeze")
        );
        let out = set(kde, "General", "ColorScheme", Some("Nuxut")).unwrap();
        assert_eq!(
            out,
            kde.replace("ColorScheme[$e]=BreezeDark", "ColorScheme[$e]=Nuxut")
        );
        let out = set(kde, "General", "Name", Some("N")).unwrap();
        assert_eq!(out, kde.replace("Name=Dark", "Name=N"));
        let out = set(kde, "Colors:Window", "BackgroundNormal", Some("4,5,6")).unwrap();
        assert!(out.ends_with("[Colors:Window]\nBackgroundNormal=4,5,6\n"));
    }

    #[test]
    fn duplicates_all_updated_and_removed() {
        let t = "[S]\na=1\nb=2\na = 3\n";
        assert_eq!(
            set(t, "S", "a", Some("9")).unwrap(),
            "[S]\na=9\nb=2\na = 9\n"
        );
        assert_eq!(set(t, "S", "a", None).unwrap(), "[S]\nb=2\n");
    }

    #[test]
    fn rejects_multiline_and_tolerates_garbage() {
        assert_eq!(
            set("[S]\n", "S", "a", Some("1\n[x]")),
            Err(EditError::InvalidValue)
        );
        // Çözülemeyen satırlar dokunulmadan kalır.
        let junk = "garbage line\n[unclosed\n[S]\n=novalue\na=1\n";
        assert_eq!(
            set(junk, "S", "a", Some("2")).unwrap(),
            junk.replace("a=1", "a=2")
        );
        assert_eq!(get(junk, "S", "a").as_deref(), Some("1"));
    }

    #[test]
    fn removing_absent_is_identity() {
        assert_eq!(set(GTK, "Settings", "yok", None).unwrap(), GTK);
        assert_eq!(set(GTK, "Yok", "yok", None).unwrap(), GTK);
    }
}
