//! Duvar kağıdı bileşeni: durağan görsel; cover/contain/fill; kaynak değişince tema
//! `motion.effects` hareketiyle yumuşak çapraz geçiş.
//!
//! Çözme paylaşılan [`WallpaperDecoder`]'dadır (yalıtılmış işçi, çıkışlar arası önbellek ve
//! türetme; ARCHITECTURE "Duvar kağıdı yalıtımı"); sonuç gelince `notify` çağrılır (uygulama
//! bir mesajla döngüyü uyandırıp `request_frame` der). Boşta iş yok: hiçbir kare/timer
//! istenmez. Animasyonlu kaynakta (GIF/WebP/APNG) ilk kare gösterilir ve
//! [`Wallpaper::animated_source`] `true` döner (ADR-0007 K5; animasyon için `nuxut-video`
//! içe aktarımı).
//!
//! ```ignore
//! // Açılışta bir kez (süreç başına):
//! let decoder = WallpaperDecoder::new(Arc::new(Sandbox::new(SandboxConfig::wallpaper())), WallpaperDecoderOptions::default());
//! // Çıkış başına:
//! let wp = Wallpaper::new(ImageFit::Cover, theme.motion.effects, decoder.clone(), move || { let _ = sender.send(Msg::Wake); });
//! wp.set_source(path, (w_px, h_px));
//! // draw:
//! let st = wp.draw(painter, Rect::from_origin_size(Point::ZERO, painter.size()), now);
//! if st.redraw { cx.request_frame(id) }
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use nuxut_core::{Color, Corners, Rect};
use nuxut_motion::{Animated, Motion};
use nuxut_render::{ImageData, ImageFit, ImageStyle, Painter, RectStyle, Texture, TextureOptions};

use crate::{Fit, ImageError, WallpaperDecoder, WallpaperFrame, WallpaperRequest, WallpaperTicket};

/// Duvar kağıdı kaynağı.
#[derive(Clone, PartialEq, Eq)]
pub enum WallpaperSource {
    File(PathBuf),
    /// Çağıranın verdiği bellek içi güvenilir kare ([`Wallpaper::set_image`]); anahtar çağıranın.
    Image(u64),
}

/// Yol ve anahtar yazılmaz (loglarda kullanıcı içeriği yok).
impl std::fmt::Debug for WallpaperSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::File(_) => f.write_str("File(<yol gizli>)"),
            Self::Image(_) => f.write_str("Image(<anahtar gizli>)"),
        }
    }
}

/// `draw`'ın uygulamaya söyledikleri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WallpaperStatus {
    /// Geçiş sürüyor: bir sonraki vsync'te yeniden çiz (`request_frame`).
    pub redraw: bool,
    /// Duvar kağıdı durağandır (ilk kare): her zaman `None`. Alan, kare zamanlaması isteyen
    /// tüketicilerin (`nuxut-video` duvar kağıdıyla ortak) kalıbı değişmesin diye durur.
    pub next_frame_in: Option<Duration>,
}

/// Görselin kaplamadığı alanın (ör. `Contain` letterbox'ı, saydam pikseller) zemini. Zemin
/// her zaman görselin altına, tüm dikdörtgene boyanır.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum WallpaperBackdrop {
    /// Zemin yok: görselin dışı saydam kalır (yüzeyin kendi arka planı görünür).
    #[default]
    None,
    /// Düz renk (tema token'ı).
    Color(Color),
    /// Aynı görselin `Cover` yerleşimli, `radius` (mantıksal px, ekranda) bulanık kopyası.
    /// Yalnızca `ImageFit::Contain`'de üretilir; diğer yerleşimlerde, kopya hazır olana dek,
    /// görsel yokken ve video duvar kağıdında (`nuxut-video`, bkz. ARCHITECTURE) `fallback`
    /// boyanır. Geçersiz yarıçap (≤ 0, NaN, ∞) → yalnızca `fallback`.
    Blur { radius: f32, fallback: Color },
}

impl WallpaperBackdrop {
    /// Bulanık kopya olmadan boyanan renk: `Color`'ın kendisi ya da `Blur`'un yedeği.
    pub fn color(&self) -> Option<Color> {
        match *self {
            Self::None => None,
            Self::Color(c) | Self::Blur { fallback: c, .. } => Some(c),
        }
    }

    /// Geçerli bulanıklık yarıçapı (mantıksal px).
    fn blur_radius(&self) -> Option<f32> {
        match *self {
            Self::Blur { radius, .. } if radius.is_finite() && radius > 0.0 => Some(radius),
            _ => None,
        }
    }
}

struct Layer {
    texture: Option<Texture>,
    size: (u32, u32),
    /// Yüklenmeyi bekleyen kare (paylaşılan pikseller; yüklenince bırakılır).
    pending: Option<ImageData>,
    /// Dokunun içeriği her değiştiğinde artar (bulanık kopyanın geçerliliği).
    content: u64,
    blur: Option<Blur>,
    /// Zeminin (`WallpaperBackdrop::Blur`) bulanık kopyası; `Cover` ile çizilir.
    backdrop: Option<Blur>,
}

/// Bulanık kopyanın anahtarı: yarıçap, ölçek ya da katmanın içeriği değişince yeniden üretilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlurKey {
    radius: u32,
    scale: u32,
    content: u64,
}

/// Katmanın bulanık kopyası. Yenisi GPU'da hazır olana dek eskisi çizilir.
struct Blur {
    shown: Option<Texture>,
    pending: Option<Texture>,
    key: BlurKey,
}

impl Layer {
    fn from_frame(frame: &WallpaperFrame) -> Self {
        Self { texture: None, size: frame.size(), pending: Some(frame.to_image_data()), content: 0, blur: None, backdrop: None }
    }

    /// Bulanık kopyayı `radius`'a (mantıksal px) uydurur: anahtar değişince bir kez
    /// `Painter::blur_texture` ister, her karede değil. `None` kopyayı bırakır. Gösterilen
    /// kopya değiştiyse `true`.
    fn sync_blur(&mut self, painter: &mut Painter<'_>, radius: Option<f32>) -> bool {
        sync_copy(&mut self.blur, self.texture.as_ref(), painter, radius, self.content)
    }

    /// Zemin kopyasını uydurur. `radius` ekrandaki (mantıksal px) yarıçap; doku `Cover` ile
    /// `rect`'e büyütülerek çizileceği için dokuda bu büyütme oranına bölünür (küçük dokuda
    /// ucuz bulanıklık). `None` kopyayı bırakır. Gösterilen kopya değiştiyse `true`.
    fn sync_backdrop(&mut self, painter: &mut Painter<'_>, rect: Rect, radius: Option<f32>) -> bool {
        let radius = radius.and_then(|r| {
            let (tw, th) = self.texture.as_ref()?.size();
            let s = painter.scale();
            let grow = (rect.width * s / tw.max(1) as f32).max(rect.height * s / th.max(1) as f32);
            let r = r / grow;
            (r.is_finite() && r > 0.0).then_some(r)
        });
        sync_copy(&mut self.backdrop, self.texture.as_ref(), painter, radius, self.content)
    }

    /// Bulanık kopya isteniyor ama henüz hiç hazır değil.
    fn blur_waiting(&self) -> bool {
        self.blur.as_ref().is_some_and(|b| b.pending.is_some())
    }

    /// Zemin kopyası isteniyor ama henüz hazır değil.
    fn backdrop_waiting(&self) -> bool {
        self.backdrop.as_ref().is_some_and(|b| b.pending.is_some())
    }

    /// Zemini `rect`'in tamamına `opacity` ile boyar (görselden önce).
    fn paint_backdrop(&self, painter: &mut Painter<'_>, rect: Rect, backdrop: &WallpaperBackdrop, radii: Corners, opacity: f32) {
        match self.backdrop.as_ref().and_then(|b| b.shown.as_ref()) {
            Some(t) if backdrop.blur_radius().is_some() => {
                painter.image(rect, t, &ImageStyle::new(ImageFit::Cover).with_radii(radii).with_opacity(opacity));
            }
            _ => paint_color(painter, rect, backdrop, radii, opacity),
        }
    }

    /// Katmanı `opacity` ile çizer; `mix` (0 keskin → 1 bulanık) arasında bulanık kopya
    /// keskin dokunun üstüne karışır. Kopya yoksa keskin doku.
    fn paint(&self, painter: &mut Painter<'_>, rect: Rect, style: &ImageStyle, opacity: f32, mix: f32) {
        let Some(sharp) = &self.texture else { return };
        match self.blur.as_ref().and_then(|b| b.shown.as_ref()) {
            Some(blurred) if mix >= 1.0 => painter.image(rect, blurred, &style.with_opacity(opacity)),
            Some(blurred) if mix > 0.0 => {
                painter.image(rect, sharp, &style.with_opacity(opacity));
                painter.image(rect, blurred, &style.with_opacity(opacity * mix));
            }
            _ => painter.image(rect, sharp, &style.with_opacity(opacity)),
        }
    }

    fn blur_shown(&self) -> bool {
        self.blur.as_ref().is_some_and(|b| b.shown.is_some())
    }

    /// GPU'daki dokuların bayt toplamı (RGBA8): keskin doku ve bulanık kopyalar.
    fn texture_bytes(&self) -> u64 {
        let bytes = |t: &Texture| {
            let (w, h) = t.size();
            u64::from(w) * u64::from(h) * 4
        };
        let copies = self.blur.iter().chain(&self.backdrop).flat_map(|b| b.shown.iter().chain(&b.pending));
        self.texture.iter().chain(copies).map(bytes).sum()
    }

    /// Bekleyen kareyi GPU'ya yükler; içerik değiştiyse `true`. Pikseller yüklenince bırakılır
    /// (CPU'da çözülmüş kare tutulmaz).
    fn sync(&mut self, painter: &mut Painter<'_>) -> bool {
        let Some(data) = self.pending.take() else { return false };
        self.content += 1;
        match &self.texture {
            Some(t) => painter.update_texture(t, data),
            None => self.texture = Some(painter.upload_texture(data, TextureOptions::default())),
        }
        true
    }
}

/// `slot`'taki bulanık kopyayı `src`'nin `radius`'lu (mantıksal px) kopyasına uydurur:
/// anahtar (yarıçap, ölçek, içerik) değişince bir kez `Painter::blur_texture` ister; yenisi
/// GPU'da hazır olana dek eskisi gösterilir. `None` kopyayı bırakır. Gösterilen kopya
/// değiştiyse `true`.
fn sync_copy(slot: &mut Option<Blur>, src: Option<&Texture>, painter: &mut Painter<'_>, radius: Option<f32>, content: u64) -> bool {
    let (Some(radius), Some(src)) = (radius, src) else {
        return slot.take().is_some_and(|b| b.shown.is_some());
    };
    let key = BlurKey { radius: radius.to_bits(), scale: painter.scale().to_bits(), content };
    match slot {
        Some(b) if b.key == key => {}
        Some(b) => {
            b.pending = Some(painter.blur_texture(src, radius));
            b.key = key;
        }
        None => *slot = Some(Blur { shown: None, pending: Some(painter.blur_texture(src, radius)), key }),
    }
    let Some(b) = slot else { return false };
    if b.pending.as_ref().is_some_and(Texture::is_ready) {
        b.shown = b.pending.take();
        return true;
    }
    false
}

/// Zeminin düz rengi (`Color` ya da `Blur`'un yedeği); `None`'da hiçbir şey.
fn paint_color(painter: &mut Painter<'_>, rect: Rect, backdrop: &WallpaperBackdrop, radii: Corners, opacity: f32) {
    if let Some(c) = backdrop.color() {
        painter.rect(rect, &RectStyle::filled(c.multiply_alpha(opacity), radii));
    }
}

/// Duvar kağıdı. Bir çıkış (yüzey) başına bir tane.
pub struct Wallpaper {
    fit: ImageFit,
    radii: Corners,
    decoder: Arc<WallpaperDecoder>,
    /// Süren isteğin bileti: yeni kaynak eskisini iptal eder (ara seçimler hiç çözülmez).
    ticket: Option<WallpaperTicket>,
    notify: Arc<dyn Fn() + Send + Sync>,
    results_tx: Sender<(u64, Result<WallpaperFrame, ImageError>)>,
    results_rx: Receiver<(u64, Result<WallpaperFrame, ImageError>)>,
    /// Son istenen yüklemenin kuşağı; eskiler gelirse atılır.
    generation: u64,
    source: Option<WallpaperSource>,
    requested: Option<(WallpaperSource, (u32, u32), ImageFit)>,
    current: Option<Layer>,
    /// Çözülmüş ama dokusu henüz hazır olmayan yeni kaynak.
    incoming: Option<Layer>,
    previous: Option<Layer>,
    /// 0 → 1: yeni katmanın opaklığı.
    fade: Animated<f32>,
    visible: bool,
    /// Bulanıklık yarıçapı (mantıksal px); `None` kapalı.
    blur: Option<f32>,
    /// 0 keskin → 1 bulanık: görünür duvar kağıdında bulanıklık açılıp kapanınca geçiş.
    blur_mix: Animated<f32>,
    backdrop: WallpaperBackdrop,
    /// Zemin değişti: bir sonraki çizim hasar bildirir.
    backdrop_changed: bool,
    last_error: Option<String>,
    /// Gösterilen (ya da gelmekte olan) kaynak animasyonluydu; ilk kare gösteriliyor.
    animated_source: bool,
}

impl std::fmt::Debug for Wallpaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallpaper")
            .field("source", &self.source)
            .field("fit", &self.fit)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Wallpaper {
    /// `motion`: geçiş hareketi (tema `motion.effects`). `decoder`: sürecin paylaşılan duvar
    /// kağıdı çözücüsü (çıkışlar aynı örneği alır; üründe yalıtılmış, örnek/testte
    /// `WallpaperDecoder::in_process_unsandboxed`). `notify`: sonuç gelince çözüm thread'inden
    /// çağrılır.
    pub fn new(fit: ImageFit, motion: impl Into<Motion>, decoder: Arc<WallpaperDecoder>, notify: impl Fn() + Send + Sync + 'static) -> Self {
        let (results_tx, results_rx) = channel();
        let motion = motion.into();
        Self {
            fit,
            radii: Corners::ZERO,
            decoder,
            ticket: None,
            notify: Arc::new(notify),
            results_tx,
            results_rx,
            generation: 0,
            source: None,
            requested: None,
            current: None,
            incoming: None,
            previous: None,
            fade: Animated::new(1.0, motion),
            visible: true,
            blur: None,
            blur_mix: Animated::new(0.0, motion),
            backdrop: WallpaperBackdrop::None,
            backdrop_changed: false,
            last_error: None,
            animated_source: false,
        }
    }

    /// Yeni kaynak; `target_px` çıkışın fiziksel boyutu (`Cover` bu kutuya ortadan kırpılır,
    /// `Contain`/`Fill` sığdırılır; asla büyütülmez). Aynı kaynak ve boyut yeniden istenirse bir
    /// şey yapılmaz. Önceki istek bitmediyse iptal edilir.
    pub fn set_source(&mut self, path: impl AsRef<Path>, target_px: (u32, u32)) {
        let source = WallpaperSource::File(path.as_ref().to_owned());
        let request = (source.clone(), target_px, self.fit);
        if self.requested.as_ref() == Some(&request) {
            return;
        }
        self.requested = Some(request);
        self.generation += 1;
        self.source = Some(source);
        let generation = self.generation;
        let fit = match self.fit {
            ImageFit::Cover => Fit::Cover,
            ImageFit::Contain => Fit::Contain,
            ImageFit::Fill => Fit::Fill,
        };
        let tx = self.results_tx.clone();
        let notify = self.notify.clone();
        // Eski bilet önce düşer: tek bekleyeni buysa iş kuyruktan silinir ya da durdurulur.
        self.ticket = None;
        self.ticket = Some(self.decoder.request(WallpaperRequest::new(path.as_ref(), target_px, fit), move |r| {
            if !matches!(r, Err(ImageError::Cancelled)) && tx.send((generation, r)).is_ok() {
                notify();
            }
        }));
    }

    /// Bellek içi **güvenilir** kaynak: çağıranın kendi ürettiği kare (shell: gömülü SVG
    /// yerleşiğinin `render_svg(svg, cover_crop(w, h))` çizimi). İşçiye, dosyaya ve
    /// `WallpaperDecoder`'a gitmez; kare aynı kare içinde katmana alınır ve doku hazır olunca
    /// mevcut yoldaki gibi çapraz geçişle gelir (ilk görsel geçişsiz). Kullanıcı verisi
    /// (indirilmiş/kullanıcı dosyası) buraya verilmez: o [`set_source`](Self::set_source) ile
    /// yalıtılmış işçiye gider.
    ///
    /// - `key`: kaynağın kimliği (ör. `derive_key(&[b"pair-flight/dark"])`); loglanmaz. Aynı
    ///   anahtar + aynı kare boyutu + aynı yerleşim yeniden verilirse hiçbir şey yapılmaz (çağıran
    ///   her `configure`'da güvenle çağırabilir). Anahtar değişince süren `set_source` isteği
    ///   iptal edilir.
    /// - Boyut: kare olduğu gibi kullanılır, Wallpaper CPU'da yeniden örneklemez; çizimde
    ///   yerleşim (`fit`) GPU'da uygulanır. Çıkışın fiziksel boyutu değişince çağıran yeni
    ///   boyutta yeniden çizip verir. Gerekçe: kaynak vektör — yeni boyutta yeniden çizim keskin
    ///   ve tek geçiştir; elde tutulan rasteri ölçeklemek büyütmede bulanık, küçültmede aynı
    ///   maliyette ikinci bir kopya olurdu. Yeni kare gelene dek eski doku yeni dikdörtgene
    ///   yerleşimle çizilir (boşluk yok).
    /// - Paylaşım: `WallpaperFrame` pikselleri `Arc`'tır; bir kez çiz, `clone()` ile bütün
    ///   çıkışlara ver (kopya yok). Doku yüklenince katman kendi payını bırakır.
    /// - Geçersiz kare (boş ya da tampon boyuta uymuyor) reddedilir: `last_error`, eski görsel
    ///   kalır. Değişince bir kare isteyin (`notify` çağrılmaz: çağıran zaten ana thread'de).
    pub fn set_image(&mut self, key: u64, frame: impl Into<WallpaperFrame>) {
        let frame: WallpaperFrame = frame.into();
        if !frame.is_valid() {
            self.last_error = Some(ImageError::InvalidData("geçersiz duvar kağıdı karesi".into()).to_string());
            return;
        }
        let source = WallpaperSource::Image(key);
        let request = (source.clone(), frame.size(), self.fit);
        if self.requested.as_ref() == Some(&request) {
            return;
        }
        self.requested = Some(request);
        // Süren dosya isteği iptal; kuşak artar, geç gelen eski sonuç `poll`'da atılır.
        self.ticket = None;
        self.generation += 1;
        self.source = Some(source);
        self.last_error = None;
        self.animated_source = false;
        self.incoming = Some(Layer::from_frame(&frame));
    }

    /// Gösterilen kaynak animasyonluydu (GIF/WebP/APNG): ilk kare gösteriliyor. Ayarlar
    /// "animasyon için içe aktar" ipucu verebilir (`nuxut-video`).
    pub fn animated_source(&self) -> bool {
        self.animated_source
    }

    pub fn source(&self) -> Option<&WallpaperSource> {
        self.source.as_ref()
    }

    /// Son yükleme hatası (eski görsel yerinde kalır).
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn set_fit(&mut self, fit: ImageFit) {
        self.fit = fit;
    }

    /// Köşe yarıçapları (önizleme kartları için; tam ekran duvar kağıdında 0).
    pub fn set_radii(&mut self, radii: Corners) {
        self.radii = radii;
    }

    /// Bulanıklık (kilit ekranı): `radius` mantıksal px, `None` (ya da ≤ 0) kapalı. Her
    /// katmanın dokusu hazır olunca bir kez bulanıklaştırılır ve saklanır; yarıçap ya da ölçek
    /// değişince yeniden üretilir. Bulanıkken animasyon o anki karede donar, kapanınca kaldığı
    /// yerden sürer. Görünür duvar kağıdında keskin ↔ bulanık geçişi geçiş hareketiyle çapraz
    /// geçer (kopya hazır olunca başlar; kapatırken kopya geçiş bitince bırakılır). Bulanıklık
    /// açıkken gelen yeni kaynak hiç keskin görünmez. Değişince bir kare isteyin.
    pub fn set_blur(&mut self, radius: Option<f32>) {
        self.blur = radius.filter(|r| r.is_finite() && *r > 0.0);
    }

    pub fn blur(&self) -> Option<f32> {
        self.blur
    }

    /// Görselin kaplamadığı alanın zemini (varsayılan `None`). `Blur` kopyası katman başına,
    /// dokusu hazır olunca bir kez üretilir ve saklanır (anahtar: ekrandaki yarıçap, ölçek,
    /// dikdörtgen/doku oranı, durağan görselde içerik); her karede yeniden hesaplanmaz, boşta
    /// iş yok. Yeni kaynak zemin kopyası hazır olmadan devreye girmez (çapraz geçiş
    /// letterbox'ta yedek renkten başlamaz). Zemin değişimi geçişsizdir; değişince bir kare
    /// isteyin.
    pub fn set_backdrop(&mut self, backdrop: WallpaperBackdrop) {
        if self.backdrop != backdrop {
            self.backdrop = backdrop;
            self.backdrop_changed = true;
        }
    }

    pub fn backdrop(&self) -> WallpaperBackdrop {
        self.backdrop
    }

    /// Katmanların GPU'daki dokularının bayt toplamı (bulanık kopyalar dahil), bellek
    /// bütçesi için.
    pub fn texture_bytes(&self) -> u64 {
        [&self.previous, &self.current, &self.incoming].into_iter().flatten().map(Layer::texture_bytes).sum()
    }

    pub fn set_motion(&mut self, motion: impl Into<Motion>, now: Instant) {
        let motion = motion.into();
        self.fade.set_motion(motion, now);
        self.blur_mix.set_motion(motion, now);
    }

    /// Yüzey görünürlüğü (ör. tam ekran pencere). Durağan duvar kağıdında yalnızca kaydedilir;
    /// görünmezken uygulama zaten çizmez.
    pub fn set_visible(&mut self, visible: bool, _now: Instant) {
        self.visible = visible;
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    /// Görüntülenen içeriğin piksel boyutu.
    pub fn content_size(&self) -> Option<(u32, u32)> {
        self.current.as_ref().map(|l| l.size)
    }

    /// Bekleyen sonuçları alır, gerekirse dokuyu yükler/günceller ve çizer.
    pub fn draw(&mut self, painter: &mut Painter<'_>, rect: Rect, now: Instant) -> WallpaperStatus {
        self.poll();
        let mut changed = std::mem::take(&mut self.backdrop_changed);
        // Zemin kopyası yalnızca letterbox'ta (Contain) gerekir; yoksa bırakılır.
        let backdrop_radius = self.backdrop.blur_radius().filter(|_| self.fit == ImageFit::Contain);
        for layer in [&mut self.previous, &mut self.current, &mut self.incoming].into_iter().flatten() {
            changed |= layer.sync(painter);
            // Kapatılırken kopyalar bulanıktan keskine geçiş bitene dek tutulur (aşağıda).
            if self.blur.is_some() {
                changed |= layer.sync_blur(painter, self.blur);
            }
            changed |= layer.sync_backdrop(painter, rect, backdrop_radius);
        }
        // Yeni katman dokusu (bulanıklık açıksa bulanık kopyası, zemin bulanıksa zemin kopyası
        // da) GPU'ya ulaşınca (kare yükleme bütçesi) devreye girer; o zamana dek eski görsel
        // kalır, çapraz geçiş yarım dokuyla ya da keskin kareyle başlamaz.
        let waiting = match self.incoming.take() {
            Some(layer)
                if layer.texture.as_ref().is_some_and(Texture::is_ready) && !layer.blur_waiting() && !layer.backdrop_waiting() =>
            {
                match self.current.replace(layer) {
                    Some(old) => {
                        self.previous = Some(old);
                        self.fade.jump_to(0.0);
                        self.fade.set_target(1.0, now);
                    }
                    None => self.fade.jump_to(1.0),
                }
                // Bulanıklık açıkken gelen görsel (bulanık kopyası hazır) doğrudan bulanık görünür.
                if self.blur.is_some() {
                    self.blur_mix.jump_to(1.0);
                }
                changed = true;
                false
            }
            Some(layer) => {
                self.incoming = Some(layer);
                true
            }
            None => false,
        };
        // Keskin ↔ bulanık: açılınca kopya hazır olduğunda geçiş başlar, kapatınca hemen.
        match self.blur {
            Some(_) if self.current.as_ref().is_some_and(Layer::blur_shown) => self.blur_mix.set_target(1.0, now),
            Some(_) => {}
            None => self.blur_mix.set_target(0.0, now),
        }
        let mix = self.blur_mix.value(now).clamp(0.0, 1.0);
        let mixing = !self.blur_mix.is_settled(now);
        if self.blur.is_none() && !mixing {
            for layer in [&mut self.previous, &mut self.current, &mut self.incoming].into_iter().flatten() {
                layer.sync_blur(painter, None);
            }
        }
        let t = self.fade.value(now).clamp(0.0, 1.0);
        let fading = !self.fade.is_settled(now);
        if !fading {
            self.previous = None;
        }
        let style = ImageStyle::new(self.fit).with_radii(self.radii);
        // Zemin her katmanın altına, katmanın opaklığıyla; katman yokken düz renk.
        if self.previous.is_none() && self.current.is_none() {
            paint_color(painter, rect, &self.backdrop, self.radii, 1.0);
        }
        if let Some(prev) = &self.previous {
            prev.paint_backdrop(painter, rect, &self.backdrop, self.radii, 1.0);
            prev.paint(painter, rect, &style, 1.0, mix);
        }
        if let Some(cur) = &self.current {
            let opacity = if self.previous.is_some() { t } else { 1.0 };
            cur.paint_backdrop(painter, rect, &self.backdrop, self.radii, opacity);
            cur.paint(painter, rect, &style, opacity, mix);
        }
        if changed || fading || mixing {
            painter.damage(rect);
        }
        // Bulanık kopya GPU'da hazır olunca gösterilir (bir sonraki kare).
        let blurring = [&self.current, &self.previous].into_iter().flatten().any(|l| l.blur_waiting() || l.backdrop_waiting());
        let redraw = fading || waiting || blurring || mixing;
        WallpaperStatus { redraw, next_frame_in: None }
    }

    fn poll(&mut self) {
        while let Ok((generation, result)) = self.results_rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            match result {
                Ok(frame) => {
                    self.last_error = None;
                    self.animated_source = frame.animated_source;
                    // İlk görsel geçişsiz; sonrakiler doku hazır olunca çapraz geçişle. Kare
                    // GPU'ya yüklenince bu katmandaki paylaşılan pikseller bırakılır.
                    self.incoming = Some(Layer::from_frame(&frame));
                }
                Err(e) => {
                    tracing::debug!("duvar kağıdı yüklenemedi: {e}");
                    self.last_error = Some(e.to_string());
                }
            }
        }
    }
}
