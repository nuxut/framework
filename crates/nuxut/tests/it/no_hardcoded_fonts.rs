//! Bekçi: koda gömülü font ailesi ve ikon glifi yok (ADR-0006). Bileşenler fontu yalnızca
//! tipografi rolüyle (`text_style(theme, TextRole::…)`), ikonu yalnızca semantik adla
//! (`Icon::named(IconName::…)`) alır; aileler `nuxut-theme`'de, glifler `icon_names.rs`'tedir.
//!
//! Taranan: `crates/*/src` ve `crates/*/examples`. İzinli:
//! - `crates/nuxut-theme/**` (varsayılan aileler),
//! - `crates/nuxut-render/src/text.rs` (genel ailelerin tanımı ve çözümü),
//! - `crates/nuxut-ui/src/icon_names.rs` (glif tablosu),
//! - `crates/nuxut-ui/src/app_glyph.rs` (uygulama logosu glif tablosu ve Nerd Font ad kuralları),
//! - `#[cfg(test)]` satırından sonrası, yorum satırları,
//! - `// font-istisnası:` yorumu taşıyan satır ya da hemen önceki satırda bu yorum olan satır.

use std::path::{Path, PathBuf};

const ALLOW_MARK: &str = "// font-istisnası:";
/// Aile adını dizgi sabitiyle kuran kalıplar.
const FAMILY_SETTERS: &[&str] = &["TextStyle::new(\"", ".family = \"", "family: \""];
/// Dizgi sabiti içinde görülmemesi gereken aile adları (genel aileler dahil).
const FAMILY_NAMES: &[&str] = &[
    "\"sans-serif\"",
    "\"serif\"",
    "\"monospace\"",
    "Inter\"",
    "Source Serif",
    "JetBrains",
    "Nerd Font",
    "Material Symbols",
    "Noto Sans",
    "Roboto",
    "DejaVu",
    "Cantarell",
    "Adwaita Sans",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace kökü")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

fn scanned_files() -> Vec<PathBuf> {
    let crates = workspace_root().join("crates");
    let mut files = Vec::new();
    for c in std::fs::read_dir(&crates).expect("crates/").flatten() {
        if c.file_name() == "nuxut-theme" {
            continue;
        }
        for sub in ["src", "examples"] {
            rust_files(&c.path().join(sub), &mut files);
        }
    }
    files.retain(|f| {
        !f.ends_with("nuxut-render/src/text.rs")
            && !f.ends_with("nuxut-ui/src/icon_names.rs")
            && !f.ends_with("nuxut-ui/src/app_glyph.rs")
    });
    files.sort();
    files
}

/// Test modülünden önceki, yorum olmayan, izin işaretsiz satırlar: (satır no, metin).
fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut prev_allowed = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("#[cfg(test)]") {
            break;
        }
        let allowed = line.contains(ALLOW_MARK);
        let is_comment = t.starts_with("//");
        if !allowed && !prev_allowed && !is_comment {
            out.push((i + 1, line));
        }
        prev_allowed = allowed && is_comment;
    }
    out
}

/// Satırda gömülü aile adı ya da Unicode özel kullanım alanı (ikon fontu glifi) var mı.
fn violation(line: &str) -> Option<&'static str> {
    if FAMILY_SETTERS.iter().any(|p| line.contains(p)) || FAMILY_NAMES.iter().any(|n| line.contains(n)) {
        return Some("gömülü font ailesi");
    }
    // `\u{e000}`–`\u{f8ff}` ve `\u{f0000}`+ kaçışları ya da doğrudan PUA karakteri.
    let escaped_pua = line.match_indices("\\u{").any(|(i, _)| {
        let hex: String = line[i + 3..].chars().take_while(char::is_ascii_hexdigit).collect();
        u32::from_str_radix(&hex, 16).is_ok_and(is_pua)
    });
    if escaped_pua || line.chars().any(|c| is_pua(c as u32)) {
        return Some("gömülü ikon glifi");
    }
    None
}

fn is_pua(c: u32) -> bool {
    (0xE000..=0xF8FF).contains(&c) || c >= 0xF0000
}

#[test]
fn no_hardcoded_fonts() {
    let root = workspace_root();
    let mut hits = Vec::new();
    for f in scanned_files() {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in production_lines(&text) {
            if let Some(kind) = violation(line) {
                hits.push(format!("{}:{n}: {kind}: {}", f.strip_prefix(&root).unwrap_or(&f).display(), line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "fontu `text_style(theme, TextRole::…)`, ikonu `Icon::named(IconName::…)` ile alın \
         (gerçekten istisnaysa satıra `{ALLOW_MARK} <gerekçe>`):\n{}",
        hits.join("\n")
    );
}

#[test]
fn detector_self_check() {
    assert!(violation(r#"TextStyle::new("Inter", 13.0, 400)"#).is_some());
    assert!(violation(r#"let f = "monospace";"#).is_some());
    assert!(violation(r#"style.family = "Hack".into();"#).is_some());
    assert!(violation(r#"label.set_text("\u{f0156}")"#).is_some());
    assert!(violation(r#"label.set_text("\u{e5cd}")"#).is_some());
    assert!(violation(r#"TextStyle::new(family, size, weight)"#).is_none());
    assert!(violation(r#"label.set_text("\u{2026}")"#).is_none());
    assert!(violation(r#"let s = "Seriler";"#).is_none());
}
