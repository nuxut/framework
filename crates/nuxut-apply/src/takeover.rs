//! Başka aracın yönettiği dosyalar: nedenin sınıflandırılması ve devralma yazıcıları
//! (shell docs/design/look-external.md §4, [`crate::Change::take_over`]).
//!
//! Devralma yazıcıları [`crate::write_file`]'ın aksine yolun **son bileşenini izlemez**: symlink'in,
//! salt okunur dosyanın ya da sabit bağın kendisini aynı dizindeki geçici bir girdiyle `rename`
//! ederek değiştirir (symlink hedefi, öbür bağ ve özgün inode'a dokunulmaz). `rename`'den hemen
//! önce yolun `lstat` kimliği yeniden denetlenir; farklıysa [`ApplyError::Changed`].

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::env::{Env, TargetKind};
use crate::error::ApplyError;
use crate::model::{FileForm, FragValue};
use crate::sys;
use crate::write::{check_shape, resolve};

/// Dosyanın neden dışarıdan yönetildiği (`nuxut-apply`'ın `ExternallyManaged` kararıyla aynı
/// ölçütler ve sıra). IPC adı `snake_case`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalReason {
    /// Yol (ya da bir atası) hedefi olmayan bir symlink.
    SymlinkBroken,
    /// Çözülen yol `/nix/store` ya da `/gnu/store` altında.
    Store,
    /// Dosya başka bir kullanıcıya ait.
    NotOwned,
    /// Sahibinin yazma biti yok.
    ReadOnly,
    /// Sabit bağlı (`nlink > 1`).
    HardLink,
    /// Yeni dosyanın en yakın var olan atası yazılamaz.
    DirNotWritable,
    /// Belirlenemedi (yarış, dosya dışı hedef).
    Unknown,
}

impl ExternalReason {
    pub fn name(self) -> &'static str {
        match self {
            ExternalReason::SymlinkBroken => "symlink_broken",
            ExternalReason::Store => "store",
            ExternalReason::NotOwned => "not_owned",
            ExternalReason::ReadOnly => "read_only",
            ExternalReason::HardLink => "hard_link",
            ExternalReason::DirNotWritable => "dir_not_writable",
            ExternalReason::Unknown => "unknown",
        }
    }

    /// Bu nedenle dışarıdan yönetilen `path` [`crate::Change::take_over`] ile devralınabilir mi?
    /// `path`'in kendisi symlink'se (son bileşen) hedefi ne olursa olsun (başka sahip, salt okunur,
    /// kırık) devralınır: bağın kendisi Nuxut'un düz dosyasıyla değiştirilir, hedefe dokunulmaz
    /// (ör. `gtk.css` → `/usr/share/themes/…`). Düz dosyada salt okunur, sabit bağ ve başka
    /// kullanıcının dosyası (okunabiliyor ve dizini yeniden adlandırmaya izin veriyorsa:
    /// [`sys::foreign_replaceable`]; kök gerekmez, 2026-10-17). Başkasının dosyası geri alınınca
    /// içerik ve izin döner, sahip kullanıcının olur (kök olmadan sahiplik geri verilemez).
    /// Depo (nix/guix; bildirimsel yapılandırma) ve yazılamayan dizin hiç devralınmaz.
    pub fn can_take_over(self, path: &Path) -> bool {
        match self {
            ExternalReason::Store | ExternalReason::DirNotWritable => false,
            ExternalReason::SymlinkBroken | ExternalReason::ReadOnly | ExternalReason::HardLink => {
                true
            }
            ExternalReason::NotOwned | ExternalReason::Unknown => std::fs::symlink_metadata(path)
                .is_ok_and(|m| {
                    m.file_type().is_symlink()
                        || (self == ExternalReason::NotOwned
                            && m.is_file()
                            && sys::foreign_replaceable(path))
                }),
        }
    }
}

/// `path` dışarıdan mı yönetiliyor, yönetiliyorsa neden? Yalnızca `stat`; `None` = Nuxut
/// yazabilir (ya da yol biçimsiz / okunamıyor: o hatayı yazım kendisi döndürür).
pub fn external_reason(path: &Path) -> Option<ExternalReason> {
    let real = match resolve(path) {
        Ok(r) => r,
        Err(ApplyError::ExternallyManaged(_)) => {
            // resolve yalnızca kırık symlink ve depo için bu hatayı verir.
            return Some(if store_path(path) {
                ExternalReason::Store
            } else {
                ExternalReason::SymlinkBroken
            });
        }
        Err(_) => return None,
    };
    match std::fs::metadata(&real) {
        Ok(m) if m.is_file() => {
            let (_, _, nlink) = sys::identity(&m);
            if !sys::owned(&m) {
                Some(ExternalReason::NotOwned)
            } else if sys::mode(&m) & 0o200 == 0 {
                Some(ExternalReason::ReadOnly)
            } else if nlink > 1 {
                Some(ExternalReason::HardLink)
            } else {
                None
            }
        }
        Ok(_) => None,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let mut dir = real.parent();
            while let Some(d) = dir {
                if std::fs::symlink_metadata(d).is_ok() {
                    return (!sys::dir_writable(d)).then_some(ExternalReason::DirNotWritable);
                }
                dir = d.parent();
            }
            None
        }
        Err(_) => None,
    }
}

/// Yol (symlink'leri çözülünce) bir nix/guix deposunda mı?
fn store_path(path: &Path) -> bool {
    let in_store = |p: &Path| p.starts_with("/nix/store") || p.starts_with("/gnu/store");
    if in_store(path) {
        return true;
    }
    // Kırık olmayan symlink'in hedefi; kırık symlink zinciri depoyu gösteriyorsa da depo.
    match std::fs::read_link(path) {
        Ok(t) => in_store(&path.parent().map_or(t.clone(), |p| p.join(&t))),
        Err(_) => path.canonicalize().is_ok_and(|r| in_store(&r)),
    }
}

/// Devralınacak yolun anahtarı: atalar kanonik, son bileşen izlenmez.
pub(crate) fn nofollow_real(path: &Path) -> Result<PathBuf, ApplyError> {
    check_shape(path)?;
    let name = path
        .file_name()
        .ok_or(ApplyError::Invalid("yolun dosya adı yok"))?;
    let parent = path
        .parent()
        .ok_or(ApplyError::Invalid("yolun atası yok"))?;
    let base = resolve(parent)?;
    if base.starts_with("/nix/store") || base.starts_with("/gnu/store") {
        return Err(ApplyError::ExternallyManaged(path.to_path_buf()));
    }
    Ok(base.join(name))
}

/// Yolun şu anki biçimi (`lstat`; son bileşen izlenmez). Yok → `Absent`. Başka kullanıcıya ait
/// düz dosya → `Foreign(izin)` (yeniden adlandırılabiliyorsa), değilse `ExternallyManaged`.
pub(crate) fn form_of(real: &Path) -> Result<FragValue, ApplyError> {
    let m = match std::fs::symlink_metadata(real) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(FragValue::Absent),
        Err(e) => return Err(e.into()),
    };
    let form = if m.file_type().is_symlink() {
        let t = std::fs::read_link(real)?;
        let t = t
            .to_str()
            .ok_or(ApplyError::Invalid("symlink hedefi UTF-8 değil"))?;
        FileForm::Symlink(t.to_owned())
    } else if !m.is_file() {
        return Err(ApplyError::Invalid("hedef düz dosya değil"));
    } else if !sys::owned(&m) {
        if !sys::foreign_replaceable(real) {
            return Err(ApplyError::ExternallyManaged(real.to_path_buf()));
        }
        FileForm::Foreign(sys::mode(&m) & 0o777)
    } else if sys::identity(&m).2 > 1 {
        FileForm::HardLink
    } else if sys::mode(&m) & 0o200 == 0 {
        FileForm::ReadOnly(sys::mode(&m) & 0o777)
    } else {
        FileForm::Regular
    };
    // Geçersiz biçim (ör. denetim karakterli symlink hedefi) saklanamaz.
    FileForm::parse(&form.to_text())
        .map(|f| FragValue::Text(f.to_text()))
        .ok_or(ApplyError::Invalid("symlink hedefi saklanamaz"))
}

/// Yolun kendi (`lstat`) kimliği: ön koşul denetimi. `None` = yok.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathId(Option<(bool, u64, i128, u64)>);

impl PathId {
    pub(crate) fn of(real: &Path) -> Result<Self, ApplyError> {
        match std::fs::symlink_metadata(real) {
            Ok(m) => {
                let (ino, mtime, _) = sys::identity(&m);
                Ok(PathId(Some((
                    m.file_type().is_symlink(),
                    ino,
                    mtime,
                    m.len(),
                ))))
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(PathId(None)),
            Err(e) => Err(e.into()),
        }
    }
}

/// Devralma yazımından önce (snapshot kaydından önce) belli olan engeller.
pub(crate) fn precheck(env: &Env, real: &Path) -> Result<(), ApplyError> {
    env.guard(TargetKind::File)?;
    env.check_path(real)?;
    let dir = real
        .parent()
        .ok_or(ApplyError::Invalid("yolun atası yok"))?;
    if !sys::dir_writable(dir) {
        return Err(ApplyError::ExternallyManaged(real.to_path_buf()));
    }
    Ok(())
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_path(real: &Path) -> Result<PathBuf, ApplyError> {
    let dir = real
        .parent()
        .ok_or(ApplyError::Invalid("yolun atası yok"))?;
    let name = real
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(ApplyError::Invalid("yol UTF-8 değil"))?;
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!(".{name}.nuxut-{}-t{n}.tmp", std::process::id())))
}

/// Geçici girdiyi hazırlayan `make` sonrası: kimlik denetimi → `rename` → dizin `fsync`.
fn swap_in(
    env: &Env,
    real: &Path,
    expect: &PathId,
    make: impl FnOnce(&Path) -> Result<(), ApplyError>,
) -> Result<(), ApplyError> {
    env.guard(TargetKind::File)?;
    env.check_path(real)?;
    let tmp = tmp_path(real)?;
    let result = (|| {
        make(&tmp)?;
        if PathId::of(real)? != *expect {
            return Err(ApplyError::Changed);
        }
        std::fs::rename(&tmp, real).map_err(|e| match e.kind() {
            ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
                ApplyError::ExternallyManaged(real.to_path_buf())
            }
            k => ApplyError::Io(k),
        })?;
        if let Some(dir) = real.parent() {
            sys::sync_dir(dir);
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Yolu (ne olursa olsun) `bytes` içerikli, `mode` izinli düz dosyayla değiştirir.
pub(crate) fn replace_with_file(
    env: &Env,
    real: &Path,
    bytes: &[u8],
    mode: u32,
    expect: &PathId,
) -> Result<(), ApplyError> {
    if bytes.len() as u64 > crate::MAX_FILE_BYTES {
        return Err(ApplyError::TooLarge(real.to_path_buf()));
    }
    swap_in(env, real, expect, |tmp| {
        let mut f = sys::create_tmp(tmp)?;
        f.write_all(bytes)?;
        sys::set_mode(&f, mode & 0o777)?;
        f.sync_all()?;
        Ok(())
    })
}

/// Yolu `target`'ı gösteren symlink'le değiştirir.
pub(crate) fn replace_with_symlink(
    env: &Env,
    real: &Path,
    target: &str,
    expect: &PathId,
) -> Result<(), ApplyError> {
    swap_in(env, real, expect, |tmp| Ok(sys::symlink(target, tmp)?))
}

/// Yolu `keeper` inode'una yeni bir sabit bağla değiştirir (`keeper` yerinde kalır).
pub(crate) fn replace_with_link(
    env: &Env,
    real: &Path,
    keeper: &Path,
    expect: &PathId,
) -> Result<(), ApplyError> {
    swap_in(env, real, expect, |tmp| {
        Ok(std::fs::hard_link(keeper, tmp)?)
    })
}

/// Sabit bağlı dosyanın inode'unu `keeper`'da tutar (geri yükleme bağı yeniden kurar). Başarısızsa
/// (ör. depo başka dosya sistemindeyse) `false`: geri yükleme yalnızca içeriği geri koyar.
pub(crate) fn keep_inode(real: &Path, keeper: &Path) -> bool {
    let _ = std::fs::remove_file(keeper);
    match std::fs::hard_link(real, keeper) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = ?e.kind(), "nuxut-apply: sabit bağ depoda tutulamadı; geri yükleme yalnızca içeriği geri koyar");
            false
        }
    }
}
