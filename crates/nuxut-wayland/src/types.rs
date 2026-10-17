//! Runtime'ın dışa açtığı tanımlayıcılar ve yüzey tanımı.

use std::ops::BitOr;

use nuxut_core::{Insets, Point, Size};

/// Bir çıkışın (monitör) kimliği. Wayland global adıdır; çıkış yaşadığı sürece sabittir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(pub(crate) u32);

/// Runtime'ın yarattığı bir yüzeyin kimliği. Tekrar kullanılmaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub(crate) u64);

/// Test kurucuları (`test-util` özelliği; yalnızca dev-dependency'de açılır). Uygulama bu
/// kimlikleri üretimde kuramaz: gerçek değerleri runtime verir. Testte keyfî sayı verilir;
/// aynı sayı aynı kimliktir (eşitlik/sıra/hash ham değere göre).
#[cfg(any(test, feature = "test-util"))]
impl OutputId {
    pub fn for_test(raw: u32) -> Self {
        Self(raw)
    }
}

#[cfg(any(test, feature = "test-util"))]
impl SurfaceId {
    pub fn for_test(raw: u64) -> Self {
        Self(raw)
    }
}

/// `Context::set_timer` ile kurulan tek atımlık zamanlayıcının kimliği.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(pub(crate) u64);

/// Bir çıkışın uygulamaya görünen özellikleri. Koordinatlar mantıksal (compositor uzayı).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    pub id: OutputId,
    /// Bağlayıcı adı (`DP-1`, `Virtual-1`…). Compositor vermezse boş.
    pub name: String,
    pub description: String,
    pub position: Point,
    pub size: Size,
    /// Compositor'ın bildirdiği tamsayı ölçek. Kesirli ölçek yüzey başına
    /// `App::surface_resized` ile gelir.
    pub scale: f32,
    /// Geçerli kipin yenileme hızı (Hz, `wl_output.mode` current); bilinmiyorsa 0.
    pub refresh: f32,
    /// Geçerli kipin çözünürlüğü, fiziksel piksel, panelin kendi yönünde (`wl_output.mode`
    /// current; dönüş uygulanmamış — "2560×1440" gibi ayarlarda gösterilen değer). Bilinmiyorsa
    /// `(0, 0)`. xdg-output'tan değil: o mantıksal boyuttur ([`OutputInfo::size`]).
    pub mode_size: (u32, u32),
    /// [`OutputInfo::mode_size`]'ın compositor uzayındaki yönü: 90/270 derece dönüşte eksenler
    /// yer değiştirir; `size` ile aynı yönde. `physical_size / size` çıkışın etkin (kesirli
    /// olabilen) ölçeğidir. Bilinmiyorsa `(0, 0)`.
    pub physical_size: (u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Layer {
    Background,
    Bottom,
    #[default]
    Top,
    Overlay,
}

/// Yüzeyin yapıştığı kenarlar. Karşılıklı iki kenar = o eksende gerilir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Anchor(u8);

impl Anchor {
    pub const NONE: Self = Self(0);
    pub const TOP: Self = Self(1);
    pub const BOTTOM: Self = Self(2);
    pub const LEFT: Self = Self(4);
    pub const RIGHT: Self = Self(8);
    pub const ALL: Self = Self(15);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl BitOr for Anchor {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyboardMode {
    /// Klavye odağı hiç almaz.
    #[default]
    None,
    /// Compositor'ın normal odak kurallarıyla (tıklayınca) odak alır.
    OnDemand,
    /// Üst katmanlarda tüm klavye girdisini alır (kilit ekranı, launcher).
    Exclusive,
}

/// Layer yüzeyinin hangi çıkışta açılacağı.
///
/// `Compositor`: `zwlr_layer_shell_v1.get_layer_surface`'e çıkış verilmez (null); çıkışı
/// compositor kendi politikasıyla seçer (Hyprland ve sway: odaklı/etkin çıkış). Yüzeyin
/// gerçek çıkışı `wl_surface.enter` ile öğrenilir: `Context::surface_output` o zamana dek
/// `None`, sonra seçilen çıkış; değişim `App::surface_output_changed` ile bildirilir.
/// Polkit diyaloğu, karşılama ve OSD gibi "kullanıcının baktığı yerde" açılan yüzeyler için.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputTarget {
    /// Belirli bir çıkış. Çıkış bilinmiyorsa (yarışta kaldırıldıysa) `Compositor` gibi
    /// davranır (uyarıyla).
    Output(OutputId),
    /// Çıkışı compositor seçer.
    Compositor,
}

impl From<OutputId> for OutputTarget {
    fn from(id: OutputId) -> Self {
        Self::Output(id)
    }
}

/// Bir layer-shell yüzeyinin tanımı.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerSurfaceSpec {
    /// Compositor kurallarında görünen ad (`hyprctl layers`).
    pub namespace: String,
    pub output: OutputTarget,
    pub layer: Layer,
    pub anchor: Anchor,
    /// Mantıksal boyut; 0 = karşılıklı anchor yönünde gerilir.
    pub size: (u32, u32),
    /// Diğer pencereler için ayrılan şerit; -1 = başka yüzeylerin bölgesini yok say.
    pub exclusive_zone: i32,
    pub margin: Insets,
    pub keyboard: KeyboardMode,
}

impl LayerSurfaceSpec {
    /// `output` üzerinde (bir `OutputId` ya da `OutputTarget`), `Top` katmanında, anchor'sız ve
    /// boyutsuz başlangıç tanımı.
    pub fn new(namespace: impl Into<String>, output: impl Into<OutputTarget>) -> Self {
        Self {
            namespace: namespace.into(),
            output: output.into(),
            layer: Layer::Top,
            anchor: Anchor::NONE,
            size: (0, 0),
            exclusive_zone: 0,
            margin: Insets::ZERO,
            keyboard: KeyboardMode::None,
        }
    }

    /// Çıkışı compositor'ın seçtiği (odaklı/etkin çıkış) başlangıç tanımı;
    /// `new(namespace, OutputTarget::Compositor)` ile aynı.
    pub fn on_focused_output(namespace: impl Into<String>) -> Self {
        Self::new(namespace, OutputTarget::Compositor)
    }
}

/// Normal uygulama penceresi (`xdg_toplevel`) tanımı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowSpec {
    pub title: String,
    /// Hyprland pencere kuralları ve `.desktop` eşlemesi için sınıf.
    pub app_id: String,
    /// Mantıksal başlangıç boyutu (compositor değiştirebilir; döşeme vb.).
    pub size: (u32, u32),
    pub min_size: Option<(u32, u32)>,
    pub max_size: Option<(u32, u32)>,
}

impl Default for WindowSpec {
    fn default() -> Self {
        Self {
            title: String::new(),
            app_id: String::new(),
            size: (800, 600),
            min_size: None,
            max_size: None,
        }
    }
}

impl WindowSpec {
    pub fn new(title: impl Into<String>, app_id: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            app_id: app_id.into(),
            ..Self::default()
        }
    }
}

/// Etkileşimli yeniden boyutlandırmada tutulan kenar/köşe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Compositor'ın bildirdiği pencere durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct WindowState {
    pub maximized: bool,
    pub fullscreen: bool,
    /// Pencere odakta/etkin (başlık çubuğu buna göre çizilir).
    pub activated: bool,
    /// Herhangi bir kenarı döşenmiş (Hyprland döşeme düzeni).
    pub tiled: bool,
}

/// Oturum kilidi hatası.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum LockError {
    #[error("compositor ext-session-lock-v1 sunmuyor")]
    Unsupported,
    #[error("zaten bir kilit isteği ya da kilit var")]
    AlreadyActive,
    #[error("oturum kilitli değil")]
    NotLocked,
    /// `draw` içinde çağrıldı: kilit yüzeyleri çizim sırasında eklenip kaldırılamaz.
    #[error("draw içinde kilit isteği/açma yapılamaz")]
    InDraw,
}

/// Oturum kilidinin durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LockState {
    #[default]
    Unlocked,
    /// İstendi; compositor'ın `locked`'ı bekleniyor (henüz kilitli SAYILMAZ).
    Pending,
    Locked,
}

/// Kilidin neden bittiği (runtime'ın kesin bilgisi).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LockEnd {
    /// `locked` hiç gelmedi: compositor reddetti ya da başka bir kilitleyici var.
    Refused,
    /// `Locked` iken compositor `finished` gönderdi.
    CompositorEnded,
    /// `unlock_session` sonrası.
    Unlocked,
}

/// `run` başarısız oldu. Panic yerine bu döner.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("Wayland bağlantısı kurulamadı: {0}")]
    Connect(String),
    #[error("compositor gerekli protokolü sunmuyor: {0}")]
    MissingProtocol(&'static str),
    #[error("EGL: {0}")]
    Egl(String),
    #[error("olay döngüsü: {0}")]
    EventLoop(String),
    #[error("compositor bağlantısı koptu: {0}")]
    Disconnected(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constructors_compare_by_raw_value() {
        assert_eq!(OutputId::for_test(7), OutputId::for_test(7));
        assert!(OutputId::for_test(1) < OutputId::for_test(2));
        assert_ne!(SurfaceId::for_test(1), SurfaceId::for_test(2));
        let set: std::collections::HashSet<_> =
            [SurfaceId::for_test(3), SurfaceId::for_test(3)].into();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn anchor_flags_match_protocol_bits() {
        let a = Anchor::TOP | Anchor::LEFT | Anchor::RIGHT;
        assert_eq!(a.bits(), 1 | 4 | 8);
        assert!(a.contains(Anchor::LEFT | Anchor::RIGHT));
        assert!(!a.contains(Anchor::BOTTOM));
        assert_eq!(
            Anchor::ALL,
            Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT
        );
    }

    #[test]
    fn spec_output_target() {
        let a = OutputId::for_test(4);
        assert_eq!(
            LayerSurfaceSpec::new("x", a).output,
            OutputTarget::Output(a)
        );
        let spec = LayerSurfaceSpec::on_focused_output("x");
        assert_eq!(spec.output, OutputTarget::Compositor);
        assert_eq!(spec, LayerSurfaceSpec::new("x", OutputTarget::Compositor));
    }
}
