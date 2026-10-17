use nuxut_core::{Rect, Size};

/// Çapraz eksende hizalama.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

/// Yatay dizilim: çocuklar soldan sağa, aralarında `spacing` boşlukla. Boşluk
/// `Theme::space(n)` ile verilir (sabit yok).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Row {
    pub spacing: f32,
    pub align: Align,
}

impl Row {
    pub fn new(spacing: f32, align: Align) -> Self {
        Self { spacing, align }
    }

    /// Toplam boyut (genişlik toplamı + boşluklar, yükseklik en büyüğü).
    pub fn measure(&self, sizes: &[Size]) -> Size {
        let width: f32 = sizes.iter().map(|s| s.width).sum::<f32>()
            + self.spacing * sizes.len().saturating_sub(1) as f32;
        let height = sizes.iter().map(|s| s.height).fold(0.0, f32::max);
        Size::new(width, height)
    }

    /// Çocukları `bounds` içine soldan dizer; dikeyde `align`'a göre hizalar.
    pub fn arrange(&self, bounds: Rect, sizes: &[Size]) -> Vec<Rect> {
        let mut x = bounds.x;
        sizes
            .iter()
            .map(|s| {
                let y = match self.align {
                    Align::Start => bounds.y,
                    Align::Center => bounds.y + (bounds.height - s.height) * 0.5,
                    Align::End => bounds.bottom() - s.height,
                };
                let r = Rect::new(x, y, s.width, s.height);
                x += s.width + self.spacing;
                r
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arranges_with_spacing_and_centers() {
        let row = Row::new(8.0, Align::Center);
        let sizes = [Size::new(10.0, 10.0), Size::new(20.0, 30.0)];
        assert_eq!(row.measure(&sizes), Size::new(38.0, 30.0));
        let rects = row.arrange(Rect::new(0.0, 0.0, 100.0, 30.0), &sizes);
        assert_eq!(rects[0], Rect::new(0.0, 10.0, 10.0, 10.0));
        assert_eq!(rects[1], Rect::new(18.0, 0.0, 20.0, 30.0));
    }

    #[test]
    fn empty_row() {
        assert_eq!(Row::default().measure(&[]), Size::ZERO);
    }
}
