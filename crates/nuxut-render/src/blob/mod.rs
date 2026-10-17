//! Blob alanı: ekran çerçevesi ve ondan büyüyerek çıkan paneller tek bir dolu alan olarak
//! çizilir (caelestia'nın "puding" efekti).
//!
//! Şekil ADR-0008'in tek tanımıdır: serbest alan `F` (çerçevenin içi eksi paneller) için
//! `H = closing(opening(F, R), r)`. Panel çerçeveye ya da başka bir panele birleşiyorsa köşe
//! `R` (`smoothing`) yarıçaplı içbükey filetodur, boşluğa açılıyorsa `r` (`corner_radius`)
//! yarıçaplı dışbükey yaydır; çerçevenin iç köşeleri de `R`'dir. Özel dal yoktur. Sınır CPU'da
//! (f64, [`morph`]) doğru parçası ve yay olarak kesin hesaplanır, yalnızca şekil değişince
//! (önbellek: renderer); shader ilkellerden kesin işaretli uzaklığı ve çift-tek kuralıyla içeriyi
//! bulur, kenar yumuşatması gradyanla 1 fiziksel piksel.

pub(crate) mod geom;
mod input;
pub(crate) mod morph;
#[cfg(test)]
mod morph_tests;
#[cfg(test)]
mod gpu_tests;

use nuxut_core::{Color, Corners, Point, Rect};

use crate::style::ShadowStyle;
use geom::{Prim, v};
use morph::{Panel, Scene};

/// Bir alandaki en fazla şekil sayısı; fazlası çizilmez (uyarı loglanır).
pub const MAX_BLOB_SHAPES: usize = 16;

/// 2x2 kimlik matrisi, satır öncelikli `[a, b, c, d]` = `[[a, b], [c, d]]`.
pub const DEFORM_IDENTITY: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

#[derive(Debug, Clone, PartialEq)]
pub struct BlobField {
    /// Ekran çerçevesi; yoksa dolu alan yalnızca şekillerdir.
    pub frame: Option<FrameShape>,
    /// ≤ [`MAX_BLOB_SHAPES`]; fazlası çizilmez (uyarı loglanır).
    pub shapes: Vec<BlobShape>,
    /// `R`: birleşim filetosu ve çerçevenin iç köşe yarıçapı (mantıksal piksel).
    pub smoothing: f32,
    /// `r`: panellerin boşluğa açılan köşelerinin yarıçapı (mantıksal piksel).
    pub corner_radius: f32,
    pub fill: Color,
    pub shadow: Option<ShadowStyle>,
}

/// Ekran çerçevesi: `outer` içinde, `inner` dışında kalan yer dolu. İç köşeler `R`'dir
/// (ADR-0008, karar 2); `inner_radius` yok sayılır (yalnızca uyumluluk için duruyor).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameShape {
    pub outer: Rect,
    pub inner: Rect,
    pub inner_radius: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlobShape {
    pub rect: Rect,
    /// Şeklin kendi köşe yuvarlaması (ör. yüzen OSD hapı). Kenara ya da panele birleşen ve
    /// boşluğa açılan köşeleri alan kendisi biçimlendirir; paneller için sıfır yeterlidir
    /// ([`BlobShape::new_rect`]). Sıfırdan büyükse boşluğa açılan köşede `r` ile büyüğü görünür.
    pub radii: Corners,
    /// `deform_anchor` etrafında uygulanan 2x2 doğrusal dönüşüm (esneme/basılma),
    /// satır öncelikli. Kimlik = [`DEFORM_IDENTITY`].
    pub deform: [f32; 4],
    /// Deformun sabit noktası, `rect`'e göre normalize (`(0.5, 0.5)` = merkez,
    /// `(0.5, 0.0)` = üst kenarın ortası: bar'a yapışık panel üst kenarından kopmaz).
    pub deform_anchor: Point,
}

impl BlobShape {
    pub const CENTER: Point = Point::new(0.5, 0.5);

    /// Panel: köşeleri alan biçimlendirir (ADR-0008).
    pub fn new_rect(rect: Rect) -> Self {
        Self::new(rect, Corners::ZERO)
    }

    /// Kendi köşe yuvarlaması olan şekil (bkz. [`BlobShape::radii`]).
    pub fn new(rect: Rect, radii: Corners) -> Self {
        Self { rect, radii, deform: DEFORM_IDENTITY, deform_anchor: Self::CENTER }
    }

    pub fn with_deform(mut self, deform: [f32; 4], anchor: Point) -> Self {
        self.deform = deform;
        self.deform_anchor = anchor;
        self
    }

    /// Deformun sabit noktası (mantıksal piksel).
    pub fn anchor_point(&self) -> Point {
        Point::new(
            self.rect.x + self.rect.width * self.deform_anchor.x,
            self.rect.y + self.rect.height * self.deform_anchor.y,
        )
    }

    /// Şeklin ekranda göründüğü dikdörtgen: içerik (metin, kartlar) buna göre yerleşir ki
    /// esneme/basılma sırasında iç boşluk her karede sabit kalsın. Eksen hizalı esnemede
    /// (köşegen deform — kenara yapışık panellerin kullanması gereken biçim) tam sonuçtur;
    /// kayma (shear) içeren deformda sınır kutusudur.
    pub fn visual_rect(&self) -> Rect {
        self.deformed_bounds()
    }

    /// Deform uygulanmış dört köşe: sol-üst, sağ-üst, sağ-alt, sol-alt.
    pub fn deformed_corners(&self) -> [Point; 4] {
        let o = self.anchor_point();
        let [a, b, c, d] = self.deform;
        let r = self.rect;
        [(r.x, r.y), (r.right(), r.y), (r.right(), r.bottom()), (r.x, r.bottom())].map(|(x, y)| {
            let (dx, dy) = (x - o.x, y - o.y);
            Point::new(o.x + a * dx + b * dy, o.y + c * dx + d * dy)
        })
    }

    /// Deform uygulanmış şeklin eksen hizalı sınır kutusu.
    pub fn deformed_bounds(&self) -> Rect {
        bounds(self.deformed_corners())
    }
}

fn bounds(cs: [Point; 4]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in cs {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// Çerçevenin iç kenarına yapışık sayılma payı (mantıksal piksel).
const JOIN_EPS: f32 = 1e-3;

/// Shader'a gidecek, fiziksel piksele çevrilmiş blob verisi.
#[derive(Debug, Clone)]
pub(crate) struct PreparedBlob {
    /// Geometrinin girdisi; renderer sonucu bununla önbelleğe alır.
    pub scene: Scene,
    pub fill: [f32; 4],
    pub shadow: [f32; 4],
    pub shadow_geom: [f32; 3],
    /// Rasterize edilecek ayrık dikdörtgenler (fiziksel piksel). Renderer bunları serbest
    /// alanın sınır kutusuna göre SDF ve düz dolgu parçalarına böler ([`split_solid`]).
    pub rects: Vec<Rect>,
}

/// Bir ilkelin GPU kaydı: iki RGBA32F dokusu. `[0..4]` yapıştırılmış uçlar (başlangıç,
/// bitiş); `[4..8]` yaysa merkez, yarıçap ve yarım çember tarafı (x'e göre ±1), doğru
/// parçasıysa sıfır.
pub(crate) type GpuPrim = [f32; 8];

/// Sahnenin serbest alan sınırı, GPU kayıtları olarak (yalnızca şekil değişince çağrılır).
pub(crate) fn encode(scene: &Scene) -> Vec<GpuPrim> {
    let h = morph::free_area(scene);
    h.prims
        .iter()
        .zip(&h.ends)
        .map(|(p, (s, e))| {
            let tail = match p {
                Prim::Seg(..) => [0.0; 4],
                Prim::Arc(a) => {
                    let side = (a.a0 + a.sweep * 0.5).cos().signum();
                    [a.c.x as f32, a.c.y as f32, a.r as f32, side as f32]
                }
            };
            [s.x as f32, s.y as f32, e.x as f32, e.y as f32, tail[0], tail[1], tail[2], tail[3]]
        })
        .collect()
}

/// Girdiler 1/256 px ızgarasına oturtulur: aynı kenara yaslanan şekiller birebir eşit
/// koordinat alır (geometri teğetleri ve paylaşılan kenarları kesin görür).
fn grid(x: f64) -> f64 {
    (x * 256.0).round() / 256.0
}

impl BlobField {
    /// Şeklin bu alanda çizilen dört köşesi (sol-üst, sağ-üst, sağ-alt, sol-alt): deform
    /// uygulanır, ama deformsuz hâlde çerçevenin iç kenarına yapışık (ya da onu aşan) bir
    /// kenar deformdan sonra da o kenarda kalır. Esneme yalnızca serbest kenarları oynatır;
    /// aksi halde alan koruyan büzülme, kenar boyunca uzanan bir çekmeceyi hareket sırasında
    /// bar'dan/çerçeveden koparır ve yay otururken kapanan aralık son karelerde ince bir yarık
    /// olarak görünürdü (t074).
    pub fn joined_corners(&self, shape: &BlobShape) -> [Point; 4] {
        let mut cs = shape.deformed_corners();
        let Some(f) = self.frame else { return cs };
        let (r, i) = (shape.rect, f.inner);
        if r.x <= i.x + JOIN_EPS {
            for k in [0, 3] {
                cs[k].x = cs[k].x.min(r.x);
            }
        }
        if r.y <= i.y + JOIN_EPS {
            for k in [0, 1] {
                cs[k].y = cs[k].y.min(r.y);
            }
        }
        if r.right() >= i.right() - JOIN_EPS {
            for k in [1, 2] {
                cs[k].x = cs[k].x.max(r.right());
            }
        }
        if r.bottom() >= i.bottom() - JOIN_EPS {
            for k in [2, 3] {
                cs[k].y = cs[k].y.max(r.bottom());
            }
        }
        cs
    }

    /// Şeklin bu alanda göründüğü dikdörtgen ([`BlobShape::visual_rect`] artı çerçeveye
    /// yapışık kenar kuralı, bkz. [`BlobField::joined_corners`]): içerik buna yerleşir.
    pub fn visual_rect(&self, shape: &BlobShape) -> Rect {
        bounds(self.joined_corners(shape))
    }

    /// `shape` bu alanda çizilince etkilediği bölge (mantıksal): görünen şekil, birleşim
    /// filetosu, köprü ve cepler (aralığı 2R'ye kadar olan boğazlar), köşe yayı ve gölge.
    /// Hasar bildirirken şeklin eski ve yeni değerinin ikisi için çağrılır.
    pub fn damage_rect(&self, shape: &BlobShape) -> Rect {
        let shadow = self.shadow.map_or(0.0, |sh| sh.blur * 1.5 + sh.offset.x.abs().max(sh.offset.y.abs()));
        self.visual_rect(shape).expand(2.0 * (self.smoothing.max(0.0) + self.corner_radius.max(0.0)) + shadow + 1.0)
    }

    /// Geçerli şekillerin (ilk [`MAX_BLOB_SHAPES`]) `s` ölçeğinde, ızgaraya oturtulmuş sahne
    /// panelleri ve her birinin mantıksal sınır kutusu (deform + yapışık kenar kuralı dahil).
    /// `prepare` ve `input_region` aynı paneli kullanır: girdi bölgesi çizilen şekilden sapmaz.
    fn scene_panels(&self, s: f64) -> Vec<(Panel, Rect)> {
        let mut out = Vec::new();
        for shape in self.shapes.iter().take(MAX_BLOB_SHAPES) {
            let r = shape.rect;
            if !(r.width >= 0.0 && r.height >= 0.0 && r.x.is_finite() && r.y.is_finite())
                || shape.deform.iter().any(|x| !x.is_finite())
            {
                continue;
            }
            let [a, b, c, d] = shape.deform;
            // Kendi yuvarlaması deform ile alan oranında ölçeklenir.
            let k = f64::from((a * d - b * c).abs().sqrt()) * s;
            let rd = shape.radii.clamped(r.size());
            let cs = self.joined_corners(shape);
            out.push((
                Panel {
                    pts: cs.map(|p| v(grid(f64::from(p.x) * s), grid(f64::from(p.y) * s))),
                    radii: [rd.top_left, rd.top_right, rd.bottom_right, rd.bottom_left].map(|x| f64::from(x) * k),
                },
                bounds(cs),
            ));
        }
        out
    }

    pub(crate) fn prepare(&self, scale: f32, viewport: (u32, u32)) -> Option<PreparedBlob> {
        let s = f64::from(scale);
        let join = f64::from(self.smoothing.max(0.0)) * s;
        let corner = f64::from(self.corner_radius.max(0.0)) * s;
        if self.shapes.len() > MAX_BLOB_SHAPES {
            tracing::warn!("blob alanında {} şekil var; ilk {MAX_BLOB_SHAPES} çizilir", self.shapes.len());
        }
        let (shadow, shadow_geom, shadow_pad) = match self.shadow {
            Some(sh) if sh.color.a > 0.0 => {
                let sigma = (sh.blur * 0.5 * scale).max(0.5);
                let (ox, oy) = (sh.offset.x * scale, sh.offset.y * scale);
                (sh.color.premultiplied(), [ox, oy, sigma], sigma * 3.0 + ox.abs().max(oy.abs()))
            }
            _ => ([0.0; 4], [0.0, 0.0, 1.0], 0.0),
        };
        // Şekil değişiminin etki erimi: köprü/cep 2R, köşe kapanışı 2r, artı gölge ve AA.
        let reach = (2.0 * (join + corner)) as f32 + shadow_pad + 2.0;
        let px = |r: Rect| Rect::new(r.x * scale, r.y * scale, r.width * scale, r.height * scale);
        let viewport_rect = Rect::new(0.0, 0.0, viewport.0 as f32, viewport.1 as f32);
        let mut regions: Vec<Rect> = Vec::new();
        let (clip, inner) = match self.frame {
            Some(f) => {
                let (o, i) = (px(f.outer), px(f.inner));
                regions.push(Rect::new(o.x, o.y, o.width, i.y - o.y + reach));
                regions.push(Rect::new(o.x, i.bottom() - reach, o.width, o.bottom() - i.bottom() + reach));
                regions.push(Rect::new(o.x, o.y, i.x - o.x + reach, o.height));
                regions.push(Rect::new(i.right() - reach, o.y, o.right() - i.right() + reach, o.height));
                (o.intersect(&viewport_rect), i)
            }
            // Çerçevesiz: serbest alanın sınırı görünen alanın çok dışında.
            None => (viewport_rect, viewport_rect.expand(2.0 * reach + 64.0)),
        };
        let mut panels = Vec::new();
        for (panel, b) in self.scene_panels(s) {
            panels.push(panel);
            regions.push(px(b).expand(reach));
        }
        let regions: Vec<Rect> =
            regions.iter().map(|r| r.round_out().intersect(&clip)).filter(|r| !r.is_empty()).collect();
        if regions.is_empty() {
            return None;
        }
        let rects = disjoint_cover(&regions);
        let scene = Scene {
            inner: (
                grid(f64::from(inner.x)),
                grid(f64::from(inner.y)),
                grid(f64::from(inner.right())),
                grid(f64::from(inner.bottom())),
            ),
            panels,
            join: grid(join),
            corner: grid(corner),
        };
        Some(PreparedBlob { scene, fill: self.fill.premultiplied(), shadow, shadow_geom, rects })
    }
}

/// Serbest alan sınırından bu kadar (fiziksel px) uzak dolu pikselin kapsaması kesin 1'dir:
/// `blob.frag`'ta d ≤ −2 ve |∇d| ≤ √2 (uzaklık 1-Lipschitz) → cov = 1, gölge terimi yok;
/// renk tam `u_fill`. Böyle pikseller SDF döngüsü yerine düz dolguyla çizilir (aynı sonuç).
pub(crate) const SOLID_MARGIN: f32 = 2.0;

/// Serbest alanın (ilkellerin) sınır kutusu, fiziksel piksel; ilkel yoksa `None` (her yer dolu).
/// Yaylar için merkez ± yarıçap (temkinli).
pub(crate) fn free_bounds(prims: &[GpuPrim]) -> Option<Rect> {
    let mut lo = [f32::INFINITY; 2];
    let mut hi = [f32::NEG_INFINITY; 2];
    let mut add = |x: f32, y: f32| {
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    };
    for p in prims {
        add(p[0], p[1]);
        add(p[2], p[3]);
        if p[6] > 0.0 {
            add(p[4] - p[6], p[5] - p[6]);
            add(p[4] + p[6], p[5] + p[6]);
        }
    }
    (lo[0] <= hi[0] && lo[1] <= hi[1]).then(|| Rect::new(lo[0], lo[1], hi[0] - lo[0], hi[1] - lo[1]))
}

/// `rect`'i serbest alan kutusuna (`free`, payıyla genişletilmiş) göre böler: kutuyla kesişen
/// parça `sdf`'e (tam alan gölgelendiricisi), kalanı en çok dört dikdörtgen olarak `solid`'e
/// (düz dolgu). Parçalar ayrık ve birleşimleri `rect`.
pub(crate) fn split_solid(rect: Rect, free: Option<Rect>, sdf: &mut Vec<Rect>, solid: &mut Vec<Rect>) {
    let Some(free) = free else {
        solid.push(rect);
        return;
    };
    let i = rect.intersect(&free);
    if i.is_empty() {
        solid.push(rect);
        return;
    }
    sdf.push(i);
    let pieces = [
        Rect::new(rect.x, rect.y, rect.width, i.y - rect.y),
        Rect::new(rect.x, i.bottom(), rect.width, rect.bottom() - i.bottom()),
        Rect::new(rect.x, i.y, i.x - rect.x, i.height),
        Rect::new(i.right(), i.y, rect.right() - i.right(), i.height),
    ];
    solid.extend(pieces.into_iter().filter(|r| !r.is_empty()));
}

/// Dikdörtgenlerin birleşimini ayrık dikdörtgenlerle kaplar (koordinat sıkıştırma ızgarası).
/// Örtüşen bölge iki kez rasterize edilmez; aksi halde yarı saydam kenarlar iki kez karışırdı.
pub(crate) fn disjoint_cover(rects: &[Rect]) -> Vec<Rect> {
    let mut xs: Vec<f32> = rects.iter().flat_map(|r| [r.x, r.right()]).collect();
    let mut ys: Vec<f32> = rects.iter().flat_map(|r| [r.y, r.bottom()]).collect();
    for v in [&mut xs, &mut ys] {
        v.sort_by(f32::total_cmp);
        v.dedup();
    }
    let mut out: Vec<Rect> = Vec::new();
    // Bir önceki satırın parçaları: aynı x aralığı alt alta gelirse dikeyde birleştirilir.
    let mut prev_row: Vec<usize> = Vec::new();
    for yw in ys.windows(2) {
        let cy = (yw[0] + yw[1]) * 0.5;
        let mut row: Vec<(f32, f32)> = Vec::new();
        for xw in xs.windows(2) {
            let cx = (xw[0] + xw[1]) * 0.5;
            let covered = rects.iter().any(|r| cx > r.x && cx < r.right() && cy > r.y && cy < r.bottom());
            if !covered {
                continue;
            }
            match row.last_mut() {
                Some(run) if run.1 == xw[0] => run.1 = xw[1],
                _ => row.push((xw[0], xw[1])),
            }
        }
        let mut this_row = Vec::new();
        for (x0, x1) in row {
            let extend = prev_row.iter().copied().find(|&i| {
                let r = &out[i];
                r.x == x0 && r.right() == x1 && r.bottom() == yw[0]
            });
            match extend {
                Some(i) => {
                    out[i].height = yw[1] - out[i].y;
                    this_row.push(i);
                }
                None => {
                    out.push(Rect::new(x0, yw[0], x1 - x0, yw[1] - yw[0]));
                    this_row.push(out.len() - 1);
                }
            }
        }
        prev_row = this_row;
    }
    out
}


#[cfg(test)]
mod tests {
    use super::*;

    fn area(rs: &[Rect]) -> f32 {
        rs.iter().map(|r| r.width * r.height).sum()
    }

    #[test]
    fn cover_is_disjoint_and_exact() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        let c = Rect::new(100.0, 0.0, 5.0, 5.0);
        let cover = disjoint_cover(&[a, b, c]);
        assert_eq!(area(&cover), 100.0 + 100.0 - 25.0 + 25.0);
        for (i, r) in cover.iter().enumerate() {
            for q in &cover[i + 1..] {
                assert!(r.intersect(q).is_empty(), "{r:?} {q:?}");
            }
        }
    }

    #[test]
    fn single_rect_stays_single() {
        let r = Rect::new(1.0, 2.0, 30.0, 40.0);
        assert_eq!(disjoint_cover(&[r, r]), vec![r]);
    }
    #[test]
    fn visual_rect_follows_anchored_stretch() {
        let shape = BlobShape::new_rect(Rect::new(100.0, 40.0, 200.0, 100.0))
            .with_deform([0.8, 0.0, 0.0, 1.25], Point::new(0.5, 0.0));
        let v = shape.visual_rect();
        // Üst kenar çapada sabit, yatayda merkeze doğru daralır, dikeyde aşağı uzar.
        assert_eq!(v.y, 40.0);
        assert!((v.x - 120.0).abs() < 1e-4 && (v.width - 160.0).abs() < 1e-4);
        assert!((v.height - 125.0).abs() < 1e-4);
        assert_eq!(BlobShape::new_rect(shape.rect).visual_rect(), shape.rect);
    }

    fn frame_field(shapes: Vec<BlobShape>) -> BlobField {
        BlobField {
            frame: Some(FrameShape {
                outer: Rect::new(0.0, 0.0, 1000.0, 800.0),
                inner: Rect::new(10.0, 40.0, 980.0, 750.0),
                inner_radius: 0.0,
            }),
            shapes,
            smoothing: 14.0,
            corner_radius: 12.0,
            fill: Color::BLACK,
            shadow: None,
        }
    }

    #[test]
    fn solid_split_is_disjoint_and_complete() {
        let r = Rect::new(0.0, 0.0, 100.0, 40.0);
        let (mut sdf, mut solid) = (Vec::new(), Vec::new());
        split_solid(r, Some(Rect::new(10.0, 30.0, 50.0, 100.0)), &mut sdf, &mut solid);
        assert_eq!(sdf, vec![Rect::new(10.0, 30.0, 50.0, 10.0)]);
        let area = |v: &[Rect]| v.iter().map(|r| r.width * r.height).sum::<f32>();
        assert_eq!(area(&sdf) + area(&solid), 100.0 * 40.0);
        let all: Vec<Rect> = sdf.iter().chain(&solid).copied().collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert!(a.intersect(b).is_empty(), "{a:?} ∩ {b:?}");
            }
        }
        // Kutuyla kesişmeyen ve serbest alanı olmayan dikdörtgen tümüyle düz dolgu.
        let (mut sdf, mut solid) = (Vec::new(), Vec::new());
        split_solid(r, Some(Rect::new(200.0, 0.0, 5.0, 5.0)), &mut sdf, &mut solid);
        split_solid(r, None, &mut sdf, &mut solid);
        assert!(sdf.is_empty());
        assert_eq!(solid, vec![r, r]);
    }

    #[test]
    fn free_bounds_cover_arcs() {
        let p = frame_field(vec![]).prepare(1.0, (1000, 800)).unwrap();
        let b = free_bounds(&encode(&p.scene)).expect("çerçevenin serbest alanı var");
        let (x0, y0, x1, y1) = p.scene.inner;
        assert!(f64::from(b.x) <= x0 && f64::from(b.y) <= y0, "{b:?}");
        assert!(f64::from(b.right()) >= x1 && f64::from(b.bottom()) >= y1, "{b:?}");
        assert_eq!(free_bounds(&[]), None);
    }

    #[test]
    fn frame_only_rasterizes_strips() {
        let p = frame_field(vec![]).prepare(1.0, (1000, 800)).unwrap();
        let covered: f32 = p.rects.iter().map(|r| r.width * r.height).sum();
        assert!(covered < 1000.0 * 800.0 * 0.4, "tam ekran çizilmemeli: {covered}");
        // Çerçeve: dört kenar ve dört R köşe yayı (yaylar y'ye göre tekdüze tek parça).
        let prims = encode(&p.scene);
        assert_eq!(prims.len(), 8, "{prims:?}");
        assert_eq!(prims.iter().filter(|q| q[6] > 0.0).count(), 4);
        assert!(prims.iter().filter(|q| q[6] > 0.0).all(|q| (q[6] - 14.0).abs() < 1e-4));
    }

    /// Shell'in sağ kenar çekmecesi: iç alanın tam boyu (üstte bar'a, altta çerçeveye yapışık),
    /// yatay harekette alan koruyan esneme (x'te uzar, y'de büzülür), çapa sağ kenarın ortası.
    fn side_drawer(stretch: f32) -> BlobShape {
        BlobShape::new_rect(Rect::new(990.0 - 380.0, 40.0, 380.0, 750.0))
            .with_deform([stretch, 0.0, 0.0, 1.0 / stretch], Point::new(1.0, 0.5))
    }

    #[test]
    fn joined_edges_stay_on_the_frame_under_deform() {
        let f = frame_field(vec![]);
        for stretch in [1.0, 1.02, 1.1, 1.3, 0.9] {
            let sh = side_drawer(stretch);
            // Deformun kendisi üst/alt kenarı kenardan ayırır (büzülmede) …
            if stretch > 1.0 {
                assert!(sh.deformed_bounds().y > 40.5, "{stretch}");
            }
            // … alan ise yapışık kenarları yerinde tutar; serbest sol kenar esner.
            let v = f.visual_rect(&sh);
            assert_eq!(v.y, sh.rect.y.min(sh.deformed_bounds().y), "{stretch}");
            assert_eq!(v.bottom(), sh.rect.bottom().max(sh.deformed_bounds().bottom()), "{stretch}");
            assert_eq!(v.right(), 990.0, "{stretch}");
            assert!((v.x - sh.deformed_bounds().x).abs() < 1e-3, "{stretch}");
            assert_eq!(f.damage_rect(&sh).intersect(&v), v);
        }
        // Çerçeveye değmeyen kenarlar eskisi gibi esner (bar'dan sarkan ortalı panel).
        let free = BlobShape::new_rect(Rect::new(400.0, 40.0, 200.0, 300.0))
            .with_deform([0.8, 0.0, 0.0, 1.25], Point::new(0.5, 0.0));
        assert_eq!(f.visual_rect(&free), free.visual_rect());
        // Çerçevesiz alanda kural yok.
        let mut bare = frame_field(vec![]);
        bare.frame = None;
        assert_eq!(bare.visual_rect(&side_drawer(1.3)), side_drawer(1.3).visual_rect());
    }

    #[test]
    fn stretched_side_drawer_has_no_slit_at_its_joins() {
        // t074 regresyonu: esneme yay ile kimliğe dönerken (1.3 → 1) çekmecenin bar'a ve alt
        // çerçeveye birleştiği yerde hiçbir karede serbest (boş) nokta açılmaz.
        for k in 0..=30 {
            let stretch = 1.0 + 0.01 * k as f32;
            let p = frame_field(vec![side_drawer(stretch)]).prepare(1.0, (1000, 800)).unwrap();
            let h = morph::free_area(&p.scene);
            let x0 = f64::from(990.0 - 380.0 * stretch) + 2.0 * 14.0 + 1.0;
            for i in 0..=40 {
                let x = x0 + (989.0 - x0) * f64::from(i) / 40.0;
                for y in [40.25, 41.0, 43.0, 46.0, 789.75, 789.0, 787.0, 784.0] {
                    assert!(h.sdf(v(x, y)) > -1e-6, "esneme {stretch}: ({x}, {y}) boş");
                }
            }
        }
    }

    #[test]
    fn scene_is_on_the_grid_and_scaled() {
        let f = frame_field(vec![BlobShape::new_rect(Rect::new(400.1, 30.0, 300.0, 200.0))]);
        let p = f.prepare(1.5, (1500, 1200)).unwrap();
        assert_eq!(p.scene.join, 21.0);
        assert_eq!(p.scene.corner, 18.0);
        assert_eq!(p.scene.inner.1, 60.0);
        let x = p.scene.panels[0].pts[0].x;
        assert_eq!(x, (400.1f64 * 1.5 * 256.0).round() / 256.0);
    }
}
