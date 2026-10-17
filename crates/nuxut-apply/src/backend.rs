//! Dosya dışı hedefler (ADR-0001 §5): dconf anahtarları ve systemd `--user` birim durumu.
//!
//! - Alt süreç, **sabit argv, kabuk yok**; stdin kapalı, stderr atılır; zaman aşımı (dconf 2 s,
//!   systemctl 5 s — `enable` örtük `daemon-reload` yapar) → çocuk öldürülür, `Io(TimedOut)`;
//!   çıktı ≤ 64 KiB.
//! - Her çağrıda önce [`Env::guard`]: test örneğinde ([`crate::INSTANCE_ENV`]) dconf/systemd'ye
//!   **hiç** dokunulmaz (`Skipped(TestInstance)`), program bile başlatılmaz.
//! - Program yoksa `Unavailable("dconf" | "systemctl")`.
//! - Crate'in kendi test derlemesinde varsayılan program yolları var olmayan bir yerdir: birim
//!   testleri yanlışlıkla gerçek oturumun dconf/systemd'sine ulaşamaz (sahte program yolu
//!   [`SystemBackend::with_programs`] ile verilir).
//!
//! `dconf` seçimi (`gsettings` değil): şema kurulu olmasa da çalışır ve "hiç ayarlanmamış"
//! (`dconf read` boş → `Absent`) ile "varsayılana eşit" ayrımını korur; `Absent` geri yüklemesi
//! `dconf reset`.

use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::env::{Env, TargetKind};
use crate::error::ApplyError;
use crate::model::{FragValue, Fragment};

/// Alt süreç çıktı sınırı.
const MAX_OUTPUT: u64 = 64 * 1024;
/// `dconf` zaman aşımı (ADR §5).
pub const DCONF_TIMEOUT: Duration = Duration::from_secs(2);
/// `systemctl --user` zaman aşımı (`enable`/`disable` örtük daemon-reload yapar).
pub const SYSTEMCTL_TIMEOUT: Duration = Duration::from_secs(5);

/// Dosya dışı parça (dconf, birim) okuyup yazan arka uç. Shell/Ayarlar testleri sahtesini verir
/// ([`crate::Applier::with_backend`]).
pub trait Backend: Send + Sync {
    /// Şu anki değer (`Absent` = ayarlanmamış / birim yok).
    fn read(&self, f: &Fragment) -> Result<FragValue, ApplyError>;
    /// Değeri yazar (`Absent` = sıfırla / devre dışı).
    fn write(&self, f: &Fragment, v: &FragValue) -> Result<(), ApplyError>;
}

/// Hiçbir dış hedefi desteklemeyen arka uç: her şey `Unavailable`.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBackend;

impl Backend for NoBackend {
    fn read(&self, f: &Fragment) -> Result<FragValue, ApplyError> {
        Err(ApplyError::Unavailable(f.kind_name().into()))
    }
    fn write(&self, f: &Fragment, _: &FragValue) -> Result<(), ApplyError> {
        Err(ApplyError::Unavailable(f.kind_name().into()))
    }
}

/// Gerçek arka uç: `dconf` ve `systemctl --user` alt süreçleri.
#[derive(Debug, Clone)]
pub struct SystemBackend {
    env: Env,
    dconf: PathBuf,
    systemctl: PathBuf,
    /// Üründe her zaman [`DCONF_TIMEOUT`] / [`SYSTEMCTL_TIMEOUT`]; yalnızca crate'in kendi
    /// testleri sahte programlar için geniş süre verir (bkz. `with_test_timeouts`).
    dconf_timeout: Duration,
    systemctl_timeout: Duration,
}

impl SystemBackend {
    /// `PATH`'teki `dconf` ve `systemctl`.
    pub fn new(env: &Env) -> Self {
        #[cfg(not(test))]
        let (dconf, systemctl) = (PathBuf::from("dconf"), PathBuf::from("systemctl"));
        #[cfg(test)]
        let (dconf, systemctl) = (
            PathBuf::from("/nonexistent/nuxut-apply-test/dconf"),
            PathBuf::from("/nonexistent/nuxut-apply-test/systemctl"),
        );
        Self::with_programs(env, dconf, systemctl)
    }

    /// Program yolları elle (testler: sahte betikler).
    pub fn with_programs(env: &Env, dconf: PathBuf, systemctl: PathBuf) -> Self {
        Self {
            env: env.clone(),
            dconf,
            systemctl,
            dconf_timeout: DCONF_TIMEOUT,
            systemctl_timeout: SYSTEMCTL_TIMEOUT,
        }
    }

    /// Yalnızca testler: sahte `dconf`/`systemctl` betikleri birkaç `fork` yapar; yüklü ya da
    /// bellek bekçisince dondurulmuş test sürecinde 2 sn'yi aşıp `TimedOut` veriyordu (testin
    /// sınadığı davranışla ilgisiz). Zaman aşımı yolunun kendisini `timeout_and_output_limit` sınar.
    #[cfg(test)]
    fn with_test_timeouts(mut self, t: Duration) -> Self {
        self.dconf_timeout = t;
        self.systemctl_timeout = t;
        self
    }

    fn dconf_read(&self, key: &str) -> Result<FragValue, ApplyError> {
        let out = run(&self.dconf, "dconf", &["read", key], self.dconf_timeout)?;
        if !out.ok {
            return Err(ApplyError::Io(ErrorKind::Other));
        }
        let text =
            String::from_utf8(out.stdout).map_err(|_| ApplyError::Io(ErrorKind::InvalidData))?;
        let value = text.strip_suffix('\n').unwrap_or(&text);
        if value.is_empty() {
            return Ok(FragValue::Absent);
        }
        if value.contains(['\n', '\r', '\0']) {
            return Err(ApplyError::Io(ErrorKind::InvalidData));
        }
        Ok(FragValue::Text(value.to_owned()))
    }

    fn dconf_write(&self, key: &str, v: &FragValue) -> Result<(), ApplyError> {
        let out = match v {
            FragValue::Absent => run(&self.dconf, "dconf", &["reset", key], self.dconf_timeout)?,
            FragValue::Text(t) => {
                run(&self.dconf, "dconf", &["write", key, t], self.dconf_timeout)?
            }
            FragValue::File { .. } => {
                return Err(ApplyError::Invalid("değer parça türüne uymuyor"));
            }
        };
        if out.ok {
            Ok(())
        } else {
            // dconf geçersiz GVariant metnini reddeder.
            Err(ApplyError::Invalid("dconf değeri reddetti"))
        }
    }

    fn unit_read(&self, unit: &str) -> Result<FragValue, ApplyError> {
        let out = run(
            &self.systemctl,
            "systemctl",
            &["--user", "is-enabled", "--", unit],
            self.systemctl_timeout,
        )?;
        let text = String::from_utf8_lossy(&out.stdout);
        match text.lines().next().unwrap_or("").trim() {
            "enabled" | "enabled-runtime" => Ok(FragValue::Text("enabled".into())),
            "disabled" => Ok(FragValue::Text("disabled".into())),
            // Birim dosyası yok (systemd ≥ 245 `not-found`; eskileri boş çıktı + hata kodu).
            "not-found" | "" if !out.ok => Ok(FragValue::Absent),
            // static, masked, linked, alias, indirect, generated, transient …: Nuxut bunları geri
            // yükleyemez; dokunulmaz.
            _ => Err(ApplyError::ExternallyManaged(PathBuf::from(unit))),
        }
    }

    fn unit_write(&self, unit: &str, v: &FragValue) -> Result<(), ApplyError> {
        let verb = match v {
            FragValue::Text(t) if t == "enabled" => "enable",
            FragValue::Text(t) if t == "disabled" => "disable",
            FragValue::Absent => {
                // Birim yoksa ya da zaten kapalıysa yapılacak bir şey yok.
                match self.unit_read(unit)? {
                    FragValue::Text(t) if t == "enabled" => "disable",
                    _ => return Ok(()),
                }
            }
            _ => return Err(ApplyError::Invalid("değer parça türüne uymuyor")),
        };
        let out = run(
            &self.systemctl,
            "systemctl",
            &["--user", verb, "--", unit],
            self.systemctl_timeout,
        )?;
        if out.ok {
            Ok(())
        } else {
            Err(ApplyError::Io(ErrorKind::Other))
        }
    }
}

impl Backend for SystemBackend {
    fn read(&self, f: &Fragment) -> Result<FragValue, ApplyError> {
        f.validate()?;
        match f {
            Fragment::Dconf { key } => {
                self.env.guard(TargetKind::Dconf)?;
                self.dconf_read(key)
            }
            Fragment::UnitState { unit } => {
                self.env.guard(TargetKind::Unit)?;
                self.unit_read(unit)
            }
            _ => Err(ApplyError::Invalid("dosya parçası arka uca verilmez")),
        }
    }

    fn write(&self, f: &Fragment, v: &FragValue) -> Result<(), ApplyError> {
        f.validate()?;
        crate::model::value_fits(f, v)?;
        match f {
            Fragment::Dconf { key } => {
                self.env.guard(TargetKind::Dconf)?;
                self.dconf_write(key, v)
            }
            Fragment::UnitState { unit } => {
                self.env.guard(TargetKind::Unit)?;
                self.unit_write(unit, v)
            }
            _ => Err(ApplyError::Invalid("dosya parçası arka uca verilmez")),
        }
    }
}

struct Output {
    ok: bool,
    stdout: Vec<u8>,
}

/// Programı sabit argv ile çalıştırır; zaman aşımında öldürür.
fn run(program: &Path, name: &str, args: &[&str], timeout: Duration) -> Result<Output, ApplyError> {
    #[cfg(test)]
    crate::test_gate::assert_spawn_allowed();
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                ApplyError::Unavailable(name.to_owned())
            }
            k => ApplyError::Io(k),
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or(ApplyError::Io(ErrorKind::Other))?;
    // Çıktı ayrı thread'de okunur (boru dolup çocuk takılmasın); sınırdan fazlası okunmaz.
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let r = (&mut stdout).take(MAX_OUTPUT + 1).read_to_end(&mut buf);
        r.map(|_| buf)
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                // Yalnızca kendi çocuğumuz.
                let _ = child.kill();
                let _ = child.wait();
                // Okuyucu beklenmez: torun süreç boruyu tutuyorsa join takılırdı; boru kapanınca biter.
                tracing::warn!(program = name, "nuxut-apply: alt süreç zaman aşımı");
                return Err(ApplyError::Io(ErrorKind::TimedOut));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                drop(reader);
                return Err(e.into());
            }
        }
    };
    let stdout = reader
        .join()
        .map_err(|_| ApplyError::Io(ErrorKind::Other))?
        .map_err(ApplyError::from)?;
    if stdout.len() as u64 > MAX_OUTPUT {
        return Err(ApplyError::Io(ErrorKind::InvalidData));
    }
    Ok(Output {
        ok: status.success(),
        stdout,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn user_env() -> Env {
        Env {
            home: Some("/nonexistent/home".into()),
            real_home: None,
            config_home: Some("/nonexistent/home/.config".into()),
            state_home: Some("/nonexistent/home/.local/state".into()),
            test_instance: false,
            hyprland: None,
            disabled: false,
        }
    }

    /// Sahte `dconf`/`systemctl`: argümanları günlüğe yazar, durumu bir dosyada tutar.
    fn fake(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        p
    }

    /// Sahte program çağrısı başına süre: test süreci dondurulsa/aç kalsa da sınır; takılan
    /// bir betikte test yine biter.
    const FAKE_TIMEOUT: Duration = Duration::from_secs(120);

    const FAKE_DCONF: &str = r#"S="$(dirname "$0")/dconf.state"
printf '%s\n' "$*" >> "$(dirname "$0")/dconf.log"
case "$1" in
  read) [ -f "$S" ] && cat "$S"; exit 0 ;;
  write) case "$3" in bad*) exit 1 ;; esac; printf '%s\n' "$3" > "$S" ;;
  reset) rm -f "$S" ;;
esac"#;

    const FAKE_SYSTEMCTL: &str = r#"S="$(dirname "$0")/unit.state"
printf '%s\n' "$*" >> "$(dirname "$0")/systemctl.log"
case "$2" in
  is-enabled) if [ -f "$S" ]; then cat "$S"; [ "$(cat "$S")" = enabled ]; exit $?; fi; echo not-found; exit 4 ;;
  enable) echo enabled > "$S" ;;
  disable) [ -f "$S" ] || exit 1; echo disabled > "$S" ;;
esac"#;

    #[test]
    fn test_build_never_reaches_real_programs() {
        // Alt süreç başlatır: kilit tutan testlerle örtüşmemeli (bkz. test_gate).
        let _gate = crate::test_gate::exclusive();
        let b = SystemBackend::new(&user_env());
        let f = Fragment::Dconf {
            key: "/org/gnome/desktop/interface/gtk-theme".into(),
        };
        assert_eq!(b.read(&f), Err(ApplyError::Unavailable("dconf".into())));
        let u = Fragment::UnitState {
            unit: "nuxut-shell.service".into(),
        };
        assert_eq!(b.read(&u), Err(ApplyError::Unavailable("systemctl".into())));
    }

    #[test]
    fn test_instance_never_spawns() {
        let t = tempfile::tempdir().unwrap();
        // Çalıştırılırsa iz bırakan program.
        let p = fake(t.path(), "dconf", "touch \"$(dirname \"$0\")/ran\"");
        let mut env = user_env();
        env.test_instance = true;
        let b = SystemBackend::with_programs(&env, p.clone(), p);
        let d = Fragment::Dconf { key: "/a/b".into() };
        let u = Fragment::UnitState {
            unit: "a.service".into(),
        };
        assert!(matches!(b.read(&d), Err(ApplyError::Skipped(_))));
        assert!(matches!(
            b.write(&d, &FragValue::Text("1".into())),
            Err(ApplyError::Skipped(_))
        ));
        assert!(matches!(b.read(&u), Err(ApplyError::Skipped(_))));
        assert!(!t.path().join("ran").exists());
    }

    #[test]
    fn dconf_roundtrip_with_fake_program() {
        // Alt süreç başlatır: kilit tutan testlerle örtüşmemeli (bkz. test_gate).
        let _gate = crate::test_gate::exclusive();
        let t = tempfile::tempdir().unwrap();
        let p = fake(t.path(), "dconf", FAKE_DCONF);
        let b = SystemBackend::with_programs(&user_env(), p, "/nonexistent".into())
            .with_test_timeouts(FAKE_TIMEOUT);
        let f = Fragment::Dconf {
            key: "/org/gnome/desktop/interface/gtk-theme".into(),
        };
        assert_eq!(b.read(&f).unwrap(), FragValue::Absent);
        b.write(&f, &FragValue::Text("'Adwaita dark; rm -rf ~'".into()))
            .unwrap();
        // Argüman tek parça gider (kabuk yok).
        assert_eq!(
            b.read(&f).unwrap(),
            FragValue::Text("'Adwaita dark; rm -rf ~'".into())
        );
        assert_eq!(
            b.write(&f, &FragValue::Text("bad".into())),
            Err(ApplyError::Invalid("dconf değeri reddetti"))
        );
        b.write(&f, &FragValue::Absent).unwrap();
        assert_eq!(b.read(&f).unwrap(), FragValue::Absent);
        // Çok satırlı değer yazılmaz (doğrulama).
        assert!(b.write(&f, &FragValue::Text("a\nb".into())).is_err());
        let log = fs::read_to_string(t.path().join("dconf.log")).unwrap();
        assert!(
            log.contains("reset /org/gnome/desktop/interface/gtk-theme"),
            "{log}"
        );
    }

    #[test]
    fn systemctl_states_with_fake_program() {
        // Alt süreç başlatır: kilit tutan testlerle örtüşmemeli (bkz. test_gate).
        let _gate = crate::test_gate::exclusive();
        let t = tempfile::tempdir().unwrap();
        let p = fake(t.path(), "systemctl", FAKE_SYSTEMCTL);
        let b = SystemBackend::with_programs(&user_env(), "/nonexistent".into(), p)
            .with_test_timeouts(FAKE_TIMEOUT);
        let u = Fragment::UnitState {
            unit: "nuxut-shell.service".into(),
        };
        assert_eq!(b.read(&u).unwrap(), FragValue::Absent);
        // Yokken Absent yazmak: hiçbir şey.
        b.write(&u, &FragValue::Absent).unwrap();
        b.write(&u, &FragValue::Text("enabled".into())).unwrap();
        assert_eq!(b.read(&u).unwrap(), FragValue::Text("enabled".into()));
        b.write(&u, &FragValue::Text("disabled".into())).unwrap();
        assert_eq!(b.read(&u).unwrap(), FragValue::Text("disabled".into()));
        fs::write(t.path().join("unit.state"), "masked\n").unwrap();
        assert!(matches!(b.read(&u), Err(ApplyError::ExternallyManaged(_))));
        let log = fs::read_to_string(t.path().join("systemctl.log")).unwrap();
        assert!(
            log.contains("--user enable -- nuxut-shell.service"),
            "{log}"
        );
        assert!(
            !log.contains("disable -- nuxut-shell.service\n--user disable"),
            "{log}"
        );
    }

    #[test]
    fn timeout_and_output_limit() {
        // Alt süreç başlatır: kilit tutan testlerle örtüşmemeli (bkz. test_gate).
        let _gate = crate::test_gate::exclusive();
        let t = tempfile::tempdir().unwrap();
        let slow = fake(t.path(), "slow", "exec sleep 30");
        let start = Instant::now();
        let r = run(&slow, "slow", &[], Duration::from_millis(200));
        assert!(matches!(r, Err(ApplyError::Io(ErrorKind::TimedOut))));
        assert!(start.elapsed() < Duration::from_secs(5));
        let loud = fake(t.path(), "loud", "exec cat /dev/zero");
        let r = run(&loud, "loud", &[], Duration::from_secs(2));
        assert!(r.is_err());
        let missing = run(
            Path::new("/nonexistent/x"),
            "x",
            &[],
            Duration::from_secs(1),
        );
        assert!(matches!(missing, Err(ApplyError::Unavailable(n)) if n == "x"));
    }
}
