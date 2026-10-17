//! Odak kapma (`hyprland_focus_grab_v1`, Hyprland'e özgü): işaretçi/dokunma ve klavye odağını
//! bir yüzey beyaz listesiyle sınırlar. Liste dışına tıklanınca compositor grab'ı bitirir ve
//! `cleared` gönderir; shell paneli o anda kapatır. Grab başlarken compositor'ın önceki grab'ı
//! (ör. bir uygulamanın `xdg_popup` bağlam menüsü) biter — menü panelin üstünde açık kalmaz.
//!
//! Protokol XML'i `protocols/hyprland-focus-grab-v1.xml`'de gömülü (hyprland-protocols, BSD-3);
//! bağlama `wayland-scanner` ile, wayland-protocols crate'iyle aynı yoldan derlemede üretilir.
//!
//! Sıralama kuralları (protokol semantiği; runtime uygular, uygulama yalnızca yüzey kümesini
//! verir):
//! - `add_surface`/`remove_surface` `commit`'e dek etkisizdir. Liste boştan dolu olunca grab
//!   başlar, doludan boşa inince biter (Hyprland bu durumda da `cleared` gönderir).
//! - Liste yalnızca **map edilmiş** (en az bir tampon commit'lenmiş) yüzeyleri içerir: grab
//!   başlarken compositor klavye odağını listedeki bir yüzeye verir; map edilmemiş yüzeye odak
//!   verilmez. Henüz çizilmemiş yüzeyler ilk tamponlarından sonra eklenir.
//! - İlk commit yalnızca birincil yüzeyi (verilen listenin ilk map edilmiş elemanı) içerir,
//!   diğerleri ikinci commit'le eklenir: compositor klavye odağını başka bir yüzeye (ör. bar)
//!   veremez.
//! - Liste boşalınca (uygulama boşalttı ya da tüm yüzeyler yok edildi) grab nesnesi yok
//!   edilir — boş commit yapılmaz; böylece kendi bıraktığımız grab için `cleared` gelmez.
//!   Yeniden dolunca yeni nesne yaratılır (nesne kuşağı: eski nesnenin geç olayı yok sayılır).
//! - Yok edilen yüzey `wl_surface` hâlâ canlıyken (yok etme turun sonunda) listeden çıkarılır
//!   ve commit'lenir; istekler aynı bağlantıda sıralı olduğundan yarış yok.
//! - `cleared` geldiğinde grab etkisizdir: beyaz liste boşaltılır, nesne yok edilir ve
//!   `App::focus_grab_cleared` çağrılır. Kimlik geçerli kalır; uygulama
//!   `focus_grab_set_surfaces` ile yeniden kurabilir ya da `focus_grab_release` der.

use std::collections::HashMap;

use smithay_client_toolkit::dispatch2::Dispatch2;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryHandler};
use wayland_client::{Connection, Proxy, QueueHandle};

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};
use crate::types::SurfaceId;

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
        wayland_scanner::generate_interfaces!("protocols/hyprland-focus-grab-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/hyprland-focus-grab-v1.xml");
}

use protocol::hyprland_focus_grab_manager_v1::HyprlandFocusGrabManagerV1;
use protocol::hyprland_focus_grab_v1::{self, HyprlandFocusGrabV1};

/// Yönetici global'inin arayüz adı.
pub(crate) const MANAGER_INTERFACE: &str = "hyprland_focus_grab_manager_v1";

/// `Context::focus_grab` ile kurulan odak kapmanın kimliği. Tekrar kullanılmaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FocusGrabId(pub(crate) u64);

/// Defterin protokol nesnesine uygulanacak adımları (sırayla).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrabOp {
    /// Yeni grab nesnesi (kuşak numarasıyla).
    Create(u64),
    Add(SurfaceId),
    Remove(SurfaceId),
    Commit,
    Destroy,
}

#[derive(Debug, Default)]
struct GrabRecord {
    /// Uygulamanın istediği liste (sıralı, tekrarsız; ilki birincil).
    wanted: Vec<SurfaceId>,
    /// Compositor'a commit'lenmiş liste.
    committed: Vec<SurfaceId>,
    /// Canlı protokol nesnesinin kuşağı; `None`: nesne yok (etkisiz).
    generation: Option<u64>,
}

/// Saf durum makinesi: istenen ve commit'lenmiş beyaz listeler, nesne kuşağı. Wayland'e
/// dokunmaz; `GrabOp` üretir (birim testli).
#[derive(Debug, Default)]
pub(crate) struct GrabBook {
    grabs: HashMap<FocusGrabId, GrabRecord>,
    next_generation: u64,
}

fn dedup(surfaces: &[SurfaceId]) -> Vec<SurfaceId> {
    let mut out = Vec::with_capacity(surfaces.len());
    for &s in surfaces {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

impl GrabBook {
    pub fn insert(&mut self, id: FocusGrabId, wanted: &[SurfaceId]) {
        self.grabs.insert(
            id,
            GrabRecord {
                wanted: dedup(wanted),
                ..Default::default()
            },
        );
    }

    #[cfg(test)]
    pub fn contains(&self, id: FocusGrabId) -> bool {
        self.grabs.contains_key(&id)
    }

    pub fn ids(&self) -> Vec<FocusGrabId> {
        self.grabs.keys().copied().collect()
    }

    /// Bilinmeyen kimlikte `false`.
    pub fn set_wanted(&mut self, id: FocusGrabId, wanted: &[SurfaceId]) -> bool {
        match self.grabs.get_mut(&id) {
            Some(rec) => {
                rec.wanted = dedup(wanted);
                true
            }
            None => false,
        }
    }

    /// Compositor'da en az bir yüzeyle commit'lenmiş canlı nesne var.
    pub fn is_active(&self, id: FocusGrabId) -> bool {
        self.grabs
            .get(&id)
            .is_some_and(|r| r.generation.is_some() && !r.committed.is_empty())
    }

    /// Kaydı siler; canlı nesne varsa `Destroy`.
    pub fn release(&mut self, id: FocusGrabId) -> Vec<GrabOp> {
        match self.grabs.remove(&id) {
            Some(rec) if rec.generation.is_some() => vec![GrabOp::Destroy],
            _ => Vec::new(),
        }
    }

    /// Yüzey yok ediliyor: her istenen listeden çıkar; değişen grab'ların kimlikleri
    /// (ardından `sync` çağrılmalı, yüzey hâlâ canlıyken).
    pub fn forget_surface(&mut self, surface: SurfaceId) -> Vec<FocusGrabId> {
        let mut touched = Vec::new();
        for (id, rec) in &mut self.grabs {
            let before = rec.wanted.len();
            rec.wanted.retain(|&s| s != surface);
            if rec.wanted.len() != before || rec.committed.contains(&surface) {
                touched.push(*id);
            }
        }
        touched
    }

    /// Bu yüzeyi isteyen grab'lar (yüzey map edilince yeniden eşitlemek için).
    pub fn ids_wanting(&self, surface: SurfaceId) -> Vec<FocusGrabId> {
        self.grabs
            .iter()
            .filter(|(_, r)| r.wanted.contains(&surface))
            .map(|(id, _)| *id)
            .collect()
    }

    /// İstenen listeyi (yalnızca map edilmiş yüzeyler) compositor'a taşır. `can_create`:
    /// yönetici global'i var. Üretilen adımlar uygulanmış varsayılır.
    pub fn sync(
        &mut self,
        id: FocusGrabId,
        mapped: impl Fn(SurfaceId) -> bool,
        can_create: bool,
    ) -> Vec<GrabOp> {
        let Some(rec) = self.grabs.get_mut(&id) else {
            return Vec::new();
        };
        let target: Vec<SurfaceId> = rec.wanted.iter().copied().filter(|&s| mapped(s)).collect();
        let mut ops = Vec::new();
        if target.is_empty() {
            // Boş commit yerine nesneyi yok et: kendi bıraktığımız grab için `cleared` gelmez.
            if rec.generation.take().is_some() {
                ops.push(GrabOp::Destroy);
            }
            rec.committed.clear();
            return ops;
        }
        if rec.generation.is_none() {
            if !can_create {
                return ops;
            }
            self.next_generation += 1;
            rec.generation = Some(self.next_generation);
            rec.committed.clear();
            ops.push(GrabOp::Create(self.next_generation));
        }
        if rec.committed.is_empty() {
            // İlk commit yalnızca birincil yüzey: compositor klavye odağını ona verir.
            ops.push(GrabOp::Add(target[0]));
            ops.push(GrabOp::Commit);
            if target.len() > 1 {
                ops.extend(target[1..].iter().map(|&s| GrabOp::Add(s)));
                ops.push(GrabOp::Commit);
            }
        } else {
            let before = ops.len();
            ops.extend(
                rec.committed
                    .iter()
                    .filter(|s| !target.contains(s))
                    .map(|&s| GrabOp::Remove(s)),
            );
            ops.extend(
                target
                    .iter()
                    .filter(|s| !rec.committed.contains(s))
                    .map(|&s| GrabOp::Add(s)),
            );
            if ops.len() > before {
                ops.push(GrabOp::Commit);
            }
        }
        rec.committed = target;
        ops
    }

    /// Compositor `cleared` gönderdi. Canlı kuşaksa grab etkisizleşir (istenen liste de
    /// boşalır; dışarı tıklamayı geri almamak için kendiliğinden yeniden kurulmaz) ve `true`:
    /// nesne yok edilmeli, uygulamaya iletilmeli. Eski kuşak/bilinmeyen kimlik: `false`.
    pub fn cleared(&mut self, id: FocusGrabId, generation: u64) -> bool {
        match self.grabs.get_mut(&id) {
            Some(rec) if rec.generation == Some(generation) => {
                rec.generation = None;
                rec.committed.clear();
                rec.wanted.clear();
                true
            }
            _ => false,
        }
    }

    #[cfg(test)]
    fn generation(&self, id: FocusGrabId) -> Option<u64> {
        self.grabs.get(&id).and_then(|r| r.generation)
    }
}

/// Context'teki odak kapma durumu: yönetici (global adıyla), defter, canlı nesneler.
#[derive(Debug, Default)]
pub(crate) struct FocusGrabs {
    pub manager: Option<(HyprlandFocusGrabManagerV1, u32)>,
    pub book: GrabBook,
    objects: HashMap<FocusGrabId, HyprlandFocusGrabV1>,
}

impl<A: App> Context<A> {
    /// Compositor `hyprland_focus_grab_v1` sunuyor (Hyprland). Değilse `focus_grab` `None`.
    pub fn focus_grab_supported(&self) -> bool {
        self.focus_grabs.manager.is_some()
    }

    /// Odak kapma kurar: işaretçi/klavye odağı `surfaces` ile sınırlanır; liste dışına
    /// tıklanınca compositor grab'ı bitirir ve `App::focus_grab_cleared` gelir. İlk yüzey
    /// birincildir (klavye odağı ona). Bilinmeyen yüzeyler atlanır; henüz çizilmemiş yüzeyler
    /// ilk tamponlarından sonra eklenir (grab o zaman başlar). Protokol yoksa (sway vb.)
    /// `None`: hiçbir şey gönderilmez, mevcut davranış korunur.
    pub fn focus_grab(&mut self, surfaces: &[SurfaceId]) -> Option<FocusGrabId> {
        self.focus_grabs.manager.as_ref()?;
        let id = FocusGrabId(self.next_id());
        let known = self.known_surfaces(surfaces);
        self.focus_grabs.book.insert(id, &known);
        tracing::debug!(id = id.0, surfaces = known.len(), "odak kapma kuruldu");
        self.sync_focus_grab(id);
        Some(id)
    }

    /// Grab'ın beyaz listesini değiştirir (fark gönderilir, tek commit). Boş liste grab'ı
    /// bitirir ama kimlik geçerli kalır; `cleared` sonrası yeniden kurmak için de kullanılır.
    /// Bilinmeyen kimlik yok sayılır.
    pub fn focus_grab_set_surfaces(&mut self, id: FocusGrabId, surfaces: &[SurfaceId]) {
        let known = self.known_surfaces(surfaces);
        if !self.focus_grabs.book.set_wanted(id, &known) {
            tracing::debug!(id = id.0, "bilinmeyen odak kapma");
            return;
        }
        self.sync_focus_grab(id);
    }

    /// Grab'ı bırakır (nesne yok edilir, `cleared` gelmez). Bilinmeyen kimlik yok sayılır.
    pub fn focus_grab_release(&mut self, id: FocusGrabId) {
        let ops = self.focus_grabs.book.release(id);
        self.apply_focus_grab_ops(id, &ops);
        tracing::debug!(id = id.0, "odak kapma bırakıldı");
    }

    /// Grab compositor'da etkin (en az bir yüzey commit'lendi, `cleared` gelmedi).
    pub fn focus_grab_active(&self, id: FocusGrabId) -> bool {
        self.focus_grabs.book.is_active(id)
    }

    fn known_surfaces(&self, surfaces: &[SurfaceId]) -> Vec<SurfaceId> {
        let known: Vec<SurfaceId> = surfaces
            .iter()
            .copied()
            .filter(|s| self.surfaces.contains_key(s))
            .collect();
        if known.len() != surfaces.len() {
            tracing::debug!(
                skipped = surfaces.len() - known.len(),
                "odak kapma: bilinmeyen yüzeyler atlandı"
            );
        }
        known
    }

    fn sync_focus_grab(&mut self, id: FocusGrabId) {
        let surfaces = &self.surfaces;
        let can_create = self.focus_grabs.manager.is_some();
        let ops = self.focus_grabs.book.sync(
            id,
            |s| surfaces.get(&s).is_some_and(|e| e.has_buffer),
            can_create,
        );
        self.apply_focus_grab_ops(id, &ops);
    }

    fn apply_focus_grab_ops(&mut self, id: FocusGrabId, ops: &[GrabOp]) {
        for &op in ops {
            match op {
                GrabOp::Create(generation) => {
                    let Some((manager, _)) = self.focus_grabs.manager.as_ref() else {
                        continue;
                    };
                    let grab = manager.create_grab(&self.qh, GrabData { id, generation });
                    if let Some(old) = self.focus_grabs.objects.insert(id, grab) {
                        old.destroy();
                    }
                }
                GrabOp::Add(s) | GrabOp::Remove(s) => {
                    let (Some(grab), Some(entry)) =
                        (self.focus_grabs.objects.get(&id), self.surfaces.get(&s))
                    else {
                        continue;
                    };
                    let wl = entry.role.wl_surface();
                    if matches!(op, GrabOp::Add(_)) {
                        grab.add_surface(wl);
                    } else {
                        grab.remove_surface(wl);
                    }
                }
                GrabOp::Commit => {
                    if let Some(grab) = self.focus_grabs.objects.get(&id) {
                        grab.commit();
                    }
                }
                GrabOp::Destroy => {
                    if let Some(grab) = self.focus_grabs.objects.remove(&id) {
                        grab.destroy();
                    }
                }
            }
        }
        if !ops.is_empty() {
            tracing::debug!(
                id = id.0,
                steps = ops.len(),
                active = self.focus_grabs.book.is_active(id),
                "odak kapma eşitlendi"
            );
        }
    }

    /// Yüzey yok edilmeden (`surfaces`'tan çıkarılmadan) ÖNCE: grab'lardan çıkar, commit'le.
    pub(crate) fn focus_grab_forget_surface(&mut self, surface: SurfaceId) {
        for id in self.focus_grabs.book.forget_surface(surface) {
            self.sync_focus_grab(id);
        }
    }

    /// Yüzey ilk tamponunu commit'ledi: onu bekleyen grab'lar eşitlenir.
    pub(crate) fn focus_grab_surface_mapped(&mut self, surface: SurfaceId) {
        for id in self.focus_grabs.book.ids_wanting(surface) {
            self.sync_focus_grab(id);
        }
    }

    /// Kapanış: tüm grab nesneleri yüzeylerden önce yok edilir.
    pub(crate) fn focus_grab_shutdown(&mut self) {
        for id in self.focus_grabs.book.ids() {
            let ops = self.focus_grabs.book.release(id);
            self.apply_focus_grab_ops(id, &ops);
        }
    }

    fn focus_grab_cleared_event(&mut self, id: FocusGrabId, generation: u64) {
        if !self.focus_grabs.book.cleared(id, generation) {
            tracing::debug!(
                id = id.0,
                "eski odak kapma nesnesinden cleared; yok sayıldı"
            );
            return;
        }
        if let Some(grab) = self.focus_grabs.objects.remove(&id) {
            grab.destroy();
        }
        tracing::debug!(id = id.0, "odak kapma compositor tarafından bitirildi");
        self.pending.push(RuntimeEvent::FocusGrabCleared(id));
    }
}

/// Grab nesnesinin verisi: kimlik ve kuşak.
#[derive(Debug)]
pub(crate) struct GrabData {
    id: FocusGrabId,
    generation: u64,
}

impl<A: App> Dispatch2<HyprlandFocusGrabV1, State<A>> for GrabData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &HyprlandFocusGrabV1,
        event: hyprland_focus_grab_v1::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        // Tek olay: `cleared`.
        let hyprland_focus_grab_v1::Event::Cleared = event;
        state.cx.focus_grab_cleared_event(self.id, self.generation);
    }
}

/// Yöneticinin olayı yok.
impl<D> Dispatch2<HyprlandFocusGrabManagerV1, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &HyprlandFocusGrabManagerV1,
        _: <HyprlandFocusGrabManagerV1 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// Yönetici global'inin çalışma sırasında ilanı/geri çekilmesi (açılıştaki ilan
/// `run_with`'te bağlanır).
#[derive(Debug)]
pub(crate) struct FocusGrabGlobal;

impl FocusGrabGlobal {
    /// Global listesinde varsa bağlar.
    pub(crate) fn bind<A: App>(state: &mut State<A>, qh: &QueueHandle<State<A>>, name: u32) {
        if state.cx.focus_grabs.manager.is_some() {
            return;
        }
        match state
            .registry()
            .bind_specific::<HyprlandFocusGrabManagerV1, _, _>(qh, name, 1..=1, NoEvents)
        {
            Ok(manager) => {
                tracing::debug!("hyprland_focus_grab_manager_v1 bağlandı");
                state.cx.focus_grabs.manager = Some((manager, name));
                // Yönetici yokken istenmiş grab'lar şimdi kurulabilir.
                for id in state.cx.focus_grabs.book.ids() {
                    state.cx.sync_focus_grab(id);
                }
            }
            Err(e) => tracing::debug!("hyprland_focus_grab_manager_v1 bağlanamadı: {e}"),
        }
    }
}

impl<A: App> RegistryHandler<State<A>> for FocusGrabGlobal {
    fn new_global(
        state: &mut State<A>,
        _: &Connection,
        qh: &QueueHandle<State<A>>,
        name: u32,
        interface: &str,
        _: u32,
    ) {
        if interface == MANAGER_INTERFACE {
            Self::bind(state, qh, name);
        }
    }

    fn remove_global(
        state: &mut State<A>,
        _: &Connection,
        _: &QueueHandle<State<A>>,
        name: u32,
        _: &str,
    ) {
        // Canlı grab nesneleri kalır (istekleri geçerli; `release` yine çalışır); yeni grab
        // `None` döner.
        if state
            .cx
            .focus_grabs
            .manager
            .as_ref()
            .is_some_and(|(_, n)| *n == name)
            && let Some((manager, _)) = state.cx.focus_grabs.manager.take()
        {
            manager.destroy();
            tracing::info!("hyprland_focus_grab_manager_v1 geri çekildi");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: SurfaceId = SurfaceId(1);
    const B: SurfaceId = SurfaceId(2);
    const C: SurfaceId = SurfaceId(3);
    const G: FocusGrabId = FocusGrabId(10);

    fn all(_: SurfaceId) -> bool {
        true
    }

    #[test]
    fn first_commit_has_only_primary() {
        let mut book = GrabBook::default();
        book.insert(G, &[A, B, A, C]);
        let ops = book.sync(G, all, true);
        assert_eq!(
            ops,
            vec![
                GrabOp::Create(1),
                GrabOp::Add(A),
                GrabOp::Commit,
                GrabOp::Add(B),
                GrabOp::Add(C),
                GrabOp::Commit,
            ]
        );
        assert!(book.is_active(G));
        // Değişiklik yoksa adım yok.
        assert!(book.sync(G, all, true).is_empty());
    }

    #[test]
    fn without_manager_nothing_is_sent() {
        let mut book = GrabBook::default();
        book.insert(G, &[A]);
        assert!(book.sync(G, all, false).is_empty());
        assert!(!book.is_active(G));
        // Yönetici sonradan gelirse kurulur.
        assert_eq!(book.sync(G, all, true)[0], GrabOp::Create(1));
    }

    #[test]
    fn unmapped_surfaces_wait_for_first_buffer() {
        let mut book = GrabBook::default();
        book.insert(G, &[A, B]);
        assert!(
            book.sync(G, |_| false, true).is_empty(),
            "map yokken nesne yok"
        );
        assert_eq!(
            book.sync(G, |s| s == B, true),
            vec![GrabOp::Create(1), GrabOp::Add(B), GrabOp::Commit]
        );
        assert_eq!(
            book.sync(G, all, true),
            vec![GrabOp::Add(A), GrabOp::Commit]
        );
    }

    #[test]
    fn set_surfaces_sends_diff_in_one_commit() {
        let mut book = GrabBook::default();
        book.insert(G, &[A, B]);
        book.sync(G, all, true);
        book.set_wanted(G, &[A, C]);
        assert_eq!(
            book.sync(G, all, true),
            vec![GrabOp::Remove(B), GrabOp::Add(C), GrabOp::Commit]
        );
    }

    #[test]
    fn emptying_destroys_instead_of_empty_commit() {
        let mut book = GrabBook::default();
        book.insert(G, &[A]);
        book.sync(G, all, true);
        book.set_wanted(G, &[]);
        assert_eq!(book.sync(G, all, true), vec![GrabOp::Destroy]);
        assert!(!book.is_active(G));
        assert!(book.contains(G), "kimlik geçerli kalır");
        // Yeniden kurulum yeni kuşak.
        book.set_wanted(G, &[B]);
        assert_eq!(book.sync(G, all, true)[0], GrabOp::Create(2));
        // Eski kuşağın geç cleared'ı yok sayılır.
        assert!(!book.cleared(G, 1));
        assert!(book.is_active(G));
    }

    #[test]
    fn destroyed_surface_is_removed_while_alive() {
        let mut book = GrabBook::default();
        book.insert(G, &[A, B]);
        book.sync(G, all, true);
        assert_eq!(book.forget_surface(B), vec![G]);
        assert_eq!(
            book.sync(G, all, true),
            vec![GrabOp::Remove(B), GrabOp::Commit]
        );
        // Son yüzey giderse nesne yok edilir (cleared beklenmez).
        assert_eq!(book.forget_surface(A), vec![G]);
        assert_eq!(book.sync(G, all, true), vec![GrabOp::Destroy]);
        // İlgisiz yüzey hiçbir grab'a dokunmaz.
        assert!(book.forget_surface(C).is_empty());
    }

    #[test]
    fn cleared_is_routed_once_and_does_not_rearm() {
        let mut book = GrabBook::default();
        book.insert(G, &[A]);
        book.sync(G, all, true);
        let generation = book.generation(G).unwrap();
        assert!(book.cleared(G, generation));
        assert!(!book.is_active(G));
        assert!(!book.cleared(G, generation), "ikinci kez iletilmez");
        // Kendiliğinden yeniden kurulmaz (dışarı tıklama geri alınmaz).
        assert!(book.sync(G, all, true).is_empty());
        assert!(!book.cleared(FocusGrabId(99), 1), "bilinmeyen kimlik");
    }

    #[test]
    fn release_destroys_only_live_object() {
        let mut book = GrabBook::default();
        book.insert(G, &[A]);
        assert!(book.release(G).is_empty(), "nesne yokken istek yok");
        book.insert(G, &[A]);
        book.sync(G, all, true);
        assert_eq!(book.release(G), vec![GrabOp::Destroy]);
        assert!(!book.contains(G));
        assert!(book.release(G).is_empty());
    }
}
