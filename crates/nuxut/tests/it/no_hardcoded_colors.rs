//! Bekçi: koda gömülü renk yok (kullanıcı kuralı). Her renk temadan gelir; varsayılanların
//! tek yeri `nuxut-theme`'dir ve hepsi Ayarlar'dan değiştirilebilir.
//!
//! Taranan: `crates/*/src` ve `crates/*/examples`. İzinli:
//! - `crates/nuxut-theme/**` (varsayılanlar), `crates/nuxut-core/src/color.rs` (tipin kendisi),
//! - `#[cfg(test)]` satırından sonrası (dosya sonundaki test modülleri),
//! - `// renk-matematiği:` yorumu taşıyan satır ya da hemen önceki satırda bu yorum olan satır
//!   (ör. HSV alanının beyaz/siyah uçları — renk değil, hesap).
//!
//! İkinci aşama (uyarı, hata değil): bileşenlerde sayısal stil sabitleri (yarıçap, boşluk,
//! süre) `NUXUT_STYLE_LINT=1` ile listelenir.

use std::path::{Path, PathBuf};

/// Sabit renk kuran çağrılar: argümanları sayı sabitiyle başlıyorsa gömülü renktir
/// (`Color::rgba(c[0], …)` gibi dönüşümler değildir).
const CONSTRUCTORS: &[&str] = &["Color::rgb(", "Color::rgba(", "from_rgba8(", "Color::from_hex("];
/// Her zaman sabit olan adlar.
/// (`Color::TRANSPARENT` renk değil, "dolgu yok" anlamındadır; serbest.)
const NAMED: &[&str] = &["Color::WHITE", "Color::BLACK"];
const STYLE_PATTERNS: &[&str] =
    &["Corners::uniform(", "Insets::uniform(", "Duration::from_millis(", "Duration::from_secs_f32("];
const ALLOW_MARK: &str = "// renk-matematiği:";

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
        let name = c.file_name().to_string_lossy().into_owned();
        if name == "nuxut-theme" {
            continue;
        }
        for sub in ["src", "examples"] {
            rust_files(&c.path().join(sub), &mut files);
        }
    }
    files.retain(|f| !f.ends_with("nuxut-core/src/color.rs"));
    files.sort();
    files
}

/// `"#rgb"`, `"#rgba"`, `"#rrggbb"`, `"#rrggbbaa"` dizgi sabiti.
fn has_hex_literal(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'"' && b[i + 1] == b'#' {
            let digits = b[i + 2..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
            if matches!(digits, 3 | 4 | 6 | 8) && b.get(i + 2 + digits) == Some(&b'"') {
                return true;
            }
        }
        i += 1;
    }
    false
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

#[test]
fn no_hardcoded_colors() {
    let root = workspace_root();
    let mut hits = Vec::new();
    for f in scanned_files() {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in production_lines(&text) {
            if is_color_literal(line) {
                hits.push(format!("{}:{n}: {}", f.strip_prefix(&root).unwrap_or(&f).display(), line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "koda gömülü renk bulundu (tema token'ı kullanın; gerçekten renk matematiğiyse satıra \
         `{ALLOW_MARK} <gerekçe>` ekleyin):\n{}",
        hits.join("\n")
    );
}

#[test]
fn style_constants_report() {
    if std::env::var_os("NUXUT_STYLE_LINT").is_none() {
        return;
    }
    let root = workspace_root();
    let mut hits = Vec::new();
    for f in scanned_files() {
        let text = std::fs::read_to_string(&f).unwrap();
        for (n, line) in production_lines(&text) {
            for p in STYLE_PATTERNS {
                if let Some(pos) = line.find(p)
                    && line[pos + p.len()..].starts_with(|c: char| c.is_ascii_digit())
                {
                    hits.push(format!("{}:{n}: {}", f.strip_prefix(&root).unwrap_or(&f).display(), line.trim()));
                }
            }
        }
    }
    eprintln!("sayısal stil sabiti adayları ({}):\n{}", hits.len(), hits.join("\n"));
}

#[test]
fn detector_self_check() {
    assert!(has_hex_literal(r##"let c = "#12141a";"##));
    assert!(has_hex_literal(r##"x("#fff")"##));
    assert!(!has_hex_literal(r##"format!("#{}", n)"##));
    assert!(!has_hex_literal(r##""#1234567""##));
    let text = "a\n// renk-matematiği: beyaz uç\nColor::WHITE\nColor::BLACK\n#[cfg(test)]\nColor::WHITE\n";
    let lines: Vec<_> = production_lines(text).into_iter().map(|(_, l)| l).collect();
    assert_eq!(lines, vec!["a", "Color::BLACK"]);
}

fn is_color_literal(line: &str) -> bool {
    if NAMED.iter().any(|n| line.contains(n)) || has_hex_literal(line) {
        return true;
    }
    CONSTRUCTORS.iter().any(|p| {
        line.match_indices(p).any(|(i, _)| {
            let rest = line[i + p.len()..].trim_start();
            rest.starts_with(|c: char| c.is_ascii_digit() || c == '"' || c == '.')
        })
    })
}

#[test]
fn literal_detection() {
    assert!(is_color_literal("Color::rgba(0.5, 0.2, 0.1, 1.0)"));
    assert!(is_color_literal("Color::from_rgba8(0x42, 0x85, 0xF4, 255)"));
    assert!(is_color_literal("Color::from_hex(\"#fff\")"));
    assert!(!is_color_literal("Color::rgba(c[0], c[1], c[2], c[3])"));
    assert!(!is_color_literal("Color::from_rgba8(c.r(), c.g(), c.b(), c.a())"));
    assert!(!is_color_literal("Color::from_hex(&s)"));
}
