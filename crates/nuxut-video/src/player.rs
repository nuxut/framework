//! Video oynatıcısı: libmpv `vo=libmpv` + render API. mpv kareyi bir `render_target`
//! dokusuna çizer (`Painter::gl_pass`), doku normal `Painter::image` ile çizilir.
//!
//! Kare akışı: mpv yeni kare hazırlayınca render update callback'i `wake`'i çağırır →
//! uygulama `request_frame` → `draw` bir GL geçişi planlar → geçiş `mpv_render_context_update`
//! ile yeni kare olup olmadığına bakar ve varsa çizer. Duraklatılmış/görünmez videoda mpv kare
//! üretmez, `wake` gelmez: boşta iş yok.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nuxut_core::{Color, Corners, Rect};
use nuxut_image::WallpaperBackdrop;
use nuxut_render::glow;
use nuxut_render::gl::{GlGraveyard, GlPass, PainterGlExt};
use nuxut_render::{ImageFit, ImageStyle, Painter, Texture};

use crate::VideoError;
use crate::ffi::{Event, Mpv, RenderContext};
use crate::source::MediaSource;

/// `draw`'ın uygulamaya söyledikleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VideoStatus {
    /// Hemen bir kare daha iste (hedef doku bu karede yeni ayrıldı).
    pub redraw: bool,
}

/// Oynatılan videonun bilgisi.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoInfo {
    pub path: Option<PathBuf>,
    pub size: Option<(u32, u32)>,
    pub duration: Option<f64>,
    pub has_audio: bool,
    /// Kullanılan donanım çözücüsü (`vaapi` vb.); `None` = yazılım.
    pub hwdec: Option<String>,
    pub error: Option<String>,
}

/// GL geçişiyle paylaşılan durum. Alan sırası düşürme sırasıdır: render context mpv'den önce.
struct Shared {
    render: Mutex<Option<RenderContext>>,
    /// Render update callback'i geldi (yeni kare olabilir).
    dirty: AtomicBool,
    /// Hedef yeni ayrıldı: kare olmasa da çiz (boş kalmasın).
    force: AtomicBool,
    wake: Arc<dyn Fn() + Send + Sync>,
    mpv: Mpv,
}

struct MpvPass(Arc<Shared>);

impl GlPass for MpvPass {
    fn render(&self, _gl: &glow::Context, fbo: u32, size: (u32, u32)) {
        let s = &self.0;
        let mut guard = s.render.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            let shared = Arc::downgrade(&self.0);
            match s.mpv.create_render_context(move || {
                if let Some(s) = shared.upgrade() {
                    s.dirty.store(true, Ordering::Release);
                    (s.wake)();
                }
            }) {
                Ok(ctx) => *guard = Some(ctx),
                Err(e) => {
                    tracing::warn!("mpv render context'i yaratılamadı: {e}");
                    return;
                }
            }
        }
        let Some(ctx) = guard.as_mut() else { return };
        let ready = ctx.frame_ready();
        if ready || s.force.swap(false, Ordering::AcqRel) {
            // FBO'nun 0. satırı (GL'de alt) dokumuzun v=0'ıdır ve `image` onu üste koyar;
            // mpv'nin olağan (çevirmesiz) çizimi bu yüzden doğru yönde çıkar.
            if let Err(e) = ctx.render(fbo, size.0 as i32, size.1 as i32, false) {
                tracing::warn!("{e}");
            }
        }
    }
}

/// Video oynatıcısı. Bir yüzey başına bir tane.
pub struct VideoPlayer {
    shared: Arc<Shared>,
    target: Option<(Texture, (u32, u32))>,
    info: VideoInfo,
    fit: ImageFit,
    radii: Corners,
    backdrop: WallpaperBackdrop,
    paused: bool,
    visible: bool,
    /// vo=libmpv render context'i olmadan yüklenen dosya video çıkışsız açılır: yükleme,
    /// context ilk GL geçişinde yaratılana dek bekletilir.
    /// Güvenli açılmış, render context'i hazır olunca mpv'ye verilecek dosya.
    pending_load: Option<std::fs::File>,
    /// Renderer'ın GL çöplüğü (ilk çizimde alınır): render context'i GL thread'inde yok edilir.
    graveyard: Option<GlGraveyard>,
}

impl std::fmt::Debug for VideoPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoPlayer").field("info", &self.info).finish_non_exhaustive()
    }
}

/// Render context'i (ve onu yaratan mpv çekirdeği, alan sırasıyla) GL thread'inde, context
/// güncelken düşer: `Shared`'ın bir başvurusu renderer'ın GL çöplüğüne gönderilir. Hiç
/// çizilmediyse render context'i yoktur; burada düşmesi GL çağrısı yapmaz.
impl Drop for VideoPlayer {
    fn drop(&mut self) {
        if let Some(g) = &self.graveyard {
            g.defer(Box::new(self.shared.clone()));
        }
    }
}

impl VideoPlayer {
    /// mpv örneğini başlatır (ses kapalı, döngü, donanım çözme). `wake` mpv kare ya da olay
    /// bildirince herhangi bir thread'den çağrılır; uygulama bir mesajla `request_frame` der.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, VideoError> {
        // Ağ, betik, yapılandırma, günlük ve dış dosyalar her örnekte kapalı (`ffi::HARDENING`).
        let mut mpv = Mpv::new(&[
            ("vo", "libmpv"),
            ("hwdec", "auto-safe"),
            ("loop-file", "inf"),
            ("keep-open", "yes"),
            ("mute", "yes"),
            ("idle", "yes"),
            ("audio-client-name", "nuxut"),
            ("panscan", "1.0"),
            // Duvar kağıdı içe aktarmada hedef boyuta indirildi: pahalı ölçekleyici/dither gereksiz.
            ("profile", "fast"),
            // Yerel dosya: büyük okuma önbelleği bellek israfı.
            ("cache", "no"),
            ("demuxer-max-bytes", "16MiB"),
            ("demuxer-max-back-bytes", "0"),
            // Ses kapalıyken ses akışı hiç çözülmez (CPU); açılınca seçilir.
            ("aid", "no"),
        ])?;
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
        let w = wake.clone();
        mpv.set_wakeup_callback(move || w());
        // Oynatıcı GL çöplüğüyle render thread'inde düşer: render context'i orada (GL gerekir),
        // çekirdeğin uzun kapanışı arka planda.
        mpv.terminate_in_background();
        let shared = Arc::new(Shared {
            render: Mutex::new(None),
            dirty: AtomicBool::new(false),
            force: AtomicBool::new(false),
            wake,
            mpv,
        });
        let player = Self {
            shared,
            target: None,
            info: VideoInfo::default(),
            fit: ImageFit::Cover,
            radii: Corners::ZERO,
            backdrop: WallpaperBackdrop::None,
            paused: false,
            visible: true,
            pending_load: None,
            graveyard: None,
        };
        player.apply_backdrop();
        Ok(player)
    }

    pub fn load(&mut self, path: &Path) -> Result<(), VideoError> {
        let file = crate::source::open_media(path)?;
        self.info = VideoInfo { path: Some(path.to_owned()), ..VideoInfo::default() };
        self.pending_load = Some(file);
        self.flush_load()
    }

    fn flush_load(&mut self) -> Result<(), VideoError> {
        let ready = self.shared.render.lock().map(|g| g.is_some()).unwrap_or(false);
        match self.pending_load.take() {
            Some(f) if ready => self.shared.mpv.load(MediaSource::File(f), true),
            other => {
                self.pending_load = other;
                Ok(())
            }
        }
    }

    /// Ses (varsayılan kapalı). Kapalıyken ses akışı seçilmez: çözme ve ses çıkışı yok.
    pub fn set_muted(&mut self, muted: bool) {
        let m = &self.shared.mpv;
        let _ = m.set_property("aid", if muted { "no" } else { "auto" });
        let _ = m.set_property("mute", if muted { "yes" } else { "no" });
    }

    /// 0…100.
    pub fn set_volume(&mut self, percent: f64) {
        let v = if percent.is_finite() { percent.clamp(0.0, 100.0) } else { 100.0 };
        let _ = self.shared.mpv.set_property("volume", &format!("{v:.0}"));
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.apply_pause();
    }

    /// Yüzey görünmez (ya da tam ekran pencere tarafından örtülü) → duraklat.
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
        self.apply_pause();
    }

    fn apply_pause(&self) {
        let pause = self.paused || !self.visible;
        let _ = self.shared.mpv.set_property("pause", if pause { "yes" } else { "no" });
    }

    pub fn set_fit(&mut self, fit: ImageFit) {
        self.fit = fit;
        let (panscan, keepaspect) = match fit {
            ImageFit::Cover => ("1.0", "yes"),
            ImageFit::Contain => ("0.0", "yes"),
            ImageFit::Fill => ("0.0", "no"),
        };
        let _ = self.shared.mpv.set_property("panscan", panscan);
        let _ = self.shared.mpv.set_property("keepaspect", keepaspect);
        self.shared.force.store(true, Ordering::Release);
    }

    pub fn set_radii(&mut self, radii: Corners) {
        self.radii = radii;
    }

    /// Videonun kaplamadığı alanın (Contain letterbox'ı) zemini; `Wallpaper::set_backdrop` ile
    /// aynı tür. `Color(c)` → `c`; `Blur { fallback, .. }` → `fallback` (videoda bulanık kopya
    /// yok: kare başına bulanıklık gerekirdi); `None` (varsayılan) → saydam, yüzeyin kendi arka
    /// planı görünür. mpv'nin `background-color`'ı olarak uygulanır: mpv kenarı her karede zaten
    /// temizler, ek çizim yok. Değişim geçişsizdir; değişince bir kare isteyin (`request_frame`),
    /// duraklatılmış videoda da son kare yeni zeminle yeniden çizilir.
    pub fn set_backdrop(&mut self, backdrop: WallpaperBackdrop) {
        if self.backdrop != backdrop {
            self.backdrop = backdrop;
            self.apply_backdrop();
            self.shared.force.store(true, Ordering::Release);
        }
    }

    pub fn backdrop(&self) -> WallpaperBackdrop {
        self.backdrop
    }

    /// Zemini mpv'ye verir. mpv ≥ 0.38: `background=color` + `background-color` (+ ≥ 0.39'da
    /// `border-background=color`; varsayılanı zaten `color`, ama `blur` kare başına bulanıklık
    /// olurdu). Daha eski mpv'de `background` rengin kendisidir.
    fn apply_backdrop(&self) {
        let m = &self.shared.mpv;
        let color = mpv_color(self.backdrop.color().unwrap_or(Color::TRANSPARENT));
        let _ = m.set_property("border-background", "color");
        if m.set_property("background", "color").is_ok() && m.set_property("background-color", &color).is_ok() {
            return;
        }
        if let Err(e) = m.set_property("background", &color) {
            tracing::warn!("video zemini uygulanamadı: {e}");
        }
    }

    pub fn info(&self) -> &VideoInfo {
        &self.info
    }

    /// Bekleyen olayları işler (yükleme bitti, hata, boyut).
    fn poll_events(&mut self) {
        loop {
            match self.shared.mpv.wait_event(0.0) {
                Event::None => break,
                Event::FileLoaded | Event::VideoReconfig if self.shared.mpv.single_media() => {
                    let m = &self.shared.mpv;
                    let (w, h) = (m.get_i64("width"), m.get_i64("height"));
                    self.info.size = w.zip(h).map(|(w, h)| (w.max(0) as u32, h.max(0) as u32));
                    self.info.duration = m.get_f64("duration");
                    let tracks = m.get_i64("track-list/count").unwrap_or(0).clamp(0, 64);
                    self.info.has_audio = (0..tracks).any(|i| m.get_string(&format!("track-list/{i}/type")).as_deref() == Some("audio"));
                    self.info.hwdec = m.get_string("hwdec-current").filter(|s| !s.is_empty() && s != "no");
                    self.info.error = None;
                    self.shared.force.store(true, Ordering::Release);
                }
                Event::FileLoaded if !self.shared.mpv.single_media() => {
                    let _ = self.shared.mpv.command(&["stop"]);
                    self.info.error = Some("playlist kabul edilmez".into());
                    tracing::warn!("video reddedildi: playlist");
                }
                Event::EndFile { reason: 4, error } => {
                    let msg = format!("video oynatılamadı (mpv hata {error})");
                    tracing::warn!("{msg}");
                    self.info.error = Some(msg);
                }
                Event::Shutdown => break,
                _ => {}
            }
        }
    }

    /// Videoyu `rect`'e çizer. Yeni kare varsa GL geçişi planlanır; yoksa son kare yeniden
    /// kullanılır (mpv'de iş yok).
    pub fn draw(&mut self, painter: &mut Painter<'_>, rect: Rect, _now: Instant) -> VideoStatus {
        if let Err(e) = self.flush_load() {
            self.info.error = Some(e.to_string());
        }
        self.poll_events();
        let s = painter.scale();
        let px = ((rect.width * s).round().max(1.0) as u32, (rect.height * s).round().max(1.0) as u32);
        let mut status = VideoStatus::default();
        if self.target.as_ref().is_none_or(|(_, size)| *size != px) {
            let t = painter.render_target(px.0, px.1);
            self.target = Some((t, px));
            self.shared.force.store(true, Ordering::Release);
            status.redraw = true;
        }
        let Some((target, _)) = &self.target else { return status };
        let changed = self.shared.dirty.swap(false, Ordering::AcqRel) || self.shared.force.load(Ordering::Acquire);
        if changed || self.shared.render.lock().map(|g| g.is_none()).unwrap_or(true) {
            if self.graveyard.is_none() {
                self.graveyard = Some(painter.gl_graveyard());
            }
            painter.gl_pass(target, Arc::new(MpvPass(self.shared.clone())));
            painter.damage(rect);
        }
        painter.image(rect, target, &ImageStyle::new(ImageFit::Fill).with_radii(self.radii));
        // Bekleyen yükleme: context bu karenin geçişinde yaratılır, sonraki karede yüklenir.
        status.redraw |= self.pending_load.is_some();
        status
    }
}

/// mpv renk dizesi `#AARRGGBB` (düz alpha, 8 bit: mpv'nin kendi duyarlığı). Kesirli `r/g/b/a`
/// biçimi mpv'de 8 bite kesilerek düşer (0x22 → 0x21); onaltılık tam eşlenir. Aralık dışı
/// kırpılır, NaN → 0.
fn mpv_color(c: Color) -> String {
    let v = |x: f32| if x.is_nan() { 0 } else { (x.clamp(0.0, 1.0) * 255.0).round() as u8 };
    format!("#{:02X}{:02X}{:02X}{:02X}", v(c.a), v(c.r), v(c.g), v(c.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mpv_color_format() {
        assert_eq!(mpv_color(Color::from_rgba8(0x11, 0x22, 0x33, 0xff)), "#FF112233");
        assert_eq!(mpv_color(Color::TRANSPARENT), "#00000000");
        assert_eq!(mpv_color(Color::rgba(f32::NAN, 2.0, -1.0, f32::INFINITY)), "#FF00FF00");
    }

    /// Zemin eşlemesi: `None` saydam, `Color` kendisi, `Blur` yedeği.
    #[test]
    fn backdrop_colors() {
        let c = Color::rgb(0.1, 0.2, 0.3);
        assert_eq!(WallpaperBackdrop::None.color(), None);
        assert_eq!(WallpaperBackdrop::Color(c).color(), Some(c));
        assert_eq!(WallpaperBackdrop::Blur { radius: 48.0, fallback: c }.color(), Some(c));
    }

    /// mpv kuruluysa zemin özelliklere yansır (sürüme göre `background-color` ya da `background`).
    #[test]
    fn backdrop_reaches_mpv() {
        if !crate::available() {
            return;
        }
        let mut p = VideoPlayer::new(|| {}).expect("mpv");
        let read = |p: &VideoPlayer| {
            let m = &p.shared.mpv;
            m.get_string("background-color").or_else(|| m.get_string("background")).unwrap_or_default()
        };
        assert_eq!(p.backdrop(), WallpaperBackdrop::None);
        let transparent = read(&p);
        assert!(transparent.starts_with("#00"), "saydam bekleniyordu: {transparent}");
        p.set_backdrop(WallpaperBackdrop::Color(Color::from_rgba8(0x11, 0x22, 0x33, 0xff)));
        assert_eq!(read(&p).to_ascii_uppercase(), "#FF112233");
        p.set_backdrop(WallpaperBackdrop::Blur { radius: 48.0, fallback: Color::from_rgba8(0x44, 0x55, 0x66, 0xff) });
        assert_eq!(read(&p).to_ascii_uppercase(), "#FF445566");
        if p.shared.mpv.get_string("border-background").is_some() {
            assert_eq!(p.shared.mpv.get_string("border-background").as_deref(), Some("color"));
        }
    }
}
