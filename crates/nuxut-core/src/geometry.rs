//! Mantıksal (logical) koordinatlarda geometri. Piksel dönüşümü render katmanının işidir;
//! burada her şey `f32` ve ölçekten bağımsızdır.

use std::ops::{Add, Mul, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

impl Add for Point {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Point {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Point {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const ZERO: Self = Self::new(0.0, 0.0);

    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn from_origin_size(origin: Point, size: Size) -> Self {
        Self::new(origin.x, origin.y, size.width, size.height)
    }

    pub fn origin(&self) -> Point {
        Point::new(self.x, self.y)
    }

    pub fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width * 0.5, self.y + self.height * 0.5)
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Her kenardan içe çeker; negatif değerler dışa genişletir.
    pub fn inset(&self, insets: Insets) -> Self {
        Self::new(
            self.x + insets.left,
            self.y + insets.top,
            (self.width - insets.left - insets.right).max(0.0),
            (self.height - insets.top - insets.bottom).max(0.0),
        )
    }

    pub fn expand(&self, amount: f32) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.width + amount * 2.0,
            self.height + amount * 2.0,
        )
    }

    pub fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        Self::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }

    pub fn translate(&self, d: Point) -> Self {
        Self::new(self.x + d.x, self.y + d.y, self.width, self.height)
    }

    /// Dış sınırları tam piksele genişletir (hasar bölgesi, input bölgesi gibi tam sayı
    /// isteyen yerler için).
    pub fn round_out(&self) -> Self {
        let x = self.x.floor();
        let y = self.y.floor();
        Self::new(x, y, self.right().ceil() - x, self.bottom().ceil() - y)
    }

    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        Self::new(
            lerp(self.x, to.x, t),
            lerp(self.y, to.y, t),
            lerp(self.width, to.width, t),
            lerp(self.height, to.height, t),
        )
    }
}

/// Kenar başına kalınlık/boşluk.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Insets {
    pub const ZERO: Self = Self::uniform(0.0);

    pub const fn uniform(v: f32) -> Self {
        Self { top: v, right: v, bottom: v, left: v }
    }

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self { top, right, bottom, left }
    }

    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }

    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        Self::new(
            lerp(self.top, to.top, t),
            lerp(self.right, to.right, t),
            lerp(self.bottom, to.bottom, t),
            lerp(self.left, to.left, t),
        )
    }
}

/// Köşe başına yarıçap. Sıra CSS ile aynı: sol-üst, sağ-üst, sağ-alt, sol-alt.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Corners {
    pub top_left: f32,
    pub top_right: f32,
    pub bottom_right: f32,
    pub bottom_left: f32,
}

impl Corners {
    pub const ZERO: Self = Self::uniform(0.0);

    pub const fn uniform(r: f32) -> Self {
        Self { top_left: r, top_right: r, bottom_right: r, bottom_left: r }
    }

    pub const fn new(top_left: f32, top_right: f32, bottom_right: f32, bottom_left: f32) -> Self {
        Self { top_left, top_right, bottom_right, bottom_left }
    }

    /// Yarıçapları dikdörtgene sığacak şekilde kırpar (her yarıçap kısa kenarın yarısını
    /// aşamaz). Shader'a giden her değer buradan geçmeli; aksi halde SDF bozulur.
    pub fn clamped(&self, size: Size) -> Self {
        let max = (size.width.min(size.height) * 0.5).max(0.0);
        Self::new(
            self.top_left.clamp(0.0, max),
            self.top_right.clamp(0.0, max),
            self.bottom_right.clamp(0.0, max),
            self.bottom_left.clamp(0.0, max),
        )
    }

    pub fn to_array(&self) -> [f32; 4] {
        [self.top_left, self.top_right, self.bottom_right, self.bottom_left]
    }
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_never_goes_negative() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(Insets::uniform(8.0));
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }

    #[test]
    fn corners_clamp_to_half_short_side() {
        let c = Corners::uniform(50.0).clamped(Size::new(40.0, 20.0));
        assert_eq!(c, Corners::uniform(10.0));
    }

    #[test]
    fn round_out_covers_fractional_edges() {
        let r = Rect::new(0.4, 1.6, 10.2, 3.0).round_out();
        assert_eq!(r, Rect::new(0.0, 1.0, 11.0, 4.0));
    }
}
