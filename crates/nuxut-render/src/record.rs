//! GL'siz çizim kaydı: bileşen testleri bir karenin ne çizdiğini (sıra ve etkin kırpma)
//! GPU olmadan denetler. Renderer'ın kullandığı kayıtla aynıdır; kare sonu birleştirmesi
//! (kaplama katmanı) de aynı yoldan geçer.

use nuxut_core::Rect;

use crate::damage::PxRect;
use crate::painter::{Op, Painter, Recording};
use crate::stroke::KIND_AREA;
use crate::text::TextSystem;

/// Kaydedilmiş çizimin türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawKind {
    /// Dikdörtgen ya da gölge örneği.
    Quad,
    /// Blob alanındaki bir şekil (menü, popover, panel).
    Blob,
    /// Metin parçası (glyph dizisi).
    Glyphs,
    Image,
    Gradient,
    /// Çizgi ya da çember yayı.
    Stroke,
    /// Eğri altı alan sütunu ([`Painter::area`]).
    Area,
}

/// Kaydedilmiş bir çizim (çizim sırasıyla).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedDraw {
    pub kind: DrawKind,
    /// Kapsam, fiziksel piksel: dikdörtgen/görüntü/geçiş için şeklin kendisi, blob için
    /// şeklin görünen dikdörtgeni, çizgi/yay/alanda sınır kutusu (AA payı dahil); metinde boş
    /// (`Rect::ZERO`, konumsuz).
    pub rect: Rect,
    /// O anki etkin kırpma (fiziksel piksel); `None` = kırpmasız.
    pub clip: Option<PxRect>,
}

/// GL'siz kare kaydedici. Bir kez oluşturulur, her [`record`](Self::record) önceki kaydı siler.
#[derive(Debug, Default)]
pub struct Recorder {
    rec: Recording,
    scale: f32,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// `f`'in çizimlerini kaydeder (`size_px` fiziksel boyut, `scale` ölçek).
    pub fn record(&mut self, text: &mut TextSystem, size_px: (u32, u32), scale: f32, f: impl FnOnce(&mut Painter)) {
        self.rec.clear();
        self.scale = scale;
        {
            let mut painter = Painter::new(&mut self.rec, text, size_px, scale);
            f(&mut painter);
        }
        self.rec.merge_overlay();
    }

    /// Tüm çizimler, çizim sırasıyla, etkin kırpmalarıyla.
    pub fn draws(&self) -> Vec<RecordedDraw> {
        let px = |[x, y, w, h]: [f32; 4]| Rect::new(x, y, w, h);
        let s = self.scale;
        let mut clip = None;
        let mut out = Vec::new();
        for op in &self.rec.ops {
            let mut push = |kind, rect| out.push(RecordedDraw { kind, rect, clip });
            match op {
                Op::Clip(c) => clip = c.map(|[x0, y0, x1, y1]| PxRect::new(x0, y0, x1 - x0, y1 - y0)),
                Op::Quads(r) => self.rec.quads[r.clone()].iter().for_each(|q| push(DrawKind::Quad, px(q.rect))),
                Op::Gradients(r) => {
                    self.rec.gradients[r.clone()].iter().for_each(|g| push(DrawKind::Gradient, px(g.rect)));
                }
                Op::Strokes(r) => self.rec.strokes[r.clone()].iter().for_each(|st| {
                    let kind = if st.param[1] == KIND_AREA { DrawKind::Area } else { DrawKind::Stroke };
                    push(kind, px(st.bounds()))
                }),
                Op::Glyphs(_) => push(DrawKind::Glyphs, Rect::ZERO),
                Op::Image(i) => push(DrawKind::Image, px(self.rec.images[*i].rect)),
                Op::Blob(i) => {
                    for shape in &self.rec.blobs[*i].shapes {
                        let v = shape.visual_rect();
                        push(DrawKind::Blob, Rect::new(v.x * s, v.y * s, v.width * s, v.height * s));
                    }
                }
            }
        }
        out
    }

    /// Yalnızca dikdörtgen/gölge örnekleri ([`draws`](Self::draws) süzgeci).
    pub fn quads(&self) -> Vec<RecordedDraw> {
        self.draws().into_iter().filter(|d| d.kind == DrawKind::Quad).collect()
    }

    /// Görüntü çizimlerinin opaklıkları, çizim sırasıyla (solma testleri için).
    pub fn image_opacities(&self) -> Vec<f32> {
        self.rec
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Image(i) => Some(self.rec.images[*i].opacity),
                _ => None,
            })
            .collect()
    }

    /// Kayıtta hiç çizim komutu yok mu (kırpma komutları sayılmaz)?
    pub fn is_empty(&self) -> bool {
        self.rec.ops.iter().all(|op| matches!(op, Op::Clip(_)))
    }
}

#[cfg(test)]
mod tests {
    use nuxut_core::{Color, Corners};

    use super::*;
    use crate::style::RectStyle;

    fn fill() -> RectStyle {
        RectStyle::filled(Color::WHITE, Corners::ZERO)
    }

    #[test]
    fn overlay_escapes_clip_and_draws_last() {
        let mut text = TextSystem::new();
        let mut r = Recorder::new();
        r.record(&mut text, (400, 300), 1.0, |p| {
            p.push_clip(Rect::new(10.0, 10.0, 100.0, 50.0));
            p.rect(Rect::new(20.0, 20.0, 10.0, 10.0), &fill());
            assert!(p.begin_overlay());
            assert!(!p.begin_overlay(), "iç içe kaplama yeni katman açmaz");
            // Kaplamanın kendi kırpması temelden bağımsız.
            p.push_clip(Rect::new(0.0, 0.0, 400.0, 300.0));
            p.rect(Rect::new(50.0, 40.0, 200.0, 150.0), &fill());
            p.pop_clip();
            p.end_overlay();
            assert!(!p.in_overlay());
            // Temel katman kırpması geri geldi: sonraki çizim yine kırpılı.
            p.rect(Rect::new(30.0, 20.0, 10.0, 10.0), &fill());
            p.pop_clip();
        });
        let q = r.quads();
        assert_eq!(q.len(), 3);
        let clip = Some(PxRect::new(10, 10, 100, 50));
        assert_eq!((q[0].rect.x, q[0].clip), (20.0, clip));
        assert_eq!((q[1].rect.x, q[1].clip), (30.0, clip), "temel katman sırası korunur");
        // Kaplama en sonda, kare kırpmasından bağımsız.
        assert_eq!(q[2].rect, Rect::new(50.0, 40.0, 200.0, 150.0));
        assert_eq!(q[2].clip, Some(PxRect::new(0, 0, 400, 300)));
    }

    #[test]
    fn unbalanced_overlay_is_closed_and_unclipped() {
        let mut text = TextSystem::new();
        let mut r = Recorder::new();
        r.record(&mut text, (200, 200), 2.0, |p| {
            p.push_clip(Rect::new(0.0, 0.0, 10.0, 10.0));
            p.begin_overlay();
            p.push_clip(Rect::new(0.0, 0.0, 5.0, 5.0));
            p.rect(Rect::new(1.0, 1.0, 2.0, 2.0), &fill());
            p.end_overlay(); // kapatılmamış kaplama kırpması atılır
            p.begin_overlay();
            p.rect(Rect::new(20.0, 20.0, 40.0, 40.0), &fill());
            // `end_overlay` yok: Painter düşerken kapatılır.
        });
        let q = r.quads();
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].clip, Some(PxRect::new(0, 0, 10, 10)));
        assert_eq!(q[1].rect, Rect::new(40.0, 40.0, 80.0, 80.0));
        assert_eq!(q[1].clip, None, "önceki kaplama parçasının kırpması sızmaz");
    }

    #[test]
    fn empty_overlay_adds_nothing() {
        let mut text = TextSystem::new();
        let mut r = Recorder::new();
        r.record(&mut text, (100, 100), 1.0, |p| {
            p.begin_overlay();
            p.end_overlay();
        });
        assert!(r.is_empty());
        assert!(r.rec.ops.is_empty(), "boş kaplama komut eklemez");
    }
}
