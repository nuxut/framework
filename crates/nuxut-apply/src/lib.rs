//! Nuxut dış yapılandırma uygulama katmanı (shell ADR-0001).
//!
//! Nuxut'un kendi dosyaları dışına (GTK/Qt ayarları, Hyprland ana dosyası, dconf, systemd
//! kullanıcı birimleri) ve kendi config göçlerine yazan **her şey** buradan geçer:
//!
//! - [`Env`] / [`Env::guard`]: yazımın bu ortamda güvenli olup olmadığına karar veren tek yer
//!   (test örneği kullanıcının dosyalarına asla yazmaz).
//! - [`read_file`] / [`write_file`]: tek atomik yazıcı (symlink korunur, izin korunur, ön koşul
//!   denetimi, dizin `fsync`, aynı içerik yazılmaz).
//! - [`edit`]: parça düzenleyicileri (TOML, INI, işaretli satır/blok, yönetilen dosya); yönetilen
//!   parça dışındaki her bayt korunur.
//! - [`Fragment`], [`FragValue`], [`Change`]: uygulama isteğinin modeli.
//! - [`Applier`]: plan / uygula / geri yükle / listele; snapshot deposu
//!   `$XDG_STATE_HOME/nuxut/snapshots` (dizin `0700`, dosyalar `0600`), `flock` kilidi, budama.
//! - [`Backend`] / [`SystemBackend`]: dconf ve systemd `--user` (sabit argv alt süreç, zaman
//!   aşımı); test örneğinde hiç çağrılmaz.
//!
//! Loglara dosya içeriği ya da değer yazılmaz (framework değişmez 9); hata ve model tiplerinin
//! `Display`/`Debug`'ı yol ve değer içermez.

#![forbid(unsafe_code)]

mod applier;
mod backend;
pub mod edit;
mod env;
mod error;
mod model;
mod store;
mod sys;
mod takeover;
mod write;

pub use applier::{
    Applier, COALESCE_MS, Filter, FragmentSummary, LOCK_WAIT, Outcome, PER_SETTING_LIMIT, Plan,
    PlanEntry, RestoreMode, RestoreOutcome, RestoredValue, RestoredValues, SnapshotDetail,
    SnapshotSummary, TOTAL_BYTES_LIMIT, TOTAL_LIMIT,
};
pub use backend::{Backend, DCONF_TIMEOUT, NoBackend, SYSTEMCTL_TIMEOUT, SystemBackend};
pub use env::{Env, INSTANCE_ENV, Skip, TargetKind};
pub use error::ApplyError;
pub use takeover::{ExternalReason, external_reason};
pub use model::{
    Change, FileForm, FragValue, Fragment, KeyPath, MAX_ADDRESS_BYTES, MAX_FRAGMENTS, MAX_LABEL_CHARS,
    MAX_PATH_BYTES, MAX_TEXT_BYTES, Position, SettingId, SnapshotId, Source, TxState,
    fragments_fit,
};
pub use write::{
    DIR_MODE, Expect, WriteOptions, Written, ensure_dir, read_bytes, read_file, read_optional,
    remove_file, write_file, write_file_with,
};

/// Okunan/yazılan dosya boyu sınırı (`nuxut_config::MAX_CONFIG_BYTES` ile aynı, 1 MiB).
pub const MAX_FILE_BYTES: u64 = nuxut_config::MAX_CONFIG_BYTES;

/// Testlerde alt süreç kapısı.
///
/// Alt süreç başlatmak (`posix_spawn` → `clone(CLONE_VFORK)`; program yoksa da) çocuğa o anki
/// TÜM fd'lerin kopyasını verir; çocuk `exec` edene ya da çıkana kadar başka test thread'lerinin
/// açık dosya açıklamalarını (OFD) da tutar. `apply.lock` bir `flock` kilidi ve OFD'ye bağlıdır:
/// sahibi guard'ı bıraksa da kilit çocuk bitene kadar sürer. Aynı süreçte koşan bir test kilidi
/// bıraktıktan hemen sonra okuma yolunda `try_lock` beklerse (çökme kurtarma: Pending →
/// Interrupted, sahipsiz dizin silme) bu pencereye düşer. Üründe bu yalnızca geçici bir
/// "başkası tutuyor" durumudur (okuma yolu kurtarmayı sonraki çağrıya bırakır, yazma yolu bekler);
/// testlerde ise belirleyici olmayan sonuç verir. Bu yüzden: depo/kilit kullanan testler kapıyı
/// paylaşımlı, alt süreç başlatan testler özel tutar — ikisi aynı anda koşmaz.
#[cfg(test)]
pub(crate) mod test_gate {
    use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

    static CHILD_PROCS: RwLock<()> = RwLock::new(());

    /// Kilit/depo kullanan test: alt süreç başlatan testlerle örtüşmez.
    pub(crate) fn shared() -> RwLockReadGuard<'static, ()> {
        CHILD_PROCS.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Alt süreç başlatan test: o sürerken kilit tutan test koşmaz.
    pub(crate) fn exclusive() -> RwLockWriteGuard<'static, ()> {
        CHILD_PROCS.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Alt süreç başlatılacak an: kapı özel tutulmuyorsa (kapısız yeni test) panik.
    /// Paylaşımlı tutan biri varken ya da kimse tutmazken `try_read` başarılı olur.
    pub(crate) fn assert_spawn_allowed() {
        assert!(
            CHILD_PROCS.try_read().is_err(),
            "alt süreç başlatan test test_gate::exclusive() tutmalı (flock OFD sızıntısı)"
        );
    }
}
