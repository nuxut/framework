//! ADR-0008 altın görüntü testi: her sahne (`support/scenes.rs`) 1.0 ve 1.5 ölçekte başsız
//! çizilir ve `tests/golden/<ad>@<ölçek>.png` ile karşılaştırılır (kanal başına ±2/255).
//!
//! - `NUXUT_BLESS=1 cargo test -p nuxut-render --test it blob_golden`: altın görüntüleri yazar.
//! - Fark varsa `target/golden-diff/` altına beklenen/gerçek/fark PNG'leri yazılır.
//! - Altın görüntüsü olmayan sahne atlanır (not düşülür); `NUXUT_GOLDEN_REQUIRE=1` ile hata.
//!
//! EGL/GLES yoksa test atlanır.

use nuxut_core::Size;
use crate::support::golden::{self, Harness, Outcome, Rgba};
use crate::support::scenes::{self, SceneParams};

/// Altın görüntü viewport'u (mantıksal).
const VIEWPORT: (f32, f32) = (480.0, 300.0);
const SCALES: [f32; 2] = [1.0, 1.5];

fn params() -> SceneParams {
    SceneParams::from_theme(&nuxut_theme::Theme::default(), Size::new(VIEWPORT.0, VIEWPORT.1))
}

fn render(h: &mut Harness, field: &nuxut_render::BlobField, scale: f32) -> Rgba {
    h.render(VIEWPORT, scale, |p| p.blob(field))
}

#[test]
fn blob_scenes_match_golden() {
    let Some(mut h) = Harness::new() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let bless = golden::bless_requested();
    let require = std::env::var_os("NUXUT_GOLDEN_REQUIRE").is_some_and(|v| v != "0");
    let (dir, diff_dir) = (golden::golden_dir(), golden::diff_dir());
    let mut failures = Vec::new();
    let mut missing = Vec::new();
    for (name, field) in scenes::scenes(&params(), 1.0) {
        for scale in SCALES {
            let id = format!("{name}@{scale}");
            match golden::check(&dir, &diff_dir, &id, &render(&mut h, &field, scale), bless) {
                Outcome::Match | Outcome::Blessed => {}
                Outcome::Missing => missing.push(id),
                other => failures.push(format!("{id}: {other:?}")),
            }
        }
    }
    if bless {
        eprintln!("altın görüntüler yazıldı: {}", dir.display());
    }
    if !missing.is_empty() {
        eprintln!("altın görüntüsü olmayan {} sahne atlandı (NUXUT_BLESS=1 ile üretilir)", missing.len());
        assert!(!require, "eksik altın görüntüler: {missing:?}");
    }
    assert!(
        failures.is_empty(),
        "altın görüntüden farklı (fark PNG'leri: {}):\n{}",
        diff_dir.display(),
        failures.join("\n")
    );
}

/// Düzeneğin kendisi: geçici dizinde bless → aynı çizim eşleşir → bozulmuş görüntü toleransı
/// aşar, sayılır ve fark PNG'leri yazılır; ±2 içindeki gürültü eşleşir.
#[test]
fn harness_blesses_compares_and_writes_diffs() {
    let Some(mut h) = Harness::new() else {
        eprintln!("EGL surfaceless yok; test atlandı");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("nuxut-golden-selftest-{}", std::process::id()));
    let (dir, diff_dir) = (tmp.join("golden"), tmp.join("diff"));
    let field = scenes::scene(&params(), "top", 1.0).expect("sahne");
    for scale in SCALES {
        let id = format!("top@{scale}");
        let image = render(&mut h, &field, scale);
        assert_eq!(image.width, (VIEWPORT.0 * scale).round() as u32);
        assert!(image.pixels.chunks_exact(4).any(|p| p[3] == 255), "{id}: blob çizilmeli");
        assert_eq!(golden::check(&dir, &diff_dir, &id, &image, false), Outcome::Missing);
        assert_eq!(golden::check(&dir, &diff_dir, &id, &image, true), Outcome::Blessed);
        // Aynı çizim tekrar: eşleşir (deterministik).
        let again = render(&mut h, &field, scale);
        assert_eq!(golden::check(&dir, &diff_dir, &id, &again, false), Outcome::Match);
        // ±2 gürültü tolerans içinde.
        let mut noisy = again.clone();
        for (i, b) in noisy.pixels.iter_mut().enumerate() {
            *b = if i % 2 == 0 { b.saturating_add(2) } else { b.saturating_sub(2) };
        }
        assert_eq!(golden::check(&dir, &diff_dir, &id, &noisy, false), Outcome::Match);
        // 3 piksel 40 birim bozulur: sayılır, en büyük fark raporlanır, PNG'ler yazılır.
        let mut broken = again.clone();
        for px in [0usize, 10, 20] {
            broken.pixels[px * 4] = broken.pixels[px * 4].wrapping_add(40);
        }
        assert_eq!(golden::check(&dir, &diff_dir, &id, &broken, false), Outcome::Differs { pixels: 3, max: 40 });
        for kind in ["expected", "actual", "diff"] {
            assert!(diff_dir.join(format!("{id}.{kind}.png")).is_file(), "{id}.{kind}.png");
        }
        // Yazılan PNG kayıpsız geri okunur.
        assert!(golden::load(&dir.join(format!("{id}.png"))) == Some(image));
    }
    // Sahne listesi: 7 grup, 15 sahne; adlar benzersiz.
    let names = scenes::names(&params());
    assert_eq!(names.len(), 15, "{names:?}");
    let mut unique = names.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), names.len());
    let _ = std::fs::remove_dir_all(&tmp);
}
