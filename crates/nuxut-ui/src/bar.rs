//! Ortak hap çubuğu (OSD ilerlemesi, kaydırıcı izi): tonal iz + aynı hapta dolgu, uçlar
//! yuvarlak. İsteğe bağlı %100 işareti (dolgunun arkasında, izden taşar) ve aşım tonu (aynı hap
//! işaretin ötesine kırpılıp üstüne biner; dikiş yok). Renkler çağırandan (solma uygulanmış).

use nuxut_core::{Color, Corners, Rect};
use nuxut_render::{Painter, RectStyle};

use crate::style::{HAIRLINE, pill};

/// Aşım: `mark` = %100 noktasının izin başından uzaklığı.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Overflow {
    pub mark: f32,
    pub tick: Color,
    /// Aşım bölgesinin rengi (çağıran dolgu → uyarı karışımını yapar).
    pub color: Color,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PillBar {
    pub track: Rect,
    /// `false`: dikey, dolgu alttan yukarı.
    pub horizontal: bool,
    /// Dolgunun eksen boyunca uzunluğu (0 = yok).
    pub fill: f32,
    pub rest: Color,
    pub fill_color: Color,
    pub overflow: Option<Overflow>,
}

impl PillBar {
    /// Eksen boyunca `[start, start + len)` dilimi.
    pub fn span(&self, start: f32, len: f32) -> Rect {
        let t = self.track;
        if self.horizontal {
            Rect::new(t.x + start, t.y, len, t.height)
        } else {
            Rect::new(t.x, t.bottom() - start - len, t.width, len)
        }
    }

    pub fn fill_rect(&self) -> Rect {
        self.span(0.0, self.fill)
    }

    pub fn draw(&self, painter: &mut Painter) {
        let t = self.track;
        let thick = if self.horizontal { t.height } else { t.width };
        painter.rect(t, &RectStyle::filled(self.rest, pill(t)));
        // %100 işareti dolgunun arkasında, izden taşar: dolu kısımda yalnız uçları görünür.
        if let Some(o) = self.overflow {
            let tick = if self.horizontal {
                Rect::new(t.x + o.mark - HAIRLINE * 0.5, t.y - thick * 0.5, HAIRLINE, thick * 2.0)
            } else {
                Rect::new(t.x - thick * 0.5, t.bottom() - o.mark - HAIRLINE * 0.5, thick * 2.0, HAIRLINE)
            };
            painter.rect(tick, &RectStyle::filled(o.tick, Corners::ZERO));
        }
        if self.fill <= 0.0 {
            return;
        }
        let fill = self.fill_rect();
        painter.rect(fill, &RectStyle::filled(self.fill_color, pill(fill)));
        if let Some(o) = self.overflow.filter(|o| self.fill > o.mark) {
            painter.push_clip(self.span(o.mark, self.fill - o.mark));
            painter.rect(fill, &RectStyle::filled(o.color, pill(fill)));
            painter.pop_clip();
        }
    }
}
