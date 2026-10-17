//! ADR-0008 köşe geometrisi sahneleri: altın görüntü testi (`tests/it/blob_golden.rs`) ve izole
//! sway görsel örneği (`nuxut-wayland/examples/blob_scenes.rs`, `#[path]` ile) AYNI tanımı
//! kullanır. Blob API'si değişince yalnız bu dosya uyarlanır.
//!
//! Çerçeve: üst bar 32, diğer kenarlar 8 (mantıksal). Panel boyutları mutlaktır, konumları iç
//! dikdörtgene göredir: sahne her viewport'ta (altın: 480×300; örnek: tam ekran) aynı biçimi
//! verir. `open` açılış ilerlemesi (0 kapalı, 1 açık; yay taşmasında 1'i aşar): kenara yapışık
//! panel o kenardan büyür; d taraması ve tohum sahnelerinde kullanılmaz (geometrileri sabittir).

#![allow(dead_code)]

use nuxut_core::{Color, Rect, Size};
use nuxut_render::{BlobField, BlobShape, FrameShape, ShadowStyle};

/// Sahne token'ları. Altın görüntüler `Theme::default()`'tan (kullanıcı config'inden değil).
#[derive(Debug, Clone, Copy)]
pub struct SceneParams {
    /// Mantıksal viewport.
    pub size: Size,
    pub bar: f32,
    pub edge: f32,
    /// Birleşim yarıçapı R (`theme.blob.smoothing`).
    pub smoothing: f32,
    /// Panel köşe yarıçapı r (`theme.shape.radius_large`).
    pub radius: f32,
    pub fill: Color,
    pub shadow: Option<ShadowStyle>,
}

impl SceneParams {
    pub const BAR: f32 = 32.0;
    pub const EDGE: f32 = 8.0;

    /// Gölgesiz (yalnız geometri) parametreler.
    pub fn from_theme(theme: &nuxut_theme::Theme, size: Size) -> Self {
        Self {
            size,
            bar: Self::BAR,
            edge: Self::EDGE,
            smoothing: theme.blob.smoothing,
            radius: theme.shape.radius_large,
            fill: theme.colors.surface,
            shadow: None,
        }
    }

    fn inner(&self) -> Rect {
        Rect::new(
            self.edge,
            self.bar,
            (self.size.width - 2.0 * self.edge).max(0.0),
            (self.size.height - self.bar - self.edge).max(0.0),
        )
    }

    fn field(&self, shapes: Vec<BlobShape>) -> BlobField {
        BlobField {
            frame: Some(FrameShape {
                outer: Rect::new(0.0, 0.0, self.size.width, self.size.height),
                inner: self.inner(),
                // ADR-0008: iç köşeler R; alan yok sayar (uyumluluk).
                inner_radius: self.smoothing,
            }),
            shapes,
            smoothing: self.smoothing,
            corner_radius: self.radius,
            fill: self.fill,
            shadow: self.shadow,
        }
    }
}

/// Panel boyutları (mantıksal).
const PANEL_W: f32 = 200.0;
const PANEL_H: f32 = 120.0;
const SIDE_W: f32 = 140.0;

/// Tüm sahne adları, sırayla (altın dosya adı: `<ad>@<ölçek>.png`).
pub fn names(p: &SceneParams) -> Vec<String> {
    scenes(p, 1.0).into_iter().map(|(name, _)| name).collect()
}

/// Ada göre tek sahne.
pub fn scene(p: &SceneParams, name: &str, open: f32) -> Option<BlobField> {
    scenes(p, open)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, f)| f)
}

/// Bütün sahneler: (ad, alan).
pub fn scenes(p: &SceneParams, open: f32) -> Vec<(String, BlobField)> {
    // Yay 1'i aşabilir (puding taşması); yalnız negatif kesilir.
    let open = open.max(0.0);
    let inner = p.inner();
    let big_r = p.smoothing;
    let cx = inner.x + (inner.width - PANEL_W) * 0.5;
    let mut out = Vec::new();

    // (1) Üst panel: üst bara yapışık, aşağı büyür. Alt köşeler boşluğa açılır.
    let h = PANEL_H * open;
    out.push((
        "top".into(),
        p.field(vec![BlobShape::new_rect(Rect::new(
            cx, inner.y, PANEL_W, h,
        ))]),
    ));

    // (2) Sağda tam yükseklik panel: sağ duvar + bar + alt duvar; sağdan sola büyür.
    let w = SIDE_W * open;
    out.push((
        "right-full".into(),
        p.field(vec![BlobShape::new_rect(Rect::new(
            inner.right() - w,
            inner.y,
            w,
            inner.height,
        ))]),
    ));

    // (3) Yalnız üste yapışık sağ panel: sağ duvardan 3R aralıklı, aşağı büyür.
    let x = inner.right() - 3.0 * big_r - SIDE_W;
    out.push((
        "right-top".into(),
        p.field(vec![BlobShape::new_rect(Rect::new(
            x,
            inner.y,
            SIDE_W,
            PANEL_H * open,
        ))]),
    ));

    // (4) Alta yapışık panel: alt duvardan yukarı büyür.
    let h = PANEL_H * open;
    out.push((
        "bottom".into(),
        p.field(vec![BlobShape::new_rect(Rect::new(
            cx,
            inner.bottom() - h,
            PANEL_W,
            h,
        ))]),
    ));

    // (5) Köşe paneli (üst-sağ): bar + sağ duvar; boşluğa tek köşe (sol-alt) açılır.
    out.push((
        "corner".into(),
        p.field(vec![BlobShape::new_rect(Rect::new(
            inner.right() - SIDE_W,
            inner.y,
            SIDE_W,
            PANEL_H * open,
        ))]),
    ));

    // (6) d taraması: bara yapışık panelin alt kenarı alt duvardan d uzakta.
    for (label, k) in [
        ("0", 0.0),
        ("0.5R", 0.5),
        ("1R", 1.0),
        ("1.5R", 1.5),
        ("2R", 2.0),
        ("3R", 3.0),
    ] {
        let d = k * big_r;
        out.push((
            format!("dsweep-{label}"),
            p.field(vec![BlobShape::new_rect(Rect::new(
                cx,
                inner.y,
                PANEL_W,
                (inner.height - d).max(0.0),
            ))]),
        ));
    }

    // (7) Tohumdan büyüme: bara yapışık panelin kalınlığı 0, 1, 4, 16 px.
    for t in [0.0f32, 1.0, 4.0, 16.0] {
        out.push((
            format!("seed-{t}"),
            p.field(vec![BlobShape::new_rect(Rect::new(
                cx, inner.y, PANEL_W, t,
            ))]),
        ));
    }
    out
}
