//! Kodlama zaman aşımında süreç temizliği: mpv durdurulur ve dönmeden önce yok edilir (iş
//! parçacığı kalmaz, alt süreç yok), yarım çıktı silinir. Tek test: iş parçacığı listesi başka
//! testlerden etkilenmesin diye kendi ikilisinde.

use std::path::Path;
use nuxut_image::CancelFlag;
use std::time::{Duration, Instant};

use nuxut_video::{ImportError, ImportLimits, ImportRequest, Importer};

/// Bu sürecin iş parçacıkları: (tid, ad), tid sıralı.
fn threads() -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|t| {
            let tid = t.file_name().to_str()?.parse().ok()?;
            Some((tid, std::fs::read_to_string(t.path().join("comm")).unwrap_or_default().trim().to_owned()))
        })
        .collect();
    v.sort();
    v
}

/// `expected` dışındaki iş parçacıkları bitene dek en çok `limit` bekler; kalanları döndürür.
/// mpv çekirdek iş parçacığı (`core`, `mpv_create`'da ayrık/detached açılır) ve havuz
/// iş parçacıkları (`worker`) `mpv_terminate_destroy` döndükten sonra çıkış yolundadır ve
/// birkaç ms içinde biter (ölçüldü: hep < 10 ms); gerçek sızıntı süre dolunca da kalır.
fn extra_threads_after(expected: &[(u32, String)], limit: Duration) -> Vec<(u32, String)> {
    let deadline = Instant::now() + limit;
    loop {
        let extra: Vec<_> = threads().into_iter().filter(|t| !expected.contains(t)).collect();
        if extra.is_empty() || Instant::now() >= deadline {
            return extra;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Bu sürecin alt süreçleri (tüm iş parçacıklarının `children` listesi).
fn children() -> String {
    let mut out = String::new();
    for task in std::fs::read_dir("/proc/self/task").unwrap().filter_map(Result::ok) {
        out += &std::fs::read_to_string(task.path().join("children")).unwrap_or_default();
    }
    out.trim().to_owned()
}

fn leftovers(cache: &Path) -> Vec<String> {
    std::fs::read_dir(cache)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".mkv") || n.contains(".tmp"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn timeout_stops_encoder_and_cleans_up() {
    if !nuxut_video::available() {
        eprintln!("libmpv yok; test atlandı");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("clip.mkv");
    // MPEG-4 Part 2: pahalı codec, aynı boyutta da yeniden kodlanır; 30 sn 640x360 yavaş
    // H.264 ayarıyla birkaç saniye sürer.
    let status = std::process::Command::new("mpv")
        .args(["--no-config", "--really-quiet", "--audio=no", "--ovc=mpeg4", "--of=matroska"])
        .arg("av://lavfi:testsrc2=size=640x360:rate=30:duration=30")
        .arg(format!("--o={}", src.display()))
        .status();
    assert!(status.is_ok_and(|s| s.success()), "test videosu üretilemedi");

    // Taban: hiçbir mpv örneği açılmadan önce (test koşucusu + bu test).
    let baseline = threads();
    let cache = dir.path().join("cache");
    let imp = Importer::in_process_unsandboxed(cache.clone());
    // Yetenek yoklaması (kendi mpv örnekleri) ölçümden önce biter; onun da kalıntısı olmamalı.
    let _ = imp.capabilities();
    let extra = extra_threads_after(&baseline, Duration::from_secs(2));
    assert!(extra.is_empty(), "yoklamadan sonra iş parçacığı kaldı: {extra:?}");

    assert_eq!(
        ImportError::TimedOut(Duration::from_millis(800)).to_string(),
        "dönüştürme 800 ms içinde bitmedi, durduruldu"
    );
    // Duvar saatinden bağımsız: kodlama gerçekten başladıktan (ilerleme > 0,05) SONRA zaman
    // aşımı dolsun diye ilk gerçek ilerlemede geri çağırma `timeout` kadar bekler (run_loop'un
    // son tarihi ondan önce başladı, dönüşte kesin dolmuştur; tamamlanma da yarışamaz). Yük
    // altında mpv ilk ilerlemeden önce son tarihe yetişemezse süre ikiye katlanıp yeniden denenir;
    // her denemede temizlik aynı biçimde doğrulanır.
    let mut timeout = Duration::from_millis(800);
    loop {
        imp.set_limits(ImportLimits { encode_timeout: timeout, ..ImportLimits::DEFAULT });
        let progress = std::sync::Mutex::new(0f32);
        let t = Instant::now();
        let r = imp.run(&ImportRequest { source: src.clone(), target_px: (640, 360) }, &CancelFlag::new(), &|p| {
            let mut last = progress.lock().unwrap();
            if p > 0.05 && *last <= 0.05 {
                std::thread::sleep(timeout);
            }
            *last = p;
        });
        let elapsed = t.elapsed();
        assert!(matches!(r, Err(ImportError::TimedOut(d)) if d == timeout), "{r:?}");
        let p = *progress.lock().unwrap();
        assert!(p < 1.0, "ilerleme {p}");
        // Durdurulup hemen döndü: ilk ilerleme son tarihten önce, bekleme bir `timeout`.
        assert!(elapsed < timeout * 2 + Duration::from_secs(5), "{elapsed:?}");
        // mpv örneği dönmeden önce yok edildi: iş parçacığı ve alt süreç yok, yarım çıktı silindi.
        let extra = extra_threads_after(&baseline, Duration::from_secs(2));
        assert!(extra.is_empty(), "mpv iş parçacıkları kaldı: {extra:?} (taban {baseline:?})");
        assert_eq!(children(), "", "alt süreç kaldı");
        assert!(leftovers(&cache).is_empty(), "{:?}", leftovers(&cache));
        if p > 0.05 {
            break;
        }
        // Kodlama başlamadan doldu (aç kalan makine): daha uzun süreyle yeniden.
        assert!(timeout < Duration::from_secs(30), "kodlama {timeout:?} içinde hiç başlamadı");
        eprintln!("kodlama {timeout:?} içinde başlamadı; süre ikiye katlanıyor");
        timeout *= 2;
    }
}
