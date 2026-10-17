//! Platforma özgü küçük parçalar. Unix'te tam güvence; diğer platformlarda (ADR-0003 çapraz
//! derleme) en iyi çaba: izin/sahip/inode bilgisi yok.

use std::fs::{File, Metadata, OpenOptions};
use std::io;
use std::path::Path;

/// Dosya kimliği: (inode, mtime ns, bağ sayısı).
#[cfg(unix)]
pub fn identity(m: &Metadata) -> (u64, i128, u64) {
    use std::os::unix::fs::MetadataExt;
    (
        m.ino(),
        i128::from(m.mtime()) * 1_000_000_000 + i128::from(m.mtime_nsec()),
        m.nlink(),
    )
}

#[cfg(not(unix))]
pub fn identity(m: &Metadata) -> (u64, i128, u64) {
    let ns = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    (0, ns, 1)
}

/// İzin bitleri (`mode & 0o7777`).
#[cfg(unix)]
pub fn mode(m: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
pub fn mode(_m: &Metadata) -> u32 {
    0o644
}

/// Dosya bu sürecin (etkin) kullanıcısına mı ait ve sahibi yazabiliyor mu?
#[cfg(unix)]
pub fn owned_and_writable(m: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.uid() == euid() && !pretend_foreign(m) && m.mode() & 0o200 != 0
}

#[cfg(not(unix))]
pub fn owned_and_writable(m: &Metadata) -> bool {
    !m.permissions().readonly()
}

/// Okumak için açar: `O_NONBLOCK` (FIFO/aygıt açılışta takılmaz; düz dosyada etkisi yok),
/// `nofollow` ise son bileşen symlink olamaz (`O_NOFOLLOW` → `ELOOP`). Çağıran ardından
/// `metadata().is_file()` denetler.
pub fn open_read(path: &Path, nofollow: bool) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    {
        use rustix::fs::OFlags;
        use std::os::unix::fs::OpenOptionsExt;
        let mut flags = OFlags::NONBLOCK;
        if nofollow {
            flags |= OFlags::NOFOLLOW;
        }
        o.custom_flags(flags.bits() as i32);
    }
    #[cfg(not(unix))]
    let _ = nofollow;
    o.open(path)
}

/// Deponun kendi dosyası için ekleme/yaratma (`0600`, son bileşen symlink olamaz).
pub fn open_private(path: &Path, append: bool) -> io::Result<File> {
    let mut o = OpenOptions::new();
    if append {
        o.append(true);
    } else {
        o.read(true).write(true).truncate(false);
    }
    o.create(true);
    #[cfg(unix)]
    {
        use rustix::fs::OFlags;
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600).custom_flags(OFlags::NOFOLLOW.bits() as i32);
    }
    o.open(path)
}

/// Dizine (etkin kullanıcıyla) yazılabilir mi (`access(W_OK|X_OK)`)?
#[cfg(unix)]
pub fn dir_writable(dir: &Path) -> bool {
    use rustix::fs::{Access, access};
    access(dir, Access::WRITE_OK | Access::EXEC_OK).is_ok()
}

#[cfg(not(unix))]
pub fn dir_writable(dir: &Path) -> bool {
    std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
}

/// Geçici dosya: `create_new`, `0600`.
pub fn create_tmp(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// İzni uygular.
#[cfg(unix)]
pub fn set_mode(f: &File, mode: u32) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    f.set_permissions(std::fs::Permissions::from_mode(mode & 0o7777))
}

#[cfg(not(unix))]
pub fn set_mode(_f: &File, _mode: u32) -> io::Result<()> {
    Ok(())
}

/// Dizin oluşturur (eksik ataları da), verilen izinle.
pub fn create_dir_all(path: &Path, mode: u32) -> io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    b.create(path)
}

/// Dizin girdisini kalıcı kılar (rename/unlink'in kendisi). Unix dışında yok.
pub fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Dosya bu sürecin (etkin) kullanıcısına mı ait?
#[cfg(unix)]
pub fn owned(m: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    m.uid() == euid() && !pretend_foreign(m)
}

#[cfg(not(unix))]
pub fn owned(_m: &Metadata) -> bool {
    true
}

/// Symlink yaratır (devralmanın geri yüklenmesi). Unix dışında desteklenmez.
#[cfg(unix)]
pub fn symlink(target: &str, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
pub fn symlink(_target: &str, _link: &Path) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Etkin kullanıcı.
#[cfg(unix)]
fn euid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Dosya başka kullanıcınınmış gibi mi davranılsın (yalnız test): başka kullanıcıya ait dosya
/// köksüz yaratılamadığından sahiplik denetimleri bu inode üzerinden sınanır. Devralma yeni bir
/// inode yazar; o artık bizimdir (gerçekteki gibi).
#[cfg(unix)]
fn pretend_foreign(m: &Metadata) -> bool {
    #[cfg(test)]
    {
        use std::os::unix::fs::MetadataExt;
        TEST_FOREIGN_INO.with(std::cell::Cell::get) == Some(m.ino())
    }
    #[cfg(not(test))]
    {
        let _ = m;
        false
    }
}

#[cfg(all(unix, test))]
thread_local! {
    pub(crate) static TEST_FOREIGN_INO: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Başka kullanıcıya ait düz dosya kök gerektirmeden, yanına yazılan dosyanın `rename`'iyle
/// değiştirilebilir mi (shell docs/design/look-external.md, 2026-10-17): dosyayı okuyabiliyoruz
/// (özgün içerik snapshot'a girer), ata dizin yazılabilir ve yapışkan bit yok ya da dizin bizim
/// (yapışkan dizinde başkasının girdisi yalnız dizin ya da dosya sahibince değiştirilir).
#[cfg(unix)]
pub fn foreign_replaceable(path: &Path) -> bool {
    use rustix::fs::{Access, access};
    use std::os::unix::fs::MetadataExt;
    let Some(dir) = path.parent() else {
        return false;
    };
    let Ok(dm) = std::fs::metadata(dir) else {
        return false;
    };
    access(path, Access::READ_OK).is_ok()
        && dir_writable(dir)
        && (dm.mode() & 0o1000 == 0 || owned(&dm))
}

#[cfg(not(unix))]
pub fn foreign_replaceable(_path: &Path) -> bool {
    false
}
