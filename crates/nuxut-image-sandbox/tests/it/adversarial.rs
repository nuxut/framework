//! ADR-0007 §Düşmanca test planı: gerçek işçi ikilisiyle (`sandbox-test-hooks` açık).
//! Süreç/fd sayan testler birbirini bozmasın diye hepsi tek kilit altında sırayla çalışır.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut_image::{DecodeOptions, Fit, ImageError, Isolation, Limits, SandboxFailure, TargetSize, Trust};
use nuxut_image_sandbox::hooks::Hook;
use nuxut_image_sandbox::protocol::{HOOK_ALLOWED, HOOK_DENIED};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};

use crate::{WORKER, serial};

fn cfg() -> SandboxConfig {
    SandboxConfig {
        worker: Some(WORKER.into()),
        // Bu süre ürünün istek zaman aşımı DEĞİL (o SandboxConfig::default, 1,5 sn, dokunulmaz):
        // test yalnız takılmayı yakalar, doğrulama sonuca bakar. Paralel derleme yükünde (yük ~50)
        // 10 sn ara sıra aşılıyordu (perf 2026-10-16); zaman aşımını sınayan testler kendi
        // süresini verir.
        timeout: Duration::from_secs(30),
        // Başlatma süresi aşılırsa Sandbox `unavailable_retry` (60 sn) boyunca kullanılamaz olur
        // ve testin geri kalanı yanlış düşer; bellek bekçisi test sürecini saniyelerce
        // dondurabildiği için geniş (yalnızca takılan başlatmayı sınırlar).
        spawn_timeout: Duration::from_secs(60),
        rate_per_sec: 100_000,
        burst: 100_000,
        ..SandboxConfig::default()
    }
}

fn icon_opts(side: u32) -> DecodeOptions {
    DecodeOptions { limits: Limits::ICON, target: Some(TargetSize::new(side, side, Fit::Contain)), animate: false }
}

fn svg(body: &str) -> Arc<[u8]> {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="64" height="64">{body}</svg>"#).into_bytes().into()
}

fn red_svg() -> Arc<[u8]> {
    svg(r##"<rect width="64" height="64" fill="#ff0000"/><circle cx="32" cy="32" r="20" fill="#0000ff" opacity="0.5"/>"##)
}

fn png(w: u32, h: u32) -> Arc<[u8]> {
    let mut buf = image::RgbaImage::new(w, h);
    for (x, y, p) in buf.enumerate_pixels_mut() {
        *p = image::Rgba([(x * 7) as u8, (y * 13) as u8, 90, if x % 3 == 0 { 128 } else { 255 }]);
    }
    let mut out = Vec::new();
    buf.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out.into()
}

fn sandbox_failure(r: Result<nuxut_image::Image, ImageError>) -> Option<SandboxFailure> {
    match r {
        Err(ImageError::Sandbox(f)) => Some(f),
        _ => None,
    }
}

/// Bu sürecin çocukları: (toplam, zombi).
fn children() -> (usize, usize) {
    let me = std::process::id().to_string();
    let mut total = 0;
    let mut zombies = 0;
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { continue };
        let mut f = rest.split_whitespace();
        let state = f.next().unwrap_or("");
        let ppid = f.next().unwrap_or("");
        if ppid == me {
            total += 1;
            if state == "Z" {
                zombies += 1;
            }
        }
    }
    (total, zombies)
}

fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd").unwrap().count()
}

#[test]
fn isolated_output_matches_in_process_pixel_for_pixel() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    assert_eq!(sb.isolation(), Isolation::Process);
    for (bytes, opts) in [
        (red_svg(), icon_opts(48)),
        (red_svg(), DecodeOptions { target: Some(TargetSize::new(100, 30, Fit::Cover)), ..icon_opts(1) }),
        (png(40, 20), icon_opts(16)),
        (png(40, 20), DecodeOptions { target: None, ..icon_opts(1) }),
    ] {
        let got = sb.decode(0, bytes.clone(), &opts).unwrap();
        let want = nuxut_image::decode_bytes(&bytes, &opts.with_trust(Trust::System)).unwrap().into_first_frame();
        assert_eq!(got.size(), want.size());
        assert!(got.pixels() == want.pixels(), "pikseller farklı");
    }
    // Tek kullanımlık: her istek ayrı süreç.
    assert!(sb.stats().spawned >= 4, "her istek taze işçi");
    assert_eq!(sb.stats().decoded, 4);
}

#[test]
fn hostile_svg_corpus_fails_safely_and_pool_recovers() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    let deep = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\">{}{}</svg>", "<g>".repeat(100_000), "</g>".repeat(100_000));
    let mut use_bomb = String::from(r#"<defs><rect id="l0" width="1" height="1"/>"#);
    for i in 1..=10 {
        use_bomb.push_str(&format!(r#"<g id="l{i}">"#));
        for _ in 0..10 {
            use_bomb.push_str(&format!(r##"<use xlink:href="#l{}"/>"##, i - 1));
        }
        use_bomb.push_str("</g>");
    }
    use_bomb.push_str(r##"</defs><use xlink:href="#l10"/>"##);
    // svgz "zip bombası": gzip başlığı + sıfırların sıkıştırılmış hâli taklidi.
    let mut gz = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3];
    gz.extend(std::iter::repeat_n(0u8, 4096));
    let corpus: Vec<Arc<[u8]>> = vec![
        b"<svg".to_vec().into(),
        br#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;"><!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;">]><svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><text>&c;&c;&c;</text></svg>"#.to_vec().into(),
        deep.into_bytes().into(),
        svg(&use_bomb),
        svg(r#"<filter id="f"><feGaussianBlur stdDeviation="5000"/></filter><rect width="64" height="64" filter="url(#f)"/>"#),
        svg(r#"<path d="M0 0 L64 64 L0 64 L64 0 Z" stroke="red" stroke-dasharray="0.00001"/>"#),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="1e30" height="1e30" viewBox="0 0 1e30 1e30"/>"#.to_vec().into(),
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><image href="file:///etc/passwd" width="64" height="64"/></svg>"#.to_vec().into(),
        gz.into(),
        vec![b' '; 9 << 20].into(),
    ];
    for (i, bytes) in corpus.into_iter().enumerate() {
        let start = Instant::now();
        let r = sb.decode(100 + i as u64, bytes, &icon_opts(64));
        // Hata ya da zararsız küçük görsel; asla ebeveyn çökmesi/donması.
        if let Ok(img) = &r {
            assert!(img.width() <= 64 && img.height() <= 64);
        }
        // Takılma yok: ebeveyn isteği en geç (başlatma + istek) zaman aşımında keser. Sabit 10 sn
        // test bekleme süresiydi, ürün sınırı değil; yük altında yanlış düşmesin.
        let c = cfg();
        assert!(start.elapsed() < c.spawn_timeout + c.timeout, "takıldı: {:?}", start.elapsed());
    }
    let ok = sb.decode(0, red_svg(), &icon_opts(32)).unwrap();
    assert_eq!(ok.size(), (32, 32));
    // Kullanılmış işçiyi temizlikçi thread öldürüp bekler (kill → wait): çocuk ölünce çekirdek onu
    // `Z` yapar, bekleyen thread uyanıp toplayana dek (yük altında birkaç ms) anlık örnekleme bir
    // zombi görebilir — her çocuğun geçtiği durum, sızıntı değil (2026-10-17 inceleme: bütün
    // yollar `process::reap` ile bekler). Sızıntı = kısa sürede oturmayan zombi ya da girdi görmüş
    // canlı işçi.
    only_fresh_children(&sb);
    // Kesin sınır: Sandbox kapanınca (temizlikçi join edilir) ne çocuk ne zombi kalır; bekleme yok.
    drop(sb);
    assert_eq!(children(), (0, 0), "Sandbox kapandıktan sonra işçi ya da zombi kaldı");
}

/// Kısa zaman aşımlı Sandbox'ta meşru çözüm: aç kalan/dondurulan test sürecinde 400 ms'yi aşıp
/// `Timeout` alabilir. İkon profilinde zaman aşımı girdiyi negatif önbelleğe koyar ve anahtarı
/// sayar; bu yüzden her deneme ayrı bayt (yorum) ve ayrı anahtarla yapılır (`salt` aynı
/// Sandbox'taki çağrıları ayırır). `Timeout` dışındaki her hata testi düşürür.
fn decode_ok_despite_timeouts(sb: &Sandbox, side: u32, salt: u64) -> nuxut_image::Image {
    for attempt in salt..salt + 50 {
        let bytes = svg(&format!(r##"<!-- {attempt} --><rect width="64" height="64" fill="#ff0000"/>"##));
        match sb.decode(10_000 + attempt, bytes, &icon_opts(side)) {
            Ok(img) => return img,
            Err(ImageError::Sandbox(SandboxFailure::Timeout)) => {}
            Err(e) => panic!("deneme {attempt}: {e:?}"),
        }
    }
    panic!("meşru çözüm 50 denemede de bitmedi");
}

#[test]
fn infinite_loop_times_out_kills_worker_and_is_negatively_cached() {
    let _s = serial();
    let sb = Sandbox::new(SandboxConfig { timeout: Duration::from_millis(400), ..cfg() });
    // İşçiyi önce ısıt (önceden ısıtılmış havuz dolu).
    decode_ok_despite_timeouts(&sb, 8, 0);
    let poison: Arc<[u8]> = b"spin-1".to_vec().into();
    let r = sb.decode_hooked(0, poison.clone(), &icon_opts(8), Hook::Spin);
    // Sonsuz döngü döndüyse süreyi ebeveyn kesmiştir; süre üst sınırı (eskiden 3 sn) yük
    // altında yanlış düşüyordu (bellek bekçisi test sürecini saniyelerce dondurabiliyor).
    assert_eq!(sandbox_failure(r), Some(SandboxFailure::Timeout));
    only_fresh_children(&sb);
    // Aynı zehirli bayt: işçi başlatılmadan reddedilir. Kanıt `Killed` + `negative_hits`
    // (işçiye gitseydi Spin yine `Timeout` dönerdi); eski 50 ms sınırı aynı nedenle kalktı.
    let failures = sb.stats().failures;
    let r = sb.decode_hooked(0, poison, &icon_opts(8), Hook::Spin);
    assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed));
    assert_eq!(sb.stats().negative_hits, 1);
    assert_eq!(sb.stats().failures, failures, "işçi denemesi yok");
    // Havuz toparlanır.
    assert_eq!(decode_ok_despite_timeouts(&sb, 8, 1000).size(), (8, 8));
}

#[test]
fn cpu_rlimit_kills_runaway_worker() {
    let _s = serial();
    // Duvar saati yalnızca güvenlik ağı: işçiyi `RLIMIT_CPU` öldürmeli. CPU'su kısılan işçi
    // 4 sn CPU'yu 30 sn duvar saatinde biriktiremeyip `Timeout` alıyordu.
    let sb = Sandbox::new(SandboxConfig { timeout: Duration::from_secs(600), ..cfg() });
    let start = Instant::now();
    let r = sb.decode_hooked(0, b"cpu".to_vec().into(), &icon_opts(8), Hook::Spin);
    // `Killed` (zaman aşımı değil) sınırın rlimit olduğunu gösterir.
    assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed), "SIGXCPU");
    // Yumuşak sınır 4 sn CPU: duvar saati CPU süresinden kısa olamaz.
    assert!(start.elapsed() >= Duration::from_secs(3), "{:?}", start.elapsed());
}

#[test]
fn memory_limit_and_panic_kill_only_the_worker() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    let r = sb.decode_hooked(0, b"alloc".to_vec().into(), &icon_opts(8), Hook::Alloc2G);
    assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed));
    let r = sb.decode_hooked(2, b"panic".to_vec().into(), &icon_opts(8), Hook::Panic);
    assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed));
    assert!(sb.decode(0, red_svg(), &icon_opts(8)).is_ok());
}

#[test]
fn seccomp_kills_on_file_network_process_and_stderr_access() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("kacak");
    let sb = Sandbox::new(cfg());
    let cases: [(Hook, Vec<u8>); 6] = [
        (Hook::ReadFile, b"r".to_vec()),
        (Hook::CreateFile, target.to_str().unwrap().as_bytes().to_vec()),
        (Hook::Connect, b"c".to_vec()),
        (Hook::Thread, b"t".to_vec()),
        (Hook::Exec, b"e".to_vec()),
        (Hook::WriteStderr, b"w".to_vec()),
    ];
    for (i, (hook, payload)) in cases.into_iter().enumerate() {
        let r = sb.decode_hooked(100 + i as u64, payload.into(), &icon_opts(8), hook);
        assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed), "{hook:?}");
    }
    assert!(!target.exists(), "dosya oluşmamalı");
    only_fresh_children(&sb);
}

#[test]
fn landlock_alone_denies_filesystem_and_network() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("kacak");
    // seccomp kapalı: yalnızca landlock + rlimit katmanı sınanır.
    let sb = Sandbox::new(SandboxConfig { worker_args: vec!["--test-no-seccomp".into()], test_allow_unconfined: true, ..cfg() });
    let cases: [(Hook, Vec<u8>); 5] = [
        (Hook::ReadFile, b"r".to_vec()),
        (Hook::CreateFile, target.to_str().unwrap().as_bytes().to_vec()),
        (Hook::Connect, b"c".to_vec()),
        (Hook::Thread, b"t".to_vec()),
        (Hook::Exec, b"e".to_vec()),
    ];
    for (i, (hook, payload)) in cases.into_iter().enumerate() {
        match sb.decode_hooked(100 + i as u64, payload.into(), &icon_opts(8), hook) {
            Err(ImageError::Unsupported(m)) if m == HOOK_DENIED => {}
            Err(ImageError::Unsupported(m)) if m == HOOK_ALLOWED => panic!("{hook:?}: yalıtım delik"),
            other => panic!("{hook:?}: beklenmeyen {:?}", other.map(|i| i.size())),
        }
    }
    assert!(!target.exists());
}

#[test]
fn forged_responses_are_protocol_errors_with_bounded_parent_memory() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    for (i, hook) in [Hook::SpoofOversize, Hook::SpoofShortExit, Hook::SpoofExtra, Hook::SpoofHuge, Hook::SpoofStatus, Hook::SpoofAnimation, Hook::FloodStdout].into_iter().enumerate() {
        let start = Instant::now();
        let r = sb.decode_hooked(100 + i as u64, format!("{hook:?}").into_bytes().into(), &icon_opts(16), hook);
        assert_eq!(sandbox_failure(r), Some(SandboxFailure::Protocol), "{hook:?}");
        assert!(start.elapsed() < Duration::from_secs(5), "{hook:?}");
    }
    only_fresh_children(&sb);
}

#[test]
fn many_jobs_leak_no_fds_processes_or_zombies() {
    let _s = serial();
    let before_fds = fd_count();
    {
        let sb = Sandbox::new(cfg());
        let bytes = svg(r##"<rect width="10" height="10" fill="#0f0"/>"##);
        for _ in 0..1000 {
            sb.decode(0, bytes.clone(), &icon_opts(8)).unwrap();
        }
        let s = sb.stats();
        assert_eq!(s.decoded, 1000);
        // Tek kullanımlık: istek başına bir süreç.
        assert!(s.spawned >= 1000, "{s:?}");
        assert!(s.live <= 2);
    }
    assert!(eventually(|| fd_count() == before_fds), "fd sızıntısı: {before_fds} → {} {:?}", fd_count(), std::fs::read_dir("/proc/self/fd").unwrap().flatten().filter_map(|e| std::fs::read_link(e.path()).ok()).collect::<Vec<_>>());
    assert!(eventually(|| children() == (0, 0)), "çocuk/zombi kalmadı");
}

#[test]
fn idle_workers_are_reaped_and_nothing_runs_when_idle() {
    let _s = serial();
    let sb = Sandbox::new(SandboxConfig { idle_timeout: Duration::from_millis(300), ..cfg() });
    sb.decode(0, red_svg(), &icon_opts(8)).unwrap();
    assert!(eventually(|| sb.stats().idle == 1), "taze işçi ısıtıldı");
    assert!(eventually(|| children() == (1, 0)));
    let until = Instant::now() + Duration::from_secs(5);
    while sb.stats().live > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(sb.stats().live, 0);
    assert_eq!(children(), (0, 0));
    // Kapatıcı thread de çıktı: süreçte yalnızca test thread'leri kalır.
    std::thread::sleep(Duration::from_millis(100));
    let reaper_alive = std::fs::read_dir("/proc/self/task")
        .unwrap()
        .flatten()
        .any(|t| std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.trim() == "nuxut-image-jan"));
    assert!(!reaper_alive, "boşta thread kalmamalı");
    // Yeniden kullanım işçiyi tembelce başlatır.
    let before = sb.stats().spawned;
    sb.decode(0, red_svg(), &icon_opts(8)).unwrap();
    assert!(sb.stats().spawned > before);
}

#[test]
fn rate_limit_and_key_mute() {
    let _s = serial();
    let sb = Sandbox::new(SandboxConfig { rate_per_sec: 0, burst: 3, ..cfg() });
    for _ in 0..3 {
        sb.decode(0, red_svg(), &icon_opts(8)).unwrap();
    }
    assert_eq!(sandbox_failure(sb.decode(0, red_svg(), &icon_opts(8))), Some(SandboxFailure::RateLimited));

    let sb = Sandbox::new(SandboxConfig { timeout: Duration::from_millis(300), ..cfg() });
    let key = 42;
    for i in 0..3 {
        let r = sb.decode_hooked(key, format!("p{i}").into_bytes().into(), &icon_opts(8), Hook::Panic);
        assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed));
    }
    // Anahtar susturuldu; başka anahtar etkilenmez.
    assert_eq!(sandbox_failure(sb.decode(key, red_svg(), &icon_opts(8))), Some(SandboxFailure::RateLimited));
    assert!(sb.decode(7, red_svg(), &icon_opts(8)).is_ok());
}

#[test]
fn concurrent_callers_share_at_most_max_workers() {
    let _s = serial();
    // 4 thread × 15 SVG, en fazla 2 işçi; 60 istekten tek bir zaman aşımı (`Timeout`) ya da
    // başlatma aşımı (→ `Unavailable`, SVG süreç içine düşmez) `unwrap`'ı düşürür. Aç kalan/
    // dondurulan test sürecinde 10 sn aşılabiliyordu; testin konusu işçi tavanı ve sayaçlar,
    // zaman aşımı değil (başlatma payı `cfg()`'de).
    let sb = Arc::new(Sandbox::new(SandboxConfig { timeout: Duration::from_secs(120), ..cfg() }));
    let peak = Arc::new(Mutex::new(0usize));
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let sb = sb.clone();
            let peak = peak.clone();
            std::thread::spawn(move || {
                for _ in 0..15 {
                    sb.decode(0, red_svg(), &icon_opts(24)).unwrap();
                    let live = sb.stats().live;
                    let mut p = peak.lock().unwrap();
                    *p = (*p).max(live);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert!(*peak.lock().unwrap() <= 2);
    assert_eq!(sb.stats().decoded, 60);
}

#[test]
fn unavailable_worker_falls_back_only_for_basic_raster() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        // Yok.
        dir.path().join("yok"),
        // Kısıtlama kurulamadı (çıkış 70).
        {
            let p = dir.path().join("exit70");
            std::fs::write(&p, "#!/bin/sh\nexit 70\n").unwrap();
            p
        },
        // Sahte el sıkışma.
        {
            let p = dir.path().join("liar");
            std::fs::write(&p, "#!/bin/sh\nprintf 'NXIW\\001\\000\\0059.9.9'\ncat >/dev/null\n").unwrap();
            p
        },
    ];
    for p in &cases {
        use std::os::unix::fs::PermissionsExt;
        if p.exists() {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    for p in cases {
        let sb = Sandbox::new(SandboxConfig { worker: Some(p), ..cfg() });
        let r = sb.decode(0, red_svg(), &icon_opts(8));
        assert_eq!(sandbox_failure(r), Some(SandboxFailure::Unavailable), "SVG asla süreç içine düşmez");
        assert_eq!(sb.isolation(), Isolation::Unavailable);
        let img = sb.decode(0, png(8, 8), &icon_opts(8)).unwrap();
        assert_eq!(img.size(), (8, 8));
        assert_eq!(sb.stats().in_process_fallbacks, 1);
        assert_eq!(sb.stats().live, 0);
    }
    // Yanlış imzalı gerçek işçi de kullanılamaz sayılır.
    let sb = Sandbox::new(SandboxConfig { worker_args: vec!["--test-bad-handshake".into()], ..cfg() });
    assert_eq!(sandbox_failure(sb.decode(0, red_svg(), &icon_opts(8))), Some(SandboxFailure::Unavailable));
    assert_eq!(children(), (0, 0));
}

#[test]
fn loader_routes_untrusted_through_sandbox() {
    let _s = serial();
    let sb = Arc::new(Sandbox::new(cfg()));
    let mut loader = nuxut_image::Loader::new(1).with_sandbox(sb.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    let req = nuxut_image::LoadRequest { source: nuxut_image::Source::Bytes(red_svg()), options: icon_opts(16) };
    loader.load_keyed(9, req, move |r| tx.send(r.map(|d| d.into_first_frame().size())).unwrap());
    let got = rx.recv_timeout(Duration::from_secs(20)).unwrap().unwrap();
    assert_eq!(got, (16, 16));
    assert_eq!(sb.stats().decoded, 1, "Untrusted SVG işçide çözüldü");
}

/// Bu sürecin çocuklarının PID'leri.
fn child_pids() -> Vec<u32> {
    let me = std::process::id().to_string();
    let mut out = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { continue };
        if rest.split_whitespace().nth(1) == Some(me.as_str()) {
            out.push(pid);
        }
    }
    out
}

#[test]
fn live_worker_is_really_confined() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    sb.decode(0, red_svg(), &icon_opts(8)).unwrap();
    // Yaşayan tek çocuk önceden ısıtılmış taze işçidir; kilitlenmesini bitirmesini bekle
    // (el sıkışmayı yazıp `read(0)`'da uyur).
    assert!(eventually(|| sb.stats().idle == 1), "taze işçi ısıtıldı");
    only_fresh_children(&sb);
    let pids = child_pids();
    assert_eq!(pids.len(), 1);
    let read_status = || std::fs::read_to_string(format!("/proc/{}/status", pids[0])).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while !read_status().contains("Seccomp:\t2") && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    let status = read_status();
    let field = |name: &str| status.lines().find_map(|l| l.strip_prefix(name)).map(str::trim).unwrap_or_default().to_owned();
    assert_eq!(field("Seccomp:"), "2", "seccomp filtre kipi");
    assert_eq!(field("NoNewPrivs:"), "1");
    let limits = std::fs::read_to_string(format!("/proc/{}/limits", pids[0])).unwrap();
    let limit = |name: &str| limits.lines().find(|l| l.starts_with(name)).map(|l| l[name.len()..].split_whitespace().take(2).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    assert_eq!(limit("Max address space"), format!("{0} {0}", 768u64 << 20));
    assert_eq!(limit("Max cpu time"), "4 5");
    assert_eq!(limit("Max file size"), "0 0");
    assert_eq!(limit("Max processes"), "0 0");
    assert_eq!(limit("Max open files"), "3 3");
    assert_eq!(limit("Max core file size"), "0 0");
    // Boştaki işçi CPU harcamaz: read(0)'da uyur.
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", pids[0])).unwrap();
    let state = stat.rsplit_once(')').map(|(_, r)| r.split_whitespace().next().unwrap_or("")).unwrap_or("");
    assert_eq!(state, "S", "boşta uyuyor");
}

/// Yaşayan her çocuk havuzdaki taze (girdi görmemiş) işçidir; zombi yok.
/// Temizlikçi kullanılmış işçiyi çağıranın yolu dışında öldürür: kısa bir süre içinde oturur.
fn only_fresh_children(sb: &Sandbox) {
    let ok = || {
        let (total, zombies) = children();
        let s = sb.stats();
        zombies == 0 && total == s.idle && s.live == s.idle
    };
    assert!(eventually(ok), "girdi görmüş işçi ya da zombi kaldı: {:?} {:?}", children(), sb.stats());
}

/// `cond` en fazla 3 sn içinde doğru olur mu.
fn eventually(cond: impl Fn() -> bool) -> bool {
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    cond()
}

/// Bulgu 3: işçiler tek kullanımlık — ardışık istekler asla aynı sürece gitmez.
#[test]
fn every_request_gets_a_fresh_worker_process() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    let mut seen = std::collections::HashSet::new();
    for _ in 0..6 {
        let img = sb.decode_hooked(0, b"pid".to_vec().into(), &icon_opts(8), Hook::Pid).unwrap();
        let pid = u32::from_le_bytes(img.pixels()[..4].try_into().unwrap());
        assert_ne!(pid, 0);
        assert!(seen.insert(pid), "PID {pid} ikinci kez kullanıldı");
    }
    // Paralel çağıranlar da asla aynı işçiyi paylaşmaz.
    let sb = Arc::new(sb);
    let pids: Vec<u32> = (0..4)
        .map(|_| {
            let sb = sb.clone();
            std::thread::spawn(move || {
                (0..5)
                    .map(|_| {
                        let img = sb.decode_hooked(0, b"pid".to_vec().into(), &icon_opts(8), Hook::Pid).unwrap();
                        u32::from_le_bytes(img.pixels()[..4].try_into().unwrap())
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flat_map(|t| t.join().unwrap())
        .collect();
    for pid in pids {
        assert!(seen.insert(pid), "PID {pid} ikinci kez kullanıldı");
    }
    only_fresh_children(&sb);
}

/// Bulgu 1b: işçi kullanılamazken de kapılar önce uygulanır — işçiyi öldürmüş (negatif
/// önbellekteki) PNG ve susturulmuş anahtarın PNG'si süreç içine düşmez.
#[test]
fn unavailable_fallback_still_honours_gates() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    let poison = png(8, 8);
    // Aynı baytlar işçiyi öldürdü (panik kancası, yük = geçerli PNG).
    assert_eq!(sandbox_failure(sb.decode_hooked(0, poison.clone(), &icon_opts(8), Hook::Panic)), Some(SandboxFailure::Killed));
    // Susturulmuş anahtar.
    for i in 0..3 {
        let r = sb.decode_hooked(5, format!("k{i}").into_bytes().into(), &icon_opts(8), Hook::Panic);
        assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed));
    }
    sb.test_mark_unavailable();
    assert_eq!(sb.isolation(), Isolation::Unavailable);
    assert_eq!(sandbox_failure(sb.decode(0, poison, &icon_opts(8))), Some(SandboxFailure::Killed), "zehirli PNG süreç içine düşmedi");
    assert_eq!(sandbox_failure(sb.decode(5, png(9, 9), &icon_opts(8))), Some(SandboxFailure::RateLimited), "susturulmuş anahtar");
    assert_eq!(sb.stats().in_process_fallbacks, 0);
    // Temiz PNG, temiz anahtar: K2-b yedeği.
    assert_eq!(sb.decode(6, png(10, 10), &icon_opts(16)).unwrap().size(), (10, 10));
    assert_eq!(sb.stats().in_process_fallbacks, 1);
    // Belirteç kovası da yedekten önce.
    let sb = Sandbox::new(SandboxConfig { worker: Some("/yok".into()), rate_per_sec: 0, burst: 1, ..cfg() });
    assert!(sb.decode(0, png(4, 4), &icon_opts(8)).is_ok());
    assert_eq!(sandbox_failure(sb.decode(0, png(5, 5), &icon_opts(8))), Some(SandboxFailure::RateLimited));
}

/// Bulgu 4d: anahtarsız istekler tek kovada susturulur.
#[test]
fn keyless_requests_are_muted_too() {
    let _s = serial();
    let sb = Sandbox::new(cfg());
    for i in 0..3 {
        let r = sb.decode_hooked(nuxut_image::NO_KEY, format!("a{i}").into_bytes().into(), &icon_opts(8), Hook::Panic);
        assert_eq!(sandbox_failure(r), Some(SandboxFailure::Killed));
    }
    assert_eq!(sandbox_failure(sb.decode(nuxut_image::NO_KEY, red_svg(), &icon_opts(8))), Some(SandboxFailure::RateLimited));
    assert!(sb.decode(3, red_svg(), &icon_opts(8)).is_ok(), "anahtarlı kaynak etkilenmez");
}

/// `printf` ile güncel sürümde el sıkışma (ikon profili) yazan sahte işçi; sonra hiç yanıt vermez (stdout
/// açık kalır).
fn fake_worker(dir: &std::path::Path, name: &str, flags: u8, landlock: u8) -> std::path::PathBuf {
    fake_worker_profile(dir, name, flags, landlock, nuxut_image_sandbox::WorkerProfile::Icon.code())
}

fn fake_worker_profile(dir: &std::path::Path, name: &str, flags: u8, landlock: u8, profile: u8) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let v = env!("CARGO_PKG_VERSION");
    let p = dir.join(name);
    let pv = nuxut_image_sandbox::protocol::PROTOCOL_VERSION;
    let script = format!("#!/bin/sh\nprintf 'NXIW\\{pv:03o}\\000\\{flags:03o}\\{landlock:03o}\\000\\000\\{profile:03o}\\{:03o}%s' '{v}'\nexec sleep 30\n", v.len());
    std::fs::write(&p, script).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

/// Bulgu 2a: ebeveyn kilit beyanını zorunlu tutar; seccomp'suz ya da landlock'suz işçi
/// (gerçek ya da sahte) kullanılamaz.
#[test]
fn handshake_without_lockdown_is_rejected() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    // Gerçek işçi, seccomp'suz; ebeveyn izin vermiyor.
    let sb = Sandbox::new(SandboxConfig { worker_args: vec!["--test-no-seccomp".into()], ..cfg() });
    assert_eq!(sandbox_failure(sb.decode(0, red_svg(), &icon_opts(8))), Some(SandboxFailure::Unavailable));
    for (name, flags, landlock) in [("no-seccomp", 0, 2), ("no-landlock", 1, 0)] {
        let sb = Sandbox::new(SandboxConfig { worker: Some(fake_worker(dir.path(), name, flags, landlock)), ..cfg() });
        assert_eq!(sandbox_failure(sb.decode(0, red_svg(), &icon_opts(8))), Some(SandboxFailure::Unavailable), "{name}");
        assert_eq!(sb.isolation(), Isolation::Unavailable);
    }
    // Pozitif kontrol: aynı sahte işçi kilit beyanıyla kabul edilir (sonra yanıt vermediği
    // için zaman aşımı) — red gerçekten bayraklardan.
    let ok = fake_worker(dir.path(), "ok", 1, 2);
    let sb = Sandbox::new(SandboxConfig { worker: Some(ok), timeout: Duration::from_millis(300), ..cfg() });
    assert_eq!(sandbox_failure(sb.decode(0, red_svg(), &icon_opts(8))), Some(SandboxFailure::Timeout));
    drop(sb);
    assert_eq!(children().1, 0);
}

/// Bulgu 2d: miras fd'ler el sıkışmada sayılır (işçi onları kullanamaz; seccomp).
#[test]
fn inherited_fds_are_reported() {
    let _s = serial();
    let base = {
        let sb = Sandbox::new(cfg());
        sb.decode(0, red_svg(), &icon_opts(8)).unwrap();
        sb.stats().inherited_fds
    };
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("sizinti");
    std::fs::write(&f, b"x").unwrap();
    // CLOEXEC'siz açılır: alt sürece miras geçer.
    let leaked = rustix::fs::open(&f, rustix::fs::OFlags::RDONLY, rustix::fs::Mode::empty()).unwrap();
    let sb = Sandbox::new(cfg());
    sb.decode(0, red_svg(), &icon_opts(8)).unwrap();
    assert!(sb.stats().inherited_fds > base, "{} > {base}", sb.stats().inherited_fds);
    drop(sb);
    drop(leaked);
}

/// İptal anahtarı ≠ kaynak anahtarı: aynı uygulamanın art arda bildirimleri (farklı iptal)
/// birbirini iptal etmez, ama susturma uygulama (kaynak) başına ortaktır.
#[test]
fn loader_cancel_and_source_keys_are_independent() {
    let _s = serial();
    let sb = Arc::new(Sandbox::new(cfg()));
    let mut loader = nuxut_image::Loader::new(1).with_sandbox(sb.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    // PNG imzalı çöp: işçide çözme hatası → kaynak anahtarı için başarısızlık.
    let bad: Arc<[u8]> = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDRbozuk".to_vec().into();
    for cancel in 1..=3u64 {
        let tx = tx.clone();
        let req = nuxut_image::LoadRequest { source: nuxut_image::Source::Bytes(bad.clone()), options: icon_opts(16) };
        loader.load_with_keys(nuxut_image::LoadKeys { cancel, source: 55 }, req, move |r| tx.send(r.map(|_| ())).unwrap());
    }
    for _ in 0..3 {
        let r = rx.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(r.is_err() && !matches!(r, Err(ImageError::Cancelled) | Err(ImageError::Sandbox(_))), "iptal edilmedi, işçi hatası: {r:?}");
    }
    // Kaynak 55 susturuldu: yeni iptal anahtarıyla gelen geçerli SVG de reddedilir.
    let tx2 = tx.clone();
    let req = nuxut_image::LoadRequest { source: nuxut_image::Source::Bytes(red_svg()), options: icon_opts(16) };
    loader.load_with_keys(nuxut_image::LoadKeys { cancel: 4, source: 55 }, req, move |r| tx2.send(r.map(|_| ())).unwrap());
    assert!(matches!(rx.recv_timeout(Duration::from_secs(20)).unwrap(), Err(ImageError::Sandbox(SandboxFailure::RateLimited))));
    // Başka kaynak etkilenmez.
    let req = nuxut_image::LoadRequest { source: nuxut_image::Source::Bytes(red_svg()), options: icon_opts(16) };
    loader.load_with_keys(nuxut_image::LoadKeys { cancel: 4, source: 56 }, req, move |r| tx.send(r.map(|_| ())).unwrap());
    assert!(rx.recv_timeout(Duration::from_secs(20)).unwrap().is_ok());
}

fn svg_with_image(href: Option<&str>) -> Arc<[u8]> {
    let image = href.map(|h| format!(r#"<image x="0" y="0" width="16" height="16" xlink:href="{h}"/>"#)).unwrap_or_default();
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="16" height="16"><rect width="16" height="16" fill="red"/>{image}<circle cx="8" cy="8" r="4" fill="blue"/></svg>"#
    )
    .into_bytes()
    .into()
}

/// Talep, test 2 (gerçek işçi): `<image href>` dış kaynakları işçiyi ÖLDÜRMEDEN
/// yanıtlanır — çözücü dosya/soket açmaya hiç kalkmaz (kalksaydı seccomp işçiyi öldürürdü:
/// `Killed`). Çıktı `<image>`'sız hâliyle aynı. Var olan dosyanın yolu da denenir.
#[test]
fn worker_never_opens_image_hrefs() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let png_path = dir.path().join("x.png");
    std::fs::write(&png_path, &*png(16, 16)).unwrap();
    let nested = "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI4IiBoZWlnaHQ9IjgiPjxyZWN0IHdpZHRoPSI4IiBoZWlnaHQ9IjgiLz48L3N2Zz4=";
    let hrefs = [
        "file:///etc/passwd".to_owned(),
        format!("file://{}", png_path.display()),
        png_path.display().to_string(),
        "../x.png".into(),
        "x.png".into(),
        "/dev/zero".into(),
        "http://127.0.0.1:9/a.png".into(),
        "https://example.com/a.png".into(),
        "ftp://example.com/a.png".into(),
        "//host/a.png".into(),
        format!("data:image/svg+xml;base64,{nested}"),
    ];
    let sb = Sandbox::new(cfg());
    let opts = DecodeOptions { limits: Limits::ICON, target: None, animate: false };
    let base = sb.decode(1, svg_with_image(None), &opts).unwrap();
    for (i, href) in hrefs.iter().enumerate() {
        let got = match sb.decode(100 + i as u64, svg_with_image(Some(href)), &opts) {
            Ok(img) => img,
            Err(e) => panic!("{href}: işçi yanıt vermedi ({e}) — dış kaynak açılmaya çalışıldı"),
        };
        assert!(got == base, "{href}: <image> bir şey yükledi");
    }
    assert_eq!(sb.stats().failures, 0, "hiçbir işçi ölmedi");
}

/// Settings'in gördüğü boştaki işçiler: girdi görmemiş taze işçiler mi, kaç tane, ne zaman
/// kapanıyor? Hizmet eden PID'ler ile boşta kalan PID'ler ayrık; boşta en çok `prewarm`;
/// `idle_timeout` sonra hiç süreç yok.
fn burst_pids(sb: &Arc<Sandbox>) -> std::collections::HashSet<u32> {
    let threads: Vec<_> = (0..3)
        .map(|t| {
            let sb = sb.clone();
            std::thread::spawn(move || {
                (0..3)
                    .map(|i| {
                        let img = sb.decode_hooked(1000 + t * 10 + i, b"pid".to_vec().into(), &icon_opts(8), Hook::Pid).unwrap();
                        u32::from_le_bytes(img.pixels()[..4].try_into().unwrap())
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    threads.into_iter().flat_map(|t| t.join().unwrap()).collect()
}

#[test]
fn idle_fresh_workers_are_bounded_unused_and_reaped() {
    let _s = serial();
    for (prewarm, max_idle) in [(1usize, 1usize), (2, 2), (0, 0)] {
        let sb = Arc::new(Sandbox::new(SandboxConfig { prewarm, idle_timeout: Duration::from_millis(400), ..cfg() }));
        let served = burst_pids(&sb);
        assert_eq!(served.len(), 9, "tek kullanımlık");
        only_fresh_children(&sb);
        let idle: Vec<u32> = child_pids();
        assert!(idle.len() <= max_idle, "prewarm {prewarm}: boşta {} işçi", idle.len());
        for pid in &idle {
            assert!(!served.contains(pid), "boştaki {pid} daha önce hizmet etti");
            if prewarm == 1 {
                let st = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
                let rss = st.lines().find(|l| l.starts_with("VmRSS")).unwrap_or("VmRSS: ?");
                eprintln!("boştaki taze işçi (debug derleme) {rss}");
            }
        }
        assert!(eventually(|| children() == (0, 0)), "prewarm {prewarm}: idle_timeout sonra süreç kalmamalı");
    }
}
