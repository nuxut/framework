//! İşçi ikilisinin yeri (ADR-0007 K3). Sıra:
//! 1. `NUXUT_IMAGE_WORKER` ortam değişkeni (geliştirme/test),
//! 2. çalışan ikiliye göreli: aynı dizin, sonra `../lib/nuxut/`,
//! 3. `~/.local/lib/nuxut/` (kullanıcı kurulumu, `install.sh`),
//! 4. `/usr/lib/nuxut/` (dağıtım paketi).
//!
//! Aday düzenli, çalıştırılabilir bir dosya olmalı; ilk uyan seçilir.

use std::path::{Path, PathBuf};

pub const WORKER_NAME: &str = "nuxut-image-worker";
pub const ENV_VAR: &str = "NUXUT_IMAGE_WORKER";

/// Aday yollar, öncelik sırasıyla (var olup olmadıklarına bakılmaz).
pub fn candidates(env: Option<PathBuf>, exe: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(p) = env.filter(|p| !p.as_os_str().is_empty()) {
        out.push(p);
    }
    if let Some(dir) = exe.and_then(Path::parent) {
        out.push(dir.join(WORKER_NAME));
        if let Some(prefix) = dir.parent() {
            out.push(prefix.join("lib/nuxut").join(WORKER_NAME));
        }
    }
    if let Some(home) = home {
        out.push(home.join(".local/lib/nuxut").join(WORKER_NAME));
    }
    out.push(Path::new("/usr/lib/nuxut").join(WORKER_NAME));
    out
}

fn is_executable_file(p: &Path) -> bool {
    match std::fs::metadata(p) {
        #[cfg(unix)]
        Ok(m) => {
            use std::os::unix::fs::PermissionsExt;
            m.is_file() && m.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        Ok(m) => m.is_file(),
        Err(_) => false,
    }
}

/// Ortamdan ve çalışan ikiliden işçiyi bulur.
pub fn locate() -> Option<PathBuf> {
    let env = std::env::var_os(ENV_VAR).map(PathBuf::from);
    let exe = std::env::current_exe().ok();
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute());
    // Ortam değişkeni verilmişse yalnızca o denenir: yanlış yol sessizce başka ikiliye düşmesin.
    if let Some(p) = env.filter(|p| !p.as_os_str().is_empty()) {
        return is_executable_file(&p).then_some(p);
    }
    candidates(None, exe.as_deref(), home.as_deref()).into_iter().find(|p| is_executable_file(p))
}

/// Açıkça verilmiş yolu doğrular.
pub fn check(p: &Path) -> Option<PathBuf> {
    is_executable_file(p).then(|| p.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_order() {
        let c = candidates(Some("/opt/w".into()), Some(Path::new("/usr/bin/nuxut-shell")), Some(Path::new("/home/u")));
        let c: Vec<_> = c.iter().map(|p| p.to_str().unwrap_or_default()).collect();
        assert_eq!(
            c,
            [
                "/opt/w",
                "/usr/bin/nuxut-image-worker",
                "/usr/lib/nuxut/nuxut-image-worker",
                "/home/u/.local/lib/nuxut/nuxut-image-worker",
                "/usr/lib/nuxut/nuxut-image-worker",
            ]
        );
        let c = candidates(None, Some(Path::new("/home/u/.local/bin/nuxut-shell")), None);
        assert_eq!(c[0], Path::new("/home/u/.local/bin/nuxut-image-worker"));
        assert_eq!(c[1], Path::new("/home/u/.local/lib/nuxut/nuxut-image-worker"));
    }

    #[cfg(unix)]
    #[test]
    fn check_requires_executable_regular_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("w");
        std::fs::write(&f, b"#!/bin/sh\n").unwrap();
        assert!(check(&f).is_none());
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(check(&f).is_some());
        assert!(check(dir.path()).is_none(), "dizin değil");
        assert!(check(&dir.path().join("yok")).is_none());
    }
}
