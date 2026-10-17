//! Snapshot deposu (ADR-0001 §8): `$XDG_STATE_HOME/nuxut/snapshots/<id>/{manifest.toml, N.blob}`,
//! `journal.log`, `$XDG_STATE_HOME/nuxut/apply.lock`.
//!
//! - Dizinler `0700`, dosyalar `0600`; her yazım geçici dosya + `fsync` + `rename` + dizin `fsync`.
//! - Manifest ≤ [`MAX_MANIFEST_BYTES`] (260 KiB); değerler (metin ya da dosya) blob'larda
//!   (≤ 1 MiB), manifest yalnızca blake3 özetlerini taşır (üç yollu denetim ve yabancı değer tespiti
//!   blob okumadan yapılır).
//! - Bozuk/okunamayan manifest atlanır; logda yalnızca kimlik.
//! - Loglara değer/yol yazılmaz.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::error::ApplyError;
use crate::model::{FragValue, Fragment, SettingId, SnapshotId, Source, TxState};
use crate::sys;

/// Manifest sınırı: geçerli her işlemin en kötü manifest'i sığar (testli:
/// `worst_case_manifest_fits_and_oversized_is_refused`), sınırı aşan okunmaz. Hesap (TOML çıktısı):
/// - sabit başlık (sürüm, kimlik, zamanlar, ayar ≤ 64, etiket 200 karakter × 4 bayt × 2 kaçış) ≤ 4 KiB;
/// - parça başına sabit pay ≤ [`ENTRY_OVERHEAD`] (tablo başlıkları, alan adları, tür adı, üç
///   `Stored` — `before`/`after`/`prior_after` — her biri 64 hex özet + blob + kip, `file_existed`,
///   `section_created`, `file_inherited`,
///   TOML yolunun 16 bölütünün tırnak/virgülü);
/// - adres baytları en çok 2 kat (`"`/`\` kaçışı; denetim karakterleri yasak) →
///   2 × [`crate::model::MAX_ADDRESS_BYTES`].
///
/// 4 KiB + 256 × 512 B + 128 KiB = 260 KiB (ölçülen en kötü: 203 KiB, parça başı ≤ 282 B pay).
/// Gerçekçi 110 parçalık birleşik görünüm ≈ 34 KiB. Okuma bütçesi:
/// `load_all` ≤ `TOTAL_LIMIT` (500) manifest; toplam disk `TOTAL_BYTES_LIMIT` (64 MiB, manifest
/// dahil) ile budanır.
pub const MAX_MANIFEST_BYTES: u64 = 4 * 1024
    + crate::model::MAX_FRAGMENTS as u64 * ENTRY_OVERHEAD
    + 2 * crate::model::MAX_ADDRESS_BYTES as u64;
/// Parça başına manifest payı (adres baytları hariç); ölçülen en kötü ≈ 282 bayt.
pub const ENTRY_OVERHEAD: u64 = 512;
pub const MAX_BLOB_BYTES: u64 = 1024 * 1024;
/// Dizinde en çok bu kadar girdi taranır (bozuk/kötü niyetli dizin şişmesine karşı).
const MAX_DIR_ENTRIES: usize = 4096;
const MAX_JOURNAL_BYTES: u64 = 256 * 1024;
const MANIFEST_VERSION: u32 = 1;
/// Bir işlemin kaydettiği en çok yaratılmış dizin.
pub const MAX_CREATED_DIRS: usize = 256;

/// Saklanan değer: içerik blob'da, burada yalnızca özet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stored {
    Absent,
    Text { blob: u32, hash: String },
    File { blob: u32, mode: u32, hash: String },
}

impl Stored {
    pub fn hash(&self) -> Option<&str> {
        match self {
            Stored::Absent => None,
            Stored::Text { hash, .. } | Stored::File { hash, .. } => Some(hash),
        }
    }

    pub fn presence(&self) -> &'static str {
        if matches!(self, Stored::Absent) {
            "absent"
        } else {
            "set"
        }
    }

    fn blob(&self) -> Option<u32> {
        match self {
            Stored::Absent => None,
            Stored::Text { blob, .. } | Stored::File { blob, .. } => Some(*blob),
        }
    }
}

/// Değerin özeti (dosya değerinde izin hariç, yalnızca içerik).
pub fn value_hash(v: &FragValue) -> Option<String> {
    v.bytes().map(|b| blake3::hash(b).to_hex().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub fragment: Fragment,
    pub before: Stored,
    pub after: Stored,
    /// Dosya parçalarında: dosya bu işlemden önce var mıydı? (geri yüklemede boş kalan ve önceden
    /// olmayan dosya silinir.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_existed: Option<bool>,
    /// Yalnızca `Pending` sırasında, birleştirmede: bir önceki `after`. Çökmede parça ya eski
    /// `after`'ında ya yenisinde kalır; ikisi de "bizim yazdığımız" sayılır (üç yollu denetim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_after: Option<Stored>,
    /// `IniKey`'de: bölüm (`[Section]`) bu işlemden önce dosyada yoktu, Nuxut yarattı. Parça
    /// silinince bölüm boş kalırsa (gövdesinde yalnızca boş satır) başlığı da silinir —
    /// kullanıcının bölümüne dokunulmaz.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub section_created: bool,
    /// Dosya parçalarında: dosya bu işlemden önce vardı (`file_existed = true`) ama onu Nuxut
    /// yaratmıştı — yazım anında hâlâ uygulanmış (`Applied`/`Interrupted`) bir işlemin
    /// "Nuxut-yaratımı" kaydı vardı ve o kaydın yazdığı değer dosyada duruyordu. Aynı dosyayı
    /// parça parça yazan işlemler (ör. `gtk-4.0/settings.ini`'nin dört `look.*` ayarı) böylece
    /// yaratıcı bilgisini devralır: hangi sırayla geri alınırsa alınsın, dosyayı boşaltan geri
    /// yükleme onu siler. Kullanıcının (Nuxut öncesi) dosyasına asla verilmez. Eski kayıtlarda
    /// yok → `false` (önceki davranış).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub file_inherited: bool,
}

impl Entry {
    /// Bu parçanın dosyası Nuxut yaratımı mı (bu işlem yarattı ya da yaratıcıdan devraldı)?
    pub fn nuxut_file(&self) -> bool {
        self.file_existed == Some(false) || self.file_inherited
    }

    /// Şu anki değerin özeti bu parçada Nuxut'un yazdığı değer mi (`after`, çökmüş birleştirmede
    /// `prior_after` da)?
    pub fn is_ours(&self, hash: Option<&str>) -> bool {
        hash == self.after.hash() || self.prior_after.as_ref().is_some_and(|p| hash == p.hash())
    }
}

/// Saklanan özet: 64 küçük onaltılık.
fn hash_ok(h: &str) -> bool {
    h.len() == 64
        && h.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Blob indeksi sınırı (birleştirmeler büyütür; manifest boyu zaten sınırlı).
const MAX_BLOB_INDEX: u32 = 1 << 20;

impl Stored {
    /// Depodan okunan değerin biçimi parça türüne uyuyor mu?
    fn fits(&self, f: &Fragment) -> bool {
        match self {
            Stored::Absent => true,
            Stored::Text { blob, hash } => {
                *blob < MAX_BLOB_INDEX
                    && hash_ok(hash)
                    && !matches!(f, Fragment::ManagedFile { .. })
            }
            Stored::File { blob, mode, hash } => {
                *blob < MAX_BLOB_INDEX
                    && hash_ok(hash)
                    && *mode & !0o777 == 0
                    && matches!(f, Fragment::ManagedFile { .. })
            }
        }
    }
}

impl Manifest {
    /// Depodan okunan manifest güvenilmez: kimlik, sürüm, etiket, parça biçimleri (mutlak yol,
    /// `..` yok, anahtar karakterleri), değer türleri ve özet biçimleri denetlenir.
    fn is_sane(&self, id: &SnapshotId) -> bool {
        let frags = self.fragments.iter().map(|e| &e.fragment);
        let frags_ok = (self.fragments.is_empty() || crate::model::fragments_fit(frags))
            && self.fragments.iter().enumerate().all(|(i, e)| {
                e.fragment.validate().is_ok()
                    && e.fragment.file().is_none_or(|p| {
                        !p.components().any(|c| {
                            matches!(
                                c,
                                std::path::Component::ParentDir | std::path::Component::CurDir
                            )
                        })
                    })
                    && e.before.fits(&e.fragment)
                    && e.after.fits(&e.fragment)
                    && e.prior_after.as_ref().is_none_or(|p| p.fits(&e.fragment))
                    && (e.file_existed.is_none() || e.fragment.file().is_some())
                    && (!e.file_inherited || e.file_existed == Some(true))
                    && (!e.section_created
                        || matches!(&e.fragment, Fragment::IniKey { section, .. } if !section.is_empty()))
                    && !self.fragments[..i].iter().any(|o| o.fragment == e.fragment)
            });
        self.id == *id
            && self.version == MANIFEST_VERSION
            && self.label.chars().count() <= crate::model::MAX_LABEL_CHARS
            && !self.label.chars().any(char::is_control)
            && self.restores.as_ref() != Some(id)
            && frags_ok
            && self.created_dirs.len() <= MAX_CREATED_DIRS
            && self.created_dirs.iter().all(|d| {
                d.is_absolute()
                    && d.as_os_str().len() <= crate::model::MAX_PATH_BYTES
                    && d.parent().is_some()
                    && !d.components().any(|c| {
                        matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)
                    })
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub id: SnapshotId,
    /// Oluşturulma (ms, UNIX) — sıralama (aynı saniyedeki kimlikler rastgele ekle sıralanır).
    pub created_ms: u64,
    /// Son güncelleme (ms, UNIX) — 2 s birleştirme penceresi.
    pub updated_ms: u64,
    pub setting: SettingId,
    pub label: String,
    pub source: Source,
    pub state: TxState,
    /// İlk (Nuxut öncesi) değeri taşıyan işlem: budanmaz.
    pub pinned: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restores: Option<SnapshotId>,
    pub fragments: Vec<Entry>,
    /// Bu işlemin (dosya yaratırken) yarattığı ya da yazım anında uygulanmış başka bir işlemden
    /// devraldığı dizinler (kanonik, mutlak). Geri yükleme bunları en derinden başlayarak ve
    /// yalnızca boşsa siler; kullanıcının dizinine asla dokunulmaz. Eski kayıtlarda yok.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub created_dirs: Vec<PathBuf>,
}

impl Manifest {
    pub fn new(
        id: SnapshotId,
        updated_ms: u64,
        setting: SettingId,
        label: String,
        source: Source,
    ) -> Self {
        Self {
            version: MANIFEST_VERSION,
            id,
            created_ms: updated_ms,
            updated_ms,
            setting,
            label,
            source,
            state: TxState::Pending,
            pinned: false,
            restores: None,
            fragments: Vec::new(),
            created_dirs: Vec::new(),
        }
    }

    pub fn next_blob(&self) -> u32 {
        self.fragments
            .iter()
            .flat_map(|e| [e.before.blob(), e.after.blob()])
            .flatten()
            .max()
            .map_or(0, |m| m + 1)
    }
}

/// Açık depo.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    lock_path: PathBuf,
}

/// Uygulama kilidi (bırakılınca açılır).
#[derive(Debug)]
pub struct LockGuard {
    _file: File,
}

impl Store {
    /// Dizinleri hazırlar (`0700`). Herhangi bir hata → `SnapshotUnavailable`.
    pub fn open(env: &Env) -> Result<Self, ApplyError> {
        let base = env
            .nuxut_state_dir()
            .ok_or(ApplyError::SnapshotUnavailable)?;
        let dir = base.join("snapshots");
        // Test örneği: depo da izinli kök altında olmalı — dizin yaratılmadan önce denetlenir.
        let real = crate::write::resolve(&dir).map_err(|_| ApplyError::SnapshotUnavailable)?;
        env.check_path(&real)?;
        let ok = (|| -> std::io::Result<()> {
            sys::create_dir_all(&dir, 0o700)?;
            restrict_dir(&dir)?;
            Ok(())
        })();
        if ok.is_err() {
            tracing::warn!("snapshot deposu hazırlanamadı");
            return Err(ApplyError::SnapshotUnavailable);
        }
        if dir.canonicalize().ok() != Some(real) {
            return Err(ApplyError::SnapshotUnavailable);
        }
        Ok(Self {
            dir,
            lock_path: base.join("apply.lock"),
        })
    }

    /// Özel kilidi alır; en çok `wait` bekler → `Busy`.
    pub fn lock(&self, wait: Duration) -> Result<LockGuard, ApplyError> {
        let file = sys::open_private(&self.lock_path, false)
            .map_err(|_| ApplyError::SnapshotUnavailable)?;
        let start = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(LockGuard { _file: file }),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if start.elapsed() >= wait {
                        return Err(ApplyError::Busy);
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(std::fs::TryLockError::Error(_)) => {
                    return Err(ApplyError::SnapshotUnavailable);
                }
            }
        }
    }

    /// Kilidi beklemeden dener (`None` = başkası tutuyor).
    pub fn try_lock(&self) -> Option<LockGuard> {
        self.lock(Duration::ZERO).ok()
    }

    /// Geçerli adlı snapshot dizinleri, eskiden yeniye.
    pub fn ids(&self) -> Result<Vec<SnapshotId>, ApplyError> {
        let rd = std::fs::read_dir(&self.dir).map_err(|_| ApplyError::SnapshotUnavailable)?;
        // Dizin sırası keyfidir: hepsi taranır, bellekte yalnızca en yeni MAX_DIR_ENTRIES tutulur
        // (en eskiler düşer — kimlik zaman sıralı).
        let mut newest: std::collections::BinaryHeap<std::cmp::Reverse<SnapshotId>> =
            std::collections::BinaryHeap::new();
        for e in rd {
            let Ok(e) = e else { continue };
            if let Some(id) = e
                .file_name()
                .to_str()
                .and_then(|n| SnapshotId::parse(n).ok())
                && e.file_type().is_ok_and(|t| t.is_dir())
            {
                newest.push(std::cmp::Reverse(id));
                if newest.len() > MAX_DIR_ENTRIES {
                    newest.pop();
                }
            }
        }
        let mut out: Vec<SnapshotId> = newest.into_iter().map(|r| r.0).collect();
        out.sort();
        Ok(out)
    }

    fn snap_dir(&self, id: &SnapshotId) -> PathBuf {
        self.dir.join(id.as_str())
    }

    /// Manifest'i okur. Yoksa ya da bozuksa `None` (bozuksa yalnızca kimlik loglanır).
    pub fn load(&self, id: &SnapshotId) -> Option<Manifest> {
        // Snapshot dizini gerçek bir dizin olmalı (symlink ile depo dışına yönlenemez).
        if !std::fs::symlink_metadata(self.snap_dir(id)).is_ok_and(|m| m.is_dir()) {
            return None;
        }
        let path = self.snap_dir(id).join("manifest.toml");
        let text = match read_limited(&path, MAX_MANIFEST_BYTES) {
            Ok(Some(b)) => String::from_utf8(b).ok(),
            Ok(None) => return None,
            Err(()) => None,
        };
        let m = text
            .and_then(|t| toml::from_str::<Manifest>(&t).ok())
            .filter(|m| m.is_sane(id));
        if m.is_none() {
            tracing::warn!(id = %id, "bozuk snapshot manifest'i atlandı");
        }
        m
    }

    /// Tüm okunabilir manifest'ler, eskiden yeniye.
    pub fn load_all(&self) -> Result<Vec<Manifest>, ApplyError> {
        Ok(self.ids()?.iter().filter_map(|id| self.load(id)).collect())
    }

    /// Manifest'i atomik yazar (dizini gerekirse yaratır).
    pub fn save(&self, m: &Manifest) -> Result<(), ApplyError> {
        let text = toml::to_string(m).map_err(|_| ApplyError::SnapshotUnavailable)?;
        if text.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(ApplyError::SnapshotUnavailable);
        }
        let dir = self.snap_dir(&m.id);
        sys::create_dir_all(&dir, 0o700).map_err(|_| ApplyError::SnapshotUnavailable)?;
        atomic_write(&dir.join("manifest.toml"), text.as_bytes())
            .map_err(|_| ApplyError::SnapshotUnavailable)
    }

    /// Değeri blob olarak saklar.
    pub fn put(
        &self,
        id: &SnapshotId,
        next: &mut u32,
        v: &FragValue,
    ) -> Result<Stored, ApplyError> {
        let (bytes, mode) = match v {
            FragValue::Absent => return Ok(Stored::Absent),
            FragValue::Text(s) => (s.as_bytes(), None),
            FragValue::File { bytes, mode } => (bytes.as_slice(), Some(*mode)),
        };
        if bytes.len() as u64 > MAX_BLOB_BYTES {
            return Err(ApplyError::SnapshotUnavailable);
        }
        let dir = self.snap_dir(id);
        sys::create_dir_all(&dir, 0o700).map_err(|_| ApplyError::SnapshotUnavailable)?;
        let blob = *next;
        *next += 1;
        atomic_write(&dir.join(format!("{blob}.blob")), bytes)
            .map_err(|_| ApplyError::SnapshotUnavailable)?;
        let hash = blake3::hash(bytes).to_hex().to_string();
        Ok(match mode {
            None => Stored::Text { blob, hash },
            Some(mode) => Stored::File { blob, mode, hash },
        })
    }

    /// Saklanan değeri geri okur; özet tutmazsa `SnapshotUnavailable`.
    pub fn get(&self, id: &SnapshotId, s: &Stored) -> Result<FragValue, ApplyError> {
        let (blob, hash) = match s {
            Stored::Absent => return Ok(FragValue::Absent),
            Stored::Text { blob, hash } | Stored::File { blob, hash, .. } => (*blob, hash),
        };
        let bytes = read_limited(
            &self.snap_dir(id).join(format!("{blob}.blob")),
            MAX_BLOB_BYTES,
        )
        .ok()
        .flatten()
        .ok_or(ApplyError::SnapshotUnavailable)?;
        if blake3::hash(&bytes).to_hex().as_str() != hash {
            tracing::warn!(id = %id, blob, "snapshot blob'u bozuk");
            return Err(ApplyError::SnapshotUnavailable);
        }
        match s {
            Stored::Text { .. } => String::from_utf8(bytes)
                .map(FragValue::Text)
                .map_err(|_| ApplyError::SnapshotUnavailable),
            Stored::File { mode, .. } => Ok(FragValue::File { bytes, mode: *mode }),
            Stored::Absent => Ok(FragValue::Absent),
        }
    }

    /// Manifest'in başvurmadığı blob'ları siler (birleştirme sonrası).
    pub fn gc_blobs(&self, m: &Manifest) {
        let keep: Vec<u32> = m
            .fragments
            .iter()
            .flat_map(|e| [e.before.blob(), e.after.blob()])
            .flatten()
            .collect();
        let Ok(rd) = std::fs::read_dir(self.snap_dir(&m.id)) else {
            return;
        };
        for e in rd.take(MAX_DIR_ENTRIES).flatten() {
            let name = e.file_name();
            let Some(n) = name
                .to_str()
                .and_then(|n| n.strip_suffix(".blob"))
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            if !keep.contains(&n) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Depoda olmayan yeni kimlik (aynı saniyede 16 bit ek çakışırsa yeniden üretilir).
    pub fn fresh_id(&self, at: std::time::SystemTime) -> SnapshotId {
        let mut id = SnapshotId::generate(at);
        for _ in 0..16 {
            if std::fs::symlink_metadata(self.snap_dir(&id)).is_err() {
                break;
            }
            id = SnapshotId::generate(at);
        }
        id
    }

    /// Manifest'i hiç yazılmamış (blob yazılırken çökmüş) snapshot dizini mi? Bozuk manifest
    /// bu değildir (o atlanır, silinmez).
    pub fn is_orphan(&self, id: &SnapshotId) -> bool {
        let dir = self.snap_dir(id);
        std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir())
            && matches!(
                std::fs::symlink_metadata(dir.join("manifest.toml")),
                Err(e) if e.kind() == ErrorKind::NotFound
            )
    }

    /// Devralınan sabit bağlı dosyanın inode'unu tutan bağ (`keep-<yol özeti>`; blob değildir,
    /// `gc_blobs` dokunmaz, snapshot silinince gider).
    pub fn keeper(&self, id: &SnapshotId, file: &Path) -> PathBuf {
        let h = blake3::hash(file.as_os_str().as_encoded_bytes()).to_hex();
        self.snap_dir(id).join(format!("keep-{}", &h[..32]))
    }

    pub fn remove(&self, id: &SnapshotId) {
        let _ = std::fs::remove_dir_all(self.snap_dir(id));
    }

    /// Snapshot dizininin toplam boyu.
    pub fn size(&self, id: &SnapshotId) -> u64 {
        std::fs::read_dir(self.snap_dir(id))
            .map(|rd| {
                rd.take(MAX_DIR_ENTRIES)
                    .flatten()
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    /// İnsan okur günlük satırı: kimlik, durum, ayar, kaynak, parça sayısı (değer yok).
    pub fn journal(&self, m: &Manifest) {
        let path = self.dir.join("journal.log");
        if std::fs::metadata(&path).is_ok_and(|md| md.len() > MAX_JOURNAL_BYTES) {
            let _ = std::fs::rename(&path, self.dir.join("journal.log.1"));
        }
        let line = format!(
            "{} {} {} {:?} fragments={}\n",
            m.id,
            m.state.name(),
            m.setting,
            m.source,
            m.fragments.len()
        );
        if let Ok(mut f) = sys::open_private(&path, true) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// Var olan dizinin iznini `0700`'e çeker (depo kişisel içerik taşır).
fn restrict_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let m = std::fs::metadata(dir)?;
        if m.permissions().mode() & 0o777 != 0o700 {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// `Ok(None)` = yok; `Err` = okunamadı, düz dosya değil (symlink, FIFO, aygıt) ya da sınırı aşıyor.
fn read_limited(path: &Path, max: u64) -> Result<Option<Vec<u8>>, ()> {
    let mut f = match sys::open_read(path, true) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !f.metadata().is_ok_and(|m| m.is_file()) {
        return Err(());
    }
    let mut buf = Vec::new();
    (&mut f)
        .take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|_| ())?;
    if buf.len() as u64 > max {
        return Err(());
    }
    Ok(Some(buf))
}

/// Deponun kendi dosyaları için atomik yazım (`0600`).
fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or(ErrorKind::InvalidInput)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(ErrorKind::InvalidInput)?;
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let r = (|| {
        let _ = std::fs::remove_file(&tmp);
        let mut f = sys::create_tmp(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        sys::sync_dir(dir);
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}
