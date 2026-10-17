//! AT-SPI ağacını yazdırır (ADR-0011): bir yardımcı teknoloji gibi erişilebilirliği açar,
//! registry'deki uygulamaları dolaşır, her düğümün rolünü, adını, durumlarını yazar. Shell ve
//! Ayarlar'ın ekran okuyucu ağacını elle/betikle doğrulamak için.
//!
//! YALNIZ yalıtılmış ortamda: `tools/a11y-e2e.sh` içinde (kullanıcının a11y bus'ında Orca
//! açıkken çalıştırmak onun ayarını değiştirmez ama ağaç içeriği — pencere/bildirim metni —
//! ekrana dökülür; yapma).
//!
//! ```sh
//! cargo build -p nuxut --example atspi_dump
//! tools/a11y-e2e.sh sh betik.sh   # betik: uygulamayı başlat, sonra:
//! target/debug/examples/atspi_dump nuxut-shell --wait 15
//! ```
//!
//! Argümanlar: `[UYGULAMA]` (ad alt dizgisi; yoksa hepsi), `--wait SN` (uygulama ve en az bir
//! adlı düğüm görünene dek bekle), `--expect AD` (bu adlı düğüm yoksa çıkış kodu 1; tekrarlanır).

#[cfg(target_os = "linux")]
fn main() {
    use std::time::{Duration, Instant};

    use atspi::proxy::accessible::AccessibleProxy;

    let mut filter = None;
    let mut wait = 0u64;
    let mut expect = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--wait" => wait = args.next().and_then(|s| s.parse().ok()).unwrap_or(10),
            "--expect" => expect.extend(args.next()),
            _ => filter = Some(a),
        }
    }

    struct Line {
        depth: usize,
        role: String,
        name: String,
        states: String,
    }

    async fn walk(bus: &zbus::Connection, dest: &str, path: &str, depth: usize, out: &mut Vec<Line>) -> zbus::Result<()> {
        let acc = AccessibleProxy::builder(bus)
            .destination(dest.to_owned())?
            .path(path.to_owned())?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let states = acc.get_state().await?;
        let states = states.into_iter().map(|s| format!("{s:?}").to_lowercase()).collect::<Vec<_>>().join(",");
        // accesskit uygulama kökünde GetRoleName yok; rol sayısal kimlikten.
        let role = acc.get_role().await.map(|r| format!("{r:?}"))?;
        out.push(Line { depth, role, name: acc.name().await?, states });
        if depth < 24 {
            for c in acc.get_children().await? {
                let d = c.name_as_str().unwrap_or(dest).to_owned();
                if let Err(e) = Box::pin(walk(bus, &d, c.path_as_str(), depth + 1, out)).await {
                    out.push(Line { depth: depth + 1, role: "<hata>".into(), name: e.to_string(), states: String::new() });
                }
            }
        }
        Ok(())
    }

    let code = zbus::block_on(async {
        let session = zbus::Connection::session().await.expect("oturum bus'ı");
        session
            .call_method(
                Some("org.a11y.Bus"),
                "/org/a11y/bus",
                Some("org.freedesktop.DBus.Properties"),
                "Set",
                &("org.a11y.Status", "IsEnabled", zbus::zvariant::Value::from(true)),
            )
            .await
            .expect("IsEnabled açılamadı (at-spi yok mu?)");
        let reply = session
            .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
            .await
            .expect("a11y bus adresi");
        let address: String = reply.body().deserialize().expect("adres");
        let bus = zbus::connection::Builder::address(address.as_str()).expect("adres").build().await.expect("a11y bus");
        let root = AccessibleProxy::builder(&bus)
            .destination("org.a11y.atspi.Registry")
            .unwrap()
            .path("/org/a11y/atspi/accessible/root")
            .unwrap()
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(wait);
        loop {
            let mut lines = Vec::new();
            for app in root.get_children().await.unwrap_or_default() {
                let dest = app.name_as_str().unwrap_or_default().to_owned();
                let mut out = Vec::new();
                // Yarım kalan dolaşma da yazdırılır (hata hangi düğümde kaldığını gösterir).
                if let Err(e) = walk(&bus, &dest, app.path_as_str(), 0, &mut out).await {
                    out.push(Line { depth: 0, role: "<hata>".into(), name: e.to_string(), states: String::new() });
                }
                if filter.as_ref().is_none_or(|f| out.first().is_some_and(|l| l.name.contains(f.as_str()))) {
                    lines.extend(out);
                }
            }
            let missing: Vec<&String> = expect.iter().filter(|e| !lines.iter().any(|l| &l.name == *e)).collect();
            let ready = lines.len() > 1 && missing.is_empty();
            if ready || Instant::now() >= deadline {
                for l in &lines {
                    println!("{}{} \"{}\" [{}]", "  ".repeat(l.depth), l.role, l.name, l.states);
                }
                if !missing.is_empty() {
                    eprintln!("eksik düğümler: {missing:?}");
                    return 1;
                }
                return if lines.is_empty() { 1 } else { 0 };
            }
            async_io::Timer::after(Duration::from_millis(200)).await;
        }
    });
    std::process::exit(code);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AT-SPI yalnız Linux'ta");
}
