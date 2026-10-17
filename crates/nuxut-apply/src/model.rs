//! Uygulama modeli (ADR-0001 §3): parça, değer, değişiklik, kimlikler, işlem durumu.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::ApplyError;

/// Bir değişiklikteki (ve birleşik işlemdeki) en çok parça sayısı. "Ayar başına tek snapshot"
/// kuralı bölmeye izin vermez; birleşik görünüm (`look.mode`: kdeglobals 7 grup × 12 anahtar +
/// GTK/Qt dosyaları) tek değişiklikte ~110 parçadır → 2× pay ile 256. Manifest boyu
/// [`MAX_ADDRESS_BYTES`] ile birlikte sınırlanır (`store::MAX_MANIFEST_BYTES` hesabı).
pub const MAX_FRAGMENTS: usize = 256;
/// Bir değişiklikteki (birleşik işlemde birleşimdeki) tüm parça adreslerinin (yol + anahtar/bölüm/
/// işaret/kimlik baytları, [`Fragment::address_bytes`]) toplam sınırı. Gerçekçi 110 parça ≈ 8 KiB;
/// parça başına alan sınırları (yol 4096, işaret 1024, TOML yolu 16 × 128 …) tek başına 256 parçada
/// MiB'lerce manifest verirdi — toplam bütçe manifest boyunu parça sayısından bağımsız sınırlar.
pub const MAX_ADDRESS_BYTES: usize = 64 * 1024;
/// Parça dosya yolu sınırı (Linux `PATH_MAX`).
pub const MAX_PATH_BYTES: usize = 4096;
/// Metin değeri sınırı (TOML/INI/satır/blok/dconf/birim).
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
/// Etiket sınırı (karakter).
pub const MAX_LABEL_CHARS: usize = 200;

/// Mantıksal ayar kimliği (`look.unified`, `hypr.source`, `migration.shell`): `[a-z0-9_.]`,
/// 1..=64 bayt, nokta ile başlamaz/bitmez, ardışık nokta yok. Ayarlar kataloğuyla aynı.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SettingId(String);

impl SettingId {
    pub const MAX_LEN: usize = 64;

    pub fn new(s: &str) -> Result<Self, ApplyError> {
        let ok = !s.is_empty()
            && s.len() <= Self::MAX_LEN
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'.')
            && !s.starts_with('.')
            && !s.ends_with('.')
            && !s.contains("..");
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(ApplyError::Invalid("geçersiz ayar kimliği"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SettingId {
    type Error = ApplyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(&s)
    }
}

impl From<SettingId> for String {
    fn from(s: SettingId) -> String {
        s.0
    }
}

impl fmt::Display for SettingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SettingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SettingId({})", self.0)
    }
}

/// Zaman sıralı snapshot kimliği: `20261010T120000Z-3fa1` (UTC + 4 küçük onaltılık).
/// Dizin adı olarak kullanılır; biçim katı denetlenir (yol kaçışı yok).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SnapshotId(String);

impl SnapshotId {
    pub const LEN: usize = 21;

    pub fn parse(s: &str) -> Result<Self, ApplyError> {
        let b = s.as_bytes();
        let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
        let ok = b.len() == Self::LEN
            && digits(0..8)
            && b[8] == b'T'
            && digits(9..15)
            && b[15] == b'Z'
            && b[16] == b'-'
            && b[17..]
                .iter()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c));
        if ok {
            Ok(Self(s.to_owned()))
        } else {
            Err(ApplyError::Invalid("geçersiz snapshot kimliği"))
        }
    }

    /// Verilen zaman ve 16 bitlik rastgele ekten.
    pub fn from_parts(at: SystemTime, rand: u16) -> Self {
        let secs = at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (y, mo, d, h, mi, s) = civil(secs);
        Self(format!(
            "{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z-{rand:04x}"
        ))
    }

    /// Şimdi + süreç içi rastgele (std `RandomState`, saat, pid, sayaç).
    pub fn generate(at: SystemTime) -> Self {
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(at.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.write_u32(std::process::id());
        h.write_u64(N.fetch_add(1, Ordering::Relaxed));
        Self::from_parts(at, h.finish() as u16)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Kimlikteki zaman (saniye, UNIX).
    pub fn unix_secs(&self) -> u64 {
        let n = |r: std::ops::Range<usize>| self.0[r].parse::<u64>().unwrap_or(0);
        days_from_civil(n(0..4) as i64, n(4..6) as u32, n(6..8) as u32).max(0) as u64 * 86_400
            + n(9..11) * 3600
            + n(11..13) * 60
            + n(13..15)
    }

    /// ISO 8601 UTC (`2026-10-10T12:00:00Z`).
    pub fn iso(&self) -> String {
        let s = &self.0;
        format!(
            "{}-{}-{}T{}:{}:{}Z",
            &s[0..4],
            &s[4..6],
            &s[6..8],
            &s[9..11],
            &s[11..13],
            &s[13..15]
        )
    }
}

impl TryFrom<String> for SnapshotId {
    type Error = ApplyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::parse(&s)
    }
}

impl From<SnapshotId> for String {
    fn from(s: SnapshotId) -> String {
        s.0
    }
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SnapshotId({})", self.0)
    }
}

/// UNIX saniyesinden UTC (yıl, ay, gün, saat, dakika, saniye). Howard Hinnant'ın algoritması.
fn civil(secs: u64) -> (i64, u32, u32, u64, u64, u64) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (
        if m <= 2 { y + 1 } else { y },
        m,
        d,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
    )
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// TOML anahtar yolu (`["gtk", "theme"]`). Bölüt: 1..=128 bayt, denetim karakteri yok; en çok 16.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct KeyPath(Vec<String>);

impl KeyPath {
    pub fn new<I, S>(segments: I) -> Result<Self, ApplyError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let v: Vec<String> = segments.into_iter().map(Into::into).collect();
        let ok = !v.is_empty()
            && v.len() <= 16
            && v.iter()
                .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control));
        if ok {
            Ok(Self(v))
        } else {
            Err(ApplyError::Invalid("geçersiz TOML anahtar yolu"))
        }
    }

    /// Noktalı yazım (`a.b.c`; tırnaklı bölüt desteklenmez).
    pub fn dotted(s: &str) -> Result<Self, ApplyError> {
        Self::new(s.split('.'))
    }

    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl TryFrom<Vec<String>> for KeyPath {
    type Error = ApplyError;
    fn try_from(v: Vec<String>) -> Result<Self, Self::Error> {
        Self::new(v)
    }
}

impl From<KeyPath> for Vec<String> {
    fn from(k: KeyPath) -> Self {
        k.0
    }
}

impl fmt::Display for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("."))
    }
}

impl fmt::Debug for KeyPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyPath({self})")
    }
}

/// Eksik satır/blok nereye eklenir. CSS `@import` en başta olmalı → `Start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    Start,
    #[default]
    End,
}

/// Bir yazım hedefinin dokunulan en küçük birimi.
///
/// `Debug` yolu göstermez (değişmez 9).
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fragment {
    /// TOML anahtarı (`toml_edit`; biçim ve yorumlar korunur). Değer: TOML değer ifadesi
    /// (`"Adwaita"`, `true`, `[1, 2]`).
    TomlKey { file: PathBuf, key: KeyPath },
    /// INI `[bölüm] anahtar` (satır tabanlı; `kdeglobals` `[$e]` ekleri korunur). `section = ""`
    /// ilk başlıktan önceki alan. Değer: `=`'den sonraki metin.
    IniKey {
        file: PathBuf,
        section: String,
        key: String,
    },
    /// Tam satır (`source = ~/.config/hypr/nuxut.conf`, `@import 'nuxut.css';`). Değer: satırın
    /// kendisi (`marker` ile aynı) — var/yok.
    Line {
        file: PathBuf,
        marker: String,
        #[serde(default)]
        at: Position,
    },
    /// `# >>> nuxut:<id>` … `# <<< nuxut:<id>` arası. Değer: iç metin (satır sonuyla biter).
    Block { file: PathBuf, id: String },
    /// Tümü Nuxut'un dosya (başlık satırıyla tanınır). Değer: `FragValue::File`.
    ManagedFile { file: PathBuf },
    /// dconf anahtarı (`/org/gnome/desktop/interface/gtk-theme`). Değer: GVariant metni.
    Dconf { key: String },
    /// systemd `--user` birimi (`nuxut-shell.service`). Değer: `enabled` | `disabled`.
    UnitState { unit: String },
    /// Dosya yolunun **biçimi** (içeriği değil): devralma ([`Change::take_over`]) kaydı. İstekte
    /// verilmez; `apply` devralınan her dosya için ekler. Değer: [`FileForm`] metni (`regular`,
    /// `symlink:<hedef>`, `mode:<sekizli>`, `hardlink`).
    FileForm { file: PathBuf },
}

impl Fragment {
    /// Kısa tür adı (log/IPC).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Fragment::TomlKey { .. } => "toml_key",
            Fragment::IniKey { .. } => "ini_key",
            Fragment::Line { .. } => "line",
            Fragment::Block { .. } => "block",
            Fragment::ManagedFile { .. } => "managed_file",
            Fragment::Dconf { .. } => "dconf",
            Fragment::UnitState { .. } => "unit_state",
            Fragment::FileForm { .. } => "file_form",
        }
    }

    /// Dosya hedefliyse dosya yolu.
    pub fn file(&self) -> Option<&Path> {
        match self {
            Fragment::TomlKey { file, .. }
            | Fragment::IniKey { file, .. }
            | Fragment::Line { file, .. }
            | Fragment::Block { file, .. }
            | Fragment::ManagedFile { file }
            | Fragment::FileForm { file } => Some(file),
            Fragment::Dconf { .. } | Fragment::UnitState { .. } => None,
        }
    }

    /// Alan biçimlerini denetler (yol mutlak, denetim karakteri yok, kimlik karakterleri).
    pub fn validate(&self) -> Result<(), ApplyError> {
        if let Some(f) = self.file()
            && (!f.is_absolute()
                || f.to_str()
                    .is_none_or(|s| s.len() > MAX_PATH_BYTES || s.chars().any(char::is_control)))
        {
            return Err(ApplyError::Invalid(
                "parça yolu mutlak UTF-8 olmalı (≤ 4096 bayt, denetim karakteri yok)",
            ));
        }
        let line_ok =
            |s: &str| !s.is_empty() && s.len() <= 1024 && !s.chars().any(char::is_control);
        match self {
            Fragment::TomlKey { .. } | Fragment::ManagedFile { .. } | Fragment::FileForm { .. } => {
                Ok(())
            }
            Fragment::IniKey { section, key, .. } => {
                let section_ok = section.len() <= 256 && !section.chars().any(char::is_control);
                let key_ok = line_ok(key)
                    && !key.contains('=')
                    // kdeglobals bayrak eki düzenleyiciye aittir (`key[$e]` → `key` ile eşleşir).
                    && !key.contains("[$")
                    && !key.starts_with(['[', '#', ';'])
                    && key.trim() == key;
                if section_ok && key_ok {
                    Ok(())
                } else {
                    Err(ApplyError::Invalid("geçersiz INI bölüm/anahtar"))
                }
            }
            Fragment::Line { marker, .. } => {
                if line_ok(marker) && marker.trim() == marker {
                    Ok(())
                } else {
                    Err(ApplyError::Invalid("geçersiz satır işareti"))
                }
            }
            Fragment::Block { id, .. } => {
                let ok = !id.is_empty()
                    && id.len() <= 64
                    && id.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b)
                    });
                if ok {
                    Ok(())
                } else {
                    Err(ApplyError::Invalid("geçersiz blok kimliği"))
                }
            }
            Fragment::Dconf { key } => {
                let ok = key.len() <= 256
                    && key.starts_with('/')
                    && !key.ends_with('/')
                    && !key.contains("//")
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b));
                if ok {
                    Ok(())
                } else {
                    Err(ApplyError::Invalid("geçersiz dconf anahtarı"))
                }
            }
            Fragment::UnitState { unit } => {
                let ok = unit.len() <= 128
                    && !unit.starts_with(['-', '.'])
                    && unit
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b":_.@-\\".contains(&b))
                    && [".service", ".timer", ".socket", ".target", ".path"]
                        .iter()
                        .any(|s| unit.ends_with(s));
                if ok {
                    Ok(())
                } else {
                    Err(ApplyError::Invalid("geçersiz birim adı"))
                }
            }
        }
    }
}

impl Fragment {
    /// Adres baytları: yol + anahtar/bölüm/işaret/kimlik (değer hariç). Manifest boyunun parça
    /// sayısıyla değişmeyen kısmı bunlardan gelir ([`MAX_ADDRESS_BYTES`]).
    pub fn address_bytes(&self) -> usize {
        let path = self.file().map_or(0, |p| p.as_os_str().len());
        path + match self {
            Fragment::TomlKey { key, .. } => key.segments().iter().map(String::len).sum(),
            Fragment::IniKey { section, key, .. } => section.len() + key.len(),
            Fragment::Line { marker, .. } => marker.len(),
            Fragment::Block { id, .. } => id.len(),
            Fragment::ManagedFile { .. } | Fragment::FileForm { .. } => 0,
            Fragment::Dconf { key } => key.len(),
            Fragment::UnitState { unit } => unit.len(),
        }
    }
}

/// Parça kümesi tek işleme (manifest'e) sığar mı: 1..=[`MAX_FRAGMENTS`] parça ve adres toplamı
/// ≤ [`MAX_ADDRESS_BYTES`]. İstek, birleştirme ve depodan okuma aynı ölçütü kullanır.
pub fn fragments_fit<'a>(frags: impl IntoIterator<Item = &'a Fragment>) -> bool {
    let (mut n, mut bytes) = (0usize, 0usize);
    for f in frags {
        n += 1;
        bytes += f.address_bytes();
        if n > MAX_FRAGMENTS || bytes > MAX_ADDRESS_BYTES {
            return false;
        }
    }
    n > 0
}

impl fmt::Debug for Fragment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fragment::TomlKey { key, .. } => write!(f, "TomlKey({key})"),
            Fragment::IniKey { section, key, .. } => write!(f, "IniKey([{section}] {key})"),
            Fragment::Line { at, .. } => write!(f, "Line({at:?})"),
            Fragment::Block { id, .. } => write!(f, "Block({id})"),
            Fragment::ManagedFile { .. } => f.write_str("ManagedFile"),
            Fragment::Dconf { key } => write!(f, "Dconf({key})"),
            Fragment::UnitState { unit } => write!(f, "UnitState({unit})"),
            Fragment::FileForm { .. } => f.write_str("FileForm"),
        }
    }
}

/// Bir parçanın değeri. `Absent` = anahtar/satır/dosya yok (geri yükleme siler).
///
/// `Debug` içeriği göstermez, yalnızca türü ve boyu.
#[derive(Clone, PartialEq, Eq)]
pub enum FragValue {
    Absent,
    Text(String),
    /// Bütün dosya; `mode` 0 = var olanınki korunur / yeni dosya `0644`. Yalnızca `0o777`
    /// bitleri (setuid/setgid/sticky reddedilir).
    File {
        bytes: Vec<u8>,
        mode: u32,
    },
}

impl FragValue {
    pub fn is_absent(&self) -> bool {
        matches!(self, FragValue::Absent)
    }

    /// `set` | `absent` (IPC özeti).
    pub fn presence(&self) -> &'static str {
        if self.is_absent() { "absent" } else { "set" }
    }

    /// İçerik baytları (`Absent` → `None`).
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            FragValue::Absent => None,
            FragValue::Text(s) => Some(s.as_bytes()),
            FragValue::File { bytes, .. } => Some(bytes),
        }
    }
}

impl fmt::Debug for FragValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FragValue::Absent => f.write_str("Absent"),
            FragValue::Text(s) => write!(f, "Text(<{} bayt>)", s.len()),
            FragValue::File { bytes, mode } => write!(f, "File(<{} bayt>, {mode:o})", bytes.len()),
        }
    }
}

/// Değişikliği kim başlattı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Settings,
    Shell,
    Welcome,
    Migration,
    Cli,
}

/// Bir uygulama isteği: hangi ayar, hangi parçalar hangi değere.
#[derive(Clone, PartialEq, Eq)]
pub struct Change {
    pub setting: SettingId,
    /// Kullanıcıya gösterilen kısa açıklama (≤ 200 karakter; değer içermemeli).
    pub label: String,
    pub source: Source,
    pub writes: Vec<(Fragment, FragValue)>,
    /// Eksikse yaratılabilecek `Line`/`Block` dosyaları (açık seçenek; varsayılan boş: kullanıcının
    /// ana dosyası yaratılmaz). Her yol bu istekteki bir `Line`/`Block` parçasının `file`'ı olmalı.
    /// Yaratılan dosya `0644` (eksik dizinler `0755`); geri yüklemede yalnızca bizim
    /// satırımız/bloğumuz kaldıysa (boş kalırsa) silinir. [`Change::write_creating`] doldurur.
    pub create_missing: Vec<PathBuf>,
    /// Devralınacak dosyalar ([`Change::take_over`]); varsayılan boş: başka aracın yönettiği
    /// dosyaya yazılmaz.
    pub take_over: Vec<PathBuf>,
}

impl Change {
    pub fn new(setting: SettingId, label: impl Into<String>, source: Source) -> Self {
        Self {
            setting,
            label: label.into(),
            source,
            writes: Vec::new(),
            create_missing: Vec::new(),
            take_over: Vec::new(),
        }
    }

    #[must_use]
    pub fn write(mut self, fragment: Fragment, value: FragValue) -> Self {
        self.writes.push((fragment, value));
        self
    }

    /// [`Change::write`] gibi; parça `Line`/`Block` ise dosyası yoksa yaratılır (ör. `gtk.css` yoksa
    /// yalnızca `@import` satırıyla). Diğer parça türlerinde `write` ile aynıdır.
    #[must_use]
    pub fn write_creating(mut self, fragment: Fragment, value: FragValue) -> Self {
        if let Fragment::Line { file, .. } | Fragment::Block { file, .. } = &fragment
            && !self.create_missing.contains(file)
        {
            self.create_missing.push(file.clone());
        }
        self.writes.push((fragment, value));
        self
    }

    /// Başka bir aracın yönettiği (`ExternallyManaged`) `path`'i devralır (shell
    /// docs/design/look-external.md §4). API isteğe bağlıdır; Nuxut çağıranları kullanıcı ilkesi
    /// gereği (2026-10-16) onay beklemeden, yedekli ve geri alınabilir biçimde **varsayılan
    /// olarak** çağırır. Dosyanın özgün biçimi (symlink hedef
    /// metni, salt okunur izin bitleri, sabit bağ) işlemin snapshot'ına [`Fragment::FileForm`]
    /// olarak girer, yol Nuxut'un yazabildiği düz dosyaya çevrilir (symlink → içerik kopyası,
    /// salt okunur → `u+w`, sabit bağ → bağı koparan kopya) ve parçalar ona yazılır — hepsi tek
    /// işlem. Geri yükleme özgün biçimi geri koyar (symlink yeniden kurulur, izin geri alınır,
    /// sabit bağ depoda tutulan bağla yeniden kurulur). Symlink'in hedefine hiç yazılmaz (başka
    /// kullanıcıya ait hedefi gösteren bağ da devralınır). `path` bu istekteki bir dosya
    /// parçasının `file`'ı olmalı. Devralınamayanlar (başka sahibin düz dosyası, nix/guix deposu,
    /// yazılamayan dizin) yine `ExternallyManaged`; dışarıdan yönetilmeyen dosyada etkisizdir.
    #[must_use]
    pub fn take_over(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        if !self.take_over.contains(&path) {
            self.take_over.push(path);
        }
        self
    }

    /// `file` eksikse yaratılabilir mi?
    pub(crate) fn may_create(&self, file: &Path) -> bool {
        self.create_missing.iter().any(|p| p == file)
    }

    /// Sınırlar, parça biçimleri, değer türü uyumu, yinelenen parça yok.
    pub fn validate(&self) -> Result<(), ApplyError> {
        if !fragments_fit(self.writes.iter().map(|(f, _)| f)) {
            return Err(ApplyError::Invalid(
                "parça sayısı 1..=256, adres toplamı ≤ 64 KiB olmalı",
            ));
        }
        if self.label.chars().count() > MAX_LABEL_CHARS || self.label.chars().any(char::is_control)
        {
            return Err(ApplyError::Invalid("geçersiz etiket"));
        }
        for (i, (frag, value)) in self.writes.iter().enumerate() {
            frag.validate()?;
            if self.writes[..i].iter().any(|(f, _)| f == frag) {
                return Err(ApplyError::Invalid("aynı parça iki kez"));
            }
            // Yönetilen dosya bütün dosyadır: aynı dosyada başka parça olamaz.
            if let Some(file) = frag.file() {
                let clash = self.writes.iter().enumerate().any(|(j, (g, _))| {
                    j != i
                        && g.file() == Some(file)
                        && (matches!(frag, Fragment::ManagedFile { .. })
                            || matches!(g, Fragment::ManagedFile { .. }))
                });
                if clash {
                    return Err(ApplyError::Invalid("yönetilen dosyada başka parça olamaz"));
                }
            }
            value_fits(frag, value)?;
        }
        // Yaratma izni yalnızca bu istekteki satır/blok dosyalarına (başka dosya "yaratılabilir"
        // ilan edilemez); yinelenen yol yok.
        for (i, p) in self.create_missing.iter().enumerate() {
            let owned = self.writes.iter().any(|(f, _)| {
                matches!(f, Fragment::Line { file, .. } | Fragment::Block { file, .. } if file == p)
            });
            if !owned || self.create_missing[..i].contains(p) {
                return Err(ApplyError::Invalid(
                    "yaratma izni yalnızca bu istekteki satır/blok dosyasına",
                ));
            }
        }
        // Biçim parçası iç kayıttır; devralma yalnızca bu istekteki dosyalara.
        if self.writes.iter().any(|(f, _)| matches!(f, Fragment::FileForm { .. })) {
            return Err(ApplyError::Invalid("biçim parçası istekte verilmez (take_over)"));
        }
        for (i, p) in self.take_over.iter().enumerate() {
            if !self.writes.iter().any(|(f, _)| f.file() == Some(p.as_path()))
                || self.take_over[..i].contains(p)
            {
                return Err(ApplyError::Invalid("devralma yalnızca bu istekteki dosyalara"));
            }
        }
        Ok(())
    }
}

/// Dosya yolunun biçimi ([`Fragment::FileForm`] değeri; devralma kaydı). Metin biçimi:
/// `regular` | `symlink:<readlink metni>` | `mode:<sekizli>` | `hardlink` | `foreign:<sekizli>`.
/// `Debug` hedefi
/// göstermez (değişmez 9).
#[derive(Clone, PartialEq, Eq)]
pub enum FileForm {
    /// Nuxut'un yazabildiği düz dosya (sahibi, `u+w`, tek bağ).
    Regular,
    /// Symlink; `readlink` metni (göreli olabilir).
    Symlink(String),
    /// Sahibine salt okunur düz dosya; izin bitleri (⊆ `0o777`, `u+w` yok).
    ReadOnly(u32),
    /// Sabit bağlı düz dosya (`nlink > 1`); inode snapshot dizininde bir bağla tutulur.
    HardLink,
    /// Başka kullanıcıya ait düz dosya; izin bitleri (⊆ `0o777`). Geri alma içeriği ve izni
    /// kullanıcının sahipliğinde geri koyar (2026-10-17).
    Foreign(u32),
}

impl FileForm {
    pub fn to_text(&self) -> String {
        match self {
            FileForm::Regular => "regular".into(),
            FileForm::Symlink(t) => format!("symlink:{t}"),
            FileForm::ReadOnly(m) => format!("mode:{m:o}"),
            FileForm::HardLink => "hardlink".into(),
            FileForm::Foreign(m) => format!("foreign:{m:o}"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        if let Some(t) = s.strip_prefix("symlink:") {
            let ok = !t.is_empty() && t.len() <= MAX_PATH_BYTES && !t.chars().any(char::is_control);
            return ok.then(|| FileForm::Symlink(t.to_owned()));
        }
        if let Some(m) = s.strip_prefix("foreign:") {
            let m = u32::from_str_radix(m, 8).ok()?;
            return (m & !0o777 == 0).then_some(FileForm::Foreign(m));
        }
        if let Some(m) = s.strip_prefix("mode:") {
            let m = u32::from_str_radix(m, 8).ok()?;
            return (m & !0o777 == 0 && m & 0o200 == 0).then_some(FileForm::ReadOnly(m));
        }
        match s {
            "regular" => Some(FileForm::Regular),
            "hardlink" => Some(FileForm::HardLink),
            _ => None,
        }
    }
}

impl fmt::Debug for FileForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileForm::Regular => f.write_str("Regular"),
            FileForm::Symlink(_) => f.write_str("Symlink"),
            FileForm::ReadOnly(m) => write!(f, "ReadOnly({m:o})"),
            FileForm::HardLink => f.write_str("HardLink"),
            FileForm::Foreign(m) => write!(f, "Foreign({m:o})"),
        }
    }
}

/// Değer parça türüne uyuyor mu (tür, boy, satır sonu/NUL yok, birim durumu, satır = işaret)?
/// Hem istekler hem de depodan okunan (güvenilmeyen) geri yükleme değerleri için.
pub(crate) fn value_fits(frag: &Fragment, value: &FragValue) -> Result<(), ApplyError> {
    let ok = match (frag, value) {
        (_, FragValue::Absent) => true,
        (Fragment::ManagedFile { .. }, FragValue::File { bytes, mode }) => {
            bytes.len() as u64 <= crate::MAX_FILE_BYTES && *mode & !0o777 == 0
        }
        (Fragment::ManagedFile { .. }, FragValue::Text(_)) | (_, FragValue::File { .. }) => false,
        (Fragment::Line { marker, .. }, FragValue::Text(t)) => t == marker,
        (Fragment::UnitState { .. }, FragValue::Text(t)) => t == "enabled" || t == "disabled",
        (Fragment::FileForm { .. }, FragValue::Text(t)) => FileForm::parse(t).is_some(),
        (Fragment::Block { .. }, FragValue::Text(t)) => {
            t.len() <= MAX_TEXT_BYTES && !t.contains('\0')
        }
        (_, FragValue::Text(t)) => {
            t.len() <= MAX_TEXT_BYTES && !t.chars().any(|c| c == '\n' || c == '\r' || c == '\0')
        }
    };
    if ok {
        Ok(())
    } else {
        Err(ApplyError::Invalid("değer parça türüne uymuyor"))
    }
}

impl fmt::Debug for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Change")
            .field("setting", &self.setting)
            .field("source", &self.source)
            .field("writes", &self.writes)
            .field("create_missing", &self.create_missing.len())
            .field("take_over", &self.take_over.len())
            .finish_non_exhaustive()
    }
}

/// İşlem durumu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxState {
    /// Yazım sürüyor (kilit altında). Kilit yokken görülürse çökme → `Interrupted`.
    Pending,
    Applied,
    /// Yazım başarısız oldu, uygulanmış parçalar geri alındı.
    RolledBack,
    /// Yazım sırasında süreç öldü; otomatik geri alınmaz, Ayarlar sunar (§11-4 A).
    Interrupted,
    /// Bu snapshot, verilen (yeni) geri yükleme işlemiyle geri alındı.
    Restored(SnapshotId),
}

impl TxState {
    pub fn name(&self) -> &'static str {
        match self {
            TxState::Pending => "pending",
            TxState::Applied => "applied",
            TxState::RolledBack => "rolled_back",
            TxState::Interrupted => "interrupted",
            TxState::Restored(_) => "restored",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_id_rules() {
        for ok in [
            "look.unified",
            "hypr.source",
            "migration.shell",
            "a",
            "x_1.y2",
        ] {
            assert!(SettingId::new(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            ".a",
            "a.",
            "a..b",
            "A",
            "a-b",
            "a/b",
            "a b",
            "ğ",
            &"a".repeat(65),
        ] {
            assert!(SettingId::new(bad).is_err(), "{bad}");
        }
        assert!(
            toml::from_str::<std::collections::BTreeMap<String, SettingId>>("x = \"../x\"")
                .is_err()
        );
    }

    #[test]
    fn snapshot_id_format_and_time() {
        // 2026-10-10 12:00:00 UTC
        let t = UNIX_EPOCH + std::time::Duration::from_secs(1_791_633_600);
        let id = SnapshotId::from_parts(t, 0x3fa1);
        assert_eq!(id.as_str(), "20261010T120000Z-3fa1");
        assert_eq!(id.unix_secs(), 1_791_633_600);
        assert!(SnapshotId::parse("20261010T120000Z-3fa1").is_ok());
        for bad in [
            "20261010T120000Z-3FA1",
            "../../etc/passwd",
            "20261010T120000Z-3fa",
            "20261010X120000Z-3fa1",
            "",
        ] {
            assert!(SnapshotId::parse(bad).is_err(), "{bad}");
        }
        // Artık yıl / epoch.
        assert_eq!(
            SnapshotId::from_parts(UNIX_EPOCH, 0).as_str(),
            "19700101T000000Z-0000"
        );
        let leap = UNIX_EPOCH + std::time::Duration::from_secs(951_782_400); // 2000-02-29
        assert!(
            SnapshotId::from_parts(leap, 1)
                .as_str()
                .starts_with("20000229T")
        );
        let a = SnapshotId::generate(t);
        let b = SnapshotId::generate(t);
        assert!(a.as_str().starts_with("20261010T120000Z-"));
        assert!(SnapshotId::parse(a.as_str()).is_ok() && SnapshotId::parse(b.as_str()).is_ok());
    }

    #[test]
    fn fragment_validation() {
        let f = PathBuf::from("/x/settings.ini");
        let ini = |s: &str, k: &str| Fragment::IniKey {
            file: f.clone(),
            section: s.into(),
            key: k.into(),
        };
        assert!(ini("Settings", "gtk-theme-name").validate().is_ok());
        assert!(ini("", "a").validate().is_ok());
        assert!(ini("Colors:Window", "BackgroundNormal").validate().is_ok());
        assert!(ini("S", "a=b").validate().is_err());
        assert!(ini("S", "a\nb").validate().is_err());
        assert!(ini("S\n", "a").validate().is_err());
        assert!(ini("S", " a").validate().is_err());
        assert!(ini("S", "k[$e]").validate().is_err());
        assert!(ini("Desktop Entry", "Name[de]").validate().is_ok());
        assert!(
            Fragment::IniKey {
                file: "rel".into(),
                section: "".into(),
                key: "a".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Fragment::Dconf {
                key: "/org/gnome/desktop/interface/gtk-theme".into()
            }
            .validate()
            .is_ok()
        );
        for bad in ["org/x", "/a/", "/a//b", "/a b", "/a;rm"] {
            assert!(
                Fragment::Dconf { key: bad.into() }.validate().is_err(),
                "{bad}"
            );
        }
        assert!(
            Fragment::UnitState {
                unit: "nuxut-shell.service".into()
            }
            .validate()
            .is_ok()
        );
        for bad in ["--now.service", "x", "a b.service", "x.service;y"] {
            assert!(
                Fragment::UnitState { unit: bad.into() }.validate().is_err(),
                "{bad}"
            );
        }
        assert!(
            Fragment::Block {
                file: f.clone(),
                id: "look".into()
            }
            .validate()
            .is_ok()
        );
        assert!(
            Fragment::Block {
                file: f.clone(),
                id: "Look x".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Fragment::Line {
                file: f.clone(),
                marker: "a\nb".into(),
                at: Position::End
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn change_validation() {
        let s = SettingId::new("look.gtk").unwrap();
        let f = PathBuf::from("/x/settings.ini");
        let frag = Fragment::IniKey {
            file: f.clone(),
            section: "Settings".into(),
            key: "a".into(),
        };
        assert!(
            Change::new(s.clone(), "x", Source::Settings)
                .validate()
                .is_err()
        );
        let c = Change::new(s.clone(), "x", Source::Settings)
            .write(frag.clone(), FragValue::Text("1".into()));
        assert!(c.validate().is_ok());
        assert!(
            c.clone()
                .write(frag.clone(), FragValue::Absent)
                .validate()
                .is_err()
        );
        let nl = Change::new(s.clone(), "x", Source::Settings)
            .write(frag.clone(), FragValue::Text("1\nexec=x".into()));
        assert!(nl.validate().is_err());
        let file_val = Change::new(s.clone(), "x", Source::Settings).write(
            frag,
            FragValue::File {
                bytes: vec![],
                mode: 0,
            },
        );
        assert!(file_val.validate().is_err());
        let unit = Change::new(s.clone(), "x", Source::Settings).write(
            Fragment::UnitState {
                unit: "a.service".into(),
            },
            FragValue::Text("masked".into()),
        );
        assert!(unit.validate().is_err());
        let line = Change::new(s, "x", Source::Settings).write(
            Fragment::Line {
                file: f,
                marker: "a".into(),
                at: Position::End,
            },
            FragValue::Text("b".into()),
        );
        assert!(line.validate().is_err());
    }

    #[test]
    fn debug_redacts() {
        let f = Fragment::IniKey {
            file: "/home/u/gizli.ini".into(),
            section: "S".into(),
            key: "k".into(),
        };
        let v = FragValue::Text("parola".into());
        let c =
            Change::new(SettingId::new("a").unwrap(), "x", Source::Cli).write(f.clone(), v.clone());
        let s = format!("{f:?} {v:?} {c:?}");
        assert!(!s.contains("gizli") && !s.contains("parola"), "{s}");
    }

    #[test]
    fn fragment_toml_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            f: Vec<Fragment>,
        }
        let w = W {
            f: vec![
                Fragment::TomlKey {
                    file: "/a.toml".into(),
                    key: KeyPath::dotted("a.b").unwrap(),
                },
                Fragment::Line {
                    file: "/b".into(),
                    marker: "x".into(),
                    at: Position::Start,
                },
                Fragment::Dconf { key: "/a/b".into() },
            ],
        };
        let s = toml::to_string(&w).unwrap();
        assert_eq!(toml::from_str::<W>(&s).unwrap(), w);
    }
}
