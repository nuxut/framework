//! Font önbelleği ölçümü: `TextSystem::new()` süresini tek açılış olarak ölçer (GUI yok).
//!
//! ```sh
//! cargo run --release -p nuxut-render --example font_cache_bench            # önbellekli
//! NUXUT_NO_FONT_CACHE=1 cargo run --release -p nuxut-render --example font_cache_bench
//! cargo run --release -p nuxut-render --example font_cache_bench -- --evict  # soğuk
//! ```
//!
//! `--evict` (yalnızca Linux): font dizinlerindeki dosyaların sayfa önbelleğini
//! `posix_fadvise(DONTNEED)` ile boşaltır (root gerekmez); "soğuk disk" yaklaşımı.

use std::time::Instant;

fn main() {
    if std::env::args().any(|a| a == "--evict") {
        evict();
    }
    let t = Instant::now();
    let text = nuxut_render::TextSystem::new();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    // Önbellekten kurulan veritabanıyla şekillendirme gerçekten çalışıyor mu (glyph verisi
    // ilk kullanımda dosyadan okunur).
    let mut text = text;
    let t = Instant::now();
    let layout = text.layout(
        "Merhaba dünya şğü",
        // font-istisnası: ölçüm genel aileyle, temadan bağımsız
        &nuxut_render::TextStyle::new("sans-serif", 14.0, 400),
        None,
    );
    let shape_ms = t.elapsed().as_secs_f64() * 1000.0;
    // Şekillendirmenin dokunduğu font dosyaları (mmap'lenmiş olanlar).
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap_or_default();
    let mut fonts: Vec<&str> = maps
        .lines()
        .filter_map(|l| l.split_whitespace().nth(5))
        .filter(|p| p.contains("/fonts/"))
        .collect();
    fonts.sort_unstable();
    fonts.dedup();
    println!(
        "ms={ms:.1} {text:?} layout={:?} ilk_sekillendirme_ms={shape_ms:.1} font_dosyasi={} {:?}",
        layout.size(),
        fonts.len(),
        fonts.iter().take(4).collect::<Vec<_>>()
    );
}

#[cfg(target_os = "linux")]
fn evict() {
    use nix::fcntl::{PosixFadviseAdvice, posix_fadvise};
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut roots: Vec<std::path::PathBuf> =
        vec!["/usr/share/fonts".into(), "/usr/local/share/fonts".into()];
    if let Some(h) = home {
        roots.push(h.join(".fonts"));
        roots.push(h.join(".local/share/fonts"));
    }
    let mut stack = roots;
    let mut files = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(file) = std::fs::File::open(&path)
                && posix_fadvise(&file, 0, 0, PosixFadviseAdvice::POSIX_FADV_DONTNEED).is_ok()
            {
                files += 1;
            }
        }
    }
    eprintln!("evict: {files} dosya");
}

#[cfg(not(target_os = "linux"))]
fn evict() {
    eprintln!("--evict yalnızca Linux'ta");
}
