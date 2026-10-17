//! Gömülü metin bekçisini bir repo kökünde çalıştırır ve bulguları dosya başına sayar
//! (taşıma parçalarını planlamak için iş listesi). Salt okunur.
//!
//! `cargo run -p nuxut-i18n --example guard -- <kök> [--list] [--raw] [--notes]`
//!
//! - `--list`: her bulguyu `dosya:satır: ileti` olarak yazar;
//! - `--raw`: `i18n-allow.txt` uygulanmaz (yapısal muafiyetler sonrası ham bulgular);
//! - `--notes`: bekçi notları (CLI/proc-macro muafiyetleri, `dosya:*` uyarıları).

use std::collections::BTreeMap;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("kullanım: guard <kök> [--list] [--raw] [--notes]");
        std::process::exit(2);
    };
    let flags: Vec<String> = args.collect();
    let has = |f: &str| flags.iter().any(|a| a == f);
    let mut guard = nuxut_i18n::check::SourceGuard::new(&root);
    if has("--raw") {
        guard = guard.allow_file("/nonexistent/i18n-allow.txt");
    }
    let report = guard.run();
    let mut per_file: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &report.errors {
        *per_file.entry(f.file.as_str()).or_default() += 1;
        if has("--list") {
            println!("{f}");
        }
    }
    if has("--notes") {
        for n in &report.notes {
            println!("not: {n}");
        }
    }
    for (file, n) in &per_file {
        println!("{n:5}  {file}");
    }
    println!(
        "{:5}  toplam ({} dosya)",
        report.errors.len(),
        per_file.len()
    );
}
