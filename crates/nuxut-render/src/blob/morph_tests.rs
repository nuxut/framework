//! ADR-0008 geometrisinin özellik ve örnek testleri (CPU referansı).

use super::geom::{Region, V, v};
use super::morph::{Panel, Scene, closing, free_area, free_space, opening};

const R: f64 = 10.0;
const CR: f64 = 8.0;

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> [V; 4] {
    [v(x0, y0), v(x1, y0), v(x1, y1), v(x0, y1)]
}

fn scene(w: f64, h: f64, panels: Vec<[V; 4]>) -> Scene {
    Scene { inner: (0.0, 0.0, w, h), panels: panels.into_iter().map(Panel::sharp).collect(), join: R, corner: CR }
}

/// Serbest mi (sınırdan `tol` kadar içeride).
fn free(h: &Region, p: V) -> bool {
    h.sdf(p) < 0.0
}

#[test]
fn top_panel_joins_concave_and_corners_convex() {
    let s = scene(400.0, 300.0, vec![rect(100.0, -10.0, 200.0, 80.0)]);
    let h = free_area(&s);
    // Birleşim filetosu: merkez (90, 10), yarıçap R.
    assert!(!free(&h, v(99.0, 1.0)), "birleşim köşesi dolu (içbükey fileto)");
    let on_arc = v(90.0, 10.0) + v(1.0, -1.0).unit() * R;
    assert!(h.sdf(on_arc).abs() < 1e-6, "fileto yayı tam R: {}", h.sdf(on_arc));
    // Panelin boşluğa açılan köşesi: merkez (108, 72), yarıçap r.
    assert!(free(&h, v(100.5, 79.5)), "panel köşesi kesik (dışbükey)");
    let on_corner = v(108.0, 72.0) + v(-1.0, 1.0).unit() * CR;
    assert!(h.sdf(on_corner).abs() < 1e-6, "köşe yayı tam r: {}", h.sdf(on_corner));
    // Çerçeve iç köşesi = R (karar 2).
    let frame_arc = v(10.0, 10.0) + v(-1.0, -1.0).unit() * R;
    assert!(h.sdf(frame_arc).abs() < 1e-6);
    // Düz yerlerde kesin uzaklık.
    assert!((h.sdf(v(150.0, 90.0)) + 10.0).abs() < 1e-9);
    assert!((h.sdf(v(300.0, 150.0)) + 100.0).abs() < 1e-9);
    assert!(!free(&h, v(150.0, 40.0)));
}

#[test]
fn opening_and_closing_are_idempotent() {
    let s = scene(300.0, 200.0, vec![rect(40.0, -5.0, 120.0, 60.0), rect(200.0, 150.0, 260.0, 205.0)]);
    let f = free_space(&s);
    let o = opening(&f, R);
    let oo = opening(&o, R);
    let c = closing(&o, CR);
    let cc = closing(&c, CR);
    for p in grid(300.0, 200.0, 2.5) {
        assert!((o.sdf(p) - oo.sdf(p)).abs() < 1e-6, "opening {p:?}");
        assert!((c.sdf(p) - cc.sdf(p)).abs() < 1e-6, "closing {p:?}");
    }
}

fn grid(w: f64, h: f64, step: f64) -> impl Iterator<Item = V> {
    let (nx, ny) = ((w / step) as usize, (h / step) as usize);
    (0..=nx).flat_map(move |i| (0..=ny).map(move |j| v(i as f64 * step + 0.123, j as f64 * step + 0.377)))
}

/// Küçük, belirlenimci rastgele üreteç.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        // 1/4 px ızgarası: kenar paylaşımları ve tam teğetler de denenir.
        ((lo + (hi - lo) * self.next()) * 4.0).round() / 4.0
    }
}

/// Kenara yapışık rastgele paneller (kare ekran; D4 bunu kendine eşler).
fn random_scene(rng: &mut Lcg, size: f64) -> Scene {
    let n = 1 + (rng.next() * 3.0) as usize;
    let mut panels = Vec::new();
    for _ in 0..n {
        let (a, b) = (rng.range(10.0, size - 60.0), rng.range(20.0, 120.0));
        let depth = rng.range(0.0, 120.0);
        let p = match (rng.next() * 4.0) as u32 {
            0 => rect(a, -5.0, a + b, depth),
            1 => rect(size - depth, a, size + 5.0, a + b),
            2 => rect(a, size - depth, a + b, size + 5.0),
            _ => rect(-5.0, a, depth, a + b),
        };
        panels.push(p);
    }
    scene(size, size, panels)
}

/// Kare `[0, s]²`'nin 8 simetrisi.
fn d4(k: usize, s: f64) -> impl Fn(V) -> V {
    move |p: V| {
        let q = if k & 4 != 0 { v(p.y, p.x) } else { p };
        let q = if k & 1 != 0 { v(s - q.x, q.y) } else { q };
        if k & 2 != 0 { v(q.x, s - q.y) } else { q }
    }
}

#[test]
fn d4_symmetry() {
    let size = 240.0;
    let mut rng = Lcg(7);
    for _ in 0..6 {
        let s = random_scene(&mut rng, size);
        let h = free_area(&s);
        for k in 1..8 {
            let g = d4(k, size);
            let gs = Scene { panels: s.panels.iter().map(|q| Panel::sharp(q.pts.map(&g))).collect(), ..s.clone() };
            let gh = free_area(&gs);
            for p in grid(size, size, 6.0) {
                let (a, b) = (h.sdf(p), gh.sdf(g(p)));
                assert!((a - b).abs() < 1e-6, "simetri {k}: {p:?} {a} != {b}\n{:?}", s.panels);
            }
        }
    }
}

#[test]
fn morphology_is_monotone() {
    // Kesin açma/kapama tekdüzedir: F' ⊆ F ⇒ op(F') ⊆ op(F). Düzenleme (dar cep) bilerek
    // tekdüze değildir: yeni bir panelin açtığı dar cebin ucu d/2 yarıçaplıdır ve R filetosunun
    // doldurduğu köşeye ondan daha çok girer (ADR-0008, karar 1).
    let size = 240.0;
    let mut rng = Lcg(11);
    for _ in 0..8 {
        let s = random_scene(&mut rng, size);
        let mut more = s.clone();
        more.panels.extend(random_scene(&mut rng, size).panels);
        let (f, fm) = (free_space(&s), free_space(&more));
        let (o, om) = (opening(&f, R), opening(&fm, R));
        let (c, cm) = (closing(&o, CR), closing(&om, CR));
        for p in grid(size, size, 5.0) {
            assert!(!(om.sdf(p) < -1e-6 && o.sdf(p) > 1e-6), "açma {p:?}\n{:?}\n{:?}", s.panels, more.panels);
            assert!(!(cm.sdf(p) < -1e-6 && c.sdf(p) > 1e-6), "kapama {p:?}\n{:?}\n{:?}", s.panels, more.panels);
        }
    }
}

#[test]
fn local_changes_stay_local() {
    let base = scene(400.0, 300.0, vec![rect(40.0, -5.0, 120.0, 80.0), rect(250.0, -5.0, 330.0, 60.0)]);
    let mut moved = base.clone();
    moved.panels[1] = Panel::sharp(rect(250.0, -5.0, 330.0, 90.0));
    let (h, hm) = (free_area(&base), free_area(&moved));
    for p in grid(400.0, 300.0, 4.0) {
        // Değişen panelin R + r komşuluğu dışında küme aynı (işaretli uzaklık değeri küresel).
        let near = p.x > 250.0 - R - CR - 1.0 && p.x < 330.0 + R + CR + 1.0 && p.y < 90.0 + R + CR + 1.0;
        if !near {
            assert_eq!(h.inside(p), hm.inside(p), "{p:?}");
        }
    }
}

/// Piksel kapsaması (1 px örnekleme, çizimdeki AA ile aynı). Görünür sıçrama budur: kıl kadar
/// ince bir geçidin kaybolması kapsamada süreklidir, işaretli uzaklıkta değil.
fn coverage(r: &Region, p: V) -> f64 {
    (0.5 - r.sdf(p)).clamp(0.0, 1.0)
}

/// Sıralı karelerin en büyük piksel kapsama değişimi ve yeri.
fn worst_step(fields: &[Region], pts: &[V]) -> (f64, usize, V) {
    let mut worst = (0.0, 0, v(0.0, 0.0));
    for (k, pair) in fields.windows(2).enumerate() {
        for &p in pts {
            let d = (coverage(&pair[0], p) - coverage(&pair[1], p)).abs();
            if d > worst.0 {
                worst = (d, k, p);
            }
        }
    }
    worst
}

/// Hiçbir pikselin kapsaması bir adımda tümüyle dönmüyor (gerçek sıçrama adım ne kadar küçük
/// olursa olsun 1'dir). Sınırın düzgün hareketi 2 kat Lipschitz'tir; köprünün `r` topunun
/// sığdığı anda karekök biçimli dik ama sürekli bir geçişi var ([`steep_event_refines_to_zero`]).
fn assert_continuous(frames: &[Scene], w: f64, h: f64) {
    let fields: Vec<Region> = frames.iter().map(free_area).collect();
    let pts: Vec<V> = grid(w, h, 1.0).collect();
    let (worst, k, at) = worst_step(&fields, &pts);
    assert!(worst < 0.9, "kare {k}→{}: kapsama sıçraması {worst} @ {at:?}\n{:?}", k + 1, frames[k].panels);
}

#[test]
fn gap_sweep_is_continuous() {
    // Alt duvardan d uzaklıkta panel; d = 3R → 0, 0.25 px adım.
    let (w, h) = (300.0, 200.0);
    let frames: Vec<Scene> =
        (0..=120).map(|k| scene(w, h, vec![rect(80.0, -5.0, 220.0, h - (30.0 - k as f64 * 0.25))])).collect();
    assert_continuous(&frames, w, h);
}

#[test]
fn seed_growth_is_continuous() {
    let (w, h) = (300.0, 200.0);
    let frames: Vec<Scene> = (0..=160)
        .map(|k| {
            let t = k as f64 * 0.25;
            scene(w, h, vec![rect(150.0 - t * 2.0, -5.0, 150.0 + t * 2.0, t)])
        })
        .collect();
    assert_continuous(&frames, w, h);
}

fn neighbours(gap: f64) -> Scene {
    // Üst bara yapışık iki panel; aralarındaki boğazın üstü kapalı, altı açık.
    scene(300.0, 200.0, vec![rect(40.0, -5.0, 140.0, 120.0), rect(140.0 + gap, -5.0, 240.0, 90.0)])
}

#[test]
fn neighbours_side_by_side_are_continuous() {
    let frames: Vec<Scene> = (0..=100).map(|k| neighbours(25.0 - k as f64 * 0.25)).collect();
    assert_continuous(&frames, 300.0, 200.0);
}

#[test]
fn timing_is_small() {
    let s = scene(1920.0, 1080.0, vec![
        rect(700.0, -5.0, 1200.0, 400.0),
        rect(1500.0, -5.0, 1925.0, 1085.0),
        rect(-5.0, 900.0, 300.0, 1085.0),
    ]);
    let t = std::time::Instant::now();
    let n = 20;
    for _ in 0..n {
        std::hint::black_box(free_area(std::hint::black_box(&s)));
    }
    let per = t.elapsed() / n;
    eprintln!("free_area: {per:?} / kare, {} ilkel", free_area(&s).prims.len());
}

/// `NUXUT_BLOB_DUMP=<dizin>` verilmişse bölgeyi PNG olarak yazar (hata ayıklama).
fn dump(name: &str, h: &Region, w: f64, hgt: f64, zoom: f64) {
    let Some(dir) = std::env::var_os("NUXUT_BLOB_DUMP") else { return };
    let (iw, ih) = ((w * zoom) as u32, (hgt * zoom) as u32);
    let img = image::RgbImage::from_fn(iw, ih, |x, y| {
        let p = v((x as f64 + 0.5) / zoom, (y as f64 + 0.5) / zoom);
        let d = h.sdf(p);
        if d.abs() < 0.6 / zoom {
            image::Rgb([230, 40, 40])
        } else if d < 0.0 {
            image::Rgb([235, 235, 228])
        } else {
            let k = (40.0 + (d * 4.0).min(60.0)) as u8;
            image::Rgb([k, k, k + 10])
        }
    });
    img.save(std::path::Path::new(&dir).join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "yalnız NUXUT_BLOB_DUMP ile görsel inceleme"]
fn dump_cases() {
    let (w, h) = (300.0, 200.0);
    for t in [7.5, 7.75, 3.0, 12.0] {
        let s = scene(w, h, vec![rect(150.0 - t * 2.0, -5.0, 150.0 + t * 2.0, t)]);
        dump(&format!("seed-{t}"), &free_area(&s), w, 60.0, 4.0);
    }
    for d in [14.5, 14.25, 20.0, 10.0, 5.0, 1.0] {
        let s = scene(w, h, vec![rect(80.0, -5.0, 220.0, h - d)]);
        dump(&format!("gap-{d}"), &free_area(&s), w, h, 3.0);
    }
    for g in [0.5, 0.25, 5.0, 12.0] {
        let s = scene(w, h, vec![rect(40.0, -5.0, 140.0, 120.0), rect(140.0 + g, -5.0, 240.0, 90.0)]);
        dump(&format!("pair-{g}"), &free_area(&s), w, h, 3.0);
    }
    let s = scene(300.0, 200.0, vec![rect(40.0, -5.0, 120.0, 60.0), rect(200.0, 150.0, 260.0, 205.0)]);
    let o = opening(&free_space(&s), R);
    dump("open", &o, 300.0, 200.0, 3.0);
    dump("open2", &opening(&o, R), 300.0, 200.0, 3.0);
}


#[test]
fn own_rounding_is_kept() {
    // Duvarlardan uzakta yüzen hap (OSD): kendi yarıçapı (h/2) korunur, r'den büyük.
    let mut s = scene(400.0, 300.0, vec![]);
    s.panels.push(Panel { pts: rect(100.0, 200.0, 300.0, 240.0), radii: [20.0; 4] });
    let h = free_area(&s);
    let on_cap = v(120.0, 220.0) + v(-1.0, 0.0) * 20.0;
    assert!(h.sdf(on_cap).abs() < 1e-6, "hap ucu yarıçap 20: {}", h.sdf(on_cap));
    assert!(free(&h, v(101.0, 201.0)), "hap köşesi kesik");
    assert!(!free(&h, v(200.0, 220.0)));
}

#[test]
#[ignore = "profil"]
fn profile_stages() {
    use super::morph::{boolean, capsule, offset, pocket_half, ridges};
    let s = scene(1920.0, 1080.0, vec![
        rect(700.0, -5.0, 1200.0, 400.0),
        rect(1500.0, -5.0, 1925.0, 1085.0),
        rect(-5.0, 900.0, 300.0, 1085.0),
    ]);
    let n = 50;
    let time = |name: &str, f: &dyn Fn()| {
        let t = std::time::Instant::now();
        for _ in 0..n {
            f();
        }
        eprintln!("{name}: {:?}", t.elapsed() / n);
    };
    let f = free_space(&s);
    let e = offset(&f, R, false);
    let o = offset(&e, R, true);
    let rs = ridges(&f, true, 2.0 * R);
    let h1 = rs.iter().fold(o.clone(), |acc, r| boolean(&acc, &capsule(r.a, r.b, pocket_half(r.half, R)), |x, y| x || y));
    let d = offset(&h1, CR, true);
    time("free_space", &|| { std::hint::black_box(free_space(&s)); });
    time("erode R", &|| { std::hint::black_box(offset(&f, R, false)); });
    time("dilate R", &|| { std::hint::black_box(offset(&e, R, true)); });
    time("ridges free", &|| { std::hint::black_box(ridges(&f, true, 2.0 * R)); });
    time("dilate r", &|| { std::hint::black_box(offset(&h1, CR, true)); });
    time("erode r", &|| { std::hint::black_box(offset(&d, CR, false)); });
    time("ridges filled", &|| { std::hint::black_box(ridges(&h1, false, 2.0 * CR)); });
    eprintln!("prims: F {} E {} O {} H1 {} D {}", f.prims.len(), e.prims.len(), o.prims.len(), h1.prims.len(), d.prims.len());
}

#[test]
#[ignore = "profil"]
fn profile_many_shapes() {
    // 16 şekil (sınır): bar'dan inen paneller, kenar çekmeceleri, yüzen haplar.
    let mut panels = Vec::new();
    for i in 0..8 {
        let x = 60.0 + i as f64 * 230.0;
        panels.push(rect(x, -5.0, x + 160.0, 120.0 + (i % 3) as f64 * 90.0));
    }
    for i in 0..4 {
        let y = 300.0 + i as f64 * 180.0;
        panels.push(rect(1700.0, y, 1925.0, y + 140.0));
    }
    for i in 0..4 {
        let x = 300.0 + i as f64 * 300.0;
        panels.push(rect(x, 900.0, x + 200.0, 940.0));
    }
    let s = scene(1920.0, 1080.0, panels);
    let n = 10;
    let t = std::time::Instant::now();
    for _ in 0..n {
        std::hint::black_box(free_area(std::hint::black_box(&s)));
    }
    eprintln!("16 şekil: {:?} / kare, {} ilkel", t.elapsed() / n, free_area(&s).prims.len());
}

#[test]
#[ignore = "profil"]
fn profile_many_stages() {
    use super::morph::{boolean, capsule, offset, pocket_half, ridges};
    let mut panels = Vec::new();
    for i in 0..8 {
        let x = 60.0 + i as f64 * 230.0;
        panels.push(rect(x, -5.0, x + 160.0, 120.0 + (i % 3) as f64 * 90.0));
    }
    for i in 0..4 {
        let y = 300.0 + i as f64 * 180.0;
        panels.push(rect(1700.0, y, 1925.0, y + 140.0));
    }
    for i in 0..4 {
        let x = 300.0 + i as f64 * 300.0;
        panels.push(rect(x, 900.0, x + 200.0, 940.0));
    }
    let s = scene(1920.0, 1080.0, panels);
    let t = std::time::Instant::now();
    let f = free_space(&s);
    eprintln!("free_space {:?} ({})", t.elapsed(), f.prims.len());
    let t = std::time::Instant::now();
    let e = offset(&f, R, false);
    eprintln!("erode R {:?} ({})", t.elapsed(), e.prims.len());
    let t = std::time::Instant::now();
    let o = offset(&e, R, true);
    eprintln!("dilate R {:?} ({})", t.elapsed(), o.prims.len());
    let t = std::time::Instant::now();
    let rs = ridges(&f, true, 2.0 * R);
    eprintln!("ridges {:?} ({})", t.elapsed(), rs.len());
    let t = std::time::Instant::now();
    let h1 = rs.iter().fold(o.clone(), |acc, r| boolean(&acc, &capsule(r.a, r.b, pocket_half(r.half, R)), |x, y| x || y));
    eprintln!("bool {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let d = offset(&h1, CR, true);
    eprintln!("dilate r {:?} ({})", t.elapsed(), d.prims.len());
    let t = std::time::Instant::now();
    let c = offset(&d, CR, false);
    eprintln!("erode r {:?} ({})", t.elapsed(), c.prims.len());
    let t = std::time::Instant::now();
    let fr = ridges(&h1, false, 2.0 * CR);
    eprintln!("filled ridges {:?} ({})", t.elapsed(), fr.len());
}

#[test]
#[ignore = "hata ayıklama"]
fn debug_seed_scene() {
    let f = super::gpu_tests::field(vec![super::BlobShape::new_rect(nuxut_core::Rect::new(220.0, 30.0, 40.0, 9.0))]);
    let p = f.prepare(1.5, (720, 450)).unwrap();
    let h = free_area(&p.scene);
    for (q, (s, e)) in h.prims.iter().zip(&h.ends) {
        eprintln!("{q:?}\n    uç {s:?} -> {e:?}");
    }
    dump("seed15", &h, 720.0, 450.0, 1.0);
}

/// Sınırın her köşesinde komşu ilkeller teğet mi (kırık köşe = sivri uç ya da kıvrım).
fn assert_smooth(h: &Region, what: &str) {
    use super::geom::Prim;
    let tangent = |p: &Prim, t: f64| match p {
        Prim::Seg(a, b) => (*b - *a).unit(),
        Prim::Arc(_) => p.normal(t).perp(),
    };
    let mut ends: Vec<(V, V)> = Vec::new();
    for p in &h.prims {
        ends.push((p.start(), tangent(p, 0.0)));
        ends.push((p.end(), tangent(p, 1.0)));
    }
    for (i, (pa, ta)) in ends.iter().enumerate() {
        for (pb, tb) in &ends[i + 1..] {
            if pa.dist(*pb) < 1e-6 {
                assert!(ta.cross(*tb).abs() < 1e-3, "{what}: kırık köşe {pa:?} ({ta:?} / {tb:?})");
            }
        }
    }
}

#[test]
fn gap_sweep_has_no_kinks() {
    // Sivri uç ya da tümsek bırakan köprü kırık köşe üretir; boğazın her aşaması düzgün olmalı.
    for k in 0..=24 {
        let d = 30.0 - k as f64 * 1.25;
        let s = scene(300.0, 200.0, vec![rect(80.0, -5.0, 220.0, 200.0 - d)]);
        assert_smooth(&free_area(&s), &format!("d = {d}"));
    }
    for g in [24.0, 15.0, 10.0, 5.0, 2.0, 0.5] {
        assert_smooth(&free_area(&neighbours(g)), &format!("yan yana {g}"));
    }
}

#[test]
fn join_radius_sweep_is_continuous() {
    // Tam ekrana geçiş: çağıran R'yi yayla 0'a sürer.
    let frames: Vec<Scene> = (0..=80)
        .map(|k| {
            let mut s = scene(300.0, 200.0, vec![rect(80.0, -5.0, 220.0, 120.0), rect(-5.0, 150.0, 60.0, 205.0)]);
            s.join = R * (1.0 - k as f64 / 80.0);
            s
        })
        .collect();
    assert_continuous(&frames, 300.0, 200.0);
}

#[test]
#[ignore = "hata ayıklama"]
fn debug_mouth() {
    let Some(dir) = std::env::var_os("NUXUT_BLOB_DUMP") else { return };
    for d in [6.25, 10.0, 15.0] {
        let s = scene(300.0, 200.0, vec![rect(80.0, -5.0, 220.0, 200.0 - d)]);
        let h = free_area(&s);
        let zoom = 12.0;
        let img = image::RgbImage::from_fn((60.0 * zoom) as u32, (40.0 * zoom) as u32, |x, y| {
            let p = v(195.0 + (x as f64 + 0.5) / zoom, 165.0 + (y as f64 + 0.5) / zoom);
            let dd = h.sdf(p);
            if dd.abs() < 0.6 / zoom { image::Rgb([230, 40, 40]) } else if dd < 0.0 { image::Rgb([235, 235, 228]) } else { image::Rgb([60, 60, 70]) }
        });
        img.save(std::path::Path::new(&dir).join(format!("mouth-{d}.png"))).unwrap();
    }
}

#[test]
fn near_neighbours_have_no_slit() {
    // Aralık ≤ R: yan yana paneller tek parça, aradaki yarıkta hiç serbest nokta yok.
    for g in [0.25, 2.0, 5.0, 9.0, R] {
        let h = free_area(&neighbours(g));
        for i in 0..=40 {
            for j in 0..=80 {
                let p = v(140.0 + g * i as f64 / 40.0, j as f64);
                assert!(h.sdf(p) > -1e-6, "aralık {g}: yarıkta serbest nokta {p:?}");
            }
        }
    }
}

#[test]
fn mouth_silhouette_is_monotone() {
    // Panelin ağzında duvar tarafındaki dolu yüzeyin yüksekliği panelden uzaklaştıkça yalnızca
    // azalır: tümsek ("kulak") fazladan bir iniş-çıkış demektir.
    let wall = 200.0;
    for k in 0..=24 {
        let d = 30.0 - k as f64 * 1.25;
        let s = scene(300.0, wall, vec![rect(80.0, -5.0, 220.0, wall - d)]);
        let h = free_area(&s);
        // x'te duvardan yukarı kesintisiz dolu kalınlık.
        let filled_height = |x: f64| {
            let mut y = wall - 0.01;
            while y > wall - d - 2.0 * R && h.sdf(v(x, y)) > 0.0 {
                y -= 0.05;
            }
            wall - y
        };
        let xs: Vec<f64> = (0..=400).map(|i| 220.0 + i as f64 * 0.1).collect();
        let hs: Vec<f64> = xs.iter().map(|&x| filled_height(x)).collect();
        for w in hs.windows(2).zip(&xs) {
            assert!(w.0[1] <= w.0[0] + 0.06, "d = {d}: x = {} civarında tümsek ({} → {})", w.1, w.0[0], w.0[1]);
        }
    }
}
