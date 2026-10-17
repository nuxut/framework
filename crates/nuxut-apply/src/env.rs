//! Ortam kapısı: yazım bu süreçte güvenli mi?
//!
//! Shell `SyncEnv::guard`'ının (`hypr_config` c7a354e) genel hâli. Kural: test/ikincil örnek
//! (`NUXUT_SHELL_INSTANCE`, `NUXUT_SETTINGS_INSTANCE`, … — `NUXUT_*_INSTANCE` ailesi ya da
//! çağıranın açıkça verdiği örnek adı) kullanıcının dosyalarına, dconf'una ve systemd birimlerine
//! **asla** yazmaz; dosya hedefleri yalnızca açıkça verilmiş, kullanıcının **gerçek** ev dizininin
//! (`getpwuid`, `$HOME` değil) dışında kalan mutlak bir `XDG_CONFIG_HOME`/`XDG_STATE_HOME`
//! altında olabilir.
//!
//! Testlerde [`Env::from_process`] kullanılmaz: [`Env::for_test`] (test örneği, geçici kökler).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::ApplyError;

/// Shell'in test/ikincil örnek değişkeni (shell `ipc::INSTANCE_ENV` ile aynı ad). Aynı ailedeki
/// her `NUXUT_<UYGULAMA>_INSTANCE` de tanınır.
pub const INSTANCE_ENV: &str = "NUXUT_SHELL_INSTANCE";

/// `NUXUT_<X>_INSTANCE` biçiminde mi?
fn is_instance_var(name: &str) -> bool {
    name.strip_prefix("NUXUT_")
        .and_then(|r| r.strip_suffix("_INSTANCE"))
        .is_some_and(|mid| {
            !mid.is_empty()
                && mid
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// Hedef türü (kapı kararı buna göre).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// Düz dosya (TOML/INI/satır/blok/yönetilen dosya).
    File,
    /// Hyprland dosyaları/denetimi: geçerli bir Hyprland oturumu gerekir; dosya kuralları da
    /// geçerli. [`crate::Applier`] parçalardan bu türü çıkaramaz (bir `hyprland.conf` parçası
    /// düz dosyadır): Hyprland hedefi kuran çağıran `guard(TargetKind::Hyprland)` /
    /// [`Env::may_control_hyprland`]'i kendisi sorar.
    Hyprland,
    /// dconf anahtarı (test örneğinde hiç yazılmaz).
    Dconf,
    /// systemd `--user` birim durumu (test örneğinde hiç yazılmaz).
    Unit,
}

/// Yazımın atlanma nedeni (hata değil; IPC'de `skipped(<ad>)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Skip {
    /// Test/ikincil örnek: bu hedefe yazılmaz.
    TestInstance,
    /// Gerekli oturum yok (Hyprland imzası, ev dizini …).
    NoSession,
    /// Çağıran uygulamayı kapattı ([`Env::disabled`]).
    Disabled,
}

impl Skip {
    pub fn name(self) -> &'static str {
        match self {
            Skip::TestInstance => "test_instance",
            Skip::NoSession => "no_session",
            Skip::Disabled => "disabled",
        }
    }
}

/// Süreç ortamı. Üretimde [`Env::from_process`], testte [`Env::for_test`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `$HOME` (yalnızca yol kısaltma `~` için; kapı ona güvenmez).
    pub home: Option<PathBuf>,
    /// Kullanıcının gerçek ev dizini (`getpwuid(geteuid())`). Test örneği kapısı bunu korur;
    /// bilinmiyorsa test örneği hiçbir dosyaya yazmaz.
    pub real_home: Option<PathBuf>,
    /// Mutlak `$XDG_CONFIG_HOME`, yoksa `$HOME/.config`.
    pub config_home: Option<PathBuf>,
    /// Mutlak `$XDG_STATE_HOME`, yoksa `$HOME/.local/state` (snapshot deposu burada).
    pub state_home: Option<PathBuf>,
    /// Test/ikincil örnek (`NUXUT_*_INSTANCE` ya da çağıranın verdiği örnek adı).
    pub test_instance: bool,
    /// `HYPRLAND_INSTANCE_SIGNATURE` (boş değilse).
    pub hyprland: Option<String>,
    /// Çağıran yazımı kapattı (ör. ayarla); her hedef `Skip::Disabled`.
    pub disabled: bool,
}

impl Env {
    /// Ortam değişkenlerinden. Göreli `XDG_*` yok sayılır (XDG kuralı).
    ///
    /// `instance`: çağıranın kendi test/ikincil örnek adı (ör. Ayarlar'ın
    /// `NUXUT_SETTINGS_INSTANCE`'tan okuduğu); `Some` (boş değil) ise test örneğidir. Ayrıca
    /// ortamdaki boş olmayan her `NUXUT_<X>_INSTANCE` de test örneği yapar (fail-closed).
    pub fn from_process(instance: Option<&str>) -> Self {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let abs = |k: &str| var(k).filter(|p| p.is_absolute());
        let home = abs("HOME");
        let family = std::env::vars_os()
            .any(|(k, v)| !v.is_empty() && k.to_str().is_some_and(is_instance_var));
        Self {
            config_home: abs("XDG_CONFIG_HOME")
                .or_else(|| home.as_ref().map(|h| h.join(".config"))),
            state_home: abs("XDG_STATE_HOME")
                .or_else(|| home.as_ref().map(|h| h.join(".local/state"))),
            test_instance: instance.is_some_and(|i| !i.is_empty()) || family,
            hyprland: std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
                .ok()
                .filter(|s| !s.is_empty()),
            real_home: real_home(),
            home,
            disabled: false,
        }
    }

    /// Test ortamı: test örneği; `root/home`, `root/config`, `root/state`; gerçek ev dizini
    /// korunur (kapı gerçek kurallarla çalışır); Hyprland yok. Bu `Env` ile kurulan
    /// [`crate::Applier`] dconf/systemd için hiçbir program başlatmaz ([`crate::NoBackend`]).
    /// `root` gerçek ev dizini dışında olmalı (ör. `tempfile::tempdir()`), yoksa her yazım
    /// `Skipped(TestInstance)`.
    pub fn for_test(root: &Path) -> Self {
        Self {
            home: Some(root.join("home")),
            real_home: real_home(),
            config_home: Some(root.join("config")),
            state_home: Some(root.join("state")),
            test_instance: true,
            hyprland: None,
            disabled: false,
        }
    }

    /// Bu türden bir hedefe yazılabilir mi? `Err` ise hiçbir dosya açılmaz.
    pub fn guard(&self, kind: TargetKind) -> Result<(), Skip> {
        if self.disabled {
            return Err(Skip::Disabled);
        }
        match kind {
            TargetKind::Dconf | TargetKind::Unit => {
                if self.test_instance {
                    return Err(Skip::TestInstance);
                }
                Ok(())
            }
            TargetKind::Hyprland => {
                let sig = self.hyprland.as_deref().ok_or(Skip::NoSession)?;
                if sig.len() > 255
                    || !sig
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err(Skip::NoSession);
                }
                self.guard_files()
            }
            TargetKind::File => self.guard_files(),
        }
    }

    /// Test örneğinde config ve state kökleri: mutlak, kanonik biçimleri gerçek ev dizinine eşit,
    /// onun atası (`/`, `/home`) ya da içinde (`~/.config`, `~/.local/state`, `~/x`) olamaz.
    /// `$HOME`'a güvenilmez (taklit edilebilir); gerçek ev bilinmiyorsa hiçbir şey.
    fn guard_files(&self) -> Result<(), Skip> {
        self.test_roots().map(|_| ())
    }

    /// Test örneğinde izinli köklerin kanonik biçimleri (`None` = test örneği değil).
    fn test_roots(&self) -> Result<Option<[PathBuf; 2]>, Skip> {
        if !self.test_instance {
            return Ok(None);
        }
        let real_home = self
            .real_home
            .as_deref()
            .filter(|h| h.is_absolute())
            .and_then(canonical)
            .ok_or(Skip::TestInstance)?;
        let root = |r: Option<&Path>| -> Result<PathBuf, Skip> {
            let r = r
                .filter(|r| r.is_absolute())
                .and_then(canonical)
                .ok_or(Skip::TestInstance)?;
            if r.starts_with(&real_home) || real_home.starts_with(&r) {
                Err(Skip::TestInstance)
            } else {
                Ok(r)
            }
        };
        Ok(Some([
            root(self.config_home.as_deref())?,
            root(self.state_home.as_deref())?,
        ]))
    }

    /// Hyprland'e canlı komut gönderilebilir mi (test örneği asla).
    pub fn may_control_hyprland(&self) -> bool {
        !self.test_instance && self.guard(TargetKind::Hyprland).is_ok()
    }

    /// Kanonik yazım hedefi bu ortamda izinli mi? Test örneğinde yalnızca (kanonik)
    /// `config_home` ya da `state_home` altı.
    pub(crate) fn check_path(&self, real: &Path) -> Result<(), ApplyError> {
        match self.test_roots()? {
            None => Ok(()),
            Some(roots) if roots.iter().any(|r| real.starts_with(r)) => Ok(()),
            Some(_) => Err(ApplyError::Skipped(Skip::TestInstance)),
        }
    }

    /// Snapshot deposu: `$XDG_STATE_HOME/nuxut/snapshots`.
    pub fn snapshot_dir(&self) -> Option<PathBuf> {
        self.nuxut_state_dir().map(|d| d.join("snapshots"))
    }

    /// `$XDG_STATE_HOME/nuxut`.
    pub fn nuxut_state_dir(&self) -> Option<PathBuf> {
        self.state_home
            .as_ref()
            .filter(|p| p.is_absolute())
            .map(|s| s.join("nuxut"))
    }

    /// Kullanıcıya gösterilecek yol: ev dizini altındaysa `~/…`.
    pub fn display_path(&self, path: &Path) -> String {
        match self.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
            Some(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
            _ => path.display().to_string(),
        }
    }
}

/// Kanonik yol: varsa `canonicalize`, yoksa en yakın var olan ata kanonikleştirilip kalan
/// bileşenler eklenir (`..` içeren ya da çözülemeyen yol → `None`).
fn canonical(p: &Path) -> Option<PathBuf> {
    crate::write::resolve(p).ok()
}

/// `getpwuid_r(geteuid())` ev dizini (NSS dahil); `$HOME`'dan bağımsız.
pub(crate) fn real_home() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        nix::unistd::User::from_uid(nix::unistd::Uid::effective())
            .ok()
            .flatten()
            .map(|u| u.dir)
            .filter(|d| d.is_absolute())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(test: bool, cfg: &str, state: &str) -> Env {
        Env {
            home: Some("/home/u".into()),
            real_home: Some("/home/u".into()),
            config_home: Some(cfg.into()),
            state_home: Some(state.into()),
            test_instance: test,
            hyprland: Some("abc_123".into()),
            disabled: false,
        }
    }

    #[test]
    fn guard_table() {
        let user = env(false, "/home/u/.config", "/home/u/.local/state");
        for k in [
            TargetKind::File,
            TargetKind::Hyprland,
            TargetKind::Dconf,
            TargetKind::Unit,
        ] {
            assert_eq!(user.guard(k), Ok(()));
        }
        // Test örneği kullanıcının dizinleriyle, onların atasıyla ya da ev içinde: hiçbir şey.
        for (cfg, state) in [
            ("/home/u/.config", "/tmp/s"),
            ("/tmp/c", "/home/u/.local/state"),
            ("/home/u", "/tmp/s"),
            ("/", "/tmp/s"),
            ("/home", "/tmp/s"),
            ("/tmp/c", "/home/u/projects/x"),
            ("/tmp/c", "/home/u/.config/../x"),
            ("rel/c", "/tmp/s"),
        ] {
            assert_eq!(
                env(true, cfg, state).guard(TargetKind::File),
                Err(Skip::TestInstance),
                "{cfg} {state}"
            );
        }
        // Gerçek ev bilinmiyorsa test örneği yazmaz.
        let mut no_home = env(true, "/tmp/c", "/tmp/s");
        no_home.real_home = None;
        assert_eq!(no_home.guard(TargetKind::File), Err(Skip::TestInstance));
        // $HOME taklidi kapıyı aşamaz: HOME=/tmp/h, XDG_CONFIG_HOME gerçek ~/.config.
        let mut spoof = env(true, "/home/u/.config", "/tmp/s");
        spoof.home = Some("/tmp/h".into());
        assert_eq!(spoof.guard(TargetKind::File), Err(Skip::TestInstance));
        // Geçici dizinlerle dosyalar evet; dconf/systemd asla.
        let test = env(true, "/tmp/c", "/tmp/s");
        assert_eq!(test.guard(TargetKind::File), Ok(()));
        assert_eq!(test.guard(TargetKind::Dconf), Err(Skip::TestInstance));
        assert_eq!(test.guard(TargetKind::Unit), Err(Skip::TestInstance));
        assert!(!test.may_control_hyprland());
        assert!(user.may_control_hyprland());
        // Hyprland imzası.
        let mut no_sig = user.clone();
        no_sig.hyprland = None;
        assert_eq!(no_sig.guard(TargetKind::Hyprland), Err(Skip::NoSession));
        no_sig.hyprland = Some("../x".into());
        assert_eq!(no_sig.guard(TargetKind::Hyprland), Err(Skip::NoSession));
        assert_eq!(no_sig.guard(TargetKind::File), Ok(()));
        // Kapalı.
        let mut off = user.clone();
        off.disabled = true;
        assert_eq!(off.guard(TargetKind::File), Err(Skip::Disabled));
    }

    #[test]
    fn instance_family_and_explicit_instance() {
        for ok in [
            "NUXUT_SHELL_INSTANCE",
            "NUXUT_SETTINGS_INSTANCE",
            "NUXUT_CALENDAR_2_INSTANCE",
        ] {
            assert!(is_instance_var(ok), "{ok}");
        }
        for bad in [
            "NUXUT_INSTANCE",
            "NUXUT__INSTANCE_X",
            "nuxut_shell_instance",
            "NUXUT_SHELL",
            "X_NUXUT_SHELL_INSTANCE",
        ] {
            assert!(!is_instance_var(bad), "{bad}");
        }
        assert!(Env::from_process(Some("test-apply")).test_instance);
    }

    #[test]
    fn for_test_is_a_test_instance_outside_real_home() {
        let t = tempfile::tempdir().unwrap();
        let e = Env::for_test(t.path());
        assert!(e.test_instance && e.hyprland.is_none());
        assert_eq!(e.guard(TargetKind::Dconf), Err(Skip::TestInstance));
        // Geçici dizin gerçek evin dışındaysa dosyalar izinli.
        let home = real_home().and_then(|h| canonical(&h));
        let tmp = canonical(t.path()).unwrap();
        if home.is_some_and(|h| !tmp.starts_with(&h)) {
            assert_eq!(e.guard(TargetKind::File), Ok(()));
        }
        // Gerçek evin içinde kurulan test kökleri reddedilir.
        if let Some(h) = real_home() {
            let inside = Env::for_test(&h.join("nuxut-apply-test-yok"));
            assert_eq!(inside.guard(TargetKind::File), Err(Skip::TestInstance));
        }
    }

    #[test]
    fn check_path_in_test_instance() {
        let test = env(true, "/tmp/nonexistent-c", "/tmp/nonexistent-s");
        assert!(
            test.check_path(Path::new("/tmp/nonexistent-c/gtk-3.0/settings.ini"))
                .is_ok()
        );
        assert!(
            test.check_path(Path::new("/tmp/nonexistent-s/nuxut/x"))
                .is_ok()
        );
        assert!(matches!(
            test.check_path(Path::new("/home/u/.config/gtk-3.0/settings.ini")),
            Err(ApplyError::Skipped(Skip::TestInstance))
        ));
        // Kök `/` ise her yol "altında" olurdu: kapı reddeder.
        let root = env(true, "/", "/tmp/nonexistent-s");
        assert!(root.check_path(Path::new("/etc/x")).is_err());
        let user = env(false, "/home/u/.config", "/home/u/.local/state");
        assert!(user.check_path(Path::new("/etc/x")).is_ok());
    }

    #[test]
    fn display_shortens_home() {
        let e = env(false, "/home/u/.config", "/s");
        assert_eq!(
            e.display_path(Path::new("/home/u/.config/gtk-3.0/settings.ini")),
            "~/.config/gtk-3.0/settings.ini"
        );
        assert_eq!(e.display_path(Path::new("/etc/x")), "/etc/x");
        assert_eq!(e.display_path(Path::new("/home/u")), "/home/u");
    }
}
