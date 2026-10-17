//! İmleç şekli: yüzey başına son istenen şekil, tekrarsız uygulama, enter'da yeniden uygulama.
//!
//! Uygulama sctk `ThemedPointer` ile yapılır: `wp_cursor_shape_v1` varsa `set_shape`, yoksa
//! xcursor teması (`XCURSOR_THEME`/`XCURSOR_SIZE`).

use std::collections::HashMap;

use nuxut_core::input::CursorShape;
use smithay_client_toolkit::seat::pointer::{CursorIcon, ThemedPointer};
use wayland_client::Connection;

use crate::types::SurfaceId;

/// Saf durum makinesi: hangi şeklin ne zaman protokole gitmesi gerektiğine karar verir.
#[derive(Debug, Default)]
pub(crate) struct CursorTracker {
    shapes: HashMap<SurfaceId, CursorShape>,
    /// İşaretçinin bulunduğu yüzey.
    focus: Option<SurfaceId>,
    /// Bu enter'dan beri protokole gönderilmiş son şekil.
    applied: Option<CursorShape>,
}

impl CursorTracker {
    /// Şekli saklar; işaretçi bu yüzeydeyse ve şekil değiştiyse uygulanacak şekli döndürür.
    pub fn set(&mut self, id: SurfaceId, shape: CursorShape) -> Option<CursorShape> {
        self.shapes.insert(id, shape);
        (self.focus == Some(id) && self.applied != Some(shape)).then(|| {
            self.applied = Some(shape);
            shape
        })
    }

    /// İşaretçi yüzeye girdi: compositor imleci sıfırladı, son şekil her zaman yeniden uygulanır.
    pub fn enter(&mut self, id: SurfaceId) -> CursorShape {
        let shape = self.shapes.get(&id).copied().unwrap_or_default();
        self.focus = Some(id);
        self.applied = Some(shape);
        shape
    }

    pub fn leave(&mut self, id: SurfaceId) {
        if self.focus == Some(id) {
            self.focus = None;
            self.applied = None;
        }
    }

    pub fn forget(&mut self, id: SurfaceId) {
        self.shapes.remove(&id);
        self.leave(id);
    }
}

/// İşaretçi + tema; Context'te yaşar ki `set_cursor` hemen uygulayabilsin.
#[derive(Default)]
pub(crate) struct Cursor {
    pub tracker: CursorTracker,
    pub pointer: Option<ThemedPointer>,
}

impl std::fmt::Debug for Cursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cursor")
            .field("tracker", &self.tracker)
            .finish_non_exhaustive()
    }
}

impl Cursor {
    pub fn apply(&self, conn: &Connection, shape: CursorShape) {
        let Some(pointer) = &self.pointer else { return };
        tracing::debug!(?shape, "imleç uygulanıyor");
        if let Err(e) = pointer.set_cursor(conn, to_icon(shape)) {
            tracing::debug!(?shape, "imleç ayarlanamadı: {e}");
        }
    }
}

/// `CursorShape` → CSS imleç adı (`cursor-icon`); protokol ve xcursor yolu aynı adları kullanır.
pub(crate) fn to_icon(shape: CursorShape) -> CursorIcon {
    match shape {
        CursorShape::Default => CursorIcon::Default,
        CursorShape::Pointer => CursorIcon::Pointer,
        CursorShape::Text => CursorIcon::Text,
        CursorShape::Grab => CursorIcon::Grab,
        CursorShape::Grabbing => CursorIcon::Grabbing,
        CursorShape::NotAllowed => CursorIcon::NotAllowed,
        CursorShape::Move => CursorIcon::Move,
        CursorShape::Crosshair => CursorIcon::Crosshair,
        CursorShape::Progress => CursorIcon::Progress,
        CursorShape::Wait => CursorIcon::Wait,
        CursorShape::Help => CursorIcon::Help,
        CursorShape::ColResize => CursorIcon::ColResize,
        CursorShape::RowResize => CursorIcon::RowResize,
        CursorShape::ResizeN => CursorIcon::NResize,
        CursorShape::ResizeS => CursorIcon::SResize,
        CursorShape::ResizeE => CursorIcon::EResize,
        CursorShape::ResizeW => CursorIcon::WResize,
        CursorShape::ResizeNe => CursorIcon::NeResize,
        CursorShape::ResizeNw => CursorIcon::NwResize,
        CursorShape::ResizeSe => CursorIcon::SeResize,
        CursorShape::ResizeSw => CursorIcon::SwResize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: SurfaceId = SurfaceId(1);
    const B: SurfaceId = SurfaceId(2);

    #[test]
    fn dedupe_and_store_while_elsewhere() {
        let mut t = CursorTracker::default();
        // İşaretçi yokken yalnızca saklanır.
        assert_eq!(t.set(A, CursorShape::Text), None);
        assert_eq!(t.enter(A), CursorShape::Text);
        // Aynı şekil tekrar: protokol çağrısı yok.
        assert_eq!(t.set(A, CursorShape::Text), None);
        assert_eq!(t.set(A, CursorShape::Pointer), Some(CursorShape::Pointer));
        assert_eq!(t.set(A, CursorShape::Pointer), None);
        // Başka yüzeyin şekli: saklanır, uygulanmaz.
        assert_eq!(t.set(B, CursorShape::Grab), None);
        t.leave(A);
        assert_eq!(t.set(A, CursorShape::Text), None);
    }

    #[test]
    fn enter_reapplies_last_shape() {
        let mut t = CursorTracker::default();
        assert_eq!(t.enter(A), CursorShape::Default);
        t.set(A, CursorShape::Grab);
        t.leave(A);
        // Yeniden girişte, aynı şekil olsa bile tekrar uygulanır (compositor sıfırladı).
        assert_eq!(t.enter(A), CursorShape::Grab);
        assert_eq!(t.set(A, CursorShape::Grab), None);
        t.forget(A);
        assert_eq!(t.enter(A), CursorShape::Default);
    }

    #[test]
    fn icon_names_match_xcursor_names() {
        for shape in [
            CursorShape::Default,
            CursorShape::Pointer,
            CursorShape::Text,
            CursorShape::Grab,
            CursorShape::Grabbing,
            CursorShape::NotAllowed,
            CursorShape::Move,
            CursorShape::Crosshair,
            CursorShape::Progress,
            CursorShape::Wait,
            CursorShape::Help,
            CursorShape::ColResize,
            CursorShape::RowResize,
            CursorShape::ResizeN,
            CursorShape::ResizeS,
            CursorShape::ResizeE,
            CursorShape::ResizeW,
            CursorShape::ResizeNe,
            CursorShape::ResizeNw,
            CursorShape::ResizeSe,
            CursorShape::ResizeSw,
        ] {
            assert_eq!(to_icon(shape).name(), shape.xcursor_name(), "{shape:?}");
        }
    }
}
