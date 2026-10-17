//! Pencere dışa aktarma (xdg-foreign-unstable-v2): başka bir sürecin (ör. xdg-desktop-portal
//! dosya seçicisi) diyaloğunu bu pencerenin üstüne ebeveynli açabilmesi için `xdg_toplevel`
//! tanıtıcısı. Portal `parent_window` olarak `wayland:<tanıtıcı>` bekler.
//!
//! Tanıtıcı, dışa aktarılan nesne yaşadıkça geçerlidir: yüzey başına bir kez dışa aktarılır,
//! yüzey kaldırılınca (`destroy_surface`) ya da `unexport_toplevel` ile bırakılır. Layer-shell
//! ve kilit yüzeyleri dışa aktarılamaz (protokol yalnız `xdg_toplevel` kabul eder); onlarda
//! portal ebeveynsiz (`""`) çağrılır. Tanıtıcı loglanmaz (başka istemciye yetki verir),
//! `ToplevelHandle`'ın Debug'ı redakte.
//!
//! Ters yön (içe aktarma): portal arka ucu olan bir uygulama (Nuxut dosya seçicisi) aldığı
//! `wayland:<tanıtıcı>`'yı `set_window_parent` ile kendi penceresinin ebeveyni yapar; compositor
//! pencereyi ebeveynin üstünde yığar (Hyprland ortalar). Ebeveyn kapanınca (`destroyed`) ilişki
//! düşer, pencere ebeveynsiz kalır.

use std::collections::HashMap;

use smithay_client_toolkit::dispatch2::Dispatch2;
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_exported_v2::{self, ZxdgExportedV2};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_exporter_v2::ZxdgExporterV2;
use wayland_protocols::xdg::foreign::zv2::client::zxdg_imported_v2::{self, ZxdgImportedV2};
use wayland_protocols::xdg::foreign::zv2::client::zxdg_importer_v2::ZxdgImporterV2;

use crate::App;
use crate::context::{Context, RuntimeEvent};
use crate::state::{NoEvents, State};
use crate::surface::Role;
use crate::types::SurfaceId;

/// `export_toplevel` isteğinin kimliği; sonuç `App::toplevel_exported` ile gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExportRequestId(pub(crate) u64);

/// Compositor'ın verdiği dışa aktarma tanıtıcısı. Debug redakte (tanıtıcıyı bilen istemci
/// kendi yüzeyini bu pencerenin çocuğu yapabilir).
#[derive(Clone, PartialEq, Eq)]
pub struct ToplevelHandle(String);

impl ToplevelHandle {
    /// Ham tanıtıcı.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// xdg-desktop-portal `parent_window` biçimi: `wayland:<tanıtıcı>`.
    pub fn window_identifier(&self) -> String {
        format!("wayland:{}", self.0)
    }
}

impl std::fmt::Debug for ToplevelHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ToplevelHandle(<gizli>)")
    }
}

/// Yüzey başına canlı dışa aktarma.
#[derive(Debug)]
struct Export {
    exported: ZxdgExportedV2,
    /// `handle` olayı geldiyse.
    handle: Option<ToplevelHandle>,
    /// Tanıtıcıyı bekleyen istekler.
    waiting: Vec<ExportRequestId>,
}

/// Yüzey başına canlı içe aktarma (yabancı ebeveyn).
#[derive(Debug)]
struct Import {
    imported: ZxdgImportedV2,
    /// `destroyed` olayının eski bir içe aktarmaya ait olup olmadığını ayırır.
    serial: u64,
}

/// Bağlı `zxdg_exporter_v2`/`zxdg_importer_v2`, canlı dışa ve içe aktarmalar.
#[derive(Debug, Default)]
pub(crate) struct Foreign {
    pub exporter: Option<ZxdgExporterV2>,
    pub importer: Option<ZxdgImporterV2>,
    exports: HashMap<SurfaceId, Export>,
    imports: HashMap<SurfaceId, Import>,
}

impl Foreign {
    /// Yüzey gidiyor: dışa aktarmayı ve yabancı ebeveyni bırakır; bekleyen istekler `None` alır.
    pub(crate) fn forget(&mut self, surface: SurfaceId, pending: &mut Vec<RuntimeEvent>) {
        if let Some(export) = self.exports.remove(&surface) {
            export.exported.destroy();
            for id in export.waiting {
                pending.push(RuntimeEvent::ToplevelExported(id, surface, None));
            }
        }
        self.drop_import(surface);
    }

    fn drop_import(&mut self, surface: SurfaceId) {
        if let Some(import) = self.imports.remove(&surface) {
            import.imported.destroy();
        }
    }
}

impl<A: App> Context<A> {
    /// Pencereyi (`create_window`) xdg-foreign-v2 ile dışa aktarır. Sonuç her istek için tam
    /// bir kez `App::toplevel_exported` ile, bu çağrının içinden değil, gelir. Yüzey zaten dışa
    /// aktarıldıysa aynı tanıtıcı döner. `None`: protokol yok, yüzey yok ya da pencere değil
    /// (layer-shell/kilit yüzeyi) — portal ebeveynsiz çağrılmalı.
    pub fn export_toplevel(&mut self, surface: SurfaceId) -> ExportRequestId {
        let id = ExportRequestId(self.next_id());
        if let Some(export) = self.foreign.exports.get_mut(&surface) {
            match &export.handle {
                Some(handle) => {
                    let handle = handle.clone();
                    self.pending
                        .push(RuntimeEvent::ToplevelExported(id, surface, Some(handle)));
                }
                None => export.waiting.push(id),
            }
            return id;
        }
        let window = self
            .surfaces
            .get(&surface)
            .and_then(|entry| match &entry.role {
                Role::Window { window, .. } => Some(window),
                _ => None,
            });
        let (Some(exporter), Some(window)) = (self.foreign.exporter.as_ref(), window) else {
            tracing::debug!(id = id.0, "pencere dışa aktarılmadı (protokol/yüzey/rol)");
            self.pending
                .push(RuntimeEvent::ToplevelExported(id, surface, None));
            return id;
        };
        use smithay_client_toolkit::shell::WaylandSurface;
        let exported = exporter.export_toplevel(window.wl_surface(), &self.qh, ExportData(surface));
        self.foreign.exports.insert(
            surface,
            Export {
                exported,
                handle: None,
                waiting: vec![id],
            },
        );
        tracing::debug!(id = id.0, "pencere dışa aktarılıyor");
        id
    }

    /// Dışa aktarmayı bırakır (tanıtıcı geçersizleşir). Yüzey kaldırılınca kendiliğinden olur.
    pub fn unexport_toplevel(&mut self, surface: SurfaceId) {
        self.foreign.forget(surface, &mut self.pending);
    }

    /// Başka bir istemcinin dışa aktardığı pencereyi (`handle`: portalın `wayland:<h>`
    /// biçimindeki `parent_window`'undan önek atılmış ham tanıtıcı) bu pencerenin (`create_window`)
    /// ebeveyni yapar; `xdg_toplevel.set_parent` ile aynı yığın/konum anlamı. Önceki yabancı
    /// ebeveyn bırakılır. `false`: protokol yok, yüzey pencere değil ya da tanıtıcı boş —
    /// pencere ebeveynsiz kalır. Geçersiz tanıtıcıyı compositor `destroyed` ile bildirir; ebeveyn
    /// kapanınca da aynısı olur ve ilişki sessizce düşer. Tanıtıcı loglanmaz.
    pub fn set_window_parent(&mut self, surface: SurfaceId, handle: &str) -> bool {
        self.foreign.drop_import(surface);
        if handle.is_empty() {
            return false;
        }
        let serial = self.next_id();
        let window = self
            .surfaces
            .get(&surface)
            .and_then(|entry| match &entry.role {
                Role::Window { window, .. } => Some(window),
                _ => None,
            });
        let (Some(importer), Some(window)) = (self.foreign.importer.as_ref(), window) else {
            tracing::debug!("yabancı ebeveyn kurulmadı (protokol/yüzey/rol)");
            return false;
        };
        use smithay_client_toolkit::shell::WaylandSurface;
        let imported = importer.import_toplevel(
            handle.to_owned(),
            &self.qh,
            ImportData { surface, serial },
        );
        imported.set_parent_of(window.wl_surface());
        self.foreign
            .imports
            .insert(surface, Import { imported, serial });
        tracing::debug!("yabancı ebeveyn kuruldu");
        true
    }

    /// Yabancı ebeveyni bırakır (pencere ebeveynsiz kalır). Yoksa no-op.
    pub fn clear_window_parent(&mut self, surface: SurfaceId) {
        self.foreign.drop_import(surface);
    }

    /// Pencerenin canlı bir yabancı ebeveyni var mı (`destroyed` gelmediyse).
    pub fn has_window_parent(&self, surface: SurfaceId) -> bool {
        self.foreign.imports.contains_key(&surface)
    }

    fn imported_destroyed(&mut self, surface: SurfaceId, serial: u64) {
        if self
            .foreign
            .imports
            .get(&surface)
            .is_some_and(|import| import.serial == serial)
        {
            tracing::debug!("yabancı ebeveyn düştü");
            self.foreign.drop_import(surface);
        }
    }

    fn toplevel_handle_arrived(&mut self, surface: SurfaceId, handle: String) {
        let Some(export) = self.foreign.exports.get_mut(&surface) else {
            return;
        };
        let handle = ToplevelHandle(handle);
        export.handle = Some(handle.clone());
        for id in std::mem::take(&mut export.waiting) {
            self.pending.push(RuntimeEvent::ToplevelExported(
                id,
                surface,
                Some(handle.clone()),
            ));
        }
    }
}

/// Dışa aktarılan nesnenin verisi: hangi yüzeye ait olduğu.
#[derive(Debug)]
pub(crate) struct ExportData(SurfaceId);

impl<A: App> Dispatch2<ZxdgExportedV2, State<A>> for ExportData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ZxdgExportedV2,
        event: zxdg_exported_v2::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        if let zxdg_exported_v2::Event::Handle { handle } = event {
            tracing::debug!("pencere tanıtıcısı geldi");
            state.cx.toplevel_handle_arrived(self.0, handle);
        }
    }
}

/// İçe aktarılan nesnenin verisi: hangi yüzeyin ebeveyni olduğu ve hangi içe aktarma.
#[derive(Debug)]
pub(crate) struct ImportData {
    surface: SurfaceId,
    serial: u64,
}

impl<A: App> Dispatch2<ZxdgImportedV2, State<A>> for ImportData {
    fn event(
        &self,
        state: &mut State<A>,
        _: &ZxdgImportedV2,
        event: zxdg_imported_v2::Event,
        _: &Connection,
        _: &QueueHandle<State<A>>,
    ) {
        if let zxdg_imported_v2::Event::Destroyed = event {
            state.cx.imported_destroyed(self.surface, self.serial);
        }
    }
}

/// `zxdg_importer_v2`'nin olayı yok.
impl<D> Dispatch2<ZxdgImporterV2, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &ZxdgImporterV2,
        _: <ZxdgImporterV2 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

/// `zxdg_exporter_v2`'nin olayı yok.
impl<D> Dispatch2<ZxdgExporterV2, D> for NoEvents {
    fn event(
        &self,
        _: &mut D,
        _: &ZxdgExporterV2,
        _: <ZxdgExporterV2 as Proxy>::Event,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_debug_is_redacted() {
        let handle = ToplevelHandle("gizli-tanitici-42".into());
        let shown = format!("{handle:?}");
        assert!(!shown.contains("gizli-tanitici"), "{shown}");
        assert_eq!(handle.as_str(), "gizli-tanitici-42");
        assert_eq!(handle.window_identifier(), "wayland:gizli-tanitici-42");
    }
}
