//! TOML anahtarı (`toml_edit`): yorumlar, boşluklar, sıra ve değer yazımı (tırnak türü, sayı
//! biçimi) korunur. Değer bir TOML **değer ifadesidir** (`"Adwaita"`, `true`, `12`, `[1, 2]`);
//! `get` mevcut ifadeyi kaynaktaki yazımıyla döndürür, böylece geri yükleme bayt bayt aynıdır.
//!
//! - Var olan anahtar değişince eski değerin çevresi (satır sonu yorumu, boşluk) korunur.
//! - Eksik anahtarın ara tabloları yaratılır (satır içi tablonun içindeyse satır içi).
//! - Silinince boş kalan ve yorumsuz (yalnız boşluk süslü) ata tablolar da silinir (eklemenin
//!   tersi; kullanıcının boş, yorumsuz tablosu da bu durumda kalkar — kabul edildi). Satır
//!   sonusuz biten dosyaya tablo eklenip silinince sonda bir `\n` kalabilir (`toml_edit`).
//! - Dizi-tablolar (`[[x]]`) içine yol desteklenmez → [`EditError::NotAValue`].

use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use super::EditError;
use crate::model::KeyPath;

fn parse(text: &str) -> Result<DocumentMut, EditError> {
    text.parse::<DocumentMut>().map_err(|_| EditError::Parse)
}

/// Anahtarın değer ifadesi (yoksa `None`). Tabloya işaret ediyorsa `NotAValue`.
pub fn get(text: &str, key: &KeyPath) -> Result<Option<String>, EditError> {
    let doc = parse(text)?;
    let mut item = doc.as_item();
    for seg in key.segments() {
        let table = item.as_table_like().ok_or(EditError::NotAValue)?;
        match table.get(seg) {
            Some(i) => item = i,
            None => return Ok(None),
        }
    }
    match item {
        Item::None => Ok(None),
        Item::Value(v) => {
            let mut v = v.clone();
            v.decor_mut().clear();
            Ok(Some(v.to_string()))
        }
        Item::Table(_) | Item::ArrayOfTables(_) => Err(EditError::NotAValue),
    }
}

/// Değer ifadesi geçerli mi (tek bir TOML değeri)?
pub fn parse_value(literal: &str) -> Result<Value, EditError> {
    if literal.len() > crate::model::MAX_TEXT_BYTES || literal.contains(['\n', '\r']) {
        return Err(EditError::InvalidValue);
    }
    literal
        .trim()
        .parse::<Value>()
        .map_err(|_| EditError::InvalidValue)
}

/// Anahtarı `literal` değer ifadesine ayarlar (`None` = sil).
pub fn set(text: &str, key: &KeyPath, literal: Option<&str>) -> Result<String, EditError> {
    let mut doc = parse(text)?;
    let (last, parents) = key.segments().split_last().ok_or(EditError::NotAValue)?;
    match literal {
        Some(lit) => {
            let mut nv = parse_value(lit)?;
            let mut cur: &mut Item = doc.as_item_mut();
            for seg in parents {
                let inline = cur.is_inline_table();
                let table = cur.as_table_like_mut().ok_or(EditError::NotAValue)?;
                if !table.contains_key(seg) {
                    let item = if inline {
                        Item::Value(Value::InlineTable(InlineTable::new()))
                    } else {
                        let mut t = Table::new();
                        t.set_implicit(true);
                        Item::Table(t)
                    };
                    table.insert(seg, item);
                }
                cur = table.get_mut(seg).ok_or(EditError::NotAValue)?;
            }
            let table = cur.as_table_like_mut().ok_or(EditError::NotAValue)?;
            match table.get_mut(last) {
                Some(Item::Value(old)) => {
                    if old.to_string() == {
                        let mut probe = nv.clone();
                        *probe.decor_mut() = old.decor().clone();
                        probe.to_string()
                    } {
                        return Ok(text.to_owned());
                    }
                    *nv.decor_mut() = old.decor().clone();
                    *old = nv;
                }
                Some(Item::None) | None => {
                    table.insert(last, Item::Value(nv));
                }
                Some(_) => return Err(EditError::NotAValue),
            }
        }
        None => {
            {
                let mut cur: &mut Item = doc.as_item_mut();
                for seg in parents {
                    cur = match cur.as_table_like_mut().and_then(|t| t.get_mut(seg)) {
                        Some(i) => i,
                        None => return Ok(text.to_owned()),
                    };
                }
                let table = cur.as_table_like_mut().ok_or(EditError::NotAValue)?;
                match table.get(last) {
                    None | Some(Item::None) => return Ok(text.to_owned()),
                    Some(Item::Value(_)) => {
                        table.remove(last);
                    }
                    Some(_) => return Err(EditError::NotAValue),
                }
            }
            // Boş kalan, yorumsuz ata tabloları derinden yukarı sil.
            for depth in (1..=parents.len()).rev() {
                let mut cur: &mut Item = doc.as_item_mut();
                for seg in &parents[..depth - 1] {
                    cur = cur
                        .as_table_like_mut()
                        .and_then(|t| t.get_mut(seg))
                        .ok_or(EditError::NotAValue)?;
                }
                let table = cur.as_table_like_mut().ok_or(EditError::NotAValue)?;
                let child = &parents[depth - 1];
                let removable = match table.get(child) {
                    Some(Item::Table(t)) => t.is_empty() && blank_decor(t.decor()),
                    Some(Item::Value(Value::InlineTable(t))) => t.is_empty(),
                    _ => false,
                };
                if !removable {
                    break;
                }
                table.remove(child);
            }
        }
    }
    Ok(doc.to_string())
}

fn blank_decor(d: &toml_edit::Decor) -> bool {
    let blank = |r: Option<&toml_edit::RawString>| {
        r.and_then(|r| r.as_str())
            .is_none_or(|s| s.trim().is_empty())
    };
    blank(d.prefix()) && blank(d.suffix())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> KeyPath {
        KeyPath::dotted(s).unwrap()
    }

    const DOC: &str = "# başlık yorumu\ntop = 'tek tırnak'  # satır sonu yorumu\n\n[look]\n# tema\ntheme   =   \"dark\"\nsize = 0x10\n\n[look.inner]\nx = { a = 1, b = 2 }\n";

    #[test]
    fn get_preserves_literal_spelling() {
        assert_eq!(
            get(DOC, &k("top")).unwrap().as_deref(),
            Some("'tek tırnak'")
        );
        assert_eq!(
            get(DOC, &k("look.theme")).unwrap().as_deref(),
            Some("\"dark\"")
        );
        assert_eq!(get(DOC, &k("look.size")).unwrap().as_deref(), Some("0x10"));
        assert_eq!(
            get(DOC, &k("look.inner.x.b")).unwrap().as_deref(),
            Some("2")
        );
        assert_eq!(get(DOC, &k("look.yok")).unwrap(), None);
        assert_eq!(get(DOC, &k("yok.yok")).unwrap(), None);
        assert_eq!(get(DOC, &k("look")), Err(EditError::NotAValue));
        assert_eq!(get(DOC, &k("top.x")), Err(EditError::NotAValue));
        assert_eq!(get("a = [", &k("a")), Err(EditError::Parse));
    }

    #[test]
    fn replace_keeps_comments_and_restores_bytewise() {
        let out = set(DOC, &k("look.theme"), Some("\"light\"")).unwrap();
        assert_eq!(out, DOC.replace("\"dark\"", "\"light\""));
        let back = set(&out, &k("look.theme"), Some("\"dark\"")).unwrap();
        assert_eq!(back, DOC);
        let out = set(DOC, &k("top"), Some("1")).unwrap();
        assert_eq!(out, DOC.replace("'tek tırnak'", "1"));
        assert_eq!(set(&out, &k("top"), Some("'tek tırnak'")).unwrap(), DOC);
        let out = set(DOC, &k("look.inner.x.a"), Some("5")).unwrap();
        assert_eq!(out, DOC.replace("a = 1", "a = 5"));
        // Aynı değer: metin aynen.
        assert_eq!(set(DOC, &k("look.size"), Some("0x10")).unwrap(), DOC);
    }

    #[test]
    fn insert_then_remove_roundtrip() {
        for original in [DOC, "", "a = 1\n", "a = 1", "[look]\n"] {
            for key in [
                "look.new",
                "fresh.key",
                "fresh.deep.key",
                "look.inner.x.c",
                "root_key",
            ] {
                if original == "[look]\n" && key == "look.inner.x.c" {
                    continue;
                }
                if !original.contains("[look.inner]") && key == "look.inner.x.c" {
                    continue;
                }
                let added = set(original, &k(key), Some("\"v\"")).unwrap();
                assert_eq!(
                    get(&added, &k(key)).unwrap().as_deref(),
                    Some("\"v\""),
                    "{original:?} {key}"
                );
                let back = set(&added, &k(key), None).unwrap();
                if original == "[look]\n" && key == "look.new" {
                    // Kullanıcının boş, yorumsuz tablosu da kalkar (belgelenmiş istisna).
                    assert_eq!(back, "");
                } else if !original.is_empty() && !original.ends_with('\n') {
                    // Satır sonusuz biten dosyaya tablo eklenince sonda `\n` kalabilir (istisna).
                    assert!(
                        back == original || back == format!("{original}\n"),
                        "{original:?} {key} → {back:?}"
                    );
                } else {
                    assert_eq!(back, original, "{original:?} {key}\n--- added:\n{added}");
                }
            }
        }
    }

    #[test]
    fn bad_values_and_shapes() {
        assert_eq!(
            set(DOC, &k("top"), Some("not toml")),
            Err(EditError::InvalidValue)
        );
        assert_eq!(
            set(DOC, &k("top"), Some("1\n[x]")),
            Err(EditError::InvalidValue)
        );
        assert_eq!(set(DOC, &k("look"), Some("1")), Err(EditError::NotAValue));
        assert_eq!(set(DOC, &k("top.x"), Some("1")), Err(EditError::NotAValue));
        assert_eq!(
            set("[[a]]\nb = 1\n", &k("a.b"), Some("2")),
            Err(EditError::NotAValue)
        );
        assert_eq!(set("x = [", &k("a"), Some("2")), Err(EditError::Parse));
        assert_eq!(set(DOC, &k("yok.yok"), None).unwrap(), DOC);
    }
}
