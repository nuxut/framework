//! Renk dönüşüm matrisi (`hyprland_ctm_control_manager_v1`, Hyprland'e özgü): çıkış başına
//! 3×3 satır-öncelikli CTM. Gece ışığı gibi tüketiciler matrisi hesaplar ve verir; runtime
//! protokol nesnesinin yaşamını, sabit nokta (`wl_fixed`) dönüşümünü ve doğrulamayı yürütür.
//!
//! Protokol XML'i `protocols/hyprland-ctm-control-v1.xml`'de gömülü (hyprland-protocols
//! 6c520135, sürüm 2, BSD-3); bağlama `wayland-scanner` ile derlemede üretilir.
//!
//! Hyprland semantiği (v0.56.2 `src/protocols/CTMControl.cpp`; ayrıntı ve satırlar
//! `docs/ARCHITECTURE.md` → "Renk dönüşümü"):
//! - Global'e ilk bağlanan yönetici sahip olur; sonra bağlananlar `blocked` alır (sürüm 2) ve
//!   istekleri sessizce yok sayılır. Sahiplik bağlanmayla alınır — bu yüzden yönetici yalnızca
//!   ilk birim olmayan `set_color_transforms` çağrısında bağlanır: gece ışığı kapalı kullanıcıda
//!   CTM'e dokunulmaz, kullanıcının hyprsunset'i engellenmez.
//! - Sahip yönetici yok edilince (istek ya da istemci kapanması/çökmesi) tüm çıkışlar birim
//!   matrise döner. `color_transform_release` bunu kullanır.
//! - Negatif ya da sonlu olmayan bileşen `invalid_matrix` protokol hatasıdır (bağlantı ölür);
//!   runtime bunu göndermeden önce reddeder.
//! - `blocked` sonrası yönetici yok edilir (protokolün önerisi; engellenmiş yöneticinin yıkımı
//!   CTM'e dokunmaz) ve uygulamaya `App::color_transform_blocked` gelir. Durum `Blocked`
//!   kalır: kendiliğinden yeniden denenmez (sahip istemci çıktığında haber gelmez);
//!   `color_transform_release` durumu sıfırlar, sonraki `set` yeniden bağlanır.

use std::collections::HashMap;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryHandler};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Proxy, QueueHandle};

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::State;
use crate::types::OutputId;

/// `wayland-scanner` ile üretilen istemci bağlaması.
#[allow(
    dead_code,
    non_camel_case_types,
    unused_unsafe,
    unused_variables,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    missing_docs,
    missing_debug_implementations,
    clippy::all
)]
pub(crate) mod protocol {
    use wayland_client;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/hyprland-ctm-control-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/hyprland-ctm-control-v1.xml");
}

use protocol::hyprland_ctm_control_manager_v1::{self, HyprlandCtmControlManagerV1};

/// Yönetici global'inin arayüz adı.
pub(crate) const MANAGER_INTERFACE: &str = "hyprland_ctm_control_manager_v1";

/// Bağlanılan en yüksek sürüm (2: `blocked` olayı).
const MAX_VERSION: u32 = 2;

/// `wl_fixed` (24.8) ile gösterilebilen en büyük değer.
const FIXED_MAX: f32 = 8_388_607.0;

/// 3×3 renk dönüşüm matrisi, satır-öncelikli: çıktı `[r', g', b'] = M · [r, g, b]`.
/// Bileşenler sonlu ve ≥ 0 olmalı (protokol); gönderimde 1/256 adıma yuvarlanır.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorMatrix(pub [f32; 9]);

impl ColorMatrix {
    /// Birim matris: etkisiz.
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    /// Köşegen matris (kanal başına çarpan; gece ışığı `diagonal(r, g, b)`).
    pub const fn diagonal(r: f32, g: f32, b: f32) -> Self {
        Self([r, 0.0, 0.0, 0.0, g, 0.0, 0.0, 0.0, b])
    }

    /// Sabit noktaya yuvarlanınca birim matrise eşit.
    pub fn is_identity(&self) -> bool {
        to_fixed(self) == to_fixed(&Self::IDENTITY)
    }
}

impl Default for ColorMatrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Compositor'ın renk dönüşümü desteği ve yöneticinin durumu (`Context::color_transform_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTransformState {
    /// `hyprland_ctm_control_manager_v1` global'i yok (sway vb.): özellik gösterilmez.
    Unsupported,
    /// Global var, yönetici bağlı değil: CTM'e dokunulmuyor (compositor'daki değer bizim değil).
    Idle,
    /// Yönetici bağlı, matrisler gönderildi. Sürüm 1 compositor'da başka istemci sahipse
    /// istekler sessizce yok sayılır ve bu bilinemez (Hyprland 0.56 sürüm 2 ilan eder).
    Active,
    /// Başka bir istemci (ör. hyprsunset) CTM'in sahibi; istekler yok sayılırdı.
    /// `color_transform_release` sonrası yeniden denenebilir.
    Blocked,
}

/// `set_color_transforms` hatası. Hata durumunda hiçbir istek gönderilmez.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorTransformError {
    /// Compositor protokolü sunmuyor.
    Unsupported,
    /// Başka bir istemci sahip (`ColorTransformState::Blocked`).
    Blocked,
    /// Bilinmeyen (ya da kaldırılmış) çıkış.
    UnknownOutput(OutputId),
    /// Matris bileşeni negatif, sonlu değil ya da `wl_fixed` aralığı dışında (`index` 0..9).
    InvalidMatrix { output: OutputId, index: usize },
}

impl std::fmt::Display for ColorTransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported => f.write_str("compositor hyprland-ctm-control-v1 sunmuyor"),
            Self::Blocked => f.write_str("renk dönüşümünü başka bir istemci yönetiyor"),
            Self::UnknownOutput(o) => write!(f, "bilinmeyen çıkış {o:?}"),
            Self::InvalidMatrix { output, index } => {
                write!(f, "geçersiz matris bileşeni {index} (çıkış {output:?})")
            }
        }
    }
}

impl std::error::Error for ColorTransformError {}

/// Matrisi `wl_fixed` ham değerlerine (değer × 256, en yakına) çevirir; önce `validate`.
pub(crate) fn to_fixed(m: &ColorMatrix) -> [i32; 9] {
    m.0.map(|v| (f64::from(v) * 256.0).round() as i32)
}

/// Gönderilebilir mi: her bileşen sonlu, ≥ 0 ve `wl_fixed` aralığında. `-0.0` geçerlidir.
pub(crate) fn validate(m: &ColorMatrix) -> Result<(), usize> {
    match m
        .0
        .iter()
        .position(|v| !v.is_finite() || *v < 0.0 || *v > FIXED_MAX)
    {
        Some(i) => Err(i),
        None => Ok(()),
    }
}

/// Ham `wl_fixed` değerinden protokol argümanı (`f64`; wayland-backend tam olarak geri çevirir).
fn fixed_to_wire(raw: i32) -> f64 {
    f64::from(raw) / 256.0
}

/// Deftere göre protokol nesnesine uygulanacak adımlar (sırayla).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CtmOp {
    /// Yöneticiyi bağla (kuşak numarasıyla).
    Bind(u64),
    Set(OutputId, [i32; 9]),
    Commit,
    Destroy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Unbound,
    Bound(u64),
    Blocked,
}

/// Saf durum makinesi: yönetici kuşağı ve çıkış başına son gönderilen matris. Wayland'e
/// dokunmaz; `CtmOp` üretir (birim testli).
#[derive(Debug)]
pub(crate) struct CtmBook {
    phase: Phase,
    next_generation: u64,
    /// Bu yöneticiyle gönderilmiş son değerler (tekrar gönderim ve geçiş animasyonunun
    /// boşuna yeniden başlaması önlenir).
    sent: HashMap<OutputId, [i32; 9]>,
}

impl Default for CtmBook {
    fn default() -> Self {
        Self {
            phase: Phase::Unbound,
            next_generation: 0,
            sent: HashMap::new(),
        }
    }
}

impl CtmBook {
    pub fn state(&self, available: bool) -> ColorTransformState {
        match self.phase {
            Phase::Blocked => ColorTransformState::Blocked,
            Phase::Bound(_) => ColorTransformState::Active,
            Phase::Unbound if available => ColorTransformState::Idle,
            Phase::Unbound => ColorTransformState::Unsupported,
        }
    }

    /// Matrisleri tek commit'le gönderir. Hepsi doğrulanır; hata varsa adım yok. Yönetici
    /// yokken yalnızca birim matrisler istenirse bağlanılmaz (CTM'e dokunulmaz). Değişmeyen
    /// çıkış yeniden gönderilmez; hiçbiri değişmediyse commit yok.
    pub fn set(
        &mut self,
        transforms: &[(OutputId, ColorMatrix)],
        available: bool,
    ) -> Result<Vec<CtmOp>, ColorTransformError> {
        for (output, m) in transforms {
            validate(m).map_err(|index| ColorTransformError::InvalidMatrix {
                output: *output,
                index,
            })?;
        }
        let mut ops = Vec::new();
        match self.phase {
            Phase::Blocked => return Err(ColorTransformError::Blocked),
            Phase::Unbound => {
                if !available {
                    return Err(ColorTransformError::Unsupported);
                }
                if transforms.iter().all(|(_, m)| m.is_identity()) {
                    return Ok(ops);
                }
                self.next_generation += 1;
                self.phase = Phase::Bound(self.next_generation);
                self.sent.clear();
                ops.push(CtmOp::Bind(self.next_generation));
            }
            Phase::Bound(_) => {}
        }
        for (output, m) in transforms {
            let raw = to_fixed(m);
            if self.sent.get(output) != Some(&raw) {
                self.sent.insert(*output, raw);
                ops.push(CtmOp::Set(*output, raw));
            }
        }
        if ops.iter().any(|op| matches!(op, CtmOp::Set(..))) {
            ops.push(CtmOp::Commit);
        }
        Ok(ops)
    }

    /// Yöneticiyi bırak (compositor CTM'i birime döndürür); `Blocked` da sıfırlanır.
    pub fn release(&mut self) -> Vec<CtmOp> {
        let was_bound = matches!(self.phase, Phase::Bound(_));
        self.phase = Phase::Unbound;
        self.sent.clear();
        if was_bound {
            vec![CtmOp::Destroy]
        } else {
            Vec::new()
        }
    }

    /// Compositor `blocked` gönderdi. Canlı kuşaksa `true`: yönetici yok edilmeli, uygulamaya
    /// iletilmeli. Eski kuşak: `false`.
    pub fn blocked(&mut self, generation: u64) -> bool {
        if self.phase == Phase::Bound(generation) {
            self.phase = Phase::Blocked;
            self.sent.clear();
            true
        } else {
            false
        }
    }

    /// Çıkış kaldırıldı: son gönderilen değer unutulur.
    pub fn forget_output(&mut self, output: OutputId) {
        self.sent.remove(&output);
    }

    /// Global geri çekildi: yönetici (varsa) yok edilmeli; durum sıfırlanır.
    pub fn global_removed(&mut self) -> Vec<CtmOp> {
        self.release()
    }
}

/// Context'teki renk dönüşümü durumu: global (bağlanmadan), defter, canlı yönetici.
#[derive(Debug, Default)]
pub(crate) struct ColorTransforms {
    /// Global ilan edildi: (kayıt, ad, sürüm). Yönetici ilk `set`'te buradan bağlanır.
    global: Option<(WlRegistry, u32, u32)>,
    book: CtmBook,
    manager: Option<HyprlandCtmControlManagerV1>,
}

impl<A: App> Context<A> {
    /// Compositor `hyprland-ctm-control-v1` sunuyor (Hyprland). Bağlanmaz; CTM'e dokunmaz.
    pub fn color_transform_supported(&self) -> bool {
        self.color_transforms.global.is_some()
    }

    /// Renk dönüşümü durumu (destek, sahiplik).
    pub fn color_transform_state(&self) -> ColorTransformState {
        self.color_transforms
            .book
            .state(self.color_transforms.global.is_some())
    }

    /// Çıkış başına matrisleri tek atomik commit'le uygular. Listede olmayan çıkışlar önceki
    /// değerini korur (bu yöneticiyle hiç ayarlanmamışsa birim). Yönetici ilk birim olmayan
    /// matriste bağlanır; yalnızca birim istenirse ve yönetici yoksa hiçbir şey gönderilmez.
    /// Değişmeyen değerler yeniden gönderilmez. Yeni çıkış için `App::output_added`'da yeniden
    /// çağrılmalı (runtime çıkışa kendiliğinden matris vermez). Kapatmak için birim matris
    /// yerine `color_transform_release` tercih edilir (sahiplik bırakılır).
    pub fn set_color_transforms(
        &mut self,
        transforms: &[(OutputId, ColorMatrix)],
    ) -> Result<(), ColorTransformError> {
        if let Some((output, _)) = transforms
            .iter()
            .find(|(o, _)| !self.outputs.contains_key(o))
        {
            return Err(ColorTransformError::UnknownOutput(*output));
        }
        let available = self.color_transforms.global.is_some();
        let ops = self.color_transforms.book.set(transforms, available)?;
        self.apply_ctm_ops(&ops);
        Ok(())
    }

    /// Yöneticiyi bırakır: Hyprland tüm çıkışların CTM'ini birime döndürür (geçiş
    /// animasyonuyla). `Blocked` durumu da sıfırlanır; sonraki `set` yeniden bağlanmayı dener.
    pub fn color_transform_release(&mut self) {
        let ops = self.color_transforms.book.release();
        self.apply_ctm_ops(&ops);
    }

    fn apply_ctm_ops(&mut self, ops: &[CtmOp]) {
        for &op in ops {
            match op {
                CtmOp::Bind(generation) => {
                    let Some((registry, name, version)) = self.color_transforms.global.as_ref()
                    else {
                        continue;
                    };
                    let manager = registry.bind::<HyprlandCtmControlManagerV1, _, State<A>>(
                        *name,
                        (*version).min(MAX_VERSION),
                        &self.qh,
                        CtmData { generation },
                    );
                    tracing::debug!(
                        version = manager.version(),
                        "hyprland_ctm_control_manager_v1 bağlandı"
                    );
                    if let Some(old) = self.color_transforms.manager.replace(manager) {
                        old.destroy();
                    }
                }
                CtmOp::Set(output, raw) => {
                    let (Some(manager), Some(entry)) = (
                        self.color_transforms.manager.as_ref(),
                        self.outputs.get(&output),
                    ) else {
                        continue;
                    };
                    let [m0, m1, m2, m3, m4, m5, m6, m7, m8] = raw.map(fixed_to_wire);
                    manager.set_ctm_for_output(&entry.wl, m0, m1, m2, m3, m4, m5, m6, m7, m8);
                }
                CtmOp::Commit => {
                    if let Some(manager) = self.color_transforms.manager.as_ref() {
                        manager.commit();
                    }
                }
                CtmOp::Destroy => {
                    if let Some(manager) = self.color_transforms.manager.take() {
                        manager.destroy();
                        tracing::debug!("hyprland_ctm_control_manager_v1 bırakıldı");
                    }
                }
            }
        }
    }

    /// Çıkış kaldırıldı (`outputs`'tan çıkarıldıktan sonra).
    pub(crate) fn color_transform_forget_output(&mut self, output: OutputId) {
        self.color_transforms.book.forget_output(output);
    }

    /// Kapanış: yönetici açıkça yok edilir (bağlantı kapanışı da aynı sonucu verir).
    pub(crate) fn color_transform_shutdown(&mut self) {
        self.color_transform_release();
    }

    fn color_transform_blocked_event(&mut self, generation: u64) {
        if !self.color_transforms.book.blocked(generation) {
            tracing::debug!("eski CTM yöneticisinden blocked; yok sayıldı");
            return;
        }
        // Engellenmiş yöneticinin yıkımı CTM'e dokunmaz (CTMControl.cpp ~CHyprlandCTMControlResource).
        if let Some(manager) = self.color_transforms.manager.take() {
            manager.destroy();
        }
        tracing::info!("renk dönüşümünü başka bir istemci yönetiyor (blocked)");
        self.pending.push(RuntimeEvent::ColorTransformBlocked);
    }
}

/// Yönetici nesnesinin verisi: kuşak.
#[derive(Debug)]
pub(crate) struct CtmData {
    generation: u64,
}

impl<A: App> Dispatch2<HyprlandCtmControlManagerV1, State<A>> for CtmData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &HyprlandCtmControlManagerV1,
        event: hyprland_ctm_control_manager_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        // Tek olay: `blocked`.
        let hyprland_ctm_control_manager_v1::Event::Blocked = event;
        state.cx.color_transform_blocked_event(self.generation);
    }
}

/// Yönetici global'inin ilanı/geri çekilmesi. Bağlanmaz; yalnızca kaydeder.
#[derive(Debug)]
pub(crate) struct ColorTransformGlobal;

impl ColorTransformGlobal {
    /// Global'i kaydeder (açılışta ve çalışma sırasında ilanda).
    pub(crate) fn record<A: App>(state: &mut State<A>, name: u32, version: u32) {
        if state.cx.color_transforms.global.is_some() {
            return;
        }
        let registry = state.registry().registry().clone();
        state.cx.color_transforms.global = Some((registry, name, version));
        tracing::debug!(
            version,
            "hyprland_ctm_control_manager_v1 ilan edildi (bağlanmadı)"
        );
    }
}

impl<A: App> RegistryHandler<State<A>> for ColorTransformGlobal {
    fn new_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        interface: &str,
        version: u32,
    ) {
        if interface == MANAGER_INTERFACE {
            Self::record(state, name, version);
        }
    }

    fn remove_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        _: &str,
    ) {
        if state
            .cx
            .color_transforms
            .global
            .as_ref()
            .is_some_and(|(_, n, _)| *n == name)
        {
            state.cx.color_transforms.global = None;
            let ops = state.cx.color_transforms.book.global_removed();
            state.cx.apply_ctm_ops(&ops);
            tracing::info!("hyprland_ctm_control_manager_v1 geri çekildi");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const O1: OutputId = OutputId(1);
    const O2: OutputId = OutputId(2);

    fn warm() -> ColorMatrix {
        ColorMatrix::diagonal(1.0, 0.8, 0.6)
    }

    #[test]
    fn fixed_conversion_rounds_to_1_256() {
        assert_eq!(
            to_fixed(&ColorMatrix::IDENTITY),
            [256, 0, 0, 0, 256, 0, 0, 0, 256]
        );
        let raw = to_fixed(&ColorMatrix::diagonal(0.5, 0.75, 1.0 / 3.0));
        assert_eq!(raw[0], 128);
        assert_eq!(raw[4], 192);
        assert_eq!(raw[8], 85, "85.33 → 85");
        assert_eq!(fixed_to_wire(85), 85.0 / 256.0);
        // Yarım adımdan küçük sapma birim sayılır.
        assert!(ColorMatrix::diagonal(1.001, 1.0, 0.999).is_identity());
        assert!(!ColorMatrix::diagonal(1.0, 1.0, 0.99).is_identity());
        assert_eq!(ColorMatrix::default(), ColorMatrix::IDENTITY);
    }

    #[test]
    fn invalid_components_are_rejected_before_sending() {
        assert_eq!(validate(&warm()), Ok(()));
        let mut m = ColorMatrix::IDENTITY;
        m.0[3] = -0.1;
        assert_eq!(validate(&m), Err(3));
        m.0[3] = f32::NAN;
        assert_eq!(validate(&m), Err(3));
        m.0[3] = f32::INFINITY;
        assert_eq!(validate(&m), Err(3));
        m.0[3] = 9.0e6;
        assert_eq!(validate(&m), Err(3));
        m.0[3] = -0.0;
        assert_eq!(validate(&m), Ok(()), "-0.0 geçerli");

        let mut book = CtmBook::default();
        let mut bad = warm();
        bad.0[8] = -1.0;
        assert_eq!(
            book.set(&[(O1, warm()), (O2, bad)], true),
            Err(ColorTransformError::InvalidMatrix {
                output: O2,
                index: 8
            })
        );
        assert_eq!(
            book.state(true),
            ColorTransformState::Idle,
            "hiçbir şey gönderilmedi"
        );
    }

    #[test]
    fn identity_without_manager_does_not_bind() {
        let mut book = CtmBook::default();
        assert_eq!(book.set(&[(O1, ColorMatrix::IDENTITY)], true), Ok(vec![]));
        assert_eq!(book.set(&[], true), Ok(vec![]));
        assert_eq!(book.state(true), ColorTransformState::Idle);
    }

    #[test]
    fn first_real_set_binds_then_sets_and_commits() {
        let mut book = CtmBook::default();
        let raw = to_fixed(&warm());
        assert_eq!(
            book.set(&[(O1, warm()), (O2, ColorMatrix::IDENTITY)], true),
            Ok(vec![
                CtmOp::Bind(1),
                CtmOp::Set(O1, raw),
                CtmOp::Set(O2, to_fixed(&ColorMatrix::IDENTITY)),
                CtmOp::Commit,
            ])
        );
        assert_eq!(book.state(true), ColorTransformState::Active);
        // Değişmeyen değer yeniden gönderilmez; commit de yok.
        assert_eq!(book.set(&[(O1, warm())], true), Ok(vec![]));
        // Yalnızca değişen çıkış.
        let cooler = ColorMatrix::diagonal(1.0, 0.9, 0.8);
        assert_eq!(
            book.set(&[(O1, cooler), (O2, ColorMatrix::IDENTITY)], true),
            Ok(vec![CtmOp::Set(O1, to_fixed(&cooler)), CtmOp::Commit])
        );
        // Bağlıyken birim matris gönderilir (sahiplik korunur).
        assert_eq!(
            book.set(&[(O1, ColorMatrix::IDENTITY)], true),
            Ok(vec![
                CtmOp::Set(O1, to_fixed(&ColorMatrix::IDENTITY)),
                CtmOp::Commit
            ])
        );
    }

    #[test]
    fn unsupported_without_global() {
        let mut book = CtmBook::default();
        assert_eq!(book.state(false), ColorTransformState::Unsupported);
        assert_eq!(
            book.set(&[(O1, warm())], false),
            Err(ColorTransformError::Unsupported)
        );
        assert!(book.release().is_empty());
    }

    #[test]
    fn release_destroys_and_next_set_rebinds_with_new_generation() {
        let mut book = CtmBook::default();
        book.set(&[(O1, warm())], true).unwrap();
        assert_eq!(book.release(), vec![CtmOp::Destroy]);
        assert_eq!(book.state(true), ColorTransformState::Idle);
        assert!(book.release().is_empty(), "ikinci bırakış istek üretmez");
        // Yeni yönetici: önceki değerler bilinmez, yeniden gönderilir.
        assert_eq!(
            book.set(&[(O1, warm())], true),
            Ok(vec![
                CtmOp::Bind(2),
                CtmOp::Set(O1, to_fixed(&warm())),
                CtmOp::Commit
            ])
        );
        // Eski kuşağın geç blocked'ı yok sayılır.
        assert!(!book.blocked(1));
        assert_eq!(book.state(true), ColorTransformState::Active);
    }

    #[test]
    fn blocked_stops_requests_until_release() {
        let mut book = CtmBook::default();
        book.set(&[(O1, warm())], true).unwrap();
        assert!(book.blocked(1));
        assert!(!book.blocked(1), "ikinci kez iletilmez");
        assert_eq!(book.state(true), ColorTransformState::Blocked);
        assert_eq!(
            book.set(&[(O1, warm())], true),
            Err(ColorTransformError::Blocked)
        );
        // Engellenmiş yönetici olay işleyicisinde yok edildi: release Destroy üretmez.
        assert!(book.release().is_empty());
        assert_eq!(book.set(&[(O1, warm())], true).unwrap()[0], CtmOp::Bind(2));
    }

    #[test]
    fn removed_output_is_forgotten_and_global_removal_resets() {
        let mut book = CtmBook::default();
        book.set(&[(O1, warm())], true).unwrap();
        book.forget_output(O1);
        assert_eq!(
            book.set(&[(O1, warm())], true),
            Ok(vec![CtmOp::Set(O1, to_fixed(&warm())), CtmOp::Commit])
        );
        assert_eq!(book.global_removed(), vec![CtmOp::Destroy]);
        assert_eq!(book.state(false), ColorTransformState::Unsupported);
    }
}
