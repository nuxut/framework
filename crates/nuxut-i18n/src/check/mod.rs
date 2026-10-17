//! Doğrulama kapıları (ADR-0010 §4). Her repo tek `#[test]` ile çağırır:
//!
//! ```ignore
//! #[test]
//! fn i18n_catalog() {
//!     let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/locales");
//!     nuxut::i18n::check::catalog_embedded(&nuxut::i18n::embed!("shell"), dir, "shell-").assert_ok();
//! }
//!
//! #[test]
//! fn i18n_source_guard() {
//!     nuxut::i18n::check::source_guard(env!("CARGO_MANIFEST_DIR")).assert_ok();
//! }
//! ```
//!
//! Katalog denetimi: her dosya hatasız ayrışır; anahtar tekrarı yok; `tr` ve `en` var ve
//! anahtar (+ öznitelik) kümeleri eşit; her dilde mesaj başına değişken kümesi `tr` ile aynı;
//! her mesajın çevirmen yorumu var ve yorum her `$değişken`i anar; `[max: N]` varsa (tr'nin
//! yorumundan; yoksa dilin kendisinden) tüm dillerde sabit metin ≤ N grafem; mesaj/terim
//! referansları aynı dosyada çözülür; kimlikler kebab-case ve repo önekiyle; kısmi diller
//! için kapsam yüzdesi not olarak yazdırılır (hata değil).

mod guard;
mod scan;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use fluent_syntax::ast;
use unicode_segmentation::UnicodeSegmentation;

pub use guard::{DEFAULT_UI_CALLS, SourceGuard, source_guard};

use crate::Source;

/// Tek bir bulgu: `dosya:satır: ileti`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    /// 1 tabanlı; 0 = dosya geneli.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}:{}: {}", self.file, self.line, self.message)
        }
    }
}

/// Denetim sonucu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub errors: Vec<Finding>,
    /// Hata olmayan bilgi (kısmi dil kapsamı vb.).
    pub notes: Vec<String>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Notları yazdırır; hata varsa hepsini listeleyerek panikler (`#[test]` için).
    #[track_caller]
    pub fn assert_ok(&self) {
        for n in &self.notes {
            println!("{n}");
        }
        if !self.errors.is_empty() {
            let list: Vec<String> = self.errors.iter().map(ToString::to_string).collect();
            panic!("{} i18n hatası:\n{}", self.errors.len(), list.join("\n"));
        }
    }

    /// Bir hata iletisi `needle` içeriyor mu (testler).
    pub fn has(&self, needle: &str) -> bool {
        self.errors.iter().any(|f| f.message.contains(needle))
    }

    fn error(&mut self, file: &str, line: usize, message: impl Into<String>) {
        self.errors.push(Finding { file: file.to_owned(), line, message: message.into() });
    }
}

/// Kaynak dil ve zorunlu-tam diller.
const SOURCE_LANG: &str = "tr";
const REQUIRED: [&str; 2] = ["tr", "en"];

/// `dir/<dil>.ftl` kataloglarını denetler; `prefix` repo ad alanı (`"shell"` ya da
/// `"shell-"`).
pub fn catalog(dir: impl AsRef<Path>, prefix: &str) -> Report {
    let mut report = Report::default();
    let files = read_dir(dir.as_ref(), &mut report);
    check_files(&files, prefix, &mut report);
    report
}

/// [`catalog`] + gömülü dil listesi diskle aynı mı (yeni `.ftl` eklenip crate yeniden
/// derlenmediyse yakalanır) ve gömülü içerik diskteki ile aynı mı.
pub fn catalog_embedded(source: &Source, dir: impl AsRef<Path>, prefix: &str) -> Report {
    let mut report = Report::default();
    let files = read_dir(dir.as_ref(), &mut report);
    for f in &files {
        match source.embedded.iter().find(|(l, _)| *l == f.lang) {
            None => report.error(&f.name, 0, "gömülü değil: makroyu çağıran dosyaya dokunup yeniden derleyin"),
            Some((_, text)) if *text != f.text => report.error(&f.name, 0, "gömülü içerik diskteki ile farklı; yeniden derleyin"),
            Some(_) => {}
        }
    }
    for lang in source.languages() {
        if !files.iter().any(|f| f.lang == lang) {
            report.error(&format!("{lang}.ftl"), 0, "gömülü ama diskte yok; yeniden derleyin");
        }
    }
    check_files(&files, prefix, &mut report);
    report
}

/// Bellekteki dosyalarla denetim (`(dil, içerik)`; testler ve araçlar).
pub fn catalog_texts(texts: &[(&str, &str)], prefix: &str) -> Report {
    let mut report = Report::default();
    let files: Vec<File> = texts
        .iter()
        .map(|(lang, text)| File { lang: (*lang).to_owned(), name: format!("{lang}.ftl"), text: (*text).to_owned() })
        .collect();
    check_files(&files, prefix, &mut report);
    report
}

struct File {
    lang: String,
    name: String,
    text: String,
}

fn read_dir(dir: &Path, report: &mut Report) -> Vec<File> {
    let shown = dir.display().to_string();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            report.error(&shown, 0, format!("dizin okunamadı: {e}"));
            return Vec::new();
        }
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(lang) = name.strip_suffix(".ftl") else { continue };
        if !valid_lang_file(lang) {
            report.error(&name, 0, "dosya adı bir dil etiketi değil (tr.ftl, pt-BR.ftl)");
            continue;
        }
        match std::fs::read_to_string(entry.path()) {
            Ok(text) => files.push(File { lang: lang.to_owned(), name, text }),
            Err(e) => report.error(&name, 0, format!("okunamadı: {e}")),
        }
    }
    files.sort_by(|a, b| a.lang.cmp(&b.lang));
    files
}

/// `[a-zA-Z]{2,3}(-[A-Za-z0-9]{1,8})*` (embed! ile aynı kural).
fn valid_lang_file(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Bir mesajın denetim için özeti.
#[derive(Debug, Default)]
struct Msg {
    line: usize,
    vars: BTreeSet<String>,
    attrs: BTreeSet<String>,
    comment: Option<String>,
    /// Sabit metnin grafem sayısı (seçicide en uzun dal; değişken/referans 0).
    len: usize,
    refs: Vec<Ref>,
}

#[derive(Debug)]
struct Ref {
    term: bool,
    id: String,
    attr: Option<String>,
}

#[derive(Debug, Default)]
struct Parsed {
    messages: BTreeMap<String, Msg>,
    /// Terim kimliği (`-` olmadan) → özet (öznitelikler, referanslar).
    terms: BTreeMap<String, Msg>,
}

fn check_files(files: &[File], prefix: &str, report: &mut Report) {
    let prefix = if prefix.ends_with('-') { prefix.to_owned() } else { format!("{prefix}-") };
    for lang in REQUIRED {
        if !files.iter().any(|f| f.lang == lang) {
            report.error(&format!("{lang}.ftl"), 0, "zorunlu dil dosyası yok");
        }
    }
    let parsed: Vec<(&File, Parsed)> = files.iter().map(|f| (f, parse_file(f, &prefix, report))).collect();
    let find = |lang: &str| parsed.iter().find(|(f, _)| f.lang == lang).map(|(_, p)| p);
    let source = find(SOURCE_LANG);

    for (file, p) in &parsed {
        // Referanslar aynı dosyada çözülür.
        let all = p.messages.iter().map(|(id, m)| (id.clone(), m)).chain(p.terms.iter().map(|(id, m)| (format!("-{id}"), m)));
        for (id, m) in all {
            for r in &m.refs {
                let ok = if r.term {
                    p.terms.get(&r.id).is_some_and(|t| r.attr.as_ref().is_none_or(|a| t.attrs.contains(a)))
                } else {
                    p.messages.get(&r.id).is_some_and(|t| r.attr.as_ref().is_none_or(|a| t.attrs.contains(a)))
                };
                if !ok {
                    let target = format!(
                        "{}{}{}",
                        if r.term { "-" } else { "" },
                        r.id,
                        r.attr.as_ref().map(|a| format!(".{a}")).unwrap_or_default()
                    );
                    report.error(&file.name, m.line, format!("{id}: çözülmeyen referans {target}"));
                }
            }
        }
        for (id, line) in cycles(p) {
            report.error(&file.name, line, format!("{id}: döngülü referans"));
        }
        // [max: N]: kaynağın sınırı (yoksa dilin kendi sınırı).
        for (id, m) in &p.messages {
            let limit = source
                .and_then(|s| s.messages.get(id))
                .and_then(|s| s.comment.as_deref().and_then(max_of))
                .or_else(|| m.comment.as_deref().and_then(max_of));
            if let Some(Ok(n)) = limit
                && m.len > n
            {
                report.error(&file.name, m.line, format!("{id}: {} grafem, sınır [max: {n}]", m.len));
            }
        }
        let Some(src) = source else { continue };
        if file.lang == SOURCE_LANG {
            continue;
        }
        let required = REQUIRED.contains(&file.lang.as_str());
        for (id, m) in &p.messages {
            let Some(s) = src.messages.get(id) else {
                report.error(&file.name, m.line, format!("{id}: {SOURCE_LANG}.ftl'de yok"));
                continue;
            };
            if m.vars != s.vars {
                report.error(
                    &file.name,
                    m.line,
                    format!("{id}: değişkenler {} ≠ {SOURCE_LANG} {}", set(&m.vars), set(&s.vars)),
                );
            }
            if required && m.attrs != s.attrs {
                report.error(
                    &file.name,
                    m.line,
                    format!("{id}: öznitelikler {} ≠ {SOURCE_LANG} {}", set(&m.attrs), set(&s.attrs)),
                );
            } else if let Some(extra) = m.attrs.difference(&s.attrs).next() {
                report.error(&file.name, m.line, format!("{id}: .{extra} özniteliği {SOURCE_LANG}'de yok"));
            }
        }
        if required {
            for (id, s) in &src.messages {
                if !p.messages.contains_key(id) {
                    report.error(&file.name, 0, format!("{id}: eksik ({SOURCE_LANG}.ftl:{})", s.line));
                }
            }
        } else if !src.messages.is_empty() {
            let present = src.messages.keys().filter(|k| p.messages.contains_key(*k)).count();
            let pct = present as f64 * 100.0 / src.messages.len() as f64;
            report.notes.push(format!("{}: kapsam %{pct:.1} ({present}/{})", file.name, src.messages.len()));
        }
    }
}

/// Mesaj/terim referans grafiğindeki döngüye giren mesajlar: `(kimlik, satır)`, her döngü
/// bir kez (döngüyü kapatan düğümde). Yinelemesiz DFS (derin zincirde yığın taşmaz).
fn cycles(p: &Parsed) -> Vec<(String, usize)> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let key = |term: bool, id: &str| (term, id.to_owned());
    let node = |k: &(bool, String)| if k.0 { p.terms.get(&k.1) } else { p.messages.get(&k.1) };
    let mut color: BTreeMap<(bool, String), Color> = BTreeMap::new();
    let mut found = Vec::new();
    let starts = p.messages.keys().map(|id| key(false, id)).chain(p.terms.keys().map(|id| key(true, id)));
    for start in starts {
        if color.get(&start).copied().unwrap_or(Color::White) != Color::White {
            continue;
        }
        // (düğüm, sonraki referans indeksi)
        let mut stack: Vec<((bool, String), usize)> = vec![(start.clone(), 0)];
        color.insert(start, Color::Gray);
        while let Some((k, idx)) = stack.last().cloned() {
            let refs = node(&k).map(|m| m.refs.as_slice()).unwrap_or_default();
            match refs.get(idx) {
                None => {
                    color.insert(k, Color::Black);
                    stack.pop();
                }
                Some(r) => {
                    if let Some(top) = stack.last_mut() {
                        top.1 += 1;
                    }
                    let next = key(r.term, &r.id);
                    if node(&next).is_none() {
                        continue;
                    }
                    match color.get(&next).copied().unwrap_or(Color::White) {
                        Color::White => {
                            color.insert(next.clone(), Color::Gray);
                            stack.push((next, 0));
                        }
                        Color::Gray => {
                            let line = node(&k).map_or(0, |m| m.line);
                            let shown = if k.0 { format!("-{}", k.1) } else { k.1.clone() };
                            found.push((shown, line));
                        }
                        Color::Black => {}
                    }
                }
            }
        }
    }
    found
}

fn set(s: &BTreeSet<String>) -> String {
    format!("{{{}}}", s.iter().map(|v| format!("${v}")).collect::<Vec<_>>().join(", "))
}

/// Yorumdaki `[max: N]`: yok → `None`, bozuk → `Some(Err)`.
fn max_of(comment: &str) -> Option<Result<usize, ()>> {
    let start = comment.find("[max:")?;
    let rest = &comment[start + 5..];
    let end = rest.find(']')?;
    Some(rest[..end].trim().parse::<usize>().map_err(|_| ()))
}

/// `[a-z0-9]+(-[a-z0-9]+)*`.
fn kebab(s: &str) -> bool {
    !s.is_empty() && s.split('-').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn line_of(text: &str, offset: usize) -> usize {
    text.get(..offset.min(text.len())).map_or(1, |t| t.bytes().filter(|b| *b == b'\n').count() + 1)
}

/// Satır başındaki `kimlik =` → satır no (mesajlar ve `-terim`ler).
fn id_lines(text: &str) -> BTreeMap<&str, Vec<usize>> {
    let mut map: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, line) in text.lines().enumerate() {
        let Some(first) = line.bytes().next() else { continue };
        if !(first.is_ascii_alphabetic() || first == b'-') {
            continue;
        }
        let Some((id, _)) = line.split_once('=') else { continue };
        map.entry(id.trim_end()).or_default().push(i + 1);
    }
    map
}

fn parse_file(file: &File, prefix: &str, report: &mut Report) -> Parsed {
    let resource = match fluent_syntax::parser::parse(file.text.as_str()) {
        Ok(r) => r,
        Err((r, errors)) => {
            for e in errors {
                report.error(&file.name, line_of(&file.text, e.pos.start), format!("ayrıştırma hatası: {}", e.kind));
            }
            r
        }
    };
    let lines = id_lines(&file.text);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut line_for = |key: &str| -> usize {
        let n = seen.entry(key.to_owned()).or_insert(0);
        *n += 1;
        lines.get(key).and_then(|v| v.get(*n - 1).copied()).unwrap_or(0)
    };
    let mut out = Parsed::default();
    for entry in &resource.body {
        match entry {
            ast::Entry::Message(m) => {
                let id = m.id.name.to_owned();
                let line = line_for(&id);
                if out.messages.contains_key(&id) {
                    report.error(&file.name, line, format!("{id}: tekrar eden anahtar"));
                    continue;
                }
                if !kebab(&id) {
                    report.error(&file.name, line, format!("{id}: kimlik kebab-case değil ([a-z0-9-])"));
                }
                if !id.starts_with(prefix) {
                    report.error(&file.name, line, format!("{id}: önek '{prefix}' değil"));
                }
                let mut msg = Msg { line, ..Msg::default() };
                if let Some(v) = &m.value {
                    msg.len = pattern(v, &mut msg);
                }
                for a in &m.attributes {
                    if !kebab(a.id.name) {
                        report.error(&file.name, line, format!("{id}: .{} özniteliği kebab-case değil", a.id.name));
                    }
                    if !msg.attrs.insert(a.id.name.to_owned()) {
                        report.error(&file.name, line, format!("{id}: .{} özniteliği tekrar ediyor", a.id.name));
                    }
                    pattern(&a.value, &mut msg);
                }
                let comment = m.comment.as_ref().map(|c| c.content.join("\n")).filter(|c| !c.trim().is_empty());
                match &comment {
                    None => report.error(&file.name, line, format!("{id}: çevirmen yorumu yok")),
                    Some(c) => {
                        for v in &msg.vars {
                            if !mentions(c, v) {
                                report.error(&file.name, line, format!("{id}: yorum ${v} değişkenini anmıyor"));
                            }
                        }
                        if let Some(Err(())) = max_of(c) {
                            report.error(&file.name, line, format!("{id}: [max: N] sayı değil"));
                        }
                    }
                }
                msg.comment = comment;
                out.messages.insert(id, msg);
            }
            ast::Entry::Term(t) => {
                let id = t.id.name.to_owned();
                let line = line_for(&format!("-{id}"));
                if out.terms.contains_key(&id) {
                    report.error(&file.name, line, format!("-{id}: tekrar eden terim"));
                    continue;
                }
                if !kebab(&id) || !id.starts_with(prefix) {
                    report.error(&file.name, line, format!("-{id}: terim kimliği kebab-case ve '-{prefix}' önekli olmalı"));
                }
                let mut msg = Msg { line, ..Msg::default() };
                pattern(&t.value, &mut msg);
                for a in &t.attributes {
                    msg.attrs.insert(a.id.name.to_owned());
                    pattern(&a.value, &mut msg);
                }
                out.terms.insert(id, msg);
            }
            _ => {}
        }
    }
    out
}

/// `$ad` yorumda kelime olarak geçiyor mu.
fn mentions(comment: &str, var: &str) -> bool {
    let needle = format!("${var}");
    comment.match_indices(&needle).any(|(i, _)| {
        comment[i + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
    })
}

/// Desendeki değişken/referansları toplar; sabit metnin grafem sayısını döndürür.
fn pattern(p: &ast::Pattern<&str>, m: &mut Msg) -> usize {
    p.elements
        .iter()
        .map(|e| match e {
            ast::PatternElement::TextElement { value } => value.graphemes(true).count(),
            ast::PatternElement::Placeable { expression } => expr(expression, m),
        })
        .sum()
}

fn expr(e: &ast::Expression<&str>, m: &mut Msg) -> usize {
    match e {
        ast::Expression::Select { selector, variants } => {
            inline(selector, m);
            variants.iter().map(|v| pattern(&v.value, m)).max().unwrap_or(0)
        }
        ast::Expression::Inline(i) => inline(i, m),
    }
}

fn inline(e: &ast::InlineExpression<&str>, m: &mut Msg) -> usize {
    match e {
        ast::InlineExpression::StringLiteral { value } => value.graphemes(true).count(),
        ast::InlineExpression::NumberLiteral { value } => value.len(),
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            args(arguments, m);
            0
        }
        ast::InlineExpression::MessageReference { id, attribute } => {
            m.refs.push(Ref { term: false, id: id.name.to_owned(), attr: attribute.as_ref().map(|a| a.name.to_owned()) });
            0
        }
        ast::InlineExpression::TermReference { id, attribute, arguments } => {
            m.refs.push(Ref { term: true, id: id.name.to_owned(), attr: attribute.as_ref().map(|a| a.name.to_owned()) });
            if let Some(a) = arguments {
                args(a, m);
            }
            0
        }
        ast::InlineExpression::VariableReference { id } => {
            m.vars.insert(id.name.to_owned());
            0
        }
        ast::InlineExpression::Placeable { expression } => expr(expression, m),
    }
}

fn args(a: &ast::CallArguments<&str>, m: &mut Msg) {
    for p in &a.positional {
        inline(p, m);
    }
    for n in &a.named {
        inline(&n.value, m);
    }
}
