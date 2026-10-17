//! Font veritabanı önbelleği: soğuk açılışta yüzlerce font dosyasını yeniden ayrıştırmamak için.
//!
//! `dirs::cache_dir()/nuxut/fonts-v1.bin` dosyası tam taramanın sonucunu (yüz meta verisi +
//! fontdb'nin genel aileleri) ve geçersizleştirme kayıtlarını tutar. Kayıtlar yol + mtime'dır:
//! Linux'ta fontconfig config dosyaları ve dizinleri, tüm font dizinleri ve alt dizinleri.
//! Herhangi biri değişirse (dosya eklendi/silindi/yeniden adlandırıldı, config değişti) tam
//! tarama yapılır ve dosya yeniden yazılır.
//!
//! Bilinen sınır: bir font dosyası yeniden adlandırılmadan yerinde değiştirilirse dizin mtime'ı
//! değişmez ve fark edilmez (paket güncellemeleri rename ile yazar). Kaçış: dosyayı silmek ya da
//! `NUXUT_NO_FONT_CACHE=1` (önbellek okunmaz, tam tarama yapılıp yeniden yazılır).
//!
//! Biçim (little-endian): sihirli + biçim sürümü + kütüphane etiketi, ardından kayıtlar, genel
//! aileler, yüzler; sonda gövdenin FNV-1a 64 sağlama toplamı. Bozuk/kesik/uyumsuz dosya
//! sessizce yok sayılır. Boyut ve sayılar sınırlı; okuma hiçbir girdide panik etmez.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use cosmic_text::fontdb::{
    self, Database, FaceInfo, Family, Language, Source, Stretch, Style, Weight,
};

const MAGIC: &[u8; 8] = b"NXFONTC1";
const FORMAT: u32 = 1;
/// fontdb'nin FaceInfo anlamı ve cosmic-text'in varsayılan aileleri bu sürümlere bağlı.
const LIB_TAG: &str = "fontdb-0.23+cosmic-text-0.19";
const FILE_NAME: &str = "fonts-v1.bin";
const MAX_FILE: u64 = 8 << 20;
const MAX_STR: usize = 4096;
const MAX_COUNT: usize = 65_536;
const MAX_FAMILIES: usize = 64;
/// Var olmayan yolun "mtime"ı.
const ABSENT: u64 = u64::MAX;

/// Geçersizleştirme kaydı: yol ve o anki mtime (ns) ya da `ABSENT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Record {
    path: PathBuf,
    mtime: u64,
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(ABSENT, |d| d.as_nanos().min(u128::from(ABSENT - 1)) as u64)
}

fn record(path: PathBuf) -> Record {
    let mtime = mtime(&path);
    Record { path, mtime }
}

pub(crate) fn cache_path() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("nuxut").join(FILE_NAME))
}

pub(crate) fn disabled() -> bool {
    // Birim testleri kullanıcının önbellek dosyasına dokunmaz.
    cfg!(test) || std::env::var_os("NUXUT_NO_FONT_CACHE").is_some_and(|v| v != "0")
}

// ---------------------------------------------------------------------------------------------
// Geçersizleştirme kayıtları
// ---------------------------------------------------------------------------------------------

/// Taramadan ÖNCE alınır: tarama sırasında eklenen bir font bir sonraki açılışta yakalanır.
pub(crate) fn current_records() -> Vec<Record> {
    let mut records = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for dir in font_roots(&mut records) {
        walk_dirs(&dir, &mut records, &mut seen, 0);
    }
    records
}

/// Dizini ve tüm alt dizinlerini (yalnızca dizinler) kaydeder. Döngülere karşı kanonik yol
/// kümesi ve derinlik sınırı.
fn walk_dirs(
    dir: &Path,
    records: &mut Vec<Record>,
    seen: &mut std::collections::HashSet<PathBuf>,
    depth: u32,
) {
    if depth > 16 || records.len() >= MAX_COUNT {
        return;
    }
    let Ok(canonical) = std::fs::canonicalize(dir) else {
        // Yoksa "yok" olarak kaydet: sonradan oluşturulursa geçersizleşir.
        records.push(record(dir.to_owned()));
        return;
    };
    if !seen.insert(canonical) {
        return;
    }
    records.push(record(dir.to_owned()));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();
    for sub in subdirs {
        walk_dirs(&sub, records, seen, depth + 1);
    }
}

/// Font kök dizinleri; Linux'ta fontdb'nin yaptığı gibi fontconfig'ten (config dosyaları da
/// kayıtlara eklenir), diğer platformlarda fontdb'nin sabit listeleri.
#[cfg(target_os = "linux")]
fn font_roots(records: &mut Vec<Record>) -> Vec<PathBuf> {
    let mut fc = fontconfig_parser::FontConfig::default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    // fontdb::Database::load_fontconfig ile aynı sıra ve seçim.
    let mut candidates = Vec::new();
    if let Some(file) = std::env::var_os("FONTCONFIG_FILE") {
        candidates.push(PathBuf::from(file));
        let _ = fc.merge_config(&candidates[0]);
    } else {
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|h| h.join(".config")));
        let read_global = match &xdg {
            Some(p) => {
                let user = p.join("fontconfig/fonts.conf");
                candidates.push(user.clone());
                fc.merge_config(&user).is_err()
            }
            None => true,
        };
        if read_global {
            candidates.push("/etc/fonts/local.conf".into());
            let _ = fc.merge_config(Path::new("/etc/fonts/local.conf"));
        }
        candidates.push("/etc/fonts/fonts.conf".into());
        let _ = fc.merge_config(Path::new("/etc/fonts/fonts.conf"));
    }
    // Aday kökler (yoksa "yok" olarak), birleştirilen tüm config dosyaları ve dizinleri
    // (conf.d'ye dosya eklenmesi dizin mtime'ını değiştirir).
    let mut files: Vec<PathBuf> = fc.config_files.iter().cloned().collect();
    files.sort();
    let mut parents: Vec<PathBuf> = files
        .iter()
        .filter_map(|f| f.parent().map(Path::to_owned))
        .collect();
    parents.sort();
    parents.dedup();
    records.extend(
        candidates
            .into_iter()
            .chain(files)
            .chain(parents)
            .map(record),
    );

    let mut dirs: Vec<PathBuf> = fc
        .dirs
        .into_iter()
        .filter_map(|d| match d.path.strip_prefix("~") {
            Ok(rest) => home.as_ref().map(|h| h.join(rest)),
            Err(_) => Some(d.path),
        })
        .collect();
    if dirs.is_empty() {
        // fontdb'nin fontconfig'siz yedeği.
        dirs.push("/usr/share/fonts/".into());
        dirs.push("/usr/local/share/fonts/".into());
        if let Some(h) = &home {
            dirs.push(h.join(".fonts"));
            dirs.push(h.join(".local/share/fonts"));
        }
    }
    dirs
}

#[cfg(target_os = "macos")]
fn font_roots(_records: &mut Vec<Record>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![
        "/Library/Fonts".into(),
        "/System/Library/Fonts".into(),
        "/Network/Library/Fonts".into(),
    ];
    if let Ok(entries) = std::fs::read_dir("/System/Library/AssetsV2") {
        dirs.extend(
            entries
                .flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("com_apple_MobileAsset_Font")
                })
                .map(|e| e.path()),
        );
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Fonts"));
    }
    dirs
}

#[cfg(target_os = "windows")]
fn font_roots(_records: &mut Vec<Record>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("SYSTEMROOT") {
        Some(root) => dirs.push(PathBuf::from(root).join("Fonts")),
        None => dirs.push("C:\\Windows\\Fonts\\".into()),
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home = PathBuf::from(home);
        dirs.push(home.join("AppData\\Local\\Microsoft\\Windows\\Fonts"));
        dirs.push(home.join("AppData\\Roaming\\Microsoft\\Windows\\Fonts"));
    }
    dirs
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn font_roots(_records: &mut Vec<Record>) -> Vec<PathBuf> {
    Vec::new()
}

// ---------------------------------------------------------------------------------------------
// Yükleme
// ---------------------------------------------------------------------------------------------

/// Geçerli önbellekten fontdb veritabanı kurar; yoksa/geçersizse `None` (tam tarama yapılır).
pub(crate) fn load() -> Option<Database> {
    if disabled() {
        tracing::debug!("NUXUT_NO_FONT_CACHE: font önbelleği atlandı");
        return None;
    }
    let path = cache_path()?;
    let bytes = read_limited(&path)?;
    let Some(snapshot) = decode(&bytes) else {
        tracing::debug!(path = %path.display(), "font önbelleği bozuk ya da uyumsuz; tam tarama");
        return None;
    };
    if let Some(stale) = snapshot.records.iter().find(|r| mtime(&r.path) != r.mtime) {
        tracing::debug!(changed = %stale.path.display(), "font önbelleği eski; tam tarama");
        return None;
    }
    Some(snapshot.into_database())
}

fn read_limited(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_FILE {
        tracing::debug!("font önbelleği boyut sınırını aşıyor; yok sayıldı");
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_FILE).then_some(bytes)
}

#[derive(Debug, PartialEq)]
struct FaceRec {
    path: PathBuf,
    index: u32,
    /// (ad, English_US mi)
    families: Vec<(String, bool)>,
    post_script_name: String,
    style: u8,
    weight: u16,
    stretch: u8,
    monospaced: bool,
}

#[derive(Debug, PartialEq)]
struct Snapshot {
    records: Vec<Record>,
    /// serif, sans-serif, cursive, fantasy, monospace
    generics: [String; 5],
    faces: Vec<FaceRec>,
}

impl Snapshot {
    fn from_db(db: &Database, records: Vec<Record>) -> Self {
        let generics = [
            db.family_name(&Family::Serif).to_owned(),
            db.family_name(&Family::SansSerif).to_owned(),
            db.family_name(&Family::Cursive).to_owned(),
            db.family_name(&Family::Fantasy).to_owned(),
            db.family_name(&Family::Monospace).to_owned(),
        ];
        let faces = db
            .faces()
            .filter_map(|f| {
                // Yalnızca dosyadan gelen yüzler önbelleğe girer (bellekteki kaynaklar değil).
                let path = match &f.source {
                    Source::File(p) | Source::SharedFile(p, _) => p.clone(),
                    Source::Binary(_) => return None,
                };
                Some(FaceRec {
                    path,
                    index: f.index,
                    families: f
                        .families
                        .iter()
                        .take(MAX_FAMILIES)
                        .map(|(n, l)| (n.clone(), *l == Language::English_UnitedStates))
                        .collect(),
                    post_script_name: f.post_script_name.clone(),
                    style: match f.style {
                        Style::Normal => 0,
                        Style::Italic => 1,
                        Style::Oblique => 2,
                    },
                    weight: f.weight.0,
                    stretch: stretch_to_u8(f.stretch),
                    monospaced: f.monospaced,
                })
            })
            .collect();
        Self {
            records,
            generics,
            faces,
        }
    }

    fn into_database(self) -> Database {
        let mut db = Database::new();
        for f in self.faces {
            db.push_face_info(FaceInfo {
                id: fontdb::ID::dummy(),
                source: Source::File(f.path),
                index: f.index,
                families: f
                    .families
                    .into_iter()
                    .map(|(n, en)| {
                        (
                            n,
                            if en {
                                Language::English_UnitedStates
                            } else {
                                Language::Unknown
                            },
                        )
                    })
                    .collect(),
                post_script_name: f.post_script_name,
                style: match f.style {
                    1 => Style::Italic,
                    2 => Style::Oblique,
                    _ => Style::Normal,
                },
                weight: Weight(f.weight),
                stretch: u8_to_stretch(f.stretch),
                monospaced: f.monospaced,
            });
        }
        let [serif, sans, cursive, fantasy, mono] = self.generics;
        db.set_serif_family(serif);
        db.set_sans_serif_family(sans);
        db.set_cursive_family(cursive);
        db.set_fantasy_family(fantasy);
        db.set_monospace_family(mono);
        db
    }
}

fn stretch_to_u8(s: Stretch) -> u8 {
    match s {
        Stretch::UltraCondensed => 1,
        Stretch::ExtraCondensed => 2,
        Stretch::Condensed => 3,
        Stretch::SemiCondensed => 4,
        Stretch::Normal => 5,
        Stretch::SemiExpanded => 6,
        Stretch::Expanded => 7,
        Stretch::ExtraExpanded => 8,
        Stretch::UltraExpanded => 9,
    }
}

fn u8_to_stretch(v: u8) -> Stretch {
    match v {
        1 => Stretch::UltraCondensed,
        2 => Stretch::ExtraCondensed,
        3 => Stretch::Condensed,
        4 => Stretch::SemiCondensed,
        6 => Stretch::SemiExpanded,
        7 => Stretch::Expanded,
        8 => Stretch::ExtraExpanded,
        9 => Stretch::UltraExpanded,
        _ => Stretch::Normal,
    }
}

// ---------------------------------------------------------------------------------------------
// Yazma
// ---------------------------------------------------------------------------------------------

/// Tam taramanın sonucunu (genel aileler `resolve_generic_families`'ten ÖNCE) yazar. En iyi
/// çaba: hata yalnızca loglanır. Yazım atomik (geçici dosya + rename), dizin 0700, dosya 0600.
pub(crate) fn store(db: &Database, records: Vec<Record>) {
    if cfg!(test) {
        return;
    }
    let Some(path) = cache_path() else { return };
    let bytes = encode(&Snapshot::from_db(db, records));
    if bytes.len() as u64 > MAX_FILE {
        tracing::debug!(
            bytes = bytes.len(),
            "font önbelleği sınırı aşıyor; yazılmadı"
        );
        return;
    }
    if let Err(e) = write_atomic(&path, &bytes) {
        tracing::debug!(path = %path.display(), "font önbelleği yazılamadı: {e}");
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("üst dizin yok"))?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)?;
    let tmp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Test için: veritabanını önbellek biçiminden geçirip geri kurar (diske yazmadan).
#[cfg(test)]
pub(crate) fn roundtrip_db(db: &Database) -> Option<Database> {
    Some(decode(&encode(&Snapshot::from_db(db, Vec::new())))?.into_database())
}
// ---------------------------------------------------------------------------------------------
// İkili biçim
// ---------------------------------------------------------------------------------------------

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        let s = &s.as_bytes()[..s.len().min(MAX_STR)];
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s);
    }
    fn path(&mut self, p: &Path) {
        self.str(&p.to_string_lossy());
    }
}

fn encode(s: &Snapshot) -> Vec<u8> {
    let mut w = Writer(Vec::with_capacity(64 * 1024));
    w.0.extend_from_slice(MAGIC);
    w.u32(FORMAT);
    w.str(LIB_TAG);
    let records = &s.records[..s.records.len().min(MAX_COUNT)];
    w.u32(records.len() as u32);
    for r in records {
        w.path(&r.path);
        w.u64(r.mtime);
    }
    for g in &s.generics {
        w.str(g);
    }
    let faces = &s.faces[..s.faces.len().min(MAX_COUNT)];
    w.u32(faces.len() as u32);
    for f in faces {
        w.path(&f.path);
        w.u32(f.index);
        w.u32(f.families.len() as u32);
        for (name, en) in &f.families {
            w.str(name);
            w.u8(u8::from(*en));
        }
        w.str(&f.post_script_name);
        w.u8(f.style);
        w.u16(f.weight);
        w.u8(f.stretch);
        w.u8(u8::from(f.monospaced));
    }
    let sum = fnv1a(&w.0);
    w.u64(sum);
    w.0
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn count(&mut self, max: usize) -> Option<usize> {
        let n = self.u32()? as usize;
        (n <= max).then_some(n)
    }
    fn str(&mut self) -> Option<String> {
        let n = self.count(MAX_STR)?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

fn decode(bytes: &[u8]) -> Option<Snapshot> {
    // Sağlama toplamı: son 8 bayt, öncesinin FNV-1a'sı.
    let body_len = bytes.len().checked_sub(8)?;
    let (body, sum) = bytes.split_at(body_len);
    if fnv1a(body) != u64::from_le_bytes(sum.try_into().ok()?) {
        return None;
    }
    let mut r = Reader {
        bytes: body,
        pos: 0,
    };
    if r.take(MAGIC.len())? != MAGIC || r.u32()? != FORMAT || r.str()? != LIB_TAG {
        return None;
    }
    let n = r.count(MAX_COUNT)?;
    let mut records = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let path = PathBuf::from(r.str()?);
        records.push(Record {
            path,
            mtime: r.u64()?,
        });
    }
    let generics = [r.str()?, r.str()?, r.str()?, r.str()?, r.str()?];
    let n = r.count(MAX_COUNT)?;
    let mut faces = Vec::with_capacity(n.min(4096));
    for _ in 0..n {
        let path = PathBuf::from(r.str()?);
        let index = r.u32()?;
        let nf = r.count(MAX_FAMILIES)?;
        let mut families = Vec::with_capacity(nf);
        for _ in 0..nf {
            families.push((r.str()?, r.u8()? != 0));
        }
        faces.push(FaceRec {
            path,
            index,
            families,
            post_script_name: r.str()?,
            style: r.u8()?,
            weight: r.u16()?,
            stretch: r.u8()?,
            monospaced: r.u8()? != 0,
        });
    }
    // Fazladan bayt = uyumsuz biçim.
    (r.pos == body.len()).then_some(Snapshot {
        records,
        generics,
        faces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Snapshot {
        Snapshot {
            records: vec![
                Record {
                    path: "/usr/share/fonts".into(),
                    mtime: 123,
                },
                Record {
                    path: "/yok".into(),
                    mtime: ABSENT,
                },
            ],
            generics: [
                "S".into(),
                "Sans".into(),
                "C".into(),
                "F".into(),
                "Mono".into(),
            ],
            faces: vec![FaceRec {
                path: "/usr/share/fonts/a.ttf".into(),
                index: 2,
                families: vec![("Aile".into(), true), ("Ailé".into(), false)],
                post_script_name: "Aile-Bold".into(),
                style: 1,
                weight: 700,
                stretch: 5,
                monospaced: true,
            }],
        }
    }

    #[test]
    fn roundtrip() {
        let s = sample();
        assert_eq!(decode(&encode(&s)).as_ref(), Some(&s));
    }

    #[test]
    fn corrupt_truncated_or_foreign_input_is_rejected() {
        let good = encode(&sample());
        // Her kesik önek ve her tek bayt bozulması reddedilir; hiçbiri panik etmez.
        for n in 0..good.len() {
            assert!(decode(&good[..n]).is_none(), "kesik {n}");
        }
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0x5a;
            assert!(decode(&bad).is_none(), "bozuk bayt {i}");
        }
        // Geçerli sağlama toplamlı ama yanlış sürüm.
        let mut other = good[..good.len() - 8].to_vec();
        other[8] = 99;
        let sum = fnv1a(&other);
        other.extend_from_slice(&sum.to_le_bytes());
        assert!(decode(&other).is_none());
        // Aşırı sayı iddiası, sağlama toplamı geçerli olsa da sınırda durur.
        let mut huge = Writer(Vec::new());
        huge.0.extend_from_slice(MAGIC);
        huge.u32(FORMAT);
        huge.str(LIB_TAG);
        huge.u32(u32::MAX);
        let sum = fnv1a(&huge.0);
        huge.u64(sum);
        assert!(decode(&huge.0).is_none());
        assert!(decode(&[]).is_none());
    }

    #[test]
    fn stretch_roundtrips() {
        for v in 1..=9 {
            assert_eq!(stretch_to_u8(u8_to_stretch(v)), v);
        }
    }

    #[test]
    fn walk_records_dirs_and_detects_new_entries() {
        let base =
            std::env::temp_dir().join(format!("nuxut-fontcache-test-{}", std::process::id()));
        let sub = base.join("alt");
        std::fs::create_dir_all(&sub).unwrap();
        let mut records = Vec::new();
        walk_dirs(&base, &mut records, &mut Default::default(), 0);
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| mtime(&r.path) == r.mtime));
        // Alt dizine dosya eklenince o dizinin mtime'ı değişir.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(sub.join("yeni.ttf"), b"x").unwrap();
        assert!(records.iter().any(|r| mtime(&r.path) != r.mtime));
        // Var olmayan dizin "yok" olarak kaydedilir.
        let mut missing = Vec::new();
        walk_dirs(&base.join("yok"), &mut missing, &mut Default::default(), 0);
        assert_eq!(missing[0].mtime, ABSENT);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
