//! Yüzey başına Wayland durumu ve kare zamanlaması.
//!
//! Kare akışı (boşta sıfır iş):
//! - `wants_frame`: uygulama (veya configure/ölçek değişimi) yeni kare istedi.
//! - `callback_pending`: son commit'le istenen `wl_surface.frame` henüz gelmedi.
//! - Her olay turunun sonunda `configured && wants_frame && !callback_pending` olan yüzeyler
//!   çizilir. Çizimden hemen önce yeni frame callback istenir; uygulama `draw` içinde
//!   `request_frame` çağırmazsa callback geldiğinde hiçbir şey olmaz ve yüzey uyur.

use nuxut_core::{Insets, Size};
use nuxut_render::DamageHistory;
use smithay_client_toolkit::session_lock::SessionLockSurface;
use smithay_client_toolkit::shell::xdg::window::Window;
use smithay_client_toolkit::shell::{WaylandSurface, wlr_layer::LayerSurface};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols::wp::{
    fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1,
    viewporter::client::wp_viewport::WpViewport,
};

use crate::egl::GpuSurface;
use crate::types::{Anchor, OutputId, OutputTarget, WindowState};

#[derive(Debug)]
pub(crate) struct SurfaceEntry {
    /// Layer/kilit yüzeyinin çıkışı; pencerelerde `None`. `follows_enter` yüzeyde ilk
    /// `wl_surface.enter`e dek `None`.
    pub output: Option<OutputId>,
    /// Çıkışı compositor seçti (`OutputTarget::Compositor` ya da bilinmeyen çıkış):
    /// `output` `wl_surface.enter`/`leave` ile izlenir.
    pub follows_enter: bool,
    /// İstenen mantıksal boyut (0 = compositor belirler).
    pub requested: (u32, u32),
    /// Son configure'daki mantıksal boyut.
    pub size: (u32, u32),
    /// Etkin ölçek (kesirli veya tamsayı).
    pub scale: f32,
    pub configured: bool,
    pub wants_frame: bool,
    pub callback_pending: bool,
    /// En az bir tampon gönderildi. Öncesinde tamponsuz commit yapılmaz (kilit yüzeyinde
    /// protokol hatası; diğerlerinde gereksiz): çift tamponlu durum ilk swap'la uygulanır.
    pub has_buffer: bool,
    /// İlk sunum takibi (`App::surface_presented`).
    pub presentation: Presentation,
    /// Viewport hedefi / buffer scale bir sonraki commit'te yeniden gönderilmeli.
    pub scale_dirty: bool,
    /// Kesirli ölçek yolu: ikisi birlikte varsa buffer scale 1, viewport hedefi mantıksal.
    pub fractional: Option<(WpFractionalScaleV1, WpViewport)>,
    /// Layer yüzeyinin yerleşimi (gönderilen + bekleyen); pencere ve kilit yüzeyinde `None`.
    pub placement: Option<PlacementState>,
    /// Buffer age ile kısmi yeniden çizim için hasar geçmişi.
    pub damage: DamageHistory,
    /// `Egl::destroy_surface` ile, `role`'den önce yok edilir.
    pub gpu: Option<GpuSurface>,
    /// Yazılım sunumunda (`shm` modülü) EGL yüzeyi yerine; `ShmSurface::destroy` ile, `role`'den önce.
    pub shm: Option<crate::shm::ShmSurface>,
    pub role: Role,
}

/// Yüzeyin ilk sunumu: ilk tamponu taşıyan commit'e bağlanan frame callback'in `done`'u.
///
/// `wl_surface.frame` her çizimde swap'tan önce istenir ve swap'ın commit'ine bağlanır; ilk
/// tampondan önce hiç commit yapılmadığından (bkz. `SurfaceEntry::has_buffer`) ilk tamponlu
/// commit'ten sonra gelen ilk `done` o commit'in (ya da swap'ı başarısız olup ona katılan daha
/// önceki isteğin) callback'idir: compositor içeriği ekrana koydu ya da koymaya hazır.
/// Yüzey başına bir kez; yeniden yaratılan yüzey yeni bir girdidir ve yeniden başlar.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Presentation {
    /// Henüz tampon gönderilmedi.
    #[default]
    Unpresented,
    /// İlk tampon commit edildi; frame callback bekleniyor.
    Committed,
    /// `done` geldi, uygulamaya bildirildi.
    Presented,
}

impl Presentation {
    /// Gerçek bir tampon gönderildi (tamponsuz commit sayılmaz).
    pub fn buffer_committed(&mut self) {
        if *self == Self::Unpresented {
            *self = Self::Committed;
        }
    }

    /// Frame callback `done` geldi. `true`: bu ilk sunum, uygulamaya bildirilmeli.
    pub fn frame_done(&mut self) -> bool {
        if *self == Self::Committed {
            *self = Self::Presented;
            true
        } else {
            false
        }
    }
}

/// Yüzeyin rolü. Alan sırasıyla en son düşer: önce rol nesneleri, sonra `wl_surface`.
#[derive(Debug)]
pub(crate) enum Role {
    Layer(LayerSurface),
    Window {
        window: Window,
        state: WindowState,
        /// Son gönderilen boyut ipuçları (mantıksal; 0 = o eksende sınır yok).
        hints: SizeHints,
    },
    /// Oturum kilidi yüzeyi (ext-session-lock); çıkışı `SurfaceEntry::output`.
    Lock(SessionLockSurface),
}

impl Role {
    pub fn wl_surface(&self) -> &WlSurface {
        match self {
            Self::Layer(layer) => layer.wl_surface(),
            Self::Window { window, .. } => window.wl_surface(),
            Self::Lock(lock) => lock.wl_surface(),
        }
    }
}

impl SurfaceEntry {
    pub fn ready_to_draw(&self) -> bool {
        self.configured && self.wants_frame && !self.callback_pending
    }

    pub fn logical_size(&self) -> Size {
        Size::new(self.size.0 as f32, self.size.1 as f32)
    }

    /// Tampon boyutu (fiziksel piksel).
    pub fn physical_size(&self) -> (u32, u32) {
        (
            physical(self.size.0, self.scale),
            physical(self.size.1, self.scale),
        )
    }

    /// Ölçekle ilgili çift tamponlu durumu gönderir; bir sonraki commit'te uygulanır.
    pub fn apply_scale(&mut self) {
        if !self.scale_dirty {
            return;
        }
        self.scale_dirty = false;
        let surface = self.role.wl_surface();
        match &self.fractional {
            Some((_, viewport)) => {
                surface.set_buffer_scale(1);
                viewport.set_destination(self.size.0.max(1) as i32, self.size.1.max(1) as i32);
            }
            None => surface.set_buffer_scale((self.scale.round() as i32).max(1)),
        }
    }

    /// Bekleyen layer yerleşimini gönderir; bir sonraki commit'te uygulanır. Gönderilen
    /// yerleşimi döner (yoksa `None`). Boyut değiştiyse compositor yeni configure gönderir.
    pub fn apply_placement(&mut self) -> Option<LayerPlacement> {
        let p = self.placement.as_mut()?.take()?;
        if let Role::Layer(layer) = &self.role {
            layer.set_anchor(smithay_client_toolkit::shell::wlr_layer::Anchor::from_bits_truncate(
                u32::from(p.anchor.bits()),
            ));
            let [top, right, bottom, left] = p.margin;
            layer.set_margin(top, right, bottom, left);
            layer.set_size(p.size.0, p.size.1);
        }
        self.requested = p.size;
        Some(p)
    }
}

/// Layer yüzeyinin compositor'a giden yerleşimi: anchor, kablodaki margin (üst, sağ, alt,
/// sol; `i32`) ve istenen mantıksal boyut. Eşitlik kablo değerleriyle: aynı tamsayıya düşen
/// margin istek üretmez.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayerPlacement {
    pub anchor: Anchor,
    pub margin: [i32; 4],
    pub size: (u32, u32),
}

impl LayerPlacement {
    pub fn new(anchor: Anchor, margin: Insets, size: (u32, u32)) -> Self {
        let m = margin;
        Self {
            anchor,
            margin: [m.top as i32, m.right as i32, m.bottom as i32, m.left as i32],
            size,
        }
    }

    #[cfg(any(test, feature = "a11y"))]
    pub fn insets(&self) -> Insets {
        let [top, right, bottom, left] = self.margin;
        Insets::new(top as f32, right as f32, bottom as f32, left as f32)
    }
}

/// Gönderilen ve bekleyen yerleşim. `request` bekleyeni değiştirir (son değer kazanır), çizim
/// yolu `take` ile bir sonraki karenin commit'inde gönderir: sürükleme sırasında kare başına
/// en çok bir yerleşim, frame callback'e hizalı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlacementState {
    sent: LayerPlacement,
    pending: Option<LayerPlacement>,
}

impl PlacementState {
    pub fn new(sent: LayerPlacement) -> Self {
        Self { sent, pending: None }
    }

    /// Yeni istenen yerleşim. `true`: bekleyen değişti (gönderilecek bir şey var ya da az önce
    /// bekleyen iptal oldu). Gönderilenle aynıysa bekleyen düşer, istek gitmez.
    pub fn request(&mut self, p: LayerPlacement) -> bool {
        let want = (p != self.sent).then_some(p);
        std::mem::replace(&mut self.pending, want) != want
    }

    /// Bekleyen varsa gönderilmiş sayar ve döner.
    pub fn take(&mut self) -> Option<LayerPlacement> {
        let p = self.pending.take()?;
        self.sent = p;
        Some(p)
    }

    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Bekleyen, yoksa gönderilen (boyut verilmeyen istek bunu korur).
    pub fn current(&self) -> LayerPlacement {
        self.pending.unwrap_or(self.sent)
    }
}

impl Drop for SurfaceEntry {
    fn drop(&mut self) {
        debug_assert!(
            self.gpu.is_none(),
            "GpuSurface Egl::destroy_surface ile yok edilmeli"
        );
        if let Some((fractional, viewport)) = self.fractional.take() {
            fractional.destroy();
            viewport.destroy();
        }
        // `role` alan sırasıyla en son düşer: önce rol nesneleri, sonra wl_surface.
    }
}

/// `xdg_toplevel` en küçük/en büyük boyut ipuçları (mantıksal). 0 = o eksende sınır yok.
///
/// Protokol, bir eksende `max > 0 && max < min` ya da negatif değeri `invalid_size` protokol
/// hatasıyla cezalandırır (tüm bağlantı düşer). İstekler i32 taşır; `u32` değerler
/// `i32::MAX`'e kırpılır. Çakışmada son çağrılan kazanır: yeni `max` küçükse `min` ona
/// iner, yeni `min` büyükse `max` ona çıkar (uyarı loglanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SizeHints {
    pub min: (u32, u32),
    pub max: (u32, u32),
}

impl SizeHints {
    /// `WindowSpec`'ten; çakışmada `max` kazanır (pencere en çok bu kadar olsun).
    pub fn new(min: Option<(u32, u32)>, max: Option<(u32, u32)>) -> Self {
        let mut hints = Self::default();
        hints.set_min(min);
        hints.set_max(max);
        hints
    }

    /// `true`: diğer ipucu çakışma yüzünden değişti.
    pub fn set_min(&mut self, min: Option<(u32, u32)>) -> bool {
        self.min = clamp_pair(min);
        let before = self.max;
        self.max = (
            raise_max(self.max.0, self.min.0),
            raise_max(self.max.1, self.min.1),
        );
        before != self.max
    }

    /// `true`: diğer ipucu çakışma yüzünden değişti.
    pub fn set_max(&mut self, max: Option<(u32, u32)>) -> bool {
        self.max = clamp_pair(max);
        let before = self.min;
        self.min = (
            lower_min(self.min.0, self.max.0),
            lower_min(self.min.1, self.max.1),
        );
        before != self.min
    }

    /// İstek değerleri (`xdg_toplevel.set_min_size`/`set_max_size`).
    pub fn wire(&self) -> ((i32, i32), (i32, i32)) {
        let w = |(a, b): (u32, u32)| (a as i32, b as i32);
        (w(self.min), w(self.max))
    }
}

fn clamp_pair(size: Option<(u32, u32)>) -> (u32, u32) {
    let c = |v: u32| v.min(i32::MAX as u32);
    size.map_or((0, 0), |(w, h)| (c(w), c(h)))
}

/// Sınırlı `max` `min`'in altındaysa `min`'e çıkar.
fn raise_max(max: u32, min: u32) -> u32 {
    if max > 0 && max < min { min } else { max }
}

/// Sınırlı `max` `min`'in altındaysa `min` ona iner.
fn lower_min(min: u32, max: u32) -> u32 {
    if max > 0 && min > max { max } else { min }
}

/// Bir layer yüzeyinin yaratılıştaki çıkış yerleşimi.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placement {
    /// `get_layer_surface`'e verilen çıkış; `None` = null, çıkışı compositor seçer.
    pub wire: Option<OutputId>,
    /// `SurfaceEntry::output` `wl_surface.enter` ile izlenir.
    pub follows_enter: bool,
    /// İlk configure'dan önceki ölçek tahmini.
    pub scale: f32,
}

/// `target`'ı bilinen çıkışlara göre çözer. `scale_of`: bilinen çıkışın tamsayı ölçeği;
/// `known_scales`: tüm bilinen çıkışların ölçekleri.
///
/// Compositor'ın seçeceği çıkışın ölçeği önceden bilinmez. Tahmin en büyük çıkış ölçeğidir:
/// tek çıkışta (ya da ölçekler eşitken) tamdır; değilse ilk kare gerekenden keskin çizilir ve
/// compositor küçültür (bulanık 1x kare görünmez). Hyprland ve sway kesirli/tercih edilen
/// ölçeği yüzey yaratılınca, ilk configure'dan önce gönderir; tahmin çoğunlukla hiç çizime
/// ulaşmaz. Geç gelirse `set_scale` yeniden çizer.
pub(crate) fn place_layer(
    target: OutputTarget,
    scale_of: impl Fn(OutputId) -> Option<i32>,
    known_scales: impl IntoIterator<Item = i32>,
) -> Placement {
    if let OutputTarget::Output(id) = target
        && let Some(scale) = scale_of(id)
    {
        return Placement {
            wire: Some(id),
            follows_enter: false,
            scale: scale.max(1) as f32,
        };
    }
    let scale = known_scales.into_iter().max().unwrap_or(1).max(1) as f32;
    Placement {
        wire: None,
        follows_enter: true,
        scale,
    }
}

/// `wl_surface.enter`: izlenen yüzeyin çıkışı `entered` olur. `true`: değişti.
pub(crate) fn track_enter(
    output: &mut Option<OutputId>,
    follows_enter: bool,
    entered: OutputId,
) -> bool {
    if !follows_enter || *output == Some(entered) {
        return false;
    }
    *output = Some(entered);
    true
}

/// `wl_surface.leave`: izlenen yüzey bulunduğu çıkıştan ayrıldıysa çıkışı `None` olur.
/// `true`: değişti.
pub(crate) fn track_leave(
    output: &mut Option<OutputId>,
    follows_enter: bool,
    left: OutputId,
) -> bool {
    if !follows_enter || *output != Some(left) {
        return false;
    }
    *output = None;
    true
}

/// Mantıksal → fiziksel. Kesirli ölçekte yuvarlama protokolün önerdiğidir
/// (`round(size * scale)`), compositor da aynı hesabı yapar.
pub(crate) fn physical(logical: u32, scale: f32) -> u32 {
    ((logical as f32 * scale).round() as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::{
        LayerPlacement, PlacementState, Presentation, SizeHints, physical, place_layer, track_enter,
        track_leave,
    };
    use crate::types::{Anchor, OutputId, OutputTarget};
    use nuxut_core::Insets;

    #[test]
    fn placement_coalesces_to_the_latest_and_skips_unchanged() {
        let at = |x: f32, y: f32| LayerPlacement::new(Anchor::TOP | Anchor::LEFT, Insets::new(y, 0.0, 0.0, x), (400, 200));
        let mut p = PlacementState::new(at(10.0, 10.0));
        // Gönderilenle aynı: istek yok.
        assert!(!p.request(at(10.0, 10.0)));
        assert!(!p.has_pending());
        // Sürükleme: kare içinde üç istek, yalnız sonuncusu gider.
        assert!(p.request(at(20.0, 15.0)));
        assert!(p.request(at(30.0, 20.0)));
        assert!(p.request(at(40.0, 25.0)));
        assert_eq!(p.take(), Some(at(40.0, 25.0)));
        assert_eq!(p.take(), None, "bir kez gönderilir");
        // Aynı tamsayıya düşen margin kablo değeri değiştirmez.
        assert!(!p.request(at(40.4, 25.2)));
        // İleri gidip gönderilene dönmek bekleyeni iptal eder.
        assert!(p.request(at(50.0, 25.0)));
        assert!(p.request(at(40.0, 25.0)));
        assert!(!p.has_pending());
        assert_eq!(p.current(), at(40.0, 25.0));
        // Boyut ve anchor da yerleşimin parçası.
        let wide = LayerPlacement::new(Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, Insets::ZERO, (0, 260));
        assert!(p.request(wide));
        assert_eq!(p.current(), wide);
        assert_eq!(p.take().map(|w| w.insets()), Some(Insets::ZERO));
    }

    /// Geçerli ipucu: her eksende max ya 0 (sınırsız) ya da ≥ min; değerler i32'ye sığar.
    fn valid(h: SizeHints) -> bool {
        let ((min_w, min_h), (max_w, max_h)) = h.wire();
        let axis = |min: i32, max: i32| min >= 0 && max >= 0 && (max == 0 || max >= min);
        axis(min_w, max_w) && axis(min_h, max_h)
    }

    #[test]
    fn size_hints_fixed_welcome_size() {
        let h = SizeHints::new(Some((720, 640)), Some((720, 640)));
        assert_eq!(h.wire(), ((720, 640), (720, 640)));
        assert!(valid(h));
    }

    #[test]
    fn size_hints_spec_conflict_max_wins() {
        // min > max protokol hatası olurdu: max kazanır.
        let h = SizeHints::new(Some((800, 300)), Some((640, 480)));
        assert_eq!(h.min, (640, 300));
        assert_eq!(h.max, (640, 480));
        assert!(valid(h));
    }

    #[test]
    fn size_hints_last_call_wins() {
        let mut h = SizeHints::new(Some((320, 200)), None);
        // Sınırsız max'a dokunulmaz.
        assert!(!h.set_max(Some((0, 500))));
        assert_eq!(h.max, (0, 500));
        // Yeni min max'ı aşıyor: max ona çıkar.
        assert!(h.set_min(Some((720, 640))));
        assert_eq!(h.max, (0, 640));
        assert!(valid(h));
        // Yeni max min'in altında: min ona iner.
        assert!(h.set_max(Some((400, 400))));
        assert_eq!(h.min, (400, 400));
        assert!(valid(h));
        // None = sınır kalkar, diğeri değişmez.
        assert!(!h.set_max(None));
        assert_eq!(
            h,
            SizeHints {
                min: (400, 400),
                max: (0, 0)
            }
        );
    }

    #[test]
    fn size_hints_clamp_to_i32() {
        let h = SizeHints::new(Some((u32::MAX, 10)), Some((u32::MAX, u32::MAX)));
        assert_eq!(h.wire(), ((i32::MAX, 10), (i32::MAX, i32::MAX)));
        assert!(valid(h));
    }

    #[test]
    fn size_hints_exhaustive_small_values_always_valid() {
        let vals = [None, Some(0), Some(1), Some(5), Some(9)];
        for a in vals {
            for b in vals {
                let pair = |v: Option<u32>| v.map(|v| (v, 9 - v.min(9)));
                let mut h = SizeHints::new(pair(a), pair(b));
                assert!(valid(h), "{a:?} {b:?}");
                h.set_min(pair(b));
                assert!(valid(h));
                h.set_max(pair(a));
                assert!(valid(h));
            }
        }
    }

    #[test]
    fn presentation_fires_once_after_first_buffer() {
        let mut p = Presentation::default();
        // Tampondan önce gelen done (olmamalı ama) bildirilmez.
        assert!(!p.frame_done());
        p.buffer_committed();
        assert_eq!(p, Presentation::Committed);
        assert!(p.frame_done());
        assert_eq!(p, Presentation::Presented);
        // Sonraki kareler ve callback'ler yeniden bildirmez.
        p.buffer_committed();
        assert!(!p.frame_done());
        assert_eq!(p, Presentation::Presented);
    }

    #[test]
    fn presentation_repeated_commits_before_done_fire_once() {
        let mut p = Presentation::default();
        p.buffer_committed();
        p.buffer_committed();
        assert!(p.frame_done());
        assert!(!p.frame_done());
    }

    #[test]
    fn physical_rounds() {
        assert_eq!(physical(100, 1.0), 100);
        assert_eq!(physical(100, 1.25), 125);
        assert_eq!(physical(33, 1.5), 50); // 49.5 → 50
        assert_eq!(physical(0, 2.0), 1);
    }

    #[test]
    fn known_output_is_sent_to_layer_shell() {
        let a = OutputId::for_test(1);
        let p = place_layer(OutputTarget::Output(a), |id| (id == a).then_some(2), [2, 1]);
        assert_eq!(p.wire, Some(a));
        assert!(!p.follows_enter);
        assert_eq!(p.scale, 2.0);
    }

    #[test]
    fn compositor_target_sends_null_output() {
        let p = place_layer(OutputTarget::Compositor, |_| Some(1), [1, 3, 2]);
        assert_eq!(p.wire, None, "get_layer_surface'e null çıkış gider");
        assert!(p.follows_enter);
        // Çıkış bilinmez: en büyük ölçek (bulanık ilk kare yok).
        assert_eq!(p.scale, 3.0);
        // Hiç çıkış yok ya da bozuk ölçek: 1.
        assert_eq!(
            place_layer(OutputTarget::Compositor, |_| None, []).scale,
            1.0
        );
        assert_eq!(
            place_layer(OutputTarget::Compositor, |_| None, [0, -1]).scale,
            1.0
        );
    }

    #[test]
    fn unknown_output_falls_back_to_compositor() {
        let gone = OutputId::for_test(9);
        let p = place_layer(OutputTarget::Output(gone), |_| None, [1]);
        assert_eq!(p.wire, None);
        assert!(
            p.follows_enter,
            "bayat kimlik değil, enter'la gelen çıkış raporlanır"
        );
    }

    #[test]
    fn enter_assigns_output_only_when_following() {
        let (a, b) = (OutputId::for_test(1), OutputId::for_test(2));
        let mut out = None;
        assert!(track_enter(&mut out, true, b));
        assert_eq!(out, Some(b));
        assert!(!track_enter(&mut out, true, b), "aynı çıkış: değişim yok");
        // Başka çıkışın leave'i yok sayılır, kendi çıkışınınki temizler.
        assert!(!track_leave(&mut out, true, a));
        assert!(track_leave(&mut out, true, b));
        assert_eq!(out, None);
        // Açık çıkışlı (izlenmeyen) yüzeyde enter/leave çıkışa dokunmaz.
        let mut fixed = Some(a);
        assert!(!track_enter(&mut fixed, false, b));
        assert!(!track_leave(&mut fixed, false, a));
        assert_eq!(fixed, Some(a));
    }
}
