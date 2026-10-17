//! Tek atomik yazıcı (ADR-0001 §4). Çekirdek: shell `hypr_config/ensure.rs::append_source_line`.
//!
//! Kurallar:
//! 1. Önce ortam kapısı ([`Env::guard`]); `Err` ise hiçbir dosya açılmaz.
//! 2. Symlink korunur: yol çözülür, hedefe yazılır. Kırık symlink, nix/guix deposu, başka
//!    kullanıcıya ait ya da sahibine salt okunur dosya, sabit bağ (nlink > 1) ve yazılamayan
//!    dizin → [`ApplyError::ExternallyManaged`]; üstüne yazılmaz.
//! 3. Okunan ve yazılan içerik ≤ [`crate::MAX_FILE_BYTES`], büyüğü [`ApplyError::TooLarge`].
//! 4. Aynı dizinde `.<ad>.nuxut-<pid>-<n>.tmp` (`create_new`, `0600`) → yaz → izin → `fsync` →
//!    ön koşul → `rename` → dizin `fsync`. Hata yolunda geçici dosya silinir.
//! 5. Ön koşul: `rename`'den hemen önce (len, mtime_ns, ino, içerik özeti) yeniden okunur;
//!    çağıranın [`Expect`]'inden ya da yazımın başındaki hâlden farklıysa [`ApplyError::Changed`].
//! 6. İçerik (ve istenen izin) aynıysa yazılmaz: [`Written::Unchanged`] (mtime oynamaz).
//! 7. Dosya yoksa yalnızca çağıran `Some(&Expect::Absent)` verdiyse yaratılır (dizinler `0755`);
//!    aksi hâlde [`ApplyError::NotFound`] — kullanıcının ana dosyası yaratılmaz. İzinler için
//!    [`write_file_with`] ([`WriteOptions::PRIVATE`]: `0600`/`0700`); yalnızca dizin: [`ensure_dir`].

use std::io::{ErrorKind, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::MAX_FILE_BYTES;
use crate::env::{Env, TargetKind};
use crate::error::ApplyError;
use crate::sys;

/// Okunan içeriğin özeti: yazım anında dosyanın hâlâ bu olup olmadığı denetlenir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expect {
    /// Dosya yoktu (yazım onu yaratır; o arada biri yaratırsa `Changed`).
    Absent,
    Present {
        len: u64,
        mtime_ns: i128,
        ino: u64,
        hash: [u8; 32],
    },
}

/// Yazım sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    Written,
    /// İçerik zaten aynıydı; dosyaya dokunulmadı.
    Unchanged,
}

/// Var olan dosyanın okunmuş hâli.
#[derive(Debug, Clone)]
pub(crate) struct Current {
    pub bytes: Vec<u8>,
    pub expect: Expect,
    pub mode: u32,
}

/// Dosyayı (symlink ise hedefini) okur: içerik + özet. Yoksa [`ApplyError::NotFound`].
pub fn read_bytes(path: &Path) -> Result<(Vec<u8>, Expect), ApplyError> {
    match read_optional_bytes(path)? {
        Some(c) => Ok((c.bytes, c.expect)),
        None => Err(ApplyError::NotFound(path.to_path_buf())),
    }
}

/// UTF-8 metin olarak okur. UTF-8 değilse [`ApplyError::Parse`].
pub fn read_file(path: &Path) -> Result<(String, Expect), ApplyError> {
    let (bytes, expect) = read_bytes(path)?;
    let text = String::from_utf8(bytes).map_err(|_| ApplyError::Parse(path.to_path_buf()))?;
    Ok((text, expect))
}

/// [`read_file`], ama dosya yoksa `None` (yazarken `Some(&Expect::Absent)` verilir).
pub fn read_optional(path: &Path) -> Result<Option<(String, Expect)>, ApplyError> {
    match read_optional_bytes(path)? {
        Some(c) => {
            let text =
                String::from_utf8(c.bytes).map_err(|_| ApplyError::Parse(path.to_path_buf()))?;
            Ok(Some((text, c.expect)))
        }
        None => Ok(None),
    }
}

pub(crate) fn read_optional_bytes(path: &Path) -> Result<Option<Current>, ApplyError> {
    check_shape(path)?;
    read_current(path)
}

/// `path`'i (symlink'leri izleyerek) açar ve sınırlı okur.
fn read_current(path: &Path) -> Result<Option<Current>, ApplyError> {
    // O_NONBLOCK: hedef bir FIFO ise açılış takılmaz (aşağıda düz dosya değil → Invalid).
    let mut file = match sys::open_read(path, false) {
        Ok(f) => f,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(ApplyError::Invalid("hedef düz dosya değil"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ApplyError::TooLarge(path.to_path_buf()));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    (&mut file)
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ApplyError::TooLarge(path.to_path_buf()));
    }
    let (ino, mtime_ns, _) = sys::identity(&meta);
    let expect = Expect::Present {
        len: bytes.len() as u64,
        mtime_ns,
        ino,
        hash: *blake3::hash(&bytes).as_bytes(),
    };
    Ok(Some(Current {
        bytes,
        expect,
        mode: sys::mode(&meta),
    }))
}

/// Mutlak, UTF-8, `..`/`.` içermeyen yol.
pub(crate) fn check_shape(path: &Path) -> Result<(), ApplyError> {
    if !path.is_absolute() {
        return Err(ApplyError::Invalid("yol mutlak değil"));
    }
    if path.to_str().is_none() {
        return Err(ApplyError::Invalid("yol UTF-8 değil"));
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(ApplyError::Invalid("yolda '..' ya da '.' var"));
    }
    if path.file_name().is_none() {
        return Err(ApplyError::Invalid("yolun dosya adı yok"));
    }
    Ok(())
}

/// Yazılacak gerçek yol: symlink'ler çözülür. Dosya yoksa en yakın var olan ata kanonikleştirilir.
pub(crate) fn resolve(path: &Path) -> Result<PathBuf, ApplyError> {
    check_shape(path)?;
    let real = match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => match path.canonicalize() {
            Ok(r) => r,
            // Kırık symlink: hedefi başka bir aracın (dotfile yöneticisi) — yerine dosya koyulmaz.
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(ApplyError::ExternallyManaged(path.to_path_buf()));
            }
            Err(e) => return Err(e.into()),
        },
        Ok(_) => path.canonicalize()?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let mut rest = Vec::new();
            let mut cur = path;
            loop {
                rest.push(
                    cur.file_name()
                        .ok_or(ApplyError::Invalid("yolun dosya adı yok"))?
                        .to_owned(),
                );
                let parent = cur.parent().ok_or(ApplyError::Invalid("yolun atası yok"))?;
                match std::fs::symlink_metadata(parent) {
                    Ok(m) => {
                        let base = parent.canonicalize().map_err(|e| {
                            if m.file_type().is_symlink() && e.kind() == ErrorKind::NotFound {
                                ApplyError::ExternallyManaged(path.to_path_buf())
                            } else {
                                e.into()
                            }
                        })?;
                        let mut out = base;
                        for c in rest.iter().rev() {
                            out.push(c);
                        }
                        break out;
                    }
                    Err(e) if e.kind() == ErrorKind::NotFound => cur = parent,
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Err(e) => return Err(e.into()),
    };
    if real.starts_with("/nix/store") || real.starts_with("/gnu/store") {
        return Err(ApplyError::ExternallyManaged(path.to_path_buf()));
    }
    Ok(real)
}

/// Var olan dosya bizim yazabileceğimiz biçimde mi (sahip, yazma biti, tek bağ)?
fn check_managed_externally(real: &Path) -> Result<(), ApplyError> {
    match std::fs::metadata(real) {
        Ok(m) => {
            if !m.is_file() {
                return Err(ApplyError::Invalid("hedef düz dosya değil"));
            }
            let (_, _, nlink) = sys::identity(&m);
            if !sys::owned_and_writable(&m) || nlink > 1 {
                return Err(ApplyError::ExternallyManaged(real.to_path_buf()));
            }
            Ok(())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Yazmadan önce (snapshot kaydından önce) belli olan engeller: kapı, kanonik kök, dışarıdan
/// yönetilme (sahip, salt okunur, sabit bağ) ve yeni dosyada yazılamayan dizin. `real` çözülmüş
/// yoldur. Geçmek yazımın başaracağını garanti etmez (yarış), yalnızca kesin başarısızlığı eler.
pub(crate) fn precheck(env: &Env, real: &Path) -> Result<(), ApplyError> {
    env.guard(TargetKind::File)?;
    env.check_path(real)?;
    check_managed_externally(real)?;
    if std::fs::symlink_metadata(real).is_err() {
        // En yakın var olan ata yazılabilir mi?
        let mut dir = real.parent();
        while let Some(d) = dir {
            if std::fs::symlink_metadata(d).is_ok() {
                if !sys::dir_writable(d) {
                    return Err(ApplyError::ExternallyManaged(real.to_path_buf()));
                }
                break;
            }
            dir = d.parent();
        }
    }
    Ok(())
}

/// `path`'e `bytes`'ı atomik yazar (kurallar modül belgesinde).
///
/// `expect`: `None` = ön koşul yalnızca bu çağrının başındaki hâl; `Some(Expect::Absent)` =
/// dosya yok olmalı (yaratılır); `Some(Present{..})` = okunan hâl hâlâ geçerli olmalı.
pub fn write_file(
    env: &Env,
    path: &Path,
    bytes: &[u8],
    expect: Option<&Expect>,
) -> Result<Written, ApplyError> {
    write_impl(env, path, bytes, expect, None, DIR_MODE)
}

/// Yeni dizinlerin varsayılan izni.
pub const DIR_MODE: u32 = 0o755;

/// [`write_file_with`] seçenekleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOptions {
    /// Dosyanın izni (`⊆ 0o777`); `None` = var olanınki korunur, yeni dosya `0644`.
    pub mode: Option<u32>,
    /// Yazım sırasında yaratılan (eksik) ata dizinlerin izni (`⊆ 0o777`, umask uygulanır).
    /// Var olan dizinin iznine dokunulmaz.
    pub dir_mode: u32,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self {
            mode: None,
            dir_mode: DIR_MODE,
        }
    }
}

impl WriteOptions {
    /// Yalnızca kullanıcının: dosya `0600`, yeni dizinler `0700` (durum/özel dosyalar).
    pub const PRIVATE: Self = Self {
        mode: Some(0o600),
        dir_mode: 0o700,
    };
}

/// [`write_file`] + izin seçenekleri. İzinler `0o777` dışında bit taşıyamaz (setuid/setgid/
/// yapışkan bit yazılmaz) → [`ApplyError::Invalid`].
pub fn write_file_with(
    env: &Env,
    path: &Path,
    bytes: &[u8],
    expect: Option<&Expect>,
    opts: &WriteOptions,
) -> Result<Written, ApplyError> {
    if opts.mode.is_some_and(|m| m & !0o777 != 0) || opts.dir_mode & !0o777 != 0 {
        return Err(ApplyError::Invalid("izin 0o777 dışında bit içeriyor"));
    }
    write_impl(env, path, bytes, expect, opts.mode, opts.dir_mode)
}

/// `dir`'i (ve eksik atalarını) `mode` izniyle yaratır (umask uygulanır); ortam kapısı ve test
/// örneği kök kuralı [`write_file`] ile aynı. Dizin zaten varsa [`Written::Unchanged`] (iznine
/// dokunulmaz); yol bir dosya ya da kırık symlink ise [`ApplyError::ExternallyManaged`].
pub fn ensure_dir(env: &Env, dir: &Path, mode: u32) -> Result<Written, ApplyError> {
    env.guard(TargetKind::File)?;
    if mode & !0o777 != 0 {
        return Err(ApplyError::Invalid("izin 0o777 dışında bit içeriyor"));
    }
    let real = resolve(dir)?;
    env.check_path(&real)?;
    match std::fs::metadata(&real) {
        Ok(m) if m.is_dir() => return Ok(Written::Unchanged),
        Ok(_) => return Err(ApplyError::ExternallyManaged(real)),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    sys::create_dir_all(&real, mode).map_err(|e| externally_if_denied(e, &real))?;
    if let Some(parent) = real.parent() {
        sys::sync_dir(parent);
    }
    Ok(Written::Written)
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    /// Test kancası: geçici dosya yazıldıktan sonra, ön koşul denetiminden hemen önce çalışır
    /// (dış değişiklik yarışını belirlenimci kılmak için).
    pub(crate) static BEFORE_RECHECK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const { std::cell::RefCell::new(None) };
}

/// `new_mode`: yeni dosyanın ya da (verilirse) var olanın izni; `None` = var olanınki korunur,
/// yeni dosya `0644`. `dir_mode`: eksik ata dizinlerin izni.
pub(crate) fn write_impl(
    env: &Env,
    path: &Path,
    bytes: &[u8],
    expect: Option<&Expect>,
    new_mode: Option<u32>,
    dir_mode: u32,
) -> Result<Written, ApplyError> {
    env.guard(TargetKind::File)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(ApplyError::TooLarge(path.to_path_buf()));
    }
    let real = resolve(path)?;
    env.check_path(&real)?;
    check_managed_externally(&real)?;
    let current = read_current(&real)?;

    match (expect, &current) {
        (Some(Expect::Absent), Some(_)) => return Err(ApplyError::Changed),
        (Some(e @ Expect::Present { .. }), Some(c)) if *e != c.expect => {
            return Err(ApplyError::Changed);
        }
        (Some(Expect::Present { .. }), None) => return Err(ApplyError::Changed),
        (None, None) => return Err(ApplyError::NotFound(path.to_path_buf())),
        _ => {}
    }

    let mode = match (&current, new_mode) {
        (_, Some(m)) => m & 0o7777,
        (Some(c), None) => c.mode,
        (None, None) => 0o644,
    };
    if let Some(c) = &current
        && c.bytes == bytes
        && c.mode == mode
    {
        return Ok(Written::Unchanged);
    }

    let dir = real
        .parent()
        .ok_or(ApplyError::Invalid("yolun atası yok"))?
        .to_path_buf();
    let name = real
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(ApplyError::Invalid("yol UTF-8 değil"))?;
    if current.is_none() {
        sys::create_dir_all(&dir, dir_mode).map_err(|e| externally_if_denied(e, &real))?;
    }
    let n = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.nuxut-{}-{n}.tmp", std::process::id()));

    let result = (|| -> Result<(), ApplyError> {
        let mut t = sys::create_tmp(&tmp).map_err(|e| externally_if_denied(e, &real))?;
        t.write_all(bytes)?;
        sys::set_mode(&t, mode)?;
        t.sync_all()?;
        drop(t);
        #[cfg(test)]
        BEFORE_RECHECK.with(|h| {
            if let Some(f) = h.borrow_mut().take() {
                f();
            }
        });
        // Ön koşul: rename'den hemen önce yeniden oku.
        match &current {
            Some(c) => match read_current(&real)? {
                Some(now) if now.expect == c.expect => {}
                _ => return Err(ApplyError::Changed),
            },
            None => match std::fs::symlink_metadata(&real) {
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                _ => return Err(ApplyError::Changed),
            },
        }
        if current.is_some() {
            std::fs::rename(&tmp, &real)?;
        } else {
            // Yaratma: sabit bağ var olanın üzerine yazmaz (arada biri yarattıysa Changed).
            match std::fs::hard_link(&tmp, &real) {
                Ok(()) => {
                    let _ = std::fs::remove_file(&tmp);
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => return Err(ApplyError::Changed),
                // Sabit bağ desteklemeyen dosya sistemi: denetlenmiş rename.
                Err(_) => std::fs::rename(&tmp, &real)?,
            }
        }
        sys::sync_dir(&dir);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| Written::Written)
}

fn externally_if_denied(e: std::io::Error, real: &Path) -> ApplyError {
    match e.kind() {
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
            ApplyError::ExternallyManaged(real.to_path_buf())
        }
        k => ApplyError::Io(k),
    }
}

/// Dosyayı siler (ör. `ManagedFile`'ın `Absent` geri yüklemesi). Symlink ise silinmez
/// (`ExternallyManaged`). Dosya zaten yoksa `Unchanged`; `expect` uyuşmazsa `Changed`.
pub fn remove_file(env: &Env, path: &Path, expect: Option<&Expect>) -> Result<Written, ApplyError> {
    env.guard(TargetKind::File)?;
    check_shape(path)?;
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(ApplyError::ExternallyManaged(path.to_path_buf()));
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return match expect {
                Some(Expect::Present { .. }) => Err(ApplyError::Changed),
                _ => Ok(Written::Unchanged),
            };
        }
        Err(e) => return Err(e.into()),
    }
    let real = resolve(path)?;
    env.check_path(&real)?;
    check_managed_externally(&real)?;
    let Some(current) = read_current(&real)? else {
        return Ok(Written::Unchanged);
    };
    match expect {
        Some(Expect::Absent) => return Err(ApplyError::Changed),
        Some(e) if *e != current.expect => return Err(ApplyError::Changed),
        _ => {}
    }
    std::fs::remove_file(&real).map_err(|e| externally_if_denied(e, &real))?;
    if let Some(dir) = real.parent() {
        sys::sync_dir(dir);
    }
    Ok(Written::Written)
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::env::Skip;

    pub(crate) fn test_env(root: &Path) -> Env {
        Env {
            home: Some(root.join("home")),
            real_home: crate::env::real_home(),
            config_home: Some(root.join("cfg")),
            state_home: Some(root.join("state")),
            test_instance: true,
            hyprland: None,
            disabled: false,
        }
    }

    fn no_tmp_left(dir: &Path) -> bool {
        fs::read_dir(dir)
            .unwrap()
            .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp"))
    }

    #[test]
    fn write_preserves_mode_and_reports_unchanged() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("cfg/a.ini");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "a=1\n").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o640)).unwrap();
        let (text, exp) = read_file(&f).unwrap();
        assert_eq!(text, "a=1\n");
        assert_eq!(
            write_file(&env, &f, b"a=2\n", Some(&exp)).unwrap(),
            Written::Written
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "a=2\n");
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o7777,
            0o640
        );
        assert!(no_tmp_left(f.parent().unwrap()));
        // Aynı içerik: mtime oynamaz.
        let before = fs::metadata(&f).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(
            write_file(&env, &f, b"a=2\n", None).unwrap(),
            Written::Unchanged
        );
        assert_eq!(fs::metadata(&f).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn stale_expect_is_changed() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("cfg/a.ini");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "a=1\n").unwrap();
        let (_, exp) = read_file(&f).unwrap();
        // Aynı boyda dış değişiklik (yalnızca içerik özeti yakalar, mtime kaba olabilir).
        fs::write(&f, "a=9\n").unwrap();
        assert_eq!(
            write_file(&env, &f, b"a=2\n", Some(&exp)),
            Err(ApplyError::Changed)
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "a=9\n");
        assert!(no_tmp_left(f.parent().unwrap()));
        // Absent beklenirken dosya var.
        assert_eq!(
            write_file(&env, &f, b"x", Some(&Expect::Absent)),
            Err(ApplyError::Changed)
        );
        // Present beklenirken dosya silinmiş.
        let (_, exp) = read_file(&f).unwrap();
        fs::remove_file(&f).unwrap();
        assert_eq!(
            write_file(&env, &f, b"x", Some(&exp)),
            Err(ApplyError::Changed)
        );
        assert!(!f.exists());
    }

    #[test]
    fn change_between_read_and_rename_is_detected() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("cfg/race.ini");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        // Var olan dosya: yazım sırasında (aynı boyda) değişir.
        fs::write(&f, "a=1\n").unwrap();
        let g = f.clone();
        BEFORE_RECHECK
            .with(|h| *h.borrow_mut() = Some(Box::new(move || fs::write(&g, "a=7\n").unwrap())));
        assert_eq!(
            write_file(&env, &f, b"a=2\n", None),
            Err(ApplyError::Changed)
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "a=7\n");
        assert!(no_tmp_left(f.parent().unwrap()));
        // Yaratılacak dosyayı arada başkası yaratır.
        let n = t.path().join("cfg/new.ini");
        let g = n.clone();
        BEFORE_RECHECK
            .with(|h| *h.borrow_mut() = Some(Box::new(move || fs::write(&g, "dış").unwrap())));
        assert_eq!(
            write_file(&env, &n, b"biz", Some(&Expect::Absent)),
            Err(ApplyError::Changed)
        );
        assert_eq!(fs::read_to_string(&n).unwrap(), "dış");
        // Dosya arada symlink'le değiştirilir: kimlik (inode) farkı yakalanır.
        let s = t.path().join("cfg/s.ini");
        fs::write(&s, "a=1\n").unwrap();
        let other = t.path().join("cfg/other.ini");
        fs::write(&other, "a=1\n").unwrap();
        let (g, o) = (s.clone(), other.clone());
        BEFORE_RECHECK.with(|h| {
            *h.borrow_mut() = Some(Box::new(move || {
                fs::remove_file(&g).unwrap();
                std::os::unix::fs::symlink(&o, &g).unwrap();
            }))
        });
        assert_eq!(
            write_file(&env, &s, b"a=2\n", None),
            Err(ApplyError::Changed)
        );
        assert_eq!(fs::read_to_string(&other).unwrap(), "a=1\n");
        assert!(no_tmp_left(f.parent().unwrap()));
    }

    #[test]
    fn symlink_target_written_link_kept() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let real = t.path().join("cfg/dotfiles/settings.ini");
        fs::create_dir_all(real.parent().unwrap()).unwrap();
        fs::write(&real, "a=1\n").unwrap();
        let link = t.path().join("cfg/settings.ini");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let (_, exp) = read_file(&link).unwrap();
        assert_eq!(
            write_file(&env, &link, b"a=2\n", Some(&exp)).unwrap(),
            Written::Written
        );
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "a=2\n");
        // Kırık symlink: yerine dosya konmaz.
        let dangling = t.path().join("cfg/broken.ini");
        std::os::unix::fs::symlink(t.path().join("cfg/yok/yok.ini"), &dangling).unwrap();
        assert!(matches!(
            write_file(&env, &dangling, b"x", Some(&Expect::Absent)),
            Err(ApplyError::ExternallyManaged(_))
        ));
        assert!(
            fs::symlink_metadata(&dangling)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // Symlink silinmez.
        assert!(matches!(
            remove_file(&env, &link, None),
            Err(ApplyError::ExternallyManaged(_))
        ));
        assert!(real.exists());
    }

    #[test]
    fn symlink_escaping_test_root_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        // "Kullanıcının" dosyası: test kökleri dışında.
        let outside = t.path().join("home/.config/real.ini");
        fs::create_dir_all(outside.parent().unwrap()).unwrap();
        fs::write(&outside, "a=1\n").unwrap();
        fs::create_dir_all(t.path().join("cfg")).unwrap();
        let link = t.path().join("cfg/real.ini");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        assert_eq!(
            write_file(&env, &link, b"a=2\n", None),
            Err(ApplyError::Skipped(Skip::TestInstance))
        );
        assert_eq!(fs::read_to_string(&outside).unwrap(), "a=1\n");
        // Doğrudan dış yol da.
        assert_eq!(
            write_file(&env, &outside, b"a=2\n", None),
            Err(ApplyError::Skipped(Skip::TestInstance))
        );
        // Dizin symlink'i üzerinden yeni dosya.
        std::os::unix::fs::symlink(t.path().join("home/.config"), t.path().join("cfg/esc"))
            .unwrap();
        let via = t.path().join("cfg/esc/new.ini");
        assert_eq!(
            write_file(&env, &via, b"x", Some(&Expect::Absent)),
            Err(ApplyError::Skipped(Skip::TestInstance))
        );
        assert!(!t.path().join("home/.config/new.ini").exists());
    }

    #[test]
    fn size_limits() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("cfg/big.ini");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, vec![b'a'; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert!(matches!(read_file(&f), Err(ApplyError::TooLarge(_))));
        assert!(matches!(
            write_file(&env, &f, b"x", None),
            Err(ApplyError::TooLarge(_))
        ));
        assert_eq!(fs::metadata(&f).unwrap().len(), MAX_FILE_BYTES + 1);
        let g = t.path().join("cfg/g.ini");
        fs::write(&g, "a").unwrap();
        let big = vec![b'b'; (MAX_FILE_BYTES + 1) as usize];
        assert!(matches!(
            write_file(&env, &g, &big, None),
            Err(ApplyError::TooLarge(_))
        ));
        assert_eq!(fs::read_to_string(&g).unwrap(), "a");
    }

    #[test]
    fn missing_file_not_created_without_absent() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("cfg/hypr/hyprland.conf");
        assert!(matches!(
            write_file(&env, &f, b"x", None),
            Err(ApplyError::NotFound(_))
        ));
        assert!(!t.path().join("cfg/hypr").exists());
        // Absent ile yaratılır (dizinler dahil), yeni dosya 0644.
        assert_eq!(
            write_file(&env, &f, b"x", Some(&Expect::Absent)).unwrap(),
            Written::Written
        );
        assert_eq!(fs::read_to_string(&f).unwrap(), "x");
        assert_eq!(
            fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert!(no_tmp_left(f.parent().unwrap()));
        assert_eq!(fs::metadata(&f).unwrap().nlink(), 1);
    }

    use std::os::unix::fs::MetadataExt;

    #[test]
    fn externally_managed_cases() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let dir = t.path().join("cfg");
        fs::create_dir_all(&dir).unwrap();
        // Sahibine salt okunur.
        let ro = dir.join("ro.ini");
        fs::write(&ro, "a=1\n").unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(matches!(
            write_file(&env, &ro, b"a=2\n", None),
            Err(ApplyError::ExternallyManaged(_))
        ));
        assert_eq!(fs::read_to_string(&ro).unwrap(), "a=1\n");
        // Sabit bağ.
        let a = dir.join("a.ini");
        fs::write(&a, "a=1\n").unwrap();
        fs::hard_link(&a, dir.join("b.ini")).unwrap();
        assert!(matches!(
            write_file(&env, &a, b"a=2\n", None),
            Err(ApplyError::ExternallyManaged(_))
        ));
        // Yazılamayan dizin (root değilsek anlamlı).
        if rustix::process::geteuid().as_raw() != 0 {
            let locked = dir.join("locked");
            fs::create_dir(&locked).unwrap();
            let f = locked.join("x.ini");
            fs::write(&f, "a=1\n").unwrap();
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
            let r = write_file(&env, &f, b"a=2\n", None);
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(matches!(r, Err(ApplyError::ExternallyManaged(_))), "{r:?}");
            assert_eq!(fs::read_to_string(&f).unwrap(), "a=1\n");
            assert!(no_tmp_left(&locked));
        }
        // Düz dosya değil.
        assert_eq!(
            write_file(&env, &dir, b"x", None),
            Err(ApplyError::Invalid("hedef düz dosya değil"))
        );
    }

    #[test]
    fn bad_paths_and_guard() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        assert_eq!(
            write_file(&env, Path::new("rel.ini"), b"x", None),
            Err(ApplyError::Invalid("yol mutlak değil"))
        );
        let dotdot = t.path().join("cfg/../cfg/a.ini");
        assert!(matches!(
            write_file(&env, &dotdot, b"x", None),
            Err(ApplyError::Invalid(_))
        ));
        let mut off = env.clone();
        off.disabled = true;
        let f = t.path().join("cfg/a.ini");
        assert_eq!(
            write_file(&off, &f, b"x", Some(&Expect::Absent)),
            Err(ApplyError::Skipped(Skip::Disabled))
        );
        assert!(!f.exists());
        // Test örneği kullanıcının config'iyle: kapı.
        let mut user_cfg = env.clone();
        user_cfg.config_home = Some(t.path().join("home/.config"));
        assert_eq!(
            write_file(&user_cfg, &f, b"x", Some(&Expect::Absent)),
            Err(ApplyError::Skipped(Skip::TestInstance))
        );
    }

    #[test]
    fn remove_respects_expect() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("cfg/m.conf");
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(&f, "x").unwrap();
        let (_, exp) = read_file(&f).unwrap();
        fs::write(&f, "y").unwrap();
        assert_eq!(remove_file(&env, &f, Some(&exp)), Err(ApplyError::Changed));
        let (_, exp) = read_file(&f).unwrap();
        assert_eq!(remove_file(&env, &f, Some(&exp)), Ok(Written::Written));
        assert!(!f.exists());
        assert_eq!(remove_file(&env, &f, None), Ok(Written::Unchanged));
    }

    #[test]
    fn non_utf8_is_parse_error() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("x");
        fs::write(&f, [0xff, 0xfe]).unwrap();
        assert!(matches!(read_file(&f), Err(ApplyError::Parse(_))));
        assert!(read_bytes(&f).is_ok());
    }

    #[test]
    fn write_file_with_private_modes() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let f = t.path().join("state/nuxut/a/b/durum.toml");
        let r = write_file_with(
            &env,
            &f,
            b"x",
            Some(&Expect::Absent),
            &WriteOptions::PRIVATE,
        );
        assert_eq!(r.unwrap(), Written::Written);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&f), 0o600);
        // Yaratılan bütün atalar 0700 (umask 0700'ü daraltmaz).
        for d in ["state", "state/nuxut", "state/nuxut/a", "state/nuxut/a/b"] {
            assert_eq!(mode(&t.path().join(d)), 0o700, "{d}");
        }
        // Var olan dizinin iznine dokunulmaz.
        let open = t.path().join("cfg/open");
        fs::create_dir_all(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o755)).unwrap();
        write_file_with(
            &env,
            &open.join("x"),
            b"x",
            Some(&Expect::Absent),
            &WriteOptions::PRIVATE,
        )
        .unwrap();
        assert_eq!(mode(&open), 0o755);
        // Varsayılan seçenekler write_file ile aynı.
        let g = t.path().join("cfg/new/g.ini");
        write_file_with(
            &env,
            &g,
            b"x",
            Some(&Expect::Absent),
            &WriteOptions::default(),
        )
        .unwrap();
        assert_eq!(mode(&g), 0o644);
        assert_eq!(mode(g.parent().unwrap()), 0o755);
        // Özel bitler reddedilir, iz yok.
        for opts in [
            WriteOptions {
                mode: Some(0o4755),
                dir_mode: 0o700,
            },
            WriteOptions {
                mode: None,
                dir_mode: 0o1777,
            },
        ] {
            let h = t.path().join("cfg/bad/h");
            assert!(matches!(
                write_file_with(&env, &h, b"x", Some(&Expect::Absent), &opts),
                Err(ApplyError::Invalid(_))
            ));
            assert!(!t.path().join("cfg/bad").exists());
        }
    }

    #[test]
    fn ensure_dir_cases() {
        let t = tempfile::tempdir().unwrap();
        let env = test_env(t.path());
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let d = t.path().join("state/nuxut/özel");
        assert_eq!(ensure_dir(&env, &d, 0o700).unwrap(), Written::Written);
        assert_eq!(mode(&d), 0o700);
        assert_eq!(mode(&t.path().join("state")), 0o700);
        // Var: dokunulmaz.
        fs::set_permissions(&d, fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(ensure_dir(&env, &d, 0o700).unwrap(), Written::Unchanged);
        assert_eq!(mode(&d), 0o750);
        // Yol bir dosya.
        let file = t.path().join("state/dosya");
        fs::write(&file, "x").unwrap();
        assert!(matches!(
            ensure_dir(&env, &file, 0o700),
            Err(ApplyError::ExternallyManaged(_))
        ));
        // Kırık symlink: hedefi yaratılmaz.
        let link = t.path().join("state/kirik");
        std::os::unix::fs::symlink(t.path().join("state/yok"), &link).unwrap();
        assert!(matches!(
            ensure_dir(&env, &link, 0o700),
            Err(ApplyError::ExternallyManaged(_))
        ));
        assert!(!t.path().join("state/yok").exists());
        // Symlink'ten test kökü dışına: kapı.
        let outside = t.path().join("dışarı");
        fs::create_dir_all(&outside).unwrap();
        let via = t.path().join("state/via");
        std::os::unix::fs::symlink(&outside, &via).unwrap();
        assert_eq!(
            ensure_dir(&env, &via.join("x"), 0o700),
            Err(ApplyError::Skipped(Skip::TestInstance))
        );
        assert!(!outside.join("x").exists());
        // Test kökü dışı, göreli yol, '..', özel bitler.
        assert_eq!(
            ensure_dir(&env, &t.path().join("başka/x"), 0o700),
            Err(ApplyError::Skipped(Skip::TestInstance))
        );
        assert!(matches!(
            ensure_dir(&env, Path::new("göreli/x"), 0o700),
            Err(ApplyError::Invalid(_))
        ));
        assert!(matches!(
            ensure_dir(&env, &t.path().join("state/../state/y"), 0o700),
            Err(ApplyError::Invalid(_))
        ));
        assert!(matches!(
            ensure_dir(&env, &t.path().join("state/z"), 0o2700),
            Err(ApplyError::Invalid(_))
        ));
        assert!(!t.path().join("state/z").exists());
    }
}
