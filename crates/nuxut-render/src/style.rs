use nuxut_core::{Color, Corners, Point, Rect};

/// Dikdörtgen kenarlığı (içe doğru çizilir; dış sınır `rect`'in kendisidir).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Color,
}

/// Yuvarlatılmış dikdörtgen stili.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectStyle {
    pub fill: Color,
    pub radii: Corners,
    pub border: Option<Border>,
}

impl RectStyle {
    pub fn filled(fill: Color, radii: Corners) -> Self {
        Self { fill, radii, border: None }
    }
}

/// Bulanık gölge. Gölge şekli, gölgeyi düşüren şeklin aynısıdır (aynı SDF).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowStyle {
    /// Düz (straight) alpha; opaklık `color.a` içindedir.
    pub color: Color,
    /// Bulanıklık yarıçapı (mantıksal piksel, ≈ 2σ).
    pub blur: f32,
    pub offset: Point,
    /// Şekli bulanıklaştırmadan önce genişletir (negatif = daraltır).
    pub spread: f32,
}

impl ShadowStyle {
    /// Gölgenin boyandığı en dış sınır (sınır kutusu hesabı için).
    pub fn bounds(&self, rect: Rect) -> Rect {
        rect.translate(self.offset).expand(self.spread.max(0.0) + self.blur * 1.5 + 1.0)
    }
}
