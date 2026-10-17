//! VA-API kod çözme yeteneği yoklaması (libmpv ile; `vainfo` gerekmez).
//!
//! Her aday codec için küçük bir test klibi yazılımla kodlanır ve `hwdec=vaapi-copy`,
//! `vo=null` ile açılır; `hwdec-current` hangi codec'in donanımla çözüldüğünü söyler. Sonuç
//! mpv sürümü + DRM render düğümleriyle anahtarlanıp `$XDG_CACHE_HOME/nuxut/hwcaps.toml`'da
//! saklanır (yoklama yalnızca sistem değişince tekrarlanır).

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::ffi::{Event, Mpv};
use crate::budget::StreamBudget;
use crate::source::{MediaSource, open_media};

/// Duvar kağıdı için düşünülen codec'ler, yazılımda çözme maliyetine göre ucuzdan pahalıya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    H264,
    Hevc,
    Vp9,
    Av1,
}

impl Codec {
    pub const ALL: [Self; 4] = [Self::H264, Self::Hevc, Self::Vp9, Self::Av1];

    /// mpv/FFmpeg codec adı (`video-format` özelliği).
    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::Hevc => "hevc",
            Self::Vp9 => "vp9",
            Self::Av1 => "av1",
        }
    }

    pub fn from_ffmpeg_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.ffmpeg_name() == name)
    }

    /// Yazılım kodlayıcısı ve kalite ayarları (mpv `ovc`/`ovcopts`). Duvar kağıdı tek seferlik
    /// arka plan işi: hız değil kalite/boyut önceliklidir.
    pub(crate) fn encoder(self) -> (&'static str, &'static str) {
        match self {
            Self::H264 => ("libx264", "crf=18,preset=slow,tune=film"),
            Self::Hevc => ("libx265", "crf=20,preset=medium,x265-params=log-level=error"),
            Self::Vp9 => ("libvpx-vp9", "crf=30,b=0,deadline=good,cpu-used=2,row-mt=1"),
            Self::Av1 => ("libsvtav1", "crf=32,preset=8"),
        }
    }
}

/// Donanımla çözülebilen codec'ler.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodeCaps {
    /// Yoklamanın anahtarı (mpv sürümü + render düğümleri).
    pub key: String,
    pub vaapi: Vec<Codec>,
}

impl DecodeCaps {
    pub fn hardware(&self, codec: Codec) -> bool {
        self.vaapi.contains(&codec)
    }

    /// Hiç donanım çözücü yok: video duvar kağıdı yazılımda çözülür (CPU maliyeti; kullanıcıya
    /// uyarı + `Importer::still_frame` ile durağan kareye geri dönüş önerilir).
    pub fn software_only(&self) -> bool {
        self.vaapi.is_empty()
    }

    /// Dönüştürmede hedef codec: donanımla çözülebilenlerin en ucuzu; hiçbiri yoksa yazılımda
    /// en ucuz çözülen H.264.
    pub fn target_codec(&self) -> Codec {
        Codec::ALL.into_iter().find(|c| self.hardware(*c)).unwrap_or(Codec::H264)
    }

    /// Bu codec'i oynatmak pahalı mı (donanım yok ve yazılımda ağır).
    pub fn expensive(&self, codec: Option<Codec>) -> bool {
        match codec {
            Some(c) if self.hardware(c) => false,
            Some(Codec::H264) => false,
            // Bilinmeyen codec'ler (mpeg4, prores…) de dönüştürülür: hedef her zaman ucuz.
            _ => true,
        }
    }
}

fn render_nodes() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/dev/dri")
        .map(|rd| rd.filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.starts_with("renderD")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Önbellekli yetenekler; anahtar değiştiyse yeniden yoklar.
pub(crate) fn load_or_probe(cache: &Path, scratch: &Path) -> DecodeCaps {
    let nodes = render_nodes();
    let version = Mpv::new(&[])
        .ok()
        .and_then(|m| m.get_string("mpv-version"))
        .unwrap_or_default();
    let key = format!("{version}|{}", nodes.join(","));
    if let Ok(text) = std::fs::read_to_string(cache)
        && let Ok(caps) = toml::from_str::<DecodeCaps>(&text)
        && caps.key == key
    {
        return caps;
    }
    let vaapi = if nodes.is_empty() || version.is_empty() { Vec::new() } else { probe(scratch) };
    tracing::info!("VA-API kod çözme: {vaapi:?}");
    let caps = DecodeCaps { key, vaapi };
    if let Ok(text) = toml::to_string(&caps) {
        let _ = crate::import::write_atomic(cache, text.as_bytes());
    }
    caps
}

fn probe(scratch: &Path) -> Vec<Codec> {
    let _ = std::fs::create_dir_all(scratch);
    let mut out = Vec::new();
    for codec in Codec::ALL {
        let clip = scratch.join(format!("probe-{}.mkv", codec.ffmpeg_name()));
        let (enc, _) = codec.encoder();
        let encoded = run_mpv(
            &[
                ("o", clip.to_str().unwrap_or_default()),
                ("of", "matroska"),
                ("ovc", enc),
                ("audio", "no"),
                ("frames", "8"),
            ],
            MediaSource::Builtin("av://lavfi:testsrc2=size=256x144:rate=8"),
            Duration::from_secs(20),
            None,
            &mut |_| true,
        ) == RunEnd::Done;
        if !encoded || !clip.exists() {
            tracing::debug!("{enc} yok ya da kodlama başarısız; {codec:?} yoklanmadı");
            let _ = std::fs::remove_file(&clip);
            continue;
        }
        let mut hw = false;
        if let Ok(m) = Mpv::new(&[
            ("vo", "null"),
            ("ao", "null"),
            ("hwdec", "vaapi-copy"),
            ("frames", "4"),
        ]) && let Ok(file) = open_media(&clip)
            && m.load(MediaSource::File(file), true).is_ok()
        {
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                match m.wait_event(0.5) {
                    Event::PlaybackRestart | Event::VideoReconfig => {
                        hw = m.get_string("hwdec-current").is_some_and(|s| s.starts_with("vaapi"));
                        if hw {
                            break;
                        }
                    }
                    Event::EndFile { .. } | Event::Shutdown => break,
                    _ => {}
                }
            }
        }
        let _ = std::fs::remove_file(&clip);
        if hw {
            out.push(codec);
        }
    }
    out
}

/// [`run_mpv`]'in sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunEnd {
    /// Normal EOF (mpv'nin bütçe kesmesi dahil).
    Done,
    /// mpv açamadı, playlist ya da hata ile bitti.
    Failed,
    /// `progress` `false` döndü (iptal).
    Stopped,
    /// Duvar saati zaman aşımı.
    TimedOut,
    /// Akış sırasında bütçe aşıldı.
    OverBudget(crate::BudgetExceeded),
}

/// Ayrı bir mpv örneğiyle `source`'u sonuna kadar oynatır (kodlama kipi için). `progress`
/// 0…1 ile çağrılır; `false` dönerse (iptal) mpv durdurulur. `budget` verilirse mpv'nin kendi
/// kesmesi (`end`, `frames`) eklenir ve akış sırasında kabın süre/kare tahmini denetlenir.
///
/// Hangi yoldan dönerse dönsün mpv örneği dönmeden önce **eşzamanlı** yok edilir
/// (`mpv_terminate_destroy`: çekirdek ve tüm iş parçacıkları biter, çıktı dosyası kapanır);
/// çağıran yarım çıktıyı güvenle silebilir.
pub(crate) fn run_mpv(
    options: &[(&str, &str)],
    source: MediaSource,
    timeout: Duration,
    budget: Option<StreamBudget>,
    progress: &mut dyn FnMut(f32) -> bool,
) -> RunEnd {
    let backstop = budget.map(|b| b.backstop()).unwrap_or_default();
    let mut all: Vec<(&str, &str)> = options.to_vec();
    all.extend(backstop.iter().map(|(k, v)| (*k, v.as_str())));
    let Ok(m) = Mpv::new(&all) else { return RunEnd::Failed };
    let end = run_loop(&m, source, timeout, budget, progress);
    // Açık sıra: önce durdur, sonra (dönüşten önce) eşzamanlı yok et.
    if end != RunEnd::Done && end != RunEnd::Failed {
        let _ = m.command(&["quit"]);
    }
    drop(m);
    end
}

fn run_loop(m: &Mpv, source: MediaSource, timeout: Duration, budget: Option<StreamBudget>, progress: &mut dyn FnMut(f32) -> bool) -> RunEnd {
    let deadline = Instant::now() + timeout;
    if m.load(source, true).is_err() {
        return RunEnd::Failed;
    }
    // percent-pos sonda kaybolur; ilerleme hiç geri gitmez.
    let mut best = 0f32;
    loop {
        match m.wait_event(0.1) {
            // Playlist açıldıysa (tek medya değil) durdurulur: iç başvurular izlenmez.
            Event::FileLoaded if !m.single_media() => {
                let _ = m.command(&["quit"]);
                return RunEnd::Failed;
            }
            Event::EndFile { reason, error } => return if reason == 0 && error >= 0 { RunEnd::Done } else { RunEnd::Failed },
            Event::Shutdown => return RunEnd::Failed,
            _ => {}
        }
        if let Some(b) = budget
            && b.estimate
        {
            // Kap kare hızı bildirmiyorsa (ör. Matroska) çözülen karelerden tahmin; dalgalanır,
            // sınırdaki kaynak yanlışlıkla reddedilmesin diye %5 pay (kesin sınır `frames`).
            let fps = m.get_f64("container-fps").or_else(|| m.get_f64("estimated-vf-fps").map(|f| f / 1.05));
            if let Err(e) = b.limits.check_estimate(m.get_f64("duration"), fps) {
                return RunEnd::OverBudget(e);
            }
        }
        if let Some(v) = m.get_f64("percent-pos") {
            best = best.max((v / 100.0).clamp(0.0, 1.0) as f32);
        }
        if !progress(best) {
            return RunEnd::Stopped;
        }
        if Instant::now() >= deadline {
            return RunEnd::TimedOut;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportLimits;

    /// Süresi bilinmeyen sonsuz kaynak: üst veri yokken de mpv'nin kendi sayacı (`frames`)
    /// kodlanan işi bütçede keser; çıktı tam bütçe kadar kare.
    #[test]
    fn endless_source_is_cut_at_frame_budget() {
        if !crate::available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("cut.mkv");
        let limits = ImportLimits { max_frames: 20, ..ImportLimits::DEFAULT };
        let end = run_mpv(
            &[("o", out.to_str().unwrap()), ("of", "matroska"), ("ovc", "libx264"), ("audio", "no")],
            MediaSource::Builtin("av://lavfi:testsrc2=size=64x36:rate=50"),
            Duration::from_secs(60),
            Some(StreamBudget { limits, estimate: false }),
            &mut |_| true,
        );
        assert_eq!(end, RunEnd::Done);
        let frames = dir.path().join("frames");
        std::fs::create_dir(&frames).unwrap();
        let status = std::process::Command::new("mpv")
            .args(["--no-config", "--really-quiet", "--ao=null", "--untimed", "--vo=image", "--vo-image-format=png"])
            .arg(format!("--vo-image-outdir={}", frames.display()))
            .arg(&out)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_dir(&frames).unwrap().count(), 20);
    }

    /// Duvar saati: sonsuz kaynak zaman aşımında durdurulur.
    #[test]
    fn endless_source_times_out() {
        if !crate::available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("t.mkv");
        let t = Instant::now();
        let end = run_mpv(
            &[("o", out.to_str().unwrap()), ("of", "matroska"), ("ovc", "libx264"), ("audio", "no")],
            MediaSource::Builtin("av://lavfi:testsrc2=size=64x36:rate=50"),
            Duration::from_millis(300),
            None,
            &mut |_| true,
        );
        assert_eq!(end, RunEnd::TimedOut);
        // Sonuç TimedOut olmalı; süre sınırı yalnız takılmayı yakalar (yük altında mpv açılışı ve
        // kapanışı 5 sn'yi aşabiliyordu: ara sıra düşen test, perf 2026-10-16).
        assert!(t.elapsed() < Duration::from_secs(60), "takıldı: {:?}", t.elapsed());
    }
}
