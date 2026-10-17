//! Hafif, panik etmeyen Rust belirteç tarayıcısı (yalnızca bekçi için): yorumları atlar,
//! dizge sabitlerini (düz, ham, bayt, C) satırıyla çıkarır, karakter sabitini ömürden ayırır,
//! parantez eşlerini bulur. Tam bir ayrıştırıcı değildir; bozuk girdide de sonlanır ve
//! kısmi sonuç verir. `syn` gerekmez (derleme süresi, tam AST gereksiz).

/// Belirteç türü.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    Ident(String),
    /// Dizge sabitinin ham içeriği (kaçışlar çözülmez; bekçi için yeterli).
    Str(String),
    Punct(u8),
    Open(u8),
    Close(u8),
    /// Sayı, karakter sabiti, ömür.
    Other,
}

#[derive(Debug, Clone)]
pub(crate) struct Token {
    pub tok: Tok,
    pub line: usize,
    /// `Open` için eşleşen `Close`'un indeksi (eşsizse dosya sonu).
    pub close: usize,
}

/// `i18n:` ile başlayan düz satır yorumu (`// i18n: ..`; belge yorumları `///`, `//!` değil).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineComment {
    pub line: usize,
    /// `i18n:` sonrası metin.
    pub body: String,
    /// Satırda yorumdan önce yalnızca boşluk var.
    pub standalone: bool,
    /// Yorumdan sonraki ilk belirtecin indeksi.
    pub next_tok: usize,
}

#[cfg(test)]
pub(crate) fn tokenize(src: &str) -> Vec<Token> {
    lex(src).0
}

/// Belirteçler ve `// i18n:` yorumları.
pub(crate) fn lex(src: &str) -> (Vec<Token>, Vec<LineComment>) {
    let b = src.as_bytes();
    let mut out: Vec<Token> = Vec::new();
    let mut comments: Vec<LineComment> = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let at = |i: usize| b.get(i).copied().unwrap_or(0);
    let text = |from: usize, to: usize| src.get(from..to).unwrap_or_default().to_owned();
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            b'/' if at(i + 1) == b'/' => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                let text = src.get(start + 2..i).unwrap_or_default();
                let doc =
                    (text.starts_with('/') && !text.starts_with("//")) || text.starts_with('!');
                if !doc && let Some(body) = text.trim_start().strip_prefix("i18n:") {
                    let line_start = src
                        .get(..start)
                        .and_then(|s| s.rfind('\n'))
                        .map_or(0, |p| p + 1);
                    comments.push(LineComment {
                        line,
                        body: body.to_owned(),
                        standalone: src
                            .get(line_start..start)
                            .is_some_and(|s| s.trim().is_empty()),
                        next_tok: out.len(),
                    });
                }
            }
            b'/' if at(i + 1) == b'*' => {
                // İç içe blok yorumları.
                let mut depth = 0usize;
                while i < b.len() {
                    if b[i] == b'/' && at(i + 1) == b'*' {
                        depth += 1;
                        i += 2;
                    } else if b[i] == b'*' && at(i + 1) == b'/' {
                        depth = depth.saturating_sub(1);
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if b[i] == b'\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            b'"' => {
                let start_line = line;
                let (end, content_end) = quoted(b, i + 1, &mut line);
                out.push(Token {
                    tok: Tok::Str(text(i + 1, content_end)),
                    line: start_line,
                    close: 0,
                });
                i = end;
            }
            b'\'' => {
                // Karakter sabiti mi ömür mü: `'x'`, `'\n'`, `'\u{..}'` sabittir.
                if at(i + 1) == b'\\' {
                    // Kaçışlı karakter (`'\''`, `'\\'`, `'\u{..}'`): kaçışın ardındaki ilk bayt
                    // (tırnak olabilir) atlanır, kapanan tırnağa dek gidilir.
                    let mut j = i + 3;
                    while j < b.len() && b[j] != b'\'' && b[j] != b'\n' {
                        j += 1;
                    }
                    i = j + 1;
                } else {
                    // UTF-8 karakter uzunluğu.
                    let len = src
                        .get(i + 1..)
                        .and_then(|s| s.chars().next())
                        .map_or(1, char::len_utf8);
                    if at(i + 1 + len) == b'\'' {
                        i += len + 2;
                    } else {
                        i += 1;
                        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                            i += 1;
                        }
                    }
                }
                out.push(Token {
                    tok: Tok::Other,
                    line,
                    close: 0,
                });
            }
            _ if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                // Önekli dizgeler: r"..", r#".."#, b"..", br"..", c"..", cr"..".
                if let Some((end, content, start_line)) = prefixed_string(src, i, &mut line) {
                    out.push(Token {
                        tok: Tok::Str(content),
                        line: start_line,
                        close: 0,
                    });
                    i = end;
                    continue;
                }
                // Ham kimlik `r#ad`.
                let mut j = i;
                if c == b'r' && at(i + 1) == b'#' {
                    j += 2;
                }
                let start = j;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] >= 0x80)
                {
                    j += 1;
                }
                out.push(Token {
                    tok: Tok::Ident(text(start, j)),
                    line,
                    close: 0,
                });
                i = j.max(i + 1);
            }
            _ if c.is_ascii_digit() => {
                while i < b.len()
                    && (b[i].is_ascii_alphanumeric()
                        || b[i] == b'_'
                        || (b[i] == b'.' && at(i + 1).is_ascii_digit()))
                {
                    i += 1;
                }
                out.push(Token {
                    tok: Tok::Other,
                    line,
                    close: 0,
                });
            }
            b'(' | b'[' | b'{' => {
                out.push(Token {
                    tok: Tok::Open(c),
                    line,
                    close: 0,
                });
                i += 1;
            }
            b')' | b']' | b'}' => {
                out.push(Token {
                    tok: Tok::Close(c),
                    line,
                    close: 0,
                });
                i += 1;
            }
            _ => {
                out.push(Token {
                    tok: Tok::Punct(c),
                    line,
                    close: 0,
                });
                i += 1;
            }
        }
    }
    match_brackets(&mut out);
    (out, comments)
}

/// `"` sonrasından kapanan `"`'ye (kaçışlar atlanır): `(sonraki indeks, içerik sonu)`.
fn quoted(b: &[u8], mut i: usize, line: &mut usize) -> (usize, usize) {
    while i < b.len() {
        match b[i] {
            b'\\' => {
                if b.get(i + 1) == Some(&b'\n') {
                    *line += 1;
                }
                i += 2;
            }
            b'"' => return (i + 1, i),
            b'\n' => {
                *line += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    (b.len(), b.len())
}

/// `i`'de önekli dizge başlıyorsa `(sonraki indeks, içerik, başlangıç satırı)`.
fn prefixed_string(src: &str, i: usize, line: &mut usize) -> Option<(usize, String, usize)> {
    let b = src.as_bytes();
    let rest = b.get(i..)?;
    let (raw, skip) = match rest {
        [b'b' | b'c', b'r', ..] => (true, 2),
        [b'r', ..] => (true, 1),
        [b'b' | b'c', b'"', ..] => (false, 1),
        _ => return None,
    };
    let start_line = *line;
    if !raw {
        let (end, content_end) = quoted(b, i + skip + 1, line);
        return Some((
            end,
            src.get(i + skip + 1..content_end)?.to_owned(),
            start_line,
        ));
    }
    let mut j = i + skip;
    let mut hashes = 0usize;
    while b.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if b.get(j) != Some(&b'"') {
        return None;
    }
    let content_start = j + 1;
    let mut k = content_start;
    while k < b.len() {
        if b[k] == b'\n' {
            *line += 1;
        }
        if b[k] == b'"'
            && b.get(k + 1..k + 1 + hashes)
                .is_some_and(|h| h.iter().all(|c| *c == b'#'))
        {
            return Some((
                k + 1 + hashes,
                src.get(content_start..k)?.to_owned(),
                start_line,
            ));
        }
        k += 1;
    }
    Some((b.len(), src.get(content_start..)?.to_owned(), start_line))
}

/// Parantez eşleri, türe duyarlı: `)` yalnızca `(` kapatır. Kapanışın türü yığının tepesine
/// uymuyorsa yığında aynı türden açılış aranır; bulunursa aradaki eşsiz açılışlar da bu
/// kapanışta biter sayılır (kapsamları dışarı taşmaz), bulunmazsa kapanış yok
/// sayılır (bozuk girdide dış kapsam erken kapanmaz).
fn match_brackets(toks: &mut [Token]) {
    let n = toks.len();
    let mut stack: Vec<(usize, u8)> = Vec::new();
    for i in 0..n {
        match toks[i].tok {
            Tok::Open(c) => stack.push((i, c)),
            Tok::Close(c) => {
                let want = match c {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if let Some(pos) = stack.iter().rposition(|(_, o)| *o == want) {
                    for (open, _) in stack.drain(pos..) {
                        toks[open].close = i;
                    }
                }
            }
            _ => {}
        }
    }
    for (open, _) in stack {
        toks[open].close = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(src: &str) -> Vec<(String, usize)> {
        tokenize(src)
            .into_iter()
            .filter_map(|t| match t.tok {
                Tok::Str(s) => Some((s, t.line)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn strings_comments_and_lifetimes() {
        let src = "// \"yorum\"\nfn f<'a>(x: &'a str) -> char { /* \"blok /* iç */ \" */ let c = '\"'; let s = \"a\\\"b\"; 'x' }\nlet r = r#\"ham \"iç\" \"#;\nlet b = b\"bayt\"; let m = \"çok\nsatır\";";
        let s = strs(src);
        assert_eq!(
            s,
            vec![
                ("a\\\"b".into(), 2),
                ("ham \"iç\" ".into(), 3),
                ("bayt".into(), 4),
                ("çok\nsatır".into(), 4)
            ]
        );
    }

    #[test]
    fn never_panics_on_garbage() {
        for src in [
            "\"",
            "r#\"",
            "'",
            "/*",
            "b'",
            "r#",
            "'\\",
            "\u{0}é'",
            "(((",
            ")))",
            "'é",
            "r##\"x\"#",
        ] {
            let _ = tokenize(src);
        }
    }

    #[test]
    fn i18n_line_comments() {
        let src = "let a = 1; // i18n: tanı x\n    // i18n:tanı y\n/// i18n: tanı belge\n//! i18n: iç\n//// i18n: düz\nlet s = \"// i18n: dizge\"; // başka\n/* i18n: blok */";
        let (toks, c) = lex(src);
        let got: Vec<(usize, &str, bool, usize)> = c
            .iter()
            .map(|c| (c.line, c.body.as_str(), c.standalone, c.next_tok))
            .collect();
        // `let a = 1 ;` = 5 belirteç; belge yorumları, `//// i18n:` (gövde `//` ile başlar),
        // dizge ve blok yorumu işaret değil.
        assert_eq!(got, vec![(1, " tanı x", false, 5), (2, "tanı y", true, 5),]);
        assert_eq!(toks.len(), 10);
    }

    #[test]
    fn brackets_match() {
        let t = tokenize("a(b[c]{d})e");
        assert_eq!(t[1].close, 9);
        assert_eq!(t[3].close, 5);
    }

    #[test]
    fn escaped_quote_char_literal() {
        // `'\''` sonrası dizge doğru satırda ve doğru içerikle çıkar.
        let s = strs("let q = '\\''; let s = \"a\";\nlet b = b'\\''; let t = \"b\";");
        assert_eq!(s, vec![("a".into(), 1), ("b".into(), 2)]);
    }

    #[test]
    fn mismatched_brackets_are_type_aware() {
        // `{ ( ] ) }`: `]` yok sayılır, `)` `(`'yi, `}` `{`'yi kapatır.
        let t = tokenize("{(])}");
        assert_eq!(t[0].close, 4);
        assert_eq!(t[1].close, 3);
        // `{ ( }`: eşsiz `(` dış `}` ile biter, dış kapsam taşmaz.
        let t = tokenize("{(}x");
        assert_eq!(t[0].close, 2);
        assert_eq!(t[1].close, 2);
        // Eşsiz açılış dosya sonuna dek.
        let t = tokenize("((x");
        assert_eq!(t[0].close, 3);
    }
}
