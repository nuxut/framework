//! `nuxut_i18n::check`: katalog kapıları (her hata türü için negatif fikstür) ve gömülü metin
//! bekçisi (pozitif/negatif, izin listesi, test modülleri).

use std::path::PathBuf;

use nuxut_i18n::check::{self, SourceGuard};

const FX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/good");

#[test]
fn good_fixture_passes_and_reports_coverage() {
    let r = check::catalog_embedded(&nuxut_i18n::embed!("fx", "tests/fixtures/good"), FX, "fx");
    r.assert_ok();
    assert!(
        r.notes
            .iter()
            .any(|n| n.starts_with("de.ftl: kapsam %28.6 (2/7)")),
        "{:?}",
        r.notes
    );
    assert!(
        r.notes.iter().any(|n| n.starts_with("ar.ftl")),
        "{:?}",
        r.notes
    );
}

#[test]
fn embedded_mismatch_is_caught() {
    static EMB: &[(&str, &str)] = &[("en", "eski"), ("tr", "eski"), ("xx", "")];
    let src = nuxut_i18n::Source {
        domain: "fx",
        embedded: EMB,
    };
    let r = check::catalog_embedded(&src, FX, "fx");
    assert!(r.has("gömülü değil"), "{:?}", r.errors); // ar, de
    assert!(r.has("gömülü içerik diskteki ile farklı"), "{:?}", r.errors);
    assert!(r.has("gömülü ama diskte yok"), "{:?}", r.errors);
}

const C: &str = "# Nerede: test.\n";

fn tr_en(tr: &str, en: &str) -> check::Report {
    check::catalog_texts(&[("tr", tr), ("en", en)], "fx")
}

fn same(s: &str) -> check::Report {
    tr_en(s, s)
}

#[test]
fn ok_minimal() {
    let r = same(&format!("{C}fx-a = A\n"));
    assert!(r.is_ok(), "{:?}", r.errors);
}

#[test]
fn parse_error() {
    let r = same(&format!("{C}fx-a = A\nfx-b = {{ \n"));
    assert!(r.has("ayrıştırma hatası"), "{:?}", r.errors);
    assert!(r.errors.iter().all(|f| f.line >= 3), "{:?}", r.errors);
}

#[test]
fn duplicate_key() {
    let r = same(&format!("{C}fx-a = A\n\n{C}fx-a = B\n"));
    assert!(r.has("tekrar eden anahtar"), "{:?}", r.errors);
    assert!(r.errors.iter().any(|f| f.line == 5), "{:?}", r.errors);
}

#[test]
fn required_languages_and_key_sets() {
    let r = check::catalog_texts(&[("tr", &format!("{C}fx-a = A\n"))], "fx");
    assert!(r.has("zorunlu dil dosyası yok"), "{:?}", r.errors);
    let r = tr_en(
        &format!("{C}fx-a = A\n{C}fx-b = B\n"),
        &format!("{C}fx-a = A\n{C}fx-c = C\n"),
    );
    assert!(r.has("fx-b: eksik"), "{:?}", r.errors);
    assert!(r.has("fx-c: tr.ftl'de yok"), "{:?}", r.errors);
}

#[test]
fn attribute_sets() {
    let r = tr_en(
        &format!("{C}fx-a = A\n    .tooltip = T\n"),
        &format!("{C}fx-a = A\n"),
    );
    assert!(r.has("öznitelikler"), "{:?}", r.errors);
}

#[test]
fn variable_sets() {
    let tr = "# Değişkenler: $n.\nfx-a = { $n } öğe\n";
    let r = tr_en(tr, "# Değişkenler: $n, $m.\nfx-a = { $n } of { $m }\n");
    assert!(r.has("değişkenler {$m, $n} ≠ tr {$n}"), "{:?}", r.errors);
    let r = tr_en(tr, "# Değişkenler: $n.\nfx-a = items\n");
    assert!(r.has("değişkenler {} ≠ tr {$n}"), "{:?}", r.errors);
    // Kısmi dilde de değişken eşliği.
    let r = check::catalog_texts(
        &[
            ("tr", tr),
            ("en", tr),
            ("de", "# Değişkenler: $x.\nfx-a = { $x }\n"),
        ],
        "fx",
    );
    assert!(r.has("değişkenler {$x} ≠ tr {$n}"), "{:?}", r.errors);
    // Seçicideki ve işlev argümanındaki değişken sayılır.
    let r = same("# $count, $n\nfx-a = { NUMBER($n) } { $count ->\n   *[other] x\n}\n");
    assert!(r.is_ok(), "{:?}", r.errors);
}

#[test]
fn comment_required_and_mentions_variables() {
    let r = same("fx-a = A\n");
    assert!(r.has("çevirmen yorumu yok"), "{:?}", r.errors);
    // Boş satır yorumu ayırır: mesaja bağlı değil.
    let r = same("# Nerede: test.\n\nfx-a = A\n");
    assert!(r.has("çevirmen yorumu yok"), "{:?}", r.errors);
    let r = same("# Değişkenler: $nam.\nfx-a = { $name }\n");
    assert!(r.has("yorum $name değişkenini anmıyor"), "{:?}", r.errors);
    let r = same("# Değişkenler: $name_x.\nfx-a = { $name }\n");
    assert!(r.has("yorum $name değişkenini anmıyor"), "{:?}", r.errors);
}

#[test]
fn max_graphemes_in_all_languages() {
    let tr = "# Nerede: test.\n# [max: 5]\nfx-a = Kısa\n";
    let r = tr_en(tr, "# Nerede: test.\n# [max: 5]\nfx-a = Too long\n");
    assert!(r.has("8 grafem, sınır [max: 5]"), "{:?}", r.errors);
    // Sınır kaynaktan: çeviride yorum sınırı yazmasa da uygulanır.
    let r = tr_en(tr, "# Nerede: test.\nfx-a = Too long\n");
    assert!(r.has("sınır [max: 5]"), "{:?}", r.errors);
    // Kısmi dil de.
    let r = check::catalog_texts(
        &[
            ("tr", tr),
            ("en", tr),
            ("de", "# Nerede: test.\nfx-a = Zu langes\n"),
        ],
        "fx",
    );
    assert!(
        r.errors
            .iter()
            .any(|f| f.file == "de.ftl" && f.message.contains("sınır")),
        "{:?}",
        r.errors
    );
    // Grafem: birleşik aksan tek sayılır; seçicide en uzun dal; değişken 0.
    let r = same(
        "# $n\n# [max: 4]\nfx-a = e\u{301}e\u{301}{ $n }{ $n ->\n    [one] ab\n   *[other] a\n}\n",
    );
    assert!(r.is_ok(), "{:?}", r.errors);
    let r = same("# Nerede: test.\n# [max: beş]\nfx-a = A\n");
    assert!(r.has("[max: N] sayı değil"), "{:?}", r.errors);
}

#[test]
fn references_resolve() {
    let r = same(&format!(
        "{C}fx-a = {{ fx-b }} {{ -fx-t }} {{ fx-c.tooltip }}\n{C}fx-c = C\n"
    ));
    assert!(r.has("çözülmeyen referans fx-b"), "{:?}", r.errors);
    assert!(r.has("çözülmeyen referans -fx-t"), "{:?}", r.errors);
    assert!(r.has("çözülmeyen referans fx-c.tooltip"), "{:?}", r.errors);
    let r = same(&format!("{C}fx-a = {{ fx-b }}\n{C}fx-b = {{ fx-a }}\n"));
    assert!(r.has("döngülü referans"), "{:?}", r.errors);
    let r = same(&format!("-fx-t = {{ -fx-t }}\n{C}fx-a = {{ -fx-t }}\n"));
    assert!(r.has("döngülü referans"), "{:?}", r.errors);
}

#[test]
fn prefix_and_kebab_case() {
    let r = same(&format!(
        "{C}shell-a = A\n{C}fx-Bad_Key = B\n{C}fx-c = C\n    .Tool = x\n-other = T\n"
    ));
    assert!(r.has("shell-a: önek 'fx-' değil"), "{:?}", r.errors);
    assert!(
        r.has("fx-Bad_Key: kimlik kebab-case değil"),
        "{:?}",
        r.errors
    );
    assert!(r.has(".Tool özniteliği kebab-case değil"), "{:?}", r.errors);
    assert!(r.has("-other: terim kimliği"), "{:?}", r.errors);
}

#[test]
fn bad_file_names_and_missing_dir() {
    let r = check::catalog("/nonexistent/nuxut-i18n", "fx");
    assert!(r.has("dizin okunamadı"), "{:?}", r.errors);
    let dir = tmp("badname");
    std::fs::write(dir.join("tr.ftl"), format!("{C}fx-a = A\n")).unwrap();
    std::fs::write(dir.join("en.ftl"), format!("{C}fx-a = A\n")).unwrap();
    std::fs::write(dir.join("english.ftl"), "").unwrap();
    let r = check::catalog(&dir, "fx");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(r.has("dosya adı bir dil etiketi değil"), "{:?}", r.errors);
    assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("nuxut-i18n-check-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

// --- source_guard ---

fn scan(src: &str) -> Vec<usize> {
    SourceGuard::new("/")
        .scan_text("src/x.rs", src)
        .into_iter()
        .map(|f| f.line)
        .collect()
}

#[test]
fn guard_flags_turkish_and_ui_literals() {
    let src = r#"fn f() {
    let a = "Ayarlar açıldı";
    let b = Label::new(theme, "Settings");
    b.set_label("Volume");
    b.with_tooltip(format!("Battery {}", n));
    let ok = Label::new(theme, t!("shell-bar-clock"));
    let ok2 = Icon::new("battery");
    let ok3 = Label::new(theme, "12:30");
    let ok4 = Label::new(format!("+{n} {name}"), style);
    let bad = Label::new(format!("{n} dosya {{x}}"), style);
    let path = "/usr/share";
}
"#;
    assert_eq!(scan(src), [2, 3, 4, 5, 10]);
}

#[test]
fn guard_exclusions() {
    let src = r#"
// "yorumda Türkçe"
/* "blok yorumda ı" */
/// belge yorumu: "çok"
fn f() {
    tracing::warn!("uyarı: başarısız");
    warn!(count = 3, "şu oldu");
    eprintln!("tanı çıktısı ü");
    let x = y.expect("kilit zehirlendi; çökme");
    let k = t!("shell-x", name = "isim");
    assert_eq!(a, "ğ");
}
#[derive(Debug)]
#[serde(rename = "örnek")]
struct S;
#[cfg(test)]
mod tests {
    fn g() { let s = "Türkçe test"; Label::new(t, "Test"); }
}
#[test]
fn h() { let s = "şey"; }
#[cfg(not(test))]
fn real() { let s = "gerçek ş"; }
#[derive(thiserror::Error, Debug)]
enum E {
    #[error("dosya okunamadı")]
    Io,
}
"#;
    // cfg(not(test)) denetlenir; thiserror iletisi (Display) tanı metnidir.
    assert_eq!(scan(src), [23]);
}

#[test]
fn guard_raw_and_byte_strings() {
    assert_eq!(
        scan("fn f() { let a = r#\"ham ç\"#; let b = r\"x\"; Label::new(t, r\"Raw\"); }\n"),
        [1, 1]
    );
}

#[test]
fn guard_repo_with_allow_list_and_test_modules() {
    let root = tmp("guard");
    let src = root.join("src");
    std::fs::create_dir_all(src.join("panel")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::create_dir_all(root.join("target/src")).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "mod panel;\n#[cfg(test)]\nmod lib_tests;\nfn f() { let a = \"açık\"; }\nfn g() { let b = \"kapalı\"; }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("lib_tests.rs"),
        "fn t() { let x = \"test ş\"; }\nmod deeper;\n",
    )
    .unwrap();
    std::fs::create_dir_all(src.join("lib_tests")).unwrap();
    std::fs::write(
        src.join("lib_tests/deeper.rs"),
        "fn t() { let x = \"daha ş\"; }\n",
    )
    .unwrap();
    std::fs::write(
        src.join("panel.rs"),
        "#[cfg(test)]\n#[path = \"panel_t.rs\"]\nmod t;\nfn p() { let s = \"panel ü\"; }\n",
    )
    .unwrap();
    std::fs::write(src.join("panel_t.rs"), "fn x() { let s = \"ö\"; }\n").unwrap();
    std::fs::write(root.join("tests/it.rs"), "fn x() { let s = \"ö\"; }\n").unwrap();
    std::fs::write(
        root.join("target/src/gen.rs"),
        "fn x() { let s = \"ö\"; }\n",
    )
    .unwrap();

    let r = check::source_guard(&root);
    let mut lines: Vec<String> = r
        .errors
        .iter()
        .map(|f| format!("{}:{}", f.file, f.line))
        .collect();
    lines.sort();
    assert_eq!(
        lines,
        ["src/lib.rs:4", "src/lib.rs:5", "src/panel.rs:4"],
        "{:?}",
        r.errors
    );

    std::fs::write(
        root.join("i18n-allow.txt"),
        "# geçici liste\nsrc/lib.rs:4  geçiş: parça 1\nsrc/panel.rs:4\nsrc/old.rs:9  silinmiş dosya\nbozuk satır\n",
    )
    .unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let mut got: Vec<String> = r.errors.iter().map(ToString::to_string).collect();
    got.sort();
    assert_eq!(
        got,
        [
            "i18n-allow.txt:3: gerekçe yok",
            "i18n-allow.txt:4: kullanılmayan izin src/old.rs:9 (satırı silin)",
            "i18n-allow.txt:5: biçim 'dosya:satır|dosya:çağrı()|dosya:*|dizin/**  gerekçe' değil",
            "src/lib.rs:5: Türkçe harfli dizge sabiti (t! kullanın)",
            "src/panel.rs:4: Türkçe harfli dizge sabiti (t! kullanın)",
        ]
    );
}

#[test]
fn guard_whole_file_allow() {
    let root = tmp("guardstar");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "fn f() { let a = \"açık\"; let b = \"kapalı\"; }\n",
    )
    .unwrap();
    std::fs::write(root.join("src/tool.rs"), "fn f() { let a = \"ok\"; }\n").unwrap();
    std::fs::write(
        root.join("i18n-allow.txt"),
        "src/lib.rs:*  tanı modülü\nsrc/tool.rs:*  artık temiz\nsrc/lib.rs:0  sıfır satır yok\nsrc/gone.rs:*  silinmiş dosya\n",
    )
    .unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let got: Vec<String> = r.errors.iter().map(ToString::to_string).collect();
    // Var olan dosyanın gereksiz `dosya:*` satırı geçiş döneminde not; yok olanınki hata.
    assert_eq!(
        got,
        [
            "i18n-allow.txt:3: biçim 'dosya:satır|dosya:çağrı()|dosya:*|dizin/**  gerekçe' değil",
            "i18n-allow.txt:4: kullanılmayan izin src/gone.rs:* (satırı silin)"
        ]
    );
    // `dosya:*` destekli ama her kullanımda uyarı notu.
    let star = r
        .notes
        .iter()
        .filter(|n| n.contains("tüm dosyayı muaf tutar"))
        .count();
    assert_eq!(star, 3, "{:?}", r.notes);
    assert!(
        r.notes
            .iter()
            .any(|n| n == "i18n-allow.txt:2: gereksiz izin src/tool.rs:* — yapısal muafiyet kapsıyor (satırı silin)"),
        "{:?}",
        r.notes
    );
}

#[test]
fn guard_custom_ui_calls() {
    let g = SourceGuard::new("/").add_ui_calls(&["Banner::new", "set_heading"]);
    let f = g.scan_text(
        "src/x.rs",
        "fn f() { Banner::new(\"Hi\"); x.set_heading(\"Hey\"); Other::new(\"Hi\"); }\n",
    );
    assert_eq!(f.len(), 2, "{f:?}");
}

// --- source_guard: yapısal muafiyetler ---

/// `// HATA` ile işaretli satırlar tam olarak bulgu satırlarıdır (yorumlar taranmaz).
#[track_caller]
fn assert_marked(guard: &SourceGuard, rel: &str, src: &str) {
    let want: Vec<usize> = src
        .lines()
        .enumerate()
        .filter(|(_, l)| l.contains("// HATA"))
        .map(|(i, _)| i + 1)
        .collect();
    let mut got: Vec<usize> = guard
        .scan_text(rel, src)
        .into_iter()
        .map(|f| f.line)
        .collect();
    got.dedup();
    assert_eq!(got, want, "{rel}");
}

#[track_caller]
fn marked(src: &str) {
    assert_marked(&SourceGuard::new("/"), "src/x.rs", src);
}

#[test]
fn guard_display_and_debug_bodies_exempt() {
    marked(
        r#"
use std::fmt;
impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self { Self::Empty => "pano boş", Self::Io => "okunamadı" })?;
        write!(f, "geçersiz renk '{}' {{", self.0)
    }
}
impl<'a, T: fmt::Debug> std::fmt::Debug for Redacted<'a, T> where T: Clone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "<{} öğe>", self.0) }
}
impl Display for Plain { fn fmt(&self, f: &mut Formatter) -> Result { f.write_str("düz") } } // HATA
fn panel() {
    let l = Label::new(theme, "Türkçe"); // HATA
    let s = "açık"; // HATA
}
impl Widget for Panel {
    fn title(&self) -> &str { "Başlık" } // HATA
}
impl Panel {
    fn fmt(&self) -> String { "biçimsiz ö".into() } // HATA
}
fn show() -> impl Display {
    "dönüş konumu ş" // HATA
}
fn arg(x: impl Debug) { let y = "argüman ş"; } // HATA
impl From<u8> for Wrapper { fn from(_: u8) -> Self { Self("ç") } } // HATA
"#,
    );
}

#[test]
fn guard_error_types_exempt_only_constructors_and_attributes() {
    marked(
        r#"
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("hedef bulunamadı")]
    NotFound,
    #[error("geçersiz girdi: {0}")]
    Invalid(&'static str),
}
fn f() -> Result<(), ApplyError> {
    let _ = std::io::Error::other("üst dizin yok");
    let _ = io::Error::new(io::ErrorKind::InvalidInput, "normal dosya değil");
    let _ = ImageError::Svg(format!("iç içe derinlik {}", 3));
    let _ = RunError::Egl { msg: "libEGL yüklenemedi".into() };
    Err(ApplyError::Invalid("değer parça türüne uymuyor"))
}
impl From<image::ImageError> for ImageError {
    fn from(e: image::ImageError) -> Self { Self::Decode(format!("sınır aşıldı: {e}")) }
}
impl ImageError {
    fn pick() -> Self { Self::Svg("kısa".into()) }
}
impl Notice {
    fn make() -> Self { Self::Text("bildirim ş".into()) } // HATA
}
#[derive(thiserror::Error, Debug)]
enum E { #[error("hata ş")] A(String) }
fn g() -> E { E::A("türetilmiş hata türü kurucusu ş".into()) }
enum Plain { A(String) }
fn g2() -> Plain { Plain::A("hata olmayan tür ş".into()) } // HATA
fn h() { Toast::new(theme, ImageError::Svg("x")); let t = Toast::new(theme, "Hata oluştu"); } // HATA
fn k() -> Result<(), String> { Err("hata dizgesi ş".into()) } // HATA
"#,
    );
}

#[test]
fn guard_diagnostic_macros_and_tests_exempt() {
    marked(
        r#"
fn f(x: Option<u8>) {
    let _ = x.expect("kilit zehirlendi");
    let _ = x.unwrap_or_else(|| panic!("olmamalı ş"));
    unreachable!("ulaşılmaz ş");
    debug_assert!(true, "doğru ş");
    tracing::warn! { target: "x", "uyarı: başarısız {}", 1 };
    log::error!("günlük ş");
    let m = format!("biçim ş"); // HATA
    let _ = write!(buf, "tampona ş"); // HATA
}
#[cfg(test)]
mod tests { fn t() { let s = "test ş"; } }
#[cfg(all(test, feature = "x"))]
fn only_test() { let s = "test ş"; }
#[cfg(any(test, feature = "x"))]
fn maybe() { let s = "özellik ş"; } // HATA
"#,
    );
}

#[test]
fn guard_cli_binaries_exempt_turkish_but_not_ui_calls() {
    let g = SourceGuard::new("/");
    let src = "fn main() {\n    eprintln!(\"x\");\n    let usage = \"kullanım: araç\";\n    Label::new(t, \"Hi\"); // HATA\n}\n";
    for rel in [
        "src/main.rs",
        "crates/tool/src/main.rs",
        "src/bin/x.rs",
        "crates/a/src/bin/sub/y.rs",
    ] {
        assert_marked(&g, rel, src);
    }
    // Yalnızca tam yol eşleşir.
    for rel in [
        "src/mainx.rs",
        "src/domain.rs",
        "src/binary.rs",
        "crates/bin/src/lib.rs",
    ] {
        assert_eq!(g.scan_text(rel, src).len(), 2, "{rel}");
    }
}

#[test]
fn guard_cli_and_proc_macro_notes() {
    let root = tmp("guardcli");
    std::fs::create_dir_all(root.join("src/bin")).unwrap();
    std::fs::create_dir_all(root.join("macros/src")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    std::fs::write(
        root.join("macros/Cargo.toml"),
        "[package]\nname = \"m\"\n[lib]\nproc-macro = true\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() { let a = \"kullanım\"; let b = \"çıkış\"; }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/bin/tool.rs"),
        "fn main() { Label::new(t, \"Hi\"); }\n",
    )
    .unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn f() { let a = \"açık\"; }\n").unwrap();
    std::fs::write(
        root.join("macros/src/lib.rs"),
        "fn f() -> Result<(), String> { Err(\"geçersiz domain\".into()) }\n",
    )
    .unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let mut got: Vec<String> = r
        .errors
        .iter()
        .map(|f| format!("{}:{}", f.file, f.line))
        .collect();
    got.sort();
    assert_eq!(got, ["src/bin/tool.rs:1", "src/lib.rs:1"], "{:?}", r.errors);
    let mut notes = r.notes.clone();
    notes.sort();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes[0].starts_with("macros/src/lib.rs: proc-macro crate'i — 1 "),
        "{notes:?}"
    );
    assert!(
        notes[1].starts_with("src/main.rs: CLI ikilisi — 2 "),
        "{notes:?}"
    );
}

#[test]
fn guard_diagnostic_calls_global() {
    let g = SourceGuard::new("/").add_diagnostic_calls(&[
        "err",
        "Report::fail",
        "warnings.push",
        "1bad",
        "a.b.c",
    ]);
    let src = r#"
fn f() {
    let e = err("geçersiz kod noktası");
    Report::fail("başarısız");
    Other::fail("başka ş"); // HATA
    self.warnings.push(format!("uyarı ş"));
    all.push("liste ş"); // HATA
    let x = err; let y = "değişken ş"; // HATA
}
"#;
    assert_marked(&g, "src/x.rs", src);
    // Varsayılan bekçide hepsi bulgu.
    let plain = SourceGuard::new("/").scan_text("src/x.rs", src);
    assert_eq!(plain.len(), 6, "{plain:?}");
}

#[test]
fn guard_file_scoped_call_allows() {
    let root = tmp("guardcall");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "fn f() {\n    let e = err(\"boş\");\n    v.warnings.push(\"uyarı ş\".into());\n    let s = \"açık\";\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/other.rs"),
        "fn g() { let e = err(\"başka\"); }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("i18n-allow.txt"),
        "src/lib.rs:err()  glif tablosu tanısı\n\
         src/lib.rs:warnings.push()  config uyarısı\n\
         src/lib.rs:unused_helper()  artık yok\n\
         src/lib.rs:err()  yinelenen\n\
         src/lib.rs:1x()  geçersiz ad\n\
         src/lib.rs:Tür::ad()\n",
    )
    .unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let mut got: Vec<String> = r.errors.iter().map(ToString::to_string).collect();
    got.sort();
    assert_eq!(
        got,
        [
            "i18n-allow.txt:3: kullanılmayan izin src/lib.rs:unused_helper() (satırı silin)",
            "i18n-allow.txt:4: yinelenen izin src/lib.rs:err()",
            "i18n-allow.txt:5: biçim 'dosya:satır|dosya:çağrı()|dosya:*|dizin/**  gerekçe' değil",
            "i18n-allow.txt:6: gerekçe yok",
            // Dosyaya özgü: başka dosyadaki `err(..)` muaf değil.
            "src/lib.rs:4: Türkçe harfli dizge sabiti (t! kullanın)",
            "src/other.rs:1: Türkçe harfli dizge sabiti (t! kullanın)",
        ]
    );
}

#[test]
fn guard_file_scoped_macro_allows() {
    let root = tmp("guardmacro");
    std::fs::create_dir_all(root.join("src")).unwrap();
    // Ayarlar registry.rs biçimi: `s!()` kaynak tablosu (bilinçli Türkçe yedek metin).
    std::fs::write(
        root.join("src/registry.rs"),
        "const T: &[(&str, &str)] = &[\n    s!(\"wifi\", \"Kablosuz ağ\"),\n    s![\"ses\", \"Ses düzeyi\"],\n    s! { \"ışık\" },\n];\nfn f() { let x = \"açık\"; let y = s(\"çağrı değil\"); }\n",
    )
    .unwrap();
    std::fs::write(root.join("src/other.rs"), "const U: &str = s!(\"başka dosya\");\n").unwrap();
    std::fs::write(
        root.join("i18n-allow.txt"),
        "src/registry.rs:s!()  ayar kaynak tablosu (yedek metin)\n\
         src/registry.rs:t2!()  kullanılmıyor\n\
         src/registry.rs:1x!()  geçersiz ad\n",
    )
    .unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let mut got: Vec<String> = r.errors.iter().map(ToString::to_string).collect();
    got.sort();
    assert_eq!(
        got,
        [
            "i18n-allow.txt:2: kullanılmayan izin src/registry.rs:t2!() (satırı silin)",
            "i18n-allow.txt:3: biçim 'dosya:satır|dosya:çağrı()|dosya:*|dizin/**  gerekçe' değil",
            "src/other.rs:1: Türkçe harfli dizge sabiti (t! kullanın)",
            // Makro dışı dizge ve aynı adlı işlev çağrısı muaf değil.
            "src/registry.rs:6: Türkçe harfli dizge sabiti (t! kullanın)",
            "src/registry.rs:6: Türkçe harfli dizge sabiti (t! kullanın)",
        ]
    );
}

// --- source_guard: düşmanca tarayıcı girdileri ---

#[test]
fn guard_adversarial_lexing_keeps_regions() {
    marked(
        r##"
/* iç içe /* yorum impl fmt::Display for X { */ hâlâ yorum "ş" */
impl fmt::Display for TrickyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = ('{', '}', '\'', '"', '\\', "}", r#"} " { ş"#, b'}');
        // } kapanış yorumda
        write!(f, "{{ kaçış }} ş {}", self.0)
    }
}
fn after() { let s = "sonra ş"; } // HATA
fn raw() { Label::new(t, r#"Ham "tırnak" }"#); } // HATA
macro_rules! m { ($x:expr) => {{ let _ = "makro gövdesi ş"; }}; } // HATA
fn braces() { tracing::info!("{} {{", "içerik ş"); let t = "ç"; } // HATA
fn lifetimes<'a>(x: &'a str) -> &'a str { let c = 'ş'; "ömür ş" } // HATA
"##,
    );
}

#[test]
fn guard_unbalanced_brackets_do_not_swallow_file() {
    // Fazladan `)` ve `]` dış kapsamı erken kapatmaz; eşsiz açılış Display bölgesini dosya
    // sonuna dek taşırabilir ama panik yok.
    marked(
        r#"
impl fmt::Display for AError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("ş") ) ] }
}
fn later() { let s = "dışarıda ş"; } // HATA
fn stray() { foo(]; let s = "ç"; } // HATA
"#,
    );
    let g = SourceGuard::new("/");
    for src in [
        "impl fmt::Display for X { \"ş\"",
        "impl",
        "impl<",
        "impl fmt::Display for",
        "#[",
        "#![cfg(test)",
        "x!(",
        "Self::",
        "::E(",
        "E::V(\"ş\"",
    ] {
        let _ = g.scan_text("src/x.rs", src);
    }
}

#[test]
fn guard_never_panics_on_any_prefix() {
    let sample = r###"//! belge
#![allow(dead_code)]
use std::fmt;
#[derive(Debug, thiserror::Error)]
pub enum ImageError { #[error("çözme hatası: {0}")] Decode(String) }
impl<'a, T: fmt::Debug + 'a> fmt::Display for W<'a, T> where T: Clone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{{ş}} {:?}", self.0) }
}
fn f() -> Result<(), ImageError> {
    let c = ['\'', '"', '\\', '\u{1F600}'];
    let r = br##"x"#y"##; let s = c"cstr"; let t = r#"ham "iç" ş"#;
    /* /* iç */ */
    tracing::warn!(n = 1, "uyarı ş");
    Label::new(t, "Merhaba"); x.set_label("ç");
    Err(ImageError::Decode(format!("hata {}", 1)))
}
#[cfg(test)]
mod tests { #[test] fn t() { assert_eq!(1, 1, "ş"); } }
"###;
    let g = SourceGuard::new("/").add_diagnostic_calls(&["err"]);
    for (i, _) in sample.char_indices() {
        let _ = g.scan_text("src/x.rs", &sample[..i]);
        let _ = g.scan_text("src/main.rs", &sample[i..]);
    }
}

#[test]
fn guard_display_exempt_only_for_error_types() {
    marked(
        r#"
#[derive(Debug, thiserror::Error)]
pub(crate) struct Derived;
impl fmt::Display for Derived { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("türetilmiş hata") } }
#[derive(Debug, Clone)]
pub enum PowerAction { Off }
impl fmt::Display for PowerAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("Kapanış") } // HATA
}
impl fmt::Display for Status { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("bağlı") } } // HATA
impl std::error::Error for Manual {}
impl fmt::Display for Manual { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("elle hata") } }
impl fmt::Display for ThemeWarning { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("uyarı ş") } }
impl fmt::Debug for PowerAction { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("hata ayıklama ş") } }
#[derive(thiserror::Error, Debug)]
#[error("x")]
enum Leaky { A }
#[derive(Debug)]
struct NotAnErr;
impl fmt::Display for NotAnErr { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("sızmaz ş") } } // HATA
impl PowerAction {
    fn make() -> Self { Self::Named("ad ş".into()) } // HATA
}
"#,
    );
}

#[test]
fn guard_ui_calls_win_over_diagnostic_regions() {
    let g = SourceGuard::new("/").add_diagnostic_calls(&["err"]);
    assert_marked(
        &g,
        "src/x.rs",
        r#"
fn f() {
    let a = Label::new(t, &ApplyError::Invalid("Kaydedilemedi").to_string()); // HATA
    let b = Toast::new(t, err("Bağlantı yok")); // HATA
    let c = err("tanı ş");
    let d = Label::new(t, &t!("shell-panel-title"));
    tracing::warn!("{}", Label::new(t, "log içinde"));
    let _ = x.expect("Label::new ş");
}
#[cfg(test)]
fn t() { Label::new(t, "Test"); }
"#,
    );
}

#[test]
fn guard_machine_shaped_ui_arguments() {
    marked(
        r#"
fn ok() {
    Label::new(t, "/usr/share/icons");
    Label::new(t, "~/Resimler");
    Label::new(t, "./x.png");
    Label::new(t, "https://example.org/a");
    Label::new(t, "audio_volume_high");
    Label::new(t, "org.nuxut.shell");
    Label::new(t, "bar.height");
    Label::new(t, "network-wireless-symbolic");
    Label::new(t, "{n}");
    Label::new(t, "");
}
fn bad() {
    Label::new(t, "Settings"); // HATA
    Label::new(t, "ok"); // HATA
    Label::new(t, "Save/Cancel"); // HATA
    Label::new(t, "on/off"); // HATA
    Label::new(t, "Wi-Fi"); // HATA
    Label::new(t, "e.g."); // HATA
    Label::new(t, "up-to-date"); // HATA
    Label::new(t, "Kaydet/İptal"); // HATA
    Label::new(t, "Open file.txt"); // HATA
    Label::new(t, "Bar.Height"); // HATA
    Label::new(t, "Not_Snake"); // HATA
    Label::new(t, "a..b"); // HATA
    Label::new(t, "{n} items"); // HATA
    Label::new(t, "Mail: x://y"); // HATA
    let s = "/yol/ş"; // HATA
}
"#,
    );
}

// --- source_guard: `// i18n: tanı` işareti ---

#[test]
fn guard_marker_scope() {
    // Satır sonu işareti o satırda başlayan dizgeleri, tek başına işaret sonraki deyimi kapsar.
    marked(
        r#"fn f(x: bool, kod: i32) {
    let a = "bağlanamadı"; // i18n: tanı journal'a gider
    let b = "bağlantı"; // HATA
    let e = ("ç", "ş"); // i18n: tanı iki dizge, aynı satır
    // i18n: tanı süreç adı, kullanıcıya gösterilmez
    let c = format!(
        "çıkış {}",
        kod,
    );
    let d = "sonraki deyim kapsanmaz ş"; // HATA
    run(
        // i18n: tanı yalnızca bu argüman
        "ç",
        "ş", // HATA
    );
    // i18n: tanı if bloğu bütün
    if x {
        log("ş");
    } else {
        log("ğ");
    }
    let g = "ğ"; // HATA
    let v = vec![
        // i18n:tanı iki noktadan sonra boşluk şart değil
        Item { ad: "ö", alt: "ü" },
        Item { ad: "ö", alt: "ü" }, // HATA
    ];
    // i18n: tanı kuyruk ifadesi, kapsayan bloğun sonunda biter
    Err("hata ş".into())
}
fn sonraki() { let s = "ç"; } // HATA
"#,
    );
}

#[test]
fn guard_marker_errors() {
    let g = SourceGuard::new("/");
    let src = r#"fn f() {
    let a = "ç"; // i18n: tanı
    let b = "ç"; // i18n: tanı   
    let c = "ç"; // i18n: izin bir şey
    let d = "ç"; // i18n: tani yazım hatası
    let e = "ç"; // i18n: tanılar bitişik
    let f = "ok"; // i18n: tanı Türkçe harf yok
    eprintln!("ş"); // i18n: tanı zaten tanı makrosu
    Label::new(t, "Kaydet ş"); // i18n: tanı UI kurucusu muaf olmaz
    // i18n: tanı ilk işaret kazanır
    let h = "ş"; // i18n: tanı çakışan ikinci işaret
    /// i18n: tanı belge yorumu işaret değil
    let i = "ş";
    // i18n: tanı dosya sonunda deyim yok
}
"#;
    let got: Vec<String> = g
        .scan_text("src/x.rs", src)
        .iter()
        .map(ToString::to_string)
        .collect();
    let no_reason = "i18n işaretinde gerekçe yok ('// i18n: tanı <gerekçe>')";
    let unknown = "bilinmeyen i18n işareti (beklenen '// i18n: tanı <gerekçe>')";
    let unused = "gereksiz işaret: '// i18n: tanı' hiçbir bulguyu bastırmıyor (yorumu silin)";
    let tr = "Türkçe harfli dizge sabiti (t! kullanın)";
    let ui = "UI kurucusuna harfli dizge sabiti (t! kullanın)";
    let want: Vec<String> = [
        (2, tr),
        (2, no_reason),
        (3, tr),
        (3, no_reason),
        (4, tr),
        (4, unknown),
        (5, tr),
        (5, unknown),
        (6, tr),
        (6, unknown),
        (7, unused),
        (8, unused),
        (9, ui),
        (9, unused),
        (11, unused),
        (13, tr),
        (14, unused),
    ]
    .iter()
    .map(|(l, m)| format!("src/x.rs:{l}: {m}"))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn guard_marker_multiline_string_starts_line() {
    // Satır sonu işareti yalnızca o satırda *başlayan* dizgeyi kapsar; tek başına işaret
    // çok satırlı dizgeyi bütün kapsar.
    let g = SourceGuard::new("/");
    let f = g.scan_text("src/x.rs", "let m = \"çok\nsatır\"; // i18n: tanı x\n");
    let lines: Vec<(usize, bool)> = f
        .iter()
        .map(|f| (f.line, f.message.starts_with("gereksiz")))
        .collect();
    assert_eq!(lines, [(1, false), (2, true)], "{f:?}");
    let f = g.scan_text("src/x.rs", "// i18n: tanı x\nlet m = \"çok\nsatır\";\n");
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn guard_marker_in_cli_is_unneeded() {
    // CLI ikilisinde Türkçe sabit zaten muaf: işaret gereksiz.
    let f = SourceGuard::new("/").scan_text(
        "src/main.rs",
        "fn main() {\n    let u = \"kullanım\"; // i18n: tanı CLI\n}\n",
    );
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].line, 2);
    assert!(f[0].message.starts_with("gereksiz işaret"), "{f:?}");
}

#[test]
fn guard_marker_in_repo_and_allow_list_shrinks() {
    let root = tmp("guardmarker");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "fn f() {\n    let a = \"açık\"; // i18n: tanı log\n    let b = \"ç\"; // i18n:\n}\n",
    )
    .unwrap();
    // İşaretin kapsadığı satırın izni artık kullanılmaz → hata.
    std::fs::write(root.join("i18n-allow.txt"), "src/lib.rs:2  eski izin\n").unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let got: Vec<String> = r.errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        got,
        [
            "src/lib.rs:3: bilinmeyen i18n işareti (beklenen '// i18n: tanı <gerekçe>')",
            "src/lib.rs:3: Türkçe harfli dizge sabiti (t! kullanın)",
            "i18n-allow.txt:1: kullanılmayan izin src/lib.rs:2 (satırı silin)",
        ]
    );
}

// --- source_guard: `dizin/**` izin satırı ---

#[test]
fn guard_dir_allow() {
    let root = tmp("guarddir");
    for d in ["src/diag/sub", "src/clean", "src/diagx"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    let tr = "fn f() { let a = \"açık\"; }\n";
    std::fs::write(root.join("src/diag/a.rs"), tr).unwrap();
    std::fs::write(root.join("src/diag/sub/b.rs"), tr).unwrap();
    std::fs::write(root.join("src/diag/sub/c.rs"), tr).unwrap();
    // `src/diag/**` önek eşlemesi değil dizin eşlemesi: `src/diagx/` kapsanmaz.
    std::fs::write(root.join("src/diagx/d.rs"), tr).unwrap();
    std::fs::write(root.join("src/clean/e.rs"), "fn f() { let a = \"ok\"; }\n").unwrap();
    std::fs::write(
        root.join("i18n-allow.txt"),
        [
            "src/diag/**  tanı modülleri",              // 1: a.rs
            "src/diag/sub/**  dar kural kazanır",       // 2: b.rs
            "src/diag/sub/c.rs:*  dosya dizinden önce", // 3: c.rs
            "src/clean/**  artık temiz",                // 4: not
            "src/gone/**  silinmiş dizin",              // 5: hata
            "tests/**  taranmayan dizin",               // 6: hata
            "src/diag/**  yinelenen",                   // 7: hata
            "src/diag/sub/**",                          // 8: gerekçe yok
            "../dış/**  üst dizin",                     // 9..: bozuk
            "/mutlak/**  mutlak yol",
            "src/**/a.rs:*  ortada **",
            "src/*/**  başka glob",
            "src/a*  başka glob",
            "**  kök",
            "src/./diag/**  nokta parçası",
            "src//diag/**  boş parça",
            "src/diag/  sonda / ama ** yok",
            "src/diag/*  tek yıldız",
            "src/diag/**/  sonda /",
            "../src/lib.rs:*  dosyada da ..",
        ]
        .join("\n"),
    )
    .unwrap();
    let r = check::source_guard(&root);
    let _ = std::fs::remove_dir_all(&root);
    let mut got: Vec<String> = r.errors.iter().map(ToString::to_string).collect();
    got.sort();
    let bad = "biçim 'dosya:satır|dosya:çağrı()|dosya:*|dizin/**  gerekçe' değil";
    let mut want: Vec<String> = vec![
        "i18n-allow.txt:5: kullanılmayan izin src/gone/** — eşleşen dosya yok (satırı silin)"
            .into(),
        "i18n-allow.txt:6: kullanılmayan izin tests/** — eşleşen dosya yok (satırı silin)".into(),
        "i18n-allow.txt:7: yinelenen izin src/diag/**".into(),
        "i18n-allow.txt:8: gerekçe yok".into(),
        "src/diagx/d.rs:1: Türkçe harfli dizge sabiti (t! kullanın)".into(),
    ];
    want.extend((9..=20).map(|l| format!("i18n-allow.txt:{l}: {bad}")));
    want.sort();
    assert_eq!(got, want);
    // Her `dizin/**` kullanımı uyarı notu; gereksiz ama dosyası olan kural not.
    let warn = r
        .notes
        .iter()
        .filter(|n| n.contains("dizindeki tüm dosyaları muaf tutar"))
        .count();
    assert_eq!(warn, 6, "{:?}", r.notes);
    assert!(
        r.notes.iter().any(|n| n
            == "i18n-allow.txt:4: gereksiz izin src/clean/** — yapısal muafiyet kapsıyor (satırı silin)"),
        "{:?}",
        r.notes
    );
}
