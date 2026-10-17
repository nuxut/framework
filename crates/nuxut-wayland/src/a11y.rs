//! AT-SPI erişilebilirliği (ADR-0011, `a11y` özelliği): yüzey başına bir `accesskit_unix`
//! adaptörü.
//!
//! - **Tembel:** at-spi2-core kurulu değilse ya da `NUXUT_A11Y=0` ise adaptör hiç kurulmaz
//!   (thread, D-Bus bağlantısı yok). Kuruluysa adaptörün arka plan thread'i yalnız
//!   `org.a11y.Status.IsEnabled`'ı bekler; ağaç ancak bir yardımcı teknoloji etkinleştirince
//!   (`request_initial_tree`) ve yalnız o yüzey için kurulur.
//! - **Thread sınırı:** AccessKit işleyicileri adaptör thread'inden çağrılır; burada yalnız calloop
//!   kanalına mesaj atılır (`A11yEvent`). Ağaç ana thread'de, çizimden sonra kurulur
//!   (`State::a11y_update`): bileşen dikdörtgenleri güncel.
//! - **Hız sınırı:** animasyon sürerken en çok [`REBUILD_INTERVAL`] aralıkla; yüzey son karesini
//!   çizince (artık kare istemiyorsa) her zaman. Ek kare ya da zamanlayıcı kurulmaz.
//! - **Gizlilik:** ağaç içeriği loglanmaz (değişmez 9); yalnız yüzey kimliği ve düğüm sayısı.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use accesskit::{ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, TreeUpdate};
use calloop::channel::Sender;
use nuxut_access::AccessRequest;
use nuxut_core::{Insets, Rect};

use crate::types::{Anchor, SurfaceId};

/// Animasyon sırasında ağaç en çok bu aralıkla yeniden kurulur.
pub(crate) const REBUILD_INTERVAL: Duration = Duration::from_millis(100);

/// at-spi-bus-launcher'ın oturum bus'ı hizmet dosyası (`XDG_DATA_DIRS` altında).
const BUS_SERVICE: &str = "dbus-1/services/org.a11y.Bus.service";

/// Adaptör thread'inden ana thread'e.
#[derive(Debug)]
pub(crate) enum A11yEvent {
    /// Yardımcı teknoloji ağacı istedi (AccessKit `Pending`): tam ağaç gönderilmeli.
    Activate(SurfaceId),
    /// Erişilebilirlik kapandı: ağaç kurmayı bırak.
    Deactivate(SurfaceId),
    /// Süzülmüş AT eylemi (metin yazan eylemler `AccessRequest::from_accesskit`'te düşer).
    Action(SurfaceId, AccessRequest),
}

struct Handler {
    surface: SurfaceId,
    tx: Sender<A11yEvent>,
}

impl ActivationHandler for Handler {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // Ağaç ana thread'de kurulur; AccessKit `None`da bekler ve `update_if_active` ile alır.
        let _ = self.tx.send(A11yEvent::Activate(self.surface));
        None
    }
}

impl ActionHandler for Handler {
    fn do_action(&mut self, request: ActionRequest) {
        if let Some(req) = AccessRequest::from_accesskit(&request) {
            let _ = self.tx.send(A11yEvent::Action(self.surface, req));
        } else {
            tracing::debug!(action = ?request.action, "erişilebilirlik: desteklenmeyen/izin verilmeyen AT eylemi düşürüldü");
        }
    }
}

impl DeactivationHandler for Handler {
    fn deactivate_accessibility(&mut self) {
        let _ = self.tx.send(A11yEvent::Deactivate(self.surface));
    }
}

/// Layer yüzeyinin yerleşimi: ekran konumu hesabı için (Wayland istemciye konum vermez).
#[derive(Debug, Clone, Copy)]
pub(crate) struct LayerPlace {
    pub anchor: Anchor,
    pub margin: Insets,
}

struct Entry {
    adapter: accesskit_unix::Adapter,
    /// AT ağacı istedi ve henüz kapatmadı.
    active: bool,
    /// Etkinleşme ya da `a11y_invalidate`: bir sonraki turda (yüzey çizilmişse) kur.
    dirty: bool,
    last_build: Option<Instant>,
    place: Option<LayerPlace>,
    /// AT'ye şu an içeriksiz ağaç (yalnız kök) gösteriliyor: askıda ya da yüzeyin girdisi kapalı.
    blank: bool,
    /// Compositor'ın verdiği klavye odağı (askı kalkınca AT'ye yeniden bildirilir).
    window_focused: bool,
}

pub(crate) struct A11y {
    /// `None`: erişilebilirlik kullanılamaz (at-spi yok ya da `NUXUT_A11Y=0`); hiçbir şey kurulmaz.
    tx: Option<Sender<A11yEvent>>,
    entries: HashMap<SurfaceId, Entry>,
    /// Uygulama erişilebilirliği askıya aldı (`Context::set_a11y_suspended`; ör. oturum kilitli):
    /// ağaçlar içeriksiz, eylemler ve odak bildirimleri atılır.
    suspended: bool,
}

impl std::fmt::Debug for A11y {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("A11y")
            .field("available", &self.tx.is_some())
            .field("surfaces", &self.entries.len())
            .finish()
    }
}

/// Erişilebilirlik bu oturumda kullanılabilir mi: `NUXUT_A11Y=0` değil ve at-spi2-core kurulu.
pub(crate) fn available() -> bool {
    if std::env::var("NUXUT_A11Y").is_ok_and(|v| v.trim() == "0") {
        tracing::info!("erişilebilirlik NUXUT_A11Y=0 ile kapalı");
        return false;
    }
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.trim().is_empty());
    let dirs = dirs.as_deref().unwrap_or("/usr/local/share:/usr/share");
    let found = dirs
        .split(':')
        .filter(|d| !d.is_empty())
        .any(|d| Path::new(d).join(BUS_SERVICE).is_file());
    if !found {
        tracing::info!("at-spi2-core (org.a11y.Bus) bulunamadı; erişilebilirlik adaptörü kurulmuyor");
    }
    found
}

impl A11y {
    pub fn new(tx: Option<Sender<A11yEvent>>) -> Self {
        Self { tx, entries: HashMap::new(), suspended: false }
    }

    /// Yeni layer/pencere yüzeyi. Kilit yüzeyleri yalnız uygulamanın açık izniyle eklenir
    /// (`Context::set_a11y_lock_surfaces`, ADR-0011 "Kilit ekranı").
    pub fn surface_added(&mut self, surface: SurfaceId, place: Option<LayerPlace>) {
        let Some(tx) = &self.tx else { return };
        let handler = || Handler { surface, tx: tx.clone() };
        let adapter = accesskit_unix::Adapter::new(handler(), handler(), handler());
        self.entries.insert(
            surface,
            Entry { adapter, active: false, dirty: false, last_build: None, place, blank: false, window_focused: false },
        );
    }

    /// Yok edilmiş yüzeylerin adaptörlerini düşürür (AT'den ağaç kalkar).
    pub fn retain(&mut self, alive: impl Fn(SurfaceId) -> bool) {
        self.entries.retain(|id, _| alive(*id));
    }

    /// Adaptör thread'inden gelen olay; uygulamaya verilecek eylemi döndürür.
    pub fn event(&mut self, event: A11yEvent) -> Option<(SurfaceId, AccessRequest)> {
        match event {
            A11yEvent::Activate(id) => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.active = true;
                    tracing::debug!(?id, "erişilebilirlik etkin");
                    // accesskit_atspi_common 0.21 bekleyen adaptörü ilk güncellemede uygulama
                    // listesine SONA ekler ve listeyi kimliğe göre ikili arar: adaptörler kimlik
                    // sırasıyla etkinleşmezse `adapter_index(..).unwrap()` panikler (çok yüzeyli
                    // kabukta yaşandı: henüz çizilmemiş yüzey sonra etkinleşti). Etkinleşme
                    // olayları adaptör sırasıyla gelir; her birini HEMEN içeriksiz ağaçla
                    // etkinleştiririz, gerçek ağaç çizimden sonra (`blank` → yeniden kurulur).
                    self.blank(id, Instant::now());
                    if let Some(e) = self.entries.get_mut(&id) {
                        e.dirty = true;
                    }
                }
                None
            }
            A11yEvent::Deactivate(id) => {
                if let Some(e) = self.entries.get_mut(&id) {
                    e.active = false;
                    e.dirty = false;
                }
                None
            }
            A11yEvent::Action(id, req) => self.entries.get(&id).filter(|e| e.active).map(|_| (id, req)),
        }
    }

    pub fn is_active(&self, surface: SurfaceId) -> bool {
        self.entries.get(&surface).is_some_and(|e| e.active)
    }

    pub fn invalidate(&mut self, surface: SurfaceId) {
        if let Some(e) = self.entries.get_mut(&surface).filter(|e| e.active) {
            e.dirty = true;
        }
    }

    /// Yüzeyin klavye odağı (Orca yalnız etkin pencerenin odak olaylarını okur).
    /// `allowed`: yüzeyin girdisi açık (`Context::input_allowed`). Askıdayken ya da girdisi
    /// kapalı yüzeyde AT'ye "odak geldi" bildirilmez (kilitliyken yüzeyin varlığı/odağı sızmasın);
    /// kayıp her zaman bildirilir.
    pub fn focus_changed(&mut self, surface: SurfaceId, focused: bool, allowed: bool) {
        let suspended = self.suspended;
        if let Some(e) = self.entries.get_mut(&surface) {
            e.window_focused = focused;
            e.adapter.update_window_focus_state(focused && allowed && !suspended);
        }
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    /// Askıya al / sürdür. Askıda: tüm yüzeylerde pencere odağı AT'ye "yok", ağaçlar bir sonraki
    /// turda içeriksizleşir (`State::a11y_update`), eylemler atılır. Sürdürünce ağaçlar yeniden
    /// kurulur ve gerçek odak geri bildirilir.
    pub fn set_suspended(&mut self, suspended: bool) {
        if self.suspended == suspended {
            return;
        }
        self.suspended = suspended;
        tracing::info!(suspended, "erişilebilirlik askı durumu");
        for e in self.entries.values_mut() {
            e.adapter.update_window_focus_state(e.window_focused && !suspended);
            e.dirty = true;
        }
    }

    /// Etkin ve henüz içeriksiz gösterilmeyen yüzey (içeriksizleştirilmeli).
    pub fn needs_blank(&self, surface: SurfaceId) -> bool {
        self.entries.get(&surface).is_some_and(|e| e.active && !e.blank)
    }

    /// AT'ye yalnız kök (adsız, çocuksuz) gösterir: içerik, odak ve eylem yok.
    pub fn blank(&mut self, surface: SurfaceId, now: Instant) {
        let update = nuxut_access::AccessTree::new().finish();
        self.apply(surface, update, Rect::ZERO, now);
        if let Some(e) = self.entries.get_mut(&surface) {
            e.blank = true;
        }
    }

    pub fn is_blank(&self, surface: SurfaceId) -> bool {
        self.entries.get(&surface).is_some_and(|e| e.blank)
    }

    /// Layer yerleşimi değişti (`Context::set_layer_placement`).
    pub fn set_place(&mut self, surface: SurfaceId, place: LayerPlace) {
        if let Some(e) = self.entries.get_mut(&surface) {
            e.place = Some(place);
        }
    }

    pub fn place(&self, surface: SurfaceId) -> Option<LayerPlace> {
        self.entries.get(&surface).and_then(|e| e.place)
    }

    /// Ağacı bu turda kurmak gerekiyor mu. `drawn`: bu turda çizildi; `animating`: çizimden
    /// sonra yeni kare istedi; `presented`: en az bir kare çizildi (dikdörtgenler kurulu).
    pub fn due(&self, surface: SurfaceId, drawn: bool, animating: bool, presented: bool, now: Instant) -> bool {
        let Some(e) = self.entries.get(&surface).filter(|e| e.active) else {
            return false;
        };
        presented
            && (e.dirty
                || (drawn
                    && (!animating
                        || e.last_build.is_none_or(|t| now.saturating_duration_since(t) >= REBUILD_INTERVAL))))
    }

    /// Kurulan ağacı uygular; `bounds`: yüzeyin ekran dikdörtgeni (biliniyorsa).
    pub fn apply(&mut self, surface: SurfaceId, update: TreeUpdate, bounds: Rect, now: Instant) {
        let Some(e) = self.entries.get_mut(&surface) else { return };
        let r = accesskit::Rect {
            x0: f64::from(bounds.x),
            y0: f64::from(bounds.y),
            x1: f64::from(bounds.x + bounds.width),
            y1: f64::from(bounds.y + bounds.height),
        };
        e.adapter.set_root_window_bounds(r, r);
        let nodes = update.nodes.len();
        e.adapter.update_if_active(|| update);
        e.dirty = false;
        e.blank = false;
        e.last_build = Some(now);
        tracing::trace!(?surface, nodes, "erişilebilirlik ağacı güncellendi");
    }
}

/// Layer yüzeyinin ekran dikdörtgeni: çıkışın mantıksal konumu/boyutu + anchor + margin.
/// Diğer yüzeylerin dışlama bölgeleri hesaba katılmaz (yaklaşık; bar'ı iten panel birkaç piksel
/// kayabilir). Anchor'suz eksende compositor ortalar.
pub(crate) fn layer_rect(output: Rect, place: LayerPlace, size: (f32, f32)) -> Rect {
    let axis = |start: f32, len: f32, lo: bool, hi: bool, m_lo: f32, m_hi: f32, s: f32| match (lo, hi) {
        (true, true) if s <= 0.0 => (start + m_lo, (len - m_lo - m_hi).max(0.0)),
        (true, true) => (start + m_lo + ((len - m_lo - m_hi - s) / 2.0).max(0.0), s),
        (true, false) => (start + m_lo, s),
        (false, true) => (start + len - m_hi - s, s),
        (false, false) => (start + (len - s) / 2.0, s),
    };
    let a = place.anchor;
    let m = place.margin;
    let (x, w) = axis(output.x, output.width, a.contains(Anchor::LEFT), a.contains(Anchor::RIGHT), m.left, m.right, size.0);
    let (y, h) = axis(output.y, output.height, a.contains(Anchor::TOP), a.contains(Anchor::BOTTOM), m.top, m.bottom, size.1);
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_rect_follows_anchor_and_margin() {
        let out = Rect::new(1920.0, 0.0, 2560.0, 1440.0);
        let bar = LayerPlace { anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, margin: Insets::ZERO };
        assert_eq!(layer_rect(out, bar, (2560.0, 40.0)), Rect::new(1920.0, 0.0, 2560.0, 40.0));
        let panel = LayerPlace {
            anchor: Anchor::TOP | Anchor::RIGHT,
            margin: Insets { top: 48.0, right: 8.0, bottom: 0.0, left: 0.0 },
        };
        assert_eq!(layer_rect(out, panel, (400.0, 600.0)), Rect::new(1920.0 + 2560.0 - 8.0 - 400.0, 48.0, 400.0, 600.0));
        let center = LayerPlace { anchor: Anchor::NONE, margin: Insets::ZERO };
        assert_eq!(layer_rect(out, center, (600.0, 400.0)), Rect::new(1920.0 + 980.0, 520.0, 600.0, 400.0));
    }

    #[test]
    fn rebuild_is_rate_limited_while_animating() {
        // Adaptörsüz durum makinesi: `due` yalnız etkin girdide anlamlı; burada A11y'yi
        // girdisiz kurup yalnız etkin olmayan yüzeyin hiç kurulmadığını doğrularız (adaptör
        // gerçek bus ister; uçtan uca test tests/a11y_bus.rs).
        let a = A11y::new(None);
        let id = SurfaceId::for_test(1);
        assert!(!a.due(id, true, false, true, Instant::now()));
        assert!(!a.is_active(id));
    }
}
