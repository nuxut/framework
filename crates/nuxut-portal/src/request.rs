//! Tek bir FileChooser isteğinin akışı (istek thread'inde, `async_io::block_on`).
//!
//! 1. `Response` sinyali aboneliği, beklenen istek yolu (`handle_token` ile öngörülen
//!    `/org/freedesktop/portal/desktop/request/<gönderen>/<belirteç>`) için çağrıdan ÖNCE
//!    kurulur (yarış yok). Portal başka yol dönerse (çok eski portal) o yola yeniden abone olunur.
//! 2. `OpenFile`/`SaveFile` çağrısı (zaman aşımı: etkinleştirme dahil 30 sn → `Unavailable`).
//! 3. `Response` ile iptal yarışır; iptal `Request.Close` gönderir (sonra portal yanıt vermez).
//!
//! `Response` yalnızca çağrıyı yanıtlayan bağlantıdan (portalın benzersiz adı) kabul edilir:
//! oturum bus'ındaki başka bir istemci sahte seçim enjekte edemez.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use async_io::Timer;
use futures_lite::StreamExt;
use futures_lite::future::or;
use zbus::message::Type as MessageType;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::{Connection, MatchRule, MessageStream};

use crate::{Bus, Inner, OpenFileOptions, PortalError, PortalRequestId, SaveFileOptions};

const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const DESKTOP_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.portal.Request";
/// Portal etkinleştirmesi (D-Bus aktivasyonu + arka uç) dahil çağrı yanıtı için üst sınır.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) enum Kind {
    Open(OpenFileOptions),
    Save(SaveFileOptions),
}

/// İsteği sonuna dek yürütür (bloklar).
pub(crate) fn run(
    inner: &Inner,
    id: PortalRequestId,
    parent: &str,
    kind: Kind,
    cancel: async_channel::Receiver<()>,
) -> Result<Vec<PathBuf>, PortalError> {
    let conn = connection(inner)?;
    let result = async_io::block_on(flow(&conn, id, parent, kind, &cancel));
    if matches!(result, Err(PortalError::Dbus(_))) {
        // Bağlantı bozulmuş olabilir: sonraki istek yeniden bağlansın.
        inner
            .connection
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
    result
}

/// Tembel, paylaşılan bağlantı. Bağlanamamak `Unavailable` (oturum bus'ı yok).
fn connection(inner: &Inner) -> Result<Connection, PortalError> {
    let mut slot = inner
        .connection
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(conn) = slot.as_ref() {
        return Ok(conn.clone());
    }
    let builder = match &inner.bus {
        Bus::Session => zbus::connection::Builder::session(),
        Bus::Address(address) => zbus::connection::Builder::address(address.as_str()),
    };
    let conn = builder
        .and_then(|b| async_io::block_on(b.build()))
        .map_err(|e| {
            tracing::info!(error = %error_kind(&e), "portal: bus'a bağlanılamadı");
            PortalError::Unavailable
        })?;
    *slot = Some(conn.clone());
    Ok(conn)
}

async fn flow(
    conn: &Connection,
    id: PortalRequestId,
    parent: &str,
    kind: Kind,
    cancel: &async_channel::Receiver<()>,
) -> Result<Vec<PathBuf>, PortalError> {
    let token = format!("nuxut_{}", id.0);
    let unique = conn
        .unique_name()
        .ok_or_else(|| PortalError::Dbus("unique-name".into()))?;
    let expected = request_path(unique.as_str(), &token)?;
    let mut responses = response_stream(conn, expected.as_ref()).await?;

    if cancel.try_recv().is_ok() {
        return Err(PortalError::Cancelled);
    }
    let (method, title, dict) = match &kind {
        Kind::Open(o) => (
            "OpenFile",
            o.title.as_str(),
            crate::options::open_file(o, &token),
        ),
        Kind::Save(o) => (
            "SaveFile",
            o.title.as_str(),
            crate::options::save_file(o, &token),
        ),
    };
    let body = (parent, title, dict);
    let call = conn.call_method(
        Some(DESTINATION),
        DESKTOP_PATH,
        Some(FILE_CHOOSER),
        method,
        &body,
    );
    let reply = or(async { Some(call.await) }, async {
        Timer::after(CALL_TIMEOUT).await;
        None
    })
    .await;
    let reply = match reply {
        None => {
            tracing::info!(id = id.0, "portal: çağrı zaman aşımı");
            return Err(PortalError::Unavailable);
        }
        Some(Err(e)) => return Err(map_call_error(&e)),
        Some(Ok(reply)) => reply,
    };
    let portal_name = reply
        .header()
        .sender()
        .map(|s| s.to_owned())
        .ok_or(PortalError::InvalidResponse)?;
    let handle: OwnedObjectPath = reply
        .body()
        .deserialize()
        .map_err(|_| PortalError::InvalidResponse)?;
    if handle != expected {
        tracing::debug!(id = id.0, "portal: istek yolu öngörülenden farklı");
        responses = response_stream(conn, handle.as_ref()).await?;
    }
    tracing::debug!(id = id.0, method, "portal: diyalog açıldı");

    enum Waited {
        Response(Result<Vec<PathBuf>, PortalError>),
        Cancel,
    }
    let waited = or(
        async { Waited::Response(next_response(&mut responses, portal_name.as_str()).await) },
        async {
            // Gönderen `Inner.pending`'de; `Portal` düşse bile thread `Inner`'i tutar.
            let _ = cancel.recv().await;
            Waited::Cancel
        },
    )
    .await;
    match waited {
        Waited::Response(result) => result,
        Waited::Cancel => {
            let close = conn.call_method(
                Some(portal_name.as_str()),
                handle.as_ref(),
                Some(REQUEST),
                "Close",
                &(),
            );
            let closed = or(async { Some(close.await) }, async {
                Timer::after(CLOSE_TIMEOUT).await;
                None
            })
            .await;
            tracing::debug!(
                id = id.0,
                closed = matches!(closed, Some(Ok(_))),
                "portal: istek iptal edildi"
            );
            Err(PortalError::Cancelled)
        }
    }
}

/// Beklenen `Request` nesne yolu: gönderenin benzersiz adı `:` olmadan, `.` → `_`.
pub(crate) fn request_path(unique_name: &str, token: &str) -> Result<OwnedObjectPath, PortalError> {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    OwnedObjectPath::try_from(format!("{DESKTOP_PATH}/request/{sender}/{token}"))
        .map_err(|_| PortalError::Dbus("object-path".into()))
}

async fn response_stream(
    conn: &Connection,
    path: ObjectPath<'_>,
) -> Result<MessageStream, PortalError> {
    let rule = MatchRule::builder()
        .msg_type(MessageType::Signal)
        .interface(REQUEST)
        .and_then(|b| b.member("Response"))
        .and_then(|b| b.path(path))
        .map_err(|e| PortalError::Dbus(error_kind(&e)))?
        .build()
        .to_owned();
    MessageStream::for_match_rule(rule, conn, Some(4))
        .await
        .map_err(|e| PortalError::Dbus(error_kind(&e)))
}

async fn next_response(
    stream: &mut MessageStream,
    portal_name: &str,
) -> Result<Vec<PathBuf>, PortalError> {
    while let Some(message) = stream.next().await {
        let message = message.map_err(|e| PortalError::Dbus(error_kind(&e)))?;
        if message.header().sender().map(|s| s.as_str()) != Some(portal_name) {
            tracing::warn!("portal: yabancı gönderenden Response yok sayıldı");
            continue;
        }
        let Ok((code, results)) = message
            .body()
            .deserialize::<(u32, HashMap<String, OwnedValue>)>()
        else {
            return Err(PortalError::InvalidResponse);
        };
        return interpret(code, results);
    }
    Err(PortalError::Dbus("connection-closed".into()))
}

/// `Response(u, a{sv})` → sonuç. 0 başarı (`uris`), 1 kullanıcı vazgeçti, diğer → reddedildi.
pub(crate) fn interpret(
    code: u32,
    mut results: HashMap<String, OwnedValue>,
) -> Result<Vec<PathBuf>, PortalError> {
    match code {
        0 => {}
        1 => return Err(PortalError::Cancelled),
        _ => return Err(PortalError::Denied),
    }
    let uris: Vec<String> = results
        .remove("uris")
        .and_then(|v| Vec::<String>::try_from(v).ok())
        .ok_or(PortalError::InvalidResponse)?;
    let total = uris.len();
    let paths: Vec<PathBuf> = uris
        .iter()
        .filter_map(|u| crate::uri::file_uri_to_path(u))
        .collect();
    if paths.len() != total {
        // URI'nin kendisi loglanmaz (kullanıcı içeriği); yalnız sayı.
        tracing::info!(
            dropped = total - paths.len(),
            total,
            "portal: file:// olmayan URI atlandı"
        );
    }
    if paths.is_empty() {
        return Err(PortalError::InvalidResponse);
    }
    Ok(paths)
}

/// Çağrı hatası → `PortalError`. Yalnız hata adına bakılır (ileti kullanıcı içeriği taşıyabilir).
fn map_call_error(error: &zbus::Error) -> PortalError {
    let name = match error {
        zbus::Error::MethodError(name, _, _) => name.as_str().to_owned(),
        zbus::Error::FDO(fdo) => zbus::DBusError::name(fdo.as_ref()).as_str().to_owned(),
        other => return PortalError::Dbus(error_kind(other)),
    };
    classify_error_name(&name)
}

pub(crate) fn classify_error_name(name: &str) -> PortalError {
    match name {
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner"
        | "org.freedesktop.DBus.Error.UnknownMethod"
        | "org.freedesktop.DBus.Error.UnknownInterface"
        | "org.freedesktop.DBus.Error.UnknownObject"
        | "org.freedesktop.DBus.Error.NoReply"
        | "org.freedesktop.DBus.Error.TimedOut"
        | "org.freedesktop.DBus.Error.Spawn.ChildExited"
        | "org.freedesktop.DBus.Error.Spawn.ExecFailed"
        | "org.freedesktop.DBus.Error.Spawn.ServiceNotFound" => PortalError::Unavailable,
        "org.freedesktop.DBus.Error.AccessDenied" | "org.freedesktop.portal.Error.NotAllowed" => {
            PortalError::Denied
        }
        other => PortalError::Dbus(other.to_owned()),
    }
}

/// Hata türü (loglanabilir; ileti metni değil).
fn error_kind(error: &zbus::Error) -> String {
    match error {
        zbus::Error::InputOutput(_) => "io".into(),
        zbus::Error::Address(_) => "address".into(),
        zbus::Error::Handshake(_) => "handshake".into(),
        zbus::Error::MethodError(name, _, _) => name.as_str().to_owned(),
        zbus::Error::FDO(fdo) => zbus::DBusError::name(fdo.as_ref()).as_str().to_owned(),
        _ => "zbus".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    fn uris(list: &[&str]) -> HashMap<String, OwnedValue> {
        let list: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
        HashMap::from([(
            "uris".to_owned(),
            OwnedValue::try_from(Value::from(list)).unwrap(),
        )])
    }

    #[test]
    fn request_path_from_unique_name() {
        assert_eq!(
            request_path(":1.42", "nuxut_7").unwrap().as_str(),
            "/org/freedesktop/portal/desktop/request/1_42/nuxut_7"
        );
    }

    #[test]
    fn interpret_codes() {
        assert_eq!(
            interpret(0, uris(&["file:///a/b%20c.png", "file:///d"])),
            Ok(vec!["/a/b c.png".into(), "/d".into()])
        );
        assert_eq!(interpret(1, HashMap::new()), Err(PortalError::Cancelled));
        assert_eq!(interpret(2, HashMap::new()), Err(PortalError::Denied));
        assert_eq!(
            interpret(0, HashMap::new()),
            Err(PortalError::InvalidResponse)
        );
        // file:// olmayanlar atlanır; hiçbiri kalmazsa geçersiz.
        assert_eq!(
            interpret(0, uris(&["https://x/a", "file:///ok"])),
            Ok(vec!["/ok".into()])
        );
        assert_eq!(
            interpret(0, uris(&["smb://srv/a"])),
            Err(PortalError::InvalidResponse)
        );
        assert_eq!(interpret(0, uris(&[])), Err(PortalError::InvalidResponse));
    }

    #[test]
    fn error_names() {
        assert_eq!(
            classify_error_name("org.freedesktop.DBus.Error.ServiceUnknown"),
            PortalError::Unavailable
        );
        assert_eq!(
            classify_error_name("org.freedesktop.DBus.Error.UnknownInterface"),
            PortalError::Unavailable
        );
        assert_eq!(
            classify_error_name("org.freedesktop.portal.Error.NotAllowed"),
            PortalError::Denied
        );
        assert_eq!(
            classify_error_name("org.example.Weird"),
            PortalError::Dbus("org.example.Weird".into())
        );
    }
}
