//! Sahte `org.freedesktop.portal.Desktop` ile uçtan uca: Response 0/1/2, iptal (Request.Close),
//! portal yok, bus yok, sahte gönderen.
//!
//! Her test kendi özel `dbus-daemon`'unu başlatır (geçici dizinde, servis etkinleştirme dizini
//! YOK, boş ortam): kullanıcının oturum bus'ına hiç bağlanılmaz ve gerçek bir portal
//! etkinleştirilemez. `dbus-daemon` yoksa testler atlanır (uyarıyla).

#![cfg(target_os = "linux")]

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut_portal::{
    Filter, OpenFileOptions, ParentWindow, Portal, PortalError, PortalResponse, SaveFileOptions,
};
use zbus::message::Header;
use zbus::object_server::ObjectServer;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const TIMEOUT: Duration = Duration::from_secs(10);

/// Özel, yalıtılmış bus. Düşerken daemon öldürülür ve dizin silinir.
struct PrivateBus {
    child: Child,
    address: String,
    dir: PathBuf,
}

impl PrivateBus {
    fn start() -> Option<Self> {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "nuxut-portal-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).ok()?;
        let config = dir.join("bus.conf");
        std::fs::write(
            &config,
            format!(
                r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:path={}/bus</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#,
                dir.display()
            ),
        )
        .ok()?;
        let child = Command::new("dbus-daemon")
            .arg(format!("--config-file={}", config.display()))
            .arg("--nofork")
            .arg("--print-address=1")
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(e) => {
                eprintln!("dbus-daemon başlatılamadı ({e}); test atlandı");
                let _ = std::fs::remove_dir_all(&dir);
                return None;
            }
        };
        let mut line = String::new();
        let stdout = child.stdout.take()?;
        BufReader::new(stdout).read_line(&mut line).ok()?;
        let address = line.trim().to_owned();
        assert!(address.starts_with("unix:"), "beklenmeyen adres");
        Some(Self {
            child,
            address,
            dir,
        })
    }
}

impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[derive(Debug)]
struct Call {
    method: &'static str,
    parent: String,
    title: String,
    options: HashMap<String, OwnedValue>,
}

#[derive(Default, Clone)]
struct Shared {
    calls: Arc<Mutex<Vec<Call>>>,
    closed: Arc<Mutex<Vec<String>>>,
    /// `spoof` senaryosu: önce bu bağlantıdan sahte bir Response.
    impostor: Arc<Mutex<Option<zbus::blocking::Connection>>>,
}

struct FakeChooser(Shared);

struct FakeRequest {
    path: String,
    shared: Shared,
}

#[zbus::interface(name = "org.freedesktop.portal.Request")]
impl FakeRequest {
    async fn close(&self) {
        self.shared.closed.lock().unwrap().push(self.path.clone());
    }
}

impl FakeChooser {
    #[allow(clippy::too_many_arguments)]
    async fn handle(
        &self,
        method: &'static str,
        header: Header<'_>,
        conn: &zbus::Connection,
        server: &ObjectServer,
        parent: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let sender = header.sender().expect("gönderen").to_owned();
        let token: String = options
            .get("handle_token")
            .and_then(|v| v.try_clone().ok())
            .and_then(|v| String::try_from(v).ok())
            .expect("handle_token");
        let path = format!(
            "{DESKTOP_PATH}/request/{}/{token}",
            sender.trim_start_matches(':').replace('.', "_")
        );
        server
            .at(
                path.as_str(),
                FakeRequest {
                    path: path.clone(),
                    shared: self.0.clone(),
                },
            )
            .await?;
        let scenario = title.clone();
        self.0.calls.lock().unwrap().push(Call {
            method,
            parent,
            title,
            options,
        });
        let code = match scenario.as_str() {
            "ok" | "spoof" => 0u32,
            "cancel" => 1,
            "other" => 2,
            _ => {
                // "hang": yanıt yok; Request.Close beklenir.
                return Ok(OwnedObjectPath::try_from(path).unwrap());
            }
        };
        let conn = zbus::blocking::Connection::from(conn.clone());
        let impostor = self.0.impostor.lock().unwrap().clone();
        let reply_path = path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            if scenario == "spoof" {
                let impostor = impostor.expect("impostor");
                let fake: HashMap<&str, Value> =
                    HashMap::from([("uris", Value::from(vec!["file:///etc/shadow".to_owned()]))]);
                impostor
                    .emit_signal(
                        Some(sender.as_str()),
                        reply_path.as_str(),
                        "org.freedesktop.portal.Request",
                        "Response",
                        &(0u32, fake),
                    )
                    .unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
            let uris = if method == "SaveFile" {
                vec!["file:///tmp/kayit%20yeri.toml".to_owned()]
            } else {
                vec![
                    "file:///tmp/a%20b.txt".to_owned(),
                    "file:///run/user/1000/doc/3f/vpn.ovpn".to_owned(),
                ]
            };
            let results: HashMap<&str, Value> = if code == 0 {
                HashMap::from([("uris", Value::from(uris))])
            } else {
                HashMap::new()
            };
            conn.emit_signal(
                Some(sender.as_str()),
                reply_path.as_str(),
                "org.freedesktop.portal.Request",
                "Response",
                &(code, results),
            )
            .unwrap();
        });
        Ok(OwnedObjectPath::try_from(path).unwrap())
    }
}

#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl FakeChooser {
    async fn open_file(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &ObjectServer,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.handle(
            "OpenFile",
            header,
            conn,
            server,
            parent_window,
            title,
            options,
        )
        .await
    }

    async fn save_file(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(object_server)] server: &ObjectServer,
        parent_window: String,
        title: String,
        options: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        self.handle(
            "SaveFile",
            header,
            conn,
            server,
            parent_window,
            title,
            options,
        )
        .await
    }
}

struct Harness {
    bus: PrivateBus,
    shared: Shared,
    _service: Option<zbus::blocking::Connection>,
    portal: Portal,
    rx: mpsc::Receiver<PortalResponse>,
}

impl Harness {
    /// `with_service`: sahte portal bus adını alır mı.
    fn new(with_service: bool) -> Option<Self> {
        let bus = PrivateBus::start()?;
        let shared = Shared::default();
        let service = with_service.then(|| {
            zbus::blocking::connection::Builder::address(bus.address.as_str())
                .unwrap()
                .name(DEST)
                .unwrap()
                .serve_at(DESKTOP_PATH, FakeChooser(shared.clone()))
                .unwrap()
                .build()
                .unwrap()
        });
        let (tx, rx) = mpsc::channel();
        let portal = Portal::with_address(bus.address.clone(), move |response| {
            let _ = tx.send(response);
        });
        Some(Self {
            bus,
            shared,
            _service: service,
            portal,
            rx,
        })
    }

    fn recv(&self) -> PortalResponse {
        self.rx
            .recv_timeout(TIMEOUT)
            .expect("portal sonucu gelmedi")
    }

    fn wait_for_call(&self) {
        let start = Instant::now();
        while self.shared.calls.lock().unwrap().is_empty() {
            assert!(start.elapsed() < TIMEOUT, "çağrı gelmedi");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn opt_str(options: &HashMap<String, OwnedValue>, key: &str) -> String {
    String::try_from(options[key].try_clone().unwrap()).unwrap()
}

fn opt_bool(options: &HashMap<String, OwnedValue>, key: &str) -> bool {
    bool::try_from(&*options[key]).unwrap()
}

#[test]
fn open_file_success() {
    let Some(h) = Harness::new(true) else { return };
    let mut options = OpenFileOptions::new("ok");
    options.multiple = true;
    options.accept_label = Some("İçe aktar".into());
    options.filters = vec![Filter::new("OpenVPN").glob("*.ovpn")];
    options.current_folder = Some("/tmp".into());
    let id = h
        .portal
        .open_file(&ParentWindow::wayland("tanitici"), options);
    let response = h.recv();
    assert_eq!(response.id, id);
    assert_eq!(
        response.result,
        Ok(vec![
            PathBuf::from("/tmp/a b.txt"),
            PathBuf::from("/run/user/1000/doc/3f/vpn.ovpn")
        ])
    );
    let calls = h.shared.calls.lock().unwrap();
    let call = &calls[0];
    assert_eq!(call.method, "OpenFile");
    assert_eq!(call.parent, "wayland:tanitici");
    assert_eq!(call.title, "ok");
    assert!(opt_str(&call.options, "handle_token").starts_with("nuxut_"));
    assert_eq!(opt_str(&call.options, "accept_label"), "İçe aktar");
    assert!(opt_bool(&call.options, "multiple"));
    assert!(opt_bool(&call.options, "modal"));
    assert!(!opt_bool(&call.options, "directory"));
    assert_eq!(call.options["filters"].value_signature(), "a(sa(us))");
    assert_eq!(call.options["current_folder"].value_signature(), "ay");
    assert_eq!(h.portal.pending(), 0);
}

#[test]
fn save_file_success() {
    let Some(h) = Harness::new(true) else { return };
    let mut options = SaveFileOptions::new("ok");
    options.current_name = Some("yedek.toml".into());
    let id = h.portal.save_file(&ParentWindow::none(), options);
    let response = h.recv();
    assert_eq!(response.id, id);
    assert_eq!(
        response.result,
        Ok(vec![PathBuf::from("/tmp/kayit yeri.toml")])
    );
    let calls = h.shared.calls.lock().unwrap();
    assert_eq!(calls[0].method, "SaveFile");
    assert_eq!(calls[0].parent, "");
    assert_eq!(opt_str(&calls[0].options, "current_name"), "yedek.toml");
}

#[test]
fn user_cancel_and_other_end() {
    let Some(h) = Harness::new(true) else { return };
    h.portal
        .open_file(&ParentWindow::none(), OpenFileOptions::new("cancel"));
    assert_eq!(h.recv().result, Err(PortalError::Cancelled));
    h.portal
        .open_file(&ParentWindow::none(), OpenFileOptions::new("other"));
    assert_eq!(h.recv().result, Err(PortalError::Denied));
}

#[test]
fn cancel_sends_request_close() {
    let Some(h) = Harness::new(true) else { return };
    let id = h
        .portal
        .open_file(&ParentWindow::none(), OpenFileOptions::new("hang"));
    h.wait_for_call();
    assert_eq!(h.portal.pending(), 1);
    h.portal.cancel(id);
    let response = h.recv();
    assert_eq!(response.id, id);
    assert_eq!(response.result, Err(PortalError::Cancelled));
    let closed = h.shared.closed.lock().unwrap();
    assert_eq!(closed.len(), 1);
    assert!(closed[0].ends_with(&format!("/nuxut_{}", id.get())));
    // Bitmiş kimliği iptal etmek etkisiz.
    h.portal.cancel(id);
    assert!(h.rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn dropping_portal_closes_pending() {
    let Some(h) = Harness::new(true) else { return };
    let Harness {
        bus: _bus,
        shared,
        _service,
        portal,
        rx,
    } = h;
    let id = portal.open_file(&ParentWindow::none(), OpenFileOptions::new("hang"));
    let start = Instant::now();
    while shared.calls.lock().unwrap().is_empty() {
        assert!(start.elapsed() < TIMEOUT);
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(portal);
    let response = rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(response.id, id);
    assert_eq!(response.result, Err(PortalError::Cancelled));
    assert_eq!(shared.closed.lock().unwrap().len(), 1);
}

#[test]
fn impostor_response_is_ignored() {
    let Some(h) = Harness::new(true) else { return };
    let impostor = zbus::blocking::connection::Builder::address(h.bus.address.as_str())
        .unwrap()
        .build()
        .unwrap();
    *h.shared.impostor.lock().unwrap() = Some(impostor);
    h.portal
        .open_file(&ParentWindow::none(), OpenFileOptions::new("spoof"));
    // Sahte Response (/etc/shadow) yok sayılır; gerçek portalın yanıtı gelir.
    let result = h.recv().result.unwrap();
    assert_eq!(result[0], PathBuf::from("/tmp/a b.txt"));
}

#[test]
fn no_portal_service_is_unavailable() {
    let Some(h) = Harness::new(false) else { return };
    h.portal
        .open_file(&ParentWindow::none(), OpenFileOptions::new("ok"));
    assert_eq!(h.recv().result, Err(PortalError::Unavailable));
}

#[test]
fn no_bus_is_unavailable() {
    let (tx, rx) = mpsc::channel();
    let dir = std::env::temp_dir().join(format!("nuxut-portal-nobus-{}", std::process::id()));
    let portal = Portal::with_address(
        format!("unix:path={}/yok", dir.display()),
        move |response| {
            let _ = tx.send(response);
        },
    );
    let id = portal.open_file(&ParentWindow::none(), OpenFileOptions::new("ok"));
    let response = rx.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(response.id, id);
    assert_eq!(response.result, Err(PortalError::Unavailable));
}
