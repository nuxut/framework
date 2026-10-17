//! Adres sınıflandırması (SSRF kararları).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Adresin sınıfı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpClass {
    Public,
    /// LAN (RFC 1918, CGNAT 100.64/10, ULA fc00::/7, kıyaslama 198.18/15): kullanıcı girerse.
    Private,
    Loopback,
    /// Hiçbir zaman: link-local (169.254/16 bulut meta verisi, fe80::/10), belirtilmemiş, 0/8,
    /// çok noktaya yayın, yayın, belgeleme (TEST-NET'ler, 2001:db8::/32, 3fff::/20), ayrılmış
    /// (240/4, 192.0.0/24), 6to4 aktarıcı (192.88.99/24), eski site-local (fec0::/10), discard
    /// (100::/64), IANA özel amaçlı 2001::/23 (Teredo, kıyaslama, ORCHID…), SRv6 (5f00::/16),
    /// yerel NAT64 (64:ff9b::/32'nin /96 dışı), 2000::/3 dışındaki tüm IPv6 (ULA ve `::1` hariç).
    Never,
}

/// Adresi kategorisine göre sınıflandırır. IPv6 içine gömülü IPv4 (eşlenmiş `::ffff:a.b.c.d`,
/// SIIT `::ffff:0:a.b.c.d`, uyumlu `::a.b.c.d`, NAT64 `64:ff9b::/96`, 6to4 `2002::/16`) gömülü
/// adrese göre karar verir.
pub fn classify(ip: IpAddr) -> IpClass {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> IpClass {
    let o = ip.octets();
    if ip.is_loopback() {
        IpClass::Loopback
    } else if ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || o[0] == 0
        || o[0] >= 240
        || matches!(
            (o[0], o[1], o[2]),
            (192, 0, 2) | (198, 51, 100) | (203, 0, 113) | (192, 0, 0) | (192, 88, 99)
        )
    {
        IpClass::Never
    } else if ip.is_private()
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        || (o[0] == 198 && (o[1] & 0xfe) == 18)
    {
        IpClass::Private
    } else {
        IpClass::Public
    }
}
/// IPv6 varsayılan reddettir: yalnızca küresel tek noktaya yayın `2000::/3` `Public` olabilir
/// (içindeki özel amaçlı bloklar çıkarılarak), ULA `fc00::/7` `Private`, `::1` `Loopback`;
/// IPv4 gömen biçimler gömülü adrese göre; tanınmayan her şey `Never`.
fn classify_v6(ip: Ipv6Addr) -> IpClass {
    let s = ip.segments();
    let embedded =
        |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    if ip.is_loopback() {
        return IpClass::Loopback;
    }
    // ::ffff:a.b.c.d (eşlenmiş), ::ffff:0:a.b.c.d (SIIT, RFC 7915), ::a.b.c.d (uyumlu; :: hariç).
    if s[..5].iter().all(|x| *x == 0) && s[5] == 0xffff
        || s[..4].iter().all(|x| *x == 0) && s[4] == 0xffff && s[5] == 0
        || s[..6].iter().all(|x| *x == 0) && !ip.is_unspecified()
    {
        return classify_v4(embedded(s[6], s[7]));
    }
    // NAT64 64:ff9b::/96 (RFC 6052); /32'nin geri kalanı (yerel NAT64 64:ff9b:1::/48 vb.) ağa
    // özgü gömme, güvenle sınıflanamaz.
    if (s[0], s[1]) == (0x0064, 0xff9b) {
        return if s[2..6].iter().all(|x| *x == 0) {
            classify_v4(embedded(s[6], s[7]))
        } else {
            IpClass::Never
        };
    }
    if (s[0] & 0xfe00) == 0xfc00 {
        return IpClass::Private;
    }
    // Küresel tek noktaya yayın dışı her şey (::, link-local, site-local, çok noktaya yayın,
    // discard 100::/64, SRv6 5f00::/16, ayrılmış alanlar).
    if (s[0] & 0xe000) != 0x2000 {
        return IpClass::Never;
    }
    // 6to4 2002:AABB:CCDD::/48
    if s[0] == 0x2002 {
        return classify_v4(embedded(s[1], s[2]));
    }
    let never = (s[0] == 0x2001 && s[1] < 0x0200) // IANA özel amaçlı 2001::/23 (Teredo, kıyaslama, ORCHID, AMT…)
        || (s[0], s[1]) == (0x2001, 0x0db8) // belgeleme
        || (s[0] == 0x3fff && s[1] & 0xf000 == 0); // belgeleme 3fff::/20
    if never {
        IpClass::Never
    } else {
        IpClass::Public
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> IpClass {
        classify(s.parse().unwrap())
    }

    #[test]
    fn ip_categories_cover_embedded_and_special_ranges() {
        for never in [
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.2.5",
            "198.51.100.1",
            "203.0.113.9",
            "192.0.0.8",
            "fe80::1",
            "::",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378::1",
            "::ffff:169.254.169.254",
            "64:ff9b::a9fe:a9fe",
            "2002:a9fe:a9fe::1",
        ] {
            assert_eq!(c(never), IpClass::Never, "{never}");
        }
        for private in [
            "10.0.0.1",
            "172.16.5.4",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.254",
            "198.18.0.1",
            "fd00::1",
            "fc12::1",
            "::ffff:10.0.0.1",
            "::10.0.0.1",
            "64:ff9b::a00:1",
            "2002:c0a8:0101::1",
        ] {
            assert_eq!(c(private), IpClass::Private, "{private}");
        }
        for lo in [
            "127.0.0.1",
            "127.255.0.1",
            "::1",
            "::ffff:127.0.0.1",
            "::127.0.0.1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::1",
        ] {
            assert_eq!(c(lo), IpClass::Loopback, "{lo}");
        }
        for public in [
            "93.184.216.34",
            "1.1.1.1",
            "100.128.0.1",
            "2606:4700::1111",
            "64:ff9b::5db8:d822",
        ] {
            assert_eq!(c(public), IpClass::Public, "{public}");
        }
    }

    /// Taşımada eklenen bloklar (eski sınıflandırıcı bunları genel sayıyordu).
    #[test]
    fn added_special_purpose_blocks_are_never() {
        for never in [
            "192.88.99.1",
            "fec0::1",
            "feff::1",
            "100::1",
            "100::ffff:1",
            "2001:2::1",
            "2001:2:0:ffff::1",
            "3fff::1",
            "3fff:fff::1",
            "5f00::1",
            "2002:c058:6301::1",
            "64:ff9b:1::a00:1",
            "64:ff9b:1:ffff::1",
        ] {
            assert_eq!(c(never), IpClass::Never, "{never}");
        }
        // Komşu bloklar genel kalır.
        for public in [
            "192.88.100.1",
            "2001:200::1",
            "3fff:1000::1",
            "3ffe::1",
            "2400:cb00::1",
        ] {
            assert_eq!(c(public), IpClass::Public, "{public}");
        }
    }

    /// İnceleme bulguları: eski sınıflandırıcı bunları genel sayıyordu.
    #[test]
    fn review_gaps_are_closed() {
        // SIIT (RFC 7915) ::ffff:0:a.b.c.d gömülü adrese göre.
        assert_eq!(c("::ffff:0:a9fe:a9fe"), IpClass::Never);
        assert_eq!(c("::ffff:0:7f00:1"), IpClass::Loopback);
        assert_eq!(c("::ffff:0:a00:1"), IpClass::Private);
        assert_eq!(c("::ffff:0:5db8:d822"), IpClass::Public);
        // 64:ff9b::/32 içinde /96 dışı biçimler.
        for never in [
            "64:ff9b:0:1::a9fe:a9fe",
            "64:ff9b:ffff::1",
            "64:ff9b::1:5db8:d822",
        ] {
            assert_eq!(c(never), IpClass::Never, "{never}");
        }
        // 2001::/23 IANA özel amaçlı (ORCHID 2001:10::/28, ORCHIDv2 2001:20::/28, AMT 2001:3::/32…).
        for never in [
            "2001:10::1",
            "2001:20::1",
            "2001:3::1",
            "2001:1::1",
            "2001:1ff::1",
        ] {
            assert_eq!(c(never), IpClass::Never, "{never}");
        }
        // 2000::/3 dışı tanınmayan IPv6 varsayılan ret.
        for never in [
            "100:0:0:1::1",
            "4000::1",
            "5f01::1",
            "8000::1",
            "e000::1",
            "1::1",
            "::ffff:1:a00:1",
        ] {
            assert_eq!(c(never), IpClass::Never, "{never}");
        }
    }
}
