//! `file://` URI → yol. Portal seçimleri URI olarak döner (RFC 8089); yol baytları
//! yüzde-kodludur ve UTF-8 olmak zorunda değildir.

use std::path::PathBuf;

/// `file:///mutlak/yol` ya da `file://localhost/mutlak/yol` → yol (yüzde-kod çözülür, UTF-8
/// olmayan baytlar korunur). Başka şema, başka host, göreli yol, sorgu/parça, bozuk kaçış ya
/// da NUL baytı → `None`. Document portal yolları (`/run/user/<uid>/doc/…`) olduğu gibi geçer.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    // Şema büyük/küçük harf duyarsız (RFC 3986 §3.1).
    let scheme_end = uri.find(':')?;
    if !uri[..scheme_end].eq_ignore_ascii_case("file") {
        return None;
    }
    let rest = uri[scheme_end + 1..].strip_prefix("//")?;
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) {
        return None;
    }
    // Sorgu/parça bir dosya seçiminde anlamsız: kabul edilmez (yüzde-kodlu `?`/`#` geçerli).
    if path.contains(['?', '#']) {
        return None;
    }
    let bytes = percent_decode(path.as_bytes())?;
    if bytes.contains(&0) {
        return None;
    }
    Some(bytes_to_path(bytes))
}

/// Unix: yol baytları olduğu gibi (UTF-8 olmayan ad korunur).
#[cfg(unix)]
fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

/// Unix dışı (Windows): yol UTF-16'dır; geçersiz UTF-8 kayıplı çevrilir. Bu platformda portal
/// yok (istekler `Unavailable`); yalnız derlenebilirlik (ADR-0003).
#[cfg(not(unix))]
fn bytes_to_path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}

fn percent_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            b'%' => {
                let hi = hex(*input.get(i + 1)?)?;
                let lo = hex(*input.get(i + 2)?)?;
                out.push(hi << 4 | lo);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn plain_and_localhost() {
        assert_eq!(
            file_uri_to_path("file:///home/u/a.png"),
            Some("/home/u/a.png".into())
        );
        assert_eq!(
            file_uri_to_path("file://localhost/etc/x"),
            Some("/etc/x".into())
        );
        assert_eq!(
            file_uri_to_path("FILE://LocalHost/etc/x"),
            Some("/etc/x".into())
        );
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(
            file_uri_to_path("file:///home/u/My%20Files/%C3%A7i%C3%A7ek.png"),
            Some("/home/u/My Files/çiçek.png".into())
        );
        assert_eq!(
            file_uri_to_path("file:///a/100%25%3F%23.txt"),
            Some("/a/100%?#.txt".into())
        );
        // UTF-8 olmayan bayt korunur.
        let p = file_uri_to_path("file:///tmp/%FFx").unwrap();
        assert_eq!(p.as_os_str().as_bytes(), b"/tmp/\xffx");
    }

    #[test]
    fn document_portal_path() {
        assert_eq!(
            file_uri_to_path("file:///run/user/1000/doc/3a2b1c/vpn.ovpn"),
            Some("/run/user/1000/doc/3a2b1c/vpn.ovpn".into())
        );
    }

    #[test]
    fn rejects() {
        for bad in [
            "",
            "/home/u/a",
            "file:/home/u/a",
            "file:home",
            "file://",
            "file://host/a",
            "file://evil.example/etc/passwd",
            "https://x/a",
            "smb://srv/share/a",
            "file:///a%",
            "file:///a%4",
            "file:///a%zz",
            "file:///a%00b",
            "file:///a?q=1",
            "file:///a#frag",
        ] {
            assert_eq!(file_uri_to_path(bad), None, "{bad}");
        }
    }
}
