//! Blob alanının girdi (fare/dokunma) bölgesi: çizilen dolu alanın tam piksele hizalı,
//! ayrık dikdörtgenlerle kaplanması (ADR-0008: "girdi bölgesi aynı `H`'den türer").
//!
//! Piksel kuralı: mantıksal pikselin merkezi dolu alandaysa (serbest alan `H`'nin dışında,
//! çerçevenin `outer`'ı içinde) piksel bölgededir. Shader'ın kapsaması `0.5 + sdf` olduğundan bu,
//! "1.0 ölçekte kapsama ≥ 0.5" ile aynıdır; bölgenin sınırı çizilen kenardan en çok ±0.5 px
//! (yarım AA pikseli) sapar. Kesirli ölçekte geometri aynıdır (1/256 px ızgarası dışında), yalnız
//! karar mantıksal piksel merkezinde verilir.

use nuxut_core::Rect;

use super::geom::Region;
use super::morph::{self, Scene};
use super::{BlobField, grid};

impl BlobField {
    /// Dolu alanı (çerçeve + paneller + birleşim filetoları; boşluğa açılan `r` köşeleri ve
    /// boğaz cepleri hariç; gölge hariç) kaplayan ayrık, tam piksele hizalı dikdörtgenler,
    /// mantıksal pikselde (alanın koordinatlarında). Wayland `wl_region`'ına doğrudan eklenir.
    ///
    /// Düz bölgeler büyük dikdörtgenlerdir; yalnızca eğri sınır geçen satırlar 1 px yüksekliğinde
    /// şeritlere bölünür (aynı yatay aralığa sahip ardışık satırlar birleştirilir). Saf CPU,
    /// çizim sahnesiyle aynı geometri (deform, yapışık kenar kuralı, ızgara). Önbelleği yoktur:
    /// yalnızca şekil değişince (animasyon oturunca ya da alan değişince) çağrılmalıdır.
    ///
    /// Çerçevesiz alanda bölge şekillerin çevresiyle sınırlıdır; çerçeveliyse `frame.outer`
    /// ile kırpılır.
    pub fn input_region(&self) -> Vec<Rect> {
        match self.input_scene() {
            Some((scene, clip)) => fill_rects(&morph::free_area(&scene), clip),
            None => Vec::new(),
        }
    }

    /// Girdi bölgesinin sahnesi (1.0 ölçek, mantıksal piksel) ve kırpma dikdörtgeni.
    pub(super) fn input_scene(&self) -> Option<(Scene, Rect)> {
        let join = f64::from(self.smoothing.max(0.0));
        let corner = f64::from(self.corner_radius.max(0.0));
        let panels = self.scene_panels(1.0);
        let reach = (2.0 * (join + corner)) as f32 + 2.0;
        let (clip, inner) = match self.frame {
            Some(f) => (f.outer, f.inner),
            None => {
                // Dolu alan şekillerden en çok birleşim erimi kadar taşar (filetolar); serbest
                // alanın dış sınırı bunun çok dışında kalır.
                let bbox = panels.iter().map(|(_, b)| *b).reduce(|a, b| a.union(&b))?;
                let clip = bbox.expand(reach);
                (clip, clip.expand(2.0 * reach + 64.0))
            }
        };
        let finite = |r: Rect| [r.x, r.y, r.width, r.height].iter().all(|x| x.is_finite());
        if !finite(clip) || !finite(inner) || clip.is_empty() {
            return None;
        }
        let scene = Scene {
            inner: (
                grid(f64::from(inner.x)),
                grid(f64::from(inner.y)),
                grid(f64::from(inner.right())),
                grid(f64::from(inner.bottom())),
            ),
            panels: panels.into_iter().map(|(p, _)| p).collect(),
            join: grid(join),
            corner: grid(corner),
        };
        Some((scene, clip))
    }
}

/// Piksel merkezi `[lo, hi)` içinde kalan tamsayı aralık: `[⌈lo − ½⌉, ⌈hi − ½⌉)`.
fn px_span(lo: f64, hi: f64) -> (i64, i64) {
    ((lo - 0.5).ceil() as i64, (hi - 0.5).ceil() as i64)
}

/// `clip` içinde, piksel merkezi `h`'nin dışında kalan pikselleri ayrık dikdörtgenlerle kaplar.
///
/// Satır başına (`y + ½`) `h`'nin ilkellerini kesen x'ler, [`Region::inside`]'ın yarı açık
/// kuralıyla bulunur (aynı yapıştırılmış uçlar); çift-tek ile `h` aralıkları çıkar, tümleyeni
/// satırın dolu parçalarıdır. Bir önceki satırda aynı `[x0, x1)` ile biten dikdörtgen aşağı
/// uzatılır. İlkeller y aralığına göre sıralı tutulur: satır başına yalnızca o satırı kesenler.
pub(super) fn fill_rects(h: &Region, clip: Rect) -> Vec<Rect> {
    // Ekran ölçeğinin çok üstündeki girdiyi sınırla (i32 dikdörtgen, satır döngüsü).
    const LIMIT: f64 = 1.0e6;
    let c = |x: f32| f64::from(x).clamp(-LIMIT, LIMIT);
    let (x0, x1) = px_span(c(clip.x), c(clip.right()));
    let (y0, y1) = px_span(c(clip.y), c(clip.bottom()));
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let spans: Vec<(f64, f64)> = h.ends.iter().map(|(s, e)| (s.y.min(e.y), s.y.max(e.y))).collect();
    let mut order: Vec<usize> = (0..spans.len()).collect();
    order.sort_by(|&a, &b| spans[a].0.total_cmp(&spans[b].0));
    let (mut next, mut active, mut xs) = (0, Vec::new(), Vec::new());

    let mut out: Vec<Rect> = Vec::new();
    // Açık dikdörtgenler: (x0, x1, başladığı satır), x0'a göre sıralı.
    let mut open: Vec<(i64, i64, i64)> = Vec::new();
    let mut row: Vec<(i64, i64)> = Vec::new();
    let close = |out: &mut Vec<Rect>, (a, b, top): (i64, i64, i64), y: i64| {
        out.push(Rect::new(a as f32, top as f32, (b - a) as f32, (y - top) as f32));
    };
    for y in y0..y1 {
        let cy = y as f64 + 0.5;
        while next < order.len() && spans[order[next]].0 <= cy {
            active.push(order[next]);
            next += 1;
        }
        active.retain(|&i| spans[i].1 > cy);
        xs.clear();
        for &i in &active {
            let (s, e) = h.ends[i];
            if let Some(x) = h.prims[i].line_x(cy, s, e) {
                xs.push(x);
            }
        }
        xs.sort_by(f64::total_cmp);
        // `x`'in sağındaki kesişim sayısı tekse `x` H'nin içinde: [xs[j], xs[j+1]) aralığında
        // bu sayı `m − 1 − j`. En soldaki aralık (−∞, xs[0]) `m` kesişim görür.
        let m = xs.len();
        row.clear();
        let mut cur = x0;
        let free = |lo: f64, hi: f64, cur: &mut i64, row: &mut Vec<(i64, i64)>| {
            let (a, b) = px_span(lo, hi);
            let (a, b) = (a.clamp(x0, x1), b.clamp(x0, x1));
            if b <= a {
                return;
            }
            if a > *cur {
                row.push((*cur, a));
            }
            *cur = (*cur).max(b);
        };
        if m % 2 == 1 && m > 0 {
            free(-LIMIT * 2.0, xs[0], &mut cur, &mut row);
        }
        for j in 0..m {
            if (m - j) % 2 == 0 {
                let hi = xs.get(j + 1).copied().unwrap_or(LIMIT * 2.0);
                free(xs[j], hi, &mut cur, &mut row);
            }
        }
        if cur < x1 {
            row.push((cur, x1));
        }
        // Dikey birleştirme: aynı aralık açık kalır, diğerleri kapanır.
        let mut kept = Vec::with_capacity(row.len());
        let mut k = 0;
        for &(a, b) in &row {
            while k < open.len() && open[k].0 < a {
                close(&mut out, open[k], y);
                k += 1;
            }
            if k < open.len() && open[k].0 == a && open[k].1 == b {
                kept.push(open[k]);
                k += 1;
            } else {
                kept.push((a, b, y));
            }
        }
        for o in &open[k..] {
            close(&mut out, *o, y);
        }
        open = kept;
    }
    for o in open {
        close(&mut out, o, y1);
    }
    out
}

#[cfg(test)]
mod tests {
    use nuxut_core::{Color, Point, Rect};

    use super::super::geom::v;
    use super::super::gpu_tests::{field, scenes};
    use super::super::{BlobField, BlobShape, FrameShape, morph};

    /// Alanı `w × h` pikselde boyar; her pikselin kaç dikdörtgende olduğunu sayar ve
    /// dikdörtgenlerin tam piksele hizalı olduğunu doğrular.
    fn paint(rects: &[Rect], w: usize, h: usize) -> Vec<u8> {
        let mut px = vec![0u8; w * h];
        for r in rects {
            for x in [r.x, r.y, r.width, r.height] {
                assert_eq!(x, x.round(), "tam piksel değil: {r:?}");
            }
            assert!(r.width > 0.0 && r.height > 0.0, "boş dikdörtgen: {r:?}");
            assert!(r.x >= 0.0 && r.y >= 0.0 && r.right() <= w as f32 && r.bottom() <= h as f32, "{r:?}");
            for y in r.y as usize..r.bottom() as usize {
                for x in r.x as usize..r.right() as usize {
                    px[y * w + x] += 1;
                }
            }
        }
        assert!(px.iter().all(|&n| n <= 1), "dikdörtgenler örtüşüyor");
        px
    }

    fn hit(rects: &[Rect], x: f32, y: f32) -> bool {
        rects.iter().any(|r| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
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
            fill: Color::BLACK, // renk-matematiği: girdi bölgesi testi rengi okumaz
            shadow: None,
        }
    }

    /// Her pikselde: bölgede ⇔ merkezi çerçeve dışında ya da `H`'nin dışında (çizim sahnesinin
    /// CPU referansı). Ayrıca ayrıklık ve tam piksel.
    fn assert_matches_reference(f: &BlobField, w: usize, h: usize, name: &str) {
        let rects = f.input_region();
        let px = paint(&rects, w, h);
        let (scene, clip) = f.input_scene().unwrap();
        let free = morph::free_area(&scene);
        let free = super::super::geom::Indexed::new(&free);
        let mut bad = 0;
        for y in 0..h {
            for x in 0..w {
                let c = v(x as f64 + 0.5, y as f64 + 0.5);
                let in_clip = c.x >= f64::from(clip.x)
                    && c.x < f64::from(clip.right())
                    && c.y >= f64::from(clip.y)
                    && c.y < f64::from(clip.bottom());
                let want = in_clip && !free.inside(c);
                if want != (px[y * w + x] == 1) {
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0, "{name}: {bad} piksel referansla çelişiyor ({} dikdörtgen)", rects.len());
    }

    #[test]
    fn top_panel_fillets_are_in_and_open_corners_are_out() {
        // Bar'dan inen panel: sol üst birleşimde R = 14 filetosu (merkez (386, 54)), sol alt
        // köşede r = 12 yayı (merkez (412, 328)).
        let f = frame_field(vec![
            BlobShape::new_rect(Rect::new(400.0, 36.0, 200.0, 304.0)).with_deform([1.0, 0.0, 0.0, 1.0], Point::new(0.5, 0.0)),
        ]);
        let rs = f.input_region();
        paint(&rs, 1000, 800);
        // Filetonun içi (panel dikdörtgeninin dışında): dolu, tıklama shell'de kalır.
        assert!(hit(&rs, 397.5, 42.5), "fileto dolu olmalı");
        assert!(hit(&rs, 602.5, 42.5), "sağ fileto dolu olmalı");
        // Filetonun çemberi içi: serbest.
        assert!(!hit(&rs, 380.5, 60.5));
        // Panelin r köşesinin dışı (dikdörtgenin içinde ama yuvarlanmış): serbest.
        assert!(!hit(&rs, 400.5, 339.5), "açık köşe boş olmalı");
        assert!(!hit(&rs, 599.5, 339.5), "açık köşe boş olmalı");
        assert!(hit(&rs, 405.5, 333.5));
        assert!(hit(&rs, 500.5, 200.5) && hit(&rs, 2.5, 400.5) && hit(&rs, 500.5, 10.5));
        assert!(!hit(&rs, 500.5, 500.5) && !hit(&rs, 200.5, 200.5));
        // Dikdörtgen sayısı makul: düz alanlar tek parça, yalnız eğri satırlar şerit.
        assert!(rs.len() < 200, "{} dikdörtgen", rs.len());
        assert_matches_reference(&f, 1000, 800, "ust-panel");
    }

    #[test]
    fn frame_only_is_a_handful_of_rects() {
        let f = frame_field(vec![]);
        let rs = f.input_region();
        // Üst ve alt bant, iki yan şerit, dört R köşesinin satırları (≈ 2 × 14 × 2).
        assert!(rs.len() <= 4 + 4 * 14, "{} dikdörtgen", rs.len());
        assert!(hit(&rs, 10.5, 40.5), "çerçeve iç köşesi R ile dolu");
        assert!(!hit(&rs, 30.5, 60.5));
        assert_matches_reference(&f, 1000, 800, "cerceve");
    }

    #[test]
    fn deformed_side_drawer_matches_reference() {
        // Shell'in sağ kenar çekmecesi esnerken (çapa sağ kenarın ortası): yapışık üst/alt
        // kenar çerçevede kalır, serbest sol kenar esner.
        for stretch in [0.9f32, 1.0, 1.15, 1.3] {
            let sh = BlobShape::new_rect(Rect::new(610.0, 40.0, 380.0, 750.0))
                .with_deform([stretch, 0.0, 0.0, 1.0 / stretch], Point::new(1.0, 0.5));
            let f = frame_field(vec![sh]);
            let rs = f.input_region();
            let left = 990.0 - 380.0 * stretch;
            // Yapışık kenar boyunca, panelin içinde, üst ve alt birleşimde boşluk yok.
            for y in [40.5, 41.5, 788.5, 789.5] {
                assert!(hit(&rs, left + 30.0, y), "esneme {stretch}: y {y}");
            }
            // Serbest kenarın hemen solu serbest, sağı dolu.
            assert!(!hit(&rs, left - 2.0, 400.5) && hit(&rs, left + 2.0, 400.5), "esneme {stretch}");
            assert_matches_reference(&f, 1000, 800, &format!("cekmece@{stretch}"));
        }
    }

    #[test]
    fn golden_scenes_match_reference() {
        for (name, shapes) in scenes() {
            assert_matches_reference(&field(shapes), 480, 300, name);
        }
    }

    #[test]
    fn frameless_field_covers_only_the_shapes() {
        let mut f = field(vec![BlobShape::new(Rect::new(170.0, 230.0, 140.0, 36.0), nuxut_core::Corners::uniform(18.0))]);
        f.frame = None;
        let rs = f.input_region();
        paint(&rs, 480, 400);
        assert!(hit(&rs, 240.5, 248.5) && !hit(&rs, 170.5, 230.5) && !hit(&rs, 100.5, 248.5));
        assert_matches_reference(&f, 480, 400, "cercevesiz-hap");
        f.shapes.clear();
        assert!(f.input_region().is_empty());
    }

    /// 3 panel / 1080p (bar'dan inen panel, sağ tam boy çekmece, sol alt köşe paneli).
    fn hd_field() -> BlobField {
        BlobField {
            frame: Some(FrameShape {
                outer: Rect::new(0.0, 0.0, 1920.0, 1080.0),
                inner: Rect::new(8.0, 40.0, 1904.0, 1032.0),
                inner_radius: 0.0,
            }),
            shapes: vec![
                BlobShape::new_rect(Rect::new(700.0, 36.0, 500.0, 364.0)),
                BlobShape::new_rect(Rect::new(1500.0, 40.0, 412.0, 1032.0)),
                BlobShape::new_rect(Rect::new(0.0, 820.0, 360.0, 260.0)),
            ],
            smoothing: 20.0,
            corner_radius: 12.0,
            fill: Color::BLACK, // renk-matematiği: girdi bölgesi testi rengi okumaz
            shadow: None,
        }
    }

    #[test]
    fn hd_three_panels_rect_count() {
        let f = hd_field();
        let rs = f.input_region();
        eprintln!("3 panel/1080p: {} dikdörtgen", rs.len());
        assert!(rs.len() < 400, "{} dikdörtgen", rs.len());
        assert_matches_reference(&f, 1920, 1080, "hd");
    }

    #[test]
    #[ignore = "profil"]
    fn profile_input_region() {
        let f = hd_field();
        let (scene, clip) = f.input_scene().unwrap();
        let n = 200;
        let t = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(morph::free_area(std::hint::black_box(&scene)));
        }
        let geom = t.elapsed() / n;
        let h = morph::free_area(&scene);
        let t = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(super::fill_rects(std::hint::black_box(&h), clip));
        }
        let rows = t.elapsed() / n;
        let t = std::time::Instant::now();
        for _ in 0..n {
            std::hint::black_box(std::hint::black_box(&f).input_region());
        }
        eprintln!(
            "3 panel/1080p: input_region {:?} (free_area {geom:?}, satırlar {rows:?}), {} ilkel, {} dikdörtgen",
            t.elapsed() / n,
            h.prims.len(),
            f.input_region().len()
        );
    }
}
