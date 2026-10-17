//! Testler için küçük, betikli HTTP/1.1 sunucusu (yalnızca `127.0.0.1`); düz ya da TLS
//! (`FakeServer::start_tls`, test CA'sı [`TEST_CA_DER`]). YALNIZ TEST: `test-support` özelliği.
//!
//! Her bağlantıda tek istek okunur, işleyicinin [`Reply`]'ı yazılır, bağlantı kapanır.
//! Gelen istekler [`FakeServer::requests`] ile sonradan incelenebilir.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use url::Url;

/// Sunucuya gelen istek.
#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    /// Sorgu dahil olmayan yol.
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// `application/x-www-form-urlencoded` gövdeden alan.
    pub fn form(&self, key: &str) -> Option<String> {
        url::form_urlencoded::parse(&self.body).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
    }

    pub fn query_param(&self, key: &str) -> Option<String> {
        let q = self.query.as_deref()?;
        url::form_urlencoded::parse(q.as_bytes()).find(|(k, _)| k == key).map(|(_, v)| v.into_owned())
    }
}

/// İşleyicinin yanıtı.
#[derive(Debug, Clone)]
pub struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Option<Duration>,
}

impl Reply {
    pub fn status(status: u16) -> Self {
        Self { status, headers: Vec::new(), body: Vec::new(), delay: None }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, body: &str) -> Self {
        self.body = body.as_bytes().to_vec();
        self
    }

    /// Ham bayt gövde (ör. sıkıştırılmış).
    pub fn bytes(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn xml(self, body: &str) -> Self {
        self.header("content-type", "application/xml; charset=utf-8").body(body)
    }

    pub fn json(self, body: &str) -> Self {
        self.header("content-type", "application/json").body(body)
    }

    /// Yanıtı yazmadan önce bekle (zaman aşımı testleri).
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

type Handler = dyn Fn(&RecordedRequest) -> Reply + Send + Sync + 'static;

// ---------------------------------------------------------------------------------------------
// YALNIZ TEST: sertifika fixture'ları. Üretimde hiçbir yerde kullanılmaz; bu modül yalnızca
// `test-support` özelliğiyle (ya da crate'in kendi testlerinde) derlenir.
//
// Bir kez üretildi (OpenSSL 3, EC P-256, 2026-01-01 → 2126-01-01); CA'ların özel anahtarları
// saklanmadı (yeniden üretmek için aynı komutlar, tüm fixture'lar birlikte değişir):
//
//   openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
//     -keyout ca.key.pem -out ca.pem -not_before 20260101000000Z -not_after 21260101000000Z \
//     -subj "/CN=Nuxut TEST ONLY CA" \
//     -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
//   openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
//     -keyout server.key.pem -out server.csr -subj "/CN=nuxut.test"
//   openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key.pem -CAcreateserial -out server.pem \
//     -not_before 20260101000000Z -not_after 21260101000000Z -extfile ext.cnf
//     # ext.cnf: basicConstraints=critical,CA:FALSE / keyUsage=critical,digitalSignature /
//     #          extendedKeyUsage=serverAuth / subjectAltName=DNS:localhost,DNS:nuxut.test,IP:127.0.0.1
//   openssl x509 -in ca.pem -outform DER -out test-ca.der
//   openssl x509 -in server.pem -outform DER -out test-server.der
//   openssl pkcs8 -topk8 -nocrypt -in server.key.pem -outform DER -out test-server.key.der
//   # other-ca.der: ilk komut, -subj "/CN=Nuxut TEST ONLY other CA" ile (yanlış CA testleri).
// ---------------------------------------------------------------------------------------------

/// **Yalnız test:** [`FakeServer::start_tls`] sertifikasını imzalayan CA (DER). İstemci tarafı:
/// [`crate::HttpClient::with_test_root_ca`] / [`crate::HttpClient::with_test_tls`].
pub const TEST_CA_DER: &[u8] = include_bytes!("testdata/test-ca.der");
/// **Yalnız test:** sunucu sertifikasını İMZALAMAMIŞ başka bir CA (yanlış CA → TLS hatası).
pub const OTHER_CA_DER: &[u8] = include_bytes!("testdata/other-ca.der");
/// Sunucu sertifikası; SAN: `localhost`, `nuxut.test`, `127.0.0.1`.
#[cfg(all(feature = "tls", any(test, feature = "test-support")))]
const TEST_SERVER_CERT_DER: &[u8] = include_bytes!("testdata/test-server.der");
/// Sunucu anahtarı (PKCS#8). Yalnız bu test sunucusu içindir; gizli değildir.
#[cfg(all(feature = "tls", any(test, feature = "test-support")))]
const TEST_SERVER_KEY_DER: &[u8] = include_bytes!("testdata/test-server.key.der");

/// Her adı `127.0.0.1`'e çözen test çözümleyicisi: SSRF politikası URL'deki adı çözer, bu
/// yüzden `https://nuxut.test:<port>/` gibi bir adres sahte sunucuya böyle ulaşır.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoopbackLookup;

impl crate::Lookup for LoopbackLookup {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<std::net::IpAddr>> {
        Ok(vec![std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)])
    }
}

/// Düşürülünce durur.
#[derive(Debug)]
pub struct FakeServer {
    base: Url,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Düz HTTP: `http://127.0.0.1:<port>/`.
    pub fn start(handler: impl Fn(&RecordedRequest) -> Reply + Send + Sync + 'static) -> Self {
        Self::spawn("http", Arc::new(handler), |stream, handler, requests| serve(stream, handler, requests))
    }

    /// TLS: `https://127.0.0.1:<port>/`; sertifika [`TEST_CA_DER`] ile imzalı, SAN `localhost`,
    /// `nuxut.test`, `127.0.0.1`. İstek/yanıt modeli [`FakeServer::start`] ile aynı. El sıkışması
    /// başarısız bağlantılar (ör. istemci CA'yı tanımadı) kaydedilmez.
    ///
    /// SSRF politikasıyla adla bağlanmak ([`Trust::Server`](crate::Trust) — ör. medya kapağı):
    ///
    /// ```
    /// # #[cfg(feature = "tls")] {
    /// use std::sync::Arc;
    /// use nuxut_net::testing::{FakeServer, LoopbackLookup, Reply, TEST_CA_DER};
    /// use nuxut_net::{HttpClient, Policy};
    ///
    /// let server = FakeServer::start_tls(|_| Reply::status(200).bytes(vec![0x89, b'P', b'N', b'G']));
    /// // Ad (`nuxut.test`) LoopbackLookup ile 127.0.0.1'e çözülür; sunucunun verdiği adrese
    /// // (Trust::Server) loopback ancak `allow_insecure_loopback` ile izinlidir — yalnız test.
    /// let policy = Policy { allow_insecure_loopback: true, ..Policy::default() };
    /// let http = HttpClient::with_test_tls(policy, TEST_CA_DER, Arc::new(LoopbackLookup));
    /// let resp = http.fetch_untrusted_media(server.url_at("nuxut.test", "/cover.png")).unwrap();
    /// assert_eq!(resp.status.as_u16(), 200);
    /// # }
    /// ```
    #[cfg(all(feature = "tls", any(test, feature = "test-support")))]
    pub fn start_tls(handler: impl Fn(&RecordedRequest) -> Reply + Send + Sync + 'static) -> Self {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("TLS sürümleri")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(TEST_SERVER_CERT_DER)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(TEST_SERVER_KEY_DER)),
            )
            .expect("test sunucusu sertifikası");
        let config = Arc::new(config);
        Self::spawn("https", Arc::new(handler), move |stream, handler, requests| {
            let Ok(conn) = rustls::ServerConnection::new(Arc::clone(&config)) else {
                return;
            };
            let mut tls = rustls::StreamOwned::new(conn, stream);
            // El sıkışması burada, ilk istek okunmadan; başarısızsa bağlantı sessizce biter.
            while tls.conn.is_handshaking() {
                if tls.conn.complete_io(&mut tls.sock).is_err() {
                    return;
                }
            }
            serve(&mut tls, handler, requests);
            tls.conn.send_close_notify();
            let _ = tls.flush();
        })
    }

    fn spawn(
        scheme: &str,
        handler: Arc<Handler>,
        serve_one: impl Fn(TcpStream, &Handler, &Mutex<Vec<RecordedRequest>>) + Send + Sync + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test sunucusu bağlanamadı");
        let port = listener.local_addr().unwrap().port();
        let base = Url::parse(&format!("{scheme}://127.0.0.1:{port}/")).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let serve_one = Arc::new(serve_one);
        let thread = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let handler = Arc::clone(&handler);
                    let requests = Arc::clone(&requests);
                    let serve_one = Arc::clone(&serve_one);
                    std::thread::spawn(move || serve_one(stream, &*handler, &requests));
                }
            })
        };
        Self { base, requests, stop, thread: Some(thread) }
    }

    pub fn url(&self, path: &str) -> Url {
        self.base.join(path.trim_start_matches('/')).unwrap()
    }

    /// Aynı sunucu, ana makine adı `host` ile (ör. `nuxut.test`; çözümü [`LoopbackLookup`]).
    pub fn url_at(&self, host: &str, path: &str) -> Url {
        let mut url = self.url(path);
        url.set_host(Some(host)).expect("geçerli ana makine adı");
        url
    }

    pub fn port(&self) -> u16 {
        self.base.port().expect("sunucu portu")
    }

    pub fn base(&self) -> &Url {
        &self.base
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Bloklayan accept'i uyandır.
        let _ = TcpStream::connect(self.base.socket_addrs(|| None).unwrap()[0]);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Tek istek okur, işleyicinin yanıtını yazar (düz TCP ya da TLS akışı).
fn serve<S: Read + Write>(mut stream: S, handler: &Handler, log: &Mutex<Vec<RecordedRequest>>) {
    let request = {
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let target = parts.next().unwrap_or_default().to_string();
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (target, None),
        };
        let mut headers = Vec::new();
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                break;
            }
            if let Some((n, v)) = h.trim_end().split_once(':') {
                headers.push((n.trim().to_string(), v.trim().to_string()));
            }
        }
        let len = headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0; len];
        let _ = reader.read_exact(&mut body);
        RecordedRequest { method, path, query, headers, body }
    };
    log.lock().unwrap().push(request.clone());
    let reply = handler(&request);
    if let Some(d) = reply.delay {
        std::thread::sleep(d);
    }
    let mut head = format!("HTTP/1.1 {} X\r\ncontent-length: {}\r\nconnection: close\r\n", reply.status, reply.body.len());
    for (n, v) in &reply.headers {
        head.push_str(&format!("{n}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(&reply.body);
    let _ = stream.flush();
}
