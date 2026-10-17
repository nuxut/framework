//! ADR-0008 geometrisi: serbest alan `F`'den çizilen serbest alan `H`.
//!
//! `H = closing(opening(F, R), r)`; birleşimler `R` yarıçaplı içbükey filetoya, boşluğa açılan
//! köşeler `r` yarıçaplı dışbükey yaya dönüşür. Morfoloji kesindir: aşındırma ve genişletme,
//! işaretli uzaklığın `∓ρ` düzey kümesidir ve doğru parçası + yay ofsetlerinin kırpılmasıyla
//! hesaplanır ([`offset`]).
//!
//! Süreklilik düzenlemesi (ADR-0008 kararları): kesin morfoloji, uzaklık alanının bir sırtı
//! (karşılıklı paralel iki kenar arasındaki orta çizgi) yarıçapı geçtiği anda sıçrar: boğaz
//! bir karede kapanır, ince panel bir karede silinir. Bu yüzden yarıçaptan dar sırtlar korunur
//! ([`ridges`]): dar boğaz, tüm örtüşme boyunca birlikte incelen bir cep olarak sıfıra iner
//! ([`free_area`], [`pocket_half`]); ince panel ucu yuvarlak bir kapsül olarak kalır.

use super::geom::{EPS, Prim, Region, V, intersect, same_prim, v};

/// Yan testlerinde normal boyunca sapma (girdiler 1/256 px ızgarasında).
const SIDE_EPS: f64 = 1e-5;
/// Düzey kümesi doğrulama toleransı.
const LEVEL_TOL: f64 = 1e-6;

/// Bir kare için girdi: fiziksel piksel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Scene {
    /// Çerçevenin iç dikdörtgeni `(x0, y0, x1, y1)`; dışı dolu.
    pub inner: (f64, f64, f64, f64),
    /// Dolu paneller.
    pub panels: Vec<Panel>,
    /// Birleşim yarıçapı.
    pub join: f64,
    /// Panel köşe yarıçapı.
    pub corner: f64,
}

/// Çizilecek serbest alan `H`.
///
/// 1. `O = opening(F, R)` kesin. Dar boğazı (aralık `d < 2R`) tümüyle doldurur; ağızlarda
///    top panel yanına ve duvara yaslanır, birleşmiş hâlin filetosu kendiliğinden oluşur.
/// 2. `C = closing(O, r)` kesin; boğaz cepleri burada YOK. Kıl kadar ince bir yarık bile `r`
///    topunun geçişini keserdi; panel değdiği anda köşe sıçrardı.
/// 3. Boğaz cebi: sırt boyunca ortalanmış, tüm örtüşme aralığında birlikte incelen şerit,
///    yarı kalınlığı `h(d) = d/2 · smoothstep((d − R)/R)`: `d = 2R`'de kanalın tamamı, `d ≤ R`'de
///    sıfır (yan yana paneller tek parça).
/// 4. Cep birleşimleri `ρ = min(r, 4·en ince cebin h)` ile kapatılır: `d = 2R`'de tam `r`
///    yuvarlaması geri gelir (açık kanalla sürekli), `d → 0`'da etkisi kendiliğinden kaybolur.
/// 5. İnce dolu şekiller (kalınlığı `2r`'den az) yarı kalınlık yarıçaplı kapsül olarak korunur.
pub(crate) fn free_area(scene: &Scene) -> Region {
    let f = free_space(scene);
    if f.is_empty() {
        return f;
    }
    let (join, corner) = (scene.join, scene.corner);
    let o = if join > EPS { opening(&f, join) } else { f.clone() };
    let pockets: Vec<(Ridge, f64)> = if join > EPS {
        ridges(&f, true, 2.0 * join).into_iter().map(|r| (r, pocket_half(r.half, join))).filter(|(_, h)| *h > EPS).collect()
    } else {
        Vec::new()
    };
    if o.is_empty() && pockets.is_empty() {
        return o;
    }
    let c = if corner > EPS && !o.is_empty() { closing(&o, corner) } else { o.clone() };
    let index = super::geom::Indexed::new(&f);
    let reach = 2.0 * (join + corner);
    let mut h = pockets.iter().fold(c, |acc, (r, half)| {
        let (a, b) = pocket_ends(&index, r, *half, reach);
        boolean(&acc, &capsule(a, b, *half), |x, y| x || y)
    });
    let rho = pockets.iter().map(|(_, half)| 4.0 * half).fold(corner, f64::min);
    if !pockets.is_empty() && rho > EPS {
        h = closing(&h, rho);
    }
    if corner <= EPS || o.is_empty() {
        return h;
    }
    ridges(&o, false, 2.0 * corner)
        .into_iter()
        .map(|r| capsule(r.a, r.b, r.half))
        .fold(h, |acc, cap| boolean(&acc, &cap, |a, b| a && !b))
}

/// Cebin uçları: açık uçta (boğazın ağzı) şerit dışarı, serbest alana çıkana dek uzar (en çok
/// `reach`, yolu `half`'tan yakın bir kenara değmeden). Uzamasa şerit ağızdaki filetonun içinde
/// biterdi; duvar tarafında kalan ince dolu bant filetoya bir tümsekle bağlanırdı.
fn pocket_ends(f: &super::geom::Indexed, r: &Ridge, half: f64, reach: f64) -> (V, V) {
    let len = r.a.dist(r.b);
    if len <= EPS {
        return (r.a, r.b);
    }
    let dir = (r.b - r.a) * (1.0 / len);
    let clear = |p: V| !f.any_closer(p, half - LEVEL_TOL, &[]) && f.inside(p);
    let extend = |from: V, d: V| {
        true_intervals(0.0, reach, |t| clear(from + d * t)).first().filter(|(t0, _)| *t0 <= 1e-9).map_or(0.0, |(_, t1)| *t1)
    };
    let a = if r.open[0] { r.a - dir * extend(r.a, -dir) } else { r.a };
    let b = if r.open[1] { r.b + dir * extend(r.b, dir) } else { r.b };
    (a, b)
}

/// Boğaz cebinin yarı kalınlığı (ADR-0008 karar 1), `d = 2·half`: `d = 2R`'de kanalın tamamı,
/// `d` küçüldükçe smoothstep ile incelir ve `d ≤ R`'de tümüyle kaybolur. Yan yana duran ya da
/// birbirine `R`'den yakın paneller tek parça görünür; kıl kadar ince yarık kalmaz.
pub(crate) fn pocket_half(half: f64, join: f64) -> f64 {
    let x = ((2.0 * half - join) / join).clamp(0.0, 1.0);
    half * x * x * (3.0 - 2.0 * x)
}

/// `F` = iç dikdörtgen eksi paneller; sınırı, her kenar parçasının iki yanından yalnız
/// birinin serbest olduğu parçalardır.
pub(crate) fn free_space(scene: &Scene) -> Region {
    let (x0, y0, x1, y1) = scene.inner;
    if x1 - x0 <= EPS || y1 - y0 <= EPS {
        return Region::default();
    }
    let rect = [v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)];
    let shapes: Vec<Region> =
        scene.panels.iter().filter(|p| quad_area(&p.pts) > EPS).map(|p| Region::new(p.prims())).collect();
    let inside = |p: V| in_quad(&rect, p) && !shapes.iter().any(|s| s.inside(p));
    let edges: Vec<Prim> = (0..4)
        .map(|i| Prim::Seg(rect[i], rect[(i + 1) % 4]))
        .chain(shapes.iter().flat_map(|s| s.prims.iter().copied()))
        .collect();
    let mut kept: Vec<Prim> = Vec::new();
    for (i, e) in edges.iter().enumerate() {
        let mut ts = vec![0.0, 1.0];
        for (j, o) in edges.iter().enumerate() {
            if i != j && boxes_touch(e, o) {
                ts.extend(intersect(e, o).into_iter().map(|(t, _)| t));
            }
        }
        for piece in pieces(e, ts) {
            if is_boundary(&piece, &inside) && !kept.iter().any(|k| same_prim(k, &piece)) {
                kept.push(piece);
            }
        }
    }
    Region::new(kept)
}

/// Dolu panel: dışbükey dörtgen (deform uygulanmış dikdörtgen) ve köşe başına kendi yuvarlaması
/// (ör. OSD hapı). Kendi yuvarlaması `r` kapanışından bağımsızdır; büyük olan görünür.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Panel {
    pub pts: [V; 4],
    pub radii: [f64; 4],
}

impl Panel {
    #[cfg(test)]
    pub fn sharp(pts: [V; 4]) -> Self {
        Self { pts, radii: [0.0; 4] }
    }

    /// Sınır ilkelleri: köşe yayları kenarlara teğet (yarıçap, kenarın yarısına sığacak kadar
    /// kırpılır) ve aradaki doğru parçaları.
    pub fn prims(&self) -> Vec<Prim> {
        use super::geom::{Arc, wrap};
        let p = &self.pts;
        // Köşe başına (teğet uzunluğu, yay).
        let corners: Vec<(f64, Option<Arc>)> = (0..4)
            .map(|i| {
                let (prev, at, next) = (p[(i + 3) % 4], p[i], p[(i + 1) % 4]);
                let (u, w) = ((prev - at).unit(), (next - at).unit());
                let half = u.dot(w).clamp(-1.0, 1.0).acos() * 0.5;
                if self.radii[i] <= EPS || half <= 1e-6 {
                    return (0.0, None);
                }
                let fit = 0.5 * prev.dist(at).min(next.dist(at)) * half.tan();
                let rho = self.radii[i].min(fit);
                let t = rho / half.tan();
                let c = at + (u + w).unit() * (rho / half.sin());
                let (a1, a2) = ((at + u * t - c).angle(), (at + w * t - c).angle());
                let s = wrap(a2 - a1);
                let arc = if s <= std::f64::consts::PI { Arc { c, r: rho, a0: a1, sweep: s } } else { Arc { c, r: rho, a0: a2, sweep: wrap(a1 - a2) } };
                (t, Some(arc))
            })
            .collect();
        let mut out = Vec::new();
        for i in 0..4 {
            let (a, b) = (p[i], p[(i + 1) % 4]);
            let d = (b - a).unit();
            out.push(Prim::Seg(a + d * corners[i].0, b - d * corners[(i + 1) % 4].0));
            if let Some(arc) = corners[i].1 {
                out.push(Prim::Arc(arc));
            }
        }
        out
    }
}

fn quad_area(q: &[V; 4]) -> f64 {
    ((q[1] - q[0]).cross(q[2] - q[0]) + (q[2] - q[0]).cross(q[3] - q[0])).abs() * 0.5
}

/// Dışbükey dörtgenin (iki yönde de) kesin içi.
fn in_quad(q: &[V; 4], p: V) -> bool {
    let s: Vec<f64> = (0..4).map(|i| (q[(i + 1) % 4] - q[i]).cross(p - q[i])).collect();
    s.iter().all(|&x| x > 0.0) || s.iter().all(|&x| x < 0.0)
}

fn boxes_touch(a: &Prim, b: &Prim) -> bool {
    let ((a0, a1), (b0, b1)) = (a.bounds(), b.bounds());
    a0.x <= b1.x + EPS && b0.x <= a1.x + EPS && a0.y <= b1.y + EPS && b0.y <= a1.y + EPS
}

/// İlkeli verilen parametrelerde parçalara böler (sıfır uzunluklular atılır).
fn pieces(p: &Prim, mut ts: Vec<f64>) -> Vec<Prim> {
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    ts.windows(2).map(|w| p.sub(w[0], w[1])).filter(|q| q.length() > EPS).collect()
}

/// Parçanın orta noktasında iki yan farklı mı (biri içeride, biri dışarıda).
fn is_boundary(piece: &Prim, inside: &impl Fn(V) -> bool) -> bool {
    let m = piece.point(0.5);
    let n = piece.normal(0.5) * SIDE_EPS;
    inside(m + n) != inside(m - n)
}

/// Bölgenin `rho` kadar genişletilmişi (`grow`) ya da aşındırılmışı: işaretli uzaklığın
/// `±rho` düzey kümesi. Adaylar her ilkelin iki yana ofseti ve her köşenin `rho` çemberidir;
/// adaylar birbirleriyle kesiştikleri yerde bölünür, orta noktası tam `±rho` uzaklıkta
/// (başka bir ilkel daha yakın değil, doğru yanda) olan parçalar kalır.
pub(crate) fn offset(x: &Region, rho: f64, grow: bool) -> Region {
    if x.is_empty() || rho <= EPS {
        return x.clone();
    }
    let src = super::geom::Indexed::new(x);
    // Yalnızca ofset yönüne bakan yan katkı verebilir: aşındırmada içeri, genişletmede dışarı.
    // Normalin hangi yanının içeride olduğu orta noktanın hemen yanında tek bir testle bulunur.
    let mut cands: Vec<Prim> = Vec::new();
    for p in &x.prims {
        let (m, n) = (p.point(0.5), p.normal(0.5));
        let toward = if src.inside(m + n * SIDE_EPS * 10.0) != grow { 1.0 } else { -1.0 };
        match p {
            Prim::Seg(a, b) => {
                let n = (*b - *a).perp().unit() * (rho * toward);
                cands.push(Prim::Seg(*a + n, *b + n));
            }
            Prim::Arc(arc) => {
                let r = arc.r + rho * toward;
                if r > EPS {
                    cands.push(Prim::Arc(super::geom::Arc { r, ..*arc }));
                }
            }
        }
    }
    // Köşe çemberleri yalnızca kırık köşelerde (teğet birleşimde ofsetler zaten buluşur); her
    // birine, köşede biten ilkellerin normal yönlerinde zorunlu bölme.
    let mut verts: super::geom::PointMap<Vec<V>> = super::geom::PointMap::new(1e-6);
    for p in &x.prims {
        for (pt, t) in [(p.start(), 0.0), (p.end(), 1.0)] {
            let n = p.normal(t);
            match verts.find(pt) {
                Some((_, ns)) => ns.push(n),
                None => verts.insert(pt, vec![n]),
            }
        }
    }
    let corners: Vec<(V, Vec<f64>)> = verts
        .into_values()
        .filter(|(_, ns)| ns.iter().any(|a| ns.iter().any(|b| a.cross(*b).abs() > 1e-9)))
        .map(|(c, ns)| (c, ns.iter().flat_map(|n| [n.angle(), (-*n).angle()]).collect()))
        .collect();
    let first_corner = cands.len();
    for (c, _) in &corners {
        cands.push(Prim::Arc(super::geom::Arc::circle(*c, rho)));
    }
    let mut out: Vec<Prim> = Vec::new();
    let cand_grid = super::geom::Grid::new(cands.iter().map(Prim::bounds).collect(), 48.0);
    // Aday parçalar kaynağından tam rho uzaklıkta kurulur; geçerli olması için başka hiçbir
    // ilkel daha yakın olmamalı ve nokta doğru yanda olmalı (`|sdf − hedef| ≤ tol` ile aynı,
    // erken çıkışlı, ızgarayla yalnızca komşu ilkeller).
    let lim = rho - LEVEL_TOL;
    let valid = |m: V| !src.any_closer(m, lim, &[]) && src.inside(m) != grow;
    for (i, cand) in cands.iter().enumerate() {
        let mut ts = vec![0.0, 1.0];
        if i >= first_corner {
            ts.extend(corners[i - first_corner].1.iter().map(|a| super::geom::wrap(*a) / std::f64::consts::TAU));
        }
        let (a0, a1) = cand.bounds();
        cand_grid.query(a0, a1, |j| {
            if i != j {
                ts.extend(intersect(cand, &cands[j]).into_iter().map(|(t, _)| t));
            }
        });
        for piece in pieces(cand, ts) {
            if valid(piece.point(0.5)) {
                out.push(piece);
            }
        }
    }
    // Çakışan iki parça sıfır kalınlıklı bir şerittir (ör. tam 2ρ'lik boğaz): alanı yok, ikisi
    // de atılır.
    let mut keep = vec![true; out.len()];
    let mut seen: super::geom::PointMap<Vec<usize>> = super::geom::PointMap::new(1e-6);
    for (i, p) in out.iter().enumerate() {
        let m = p.point(0.5);
        match seen.find(m) {
            Some((_, idx)) => {
                if let Some(&j) = idx.iter().find(|&&j| keep[j] && same_prim(&out[j], p)) {
                    keep[i] = false;
                    keep[j] = false;
                } else {
                    idx.push(i);
                }
            }
            None => seen.insert(m, vec![i]),
        }
    }
    Region::new(out.into_iter().zip(keep).filter_map(|(p, k)| k.then_some(p)))
}

pub(crate) fn opening(x: &Region, rho: f64) -> Region {
    let e = offset(x, rho, false);
    if e.is_empty() { e } else { offset(&e, rho, true) }
}

pub(crate) fn closing(x: &Region, rho: f64) -> Region {
    offset(&offset(x, rho, true), rho, false)
}

/// İki bölgenin boole işlemi (`keep(a_içinde, b_içinde)`): ilkeller birbirleriyle kesiştikleri
/// yerde bölünür, iki yanında sonucu değişen parçalar kalır; çakışan kopyalardan biri kalır.
pub(crate) fn boolean(a: &Region, b: &Region, keep: impl Fn(bool, bool) -> bool) -> Region {
    let inside = |p: V| keep(a.inside(p), b.inside(p));
    let mut out: Vec<Prim> = Vec::new();
    for (mine, theirs) in [(a, b), (b, a)] {
        for p in &mine.prims {
            let mut ts = vec![0.0, 1.0];
            for o in &theirs.prims {
                if boxes_touch(p, o) {
                    ts.extend(intersect(p, o).into_iter().map(|(t, _)| t));
                }
            }
            for piece in pieces(p, ts) {
                if is_boundary(&piece, &inside) && !out.iter().any(|k| same_prim(k, &piece)) {
                    out.push(piece);
                }
            }
        }
    }
    Region::new(out)
}

/// Karşılıklı paralel iki kenar arasındaki sırt parçası: merkez doğrusu `a → b`, top
/// yarıçapı `half` (= aralık / 2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ridge {
    pub a: V,
    pub b: V,
    pub half: f64,
    /// O uç kenarların bittiği yere (boğazın ağzına) kadar uzanıyor mu; değilse başka bir
    /// kenar kesmiştir (kapalı uç).
    pub open: [bool; 2],
}

/// `x`'in serbest (`free`) ya da dolu yanında, aralığı `max_gap`'ten küçük ya da eşit olan
/// paralel kenar çiftlerinin sırtları. Sırt, toplarının başka hiçbir ilkele değmediği orta
/// çizgi parçalarıdır.
pub(crate) fn ridges(x: &Region, free: bool, max_gap: f64) -> Vec<Ridge> {
    let segs: Vec<(usize, V, V)> = x
        .prims
        .iter()
        .enumerate()
        .filter_map(|(i, p)| if let Prim::Seg(a, b) = p { Some((i, *a, *b)) } else { None })
        .collect();
    let index = super::geom::Indexed::new(x);
    let mut out = Vec::new();
    for (k, &(i, a, b)) in segs.iter().enumerate() {
        let dir = (b - a).unit();
        let len = a.dist(b);
        for &(j, c, d) in &segs[k + 1..] {
            if dir.cross((d - c).unit()).abs() > 1e-9 {
                continue;
            }
            let gap = dir.cross(c - a).abs();
            if gap <= EPS || gap > max_gap + EPS {
                continue;
            }
            let (uc, ud) = ((c - a).dot(dir), (d - a).dot(dir));
            let (u0, u1) = (uc.min(ud).max(0.0), uc.max(ud).min(len));
            if u1 - u0 <= EPS {
                continue;
            }
            let mut n = dir.perp();
            if n.dot(c - a) < 0.0 {
                n = -n;
            }
            let half = gap * 0.5;
            let at = |t: f64| a + dir * t + n * half;
            let mid = at((u0 + u1) * 0.5);
            if x.inside(mid) != free {
                continue;
            }
            let clear = |t: f64| !index.any_closer(at(t), half - LEVEL_TOL, &[i, j]);
            for (t0, t1) in true_intervals(u0, u1, clear) {
                out.push(Ridge {
                    a: at(t0),
                    b: at(t1),
                    half,
                    open: [t0 - u0 <= 1e-6, u1 - t1 <= 1e-6],
                });
            }
        }
    }
    out
}

/// `[lo, hi]` içinde `f`'nin doğru olduğu kapalı aralıklar: örnekleme + ikiye bölme.
fn true_intervals(lo: f64, hi: f64, f: impl Fn(f64) -> bool) -> Vec<(f64, f64)> {
    const N: usize = 48;
    let edge = |mut t_false: f64, mut t_true: f64| {
        for _ in 0..48 {
            let m = (t_false + t_true) * 0.5;
            if f(m) { t_true = m } else { t_false = m }
        }
        t_true
    };
    let ts: Vec<f64> = (0..=N).map(|k| lo + (hi - lo) * k as f64 / N as f64).collect();
    let vals: Vec<bool> = ts.iter().map(|&t| f(t)).collect();
    let mut out = Vec::new();
    let mut start: Option<f64> = None;
    for k in 0..=N {
        match (vals[k], start) {
            (true, None) => start = Some(if k == 0 { lo } else { edge(ts[k - 1], ts[k]) }),
            (false, Some(s)) => {
                out.push((s, edge(ts[k], ts[k - 1])));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, hi));
    }
    out
}

/// `a → b` merkez doğrulu, `h` yarıçaplı kapsül (uzunluk sıfırsa daire).
pub(crate) fn capsule(a: V, b: V, h: f64) -> Region {
    use super::geom::Arc;
    use std::f64::consts::{FRAC_PI_2, PI};
    if a.dist(b) <= EPS {
        return Region::new([Prim::Arc(Arc::circle(a, h))]);
    }
    let dir = (b - a).unit();
    let n = dir.perp() * h;
    let ang = dir.angle();
    Region::new([
        Prim::Seg(a + n, b + n),
        Prim::Seg(a - n, b - n),
        Prim::Arc(Arc { c: b, r: h, a0: ang - FRAC_PI_2, sweep: PI }),
        Prim::Arc(Arc { c: a, r: h, a0: ang + FRAC_PI_2, sweep: PI }),
    ])
}
