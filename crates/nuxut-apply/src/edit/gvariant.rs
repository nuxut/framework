//! GVariant metin biçiminin anlamsal karşılaştırması (dconf parçaları).
//!
//! `dconf read` değeri GLib'in `g_variant_print` biçiminde döndürür: çiftler `%.17g` (+ gerekirse
//! `.0`), dizgeler tek tırnaklı. Yazılan metin (`1.00`, `"Adwaita"`) bu biçimde geri okunmaz;
//! bayt karşılaştırması aynı değeri "değişti" sayar ve her uygulamada yeniden yazım + sinyal
//! doğurur (shell docs/design/look-external.md §1: saniyede bir yeniden yazım döngüsü).
//!
//! Karşılaştırma belirteç düzeyindedir: sayılar değerleriyle (kesirli/üslü → `f64`, tamsayı →
//! `i128`; ikisi ayrı türdür — GVariant'ta `1` int32, `1.0` double), dizgeler kaçışları
//! çözülmüş içerikleriyle (tırnak türü önemsiz), diğer her şey (anahtar sözcükler, tür
//! ekleri, noktalama) olduğu gibi; dizge dışındaki boşluklar yok sayılır. Çözümlenemeyen metin
//! bayt eşitliğine düşer.

#[derive(Debug, PartialEq)]
enum Tok<'a> {
    Str(String),
    Int(i128),
    Float(f64),
    Word(&'a str),
    Punct(char),
}

/// İki GVariant metni aynı değeri mi gösteriyor (`"1.00"` == `"1.0"`, `"'a'"` == `"\"a\""`)?
pub fn equivalent(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (tokens(a), tokens(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

fn tokens(s: &str) -> Option<Vec<Tok<'_>>> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'\'' || c == b'"' {
            let (text, next) = string(s, i)?;
            out.push(Tok::Str(text));
            i = next;
        } else if c.is_ascii_digit()
            || ((c == b'-' || c == b'+' || c == b'.')
                && b.get(i + 1)
                    .is_some_and(|d| d.is_ascii_digit() || *d == b'.'))
        {
            let start = i;
            i += 1;
            while i < b.len() {
                let d = b[i];
                let exp_sign = (d == b'-' || d == b'+') && matches!(b[i - 1], b'e' | b'E');
                if d.is_ascii_alphanumeric() || d == b'.' || exp_sign {
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(number(&s[start..i])?);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let start = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Word(&s[start..i]));
        } else {
            let ch = s[i..].chars().next()?;
            out.push(Tok::Punct(ch));
            i += ch.len_utf8();
        }
    }
    Some(out)
}

fn number(t: &str) -> Option<Tok<'static>> {
    let (neg, body) = match t.as_bytes()[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        let v = i128::from_str_radix(hex, 16).ok()?;
        return Some(Tok::Int(if neg { -v } else { v }));
    }
    if body.contains(['.', 'e', 'E']) {
        let v: f64 = body.parse().ok()?;
        return Some(Tok::Float(if neg { -v } else { v }));
    }
    let v: i128 = body.parse().ok()?;
    Some(Tok::Int(if neg { -v } else { v }))
}

/// `s[start]` tırnağıyla başlayan dizge: (çözülmüş içerik, kapanıştan sonraki konum).
fn string(s: &str, start: usize) -> Option<(String, usize)> {
    let quote = s.as_bytes()[start] as char;
    let mut out = String::new();
    let mut chars = s[start + 1..].char_indices();
    while let Some((off, c)) = chars.next() {
        match c {
            '\\' => {
                let (_, e) = chars.next()?;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'v' => out.push('\u{b}'),
                    'a' => out.push('\u{7}'),
                    'u' | 'U' => {
                        let n = if e == 'u' { 4 } else { 8 };
                        let mut v = 0u32;
                        for _ in 0..n {
                            v = v * 16 + chars.next()?.1.to_digit(16)?;
                        }
                        out.push(char::from_u32(v)?);
                    }
                    other => out.push(other),
                }
            }
            c if c == quote => return Some((out, start + 1 + off + c.len_utf8())),
            c => out.push(c),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_compare_by_value() {
        // GLib 2.86 `dconf write` → `dconf read` ölçümleri (shell look-external.md §1).
        for (written, read) in [
            ("1.00", "1.0"),
            ("1.25", "1.25"),
            ("1.10", "1.1000000000000001"),
            ("0.90", "0.90000000000000002"),
            ("-2.5e1", "-25.0"),
        ] {
            assert!(equivalent(written, read), "{written} ~ {read}");
        }
        assert!(!equivalent("1.0", "1.01"));
        // Tamsayı ve çift ayrı türler.
        assert!(!equivalent("1", "1.0"));
        assert!(equivalent("uint32 0x10", "uint32 16"));
    }

    #[test]
    fn strings_compare_by_content() {
        assert!(equivalent("\"Adwaita\"", "'Adwaita'"));
        assert!(equivalent("'it\\'s'", "\"it's\""));
        assert!(equivalent("'\\u00e7'", "'ç'"));
        assert!(!equivalent("'Adwaita'", "'adwaita'"));
        assert!(equivalent("['a',  'b']", "['a', 'b']"));
        assert!(!equivalent("['a', 'b']", "['a b']"));
        // Dizge içindeki boşluk ve sayı benzeri metin anlamlıdır.
        assert!(!equivalent("'1.0'", "'1.00'"));
        assert!(!equivalent("'a  b'", "'a b'"));
    }

    #[test]
    fn other_tokens_are_literal() {
        assert!(equivalent("true", "true"));
        assert!(!equivalent("true", "false"));
        assert!(equivalent("@as []", "@as  []"));
        assert!(!equivalent("@as []", "[]"));
        assert!(equivalent("(1, 2.0)", "(1,2.00)"));
        // Kapanmamış dizge: bayt eşitliği.
        assert!(!equivalent("'abc", "'abc '"));
        assert!(equivalent("'abc", "'abc"));
    }
}
