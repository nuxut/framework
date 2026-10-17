//! Paralel tam font taraması (yalnızca Linux): önbellek yokken/eskiyken soğuk açılış maliyeti.
//!
//! fontdb'nin `load_system_fonts`'u dosyaları tek iş parçacığında sırayla açar; soğuk diskte
//! maliyet ayrıştırma değil G/Ç gecikmesidir (yüzlerce dosya × birkaç rastgele okuma; bu VM'de
//! ~1000 dosya, 2–3 sn). Burada dosya listesi fontdb'nin dolaşma sırası ve tekrar eleme kuralıyla
//! aynen çıkarılır, dosyalar birkaç iş parçacığında ayrıştırılır ve yüzler LİSTE SIRASIYLA
//! veritabanına eklenir: sonuç `FontSystem::new()`in veritabanıyla yüz yüze aynıdır
//! (`parallel_scan_matches_fontdb` testi). Paralel okuma diskin kuyruk derinliğini kullanır.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use cosmic_text::fontdb::{Database, FaceInfo};

use crate::font_cache::Reuse;

/// Ayrıştırma iş parçacığı üst sınırı: G/Ç kuyruğunu doldurmaya yeter, makineyi boğmaz.
const MAX_THREADS: usize = 8;

/// fontdb `load_fontconfig` ile aynı veritabanı: takma adlardan genel aileler, ardından
/// dizinlerdeki fontlar (paralel ayrıştırma, sıralı ekleme). `reuse` değişmemiş dizinlerdeki
/// dosyaların önceki yüzlerini verir; yalnızca kalanlar ayrıştırılır.
pub(crate) fn system_database(
    dirs: &[PathBuf],
    aliases: &[fontconfig_parser::Alias],
    mut reuse: Option<Reuse>,
) -> Database {
    let mut db = Database::new();
    for alias in aliases {
        let name = alias.prefer.first().or_else(|| alias.accept.first()).or_else(|| alias.default.first());
        if let Some(name) = name {
            match alias.alias.to_lowercase().as_str() {
                // font-istisnası: fontconfig takma adının genel aile adı (fontdb load_fontconfig ile aynı)
                "serif" => db.set_serif_family(name),
                // font-istisnası: aynı
                "sans-serif" | "sans serif" => db.set_sans_serif_family(name),
                // font-istisnası: aynı
                "monospace" => db.set_monospace_family(name),
                "cursive" => db.set_cursive_family(name),
                "fantasy" => db.set_fantasy_family(name),
                _ => {}
            }
        }
    }
    let files = font_files(dirs);
    let mut slots: Vec<Option<Vec<FaceInfo>>> =
        files.iter().map(|f| reuse.as_mut().and_then(|r| r.take(f))).collect();
    let reused = slots.iter().filter(|s| s.is_some()).count();
    parse_missing(&files, &mut slots);
    tracing::debug!(files = files.len(), reused, "font taraması");
    for face in slots.into_iter().flatten().flatten() {
        db.push_face_info(face);
    }
    db
}

/// fontdb `load_fonts_dir_impl` + `canonicalize` sırası ve tekrar elemesi. (fontdb'nin "ilk
/// sembolik bağda `seen`'i yüklenmiş yüzlerle doldur" dalı `DirEntry::file_type()` bir bağda
/// `is_dir` vermediği için hiç çalışmaz; burada yok.)
fn font_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for dir in dirs {
        walk(dir, &mut files, &mut seen);
    }
    files
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let Some((path, file_type)) = canonicalize(entry.path(), &entry, seen) else {
            continue;
        };
        if file_type.is_file() {
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("ttf" | "ttc" | "TTF" | "TTC" | "otf" | "otc" | "OTF" | "OTC")
            ) {
                files.push(path);
            }
        } else if file_type.is_dir() {
            walk(&path, files, seen);
        }
    }
}

fn canonicalize(
    path: PathBuf,
    entry: &std::fs::DirEntry,
    seen: &mut HashSet<PathBuf>,
) -> Option<(PathBuf, std::fs::FileType)> {
    let file_type = entry.file_type().ok()?;
    if !file_type.is_symlink() {
        if !seen.is_empty() {
            if seen.contains(&path) {
                return None;
            }
            seen.insert(path.clone());
        }
        return Some((path, file_type));
    }
    let stat = std::fs::metadata(&path).ok()?;
    if stat.is_symlink() {
        return None;
    }
    let canon = std::fs::canonicalize(path).ok()?;
    if seen.contains(&canon) {
        return None;
    }
    seen.insert(canon.clone());
    Some((canon, stat.file_type()))
}

/// Boş yuvaların dosyalarını iş parçacıklarında ayrıştırır; yuva sırası dosya sırasıdır.
fn parse_missing(files: &[PathBuf], slots: &mut [Option<Vec<FaceInfo>>]) {
    let missing: Vec<usize> = (0..files.len()).filter(|&i| slots[i].is_none()).collect();
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, MAX_THREADS)
        .min(missing.len());
    if threads == 0 {
        return;
    }
    let next = AtomicUsize::new(0);
    let parsed: Vec<(usize, Vec<FaceInfo>)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    while let Some(&i) = missing.get(next.fetch_add(1, Ordering::Relaxed)) {
                        out.push((i, parse_file(&files[i])));
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    for (i, faces) in parsed {
        slots[i] = Some(faces);
    }
}

/// Tek dosyanın yüzleri (koleksiyonda hepsi). fontdb'nin kendi ayrıştırıcısı: alan anlamı aynı.
fn parse_file(path: &Path) -> Vec<FaceInfo> {
    let mut db = Database::new();
    if let Err(e) = db.load_font_file(path) {
        tracing::debug!(path = %path.display(), "font dosyası okunamadı: {e}");
    }
    db.faces().cloned().collect()
}

/// Font dosyalarının baş kısmını arka planda sayfa önbelleğine ister (`POSIX_FADV_WILLNEED`).
///
/// cosmic-text her yeni öznitelik kümesi için (`get_font_matches`) ağırlığı istenenden farklı HER
/// yüzün dosyasını açıp `wght` eksenine bakar: soğuk diskte bu ilk şekillendirmeyi sıralı
/// yüzlerce okumayla saniyelerce bekletir (önbellekli açılışta bile). Burada çekirdekten
/// tablo dizini ve çevresi (`PREFETCH_BYTES`) zaman uyumsuz istenir; okumalar paralel kuyruğa
/// girer, iş parçacığı beklemez. Sayfalar zaten bellekteyse maliyet dosya başına üç sistem
/// çağrısıdır. Tek seferlik, kısa ömürlü bir iş parçacığı; boşta iş yok.
pub(crate) fn prefetch(db: &Database) {
    /// Çekirdeğin mmap hata çevresi okumasıyla (128 KiB) aynı pencere.
    const PREFETCH_BYTES: u64 = 128 << 10;
    const PREFETCH_THREADS: usize = 4;
    let mut files: Vec<PathBuf> = db
        .faces()
        .filter_map(|f| match &f.source {
            cosmic_text::fontdb::Source::File(p) => Some(p.clone()),
            _ => None,
        })
        .collect();
    files.dedup();
    if files.is_empty() {
        return;
    }
    // `open` soğuk inode önbelleğinde eşzamanlı okuma yapar: birkaç iş parçacığı ile paralel.
    let spawned = std::thread::Builder::new().name("nuxut-font-prefetch".into()).spawn(move || {
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..PREFETCH_THREADS {
                s.spawn(|| {
                    while let Some(path) = files.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let Ok(file) = std::fs::File::open(path) else { continue };
                        let len = std::num::NonZeroU64::new(PREFETCH_BYTES);
                        let _ = rustix::fs::fadvise(&file, 0, len, rustix::fs::Advice::WillNeed);
                    }
                });
            }
        });
    });
    if let Err(e) = spawned {
        tracing::debug!("font ön okuma iş parçacığı açılamadı: {e}");
    }
}
