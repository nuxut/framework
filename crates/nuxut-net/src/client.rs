//! Sınırları belli, senkron HTTP istemcisi ve **kökene bağlı** kimlikli istemci.
//!
//! Kaynak: `calendar/crates/nuxut-accounts/src/http.rs` (aynı proje, MIT); framework'e taşınırken
//! log politikası, vekil, `https_only`, yönlendirmede userinfo ve güvenilmeyen medya profili
//! eklendi. Güvenlik kuralları tek yerde ve tip düzeyindedir:
//!
//! - **Kimlik yalnızca [`OriginBoundClient`] ile.** `Authorization` başlığını ekleyebilen tek yol
//!   budur ([`Request::header`] bu başlığı reddeder). İstemci bir kökene (şema + ana makine +
//!   port) bağlıdır; URL'si bu kökenden olmayan istek **gönderilmeden** reddedilir
//!   ([`HttpError::OriginMismatch`]). Köken dışına yönlendirme izlenmez ve kimlik gönderilmez
//!   ([`HttpError::CrossOriginRedirect`]); çağıran kullanıcı onayı isteyebilir.
//! - **Kimlik yalnızca https üzerinden.** `http://` kökene bağlanmak yalnızca loopback adres ve
//!   açık geliştirme bayrağıyla ([`Policy::allow_insecure_loopback`]) mümkündür.
//! - **SSRF:** her adımda ad bir kez çözülür, tüm adresler sınıflandırılır ([`crate::classify`]),
//!   bağlantı yalnızca denetlenmiş adrese yapılır (DNS rebinding yok). Sunucunun yönlendirdiği
//!   **başka köken** yalnızca genel internete gidebilir ([`Trust::Server`]). Kullanıcının
//!   kendisinin girdiği adres (ev sunucusu 192.168.x) serbesttir; link-local ve özel amaçlı
//!   bloklar hiçbir zaman.
//! - **URL'de kimlik yok:** `user:parola@` içeren adres (ilk ya da yönlendirme) reddedilir.
//! - **Vekil yok:** ortamdaki `HTTPS_PROXY`/`ALL_PROXY` yok sayılır (vekil hedefi kendisi çözer,
//!   sabitleme atlanırdı). Bağlantı havuzu kapalı (aynı nedenle).
//! - **TLS:** rustls; kök sertifikalar sistemden (`rustls-platform-verifier`).
//! - **Sınırlar:** bağlantı 10 sn, istek başına toplam süre (varsayılan 60 sn), yanıt gövdesi
//!   **açılmış** boyutuyla (gzip bombası) istek başına sınırlı (varsayılan 16 MiB), en çok
//!   [`MAX_REDIRECTS`] yönlendirme, `https → http` reddedilir.
//! - **Log/Debug (değişmez 9):** hata iletileri ve `Debug` URL, ana makine, gövde ya da kimlik
//!   yazmaz; URL'ye ihtiyaç duyan çağıran [`HttpError::target_url`] kullanır.

use std::cell::RefCell;
use std::fmt;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use ureq::http::{self, HeaderMap, HeaderName, HeaderValue, StatusCode};
pub use ureq::http::Method;
use url::{Origin, Url};
use zeroize::Zeroizing;

use crate::ip::{IpClass, classify};

pub const MAX_REDIRECTS: u32 = 5;
/// Varsayılan yanıt gövdesi sınırı (açılmış bayt).
pub const DEFAULT_MAX_BODY: u64 = 16 * 1024 * 1024;
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
pub const USER_AGENT: &str = concat!("nuxut/", env!("CARGO_PKG_VERSION"));

/// [`Request::untrusted_media`] profili: gövde, toplam süre, yönlendirme sınırları.
pub const MEDIA_MAX_BODY: u64 = 8 * 1024 * 1024;
pub const MEDIA_TIMEOUT: Duration = Duration::from_secs(5);
pub const MEDIA_MAX_REDIRECTS: u32 = 3;

/// HTTP hatası. `Display`/`Debug` yalnızca türü yazar (URL, ana makine, sunucu iletisi yok).
#[derive(thiserror::Error)]
pub enum HttpError {
    #[error("geçersiz adres")]
    BadUrl,
    /// Adreste kullanıcı adı/parola (`user:pass@`) var.
    #[error("adreste kimlik bilgisi kabul edilmez")]
    CredentialsInUrl,
    #[error("sunucu bulunamadı")]
    HostNotFound,
    #[error("bağlantı zaman aşımına uğradı")]
    Timeout,
    #[error("TLS hatası")]
    Tls,
    #[error("ağ hatası")]
    Io(std::io::ErrorKind),
    #[error("yanıt çok büyük (sınır {0} bayt)")]
    TooLarge(u64),
    #[error("çok fazla yönlendirme")]
    TooManyRedirects,
    #[error("güvenli bağlantıdan güvensiz bağlantıya yönlendirme reddedildi")]
    Downgrade,
    /// `https_only` istekte `http://` hedef.
    #[error("yalnızca https kabul edilir")]
    InsecureScheme,
    #[error("kimlik bilgisi şifrelenmemiş bağlantı üzerinden gönderilmez")]
    InsecureAuth,
    /// Kökene bağlı istemciden başka kökene istek (gönderilmedi).
    #[error("istek bağlı sunucunun dışına çıkıyor")]
    OriginMismatch(Url),
    /// Sunucu başka bir kökene yönlendirdi; kimlik gönderilmedi, istek durdu.
    #[error("sunucu başka bir adrese yönlendirdi")]
    CrossOriginRedirect(Url),
    /// Hedef iç ağ / loopback / meta veri adresi (SSRF koruması). Ana makine adı taşır.
    #[error("güvenlik nedeniyle izin verilmeyen hedef")]
    ForbiddenTarget(String),
    #[error("HTTP protokol hatası")]
    Protocol,
}

impl fmt::Debug for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(kind) => write!(f, "Io({kind:?})"),
            Self::TooLarge(n) => write!(f, "TooLarge({n})"),
            Self::OriginMismatch(_) => f.write_str("OriginMismatch(..)"),
            Self::CrossOriginRedirect(_) => f.write_str("CrossOriginRedirect(..)"),
            Self::ForbiddenTarget(_) => f.write_str("ForbiddenTarget(..)"),
            other => fmt::Display::fmt(other, f),
        }
    }
}

impl HttpError {
    /// Geçici mi (yeniden denemeye değer)?
    pub fn is_transient(&self) -> bool {
        matches!(self, Self::HostNotFound | Self::Timeout | Self::Io(_) | Self::Protocol)
    }

    /// Köken dışı istek/yönlendirmenin hedefi (kullanıcıya "şu adrese gidilsin mi?" sormak
    /// için). Loglanmaz.
    pub fn target_url(&self) -> Option<&Url> {
        match self {
            Self::OriginMismatch(u) | Self::CrossOriginRedirect(u) => Some(u),
            _ => None,
        }
    }

    /// Reddedilen hedefin ana makine adı (UI için). Loglanmaz.
    pub fn forbidden_host(&self) -> Option<&str> {
        match self {
            Self::ForbiddenTarget(h) => Some(h),
            _ => None,
        }
    }
}

impl From<ureq::Error> for HttpError {
    fn from(e: ureq::Error) -> Self {
        match e {
            ureq::Error::HostNotFound => Self::HostNotFound,
            ureq::Error::Timeout(_) => Self::Timeout,
            ureq::Error::Tls(_) => Self::Tls,
            ureq::Error::BadUri(_) => Self::BadUrl,
            ureq::Error::Io(e) if is_timeout_kind(e.kind()) => Self::Timeout,
            // rustls el sıkışma/sertifika hataları ureq'ten `Io(InvalidData)` içinde gelir; geçici
            // ağ hatası sayılıp yeniden denenmesin (`is_transient`), TLS hatası olarak sınıflanır.
            #[cfg(feature = "tls")]
            ureq::Error::Io(e) if e.get_ref().is_some_and(|inner| inner.is::<rustls::Error>()) => Self::Tls,
            #[cfg(feature = "tls")]
            ureq::Error::Rustls(_) => Self::Tls,
            ureq::Error::Io(e) => Self::Io(e.kind()),
            ureq::Error::ConnectionFailed => Self::Io(std::io::ErrorKind::ConnectionRefused),
            ureq::Error::BodyExceedsLimit(n) => Self::TooLarge(n),
            ureq::Error::TooManyRedirects => Self::TooManyRedirects,
            other => match other.into_io() {
                e if is_timeout_kind(e.kind()) => Self::Timeout,
                _ => Self::Protocol,
            },
        }
    }
}

/// Soket zaman aşımı türleri: Unix'te `SO_RCVTIMEO` dolunca `WouldBlock`, Windows'ta `TimedOut`.
/// Soketler bloklayan kipte olduğundan `WouldBlock` yalnızca süre dolması demektir.
fn is_timeout_kind(kind: std::io::ErrorKind) -> bool {
    matches!(kind, std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
}

/// Bellekten silinirken sıfırlanan, `Debug`'da gizlenen gizli metin.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl From<&str> for Secret {
    fn from(s: &str) -> Self {
        Self::new(s.to_string())
    }
}

/// Kimlik bilgisi. Yalnızca [`OriginBoundClient`] başlığa çevirebilir.
#[derive(Clone)]
pub enum Auth {
    Basic { user: String, password: Secret },
    Bearer(Secret),
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Basic { .. } => f.write_str("Basic(..)"),
            Self::Bearer(_) => f.write_str("Bearer(..)"),
        }
    }
}

impl Auth {
    fn header_value(&self) -> HeaderValue {
        use base64::Engine;
        let text = Zeroizing::new(match self {
            Self::Basic { user, password } => {
                let raw = Zeroizing::new(format!("{user}:{}", password.expose()));
                format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(raw.as_bytes()))
            }
            Self::Bearer(token) => format!("Bearer {}", token.expose()),
        });
        let mut value = HeaderValue::from_str(&text).unwrap_or_else(|_| HeaderValue::from_static(""));
        value.set_sensitive(true);
        value
    }
}

/// Tek bir HTTP isteği. Kimlik taşımaz; kimlikli gönderim [`OriginBoundClient::send`].
#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub url: Url,
    headers: Vec<(HeaderName, HeaderValue)>,
    pub body: Vec<u8>,
    /// Açılmış (gzip sonrası) gövde sınırı.
    pub max_body: u64,
    /// Yönlendirmeler dahil toplam süre.
    pub timeout: Duration,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    /// Yalnızca `https://` (ilk adres ve her yönlendirme).
    pub https_only: bool,
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("scheme", &self.url.scheme())
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

impl Request {
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: Vec::new(),
            max_body: DEFAULT_MAX_BODY,
            timeout: DEFAULT_TIMEOUT,
            follow_redirects: true,
            max_redirects: MAX_REDIRECTS,
            https_only: false,
        }
    }

    pub fn get(url: Url) -> Self {
        Self::new(Method::GET, url)
    }

    /// Başka bir uygulamanın verdiği medya adresi (MPRIS `artUrl` gibi): GET, yalnızca https,
    /// 8 MiB, toplam 5 sn, en çok 3 yönlendirme. [`HttpClient::fetch_untrusted_media`] bunu
    /// [`Trust::Server`] ile gönderir. `Content-Type`'a güvenilmez: çağıran baytları görsel
    /// imzasıyla doğrular ve güvenilmeyen kaynak olarak çözer.
    pub fn untrusted_media(url: Url) -> Self {
        let mut req = Self::get(url).max_body(MEDIA_MAX_BODY).timeout(MEDIA_TIMEOUT);
        req.max_redirects = MEDIA_MAX_REDIRECTS;
        req.https_only = true;
        req
    }

    pub fn post_form(url: Url, fields: &[(&str, &str)]) -> Self {
        let body = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(fields).finish();
        Self::new(Method::POST, url)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body.into_bytes())
    }

    /// WebDAV/CalDAV yöntemi (`PROPFIND`, `REPORT`); XML gövde ve `Depth` başlığıyla.
    pub fn dav(method: &str, url: Url, depth: Option<&str>, xml: &str) -> Self {
        let method = Method::from_bytes(method.as_bytes()).expect("sabit WebDAV yöntemi");
        let mut req = Self::new(method, url)
            .header("content-type", "application/xml; charset=utf-8")
            .body(xml.as_bytes().to_vec());
        if let Some(depth) = depth {
            req = req.header("depth", depth);
        }
        req
    }

    /// Başlık ekler. `Authorization`/`Cookie`/`Proxy-Authorization` **eklenemez** (kimlik
    /// yalnızca [`OriginBoundClient`] ile gider). Geçersiz ad/değer programcı hatasıdır.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let lower = name.to_ascii_lowercase();
        if matches!(lower.as_str(), "authorization" | "proxy-authorization" | "cookie") {
            debug_assert!(false, "kimlik başlığı Request::header ile eklenemez");
            return self;
        }
        match (HeaderName::from_bytes(name.as_bytes()), HeaderValue::from_str(value)) {
            (Ok(n), Ok(v)) => self.headers.push((n, v)),
            _ => debug_assert!(false, "geçersiz başlık"),
        }
        self
    }

    pub fn body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    pub fn max_body(mut self, max: u64) -> Self {
        self.max_body = max;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn no_redirects(mut self) -> Self {
        self.follow_redirects = false;
        self
    }
}

/// Tam okunmuş yanıt.
#[derive(Clone)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    /// Yönlendirmeler izlendiyse son adres.
    pub url: Url,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

/// Ağ güvenlik politikası. Varsayılan en sıkı olandır.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Policy {
    /// Geliştirme/test: `http://localhost` ve `127.0.0.0/8` kökenine kimlik gönderilebilir ve
    /// başka loopback kökenine yönlendirme izlenir. Üretimde kapalı.
    pub allow_insecure_loopback: bool,
    /// Bu istemcideki her istek yalnızca `https://` (istek düzeyindeki [`Request::https_only`]
    /// ile birleşir).
    pub https_only: bool,
}

/// Bir hedefe ne kadar güvenildiği: hangi adres sınıflarına bağlanılabileceğini belirler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Kullanıcının kendisinin girdiği ya da açıkça onayladığı adres: genel, LAN, loopback
    /// serbest; link-local / meta veri / belgeleme / ayrılmış hiçbir zaman.
    User,
    /// Sunucunun yönlendirdiği başka köken, sabit sağlayıcı API'si ya da başka bir uygulamanın
    /// verdiği adres: yalnızca genel internet (geliştirme bayrağıyla loopback).
    Server,
}

/// Ad çözümleyici (testlerde sahtesi verilir).
pub trait Lookup: Send + Sync + fmt::Debug + 'static {
    fn lookup(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>>;
}

/// İşletim sistemi çözümleyicisi (`getaddrinfo`).
#[derive(Debug, Default)]
pub struct SystemLookup;

impl Lookup for SystemLookup {
    fn lookup(&self, host: &str, port: u16) -> std::io::Result<Vec<IpAddr>> {
        Ok((host, port).to_socket_addrs()?.map(|a| a.ip()).collect())
    }
}

thread_local! {
    /// Bu thread'deki bir sonraki bağlantı için denetlenmiş adresler: `(ana makine, port, adresler)`.
    /// ureq'in çözümleyicisi ([`PinnedResolver`]) yalnızca bunu döndürür, kendisi çözmez.
    static PINNED: RefCell<Option<(String, u16, Vec<SocketAddr>)>> = const { RefCell::new(None) };
}

/// ureq çözümleyicisi: yalnızca bu thread için sabitlenmiş (denetlenmiş) adresleri verir. Böylece
/// "çöz → denetle → bağlan" arasında ikinci bir DNS sorgusu olmaz (DNS rebinding kapalı). TLS SNI ve
/// `Host` başlığı URL'deki özgün ana makine adıyla kalır.
#[derive(Debug)]
struct PinnedResolver;

impl ureq::unversioned::resolver::Resolver for PinnedResolver {
    fn resolve(
        &self,
        uri: &ureq::http::Uri,
        _config: &ureq::config::Config,
        _timeout: ureq::unversioned::transport::NextTimeout,
    ) -> Result<ureq::unversioned::resolver::ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        let port = uri.port_u16().or_else(|| match uri.scheme_str() {
            Some("https") => Some(443),
            Some("http") => Some(80),
            _ => None,
        });
        PINNED.with(|p| {
            let p = p.borrow();
            match (&*p, port) {
                (Some((h, pp, addrs)), Some(port)) if *h == host && *pp == port && !addrs.is_empty() => {
                    let mut out = self.empty();
                    for a in addrs.iter().take(16) {
                        out.push(*a);
                    }
                    Ok(out)
                }
                // Sabitlenmemiş hedef (vekil dahil): asla kendimiz çözmeyiz.
                _ => Err(ureq::Error::HostNotFound),
            }
        })
    }
}

/// Paylaşılan HTTP istemcisi. `Clone` ucuzdur. Kimlik taşımaz.
#[derive(Clone)]
pub struct HttpClient {
    agent: ureq::Agent,
    policy: Policy,
    lookup: Arc<dyn Lookup>,
}

impl fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpClient").field("policy", &self.policy).finish()
    }
}

impl Default for HttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient {
    pub fn new() -> Self {
        Self::with_policy(Policy::default())
    }

    pub fn with_policy(policy: Policy) -> Self {
        Self::with_lookup(policy, Arc::new(SystemLookup))
    }

    /// Özel çözümleyiciyle (testler).
    pub fn with_lookup(policy: Policy, lookup: Arc<dyn Lookup>) -> Self {
        Self::build(policy, lookup, Roots::platform())
    }

    /// **Yalnızca testler:** sistem kökleri yerine verilen test CA'sına güvenir (yerel
    /// kendinden imzalı TLS sunucusu). `test-support` özelliği olmadan derlenmez; üretim
    /// ikilisinde özel CA kabul etmenin hiçbir yolu yoktur.
    #[cfg(all(feature = "tls", any(test, feature = "test-support")))]
    pub fn with_test_root_ca(policy: Policy, ca_der: &[u8]) -> Self {
        Self::with_test_tls(policy, ca_der, Arc::new(SystemLookup))
    }

    /// **Yalnızca testler:** [`HttpClient::with_test_root_ca`] + özel çözümleyici; ör.
    /// `testing::LoopbackLookup` ile `https://nuxut.test:<port>/` sahte TLS sunucusuna gider
    /// (örnek: `testing::FakeServer::start_tls`).
    #[cfg(all(feature = "tls", any(test, feature = "test-support")))]
    pub fn with_test_tls(policy: Policy, ca_der: &[u8], lookup: Arc<dyn Lookup>) -> Self {
        let cert = ureq::tls::Certificate::from_der(ca_der).to_owned();
        Self::build(policy, lookup, Roots(ureq::tls::RootCerts::Specific(Arc::new(vec![cert]))))
    }

    fn build(policy: Policy, lookup: Arc<dyn Lookup>, roots: Roots) -> Self {
        let builder = ureq::Agent::config_builder();
        #[cfg(feature = "tls")]
        let builder = builder.tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::Rustls)
                .root_certs(roots.0)
                .build(),
        );
        #[cfg(not(feature = "tls"))]
        let Roots() = roots;
        let config = builder
            // Ortamdaki vekil yok sayılır: vekil hedefi kendisi çözer, sabitleme atlanırdı.
            .proxy(None)
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .allow_non_standard_methods(true)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .max_response_header_size(64 * 1024)
            // Havuz kapalı: havuzdaki bağlantı ana makine adına göre seçilir, adrese göre değil;
            // başka güven düzeyinde denetlenmiş eski bir bağlantının yeniden kullanılması DNS
            // sabitlemesini atlatırdı.
            .max_idle_connections(0)
            .max_idle_connections_per_host(0)
            .build();
        // `DefaultConnector` yerine açık zincir: TCP → EINTR yeniden denemesi → TLS. Vekil kapalı
        // olduğundan SOCKS/CONNECT halkaları gereksiz. `crate::transport` kök nedeni anlatır.
        use ureq::unversioned::transport::{Connector, TcpConnector};
        let connector = ().chain(TcpConnector::default()).chain(crate::transport::RetryInterrupted);
        #[cfg(feature = "tls")]
        let connector = connector.chain(ureq::unversioned::transport::RustlsConnector::default());
        let agent = ureq::Agent::with_parts(config, connector, PinnedResolver);
        Self { agent, policy, lookup }
    }

    pub fn policy(&self) -> Policy {
        self.policy
    }

    /// Kimliksiz istek. İlk adres kullanıcınındır ([`Trust::User`]); başka kökene yönlendirmede
    /// hedef [`Trust::Server`] kurallarıyla denetlenir.
    pub fn send(&self, req: Request) -> Result<Response, HttpError> {
        self.run(req, None, Trust::User)
    }

    /// Kimliksiz istek, açık güven düzeyiyle: dışarıdan gelen adresler [`Trust::Server`] ile
    /// gönderilir — yalnızca genel internet, yönlendirmeler dahil.
    pub fn send_as(&self, req: Request, trust: Trust) -> Result<Response, HttpError> {
        self.run(req, None, trust)
    }

    /// Başka bir uygulamanın verdiği medya adresini indirir ([`Request::untrusted_media`] +
    /// [`Trust::Server`]). Baytlar güvenilmezdir: görsel imzasıyla doğrulanıp güvenilmeyen
    /// kaynak olarak çözülmelidir.
    pub fn fetch_untrusted_media(&self, url: Url) -> Result<Response, HttpError> {
        self.send_as(Request::untrusted_media(url), Trust::Server)
    }

    /// Hedefin adreslerini çözer ve güven düzeyine göre denetler. Tek bir adres bile yasak
    /// sınıftaysa hedef reddedilir (karışık yanıtla atlatma olmasın).
    pub fn vet(&self, url: &Url, trust: Trust) -> Result<Vec<SocketAddr>, HttpError> {
        let host_str = || url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().ok_or(HttpError::BadUrl)?;
        let ips: Vec<IpAddr> = match url.host() {
            Some(url::Host::Ipv4(ip)) => vec![IpAddr::V4(ip)],
            Some(url::Host::Ipv6(ip)) => vec![IpAddr::V6(ip)],
            Some(url::Host::Domain(d)) if is_localhost_name(d) => vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
            Some(url::Host::Domain(d)) => self.lookup.lookup(d, port).map_err(|_| HttpError::HostNotFound)?,
            None => return Err(HttpError::BadUrl),
        };
        if ips.is_empty() {
            return Err(HttpError::HostNotFound);
        }
        for ip in &ips {
            let allowed = match (classify(*ip), trust) {
                (IpClass::Never, _) => false,
                (IpClass::Public, _) => true,
                (IpClass::Private | IpClass::Loopback, Trust::User) => true,
                (IpClass::Loopback, Trust::Server) => self.policy.allow_insecure_loopback,
                (IpClass::Private, Trust::Server) => false,
            };
            if !allowed {
                return Err(HttpError::ForbiddenTarget(host_str()));
            }
        }
        Ok(ips.into_iter().map(|ip| SocketAddr::new(ip, port)).collect())
    }

    /// Şema ve URL kimliği denetimi (ilk adres ve her yönlendirme).
    fn check_url(&self, url: &Url, https_only: bool) -> Result<(), HttpError> {
        match url.scheme() {
            "https" => {}
            "http" if https_only || self.policy.https_only => return Err(HttpError::InsecureScheme),
            "http" => {}
            _ => return Err(HttpError::BadUrl),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(HttpError::CredentialsInUrl);
        }
        Ok(())
    }

    /// Ortak döngü. `bound` verilirse: kimlik eklenir, yalnızca aynı kökende yönlendirme izlenir.
    fn run(&self, mut req: Request, bound: Option<(&Origin, &Auth)>, mut trust: Trust) -> Result<Response, HttpError> {
        self.check_url(&req.url, req.https_only)?;
        let deadline = std::time::Instant::now() + req.timeout;
        let mut redirects = 0;
        loop {
            let auth = match bound {
                Some((origin, auth)) if &req.url.origin() == origin => Some(auth),
                Some(_) => return Err(HttpError::OriginMismatch(req.url.clone())),
                None => None,
            };
            let addrs = self.vet(&req.url, trust)?;
            // Toplam süre yönlendirmeler boyunca paylaşılır.
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(HttpError::Timeout);
            }
            let resp = self.send_pinned(&req, auth, addrs, left)?;
            let status = resp.status;
            if !(req.follow_redirects && status.is_redirection() && status != StatusCode::NOT_MODIFIED) {
                return Ok(resp);
            }
            let Some(location) = resp.header("location") else { return Ok(resp) };
            let next = req.url.join(location).map_err(|_| HttpError::BadUrl)?;
            if req.url.scheme() == "https" && next.scheme() != "https" {
                return Err(HttpError::Downgrade);
            }
            self.check_url(&next, req.https_only)?;
            redirects += 1;
            if redirects > req.max_redirects {
                return Err(HttpError::TooManyRedirects);
            }
            if let Some((origin, _)) = bound
                && &next.origin() != origin
            {
                // Kimlikli istek köken dışına çıkmaz; karar (kullanıcı onayı) çağıranındır.
                return Err(HttpError::CrossOriginRedirect(next));
            }
            // 303 her zaman GET'e döner; 301/302'de POST da GET olur (tarayıcı davranışı).
            // PROPFIND/REPORT gibi diğer yöntemler korunur (RFC 6764 .well-known yönlendirmesi).
            let to_get = status == StatusCode::SEE_OTHER || (req.method == Method::POST && matches!(status.as_u16(), 301 | 302));
            if to_get {
                req.method = Method::GET;
                req.body.clear();
                req.headers.retain(|(n, _)| n != http::header::CONTENT_TYPE);
            }
            if next.origin() != req.url.origin() {
                // Gövde (OAuth `client_secret`/`code`, form) sunucunun seçtiği başka kökene
                // taşınmaz (307/308); karar çağıranın.
                if !req.body.is_empty() {
                    return Err(HttpError::CrossOriginRedirect(next));
                }
                // Sunucunun seçtiği başka köken: yalnızca genel internet.
                trust = Trust::Server;
            }
            req.url = next;
        }
    }

    /// Denetlenmiş adresleri bu thread için sabitleyip tek istek gönderir.
    fn send_pinned(&self, req: &Request, auth: Option<&Auth>, addrs: Vec<SocketAddr>, timeout: Duration) -> Result<Response, HttpError> {
        let host = req.url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        let port = req.url.port_or_known_default().unwrap_or(443);
        PINNED.with(|p| *p.borrow_mut() = Some((host, port, addrs)));
        let result = self.send_once(req, auth, timeout);
        PINNED.with(|p| *p.borrow_mut() = None);
        result
    }

    fn send_once(&self, req: &Request, auth: Option<&Auth>, timeout: Duration) -> Result<Response, HttpError> {
        let mut builder = http::Request::builder().method(req.method.clone()).uri(req.url.as_str());
        for (name, value) in &req.headers {
            builder = builder.header(name, value);
        }
        if let Some(auth) = auth {
            builder = builder.header(http::header::AUTHORIZATION, auth.header_value());
        }
        let request = builder.body(req.body.clone()).map_err(|_| HttpError::BadUrl)?;
        let request = self.agent.configure_request(request).timeout_global(Some(timeout)).build();
        let mut response = self.agent.run(request).map_err(HttpError::from)?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        // Okuyucu gzip'i açar; sınır AÇILMIŞ baytlara uygulanır (sıkıştırma bombası). Sınırın bir
        // bayt fazlasını okumaya çalış: dolarsa gövde sınırı aştı demektir.
        let reader = response.body_mut().with_config().limit(req.max_body.saturating_add(1)).reader();
        // `read_to_end` `Interrupted`'ı kendisi yineler; alttaki taşıma da kalan süreyle yineler.
        reader.take(req.max_body.saturating_add(1)).read_to_end(&mut body).map_err(|e| match e.kind() {
            k if is_timeout_kind(k) => HttpError::Timeout,
            k => HttpError::Io(k),
        })?;
        if body.len() as u64 > req.max_body {
            return Err(HttpError::TooLarge(req.max_body));
        }
        Ok(Response { status, headers, body, url: req.url.clone() })
    }
}

/// Tek bir kökene (şema + ana makine + port) bağlı, kimlik taşıyan istemci. `Authorization`
/// başlığı yalnızca buradan ve yalnızca bağlı kökene gider.
#[derive(Clone)]
pub struct OriginBoundClient {
    http: HttpClient,
    origin: Origin,
    base: Url,
    auth: Auth,
    trust: Trust,
}

impl fmt::Debug for OriginBoundClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OriginBoundClient")
            .field("trust", &self.trust)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}

impl OriginBoundClient {
    /// `base`'in kökenine bağlanır (kullanıcının girdiği/onayladığı adres, [`Trust::User`]).
    /// Kimlik yalnızca https üzerinden (ya da geliştirme bayrağıyla loopback http); aksi
    /// [`HttpError::InsecureAuth`].
    pub fn new(http: HttpClient, base: &Url, auth: Auth) -> Result<Self, HttpError> {
        match base.scheme() {
            "https" => {}
            "http" if http.policy.allow_insecure_loopback && is_loopback(base) => {}
            "http" => return Err(HttpError::InsecureAuth),
            _ => return Err(HttpError::BadUrl),
        }
        if base.host().is_none() {
            return Err(HttpError::BadUrl);
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(HttpError::CredentialsInUrl);
        }
        Ok(Self { http, origin: base.origin(), base: base.clone(), auth, trust: Trust::User })
    }

    /// Kullanıcının seçmediği hedefler (sabit sağlayıcı API'si, sağlayıcı içi geçiş): yalnızca
    /// genel internet adreslerine bağlanılır.
    pub fn with_server_trust(mut self) -> Self {
        self.trust = Trust::Server;
        self
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    /// Bağlanılan ilk adres.
    pub fn base(&self) -> &Url {
        &self.base
    }

    /// Aynı kimlikle başka bir kökene yeniden bağlanır (yalnızca kullanıcı onayından sonra).
    pub fn rebind(&self, base: &Url) -> Result<Self, HttpError> {
        Self::new(self.http.clone(), base, self.auth.clone())
    }

    /// Sunucudan gelen href'i `base`'e göre çözer ve bağlı kökende olduğunu doğrular.
    pub fn resolve(&self, base: &Url, href: &str) -> Result<Url, HttpError> {
        let url = base.join(href).map_err(|_| HttpError::BadUrl)?;
        if url.origin() != self.origin {
            return Err(HttpError::OriginMismatch(url));
        }
        Ok(url)
    }

    pub fn is_same_origin(&self, url: &Url) -> bool {
        url.origin() == self.origin
    }

    /// Kimlikli gönderim. URL bağlı kökende değilse hiçbir bayt gönderilmeden reddedilir;
    /// köken dışı yönlendirme [`HttpError::CrossOriginRedirect`] olarak durur.
    pub fn send(&self, req: Request) -> Result<Response, HttpError> {
        if req.url.origin() != self.origin {
            return Err(HttpError::OriginMismatch(req.url));
        }
        self.http.run(req, Some((&self.origin, &self.auth)), self.trust)
    }
}

/// TLS kök sertifikaları (`tls` özelliği kapalıyken boş: yalnızca çapraz denetim için).
struct Roots(#[cfg(feature = "tls")] ureq::tls::RootCerts);

impl Roots {
    fn platform() -> Self {
        Self(
            #[cfg(feature = "tls")]
            ureq::tls::RootCerts::PlatformVerifier,
        )
    }
}

fn is_localhost_name(d: &str) -> bool {
    let d = d.trim_end_matches('.').to_ascii_lowercase();
    d == "localhost" || d.ends_with(".localhost")
}

/// `localhost`, `127.0.0.0/8` veya `::1`.
pub fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => is_localhost_name(d),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
