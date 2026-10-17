//! Blob geometrisinin f64 çekirdeği: doğru parçaları ve çember yaylarından oluşan sınırlar.
//!
//! Bölge, kapalı döngüler oluşturan ilkellerin kümesidir; içerisi çift-tek kuralıyla (yatay
//! ışın) belirlenir, bu yüzden ilkellerin yönü yoktur. Yaylar her zaman y'ye göre tekdüze
//! parçalara bölünür ([`Region::new`]); ışın kesişimi parçalarda yarı açık kuralla sayılır,
//! paylaşılan uç noktalar çift sayılmaz. Aynı kural shader'da da uygulanır.

use std::f64::consts::{FRAC_PI_2, TAU};
use std::ops::{Add, Mul, Neg, Sub};

/// Uzunluk ve konum toleransı (fiziksel piksel). Girdiler 1/256 px ızgarasına oturtulur;
/// bu değer ondan çok küçük, kayan nokta gürültüsünden çok büyüktür.
pub(crate) const EPS: f64 = 1e-7;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct V {
    pub x: f64,
    pub y: f64,
}

pub(crate) const fn v(x: f64, y: f64) -> V {
    V { x, y }
}

impl Add for V {
    type Output = V;
    fn add(self, o: V) -> V {
        v(self.x + o.x, self.y + o.y)
    }
}

impl Sub for V {
    type Output = V;
    fn sub(self, o: V) -> V {
        v(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f64> for V {
    type Output = V;
    fn mul(self, k: f64) -> V {
        v(self.x * k, self.y * k)
    }
}

impl Neg for V {
    type Output = V;
    fn neg(self) -> V {
        v(-self.x, -self.y)
    }
}

impl V {
    pub fn dot(self, o: V) -> f64 {
        self.x * o.x + self.y * o.y
    }

    pub fn cross(self, o: V) -> f64 {
        self.x * o.y - self.y * o.x
    }

    pub fn len(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// Birim vektör; sıfır vektörde sıfır.
    pub fn unit(self) -> V {
        let l = self.len();
        if l > 0.0 { self * (1.0 / l) } else { v(0.0, 0.0) }
    }

    /// Saat yönünün tersine 90° (y aşağı ekranda görsel olarak saat yönü; yön önemsiz).
    pub fn perp(self) -> V {
        v(-self.y, self.x)
    }

    pub fn dist(self, o: V) -> f64 {
        (self - o).len()
    }

    pub fn from_angle(a: f64) -> V {
        v(a.cos(), a.sin())
    }

    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }
}

/// `a`'yı `[0, 2π)` aralığına getirir.
pub(crate) fn wrap(a: f64) -> f64 {
    let w = a.rem_euclid(TAU);
    if w >= TAU { 0.0 } else { w }
}

/// Çember yayı: `c` merkezli, `r` yarıçaplı, `a0` açısından başlayıp artan açı yönünde
/// `sweep` (0 < sweep ≤ 2π) kadar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Arc {
    pub c: V,
    pub r: f64,
    pub a0: f64,
    pub sweep: f64,
}

impl Arc {
    pub fn circle(c: V, r: f64) -> Self {
        Self { c, r, a0: 0.0, sweep: TAU }
    }

    pub fn at_angle(&self, a: f64) -> V {
        self.c + V::from_angle(a) * self.r
    }

    /// Açının yaydaki parametresi (`0..=1`); yayın dışındaysa `None`.
    pub fn param_of_angle(&self, a: f64, tol: f64) -> Option<f64> {
        let d = wrap(a - self.a0);
        let tol_a = tol / self.r.max(EPS);
        if d <= self.sweep + tol_a {
            Some((d / self.sweep).min(1.0))
        } else if d >= TAU - tol_a {
            Some(0.0)
        } else {
            None
        }
    }

    pub fn is_full(&self) -> bool {
        self.sweep >= TAU - 1e-12
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Prim {
    Seg(V, V),
    Arc(Arc),
}

impl Prim {
    pub fn point(&self, t: f64) -> V {
        match self {
            Prim::Seg(a, b) => *a + (*b - *a) * t,
            Prim::Arc(arc) => arc.at_angle(arc.a0 + arc.sweep * t),
        }
    }

    pub fn start(&self) -> V {
        self.point(0.0)
    }

    pub fn end(&self) -> V {
        self.point(1.0)
    }

    pub fn length(&self) -> f64 {
        match self {
            Prim::Seg(a, b) => a.dist(*b),
            Prim::Arc(arc) => arc.r * arc.sweep,
        }
    }

    /// `t`'deki birim normal (yön işaretsiz; yan testleri iki yöne de bakar).
    pub fn normal(&self, t: f64) -> V {
        match self {
            Prim::Seg(a, b) => (*b - *a).perp().unit(),
            Prim::Arc(arc) => V::from_angle(arc.a0 + arc.sweep * t),
        }
    }

    /// `[t0, t1]` alt parçası.
    pub fn sub(&self, t0: f64, t1: f64) -> Prim {
        match self {
            Prim::Seg(..) => Prim::Seg(self.point(t0), self.point(t1)),
            Prim::Arc(arc) => Prim::Arc(Arc { c: arc.c, r: arc.r, a0: arc.a0 + arc.sweep * t0, sweep: arc.sweep * (t1 - t0) }),
        }
    }

    /// Noktaya en yakın uzaklık.
    pub fn dist(&self, p: V) -> f64 {
        match self {
            Prim::Seg(a, b) => seg_dist(*a, *b, p),
            Prim::Arc(arc) => {
                let d = p - arc.c;
                let l = d.len();
                // Yarım turdan kısa yayda açı aralığı iki çapraz çarpımla (atan2'siz).
                let within = if arc.sweep <= std::f64::consts::PI {
                    let (s, e) = (V::from_angle(arc.a0), V::from_angle(arc.a0 + arc.sweep));
                    s.cross(d) >= 0.0 && d.cross(e) >= 0.0
                } else {
                    arc.is_full() || arc.param_of_angle(d.angle(), 0.0).is_some()
                };
                if l > 0.0 && within {
                    (l - arc.r).abs()
                } else if l == 0.0 && arc.is_full() {
                    arc.r
                } else {
                    p.dist(self.start()).min(p.dist(self.end()))
                }
            }
        }
    }

    /// Eksen hizalı sınır kutusu `(min, max)`.
    pub fn bounds(&self) -> (V, V) {
        match self {
            Prim::Seg(a, b) => (v(a.x.min(b.x), a.y.min(b.y)), v(a.x.max(b.x), a.y.max(b.y))),
            Prim::Arc(arc) => {
                let (s, e) = (self.start(), self.end());
                let (mut lo, mut hi) = (v(s.x.min(e.x), s.y.min(e.y)), v(s.x.max(e.x), s.y.max(e.y)));
                for k in 0..4 {
                    let a = k as f64 * FRAC_PI_2;
                    if arc.is_full() || arc.param_of_angle(a, 0.0).is_some() {
                        let p = arc.at_angle(a);
                        lo = v(lo.x.min(p.x), lo.y.min(p.y));
                        hi = v(hi.x.max(p.x), hi.y.max(p.y));
                    }
                }
                (lo, hi)
            }
        }
    }

    /// Yatay ışın (p'den +x yönüne) bu ilkeli kesiyor mu. Yaylar y'ye göre tekdüze olmalı. `s`, `e`:
    /// bölgenin paylaşılan (yapıştırılmış) uç noktaları; komşu ilkellerde birebir aynı olmaları
    /// yarı açık kuralın çift saymamasının koşuludur.
    pub fn crosses_ray(&self, p: V, s: V, e: V) -> bool {
        self.line_x(p.y, s, e).is_some_and(|x| x > p.x)
    }

    /// `y` yüksekliğindeki yatay doğrunun bu ilkeli kestiği x; [`Prim::crosses_ray`] ile aynı
    /// yarı açık kural (`s`, `e` yapıştırılmış uçlar): kesişmiyorsa `None`.
    pub fn line_x(&self, y: f64, s: V, e: V) -> Option<f64> {
        if (s.y > y) == (e.y > y) {
            return None;
        }
        Some(match self {
            Prim::Seg(a, b) => a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y),
            Prim::Arc(arc) => {
                let side = (arc.a0 + arc.sweep * 0.5).cos().signum();
                let dy = y - arc.c.y;
                arc.c.x + side * (arc.r * arc.r - dy * dy).max(0.0).sqrt()
            }
        })
    }

    /// Yayı y'ye göre tekdüze parçalara böler (açı π/2 ve 3π/2'de); doğru parçası aynen.
    pub fn monotonic(&self, out: &mut Vec<Prim>) {
        let Prim::Arc(arc) = self else {
            out.push(*self);
            return;
        };
        let mut cuts: Vec<f64> = Vec::new();
        for base in [FRAC_PI_2, 3.0 * FRAC_PI_2] {
            let d = wrap(base - arc.a0);
            if d > 1e-12 && d < arc.sweep - 1e-12 {
                cuts.push(d / arc.sweep);
            }
        }
        cuts.sort_by(f64::total_cmp);
        let mut t0 = 0.0;
        for t in cuts.into_iter().chain([1.0]) {
            if t - t0 > 0.0 {
                out.push(self.sub(t0, t));
            }
            t0 = t;
        }
    }
}

pub(crate) fn seg_dist(a: V, b: V, p: V) -> f64 {
    let d = b - a;
    let l2 = d.dot(d);
    if l2 <= 0.0 {
        return p.dist(a);
    }
    let t = ((p - a).dot(d) / l2).clamp(0.0, 1.0);
    p.dist(a + d * t)
}

/// İki ilkelin kesişim noktaları, her birindeki parametreleriyle `(t_self, t_other)`.
/// Teğet temas tek nokta verir; çakışan doğrusal parçalar birbirinin uç noktalarını verir.
pub(crate) fn intersect(p: &Prim, q: &Prim) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    match (p, q) {
        (Prim::Seg(a, b), Prim::Seg(c, d)) => seg_seg(*a, *b, *c, *d, &mut out),
        (Prim::Seg(a, b), Prim::Arc(arc)) => seg_arc(*a, *b, arc, &mut out, false),
        (Prim::Arc(arc), Prim::Seg(a, b)) => seg_arc(*a, *b, arc, &mut out, true),
        (Prim::Arc(x), Prim::Arc(y)) => arc_arc(x, y, &mut out),
    }
    out
}

fn seg_seg(a: V, b: V, c: V, d: V, out: &mut Vec<(f64, f64)>) {
    let (r, s) = (b - a, d - c);
    let denom = r.cross(s);
    let (lr, ls) = (r.len(), s.len());
    if lr <= 0.0 || ls <= 0.0 {
        return;
    }
    if denom.abs() <= 1e-12 * lr * ls {
        // Paralel: yalnızca doğrudaşsa uç noktaları birbirine böl.
        if (c - a).cross(r).abs() / lr > EPS {
            return;
        }
        let on = |x: V, p0: V, p1: V| {
            let dd = p1 - p0;
            let t = (x - p0).dot(dd) / dd.dot(dd);
            (-1e-12..=1.0 + 1e-12).contains(&t).then_some(t.clamp(0.0, 1.0))
        };
        for x in [c, d] {
            if let (Some(t), Some(u)) = (on(x, a, b), on(x, c, d)) {
                out.push((t, u));
            }
        }
        for x in [a, b] {
            if let (Some(t), Some(u)) = (on(x, a, b), on(x, c, d)) {
                out.push((t, u));
            }
        }
        return;
    }
    let t = (c - a).cross(s) / denom;
    let u = (c - a).cross(r) / denom;
    let (te, ue) = (EPS / lr, EPS / ls);
    if t >= -te && t <= 1.0 + te && u >= -ue && u <= 1.0 + ue {
        out.push((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)));
    }
}

fn seg_arc(a: V, b: V, arc: &Arc, out: &mut Vec<(f64, f64)>, swap: bool) {
    let d = b - a;
    let f = a - arc.c;
    let (qa, qb, qc) = (d.dot(d), 2.0 * f.dot(d), f.dot(f) - arc.r * arc.r);
    if qa <= 0.0 {
        return;
    }
    let disc = qb * qb - 4.0 * qa * qc;
    // Teğet: ayırtaç gürültüden ötürü hafif negatif olabilir (uzunluk ölçeğinde EPS).
    let tol = 4.0 * qa * (2.0 * arc.r * EPS);
    let roots: Vec<f64> = if disc < -tol {
        return;
    } else if disc <= tol {
        vec![-qb / (2.0 * qa)]
    } else {
        let s = disc.sqrt();
        vec![(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)]
    };
    let te = EPS / qa.sqrt();
    for t in roots {
        if t < -te || t > 1.0 + te {
            continue;
        }
        let t = t.clamp(0.0, 1.0);
        let pt = a + d * t;
        if let Some(u) = arc.param_of_angle((pt - arc.c).angle(), EPS) {
            out.push(if swap { (u, t) } else { (t, u) });
        }
    }
}

fn arc_arc(x: &Arc, y: &Arc, out: &mut Vec<(f64, f64)>) {
    let dc = y.c - x.c;
    let dist = dc.len();
    if dist <= EPS {
        // Eş merkezli: yalnızca aynı çemberdeyse (çakışan yaylar) uçlar birbirine bölünür.
        if (x.r - y.r).abs() <= EPS {
            for (from, to, flip) in [(x, y, false), (y, x, true)] {
                for a in [from.a0, from.a0 + from.sweep] {
                    if let (Some(t), Some(u)) = (from.param_of_angle(a, EPS), to.param_of_angle(a, EPS)) {
                        out.push(if flip { (u, t) } else { (t, u) });
                    }
                }
            }
        }
        return;
    }
    let (r0, r1) = (x.r, y.r);
    if dist > r0 + r1 + EPS || dist < (r0 - r1).abs() - EPS {
        return;
    }
    let a = (r0 * r0 - r1 * r1 + dist * dist) / (2.0 * dist);
    let h2 = r0 * r0 - a * a;
    let base = x.c + dc * (a / dist);
    let pts: Vec<V> = if h2 <= 2.0 * r0 * EPS {
        vec![base]
    } else {
        let h = h2.sqrt();
        let n = dc.perp() * (h / dist);
        vec![base + n, base - n]
    };
    for pt in pts {
        if let (Some(t), Some(u)) =
            (x.param_of_angle((pt - x.c).angle(), EPS), y.param_of_angle((pt - y.c).angle(), EPS))
        {
            out.push((t, u));
        }
    }
}

/// Kapalı döngülerden oluşan bölge; içerisi çift-tek kuralıyla.
#[derive(Debug, Clone, Default)]
pub(crate) struct Region {
    pub prims: Vec<Prim>,
    /// Her ilkelin uç noktaları, paylaşılan köşelerde tek bir temsilciye yapıştırılmış
    /// (yay uçları açıdan hesaplandığı için komşu parçayla bit düzeyinde eşit değildir).
    pub ends: Vec<(V, V)>,
}

impl Region {
    /// İlkelleri y'ye göre tekdüze parçalara böler, sıfır uzunluklu parçaları atar, aynı
    /// doğru üzerinde uç uca gelen parçaları birleştirir ve uçları yapıştırır.
    pub fn new(prims: impl IntoIterator<Item = Prim>) -> Self {
        let mut out = Vec::new();
        for p in prims {
            if p.length() > EPS {
                p.monotonic(&mut out);
            }
        }
        out.retain(|p| p.length() > EPS);
        merge_collinear(&mut out);
        // Temsilci ikili ızgaraya (2⁻²⁴ px) yuvarlanır: yay uçları trigonometriden gelir;
        // yatay bir parçanın iki ucu 58.499999999999986 ve 58.500000000000014 olursa tam
        // y = 58.5'teki ışın onu yanlışlıkla keser. Yuvarlamayla ikisi de 58.5 olur.
        let mut reps: PointMap<()> = PointMap::new(1e-6);
        let mut snap = |q: V| match reps.find(q) {
            Some((r, _)) => *r,
            None => {
                const K: f64 = (1u64 << 24) as f64;
                let r = v((q.x * K).round() / K, (q.y * K).round() / K);
                reps.insert(r, ());
                r
            }
        };
        let ends = out.iter().map(|p| (snap(p.start()), snap(p.end()))).collect();
        Self { prims: out, ends }
    }

    pub fn is_empty(&self) -> bool {
        self.prims.is_empty()
    }

    pub fn inside(&self, p: V) -> bool {
        self.prims.iter().zip(&self.ends).filter(|(q, (s, e))| q.crosses_ray(p, *s, *e)).count() % 2 == 1
    }

    #[cfg(test)]
    pub fn dist(&self, p: V) -> f64 {
        self.prims.iter().map(|q| q.dist(p)).fold(f64::INFINITY, f64::min)
    }

    /// İşaretli uzaklık: içeride negatif.
    #[cfg(test)]
    pub fn sdf(&self, p: V) -> f64 {
        let d = self.dist(p);
        if self.inside(p) { -d } else { d }
    }
}

/// Küçük tamsayı anahtarlar için hızlı karma (FxHash biçimi; std'nin SipHash'i burada gereksiz
/// pahalı, anahtarlar saldırgan girdi değil).
#[derive(Default)]
pub(crate) struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }

    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_i64(&mut self, x: i64) {
        self.write_u64(x as u64);
    }
}

pub(crate) type FxMap<K, T> = std::collections::HashMap<K, T, std::hash::BuildHasherDefault<Fx>>;

/// Aynı doğru üzerinde uç uca eklenen doğru parçalarını tek parçada birleştirir (bölme ve
/// kırpma ara noktalar bırakır; sırt araması tek parça bekler). Doğrular anahtarla gruplanır,
/// grup içinde izdüşüme göre sıralanır. Çakışan (üst üste binen) parçalar birleştirilmez: çift-tek
/// kuralında ikisi birlikte anlam taşır.
fn merge_collinear(prims: &mut Vec<Prim>) {
    // Doğru anahtarı → (izdüşüm başı, sonu, yön, doğrunun uzaklığı).
    type Span = (f64, f64, V, f64);
    let mut lines: FxMap<(i64, i64, i64), Vec<Span>> = FxMap::default();
    let mut out = Vec::with_capacity(prims.len());
    for p in prims.drain(..) {
        let Prim::Seg(a, b) = p else {
            out.push(p);
            continue;
        };
        let mut d = (b - a).unit();
        if d.x < 0.0 || (d.x == 0.0 && d.y < 0.0) {
            d = -d;
        }
        let c = d.cross(a);
        let key = ((d.x * 1e9).round() as i64, (d.y * 1e9).round() as i64, (c * 1e6).round() as i64);
        let (ta, tb) = (a.dot(d), b.dot(d));
        lines.entry(key).or_default().push((ta.min(tb), ta.max(tb), d, c));
    }
    for mut segs in lines.into_values() {
        segs.sort_by(|x, y| x.0.total_cmp(&y.0));
        let at = |t: f64, d: V, c: f64| d * t + d.perp() * c;
        let mut cur = segs[0];
        for s in segs.into_iter().skip(1) {
            if (s.0 - cur.1).abs() <= EPS {
                cur.1 = s.1;
            } else {
                out.push(Prim::Seg(at(cur.0, cur.2, cur.3), at(cur.1, cur.2, cur.3)));
                cur = s;
            }
        }
        out.push(Prim::Seg(at(cur.0, cur.2, cur.3), at(cur.1, cur.2, cur.3)));
    }
    *prims = out;
}

/// Toleransla nokta eşleme (karma ızgara; komşu hücreler de bakılır).
pub(crate) struct PointMap<T> {
    tol: f64,
    cells: FxMap<(i64, i64), Vec<(V, T)>>,
}

impl<T> PointMap<T> {
    pub fn new(tol: f64) -> Self {
        Self { tol, cells: FxMap::default() }
    }

    fn key(&self, p: V) -> (i64, i64) {
        ((p.x / self.tol).floor() as i64, (p.y / self.tol).floor() as i64)
    }

    /// `p`'ye `tol`'dan yakın kayıt.
    pub fn find(&mut self, p: V) -> Option<&mut (V, T)> {
        let (kx, ky) = self.key(p);
        let tol = self.tol;
        let mut hit = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(v) = self.cells.get(&(kx + dx, ky + dy))
                    && let Some(i) = v.iter().position(|(q, _)| q.dist(p) <= tol)
                {
                    hit = Some((kx + dx, ky + dy, i));
                    break 'search;
                }
            }
        }
        let (cx, cy, i) = hit?;
        self.cells.get_mut(&(cx, cy)).map(|v| &mut v[i])
    }

    pub fn insert(&mut self, p: V, t: T) {
        let k = self.key(p);
        self.cells.entry(k).or_default().push((p, t));
    }

    pub fn into_values(self) -> impl Iterator<Item = (V, T)> {
        self.cells.into_values().flatten()
    }
}

/// Sınır kutularına göre düzgün ızgara dizini (CSR): komşuluk sorguları yalnızca ilgili
/// hücrelerdeki ilkelleri verir (her biri bir kez).
pub(crate) struct Grid {
    origin: V,
    cell: f64,
    cols: usize,
    rows: usize,
    starts: Vec<u32>,
    items: Vec<u32>,
    boxes: Vec<(V, V)>,
    stamp: std::cell::RefCell<(u32, Vec<u32>)>,
}

impl Grid {
    pub fn new(boxes: Vec<(V, V)>, cell: f64) -> Self {
        let (mut lo, mut hi) = (v(f64::MAX, f64::MAX), v(f64::MIN, f64::MIN));
        for (a, b) in &boxes {
            lo = v(lo.x.min(a.x), lo.y.min(a.y));
            hi = v(hi.x.max(b.x), hi.y.max(b.y));
        }
        if boxes.is_empty() {
            (lo, hi) = (v(0.0, 0.0), v(0.0, 0.0));
        }
        // Hücre sayısı sınırlı: büyük alanda hücre büyür.
        let cell = cell.max((hi.x - lo.x).max(hi.y - lo.y) / 64.0);
        let cols = ((hi.x - lo.x) / cell).floor() as usize + 1;
        let rows = ((hi.y - lo.y) / cell).floor() as usize + 1;
        let mut g = Self {
            origin: lo,
            cell,
            cols,
            rows,
            starts: vec![0; cols * rows + 1],
            items: Vec::new(),
            boxes: Vec::new(),
            stamp: std::cell::RefCell::new((0, vec![0; boxes.len()])),
        };
        let spans: Vec<_> = boxes.iter().map(|(a, b)| g.span(*a, *b)).collect();
        for &(c0, r0, c1, r1) in &spans {
            for r in r0..=r1 {
                for c in c0..=c1 {
                    g.starts[r * cols + c + 1] += 1;
                }
            }
        }
        for k in 1..g.starts.len() {
            g.starts[k] += g.starts[k - 1];
        }
        let mut fill = g.starts.clone();
        g.items = vec![0; *g.starts.last().unwrap() as usize];
        for (i, &(c0, r0, c1, r1)) in spans.iter().enumerate() {
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let slot = &mut fill[r * cols + c];
                    g.items[*slot as usize] = i as u32;
                    *slot += 1;
                }
            }
        }
        g.boxes = boxes;
        g
    }

    fn span(&self, a: V, b: V) -> (usize, usize, usize, usize) {
        let cl = |x: f64, n: usize| (x.floor().max(0.0) as usize).min(n - 1);
        let ((ax, ay), (bx, by)) = (
            ((a.x - self.origin.x) / self.cell, (a.y - self.origin.y) / self.cell),
            ((b.x - self.origin.x) / self.cell, (b.y - self.origin.y) / self.cell),
        );
        (cl(ax, self.cols), cl(ay, self.rows), cl(bx, self.cols), cl(by, self.rows))
    }

    /// Kutusu `[lo, hi]` ile kesişen ilkellerin dizinleri.
    pub fn query(&self, lo: V, hi: V, mut f: impl FnMut(usize)) {
        let mut st = self.stamp.borrow_mut();
        st.0 = st.0.wrapping_add(1);
        let tag = st.0;
        let (c0, r0, c1, r1) = self.span(lo, hi);
        for r in r0..=r1 {
            for c in c0..=c1 {
                let k = r * self.cols + c;
                for &i in &self.items[self.starts[k] as usize..self.starts[k + 1] as usize] {
                    let i = i as usize;
                    if st.1[i] == tag {
                        continue;
                    }
                    st.1[i] = tag;
                    let (a, b) = self.boxes[i];
                    if a.x <= hi.x + EPS && lo.x <= b.x + EPS && a.y <= hi.y + EPS && lo.y <= b.y + EPS {
                        f(i);
                    }
                }
            }
        }
    }
}

/// Bölgenin hızlı sorguları: noktaya `lim`'den yakın ilkel var mı ve çift-tek içerisi (yalnızca
/// noktanın y bandındaki ilkeller).
pub(crate) struct Indexed<'a> {
    pub region: &'a Region,
    grid: Grid,
}

impl<'a> Indexed<'a> {
    pub fn new(region: &'a Region) -> Self {
        let grid = Grid::new(region.prims.iter().map(Prim::bounds).collect(), 48.0);
        Self { region, grid }
    }

    /// `p`'ye `lim`'den daha yakın bir ilkel var mı (`skip` hariç).
    pub fn any_closer(&self, p: V, lim: f64, skip: &[usize]) -> bool {
        let mut hit = false;
        let r = v(lim, lim);
        self.grid.query(p - r, p + r, |i| {
            if !hit && !skip.contains(&i) && self.region.prims[i].dist(p) < lim {
                hit = true;
            }
        });
        hit
    }

    pub fn inside(&self, p: V) -> bool {
        let mut n = 0;
        let far = v(f64::MAX / 4.0, p.y);
        self.grid.query(p, far, |i| {
            let (s, e) = self.region.ends[i];
            if self.region.prims[i].crosses_ray(p, s, e) {
                n += 1;
            }
        });
        n % 2 == 1
    }
}

/// İki ilkel aynı nokta kümesi mi (yön önemsiz).
pub(crate) fn same_prim(p: &Prim, q: &Prim) -> bool {
    const T: f64 = 1e-6;
    match (p, q) {
        (Prim::Seg(a, b), Prim::Seg(c, d)) => (a.dist(*c) < T && b.dist(*d) < T) || (a.dist(*d) < T && b.dist(*c) < T),
        (Prim::Arc(x), Prim::Arc(y)) => {
            x.c.dist(y.c) < T
                && (x.r - y.r).abs() < T
                && (x.sweep - y.sweep).abs() * x.r < T
                && p.start().dist(q.start()) < T
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, s: f64) -> Region {
        let (a, b, c, d) = (v(x0, y0), v(x0 + s, y0), v(x0 + s, y0 + s), v(x0, y0 + s));
        Region::new([Prim::Seg(a, b), Prim::Seg(b, c), Prim::Seg(c, d), Prim::Seg(d, a)])
    }

    #[test]
    fn square_sdf() {
        let r = square(0.0, 0.0, 10.0);
        assert!(r.inside(v(5.0, 5.0)));
        assert!(!r.inside(v(15.0, 5.0)));
        assert!((r.sdf(v(5.0, 5.0)) + 5.0).abs() < 1e-12);
        assert!((r.sdf(v(13.0, 14.0)) - 5.0).abs() < 1e-12);
        // Köşe hizasında ışın: tepe noktası paylaşılan uçta çift sayılmaz.
        assert!(!r.inside(v(-1.0, 0.0)) && !r.inside(v(-1.0, 10.0)));
    }

    #[test]
    fn circle_region_and_monotonic_split() {
        let r = Region::new([Prim::Arc(Arc::circle(v(0.0, 0.0), 2.0))]);
        assert_eq!(r.prims.len(), 3, "tam çember tepe ve dipte bölünür");
        assert!(r.inside(v(0.0, 0.0)) && r.inside(v(1.9, 0.0)) && !r.inside(v(2.1, 0.0)));
        assert!(!r.inside(v(0.0, 2.0)) || r.inside(v(0.0, 1.99)));
        assert!((r.sdf(v(0.0, 0.0)) + 2.0).abs() < 1e-12);
        assert!((r.sdf(v(3.0, 4.0)) - 3.0).abs() < 1e-12);
    }

    #[test]
    fn intersections() {
        let s = Prim::Seg(v(-5.0, 0.0), v(5.0, 0.0));
        let c = Prim::Arc(Arc::circle(v(0.0, 0.0), 3.0));
        assert_eq!(intersect(&s, &c).len(), 2);
        let tangent = Prim::Seg(v(-5.0, 3.0), v(5.0, 3.0));
        assert_eq!(intersect(&tangent, &c).len(), 1);
        let c2 = Prim::Arc(Arc::circle(v(4.0, 0.0), 1.0));
        assert_eq!(intersect(&c, &c2).len(), 1, "dıştan teğet");
        let collinear = Prim::Seg(v(2.0, 0.0), v(8.0, 0.0));
        assert_eq!(intersect(&s, &collinear).len(), 2, "çakışan parçalar uçları verir");
    }

    #[test]
    fn collinear_pieces_merge() {
        let r = Region::new([Prim::Seg(v(0.0, 0.0), v(1.0, 0.0)), Prim::Seg(v(1.0, 0.0), v(3.0, 0.0))]);
        assert_eq!(r.prims, vec![Prim::Seg(v(0.0, 0.0), v(3.0, 0.0))]);
    }
}
