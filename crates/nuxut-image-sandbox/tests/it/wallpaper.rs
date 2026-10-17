//! Duvar kağıdı profili ve `WallpaperDecoder` (ARCHITECTURE "Duvar kağıdı yalıtımı"), gerçek
//! (kancalı) işçiyle: profil sınırları, dosya akışı, iptal, negatif önbellek kuralları, K2-b
//! yedeği, paylaşılan çözüm/türetme/linger ve boşta sıfır thread/işçi değişmezi.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use nuxut_image::{
    CancelFlag, DecodeOptions, DecodedFile, Fit, ImageError, Isolation, Limits, SandboxFailure, TargetSize, Trust, WallpaperDecoder,
    WallpaperDecoderOptions, WallpaperFrame, WallpaperRequest, open_user_file,
};
use nuxut_image_sandbox::hooks::Hook;
use nuxut_image_sandbox::{Sandbox, SandboxConfig, WorkerProfile};

use crate::{WORKER, serial};

/// Durum bekleyişlerinin (`eventually`) üst sınırı: koşul sağlanınca hemen döner; yalnızca
/// takılan testi bitirir. Yüklü VM'de bellek bekçisi (`nuxut-mem-watch`) derleme/test dilimini
/// saniyelerce dondurabiliyor; birkaç saniyelik sınırlar o yüzden yanlış düşüyordu.
const SETTLE: Duration = Duration::from_secs(60);
/// Çözücü sonucunu bekleme sınırı (soğuk işçi + debug çözüm + dondurma payı).
const RESULT_WAIT: Duration = Duration::from_secs(180);

/// Testlere özgü duvar kağıdı yapılandırması. Ürün sınırları (`SandboxConfig::wallpaper()`:
/// rlimit'ler, girdi/çıktı tavanı) aynen kalır; yalnızca duvar saati süreleri geniş: üründeki
/// 20 sn istek süresi debug derlemede, aç kalan/dondurulan test sürecinde soğuk ilk çözümde
/// (35 sn gözlendi) aşılıyordu. Zaman aşımını sınayan testler kendi kısa süresini verir.
fn wp_cfg() -> SandboxConfig {
    SandboxConfig {
        worker: Some(WORKER.into()),
        timeout: Duration::from_secs(120),
        spawn_timeout: Duration::from_secs(60),
        rate_per_sec: 100_000,
        burst: 100_000,
        ..SandboxConfig::wallpaper()
    }
}

fn opts(target: TargetSize) -> DecodeOptions {
    DecodeOptions { limits: Limits::WALLPAPER, target: Some(target), animate: false }
}

/// Yumuşak gradyan + hafif desen (Lanczos sınırı ölçülebilir, aşırı salınım yok).
fn write_png(path: &Path, w: u32, h: u32, salt: u8) {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, salt.wrapping_add(((x / 16 + y / 16) % 2) as u8 * 40)]));
    img.save(path).unwrap();
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Yalnızca başlık: imza + IHDR (`depth`, `color`) + boş IDAT + IEND.
fn png_header(w: u32, h: u32, depth: u8, color: u8) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[depth, color, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &[]);
    chunk(b"IEND", &[]);
    out
}

/// Başında `head` olan, toplam `len` baytlık seyrek dosya (diskte yer tutmaz).
fn sparse(path: &Path, head: &[u8], len: u64) {
    std::fs::write(path, head).unwrap();
    File::options().write(true).open(path).unwrap().set_len(len).unwrap();
}

fn sandbox_failure<T>(r: &Result<T, ImageError>) -> Option<SandboxFailure> {
    match r {
        Err(ImageError::Sandbox(f)) => Some(*f),
        _ => None,
    }
}

fn decode(sb: &Sandbox, path: &Path, target: TargetSize) -> Result<DecodedFile, ImageError> {
    sb.decode_file(nuxut_image::file_key("wallpaper", path), open_user_file(path)?, &opts(target), &CancelFlag::new())
}

/// Bu sürecin çocukları (pid, cmdline).
fn children() -> Vec<(u32, String)> {
    let me = std::process::id().to_string();
    let mut out = Vec::new();
    for e in std::fs::read_dir("/proc").unwrap().flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        let Some(rest) = stat.rsplit_once(')').map(|(_, r)| r) else { continue };
        if rest.split_whitespace().nth(1) == Some(me.as_str()) {
            let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
            out.push((pid, String::from_utf8_lossy(&cmd).replace('\0', " ")));
        }
    }
    out
}

/// Bu süreçte adı `prefix` ile başlayan thread sayısı.
fn threads_named(prefix: &str) -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .flatten()
        .filter(|t| std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.trim_end().starts_with(prefix)))
        .count()
}

/// Sürecin toplam CPU süresi (utime + stime, saat tıkı).
fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let rest = stat.rsplit_once(')').unwrap().1;
    let f: Vec<&str> = rest.split_whitespace().collect();
    f[11].parse::<u64>().unwrap() + f[12].parse::<u64>().unwrap()
}

fn vm_rss_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap();
    s.lines().find_map(|l| l.strip_prefix("VmRSS:")).and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok()).unwrap()
}

fn eventually(limit: Duration, cond: impl Fn() -> bool) -> bool {
    let until = Instant::now() + limit;
    while Instant::now() < until {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    cond()
}

/// Kırpılmış Cover: çıktı tam ≤ kutu; işçi süreç içiyle piksel aynı; tam Cover + ortadan
/// kırpmayla iç bölgede ±1 (Lanczos sınır payı yalnızca kenarda).
#[test]
fn cover_crop_through_worker_matches_in_process() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pano.png");
    write_png(&path, 2000, 1000, 7);
    let sb = Sandbox::new(wp_cfg());
    assert_eq!(sb.isolation(), Isolation::Process);
    let got = decode(&sb, &path, TargetSize::cover_crop(640, 480)).unwrap();
    assert_eq!(got.image.size(), (640, 480), "çıktı kutuyu aşmaz");
    assert!(!got.animated_source);
    assert_eq!(got.animation, None);
    let bytes = std::fs::read(&path).unwrap();
    let local = nuxut_image::decode_bytes(&bytes, &opts(TargetSize::cover_crop(640, 480)).with_trust(Trust::System)).unwrap().into_first_frame();
    assert!(got.image.pixels() == local.pixels(), "işçi = süreç içi");
    // Kırpmasız Cover 960×480; ortadaki 640 sütun.
    let full = nuxut_image::decode_bytes(&bytes, &opts(TargetSize::new(640, 480, Fit::Cover)).with_trust(Trust::System)).unwrap().into_first_frame();
    assert_eq!(full.size(), (960, 480));
    let (mut interior_max, mut edge_max) = (0u8, 0u8);
    for y in 0..480u32 {
        for x in 0..640u32 {
            let a = &got.image.pixels()[((y * 640 + x) * 4) as usize..][..4];
            let b = &full.pixels()[((y * 960 + x + 160) * 4) as usize..][..4];
            let d = a.iter().zip(b).map(|(p, q)| p.abs_diff(*q)).max().unwrap();
            if (4..636).contains(&x) {
                interior_max = interior_max.max(d);
            } else {
                edge_max = edge_max.max(d);
            }
        }
    }
    assert!(interior_max <= 1, "iç bölge ±1: {interior_max}");
    assert!(edge_max <= 24, "kenar (Lanczos sınırı): {edge_max}");
}

/// Tel v4: animasyonlu kaynağın kare sayısı ve tur süresi işçide kap yapısından sayılır ve
/// yanıtla gelir (ana süreç kabı yürümez; `nuxut-video` içe aktarma bütçesi bunu kullanır).
/// İşçi yokken (S1) K2-b yedeği aynı bilgiyi süreç içinde verir.
#[test]
fn animation_info_comes_from_the_worker() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let gif = dir.path().join("a.gif");
    {
        let delays = [30u32, 50, 70];
        let frames = delays.iter().enumerate().map(|(i, &ms)| {
            let px = image::RgbaImage::from_pixel(40, 20, image::Rgba([i as u8 * 80, 0, 255, 255]));
            image::Frame::from_parts(px, 0, 0, image::Delay::from_numer_denom_ms(ms, 1))
        });
        let mut out = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut out).encode_frames(frames).unwrap();
        std::fs::write(&gif, out).unwrap();
    }
    let want = Some(nuxut_image::AnimationInfo { frames: 3, duration: Duration::from_millis(150) });
    let sb = Sandbox::new(wp_cfg());
    assert_eq!(sb.isolation(), Isolation::Process);
    let got = decode(&sb, &gif, TargetSize::cover_crop(20, 10)).unwrap();
    assert!(got.animated_source);
    assert_eq!(got.animation, want);
    assert_eq!(got.image.size(), (20, 10));
    let fallback = Sandbox::new(SandboxConfig { worker: Some(dir.path().join("yok")), ..wp_cfg() });
    assert_eq!(decode(&fallback, &gif, TargetSize::cover_crop(20, 10)).unwrap().animation, want);
}

/// Profil kapıları: dosya boyutu (işçi başlamaz), kenar, çözücü bütçesi; 8 bit 16K² geçer.
#[test]
fn wallpaper_profile_gates() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let sb = Sandbox::new(wp_cfg());
    let big = dir.path().join("big.png");
    sparse(&big, &png_header(64, 64, 8, 2), (256 << 20) + 1);
    let r = decode(&sb, &big, TargetSize::cover_crop(3840, 2160));
    assert!(matches!(r, Err(ImageError::FileTooLarge { size, limit }) if size == (256 << 20) + 1 && limit == 256 << 20), "{r:?}");
    assert_eq!(sb.stats().spawned, 0, "boyut kapısı işçi başlatmaz");
    let cases = [
        ("kenar", png_header(16_385, 16, 8, 6), true),
        ("bütçe-16bit-rgba", png_header(16_384, 16_384, 16, 6), true),
        ("8bit-rgb-16k", png_header(16_384, 16_384, 8, 2), false),
    ];
    for (name, bytes, too_large) in cases {
        let p = dir.path().join(format!("{name}.png"));
        std::fs::write(&p, bytes).unwrap();
        let r = decode(&sb, &p, TargetSize::cover_crop(3840, 2160));
        if too_large {
            assert!(matches!(r, Err(ImageError::TooLarge { .. })), "{name}: {r:?}");
        } else {
            // Kapılardan geçti (JPEG 16K² ≈ 805 MB bütçede); boş IDAT çözme hatası.
            assert!(matches!(r, Err(ImageError::Decode(_))), "{name}: {r:?}");
        }
    }
    assert_eq!(sb.stats().failures, 0, "işçi ölmedi");
    assert_eq!(sb.stats().negative_hits, 0);
}

/// Duvar kağıdı: zaman aşımı negatif önbelleğe GİRMEZ (yük altında meşru dosya karalisteye
/// düşmesin); SIGXCPU (`RLIMIT_CPU` 10 sn) `Killed` olarak GİRER.
#[test]
fn timeout_is_not_negatively_cached_but_cpu_kill_is() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("spin.png");
    write_png(&path, 32, 32, 1);
    // Anahtar susturma eşiği yüksek: aşağıdaki meşru çözüm yük altında kısa sürede zaman aşımına
    // düşüp yeniden denenebilsin (susturma bu testin konusu değil; `rate_limit_and_key_mute`).
    let sb = Sandbox::new(SandboxConfig { timeout: Duration::from_millis(400), key_failure_threshold: u32::MAX, ..wp_cfg() });
    for i in 0..2 {
        let r = sb.decode_file_hooked(9, open_user_file(&path).unwrap(), &opts(TargetSize::cover_crop(64, 64)), &CancelFlag::new(), Hook::Spin);
        assert_eq!(sandbox_failure(&r), Some(SandboxFailure::Timeout), "deneme {i}");
    }
    assert_eq!(sb.stats().negative_hits, 0, "zaman aşımı negatif önbellekte değil");
    assert_eq!(sb.stats().spawned, 2, "ikinci deneme yine işçiye gitti");
    // Aynı dosya kancasız çözülür. 400 ms bu sandbox'ın süresi: aç kalan/dondurulan test
    // sürecinde meşru çözüm de aşabilir. Zaman aşımı negatif önbelleğe girmediği için yeniden
    // denenir (testin iddiası tam da bu); her başarısızlık `Timeout` olmalı — negatif önbellek
    // isabeti `Killed` dönerdi ve `negative_hits` artardı.
    let mut attempts = 0;
    let img = loop {
        attempts += 1;
        match decode(&sb, &path, TargetSize::cover_crop(16, 16)) {
            Ok(got) => break got.image,
            Err(e) => {
                assert_eq!(sandbox_failure(&Err::<(), _>(e)), Some(SandboxFailure::Timeout), "deneme {attempts}");
                assert!(attempts < 50, "meşru çözüm {attempts} denemede de bitmedi");
            }
        }
    };
    assert_eq!(img.size(), (16, 16));
    assert_eq!(sb.stats().negative_hits, 0);

    // Duvar saati süresi yalnızca güvenlik ağı: işçiyi `RLIMIT_CPU` (10 sn CPU) öldürmeli. CPU'su
    // kısılan işçi 10 sn CPU'yu 90 sn duvar saatinde biriktiremeyip `Timeout` alıyordu.
    let sb = Sandbox::new(SandboxConfig { timeout: Duration::from_secs(600), ..wp_cfg() });
    let started = Instant::now();
    let r = sb.decode_file_hooked(9, open_user_file(&path).unwrap(), &opts(TargetSize::cover_crop(64, 64)), &CancelFlag::new(), Hook::Spin);
    assert_eq!(sandbox_failure(&r), Some(SandboxFailure::Killed), "SIGXCPU");
    assert!(started.elapsed() >= Duration::from_secs(9), "CPU sınırı 10 sn: {:?}", started.elapsed());
    let r = decode(&sb, &path, TargetSize::cover_crop(16, 16));
    assert_eq!(sandbox_failure(&r), Some(SandboxFailure::Killed), "dosya kimliği negatif önbellekte");
    assert_eq!(sb.stats().negative_hits, 1);
    // Dosya değişince kimlik değişir: yeniden denenir.
    std::thread::sleep(Duration::from_millis(20));
    write_png(&path, 33, 32, 1);
    assert!(decode(&sb, &path, TargetSize::cover_crop(16, 16)).is_ok());
    assert!(children().is_empty());
}

/// `RLIMIT_AS` 2.5 GiB (3 GiB ayırma ölür) ve kırpılmış Cover'da taşan eksen protokol ihlali.
#[test]
fn address_space_limit_and_crop_forgery() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.png");
    write_png(&path, 16, 16, 2);
    let sb = Sandbox::new(wp_cfg());
    let path2 = dir.path().join("y.png");
    write_png(&path2, 16, 16, 9);
    // Her kanca ayrı dosya: ilki dosya kimliğini negatif önbelleğe koyar.
    let run = |p: &Path, hook| sb.decode_file_hooked(11, open_user_file(p).unwrap(), &opts(TargetSize::cover_crop(64, 64)), &CancelFlag::new(), hook);
    assert_eq!(sandbox_failure(&run(&path, Hook::Alloc3G)), Some(SandboxFailure::Killed));
    assert_eq!(sandbox_failure(&run(&path2, Hook::SpoofCropOverflow)), Some(SandboxFailure::Protocol));
    assert_eq!(sandbox_failure(&run(&path, Hook::Pid)), Some(SandboxFailure::Killed), "negatif önbellek");
    assert_eq!(sb.stats().negative_hits, 1);
    assert!(children().is_empty());
}

/// Yanlış profil beyan eden işçi (gerçek ya da sahte) kullanılamaz.
#[test]
fn profile_mismatch_is_unavailable() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.png");
    write_png(&path, 16, 16, 3);
    let strict = |cfg: SandboxConfig| Sandbox::new(SandboxConfig { in_process_fallback: false, ..cfg });
    let sb = strict(SandboxConfig { worker_args: vec!["--test-wrong-profile".into()], ..wp_cfg() });
    assert_eq!(sandbox_failure(&decode(&sb, &path, TargetSize::cover_crop(8, 8))), Some(SandboxFailure::Unavailable));
    assert_eq!(sb.isolation(), Isolation::Unavailable);
    // İkon profilli Sandbox da yanlış beyanı reddeder.
    let icon = Sandbox::new(SandboxConfig { worker: Some(WORKER.into()), worker_args: vec!["--test-wrong-profile".into()], in_process_fallback: false, ..SandboxConfig::default() });
    assert_eq!(icon.profile(), WorkerProfile::Icon);
    let png: Arc<[u8]> = std::fs::read(&path).unwrap().into();
    let r = icon.decode(0, png, &DecodeOptions { limits: Limits::ICON, target: None, animate: false });
    assert_eq!(sandbox_failure(&r), Some(SandboxFailure::Unavailable));
    // Sahte işçi: ikon profili beyan eder, duvar kağıdı ebeveyni reddeder.
    let fake = dir.path().join("fake");
    {
        use std::os::unix::fs::PermissionsExt;
        let v = env!("CARGO_PKG_VERSION");
        let pv = nuxut_image_sandbox::protocol::PROTOCOL_VERSION;
        let script = format!("#!/bin/sh\nprintf 'NXIW\\{pv:03o}\\000\\001\\002\\000\\000\\001\\{:03o}%s' '{v}'\nexec sleep 30\n", v.len());
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let sb = strict(SandboxConfig { worker: Some(fake), ..wp_cfg() });
    assert_eq!(sandbox_failure(&decode(&sb, &path, TargetSize::cover_crop(8, 8))), Some(SandboxFailure::Unavailable));
    assert!(children().is_empty());
}

/// Akış: 200 MiB dosya ebeveynin belleğine alınmaz (VmRSS artışı < 8 MiB).
#[test]
fn streams_large_file_with_bounded_parent_memory() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.png");
    sparse(&path, &png_header(64, 64, 8, 2), 200 << 20);
    let sb = Sandbox::new(wp_cfg());
    let base = vm_rss_kib();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let sampler = {
        let done = done.clone();
        std::thread::spawn(move || {
            let mut peak = 0;
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                peak = peak.max(vm_rss_kib());
                std::thread::sleep(Duration::from_millis(2));
            }
            peak
        })
    };
    let r = decode(&sb, &path, TargetSize::cover_crop(64, 64));
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let peak = sampler.join().unwrap();
    assert!(matches!(r, Err(ImageError::Decode(_))), "{r:?}");
    assert_eq!(sb.stats().failures, 0);
    let grew = peak.saturating_sub(base);
    eprintln!("200 MiB akış: ebeveyn VmRSS artışı {grew} KiB");
    assert!(grew < 8 << 10, "ebeveyn {grew} KiB büyüdü");
}

/// Akış ortasında iptal: işçi ≤ 100 ms'de ölür, `Cancelled`, negatif önbellek ve susturma
/// etkilenmez. Duvar kağıdı işçisinin önceliği: nice 10, `oom_score_adj` 1000.
#[test]
fn cancel_mid_stream_kills_worker_quickly() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stall.png");
    sparse(&path, &png_header(64, 64, 8, 2), 32 << 20);
    let sb = Arc::new(Sandbox::new(wp_cfg()));
    let cancel = CancelFlag::new();
    let worker = {
        let (sb, cancel, path) = (sb.clone(), cancel.clone(), path.clone());
        std::thread::spawn(move || sb.decode_file_hooked(5, open_user_file(&path).unwrap(), &opts(TargetSize::cover_crop(64, 64)), &cancel, Hook::SpinBeforePayload))
    };
    // İşçi başladı ve ebeveyn borunun dolmasıyla akışta bekliyor.
    assert!(eventually(SETTLE, || children().iter().any(|(_, c)| c.contains("--profile=wallpaper"))));
    std::thread::sleep(Duration::from_millis(300));
    let (pid, _) = children().into_iter().find(|(_, c)| c.contains("--profile=wallpaper")).unwrap();
    let adj = std::fs::read_to_string(format!("/proc/{pid}/oom_score_adj")).unwrap();
    assert_eq!(adj.trim(), "1000");
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let nice: i32 = stat.rsplit_once(')').unwrap().1.split_whitespace().nth(16).unwrap().parse().unwrap();
    assert_eq!(nice, 10);
    assert!(!worker.is_finished(), "akış sürüyor olmalı");
    let t = Instant::now();
    cancel.cancel();
    let r = worker.join().unwrap();
    let took = t.elapsed();
    eprintln!("iptal → işçi ölü: {took:?}");
    assert!(matches!(r, Err(ImageError::Cancelled)), "{r:?}");
    assert!(!Path::new(&format!("/proc/{pid}")).exists() || children().iter().all(|(p, _)| *p != pid), "işçi öldürüldü ve beklendi");
    assert!(took <= Duration::from_millis(100), "iptal {took:?}");
    let st = sb.stats();
    assert_eq!((st.cancelled, st.failures, st.negative_hits), (1, 0, 0));
    // Aynı dosya negatif önbellekte değil: işçiye gider (boş IDAT → çözme hatası).
    assert!(matches!(decode(&sb, &path, TargetSize::cover_crop(64, 64)), Err(ImageError::Decode(_))));
    assert_eq!(sb.stats().negative_hits, 0);
}

/// S1: işçi yokken temel raster süreç içinde (aynı kapılar), JXL/SVG asla;
/// `in_process_fallback: false` kapalı-güvenli.
#[test]
fn unavailable_wallpaper_falls_back_for_basic_raster_only() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let png = dir.path().join("x.png");
    write_png(&png, 300, 200, 4);
    let jxl = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../nuxut-image-worker/tests/fixtures/red4.jxl"));
    let svg = dir.path().join("x.svg");
    std::fs::write(&svg, br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>"#).unwrap();
    let giant = dir.path().join("giant.png");
    std::fs::write(&giant, png_header(16_385, 16, 8, 6)).unwrap();
    let sb = Sandbox::new(SandboxConfig { worker: Some(dir.path().join("yok")), ..wp_cfg() });
    assert_eq!(sb.isolation(), Isolation::Unavailable);
    assert_eq!(decode(&sb, &png, TargetSize::cover_crop(150, 100)).unwrap().image.size(), (150, 100));
    assert_eq!(sandbox_failure(&decode(&sb, &jxl, TargetSize::cover_crop(4, 4))), Some(SandboxFailure::Unavailable));
    assert_eq!(sandbox_failure(&decode(&sb, &svg, TargetSize::cover_crop(4, 4))), Some(SandboxFailure::Unavailable));
    assert!(matches!(decode(&sb, &giant, TargetSize::cover_crop(4, 4)), Err(ImageError::TooLarge { .. })), "aynı kapılar");
    assert_eq!(sb.stats().in_process_fallbacks, 2);
    let strict = Sandbox::new(SandboxConfig { worker: Some(dir.path().join("yok")), in_process_fallback: false, ..wp_cfg() });
    assert_eq!(sandbox_failure(&decode(&strict, &png, TargetSize::cover_crop(8, 8))), Some(SandboxFailure::Unavailable));
    assert_eq!(strict.stats().in_process_fallbacks, 0);
    // Gerçek işçiyle JXL çözülür (yalnızca işçide).
    let sb = Sandbox::new(wp_cfg());
    assert_eq!(decode(&sb, &jxl, TargetSize::cover_crop(4, 4)).unwrap().image.size(), (4, 4));
}

/// Kullanıcı SVG duvar kağıdı işçide kırpılmış Cover ile çözülür: çıktı tam kutu, ortadan
/// kırpılmış. Önceden `render_svg` kırpmayı yok sayıp ölçeklenmiş tam görüntüyü veriyordu;
/// ebeveyn (kırpılmış Cover: iki eksen ≤ kutu) bunu `Protocol` ile reddediyordu.
#[test]
fn user_svg_wallpaper_is_cropped_in_worker() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let svg = dir.path().join("duvar.svg");
    // 2:1, sol yarı kırmızı, sağ yarı mavi.
    std::fs::write(&svg, br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10" preserveAspectRatio="xMidYMid slice"><rect width="10" height="10" fill="#ff0000"/><rect x="10" width="10" height="10" fill="#0000ff"/></svg>"##).unwrap();
    let sb = Sandbox::new(wp_cfg());
    let img = decode(&sb, &svg, TargetSize::cover_crop(400, 400)).expect("Protocol değil").image;
    assert_eq!(img.size(), (400, 400));
    assert_eq!(img.straight_pixel(50, 200), [255, 0, 0, 255]);
    assert_eq!(img.straight_pixel(350, 200), [0, 0, 255, 255]);
    assert_eq!(sb.stats().failures, 0);
}

fn wp_decoder(linger: Duration) -> (Arc<Sandbox>, Arc<WallpaperDecoder>) {
    let sb = Arc::new(Sandbox::new(wp_cfg()));
    let dec = WallpaperDecoder::new(sb.clone(), WallpaperDecoderOptions { linger, ..WallpaperDecoderOptions::default() });
    (sb, dec)
}

type Rx = mpsc::Receiver<Result<WallpaperFrame, ImageError>>;

fn request(dec: &Arc<WallpaperDecoder>, path: &Path, px: (u32, u32), fit: Fit) -> (nuxut_image::WallpaperTicket, Rx) {
    let (tx, rx) = mpsc::channel();
    let t = dec.request(WallpaperRequest::new(path, px, fit), move |r| {
        let _ = tx.send(r);
    });
    (t, rx)
}

fn recv(rx: &Rx) -> Result<WallpaperFrame, ImageError> {
    rx.recv_timeout(RESULT_WAIT).expect("sonuç gelmedi")
}

/// Çok çıkış tek çözüm (birleştirme), 4K+1080p tek çözüm (türetme), isabet; `trim` ve linger
/// sonra boşta: kare, thread, işçi ve CPU sıfır.
#[test]
fn decoder_joins_derives_lingers_and_goes_idle() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wp.png");
    write_png(&path, 1600, 900, 5);
    // Linger sayaç sınamaları boyunca dolmasın: önceki 4 sn'lik linger, aç kalan/dondurulan test
    // sürecinde 800×450 çözümü sürerken 400×300 karesini düşürüyor, isabet/türetme sayaçları
    // yanlış düşüyordu. Linger'ın kendi süresi aşağıda ayrı, kısa linger'lı çözücüyle sınanır.
    let (sb, dec) = wp_decoder(Duration::from_secs(600));
    // Önceki testlerin (linger 10 sn) düşürülmüş çözücü thread'leri bitene dek bekle; bu çözücü
    // kurulumda thread açmaz.
    assert!(eventually(SETTLE, || threads_named("nuxut-wallpaper") == 0), "kurulumda thread yok");
    // 3 çıkış, aynı çözünürlük.
    let reqs: Vec<_> = (0..3).map(|_| request(&dec, &path, (400, 300), Fit::Cover)).collect();
    let frames: Vec<WallpaperFrame> = reqs.iter().map(|(_, rx)| recv(rx).unwrap()).collect();
    assert!(frames.iter().all(|f| f.size() == (400, 300) && Arc::ptr_eq(&f.pixels, &frames[0].pixels)), "tek tampon paylaşılır");
    let st = dec.stats();
    assert_eq!((st.decodes, st.joined), (1, 2), "{st:?}");
    // Karışık çözünürlük, Contain: büyük çözülür, küçük türetilir.
    let (_a, big) = request(&dec, &path, (800, 450), Fit::Contain);
    assert_eq!(recv(&big).unwrap().size(), (800, 450));
    let (_b, small) = request(&dec, &path, (400, 225), Fit::Contain);
    assert_eq!(recv(&small).unwrap().size(), (400, 225));
    let st = dec.stats();
    assert_eq!((st.decodes, st.derived), (2, 1), "{st:?}");
    assert_eq!(sb.stats().decoded, 2, "işçiye yalnızca iki istek");
    // Aynı istek tekrar: isabet.
    let (_c, again) = request(&dec, &path, (400, 300), Fit::Cover);
    recv(&again).unwrap();
    assert_eq!(dec.stats().hits, 1);
    assert!(dec.stats().resident_bytes > 0);
    drop((reqs, frames));
    // `trim` (bellek baskısı): kareler hemen bırakılır, thread linger beklemeden çıkar.
    dec.trim();
    assert!(eventually(SETTLE, || !dec.stats().thread_alive), "{:?}", dec.stats());
    let st = dec.stats();
    assert_eq!((st.resident_bytes, st.entries, st.pending), (0, 0, 0));
    // Linger: kısa linger'lı çözücü, tek istek; süre dolunca kare ve thread kendiliğinden gider.
    let (sb2, short) = wp_decoder(Duration::from_millis(200));
    let (_d, rx) = request(&short, &path, (400, 300), Fit::Cover);
    recv(&rx).unwrap();
    assert!(eventually(SETTLE, || !short.stats().thread_alive), "{:?}", short.stats());
    let st = short.stats();
    assert_eq!((st.resident_bytes, st.entries, st.pending), (0, 0, 0));
    // Boşta: thread yok, işçi yok.
    assert!(eventually(SETTLE, || threads_named("nuxut-wallpaper") == 0 && threads_named("nuxut-image-jan") == 0));
    assert!(eventually(SETTLE, || children().is_empty()));
    assert_eq!((sb.stats().live, sb.stats().idle), (0, 0));
    assert_eq!((sb2.stats().live, sb2.stats().idle), (0, 0));
    // Boşta CPU: 500 ms'de en fazla bir tık (ölçüm gürültüsü; iş yapan thread yok).
    let before = cpu_ticks();
    std::thread::sleep(Duration::from_millis(500));
    assert!(cpu_ticks() - before <= 1, "boşta CPU harcandı");
}

/// Dosya değişince yeniden çözülür; hızlı 10 değişimde en fazla 2 çözüm, yalnızca sonuncusu
/// teslim edilir.
#[test]
fn decoder_redecodes_on_change_and_drops_superseded_requests() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wp.png");
    write_png(&path, 640, 400, 6);
    let (sb, dec) = wp_decoder(Duration::from_secs(10));
    let (_t, rx) = request(&dec, &path, (320, 200), Fit::Cover);
    let first = recv(&rx).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    write_png(&path, 640, 400, 200);
    let (_t, rx) = request(&dec, &path, (320, 200), Fit::Cover);
    let second = recv(&rx).unwrap();
    assert_eq!(dec.stats().decodes, 2, "mtime değişti");
    assert!(first.pixels != second.pixels);

    let files: Vec<PathBuf> = (0..10)
        .map(|i| {
            let p = dir.path().join(format!("{i}.png"));
            write_png(&p, 1200, 800, i as u8);
            p
        })
        .collect();
    let (sb2, dec) = wp_decoder(Duration::from_secs(10));
    let (tx, rx) = mpsc::channel();
    let mut ticket = None;
    for (i, p) in files.iter().enumerate() {
        let tx = tx.clone();
        // Wallpaper::set_source gibi: yeni bilet eskisini düşürür.
        drop(ticket.take());
        ticket = Some(dec.request(WallpaperRequest::new(p, (600, 400), Fit::Cover), move |r| {
            let _ = tx.send((i, r));
        }));
    }
    let (i, r) = rx.recv_timeout(RESULT_WAIT).unwrap();
    assert_eq!(i, 9, "yalnızca son seçim teslim edilir");
    assert!(r.is_ok());
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "başka teslim yok");
    let st = dec.stats();
    assert!(st.decodes <= 2, "{st:?}");
    assert!(st.cancelled >= 8, "{st:?}");
    drop(ticket);
    drop((sb, sb2));
}

/// Animasyonlu kaynak: işçi ilk kareyi ve `ANIMATED_SOURCE` bayrağını verir. Hata birleşen
/// bütün bekleyenlere gider; bilet düşünce süren çözüm durdurulur.
#[test]
fn decoder_animated_flag_errors_and_cancel() {
    let _s = serial();
    let dir = tempfile::tempdir().unwrap();
    let gif = dir.path().join("a.gif");
    {
        let a = image::RgbaImage::from_pixel(64, 32, image::Rgba([255, 0, 0, 255]));
        let b = image::RgbaImage::from_pixel(64, 32, image::Rgba([0, 0, 255, 255]));
        let delay = image::Delay::from_numer_denom_ms(50, 1);
        let mut out = Vec::new();
        {
            let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
            enc.encode_frames([image::Frame::from_parts(a, 0, 0, delay), image::Frame::from_parts(b, 0, 0, delay)]).unwrap();
        }
        std::fs::write(&gif, out).unwrap();
    }
    let (_sb, dec) = wp_decoder(Duration::ZERO);
    let (_t, rx) = request(&dec, &gif, (64, 32), Fit::Contain);
    let f = recv(&rx).unwrap();
    assert!(f.animated_source);
    assert_eq!(&f.pixels[..4], &[255, 0, 0, 255], "ilk kare");
    let bad = dir.path().join("bad.png");
    std::fs::write(&bad, b"\x89PNG\r\n\x1a\nbozuk").unwrap();
    let rxs: Vec<_> = (0..2).map(|_| request(&dec, &bad, (64, 64), Fit::Cover)).collect();
    for (_, rx) in &rxs {
        assert!(recv(rx).is_err());
    }
    assert_eq!(dec.stats().failed, 1, "tek iş, iki bekleyen");
    // Linger 0: kuyruk boşalınca kare bırakılır.
    assert!(eventually(SETTLE, || !dec.stats().thread_alive));
    assert_eq!(dec.stats().resident_bytes, 0);
}
