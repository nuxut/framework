//! Çizim bağlamı ([`Ui`]), hasar bildirimi ([`DamageState`]), bileşen yanıtı ([`Response`])
//! ve kaplama girdi bölgeleri ([`OverlayRegions`]).

use std::time::Instant;

use nuxut_core::{Color, Point, Rect};
use nuxut_render::{Painter, TextSystem};
use nuxut_theme::Theme;

/// Bir karede bileşenlerin ortak çizim bağlamı: painter, tema, kare saati.
///
/// - **Kare isteği:** canlanan bileşen [`request_frame`](Self::request_frame) der; uygulama
///   karenin sonunda [`wants_frame`](Self::wants_frame) `true` ise `cx.request_frame` çağırır.
///   Böylece bileşen başına `is_animating` yoklaması gerekmez; hiçbir şey canlanmıyorsa yüzey
///   uyur (boşta sıfır iş).
/// - **Uyanma:** ileride bir anda değişecek ama o ana kadar durağan bileşen (ipucu gecikmesi)
///   kare istemez, [`wake_at`](Self::wake_at) der; uygulama [`wake_deadline`](Self::wake_deadline)
///   için tek bir zamanlayıcı kurar. Bekleme süresince kare çizilmez.
/// - **Hasar:** [`with_damage_tracking`](Self::with_damage_tracking) açıksa bileşenler
///   değişen bölgelerini (eski ∪ yeni) `painter.damage` ile bildirir. Kapalıyken (varsayılan)
///   hiçbir şey bildirilmez ve tüm yüzey çizilir — kendi içeriğinin hasarını takip etmeyen
///   uygulama yanlışlıkla kısmi çizime düşmez.
/// - **Opaklık:** [`alpha`](Self::alpha) iç içe solmalar için çarpandır (menü sayfa geçişi,
///   bildirim kaydırma); bileşenler tüm renklerini [`fade`](Self::fade) ile geçirir.
/// - **Kaplama (overlay):** [`overlay`](Self::overlay) içindeki çizimler kare sonunda her şeyin
///   üstüne, kapsayıcıların (kaydırma alanı, kart, sayfa yığını) kırpmasından bağımsız çizilir.
///   İpucu ve açılır menüler tetikleyicinin hemen yanında, kırpılı kapsayıcının içinden
///   çağrılabilir; koordinatlar zaten yüzey koordinatıdır, sığdırma alanı
///   [`surface_rect`](Self::surface_rect). Kaplamanın girdi bölgesi
///   [`overlay_input`](Self::overlay_input) ile bildirilir; uygulama kare sonunda
///   [`take_overlay_regions`](Self::take_overlay_regions) ile saklar ve sonraki işaretçi
///   olaylarını önce kaplamanın sahibine verir ([`OverlayRegions`]).
#[derive(Debug)]
pub struct Ui<'p, 'a> {
    pub painter: &'p mut Painter<'a>,
    pub theme: &'p Theme,
    pub now: Instant,
    track_damage: bool,
    wants_frame: bool,
    wake_at: Option<Instant>,
    alpha: f32,
    overlay_regions: OverlayRegions,
}

impl<'p, 'a> Ui<'p, 'a> {
    pub fn new(painter: &'p mut Painter<'a>, theme: &'p Theme, now: Instant) -> Self {
        Self {
            painter,
            theme,
            now,
            track_damage: false,
            wants_frame: false,
            wake_at: None,
            alpha: 1.0,
            overlay_regions: OverlayRegions::default(),
        }
    }

    pub fn with_damage_tracking(mut self, on: bool) -> Self {
        self.track_damage = on;
        self
    }

    pub fn tracks_damage(&self) -> bool {
        self.track_damage
    }

    /// Hasar takibi açıksa bölgeyi bildirir.
    pub fn damage(&mut self, rect: Rect) {
        if self.track_damage && !rect.is_empty() {
            self.painter.damage(rect);
        }
    }

    pub fn request_frame(&mut self) {
        self.wants_frame = true;
    }

    /// Bu karede bir bileşen sonraki kareyi istedi mi?
    pub fn wants_frame(&self) -> bool {
        self.wants_frame
    }

    /// Bileşen `at` anında bir kare ister ama o ana kadar hiçbir şey değişmez (ipucu
    /// gecikmesi): kare istenmez, uygulama [`wake_deadline`](Self::wake_deadline)'a
    /// zamanlayıcı kurar. Birden çok çağrıda en erkeni kalır.
    pub fn wake_at(&mut self, at: Instant) {
        self.wake_at = Some(self.wake_at.map_or(at, |w| w.min(at)));
    }

    /// Bu karede bildirilen en erken uyanma anı. Uygulama karenin sonunda
    /// `cx.set_timer(at.saturating_duration_since(now))` kurar (öncekini iptal ederek);
    /// zamanlayıcıda `cx.request_frame(surface)` der.
    pub fn wake_deadline(&self) -> Option<Instant> {
        self.wake_at
    }

    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// `f` süresince opaklık çarpanı `alpha` ile çarpılır.
    pub fn with_alpha<R>(&mut self, alpha: f32, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.alpha;
        self.alpha = (saved * alpha).clamp(0.0, 1.0);
        let r = f(self);
        self.alpha = saved;
        r
    }

    /// Rengi geçerli opaklık çarpanıyla soldurur.
    pub fn fade(&self, color: Color) -> Color {
        color.multiply_alpha(self.alpha)
    }

    pub fn text(&mut self) -> &mut TextSystem {
        self.painter.text_system()
    }

    /// Yüzeyin tamamı (mantıksal, yüzey koordinatı): kaplamaların sığdırma alanı.
    pub fn surface_rect(&self) -> Rect {
        let s = self.painter.size();
        Rect::new(0.0, 0.0, s.width, s.height)
    }

    /// `f`'in çizimleri kaplama katmanına gider: kare sonunda en üstte, temel katmanın
    /// kırpmalarından bağımsız. Opaklık çarpanı ([`alpha`](Self::alpha)) korunur (solan bir
    /// sayfanın menüsü de söner). İç içe çağrı tek katmandır.
    pub fn overlay<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let opened = self.painter.begin_overlay();
        let r = f(self);
        if opened {
            self.painter.end_overlay();
        }
        r
    }

    /// Bu karede bir kaplamanın girdi alacağı bölge (açık menü, popover). İşaretçi bu bölgede
    /// olduğunda uygulama olayı altındaki içerikten önce kaplamanın sahibine vermeli.
    pub fn overlay_input(&mut self, rect: Rect) {
        self.overlay_regions.push(rect);
    }

    /// Bu karede bildirilen kaplama girdi bölgeleri.
    pub fn overlay_regions(&self) -> &OverlayRegions {
        &self.overlay_regions
    }

    /// Kare sonunda: bölgeleri uygulamaya devreder (sonraki olayların yönlendirmesi için).
    pub fn take_overlay_regions(&mut self) -> OverlayRegions {
        std::mem::take(&mut self.overlay_regions)
    }
}

/// Son karede kaplamaların girdi bölgeleri (yüzey koordinatı). Uygulama yüzey başına tutar,
/// her kare [`Ui::take_overlay_regions`] ile yeniler; işaretçi olayında
/// [`contains`](Self::contains) ise olayı önce kaplamanın sahibine verir (kaydırma alanı ya
/// da altındaki satır görmez).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OverlayRegions {
    rects: Vec<Rect>,
}

impl OverlayRegions {
    pub fn push(&mut self, rect: Rect) {
        if !rect.is_empty() {
            self.rects.push(rect);
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        self.rects.iter().any(|r| r.contains(p))
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn rects(&self) -> &[Rect] {
        &self.rects
    }
}

/// Bileşenin hasar durumu: son çizildiği bölge ve "görünüş değişti" bayrağı.
///
/// Durum değişikliğinde [`mark`](Self::mark), çizimde [`report`](Self::report) çağrılır;
/// değişiklik ya da animasyon varsa eski ∪ yeni bölge bildirilir.
#[derive(Debug, Clone, Default)]
pub struct DamageState {
    last: Option<Rect>,
    dirty: bool,
}

impl DamageState {
    pub fn new() -> Self {
        Self { last: None, dirty: true }
    }

    pub fn mark(&mut self) {
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// `current`: bu karede kaplanan bölge (gölge/taşma dahil). `animating`: değer hâlâ
    /// değişiyor (bu kare de hasarlı).
    pub fn report(&mut self, ui: &mut Ui, current: Rect, animating: bool) {
        if self.dirty || animating || self.last != Some(current) {
            let region = match self.last {
                Some(last) if !last.is_empty() => last.union(&current),
                _ => current,
            };
            ui.damage(region);
        }
        self.last = Some(current);
        self.dirty = false;
    }

    /// Bileşen artık çizilmiyor: son bölgesi silinsin diye bildirilir.
    pub fn report_hidden(&mut self, ui: &mut Ui) {
        if let Some(last) = self.last.take() {
            ui.damage(last);
        }
        self.dirty = false;
    }
}

/// Girdiye bileşen yanıtı: yeniden çizim gerekir mi, bir olay üretti mi.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct Response<T = ()> {
    /// Görünüş değişti; uygulama kare istemeli.
    pub repaint: bool,
    pub event: Option<T>,
}

impl<T> Default for Response<T> {
    fn default() -> Self {
        Self { repaint: false, event: None }
    }
}

impl<T> Response<T> {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn repaint() -> Self {
        Self { repaint: true, event: None }
    }

    pub fn event(event: T) -> Self {
        Self { repaint: true, event: Some(event) }
    }

    pub fn with_repaint(mut self, repaint: bool) -> Self {
        self.repaint |= repaint;
        self
    }

    /// Olay tipini dönüştürür.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Response<U> {
        Response { repaint: self.repaint, event: self.event.map(f) }
    }

    /// Olayı atar, yalnızca yeniden çizim bilgisini taşır.
    pub fn discard<U>(self) -> Response<U> {
        Response { repaint: self.repaint, event: None }
    }
}
