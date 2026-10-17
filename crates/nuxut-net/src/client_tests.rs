use std::io::Write;

use super::*;
use crate::testing::{FakeServer, Reply};

fn dev() -> HttpClient {
    HttpClient::with_policy(Policy { allow_insecure_loopback: true, ..Policy::default() })
}

fn basic() -> Auth {
    Auth::Basic { user: "u".into(), password: Secret::new("p".into()) }
}

#[test]
fn follows_same_origin_redirect_keeping_method_and_auth() {
    let server = FakeServer::start(|req| match req.path.as_str() {
        "/.well-known/caldav" => Reply::status(301).header("location", "/dav/"),
        "/dav/" => {
            assert_eq!(req.method, "PROPFIND");
            assert!(req.header("authorization").is_some());
            Reply::status(207).body("<ok/>")
        }
        _ => Reply::status(404),
    });
    let client = OriginBoundClient::new(dev(), server.base(), basic()).unwrap();
    let resp = client.send(Request::dav("PROPFIND", server.url("/.well-known/caldav"), Some("0"), "<x/>")).unwrap();
    assert_eq!(resp.status.as_u16(), 207);
    assert_eq!(resp.url.path(), "/dav/");
}

#[test]
fn bound_client_stops_at_cross_origin_redirect_without_sending_credentials() {
    let target = FakeServer::start(|_| panic!("kimlikli istek başka köke gitmemeli"));
    let target_url = target.url("/x").to_string();
    let source = FakeServer::start(move |_| Reply::status(302).header("location", &target_url));
    let client = OriginBoundClient::new(dev(), source.base(), basic()).unwrap();
    let err = client.send(Request::get(source.url("/"))).unwrap_err();
    assert!(matches!(err, HttpError::CrossOriginRedirect(ref u) if u.port() == target.base().port()), "{err:?}");
    assert_eq!(err.target_url().and_then(Url::port), target.base().port());
    assert!(target.requests().is_empty());
}

#[test]
fn bound_client_refuses_foreign_urls_and_hrefs_before_sending() {
    let server = FakeServer::start(|_| Reply::status(200));
    let client = OriginBoundClient::new(dev(), server.base(), basic()).unwrap();
    let foreign = Url::parse("https://evil.example/steal").unwrap();
    assert!(matches!(client.send(Request::get(foreign.clone())), Err(HttpError::OriginMismatch(_))));
    assert!(matches!(client.resolve(server.base(), "https://evil.example/cal/"), Err(HttpError::OriginMismatch(_))));
    assert!(matches!(client.resolve(server.base(), "//evil.example/cal/"), Err(HttpError::OriginMismatch(_))));
    assert!(client.resolve(server.base(), "/remote.php/dav/").is_ok());
    assert!(server.requests().is_empty());
}

#[test]
fn credentials_require_https_unless_dev_loopback() {
    let http_lan = Url::parse("http://192.168.1.10/dav/").unwrap();
    assert!(matches!(OriginBoundClient::new(HttpClient::new(), &http_lan, basic()), Err(HttpError::InsecureAuth)));
    assert!(matches!(OriginBoundClient::new(dev(), &http_lan, basic()), Err(HttpError::InsecureAuth)));
    let local = Url::parse("http://127.0.0.1:5232/").unwrap();
    assert!(matches!(OriginBoundClient::new(HttpClient::new(), &local, basic()), Err(HttpError::InsecureAuth)));
    assert!(OriginBoundClient::new(dev(), &local, basic()).is_ok());
    assert!(OriginBoundClient::new(HttpClient::new(), &Url::parse("https://192.168.1.10/").unwrap(), basic()).is_ok());
}

#[test]
fn redirect_to_metadata_or_private_targets_is_refused_even_without_credentials() {
    for target in ["http://169.254.169.254/latest/meta-data/", "http://10.0.0.5/", "http://[::ffff:127.0.0.1]/", "http://[fe80::1]/"] {
        let t = target.to_string();
        let source = FakeServer::start(move |_| Reply::status(302).header("location", &t));
        let err = HttpClient::new().send(Request::get(source.url("/feed.ics"))).unwrap_err();
        assert!(matches!(err, HttpError::ForbiddenTarget(_)), "{target}: {err:?}");
    }
    // Başka bir loopback kökene yönlendirme de (üretim politikası).
    let other = FakeServer::start(|_| Reply::status(200));
    let o = other.url("/").to_string();
    let source = FakeServer::start(move |_| Reply::status(302).header("location", &o));
    assert!(matches!(HttpClient::new().send(Request::get(source.url("/"))), Err(HttpError::ForbiddenTarget(_))));
    assert!(other.requests().is_empty());
}

#[test]
fn user_entered_link_local_is_refused_but_lan_is_allowed() {
    let c = HttpClient::new();
    let u = |s: &str| Url::parse(s).unwrap();
    assert!(c.vet(&u("http://169.254.169.254/"), Trust::User).is_err());
    assert!(c.vet(&u("http://0.0.0.0/"), Trust::User).is_err());
    assert!(c.vet(&u("https://192.168.1.10/"), Trust::User).is_ok());
    assert!(c.vet(&u("https://192.168.1.10/"), Trust::Server).is_err());
    assert!(c.vet(&u("https://[::ffff:192.168.1.10]/"), Trust::Server).is_err());
}

/// İlk çözümde bir adres, sonrakilerde başka (saldırganın) adres döndüren çözümleyici.
#[derive(Debug)]
struct Rebinding {
    first: IpAddr,
    then: IpAddr,
    calls: std::sync::atomic::AtomicUsize,
}

impl Lookup for Rebinding {
    fn lookup(&self, _host: &str, _port: u16) -> std::io::Result<Vec<IpAddr>> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(vec![if n == 0 { self.first } else { self.then }])
    }
}

#[test]
fn dns_rebinding_cannot_change_the_address_after_validation() {
    let server = FakeServer::start(|req| {
        assert_eq!(req.header("host").unwrap().split(':').next(), Some("rebind.example"), "Host özgün ad kalır");
        Reply::status(200).body("doğrulanmış adres")
    });
    let lookup = Arc::new(Rebinding {
        first: "127.0.0.1".parse().unwrap(),
        then: "169.254.169.254".parse().unwrap(),
        calls: Default::default(),
    });
    let client = HttpClient::with_lookup(Policy::default(), lookup.clone());
    let url = Url::parse(&format!("http://rebind.example:{}/", server.base().port().unwrap())).unwrap();
    let resp = client.send(Request::get(url)).unwrap();
    assert_eq!(resp.text(), "doğrulanmış adres");
    assert_eq!(lookup.calls.load(std::sync::atomic::Ordering::SeqCst), 1, "tek çözüm: çöz → denetle → o adrese bağlan");
}

#[test]
fn redirect_to_name_resolving_to_internal_address_is_refused() {
    let lookup = Arc::new(Rebinding { first: "10.1.2.3".parse().unwrap(), then: "10.1.2.3".parse().unwrap(), calls: Default::default() });
    let source = FakeServer::start(|_| Reply::status(302).header("location", "https://intranet.example/admin"));
    let client = HttpClient::with_lookup(Policy::default(), lookup);
    let err = client.send(Request::get(source.url("/"))).unwrap_err();
    assert_eq!(err.forbidden_host(), Some("intranet.example"), "{err:?}");
}

/// Gerçek ağ: sabitlenmiş çözümle TLS (SNI + sertifika doğrulama) çalışıyor mu.
/// Elle: `cargo test -p nuxut-net -- --ignored real_https`.
#[test]
#[ignore = "internet gerektirir"]
fn real_https_with_pinned_resolution() {
    let resp = HttpClient::new().send(Request::get(Url::parse("https://example.com/").unwrap())).unwrap();
    assert!(resp.is_success(), "{}", resp.status);
    assert!(matches!(
        HttpClient::new().send(Request::get(Url::parse("https://wrong.host.badssl.com/").unwrap())),
        Err(HttpError::Tls | HttpError::Io(_) | HttpError::Protocol)
    ));
}

#[test]
fn mixed_answers_are_refused_entirely() {
    #[derive(Debug)]
    struct Mixed;
    impl Lookup for Mixed {
        fn lookup(&self, _: &str, _: u16) -> std::io::Result<Vec<IpAddr>> {
            Ok(vec!["93.184.216.34".parse().unwrap(), "127.0.0.1".parse().unwrap()])
        }
    }
    let client = HttpClient::with_lookup(Policy::default(), Arc::new(Mixed));
    assert!(client.vet(&Url::parse("https://mixed.example/").unwrap(), Trust::Server).is_err());
}

#[test]
fn authorization_header_cannot_be_added_manually() {
    // Release derlemesinde sessizce atlanır; debug'da programcı hatası.
    let r = std::panic::catch_unwind(|| Request::get(Url::parse("https://x/").unwrap()).header("Authorization", "Basic x"));
    if let Ok(req) = r {
        assert!(req.headers.is_empty());
    }
}

#[test]
fn enforces_body_limit() {
    let server = FakeServer::start(|_| Reply::status(200).body(&"x".repeat(2048)));
    let err = HttpClient::new().send(Request::get(server.url("/")).max_body(1024)).unwrap_err();
    assert!(matches!(err, HttpError::TooLarge(1024)));
    let ok = HttpClient::new().send(Request::get(server.url("/")).max_body(2048)).unwrap();
    assert_eq!(ok.body.len(), 2048);
}

#[test]
fn redirect_loop_is_bounded() {
    let server = FakeServer::start(|_| Reply::status(302).header("location", "/again"));
    let err = HttpClient::new().send(Request::get(server.url("/"))).unwrap_err();
    assert!(matches!(err, HttpError::TooManyRedirects));
}

#[test]
fn times_out_on_silent_server() {
    let server = FakeServer::start(|_| Reply::status(200).delay(Duration::from_secs(3)));
    let err = HttpClient::new().send(Request::get(server.url("/")).timeout(Duration::from_millis(300))).unwrap_err();
    assert!(matches!(err, HttpError::Timeout), "{err:?}");
}

/// Soket zaman aşımı türlerinin ikisi de (Unix `WouldBlock`, Windows `TimedOut`) `Timeout`'tur;
/// `Interrupted` taşımada yinelenir, buraya ulaşırsa zaman aşımı sayılmaz.
#[test]
fn socket_timeout_kinds_map_to_timeout() {
    use std::io::ErrorKind;
    for kind in [ErrorKind::WouldBlock, ErrorKind::TimedOut] {
        let err = HttpError::from(ureq::Error::Io(kind.into()));
        assert!(matches!(err, HttpError::Timeout), "{kind:?} → {err:?}");
    }
    let err = HttpError::from(ureq::Error::Io(ErrorKind::Interrupted.into()));
    assert!(matches!(err, HttpError::Io(ErrorKind::Interrupted)), "{err:?}");
}

#[test]
fn debug_output_never_contains_secrets() {
    let client = OriginBoundClient::new(dev(), &Url::parse("http://127.0.0.1/").unwrap(), Auth::Bearer("tok-SECRET".into())).unwrap();
    let shown = format!("{client:?} {:?}", basic());
    assert!(!shown.contains("SECRET") && !shown.contains("\"p\"") && !shown.contains("\"u\""), "{shown}");
}

// ---- Taşımada eklenenler -------------------------------------------------------------------

/// Hata, istek ve yanıt metinleri URL, ana makine ya da gövde taşımaz (değişmez 9).
#[test]
fn errors_requests_and_responses_do_not_print_urls_or_hosts() {
    let url = Url::parse("https://gizli-host.example/yol?token=abc").unwrap();
    let errs = [
        HttpError::ForbiddenTarget("gizli-host.example".into()),
        HttpError::OriginMismatch(url.clone()),
        HttpError::CrossOriginRedirect(url.clone()),
        HttpError::BadUrl,
        HttpError::Io(std::io::ErrorKind::ConnectionReset),
    ];
    for e in &errs {
        let shown = format!("{e} {e:?}");
        assert!(!shown.contains("gizli") && !shown.contains("token"), "{shown}");
    }
    let req = Request::get(url.clone()).body(b"govde-icerigi".to_vec());
    let resp = Response { status: StatusCode::OK, headers: HeaderMap::new(), body: b"yanit-icerigi".to_vec(), url };
    let shown = format!("{req:?} {resp:?}");
    assert!(!shown.contains("gizli") && !shown.contains("icerigi"), "{shown}");
}

/// `user:parola@` içeren adres: ne ilk istekte ne yönlendirmede gönderilir.
#[test]
fn credentials_in_url_are_refused_initially_and_on_redirect() {
    let target = FakeServer::start(|_| Reply::status(200));
    let mut with_creds = target.url("/x");
    with_creds.set_username("u").unwrap();
    with_creds.set_password(Some("p")).unwrap();
    assert!(matches!(dev().send(Request::get(with_creds.clone())), Err(HttpError::CredentialsInUrl)));
    assert!(matches!(OriginBoundClient::new(dev(), &with_creds, basic()), Err(HttpError::CredentialsInUrl)));
    let loc = with_creds.to_string();
    let source = FakeServer::start(move |_| Reply::status(302).header("location", &loc));
    assert!(matches!(dev().send(Request::get(source.url("/"))), Err(HttpError::CredentialsInUrl)));
    assert!(target.requests().is_empty());
}

/// Göreli ve şemasız (`//host`) Location her adımda yeniden çözülür ve denetlenir.
#[test]
fn relative_and_scheme_relative_redirects_are_vetted() {
    let other = FakeServer::start(|_| Reply::status(200).body("öteki"));
    let loc = format!("//127.0.0.1:{}/x", other.base().port().unwrap());
    let source = FakeServer::start(move |req| match req.path.as_str() {
        "/" => Reply::status(302).header("location", "./ara"),
        "/ara" => Reply::status(302).header("location", &loc),
        _ => Reply::status(404),
    });
    // Üretim: şemasız yönlendirme başka loopback kökene → Trust::Server → reddedilir.
    assert!(matches!(HttpClient::new().send(Request::get(source.url("/"))), Err(HttpError::ForbiddenTarget(_))));
    assert!(other.requests().is_empty());
    // Geliştirme bayrağıyla aynı zincir izlenir (göreli → şemasız → hedef).
    let resp = dev().send(Request::get(source.url("/"))).unwrap();
    assert_eq!(resp.text(), "öteki");
}

/// `https_only`: http hedefe hiç bağlanılmaz (istek ya da politika düzeyinde).
#[test]
fn https_only_refuses_plain_http() {
    let server = FakeServer::start(|_| Reply::status(200));
    assert!(matches!(dev().fetch_untrusted_media(server.url("/kapak.jpg")), Err(HttpError::InsecureScheme)));
    let strict = HttpClient::with_policy(Policy { allow_insecure_loopback: true, https_only: true });
    assert!(matches!(strict.send(Request::get(server.url("/"))), Err(HttpError::InsecureScheme)));
    assert!(server.requests().is_empty());
    let media = Request::untrusted_media(Url::parse("https://cdn.example/a.jpg").unwrap());
    assert!(media.https_only && media.max_body == MEDIA_MAX_BODY && media.timeout == MEDIA_TIMEOUT);
    assert_eq!(media.max_redirects, MEDIA_MAX_REDIRECTS);
}

/// Güvenilmeyen medya başka uygulamanın adresidir: LAN/loopback'e gidemez.
#[test]
fn untrusted_media_never_reaches_internal_addresses() {
    let client = HttpClient::with_lookup(Policy::default(), Arc::new(Rebinding {
        first: "192.168.1.5".parse().unwrap(),
        then: "192.168.1.5".parse().unwrap(),
        calls: Default::default(),
    }));
    let err = client.fetch_untrusted_media(Url::parse("https://router.example/kapak.png").unwrap()).unwrap_err();
    assert!(matches!(err, HttpError::ForbiddenTarget(_)), "{err:?}");
    let err = HttpClient::new().fetch_untrusted_media(Url::parse("https://127.0.0.1/kapak.png").unwrap()).unwrap_err();
    assert!(matches!(err, HttpError::ForbiddenTarget(_)), "{err:?}");
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// Sıkıştırma bombası: sınır açılmış boyuta uygulanır.
#[test]
fn body_limit_applies_to_decompressed_size() {
    let bomb = gzip(&vec![0u8; 8 * 1024 * 1024]);
    assert!(bomb.len() < 64 * 1024, "örnek gerçekten yüksek oranlı olmalı");
    let small = gzip(b"merhaba");
    let server = FakeServer::start(move |req| {
        let body = if req.path == "/bomba" { bomb.clone() } else { small.clone() };
        Reply::status(200).header("content-encoding", "gzip").bytes(body)
    });
    let err = HttpClient::new().send(Request::get(server.url("/bomba")).max_body(1024 * 1024)).unwrap_err();
    assert!(matches!(err, HttpError::TooLarge(_)), "{err:?}");
    let ok = HttpClient::new().send(Request::get(server.url("/kucuk"))).unwrap();
    assert_eq!(ok.text(), "merhaba");
}

/// Ortamdaki vekil yok sayılır: istek doğrudan denetlenmiş adrese gider, vekile hiçbir bayt
/// gitmez. Ortam değişkeni süreç genelidir; bu yüzden yardımcı test ayrı bir süreçte koşar.
#[test]
fn environment_proxy_is_ignored() {
    let proxy = FakeServer::start(|_| Reply::status(200).body("vekil"));
    let target = FakeServer::start(|_| Reply::status(200).body("hedef"));
    let exe = std::env::current_exe().unwrap();
    let proxy_url = proxy.base().to_string();
    let out = std::process::Command::new(exe)
        .args(["--exact", "client::tests::proxy_helper", "--ignored", "--nocapture", "--test-threads=1"])
        .env("NUXUT_NET_PROXY_TARGET", target.url("/").as_str())
        .env("HTTP_PROXY", &proxy_url)
        .env("http_proxy", &proxy_url)
        .env("HTTPS_PROXY", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success() && text.contains("1 passed"), "{text}");
    assert!(proxy.requests().is_empty(), "vekile istek gitti");
    assert_eq!(target.requests().len(), 1);
}

#[test]
#[ignore = "environment_proxy_is_ignored tarafından ayrı süreçte çalıştırılır"]
fn proxy_helper() {
    let Ok(target) = std::env::var("NUXUT_NET_PROXY_TARGET") else { return };
    let resp = dev().send(Request::get(Url::parse(&target).unwrap())).unwrap();
    assert_eq!(resp.text(), "hedef");
}

/// 307/308 gövdeyi korur: gövdeli istek (OAuth gizli anahtarı, form) başka kökene taşınmaz;
/// aynı kökende izlenir, 303'te gövde düşer ve başka kökene GET olarak gidilir.
#[test]
fn request_body_never_follows_a_cross_origin_redirect() {
    let other = FakeServer::start(|_| Reply::status(200).body("öteki"));
    let o = other.url("/token").to_string();
    let source = FakeServer::start(move |req| match req.path.as_str() {
        "/307" => Reply::status(307).header("location", &o),
        "/303" => Reply::status(303).header("location", &o),
        "/ayni" => Reply::status(308).header("location", "/son"),
        "/son" => {
            assert_eq!(req.form("client_secret").as_deref(), Some("gizli"));
            Reply::status(200).body("aynı")
        }
        _ => Reply::status(404),
    });
    let form = |path: &str| Request::post_form(source.url(path), &[("client_secret", "gizli")]);
    let err = dev().send(form("/307")).unwrap_err();
    assert!(matches!(err, HttpError::CrossOriginRedirect(_)), "{err:?}");
    assert!(other.requests().is_empty(), "gövde başka kökene gitmemeli");
    assert_eq!(dev().send(form("/ayni")).unwrap().text(), "aynı");
    assert_eq!(dev().send(form("/303")).unwrap().text(), "öteki");
    let seen = other.requests();
    assert_eq!((seen.len(), seen[0].method.as_str(), seen[0].body.len()), (1, "GET", 0));
}

// ---- TLS sahte sunucu (testing::FakeServer::start_tls) ----

#[cfg(feature = "tls")]
mod tls {
    use super::*;
    use crate::testing::{LoopbackLookup, OTHER_CA_DER, TEST_CA_DER};

    fn test_tls(ca: &[u8]) -> HttpClient {
        // Sunucunun verdiği adres (Trust::Server) loopback'e ancak bu bayrakla gidebilir.
        let policy = Policy { allow_insecure_loopback: true, ..Policy::default() };
        HttpClient::with_test_tls(policy, ca, Arc::new(LoopbackLookup))
    }

    /// TLS yolu da aynı süre kuralına uyar: rustls baytlarını EINTR'yi yineleyen taşımadan okur.
    #[test]
    fn tls_times_out_on_silent_server() {
        let server = FakeServer::start_tls(|_| Reply::status(200).delay(Duration::from_secs(3)));
        let err = test_tls(TEST_CA_DER).send(Request::get(server.url("/")).timeout(Duration::from_millis(300))).unwrap_err();
        assert!(matches!(err, HttpError::Timeout), "{err:?}");
    }

    #[test]
    fn https_only_fetch_succeeds_by_name_ip_and_localhost() {
        let server = FakeServer::start_tls(|req| match req.path.as_str() {
            "/cover.png" => Reply::status(200).header("content-type", "image/png").bytes(vec![0x89, b'P', b'N', b'G']),
            _ => Reply::status(404),
        });
        assert_eq!(server.base().scheme(), "https");
        let http = test_tls(TEST_CA_DER);
        // Medya profili: https_only + Trust::Server, ad LoopbackLookup ile 127.0.0.1.
        let resp = http.fetch_untrusted_media(server.url_at("nuxut.test", "/cover.png")).unwrap();
        assert_eq!(resp.status.as_u16(), 200);
        assert_eq!(resp.body, vec![0x89, b'P', b'N', b'G']);
        // Diğer SAN'lar: IP ve localhost.
        for url in [server.url("/cover.png"), server.url_at("localhost", "/cover.png")] {
            let mut req = Request::get(url);
            req.https_only = true;
            assert_eq!(http.send(req).unwrap().status.as_u16(), 200);
        }
        assert_eq!(server.requests().len(), 3);
        assert!(server.requests().iter().all(|r| r.method == "GET" && r.path == "/cover.png"));
    }

    #[test]
    fn wrong_ca_or_unlisted_name_is_tls_error_and_nothing_is_served() {
        let server = FakeServer::start_tls(|_| Reply::status(200).body("gizli"));
        let err = test_tls(OTHER_CA_DER).fetch_untrusted_media(server.url_at("nuxut.test", "/")).unwrap_err();
        assert!(matches!(err, HttpError::Tls), "{err:?}");
        // Doğru CA, ama sertifikada olmayan ad.
        let err = test_tls(TEST_CA_DER).fetch_untrusted_media(server.url_at("evil.test", "/")).unwrap_err();
        assert!(matches!(err, HttpError::Tls), "{err:?}");
        assert!(server.requests().is_empty(), "el sıkışması başarısızken istek okunmamalı");
    }

    #[test]
    fn never_falls_back_to_plain_http() {
        let http = test_tls(TEST_CA_DER);
        // https adresi düz HTTP sunucusuna: TLS kurulamaz, düz metinle yeniden denenmez.
        let plain = FakeServer::start(|_| Reply::status(200).body("düz"));
        let mut https_to_plain = plain.url("/x");
        https_to_plain.set_scheme("https").unwrap();
        assert!(http.fetch_untrusted_media(https_to_plain).is_err());
        assert!(plain.requests().iter().all(|r| r.method != "GET"), "düz HTTP isteği gönderilmemeli");
        // http adresi https_only istekte: hiç bağlanılmaz.
        let err = http.fetch_untrusted_media(plain.url_at("nuxut.test", "/x")).unwrap_err();
        assert!(matches!(err, HttpError::InsecureScheme), "{err:?}");
        // TLS sunucusu http'ye yönlendirirse izlenmez.
        let target = plain.url_at("nuxut.test", "/kapak").to_string();
        let tls = FakeServer::start_tls(move |_| Reply::status(302).header("location", &target));
        let err = http.fetch_untrusted_media(tls.url_at("nuxut.test", "/")).unwrap_err();
        assert!(matches!(err, HttpError::InsecureScheme | HttpError::Downgrade), "{err:?}");
        assert!(plain.requests().iter().all(|r| r.method != "GET"));
    }
}
