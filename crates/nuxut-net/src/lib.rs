//! SSRF'ye dayanıklı, senkron HTTP istemcisi (ADR-0003: her platformda derlenir).
//!
//! Başka bir sürecin ya da sunucunun verdiği adrese (MPRIS kapağı, CalDAV yönlendirmesi, ICS
//! aboneliği) giden her istek buradan geçer. Kurallar [`client`] modül belgesindedir: çöz →
//! denetle → yalnızca denetlenmiş adrese bağlan, güven düzeyleri ([`Trust`]), kimlik yalnızca
//! kökene bağlı istemciyle, sınırlı gövde/süre/yönlendirme, URL'siz loglar.
//!
//! Kaynak: `calendar/crates/nuxut-accounts/src/http.rs` (aynı proje, MIT).

#![forbid(unsafe_code)]

pub mod client;
mod ip;
mod transport;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;

pub use client::{
    Auth, CONNECT_TIMEOUT, DEFAULT_MAX_BODY, DEFAULT_TIMEOUT, HttpClient, HttpError, Lookup, MAX_REDIRECTS,
    MEDIA_MAX_BODY, MEDIA_MAX_REDIRECTS, MEDIA_TIMEOUT, Method, OriginBoundClient, Policy, Request, Response, Secret,
    SystemLookup, Trust, USER_AGENT, is_loopback,
};
pub use ip::{IpClass, classify};
pub use url::Url;
