//! İşlem akışı (ADR-0001 §6–§8): plan, uygula, geri yükle, listele.
//!
//! ```text
//! apply(change):
//!   doğrula → kapı (her parça türü + depo) ─Err→ Skipped
//!   depo aç ─Err→ SnapshotUnavailable;  kilit (≤ 5 s) ─→ Busy;  kalmış Pending → Interrupted
//!   her parça oku (dosya başına tek okuma) → before;  before == after olanlar düşer → hepsi: Noop
//!   2 s içinde aynı ayarın son işlemi (aynı kaynak, araya kimse girmemiş) → onunla birleş
//!   blob'lar + manifest (Pending, fsync)  ← işlem noktası
//!   tüm dosya içerikleri önce hesaplanır (çözümleme hatası → hiçbir şey yazılmaz), sonra sırayla
//!   yazılır (dosya başına tek yazım); hata → yazılanlar eski baytlarına döner → RolledBack, Err
//!   (geri dönüş de başarısızsa → Interrupted, Partial)
//!   yazılan değerler yeniden okunur (after = gerçekte diskte/dconf'ta olan) → Applied → günlük → budama
//! ```

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::backend::{Backend, SystemBackend};
use crate::edit::{self, EditError};
use crate::env::{Env, Skip, TargetKind};
use crate::error::ApplyError;
use crate::model::{
    Change, FileForm, FragValue, Fragment, SettingId, SnapshotId, Source, TxState,
};
use crate::store::{Entry, Manifest, Store, value_hash};
use crate::takeover::{
    ExternalReason, PathId, external_reason, form_of, keep_inode, nofollow_real, replace_with_file,
    replace_with_link, replace_with_symlink,
};
use crate::write::{
    Current, DIR_MODE, Expect, read_optional_bytes, remove_file, resolve, write_impl,
};

/// Ayar başına saklanan (sabitlenmemiş) işlem sayısı.
pub const PER_SETTING_LIMIT: usize = 20;
/// Depodaki toplam işlem sınırı.
pub const TOTAL_LIMIT: usize = 500;
/// Deponun toplam boyut sınırı.
pub const TOTAL_BYTES_LIMIT: u64 = 64 * 1024 * 1024;
/// Aynı ayarın ardışık uygulamaları bu süre içinde tek snapshot'ta birleşir.
pub const COALESCE_MS: u64 = 2000;
/// Kilit bekleme süresi.
pub const LOCK_WAIT: Duration = Duration::from_secs(5);
/// `list`/`show` çakışma sayımında dış hedef (dconf/systemctl alt süreci) okumalarının toplam
/// bütçesi. Aşılınca kalan dış parçalar "bilinmiyor" sayılır. En kötü süre: bütçe + bir alt
/// sürecin zaman aşımı.
pub const PEEK_EXT_BUDGET: Duration = Duration::from_secs(2);

/// Bir `list`/`show` çağrısı boyunca paylaşılan okuma önbelleği.
struct PeekCache {
    files: HashMap<PathBuf, Option<Option<Current>>>,
    ext: HashMap<Fragment, Option<FragValue>>,
    deadline: Instant,
}

impl PeekCache {
    fn new() -> Self {
        Self {
            files: HashMap::new(),
            ext: HashMap::new(),
            deadline: Instant::now() + PEEK_EXT_BUDGET,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Dışa dönük sonuç tipleri (IPC JSON; değer içermez)

/// Bir parçanın özeti: tür, hedef (`~` ile kısaltılmış yol ya da dconf anahtarı / birim), varsa
/// anahtar ve önce/sonra var mı.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct FragmentSummary {
    pub kind: &'static str,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub before: &'static str,
    pub after: &'static str,
}

impl fmt::Debug for FragmentSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FragmentSummary({} {}→{})",
            self.kind, self.before, self.after
        )
    }
}

/// `apply` sonucu (ADR §10.3 adları).
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum Outcome {
    Applied {
        id: SnapshotId,
    },
    /// Her parça zaten istenen değerde; snapshot yok.
    Noop,
    /// Yazım başarısız oldu ve bazı parçalar geri döndürülemedi; işlem `Interrupted`.
    Partial {
        id: SnapshotId,
        failed: Vec<FragmentSummary>,
    },
    Skipped {
        reason: Skip,
    },
    /// Hedef başka bir aracın yönettiği dosya; `reason` nedenini verir (devralınabilir olanlar:
    /// [`ExternalReason::can_take_over`], [`Change::take_over`]).
    ExternallyManaged {
        target: String,
        reason: ExternalReason,
    },
    /// Hedef okunduktan sonra değişti; yeniden dene.
    Changed,
    Busy,
    TooLarge {
        target: String,
    },
    Unavailable {
        target: String,
    },
    SnapshotUnavailable,
    Error {
        kind: &'static str,
    },
}

impl Outcome {
    pub fn name(&self) -> &'static str {
        match self {
            Outcome::Applied { .. } => "applied",
            Outcome::Noop => "noop",
            Outcome::Partial { .. } => "partial",
            Outcome::Skipped { .. } => "skipped",
            Outcome::ExternallyManaged { .. } => "externally_managed",
            Outcome::Changed => "changed",
            Outcome::Busy => "busy",
            Outcome::TooLarge { .. } => "too_large",
            Outcome::Unavailable { .. } => "unavailable",
            Outcome::SnapshotUnavailable => "snapshot_unavailable",
            Outcome::Error { .. } => "error",
        }
    }
}

/// Yol göstermez.
impl fmt::Debug for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outcome::Applied { id } | Outcome::Partial { id, .. } => {
                write!(f, "{}({id})", self.name())
            }
            Outcome::Skipped { reason } => write!(f, "skipped({})", reason.name()),
            Outcome::Error { kind } => write!(f, "error({kind})"),
            _ => f.write_str(self.name()),
        }
    }
}

/// Geri yükleme kipi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    /// Sonradan değişmiş parçalar atlanır (`Conflict`/`Partial`).
    Safe,
    /// Sonradan değişmiş parçaların da üzerine yazılır.
    Force,
}

/// `restore` sonucu.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum RestoreOutcome {
    /// `id` = geri yüklemenin kendi snapshot'ı (`None`: her parça zaten öncesindeydi).
    Restored {
        id: Option<SnapshotId>,
    },
    /// Bazı parçalar sonradan değişmişti ve atlandı; diğerleri geri yüklendi.
    Partial {
        id: SnapshotId,
        conflicts: Vec<FragmentSummary>,
    },
    /// Geri yüklenecek her parça sonradan değişmiş; hiçbir şey yazılmadı.
    Conflict {
        conflicts: Vec<FragmentSummary>,
    },
    NotFound,
    Busy,
    Skipped {
        reason: Skip,
    },
    Error {
        kind: &'static str,
    },
}

impl RestoreOutcome {
    pub fn name(&self) -> &'static str {
        match self {
            RestoreOutcome::Restored { .. } => "restored",
            RestoreOutcome::Partial { .. } => "partial",
            RestoreOutcome::Conflict { .. } => "conflict",
            RestoreOutcome::NotFound => "not_found",
            RestoreOutcome::Busy => "busy",
            RestoreOutcome::Skipped { .. } => "skipped",
            RestoreOutcome::Error { .. } => "error",
        }
    }
}

impl fmt::Debug for RestoreOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RestoreOutcome::Error { kind } => write!(f, "error({kind})"),
            RestoreOutcome::Conflict { conflicts } => write!(f, "conflict({})", conflicts.len()),
            RestoreOutcome::Partial { id, conflicts } => {
                write!(f, "partial({id}, {})", conflicts.len())
            }
            RestoreOutcome::Restored { id } => write!(f, "restored({id:?})"),
            _ => f.write_str(self.name()),
        }
    }
}

/// Geri yüklemede bir parçaya geri yazılan değer: parça adresi ve `Text` değeri (`None` = parça
/// kaldırıldı / ayarlanmamış). Yönetilen dosyanın içeriği (`FragValue::File`) listeye girmez.
/// `Debug` değer ve yol göstermez; `Serialize` yok (IPC JSON'una ve loglara girmez).
#[derive(Clone, PartialEq, Eq)]
pub struct RestoredValue {
    pub fragment: Fragment,
    pub value: Option<String>,
}

impl fmt::Debug for RestoredValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RestoredValue({} {})",
            self.fragment.kind_name(),
            if self.value.is_some() {
                "set"
            } else {
                "absent"
            }
        )
    }
}

/// [`Applier::restore_with_values`]'ın değer listesi: hangi ayar, hangi parçalara ne yazıldı.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RestoredValues {
    /// Geri yüklenen işlemin ayarı (hiçbir şey yazılmadıysa `None`).
    pub setting: Option<SettingId>,
    pub values: Vec<RestoredValue>,
}

impl RestoredValues {
    /// Parçanın geri yüklenen değeri: `None` = bu geri yüklemede yazılmadı; `Some(None)` =
    /// kaldırıldı; `Some(Some(v))` = `v` yazıldı.
    pub fn get(&self, fragment: &Fragment) -> Option<Option<&str>> {
        self.values
            .iter()
            .find(|v| v.fragment == *fragment)
            .map(|v| v.value.as_deref())
    }
}

impl fmt::Debug for RestoredValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RestoredValues")
            .field("setting", &self.setting)
            .field("values", &self.values)
            .finish()
    }
}

/// `list` süzgeci.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub setting: Option<SettingId>,
    /// 1..=200 (kırpılır).
    pub limit: usize,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            setting: None,
            limit: 50,
        }
    }
}

/// `list` satırı (ADR §9.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotSummary {
    pub id: SnapshotId,
    /// ISO 8601 UTC.
    pub at: String,
    /// Oluşturulma anı (UNIX epoch ms; `at` saniye çözünürlüklü, sıralama ve göreli zaman için).
    pub created_ms: u64,
    pub setting: SettingId,
    pub label: String,
    pub source: Source,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_by: Option<SnapshotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restores: Option<SnapshotId>,
    pub pinned: bool,
    pub fragments: usize,
    /// Sonradan değişmiş (geri yüklemede çakışacak) parça sayısı; yalnızca okunabilen hedefler.
    pub conflicts: usize,
}

/// `show` yanıtı: değer yok, yalnızca tür, hedef, önce/sonra var mı.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SnapshotDetail {
    pub id: SnapshotId,
    pub at: String,
    /// Oluşturulma anı (UNIX epoch ms; `at` saniye çözünürlüklü, sıralama ve göreli zaman için).
    pub created_ms: u64,
    pub setting: SettingId,
    pub label: String,
    pub source: Source,
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored_by: Option<SnapshotId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restores: Option<SnapshotId>,
    pub pinned: bool,
    pub conflicts: usize,
    pub fragments: Vec<FragmentSummary>,
}

/// `plan` sonucu: yazmadan, değişecek parçalar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanEntry {
    pub fragment: FragmentSummary,
    /// Şu anki değer: `set` | `absent`.
    pub current: &'static str,
    /// Değer değişecek mi?
    pub changes: bool,
    /// Nuxut'a ait olmayan bir değer ezilecek mi (Nuxut'un hiç yazmadığı, farklı bir değer)?
    pub foreign: bool,
}

// ---------------------------------------------------------------------------------------------
// İç tipler

/// Okunmuş dosya (gerçek yolu, kilit altında okunmuş hâli).
#[derive(Debug)]
struct FileSlot {
    real: PathBuf,
    current: Option<Current>,
    /// Biçimi izlenen (devralınan / devralması geri yüklenen) yol: `real` son bileşeni izlenmemiş
    /// yoldur, yazım yolun kendisini değiştirir. İçerik (`current`) symlink'in hedefinden okunur.
    /// Değer: okunduğu andaki `lstat` kimliği (ön koşul).
    nofollow: Option<PathId>,
    /// Okunduğu andaki biçim (geri dönüş için; `None` = yol yoktu).
    form: Option<FileForm>,
}

#[derive(Debug)]
struct Item {
    fragment: Fragment,
    before: FragValue,
    after: FragValue,
    slot: Option<usize>,
}

/// Dosya başına yazım kuralları (çağıran hesaplar; `render` uygular).
#[derive(Debug, Default)]
struct FileRules {
    /// Yazım sonunda boş kalırsa dosya silinir (Nuxut'un yarattığı dosyanın geri yüklenmesi).
    delete_if_empty: bool,
    /// Eksik dosya `Line`/`Block` parçası için yaratılabilir ([`Change::create_missing`]).
    create: bool,
    /// Boş kalırsa başlığı silinecek, Nuxut'un yarattığı INI bölümleri.
    drop_sections: Vec<String>,
    /// Sabit bağ devralınırken özgün inode'un tutulacağı bağ (bu işlemin snapshot dizininde).
    keep_to: Option<PathBuf>,
    /// Sabit bağ geri yüklenirken inode'u tutan bağ (geri yüklenen işlemin snapshot dizininde).
    keep_from: Option<PathBuf>,
}

impl FileRules {
    const NONE: FileRules = FileRules {
        delete_if_empty: false,
        create: false,
        drop_sections: Vec::new(),
        keep_to: None,
        keep_from: None,
    };
}

#[derive(Debug)]
enum Action {
    Write { bytes: Vec<u8>, mode: Option<u32> },
    Remove,
    Nothing,
    /// Biçim değişimi: yolun kendisi (izlenmeden) değiştirilir.
    Form(FormAction),
}

#[derive(Debug)]
enum FormAction {
    /// Düz dosya; `keep`: önce özgün inode bu bağda tutulur (sabit bağın devralınması).
    File {
        bytes: Vec<u8>,
        mode: u32,
        keep: Option<PathBuf>,
    },
    Symlink(String),
    /// Tutulan inode'a yeniden bağ.
    Link(PathBuf),
}

#[derive(Debug)]
enum Step {
    File(usize),
    Ext(usize),
}

enum Commit {
    Applied(SnapshotId),
    Noop,
    Partial(SnapshotId, Vec<FragmentSummary>),
}

type Clock = Box<dyn Fn() -> SystemTime + Send + Sync>;
type ReadState = (Vec<FileSlot>, Vec<Option<usize>>, Vec<FragValue>);

/// Uygulayıcı. Ucuz; her çağrı depoyu ve kilidi kendisi açar (çok süreçli güvenli).
pub struct Applier {
    env: Env,
    source: Source,
    ext: Box<dyn Backend>,
    clock: Clock,
    lock_wait: Duration,
    /// (ayar başına, toplam işlem, toplam bayt)
    limits: (usize, usize, u64),
}

impl fmt::Debug for Applier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Applier")
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Applier {
    /// IO yapmaz; depo hataları işlemlerde `SnapshotUnavailable` olarak döner. dconf/systemd için
    /// [`SystemBackend`] (test örneğinde onlara hiç dokunulmaz).
    pub fn open(env: Env) -> Self {
        // Test örneği dconf/systemd'ye hiç ulaşmaz (kapı zaten reddeder; program yolu bile yok).
        let ext: Box<dyn Backend> = if env.test_instance {
            Box::new(crate::backend::NoBackend)
        } else {
            Box::new(SystemBackend::new(&env))
        };
        Self {
            ext,
            env,
            source: Source::Settings,
            clock: Box::new(SystemTime::now),
            lock_wait: LOCK_WAIT,
            limits: (PER_SETTING_LIMIT, TOTAL_LIMIT, TOTAL_BYTES_LIMIT),
        }
    }

    /// Geri yükleme kayıtlarının kaynağı (varsayılan `Settings`).
    #[must_use]
    pub fn with_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    #[cfg(test)]
    pub(crate) fn set_clock(&mut self, clock: Clock) {
        self.clock = clock;
    }

    #[cfg(test)]
    pub(crate) fn set_limits(&mut self, per_setting: usize, total: usize, bytes: u64) {
        self.limits = (per_setting, total, bytes);
    }

    #[cfg(test)]
    pub(crate) fn set_lock_wait(&mut self, d: Duration) {
        self.lock_wait = d;
    }

    /// dconf/systemd arka ucu (varsayılan [`SystemBackend`]; testlerde sahte, ya da
    /// [`crate::NoBackend`]).
    #[must_use]
    pub fn with_backend(mut self, backend: Box<dyn Backend>) -> Self {
        self.ext = backend;
        self
    }

    fn now_ms(&self) -> u64 {
        (self.clock)()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    // -----------------------------------------------------------------------------------------
    // Kapı, okuma, özet

    fn guard_all<'a>(
        &self,
        frags: impl IntoIterator<Item = &'a Fragment>,
    ) -> Result<(), ApplyError> {
        self.env.guard(TargetKind::File)?;
        for f in frags {
            self.env.guard(kind_of(f))?;
        }
        Ok(())
    }

    /// Parçaların şu anki değerleri; dosya başına tek okuma (gerçek yola göre).
    /// Dönüş: (dosyalar, her parçanın dosya indeksi, her parçanın değeri).
    fn read_state(&self, frags: &[&Fragment]) -> Result<ReadState, ApplyError> {
        let mut slots: Vec<FileSlot> = Vec::new();
        let mut slot_of = Vec::with_capacity(frags.len());
        let mut values = Vec::with_capacity(frags.len());
        // Biçimi izlenen dosyalar: bu dosyaların parçaları yolun kendisine (izlemeden) yazılır.
        let forms: Vec<&Path> = frags
            .iter()
            .filter_map(|f| match f {
                Fragment::FileForm { file } => Some(file.as_path()),
                _ => None,
            })
            .collect();
        for f in frags {
            match f.file() {
                Some(path) => {
                    let nofollow = forms.contains(&path);
                    let real = if nofollow {
                        nofollow_real(path)?
                    } else {
                        resolve(path)?
                    };
                    self.env.check_path(&real)?;
                    let idx = match slots.iter().position(|s| s.real == real) {
                        Some(i) => i,
                        None => {
                            let current = read_optional_bytes(&real)?;
                            slots.push(FileSlot {
                                real,
                                current,
                                nofollow: None,
                                form: None,
                            });
                            slots.len() - 1
                        }
                    };
                    if nofollow && slots[idx].nofollow.is_none() {
                        let slot = &mut slots[idx];
                        slot.nofollow = Some(PathId::of(&slot.real)?);
                    }
                    if let Fragment::FileForm { .. } = f {
                        let v = form_of(&slots[idx].real)?;
                        slots[idx].form = match &v {
                            FragValue::Text(t) => FileForm::parse(t),
                            _ => None,
                        };
                        values.push(v);
                        slot_of.push(Some(idx));
                        continue;
                    }
                    values.push(frag_value(
                        f,
                        slots[idx].current.as_ref().map(|c| c.bytes.as_slice()),
                        path,
                        slots[idx].current.as_ref().map(|c| c.mode),
                    )?);
                    slot_of.push(Some(idx));
                }
                None => {
                    values.push(self.ext.read(f)?);
                    slot_of.push(None);
                }
            }
        }
        Ok((slots, slot_of, values))
    }

    /// En iyi çaba: okunamayan ya da kapının izin vermediği hedef `None`. Önbellek bir
    /// `list`/`show` çağrısı boyunca paylaşılır (aynı dosya bir kez okunur, aynı dış hedef bir kez
    /// sorulur); dış hedefler (alt süreç) toplam [`PEEK_EXT_BUDGET`] ile sınırlı — bütçe bitince
    /// "bilinmiyor" (`None`, çakışma sayılmaz).
    fn peek(&self, cache: &mut PeekCache, frags: &[&Fragment]) -> Vec<Option<FragValue>> {
        frags
            .iter()
            .map(|f| match f.file() {
                Some(path) if matches!(f, Fragment::FileForm { .. }) => {
                    let real = nofollow_real(path).ok()?;
                    self.env.check_path(&real).ok()?;
                    form_of(&real).ok()
                }
                Some(path) => {
                    let cur = cache.files.entry(path.to_path_buf()).or_insert_with(|| {
                        let real = resolve(path).ok()?;
                        self.env.check_path(&real).ok()?;
                        read_optional_bytes(&real).ok()
                    });
                    let cur = cur.as_ref()?;
                    frag_value(
                        f,
                        cur.as_ref().map(|c| c.bytes.as_slice()),
                        path,
                        cur.as_ref().map(|c| c.mode),
                    )
                    .ok()
                }
                None => {
                    if let Some(v) = cache.ext.get(*f) {
                        return v.clone();
                    }
                    if Instant::now() >= cache.deadline {
                        return None;
                    }
                    let v = self
                        .env
                        .guard(kind_of(f))
                        .ok()
                        .and_then(|()| self.ext.read(f).ok());
                    cache.ext.insert((*f).clone(), v.clone());
                    v
                }
            })
            .collect()
    }

    fn summarize(
        &self,
        f: &Fragment,
        before: &'static str,
        after: &'static str,
    ) -> FragmentSummary {
        let target = match f {
            Fragment::Dconf { key } => key.clone(),
            Fragment::UnitState { unit } => unit.clone(),
            other => other
                .file()
                .map(|p| self.env.display_path(p))
                .unwrap_or_default(),
        };
        let key = match f {
            Fragment::TomlKey { key, .. } => Some(key.to_string()),
            Fragment::IniKey { section, key, .. } => Some(format!("[{section}] {key}")),
            Fragment::Block { id, .. } => Some(id.clone()),
            _ => None,
        };
        FragmentSummary {
            kind: f.kind_name(),
            target,
            key,
            before,
            after,
        }
    }

    fn entry_summary(&self, e: &Entry) -> FragmentSummary {
        self.summarize(&e.fragment, e.before.presence(), e.after.presence())
    }

    fn outcome_err(&self, e: ApplyError) -> Outcome {
        match e {
            ApplyError::Skipped(reason) => Outcome::Skipped { reason },
            ApplyError::ExternallyManaged(p) => Outcome::ExternallyManaged {
                target: self.env.display_path(&p),
                reason: external_reason(&p).unwrap_or(ExternalReason::Unknown),
            },
            ApplyError::Changed => Outcome::Changed,
            ApplyError::Busy => Outcome::Busy,
            ApplyError::TooLarge(p) => Outcome::TooLarge {
                target: self.env.display_path(&p),
            },
            ApplyError::Unavailable(target) => Outcome::Unavailable { target },
            ApplyError::SnapshotUnavailable => Outcome::SnapshotUnavailable,
            other => Outcome::Error {
                kind: other.kind_name(),
            },
        }
    }

    // -----------------------------------------------------------------------------------------
    // Plan

    /// Yazmadan: her parçanın şu anki durumu, değişip değişmeyeceği ve Nuxut'a ait olmayan bir
    /// değeri ezip ezmeyeceği.
    pub fn plan(&self, change: &Change) -> Result<Plan, ApplyError> {
        change.validate()?;
        self.guard_all(change.writes.iter().map(|(f, _)| f))?;
        let forms = takeover_forms(change)?;
        let writes: Vec<&(Fragment, FragValue)> = change.writes.iter().chain(&forms).collect();
        let frags: Vec<&Fragment> = writes.iter().map(|(f, _)| f).collect();
        let (_, _, current) = self.read_state(&frags)?;
        let known = Store::open(&self.env)
            .ok()
            .and_then(|s| s.load_all().ok())
            .unwrap_or_default();
        let mut entries = Vec::with_capacity(frags.len());
        for ((f, desired), cur) in writes.into_iter().zip(current) {
            let after = normalize(f, desired.clone())?;
            let changes = !same(f, &cur, &after);
            let foreign = changes
                && !cur.is_absent()
                && match (f, &cur) {
                    (Fragment::ManagedFile { .. }, FragValue::File { bytes, .. }) => {
                        !edit::managed::is_managed(bytes)
                    }
                    _ => {
                        let h = value_hash(&cur);
                        !known.iter().any(|m| {
                            m.fragments
                                .iter()
                                .any(|e| e.fragment == *f && e.after.hash() == h.as_deref())
                        })
                    }
                };
            entries.push(PlanEntry {
                fragment: self.summarize(f, cur.presence(), after.presence()),
                current: cur.presence(),
                changes,
                foreign,
            });
        }
        Ok(Plan { entries })
    }

    // -----------------------------------------------------------------------------------------
    // Uygula

    pub fn apply(&self, change: Change) -> Outcome {
        let setting = change.setting.clone();
        let out = self
            .apply_inner(change, false)
            .unwrap_or_else(|e| self.outcome_err(e));
        tracing::info!(setting = %setting, result = out.name(), "nuxut-apply: uygula");
        out
    }

    /// Türetilmiş yeniden üretim (ADR-0001 §6; ör. palet değişince `kdeglobals` renkleri, GTK
    /// CSS'i): yeni snapshot almadan aynı işlem katmanından geçer (kapı, kilit, doğrulama, tek
    /// yazım, hata → geri dönüş). Ayarın son işlemi bu kaynağın uygulanmış (geri yükleme olmayan)
    /// işlemiyse ve örtüşen her parçanın şu anki değeri o işlemin `after`'ıysa (araya kimse
    /// girmemiş) yazım o işleme **süre penceresi olmadan** birleşir: ilk `before` korunur, `after`
    /// güncellenir — "önceki hale dön" yine Nuxut öncesine döner, üç yollu denetim çakışma görmez.
    /// Aksi hâlde (ilk dokunuş, dış değişiklik, son işlem bir geri yükleme ya da başka kaynak, tek
    /// işlem sınırı aşılıyor) normal [`Applier::apply`] gibi yeni snapshot alınır — kullanıcının
    /// elle yaptığı değişiklik geri alınabilir kalır. Sonuç `apply` ile aynı türdür
    /// (`Applied { id }`: birleşmişse önceki işlemin kimliği).
    ///
    /// Neden `write_file` yetmez: snapshot'ı alınmış bir parça (`kdeglobals` anahtarı, yönetilen
    /// dosya) snapshot dışından yeniden yazılırsa kaydın `after`'ı eskir; geri yükleme onu
    /// "kullanıcı değiştirmiş" sayıp güvenli kipte atlar (`conflict`). `write_file` yalnızca hiçbir
    /// işlemin izlemediği, tamamı Nuxut'a ait dosyalar için uygundur.
    pub fn apply_derived(&self, change: Change) -> Outcome {
        let setting = change.setting.clone();
        let out = self
            .apply_inner(change, true)
            .unwrap_or_else(|e| self.outcome_err(e));
        tracing::info!(setting = %setting, result = out.name(), "nuxut-apply: türetilmiş yazım");
        out
    }

    fn apply_inner(&self, change: Change, derived: bool) -> Result<Outcome, ApplyError> {
        change.validate()?;
        self.guard_all(change.writes.iter().map(|(f, _)| f))?;
        let store = Store::open(&self.env)?;
        let _lock = store.lock(self.lock_wait)?;
        self.recover(&store);

        // Devralınacak dosyalar (kilit altında karar): biçim parçası işleme girer.
        let mut change = change;
        let forms = takeover_forms(&change)?;
        change.writes.extend(forms);
        if !crate::model::fragments_fit(change.writes.iter().map(|(f, _)| f)) {
            return Err(ApplyError::Invalid(
                "parça sayısı 1..=256, adres toplamı ≤ 64 KiB olmalı",
            ));
        }
        let frags: Vec<&Fragment> = change.writes.iter().map(|(f, _)| f).collect();
        let (slots, slot_of, before) = self.read_state(&frags)?;
        // Yaratma izni gerçek yola göre (aynı dosyaya iki yoldan ulaşılsa da).
        let mut rules: Vec<FileRules> = (0..slots.len()).map(|_| FileRules::default()).collect();
        for ((f, _), slot) in change.writes.iter().zip(&slot_of) {
            if let (Some(s), Some(p)) = (slot, f.file())
                && matches!(f, Fragment::Line { .. } | Fragment::Block { .. })
                && change.may_create(p)
            {
                rules[*s].create = true;
            }
        }
        // Devralınan kırık symlink: içerik dosyası yok, devralma onu yaratır.
        for (s, slot) in slots.iter().enumerate() {
            if slot.nofollow.is_some() && slot.current.is_none() {
                rules[s].create = true;
            }
        }
        let mut items = Vec::new();
        for (((fragment, desired), before), slot) in
            change.writes.into_iter().zip(before).zip(slot_of)
        {
            let after = normalize(&fragment, desired)?;
            if !same(&fragment, &before, &after) {
                items.push(Item {
                    fragment,
                    before,
                    after,
                    slot,
                });
            }
        }
        if items.is_empty() {
            return Ok(Outcome::Noop);
        }

        let now_ms = self.now_ms();
        let all = store.load_all()?;
        set_drop_sections(&mut rules, &slots, &items, &all);
        let window = if derived { None } else { Some(COALESCE_MS) };
        let base = coalesce_target(&all, &change.setting, change.source, now_ms, window, &items);
        if derived && base.is_none() {
            tracing::debug!(setting = %change.setting, "nuxut-apply: türetilmiş yazım yeni snapshot alıyor");
        }
        let m = match &base {
            Some(b) => {
                let mut m = b.clone();
                m.label = change.label;
                m
            }
            None => Manifest::new(
                store.fresh_id((self.clock)()),
                now_ms,
                change.setting,
                change.label,
                change.source,
            ),
        };
        set_keepers(&mut rules, &slots, &items, &store, &m.id, None);
        Ok(
            match self.commit(&store, &all, base, m, items, &slots, &rules, now_ms)? {
                Commit::Applied(id) => {
                    self.prune(&store, &id);
                    Outcome::Applied { id }
                }
                Commit::Noop => Outcome::Noop,
                Commit::Partial(id, failed) => Outcome::Partial { id, failed },
            },
        )
    }

    /// Manifest'i (Pending) yazar, parçaları uygular, sonucu kaydeder. Kilit altında çağrılır.
    /// Budama çağıranın işidir (geri yüklemede eski kayıt kaydedildikten sonra).
    #[allow(clippy::too_many_arguments)]
    fn commit(
        &self,
        store: &Store,
        all: &[Manifest],
        base: Option<Manifest>,
        mut m: Manifest,
        items: Vec<Item>,
        slots: &[FileSlot],
        rules: &[FileRules],
        now_ms: u64,
    ) -> Result<Commit, ApplyError> {
        // Önce: tüm dosya içerikleri ve yazılabilirlik — başarısız olacağı belli işlem için
        // snapshot hiç yazılmaz (depo büyümez).
        let (steps, actions) = self.prepare(slots, &items, rules)?;

        m.updated_ms = now_ms;
        m.state = TxState::Pending;
        m.pinned |= items.iter().any(|it| {
            !all.iter().any(|o| {
                o.state != TxState::RolledBack
                    && o.fragments.iter().any(|e| e.fragment == it.fragment)
            })
        });
        let mut next = m.next_blob();
        // Var olan dosyanın yaratıcı bilgisi dosya başına bir kez (yalnızca yeni kayıt açılan
        // parçalar için gerekir; birleştirmede ilk kaydın bilgisi korunur).
        let mut inherited: HashMap<usize, bool> = HashMap::new();
        for it in &items {
            let after = store.put(&m.id, &mut next, &it.after)?;
            match m.fragments.iter_mut().find(|e| e.fragment == it.fragment) {
                // Birleştirme: ilk `before` korunur; çökmeye karşı önceki `after` da tutulur.
                Some(e) => e.prior_after = Some(std::mem::replace(&mut e.after, after)),
                None => {
                    let before = store.put(&m.id, &mut next, &it.before)?;
                    let file_existed = it.slot.map(|s| slots[s].current.is_some());
                    let section_created = match (&it.fragment, &it.after, it.slot) {
                        (Fragment::IniKey { section, .. }, FragValue::Text(_), Some(s))
                            if !section.is_empty() =>
                        {
                            slots[s].current.as_ref().is_none_or(|c| {
                                std::str::from_utf8(&c.bytes)
                                    .is_ok_and(|t| !edit::ini::has_section(t, section))
                            })
                        }
                        _ => false,
                    };
                    let file_inherited = match (file_existed, it.slot) {
                        (Some(true), Some(s)) => *inherited
                            .entry(s)
                            .or_insert_with(|| inherits_nuxut_file(all, &slots[s])),
                        _ => false,
                    };
                    m.fragments.push(Entry {
                        fragment: it.fragment.clone(),
                        before,
                        after,
                        file_existed,
                        prior_after: None,
                        section_created,
                        file_inherited,
                    });
                }
            }
        }
        record_created_dirs(&mut m, all, slots, &items);
        if let Err(e) = store.save(&m) {
            match &base {
                Some(b) => store.gc_blobs(b),
                None => store.remove(&m.id),
            }
            return Err(e);
        }

        match self.execute(slots, &items, &steps, &actions) {
            Ok(observed) => {
                // after = gerçekte yazılan (dconf biçimlendirmesi vb.).
                for (it, obs) in items.iter().zip(observed) {
                    if let Some(e) = m.fragments.iter_mut().find(|e| e.fragment == it.fragment)
                        && e.after.hash() != value_hash(&obs).as_deref()
                        && let Ok(s) = store.put(&m.id, &mut next, &obs)
                    {
                        e.after = s;
                    }
                }
                // Birleştirilen işlem başa döndüyse (aç-kapa) iz bırakmaz.
                if m.fragments
                    .iter()
                    .all(|e| e.before.hash() == e.after.hash())
                {
                    store.remove(&m.id);
                    return Ok(Commit::Noop);
                }
                m.state = TxState::Applied;
                for e in &mut m.fragments {
                    e.prior_after = None;
                }
                // Yazım yapıldı: kayıt güncellenemese de sonuç Applied. Diskte Pending kalır →
                // sonraki kilitte Interrupted olarak görünür ve yine geri alınabilir (blob'lar
                // silinmez).
                match store.save(&m) {
                    Ok(()) => {
                        store.gc_blobs(&m);
                        store.journal(&m);
                    }
                    Err(_) => tracing::warn!(
                        id = %m.id,
                        "nuxut-apply: uygulandı ama manifest güncellenemedi (Interrupted görünecek)"
                    ),
                }
                Ok(Commit::Applied(m.id))
            }
            Err((err, failed)) if failed.is_empty() => {
                // Dışarıda hiçbir şey değişmedi: yeni işlemin izi silinir, birleştirilen işlem
                // önceki hâline döner.
                match base {
                    Some(b) => {
                        let _ = store.save(&b);
                        store.gc_blobs(&b);
                    }
                    None => {
                        m.state = TxState::RolledBack;
                        store.journal(&m);
                        store.remove(&m.id);
                    }
                }
                Err(err)
            }
            Err((_, failed)) => {
                m.state = TxState::Interrupted;
                let _ = store.save(&m);
                store.journal(&m);
                tracing::warn!(id = %m.id, failed = failed.len(), "nuxut-apply: geri dönüş tamamlanamadı");
                Ok(Commit::Partial(m.id, failed))
            }
        }
    }

    /// Adımlar (dosya başına bir, dış hedef başına bir; ilk görünüş sırasıyla), dosya içerikleri
    /// ve dosyaların yazılabilirlik ön denetimi. Hiçbir şey yazmaz.
    fn prepare(
        &self,
        slots: &[FileSlot],
        items: &[Item],
        rules: &[FileRules],
    ) -> Result<(Vec<Step>, HashMap<usize, Action>), ApplyError> {
        let mut steps: Vec<Step> = Vec::new();
        for (i, it) in items.iter().enumerate() {
            match it.slot {
                Some(s)
                    if !steps
                        .iter()
                        .any(|st| matches!(st, Step::File(x) if *x == s)) =>
                {
                    steps.push(Step::File(s))
                }
                Some(_) => {}
                None => steps.push(Step::Ext(i)),
            }
        }
        let mut actions: HashMap<usize, Action> = HashMap::new();
        for st in &steps {
            if let Step::File(s) = st {
                let edits: Vec<&Item> = items.iter().filter(|it| it.slot == Some(*s)).collect();
                let a = render(
                    &slots[*s],
                    &edits,
                    rules.get(*s).unwrap_or(&FileRules::NONE),
                )?;
                match a {
                    Action::Nothing => {}
                    Action::Form(_) => crate::takeover::precheck(&self.env, &slots[*s].real)?,
                    _ => crate::write::precheck(&self.env, &slots[*s].real)?,
                }
                actions.insert(*s, a);
            }
        }
        Ok((steps, actions))
    }

    /// Hazırlanmış adımları sırayla uygular. Hata → yazılanlar geri.
    /// Dönüş: her öğe için yazılan değer (yeniden okunmuş).
    fn execute(
        &self,
        slots: &[FileSlot],
        items: &[Item],
        steps: &[Step],
        actions: &HashMap<usize, Action>,
    ) -> Result<Vec<FragValue>, (ApplyError, Vec<FragmentSummary>)> {
        let mut done: Vec<&Step> = Vec::new();
        for st in steps {
            let r = match st {
                Step::File(s) => self.perform(&slots[*s], &actions[s]),
                Step::Ext(i) => self.ext.write(&items[*i].fragment, &items[*i].after),
            };
            if let Err(e) = r {
                let mut failed = Vec::new();
                for d in done.iter().rev() {
                    let ok = match d {
                        Step::File(s) => self.undo(&slots[*s], &actions[s]).is_ok(),
                        Step::Ext(i) => self
                            .ext
                            .write(&items[*i].fragment, &items[*i].before)
                            .is_ok(),
                    };
                    if !ok {
                        for it in items.iter().filter(|it| match d {
                            Step::File(s) => it.slot == Some(*s),
                            Step::Ext(i) => std::ptr::eq(*it, &items[*i]),
                        }) {
                            failed.push(self.summarize(
                                &it.fragment,
                                it.before.presence(),
                                it.after.presence(),
                            ));
                        }
                    }
                }
                return Err((e, failed));
            }
            done.push(st);
        }

        Ok(items
            .iter()
            .map(|it| match it.slot {
                // Biçimi değişen yol: gerçekte ne olduğu yeniden okunur (symlink geri kurulduysa
                // içerik onun hedefinden).
                Some(s) if slots[s].nofollow.is_some() => {
                    let real = &slots[s].real;
                    if let Fragment::FileForm { .. } = it.fragment {
                        return form_of(real).unwrap_or_else(|_| it.after.clone());
                    }
                    let now = read_optional_bytes(real).ok().flatten();
                    let path = it.fragment.file().unwrap_or(Path::new(""));
                    frag_value(
                        &it.fragment,
                        now.as_ref().map(|c| c.bytes.as_slice()),
                        path,
                        now.as_ref().map(|c| c.mode),
                    )
                    .unwrap_or_else(|_| it.after.clone())
                }
                Some(s) => {
                    let (bytes, mode) = match &actions[&s] {
                        Action::Write { bytes, mode } => (Some(bytes.as_slice()), *mode),
                        Action::Remove => (None, None),
                        Action::Nothing | Action::Form(_) => {
                            (slots[s].current.as_ref().map(|c| c.bytes.as_slice()), None)
                        }
                    };
                    let mode = mode
                        .or(slots[s].current.as_ref().map(|c| c.mode))
                        .or(Some(0o644));
                    let path = it.fragment.file().unwrap_or(Path::new(""));
                    frag_value(&it.fragment, bytes, path, mode).unwrap_or_else(|_| it.after.clone())
                }
                None => self
                    .ext
                    .read(&it.fragment)
                    .unwrap_or_else(|_| it.after.clone()),
            })
            .collect())
    }

    fn perform(&self, slot: &FileSlot, action: &Action) -> Result<(), ApplyError> {
        let expect = slot
            .current
            .as_ref()
            .map_or(Expect::Absent, |c| c.expect.clone());
        match action {
            Action::Nothing => Ok(()),
            Action::Write { bytes, mode } => {
                write_impl(&self.env, &slot.real, bytes, Some(&expect), *mode, DIR_MODE).map(|_| ())
            }
            Action::Remove => remove_file(&self.env, &slot.real, Some(&expect)).map(|_| ()),
            Action::Form(fa) => {
                let Some(id) = &slot.nofollow else {
                    return Err(ApplyError::Invalid("biçim yazımı izlenmeyen yol ister"));
                };
                // İçerik (symlink'in hedefi dahil) okunduğundan beri değişmedi mi?
                let now = read_optional_bytes(&slot.real)?;
                if now.as_ref().map(|c| &c.expect) != slot.current.as_ref().map(|c| &c.expect) {
                    return Err(ApplyError::Changed);
                }
                self.perform_form(slot, fa, id)
            }
        }
    }

    fn perform_form(&self, slot: &FileSlot, fa: &FormAction, id: &PathId) -> Result<(), ApplyError> {
        match fa {
            FormAction::File { bytes, mode, keep } => {
                if let Some(k) = keep {
                    keep_inode(&slot.real, k);
                }
                replace_with_file(&self.env, &slot.real, bytes, *mode, id)
            }
            FormAction::Symlink(t) => replace_with_symlink(&self.env, &slot.real, t, id),
            FormAction::Link(k) => replace_with_link(&self.env, &slot.real, k, id),
        }
    }

    /// Yazdığımızı geri alır (yalnızca dosya hâlâ bizim yazdığımız gibiyse).
    fn undo(&self, slot: &FileSlot, action: &Action) -> Result<(), ApplyError> {
        if let Action::Form(fa) = action {
            return self.undo_form(slot, fa);
        }
        let now = read_optional_bytes(&slot.real)?;
        let ours = match (action, &now) {
            (Action::Nothing, _) => return Ok(()),
            (Action::Write { bytes, .. }, Some(n)) => n.bytes == *bytes,
            (Action::Remove, None) => true,
            _ => false,
        };
        if !ours {
            return Err(ApplyError::Changed);
        }
        let expect = now.as_ref().map_or(Expect::Absent, |n| n.expect.clone());
        match &slot.current {
            Some(c) => write_impl(
                &self.env,
                &slot.real,
                &c.bytes,
                Some(&expect),
                Some(c.mode),
                DIR_MODE,
            )
            .map(|_| ()),
            None => remove_file(&self.env, &slot.real, Some(&expect)).map(|_| ()),
        }
    }

    /// Biçim yazımını geri alır: yol hâlâ bizim koyduğumuz gibiyse okunduğu andaki biçimine.
    fn undo_form(&self, slot: &FileSlot, fa: &FormAction) -> Result<(), ApplyError> {
        let ours = match fa {
            FormAction::File { bytes, .. } => std::fs::symlink_metadata(&slot.real)
                .is_ok_and(|m| m.is_file())
                && read_optional_bytes(&slot.real)?.is_some_and(|c| c.bytes == *bytes),
            FormAction::Symlink(t) => {
                std::fs::read_link(&slot.real).is_ok_and(|r| r.as_os_str() == t.as_str())
            }
            FormAction::Link(k) => {
                let a = std::fs::symlink_metadata(&slot.real).map(|m| crate::sys::identity(&m).0);
                let b = std::fs::symlink_metadata(k).map(|m| crate::sys::identity(&m).0);
                matches!((a, b), (Ok(x), Ok(y)) if x == y)
            }
        };
        if !ours {
            return Err(ApplyError::Changed);
        }
        let now = PathId::of(&slot.real)?;
        let (bytes, mode) = slot
            .current
            .as_ref()
            .map_or((&[][..], 0o644), |c| (c.bytes.as_slice(), c.mode));
        match &slot.form {
            Some(FileForm::Symlink(t)) => replace_with_symlink(&self.env, &slot.real, t, &now),
            Some(FileForm::HardLink) => match fa {
                FormAction::File { keep: Some(k), .. } if k.exists() => {
                    replace_with_link(&self.env, &slot.real, k, &now)
                }
                _ => replace_with_file(&self.env, &slot.real, bytes, mode, &now),
            },
            Some(FileForm::Regular | FileForm::ReadOnly(_) | FileForm::Foreign(_)) => {
                replace_with_file(&self.env, &slot.real, bytes, mode, &now)
            }
            None => match std::fs::remove_file(&slot.real) {
                Ok(()) => Ok(()),
                Err(e) => Err(e.into()),
            },
        }
    }

    // -----------------------------------------------------------------------------------------
    // Geri yükle

    /// Snapshot'ı geri yükler (üç yollu denetim). Geri yükleme de bir işlemdir (yeni snapshot;
    /// eski kayıt `Restored(yeni)` olur), yani o da geri alınabilir.
    pub fn restore(&self, id: &SnapshotId, mode: RestoreMode) -> RestoreOutcome {
        self.restore_with_values(id, mode).0
    }

    /// [`Applier::restore`] ve gerçekten geri yazılan değerler (çağıran kendi config'ini/durumunu
    /// ve canlı eylemlerini eşitlesin diye; ADR-0004 §14-5). Değerler yalnızca sonuç `Restored {
    /// id: Some(_) }` ya da `Partial` iken dolu; çakışıp atlanan parçalar listede yok. Değerler
    /// loglara ve IPC JSON'una girmez ([`RestoredValues`]'ın `Debug`'ı içerik göstermez,
    /// `Serialize` yok).
    pub fn restore_with_values(
        &self,
        id: &SnapshotId,
        mode: RestoreMode,
    ) -> (RestoreOutcome, RestoredValues) {
        let (out, values) = self.restore_inner(id, mode).unwrap_or_else(|e| {
            let out = match e {
                ApplyError::Busy => RestoreOutcome::Busy,
                ApplyError::Skipped(reason) => RestoreOutcome::Skipped { reason },
                other => RestoreOutcome::Error {
                    kind: other.kind_name(),
                },
            };
            (out, RestoredValues::default())
        });
        tracing::info!(id = %id, result = out.name(), "nuxut-apply: geri yükle");
        (out, values)
    }

    fn restore_inner(
        &self,
        id: &SnapshotId,
        mode: RestoreMode,
    ) -> Result<(RestoreOutcome, RestoredValues), ApplyError> {
        let none = RestoredValues::default;
        self.env.guard(TargetKind::File)?;
        let store = Store::open(&self.env)?;
        let _lock = store.lock(self.lock_wait)?;
        self.recover(&store);
        let Some(mut old) = store.load(id) else {
            return Ok((RestoreOutcome::NotFound, none()));
        };
        self.guard_all(old.fragments.iter().map(|e| &e.fragment))?;

        let frags: Vec<&Fragment> = old.fragments.iter().map(|e| &e.fragment).collect();
        let (slots, slot_of, current) = self.read_state(&frags)?;
        let mut items = Vec::new();
        let mut conflicts = Vec::new();
        for ((e, cur), slot) in old.fragments.iter().zip(current).zip(slot_of) {
            let h = value_hash(&cur);
            if h.as_deref() == e.before.hash() {
                continue; // zaten öncesinde
            }
            // Sonradan değişmiş: güvenli kipte atlanır.
            if !e.is_ours(h.as_deref()) && mode == RestoreMode::Safe {
                conflicts.push(self.summarize(&e.fragment, cur.presence(), e.before.presence()));
                continue;
            }
            let target = store.get(&old.id, &e.before)?;
            // Depodan okunan değer güvenilmez: istek kurallarıyla yeniden denetlenir (ör. INI
            // değerinde satır sonu → başka anahtar enjeksiyonu).
            crate::model::value_fits(&e.fragment, &target)
                .map_err(|_| ApplyError::SnapshotUnavailable)?;
            items.push(Item {
                fragment: e.fragment.clone(),
                before: cur,
                after: target,
                slot,
            });
        }
        if items.is_empty() {
            return Ok((
                if conflicts.is_empty() {
                    RestoreOutcome::Restored { id: None }
                } else {
                    RestoreOutcome::Conflict { conflicts }
                },
                none(),
            ));
        }
        // Nuxut'un yarattığı ve geri yüklemede boş kalan dosya silinir. Eksik dosya yaratılmaz
        // (kullanıcı ana dosyayı silmişse satır geri eklenmez: `not_found`).
        let mut rules: Vec<FileRules> = (0..slots.len())
            .map(|s| {
                let mine: Vec<&Entry> = old
                    .fragments
                    .iter()
                    .filter(|e| {
                        e.fragment
                            .file()
                            .and_then(|p| resolve(p).ok())
                            .is_some_and(|r| r == slots[s].real)
                    })
                    .collect();
                // Dosya başına: bu işlemin dosyadaki tüm parçaları Nuxut yaratımı (kendisi yarattı
                // ya da yazım anında yaratıcıdan devraldı). Hangi sırayla geri alınırsa alınsın
                // dosyayı boşaltan son geri yükleme siler; tek bir ortadaki geri yükleme boşaltmaz.
                FileRules {
                    delete_if_empty: !mine.is_empty() && mine.iter().all(|e| e.nuxut_file()),
                    ..FileRules::default()
                }
            })
            .collect();

        let now_ms = self.now_ms();
        let all = store.load_all()?;
        set_drop_sections(&mut rules, &slots, &items, &all);
        let values = RestoredValues {
            setting: Some(old.setting.clone()),
            values: items
                .iter()
                .filter(|it| !matches!(it.fragment, Fragment::FileForm { .. }))
                .filter_map(|it| {
                    let value = match &it.after {
                        FragValue::Text(t) => Some(t.clone()),
                        FragValue::Absent => None,
                        FragValue::File { .. } => return None,
                    };
                    Some(RestoredValue {
                        fragment: it.fragment.clone(),
                        value,
                    })
                })
                .collect(),
        };
        let mut m = Manifest::new(
            store.fresh_id((self.clock)()),
            now_ms,
            old.setting.clone(),
            old.label.clone(),
            self.source,
        );
        m.restores = Some(old.id.clone());
        set_keepers(&mut rules, &slots, &items, &store, &m.id, Some(&old.id));
        match self.commit(&store, &all, None, m, items, &slots, &rules, now_ms)? {
            Commit::Applied(new_id) => {
                remove_created_dirs(&self.env, &old.created_dirs);
                old.state = TxState::Restored(new_id.clone());
                // Geri yükleme yazıldı: eski kaydın işareti güncellenemese de sonuç başarılı.
                match store.save(&old) {
                    Ok(()) => store.journal(&old),
                    Err(_) => tracing::warn!(
                        id = %old.id,
                        "nuxut-apply: geri yüklenen kaydın durumu güncellenemedi"
                    ),
                }
                // Budama eski kayıt kaydedildikten SONRA: eski kayıt budanırsa temizce gider
                // (önce budanıp sonra kaydedilseydi blob'suz bir kayıt dirilirdi).
                self.prune(&store, &new_id);
                Ok((
                    if conflicts.is_empty() {
                        RestoreOutcome::Restored { id: Some(new_id) }
                    } else {
                        RestoreOutcome::Partial {
                            id: new_id,
                            conflicts,
                        }
                    },
                    values,
                ))
            }
            Commit::Noop => Ok((RestoreOutcome::Restored { id: None }, none())),
            Commit::Partial(_, _) => Ok((
                RestoreOutcome::Error {
                    kind: "partial_rollback",
                },
                none(),
            )),
        }
    }

    // -----------------------------------------------------------------------------------------
    // Listeleme

    /// Yeniden eskiye snapshot'lar.
    pub fn list(&self, filter: &Filter) -> Result<Vec<SnapshotSummary>, ApplyError> {
        let store = self.open_for_read()?;
        let mut all = store.load_all()?;
        all.retain(|m| filter.setting.as_ref().is_none_or(|s| *s == m.setting));
        all.sort_by(|a, b| (b.created_ms, &b.id).cmp(&(a.created_ms, &a.id)));
        all.truncate(filter.limit.clamp(1, 200));
        let mut cache = PeekCache::new();
        Ok(all.iter().map(|m| self.summary(m, &mut cache)).collect())
    }

    /// Tek snapshot ayrıntısı (değer yok).
    pub fn show(&self, id: &SnapshotId) -> Result<Option<SnapshotDetail>, ApplyError> {
        let store = self.open_for_read()?;
        Ok(store.load(id).map(|m| {
            let s = self.summary(&m, &mut PeekCache::new());
            SnapshotDetail {
                id: s.id,
                at: s.at,
                created_ms: s.created_ms,
                setting: s.setting,
                label: s.label,
                source: s.source,
                state: s.state,
                restored_by: s.restored_by,
                restores: s.restores,
                pinned: s.pinned,
                conflicts: s.conflicts,
                fragments: m.fragments.iter().map(|e| self.entry_summary(e)).collect(),
            }
        }))
    }

    /// Ayarın en son `Applied` işlemi ("önceki hale dön").
    pub fn latest(&self, setting: &SettingId) -> Result<Option<SnapshotSummary>, ApplyError> {
        let store = self.open_for_read()?;
        let all = store.load_all()?;
        Ok(all
            .iter()
            .filter(|m| m.setting == *setting && m.state == TxState::Applied)
            .max_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)))
            .map(|m| self.summary(m, &mut PeekCache::new())))
    }

    /// Ayarın sabitlenmiş ilk işlemi ("Nuxut öncesine dön"); yoksa en eskisi.
    pub fn original(&self, setting: &SettingId) -> Result<Option<SnapshotSummary>, ApplyError> {
        let store = self.open_for_read()?;
        let all = store.load_all()?;
        let mut mine: Vec<&Manifest> = all
            .iter()
            .filter(|m| m.setting == *setting && m.state != TxState::RolledBack)
            .collect();
        mine.sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
        Ok(mine
            .iter()
            .find(|m| m.pinned)
            .or(mine.first())
            .map(|m| self.summary(m, &mut PeekCache::new())))
    }

    fn open_for_read(&self) -> Result<Store, ApplyError> {
        self.env.guard(TargetKind::File)?;
        let store = Store::open(&self.env)?;
        if let Some(_g) = store.try_lock() {
            self.recover(&store);
        }
        Ok(store)
    }

    fn summary(&self, m: &Manifest, cache: &mut PeekCache) -> SnapshotSummary {
        let conflicts = match m.state {
            TxState::Applied | TxState::Interrupted => {
                let frags: Vec<&Fragment> = m.fragments.iter().map(|e| &e.fragment).collect();
                self.peek(cache, &frags)
                    .iter()
                    .zip(&m.fragments)
                    .filter(|(cur, e)| {
                        cur.as_ref()
                            .is_some_and(|c| !e.is_ours(value_hash(c).as_deref()))
                    })
                    .count()
            }
            _ => 0,
        };
        SnapshotSummary {
            id: m.id.clone(),
            at: m.id.iso(),
            created_ms: m.created_ms,
            setting: m.setting.clone(),
            label: m.label.clone(),
            source: m.source,
            state: m.state.name(),
            restored_by: match &m.state {
                TxState::Restored(id) => Some(id.clone()),
                _ => None,
            },
            restores: m.restores.clone(),
            pinned: m.pinned,
            fragments: m.fragments.len(),
            conflicts,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Bakım (kilit altında)

    /// Kilit bizdeyken `Pending` görülen işlem yarıda kalmıştır → `Interrupted` (geri alınmaz).
    fn recover(&self, store: &Store) {
        let Ok(ids) = store.ids() else { return };
        for id in ids {
            // Manifest'ten önce çökmüş: hiçbir dış hedefe dokunulmamıştı (manifest yazılmadan
            // yazım yapılmaz) → iz silinir.
            if store.is_orphan(&id) {
                tracing::warn!(id = %id, "nuxut-apply: manifest'siz snapshot dizini silindi");
                store.remove(&id);
                continue;
            }
            if let Some(mut m) = store.load(&id)
                && m.state == TxState::Pending
            {
                m.state = TxState::Interrupted;
                if store.save(&m).is_ok() {
                    tracing::warn!(id = %id, "nuxut-apply: yarım kalmış işlem işaretlendi");
                    store.journal(&m);
                }
            }
        }
    }

    /// Saklama: ayar başına son 20 + sabitlenmişler; toplam 500 işlem ve 64 MiB.
    fn prune(&self, store: &Store, keep: &SnapshotId) {
        let Ok(ids) = store.ids() else { return };
        // (id, oluşturulma, ayar, sabit, bekliyor)
        let mut rows: Vec<(SnapshotId, u64, Option<SettingId>, bool, bool)> = ids
            .into_iter()
            .map(|id| match store.load(&id) {
                Some(m) => (
                    id,
                    m.created_ms,
                    Some(m.setting),
                    m.pinned,
                    m.state == TxState::Pending,
                ),
                None => {
                    let ms = id.unix_secs() * 1000;
                    (id, ms, None, false, false)
                }
            })
            .collect();
        rows.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
        let deletable =
            |r: &(SnapshotId, u64, Option<SettingId>, bool, bool)| !r.3 && !r.4 && r.0 != *keep;

        let mut doomed: Vec<SnapshotId> = Vec::new();
        let mut per: HashMap<&SettingId, usize> = HashMap::new();
        for r in rows.iter().rev() {
            if let Some(s) = &r.2
                && !r.3
            {
                let n = per.entry(s).or_default();
                *n += 1;
                if *n > self.limits.0 && deletable(r) {
                    doomed.push(r.0.clone());
                }
            }
        }
        let mut live: Vec<&(SnapshotId, u64, Option<SettingId>, bool, bool)> =
            rows.iter().filter(|r| !doomed.contains(&r.0)).collect();
        let mut sizes: HashMap<SnapshotId, u64> = live
            .iter()
            .map(|r| (r.0.clone(), store.size(&r.0)))
            .collect();
        let mut total: u64 = sizes.values().sum();
        let mut i = 0;
        while (live.len() > self.limits.1 || total > self.limits.2) && i < live.len() {
            if deletable(live[i]) {
                let r = live.remove(i);
                total -= sizes.remove(&r.0).unwrap_or(0);
                doomed.push(r.0.clone());
            } else {
                i += 1;
            }
        }
        for id in &doomed {
            store.remove(id);
        }
        if !doomed.is_empty() {
            tracing::debug!(removed = doomed.len(), "nuxut-apply: snapshot budaması");
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Yardımcılar

fn kind_of(f: &Fragment) -> TargetKind {
    match f {
        Fragment::Dconf { .. } => TargetKind::Dconf,
        Fragment::UnitState { .. } => TargetKind::Unit,
        _ => TargetKind::File,
    }
}

fn edit_err(e: EditError, path: &Path) -> ApplyError {
    match e {
        EditError::InvalidValue => ApplyError::Invalid("değer bu dosya biçimine yazılamaz"),
        EditError::Parse | EditError::NotAValue | EditError::Malformed => {
            ApplyError::Parse(path.to_path_buf())
        }
    }
}

/// Parçanın dosya içeriğindeki değeri (`bytes = None` → dosya yok).
fn frag_value(
    f: &Fragment,
    bytes: Option<&[u8]>,
    path: &Path,
    mode: Option<u32>,
) -> Result<FragValue, ApplyError> {
    let Some(bytes) = bytes else {
        return Ok(FragValue::Absent);
    };
    if let Fragment::ManagedFile { .. } = f {
        return Ok(FragValue::File {
            bytes: bytes.to_vec(),
            mode: mode.unwrap_or(0) & 0o777,
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ApplyError::Parse(path.to_path_buf()))?;
    let v = match f {
        Fragment::TomlKey { key, .. } => {
            edit::toml::get(text, key).map_err(|e| edit_err(e, path))?
        }
        Fragment::IniKey { section, key, .. } => edit::ini::get(text, section, key),
        Fragment::Line { marker, .. } => edit::line::get(text, marker),
        Fragment::Block { id, .. } => edit::block::get(text, id).map_err(|e| edit_err(e, path))?,
        _ => None,
    };
    Ok(v.map_or(FragValue::Absent, FragValue::Text))
}

/// İstenen değeri `get`'in döndüreceği biçime getirir (karşılaştırma için).
fn normalize(f: &Fragment, v: FragValue) -> Result<FragValue, ApplyError> {
    Ok(match (f, v) {
        (Fragment::TomlKey { .. }, FragValue::Text(t)) => {
            edit::toml::parse_value(&t).map_err(|_| ApplyError::Invalid("geçersiz TOML değeri"))?;
            FragValue::Text(t.trim().to_owned())
        }
        (Fragment::IniKey { .. }, FragValue::Text(t)) => FragValue::Text(t.trim().to_owned()),
        (Fragment::Block { .. }, FragValue::Text(t)) => FragValue::Text(edit::block::normalize(&t)),
        (_, v) => v,
    })
}

/// İçerik eşitliği (dosya izni hariç). dconf değerleri GVariant anlamıyla karşılaştırılır
/// (`1.00` == `1.0`; `dconf read` GLib biçiminde döndürür — [`edit::gvariant`]).
fn same(f: &Fragment, a: &FragValue, b: &FragValue) -> bool {
    match (a, b) {
        (FragValue::Absent, FragValue::Absent) => true,
        (FragValue::Absent, _) | (_, FragValue::Absent) => false,
        (FragValue::Text(x), FragValue::Text(y)) if matches!(f, Fragment::Dconf { .. }) => {
            edit::gvariant::equivalent(x, y)
        }
        _ => a.bytes() == b.bytes(),
    }
}

/// Dosyanın yeni hâli: içerik ([`render_content`]) ve biçim parçası varsa biçim değişimi.
/// Biçim değişimi içeriği taşır (devralma: düz dosya; salt okunura dönüş: izinle); symlink ya da
/// sabit bağa dönüşte içerik bağın hedefinindir, düzenlemeler yazılmaz.
fn render(slot: &FileSlot, edits: &[&Item], rules: &FileRules) -> Result<Action, ApplyError> {
    let form = edits
        .iter()
        .find(|it| matches!(it.fragment, Fragment::FileForm { .. }));
    let content: Vec<&Item> = edits
        .iter()
        .filter(|it| !matches!(it.fragment, Fragment::FileForm { .. }))
        .copied()
        .collect();
    let action = if content.is_empty() {
        Action::Nothing
    } else {
        render_content(slot, &content, rules)?
    };
    let Some(form) = form else {
        return Ok(action);
    };
    let parse = |v: &FragValue| match v {
        FragValue::Text(t) => FileForm::parse(t),
        _ => None,
    };
    let target = parse(&form.after).ok_or(ApplyError::Invalid("biçim geri yüklenemez"))?;
    let cur_mode = slot.current.as_ref().map(|c| c.mode & 0o777);
    let (bytes, mode) = match action {
        Action::Write { bytes, mode } => (bytes, mode.or(cur_mode)),
        // Nuxut'un yarattığı dosyanın "boş kalınca silinmesi" biçim dönüşünde uygulanmaz.
        Action::Nothing | Action::Remove | Action::Form(_) => (
            slot.current.as_ref().map(|c| c.bytes.clone()).unwrap_or_default(),
            cur_mode,
        ),
    };
    let mode = mode.unwrap_or(0o644) & 0o777;
    Ok(Action::Form(match target {
        FileForm::Regular => FormAction::File {
            bytes,
            mode: mode | 0o200,
            keep: (parse(&form.before) == Some(FileForm::HardLink))
                .then(|| rules.keep_to.clone())
                .flatten(),
        },
        FileForm::ReadOnly(m) | FileForm::Foreign(m) => FormAction::File {
            bytes,
            mode: m,
            keep: None,
        },
        FileForm::Symlink(t) => FormAction::Symlink(t),
        // Tutulan inode yoksa (budandı, başka dosya sistemi) yalnızca içerik geri gelir.
        FileForm::HardLink => match &rules.keep_from {
            Some(k) if std::fs::symlink_metadata(k).is_ok_and(|m| m.is_file()) => {
                FormAction::Link(k.clone())
            }
            _ => FormAction::File {
                bytes,
                mode,
                keep: None,
            },
        },
    }))
}

/// Dosyanın yeni içeriği. Satır/blok parçası eksik ana dosyayı yaratmaz (`rules.create` açıkça
/// istenmediyse).
fn render_content(
    slot: &FileSlot,
    edits: &[&Item],
    rules: &FileRules,
) -> Result<Action, ApplyError> {
    let path = edits
        .first()
        .and_then(|it| it.fragment.file())
        .unwrap_or(&slot.real)
        .to_path_buf();
    if let Some(it) = edits
        .iter()
        .find(|it| matches!(it.fragment, Fragment::ManagedFile { .. }))
    {
        return Ok(match &it.after {
            FragValue::File { bytes, mode } => Action::Write {
                bytes: bytes.clone(),
                mode: (*mode != 0).then_some(*mode),
            },
            FragValue::Absent if slot.current.is_some() => Action::Remove,
            _ => Action::Nothing,
        });
    }
    let mut text = match &slot.current {
        Some(c) => {
            String::from_utf8(c.bytes.clone()).map_err(|_| ApplyError::Parse(path.clone()))?
        }
        None => {
            if edits.iter().all(|it| it.after.is_absent()) {
                return Ok(Action::Nothing);
            }
            if !rules.create
                && edits
                    .iter()
                    .any(|it| matches!(it.fragment, Fragment::Line { .. } | Fragment::Block { .. }))
            {
                return Err(ApplyError::NotFound(path));
            }
            String::new()
        }
    };
    for it in edits {
        let value = match &it.after {
            FragValue::Text(t) => Some(t.as_str()),
            FragValue::Absent => None,
            FragValue::File { .. } => {
                return Err(ApplyError::Invalid("değer parça türüne uymuyor"));
            }
        };
        text = match &it.fragment {
            Fragment::TomlKey { key, .. } => edit::toml::set(&text, key, value),
            Fragment::IniKey { section, key, .. } => edit::ini::set(&text, section, key, value),
            Fragment::Line { marker, at, .. } => {
                edit::line::set(&text, marker, value.is_some(), *at)
            }
            Fragment::Block { id, .. } => edit::block::set(&text, id, value),
            _ => return Err(ApplyError::Invalid("dosya parçası değil")),
        }
        .map_err(|e| edit_err(e, &path))?;
    }
    // Nuxut'un yarattığı ve boş kalan INI bölümleri (tüm düzenlemelerden sonra: aynı bölümün
    // birden çok anahtarı aynı işlemde silinebilir).
    for section in &rules.drop_sections {
        text = edit::ini::remove_empty_section(&text, section);
    }
    if rules.delete_if_empty && vacant(&text, edits) && slot.current.is_some() {
        return Ok(Action::Remove);
    }
    Ok(Action::Write {
        bytes: text.into_bytes(),
        mode: None,
    })
}

/// Nuxut'un yarattığı dosya geri yükleme sonunda "boş" mu (silinebilir mi)? Biçim, dosyaya
/// yazan parçalardan: yalnızca `IniKey` → boş satır/boşluk ve bölüm başlıkları (`[Settings]`);
/// yalnızca `TomlKey` → boş satır/boşluk ve tablo başlıkları (`[a.b]`; `[[dizi]]` öğesi içerik
/// sayılır); diğerleri (`Line`/`Block`, karışık) → yalnızca boşluk. Yorum, anahtar ya da başka her
/// satır içeriktir (kullanıcının eklediği) → dosya kalır. `ManagedFile` `render`'da ayrıca
/// (değer `Absent` → silme), dconf/birim dosya değildir.
fn vacant(text: &str, edits: &[&Item]) -> bool {
    let ini = edits
        .iter()
        .all(|it| matches!(it.fragment, Fragment::IniKey { .. }));
    let toml = edits
        .iter()
        .all(|it| matches!(it.fragment, Fragment::TomlKey { .. }));
    text.lines().all(|l| {
        let t = l.trim();
        t.is_empty()
            || ((ini || toml)
                && t.len() > 2
                && t.starts_with('[')
                && t.ends_with(']')
                && !(toml && t.starts_with("[[")))
    })
}

/// Var olan dosya Nuxut yaratımı mı (yaratıcı bilgisinin devri)? Ancak hâlâ uygulanmış
/// (`Applied`/`Interrupted`; geri alınmış ya da yuvarlanmış değil) bir işlemin bu gerçek dosyadaki
/// Nuxut-yaratımı parçası (`file_existed = false` ya da kendisi devralmış) bir değer yazmışsa ve o
/// değer dosyada hâlâ duruyorsa (`is_ours`) evet. Değer koşulu sürekliliğin kanıtıdır: kullanıcı
/// dosyayı silip kendi dosyasını yarattıysa yaratıcının değeri orada olmaz, devir olmaz.
/// Yaratılacak dosyaların (şu an yok, işlem bir değer yazacak) ataları: yolun yukarısında var
/// olmayan her dizin bu işlemin yarattığıdır; var olan bir ata, uygulanmış (`Applied` /
/// `Interrupted`) başka bir işlemin yarattığı dizinse devralınır (aynı dizine birden çok ayar
/// yazar: hangi sırayla geri alınırsa alınsın son boşaltan siler). İlk "kullanıcının" (kayıtsız
/// var olan) dizinde durulur. Kilit altında, yazımdan önce çağrılır.
fn record_created_dirs(m: &mut Manifest, all: &[Manifest], slots: &[FileSlot], items: &[Item]) {
    let live: Vec<&PathBuf> = all
        .iter()
        .filter(|o| o.id != m.id && matches!(o.state, TxState::Applied | TxState::Interrupted))
        .flat_map(|o| o.created_dirs.iter())
        .collect();
    let creating = items.iter().filter(|it| !it.after.is_absent()).filter_map(|it| it.slot);
    for s in creating {
        if slots[s].current.is_some() {
            continue;
        }
        let mut dir = slots[s].real.parent();
        for _ in 0..32 {
            let Some(d) = dir.filter(|d| d.parent().is_some()) else { break };
            let ours = match std::fs::symlink_metadata(d) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
                Ok(md) if md.is_dir() => live.iter().any(|l| l.as_path() == d),
                _ => false,
            };
            if !ours {
                break;
            }
            if !m.created_dirs.iter().any(|c| c == d) && m.created_dirs.len() < crate::store::MAX_CREATED_DIRS {
                m.created_dirs.push(d.to_path_buf());
            }
            dir = d.parent();
        }
    }
}

/// Geri yükleme sonrası: işlemin yarattığı dizinler en derinden başlayarak, yalnızca boşsa ve
/// hâlâ (symlink olmayan) dizinse silinir. Ortam kapısı (test örneği kökleri) uygulanır; hata
/// önemsiz (dolu dizin, yarış).
fn remove_created_dirs(env: &Env, dirs: &[PathBuf]) {
    let mut dirs: Vec<&PathBuf> = dirs.iter().collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs {
        let is_dir = std::fs::symlink_metadata(d).is_ok_and(|m| m.is_dir());
        if is_dir
            && env.check_path(d).is_ok()
            && std::fs::remove_dir(d).is_ok()
            && let Some(parent) = d.parent()
        {
            crate::sys::sync_dir(parent);
        }
    }
}

fn inherits_nuxut_file(all: &[Manifest], slot: &FileSlot) -> bool {
    let Some(cur) = &slot.current else {
        return false;
    };
    all.iter()
        .filter(|m| matches!(m.state, TxState::Applied | TxState::Interrupted))
        .flat_map(|m| m.fragments.iter())
        .filter(|e| e.nuxut_file() && e.after.hash().is_some())
        .any(|e| {
            let Some(f) = e.fragment.file() else {
                return false;
            };
            (f == slot.real || resolve(f).is_ok_and(|r| r == slot.real))
                && frag_value(&e.fragment, Some(&cur.bytes), f, Some(cur.mode))
                    .is_ok_and(|v| e.is_ours(value_hash(&v).as_deref()))
        })
}

/// Aynı ayar/kaynağın en son işlemi; `window` içinde (türetilmiş yazımda `None`: süresiz) ve araya
/// kimse girmemişse (örtüşen parçaların şu anki değeri o işlemin `after`'ı).
fn coalesce_target(
    all: &[Manifest],
    setting: &SettingId,
    source: Source,
    now_ms: u64,
    window: Option<u64>,
    items: &[Item],
) -> Option<Manifest> {
    let last = all
        .iter()
        .filter(|m| m.setting == *setting)
        .max_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)))?;
    let fits = last.state == TxState::Applied
        && last.source == source
        && last.restores.is_none()
        && window.is_none_or(|w| now_ms.saturating_sub(last.updated_ms) <= w)
        // Birleşik işlem de tek işlem sınırına uyar: birleşim (örtüşen parça bir kez sayılır).
        && crate::model::fragments_fit(
            last.fragments.iter().map(|e| &e.fragment).chain(
                items
                    .iter()
                    .map(|it| &it.fragment)
                    .filter(|f| !last.fragments.iter().any(|e| e.fragment == **f)),
            ),
        )
        && items.iter().all(|it| {
            last.fragments
                .iter()
                .find(|e| e.fragment == it.fragment)
                .is_none_or(|e| e.after.hash() == value_hash(&it.before).as_deref())
        });
    fits.then(|| last.clone())
}

/// İstekteki devralınacak yollardan gerçekten dışarıdan yönetilenler için biçim parçası (istenen:
/// düz dosya). Devralınamayan neden (başka sahip, depo, yazılamayan dizin) → `ExternallyManaged`;
/// dışarıdan yönetilmeyen yol atlanır.
fn takeover_forms(change: &Change) -> Result<Vec<(Fragment, FragValue)>, ApplyError> {
    let mut out = Vec::new();
    for p in &change.take_over {
        match external_reason(p) {
            None => {}
            Some(r) if r.can_take_over(p) => out.push((
                Fragment::FileForm { file: p.clone() },
                FragValue::Text(FileForm::Regular.to_text()),
            )),
            Some(_) => return Err(ApplyError::ExternallyManaged(p.clone())),
        }
    }
    Ok(out)
}

/// Sabit bağ biçim geçişleri için inode bağları: sabit bağdan çıkan yol özgün inode'u bu işlemin
/// dizininde tutar (`keep_to`); sabit bağa dönen yol geri yüklenen işlemin (`from`) bağını kullanır.
fn set_keepers(
    rules: &mut [FileRules],
    slots: &[FileSlot],
    items: &[Item],
    store: &Store,
    id: &SnapshotId,
    from: Option<&SnapshotId>,
) {
    let hard = |v: &FragValue| matches!(v, FragValue::Text(t) if t == "hardlink");
    for it in items {
        let (Fragment::FileForm { .. }, Some(s)) = (&it.fragment, it.slot) else {
            continue;
        };
        let real = &slots[s].real;
        if hard(&it.before) {
            rules[s].keep_to = Some(store.keeper(id, real));
        }
        if hard(&it.after)
            && let Some(old) = from
        {
            rules[s].keep_from = Some(store.keeper(old, real));
        }
    }
}

/// Silinen INI anahtarlarının bölümlerinden, Nuxut'un yarattığı (depoda `section_created` ile
/// kayıtlı, geri alınmamış bir işlemde aynı gerçek dosya + bölüm) olanları işaretler: yazım
/// sonunda boş kalırlarsa başlıkları da silinir. Kullanıcının bölümü hiç işaretlenmez.
fn set_drop_sections(
    rules: &mut [FileRules],
    slots: &[FileSlot],
    items: &[Item],
    all: &[Manifest],
) {
    for it in items {
        let (Fragment::IniKey { file, section, .. }, FragValue::Absent, Some(s)) =
            (&it.fragment, &it.after, it.slot)
        else {
            continue;
        };
        if section.is_empty() || rules[s].drop_sections.contains(section) {
            continue;
        }
        let created = all
            .iter()
            .filter(|m| m.state != TxState::RolledBack)
            .any(|m| {
                m.fragments.iter().any(|e| match &e.fragment {
                    Fragment::IniKey {
                        file: f,
                        section: sec,
                        ..
                    } if e.section_created && sec == section => {
                        f == file || resolve(f).is_ok_and(|r| r == slots[s].real)
                    }
                    _ => false,
                })
            });
        if created {
            rules[s].drop_sections.push(section.clone());
        }
    }
}

#[cfg(all(test, unix))]
#[path = "applier_tests.rs"]
mod tests;
