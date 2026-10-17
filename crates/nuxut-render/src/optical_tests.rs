//! Optik ortalama (ADR-0009): glifler rasterleştirilip mürekkebin fiziksel piksel sınırı ölçülür,
//! hücre (layout kutusu) merkeziyle karşılaştırılır. Painter'la aynı yerleştirme yolu kullanılır.

#[cfg(test)]
use super::text::*;
use nuxut_core::Point;

/// Mürekkep sınırı (x0, y0, x1, y1), fiziksel px. Kenarlar alt piksel tahminlidir: kenardaki
/// satır/sütunun en yüksek örtüsü kadar içeri (düz kenarda gerçek kenara yakın).
/// `legacy`: eski yerleştirme (cosmic-text `physical`, taban çizgisi kesilir).
pub(crate) fn ink_bounds(ts: &mut TextSystem, layout: &TextLayout, origin: Point, scale: f32, legacy: bool) -> Option<[f32; 4]> {
    // Tüm glifleri tek bir kapsama tuvaline topla (örtüşen glifler en yüksek değeri verir).
    let mut px: std::collections::HashMap<(i32, i32), u8> = std::collections::HashMap::new();
    for placed in &layout.glyphs {
        let (key, x, y) = if legacy {
            let p = placed.glyph.physical((origin.x * scale, (origin.y + placed.line_y) * scale), scale);
            (p.cache_key, p.x, p.y)
        } else {
            place_glyph(placed, origin, scale)
        };
        let Some(img) = ts.rasterize(key) else { continue };
        let (w, h) = (img.placement.width as usize, img.placement.height as usize);
        let bpp = if matches!(img.content, cosmic_text::SwashContent::Mask) { 1 } else { 4 };
        let (gx, gy) = (x + img.placement.left, y - img.placement.top);
        for yy in 0..h {
            for xx in 0..w {
                let a = img.data[(yy * w + xx) * bpp + bpp - 1];
                if a > 0 {
                    let e = px.entry((gx + xx as i32, gy + yy as i32)).or_default();
                    *e = (*e).max(a);
                }
            }
        }
    }
    let x0 = px.keys().map(|k| k.0).min()?;
    let x1 = px.keys().map(|k| k.0).max()?;
    let y0 = px.keys().map(|k| k.1).min()?;
    let y1 = px.keys().map(|k| k.1).max()?;
    let col = |x: i32| px.iter().filter(|(k, _)| k.0 == x).map(|(_, a)| *a).max().unwrap_or(0) as f32 / 255.0;
    let row = |y: i32| px.iter().filter(|(k, _)| k.1 == y).map(|(_, a)| *a).max().unwrap_or(0) as f32 / 255.0;
    Some([
        x0 as f32 + 1.0 - col(x0),
        y0 as f32 + 1.0 - row(y0),
        x1 as f32 + col(x1),
        y1 as f32 + row(y1),
    ])
}

/// Hücre merkezinden mürekkep merkezinin farkı (dx, dy), fiziksel px. Layout kutusu kesirli bir
/// hücrede ortalanır (çağıranların yaptığı gibi: `center - size / 2`).
fn offset(ts: &mut TextSystem, layout: &TextLayout, cell_center: Point, scale: f32, legacy: bool) -> Option<(f32, f32)> {
    let s = layout.size();
    let origin = Point::new(cell_center.x - s.width * 0.5, cell_center.y - s.height * 0.5);
    let b = ink_bounds(ts, layout, origin, scale, legacy)?;
    Some(((b[0] + b[2]) * 0.5 - cell_center.x * scale, (b[1] + b[3]) * 0.5 - cell_center.y * scale))
}

const SCALES: [f32; 6] = [1.0, 1.25, 1.5, 1.75, 2.0, 2.5];

/// Dikey tolerans (fiziksel px). 1.0 ölçekte taban çizgisi tam piksele yuvarlanır ve anahat
/// ipuçlanır (netlik; `place_glyph`): yuvarlama tek başına ≤0.5 px, ipuçlamanın büyük harf
/// yüksekliği yuvarlaması üstüne ≤0.25 px ekler. Diğer ölçeklerde ipucusuz + dikey alt piksel.
fn tolerance_y(scale: f32) -> f32 {
    if (scale - 1.0).abs() < 1e-3 { 0.75 } else { 0.5 }
}

/// Kesirli ölçekte farklı piksel evrelerini sınamak için birkaç hücre merkezi.
const CENTERS: [(f32, f32); 3] = [(20.0, 14.0), (37.3, 21.7), (51.55, 30.45)];

/// İlk kurulu aile (yoksa `None`; benzeri aranır, test atlanmaz — en az biri kurulu olmalı).
fn first_installed(ts: &mut TextSystem, names: &[&str]) -> Option<String> {
    names.iter().find(|n| ts.has_family(n)).map(|n| (*n).to_owned())
}

/// Metin: rakam ve büyük harf bandı (saat) hücre ortasında, her font × ölçekte ≤ 0.5 fiziksel px.
/// Rakamlar büyük harf yüksekliğindedir (lining figures); ölçülen mürekkep = büyük harf bandı.
#[test]
fn clock_digits_are_optically_centered() {
    let mut ts = TextSystem::new();
    let sans = ["Inter", "Adwaita Sans", "Noto Sans", "Google Sans Flex", "Rubik", "DejaVu Sans"];
    let mono = ["JetBrainsMono Nerd Font", "JetBrains Mono", "Noto Sans Mono", "DejaVu Sans Mono"];
    let families: Vec<String> = sans.iter().chain(&mono).filter(|n| ts.has_family(n)).map(|n| (*n).to_owned()).collect();
    assert!(first_installed(&mut ts, &sans).is_some() && first_installed(&mut ts, &mono).is_some(), "sans ve mono font gerekli");
    let mut worst = 0.0f32;
    for family in &families {
        for (size, lh) in [(13.0, 1.3), (16.0, 1.3), (13.0, 1.05)] {
            let mut style = TextStyle::new(family.as_str(), size, 500).with_tabular_numbers();
            style.line_height = lh;
            let l = ts.layout("14:05", &style, None);
            for s in SCALES {
                for c in CENTERS {
                    let (_, dy) = offset(&mut ts, &l, Point::new(c.0, c.1), s, false).expect("mürekkep");
                    worst = worst.max(dy.abs());
                    assert!(dy.abs() <= tolerance_y(s), "{family} {size}px lh {lh} ölçek {s} merkez {c:?}: dy {dy:+.2} px");
                }
            }
        }
    }
    eprintln!("en kötü dikey sapma: {worst:.2} px");
}

/// İkon: tek glif (Nerd Font PUA, Material ligatürü) mürekkep kutusu hücre ortasında, iki eksende.
#[test]
fn icon_glyphs_are_ink_centered() {
    let mut ts = TextSystem::new();
    let mut cases: Vec<(String, &str)> = Vec::new();
    // Nerd glifleri: yalnız sembol fontu ve hücre genişliğini aşan eş aralıklı olmayan Nerd Font.
    for family in ["Symbols Nerd Font", "JetBrainsMono Nerd Font", "JetBrainsMono Nerd Font Mono", "CaskaydiaCove Nerd Font"] {
        if ts.has_family(family) {
            for g in ["\u{f0239}", "\u{f018d}", "\u{f372}", "\u{e6ae}", "\u{f024b}"] {
                cases.push((family.to_owned(), g));
            }
        }
    }
    for family in ["Material Symbols Rounded", "Material Symbols Outlined"] {
        if ts.has_family(family) {
            for g in ["home", "wifi", "volume_up", "battery_full"] {
                cases.push((family.to_owned(), g));
            }
        }
    }
    assert!(!cases.is_empty(), "Nerd Font ya da Material Symbols kurulu olmalı");
    let mut worst = 0.0f32;
    for (family, glyph) in &cases {
        for (size, lh) in [(20.0, 1.0), (14.0, 1.3), (24.0, 1.0)] {
            let mut style = TextStyle::new(family.as_str(), size, 400);
            style.line_height = lh;
            let l = ts.layout(glyph, &style, None);
            assert_eq!(l.glyphs.len(), 1, "{family} {glyph:?}: tek glif");
            for s in SCALES {
                for c in CENTERS {
                    let (dx, dy) = offset(&mut ts, &l, Point::new(c.0, c.1), s, false).expect("mürekkep");
                    worst = worst.max(dx.abs()).max(dy.abs());
                    assert!(
                        dx.abs() <= 0.5 && dy.abs() <= tolerance_y(s),
                        "{family} {glyph:?} {size}px ölçek {s} merkez {c:?}: dx {dx:+.2} dy {dy:+.2} px"
                    );
                }
            }
        }
    }
    eprintln!("en kötü ikon sapması: {worst:.2} px");
}

/// Çok satırlı metinde satır aralığı değişmez: kayma tüm bloğa aynıdır.
#[test]
fn cap_height_shift_keeps_line_spacing() {
    let mut ts = TextSystem::new();
    let style = TextStyle::new("sans-serif", 13.0, 400);
    let metrics = ts.layout("Bir\nİki\nÜç", &style.clone().with_optical(OpticalCenter::Metrics), None);
    let cap = ts.layout("Bir\nİki\nÜç", &style.with_optical(OpticalCenter::CapHeight), None);
    if metrics.is_empty() {
        return;
    }
    assert_eq!(metrics.size(), cap.size(), "kutu değişmez");
    let lines = |l: &TextLayout| {
        let mut ys: Vec<f32> = l.glyphs.iter().map(|g| g.line_y).collect();
        ys.dedup();
        ys
    };
    let (a, b) = (lines(&metrics), lines(&cap));
    assert_eq!(a.len(), 3);
    let d = b[0] - a[0];
    for (x, y) in a.iter().zip(&b) {
        assert!((y - x - d).abs() < 1e-4, "satırlar aynı miktarda kaymalı");
    }
    assert!((cap.baseline() - metrics.baseline() - d).abs() < 1e-4);
    // Kayma küçüktür (font metriği düzeltmesi), satır yüksekliğinin çok altında.
    assert!(d.abs() < 13.0 * 0.15, "{d}");
}

/// `Auto`: PUA ve ligatür ikondur, sıradan tek harf/rakam metindir.
#[test]
fn auto_detects_icon_text() {
    let mut ts = TextSystem::new();
    if !ts.has_family("Symbols Nerd Font") && !ts.has_family("JetBrainsMono Nerd Font") {
        return;
    }
    let fam = first_installed(&mut ts, &["Symbols Nerd Font", "JetBrainsMono Nerd Font"]).unwrap_or_default();
    let style = TextStyle::new(fam.as_str(), 20.0, 400);
    let auto = ts.layout("\u{f0239}", &style, None);
    let ink = ts.layout("\u{f0239}", &style.clone().with_optical(OpticalCenter::Ink), None);
    assert_eq!(auto.glyphs[0].glyph.x, ink.glyphs[0].glyph.x);
    assert_eq!(auto.glyphs[0].line_y, ink.glyphs[0].line_y);
    let digit = ts.layout("5", &TextStyle::new("sans-serif", 13.0, 400), None);
    let cap = ts.layout("5", &TextStyle::new("sans-serif", 13.0, 400).with_optical(OpticalCenter::CapHeight), None);
    assert_eq!(digit.glyphs[0].line_y, cap.glyphs[0].line_y);
    assert_eq!(digit.glyphs[0].glyph.x, 0.0, "metin yatayda kaymaz");
}

/// Önce/sonra ölçüm tablosu (`cargo test -p nuxut-render optical_table -- --ignored --nocapture`).
#[test]
#[ignore = "ölçüm tablosu"]
fn optical_table() {
    let mut ts = TextSystem::new();
    let cases: &[(&str, &str, f32, f32)] = &[
        ("Inter", "14:05", 13.0, 1.3),
        ("Adwaita Sans", "14:05", 13.0, 1.3),
        ("Noto Sans", "14:05", 13.0, 1.3),
        ("Google Sans Flex", "14:05", 13.0, 1.3),
        ("JetBrainsMono Nerd Font", "14:05", 13.0, 1.3),
        ("Symbols Nerd Font", "\u{f0239}", 15.0, 1.3),
        ("JetBrainsMono Nerd Font", "\u{f0239}", 15.0, 1.3),
        ("JetBrainsMono Nerd Font", "\u{f018d}", 15.0, 1.3),
        ("Material Symbols Rounded", "home", 20.0, 1.0),
    ];
    println!("| font | metin | ölçek | önce dx | önce dy | sonra dx | sonra dy |");
    println!("|---|---|---|---|---|---|---|");
    for &(fam, text, size, lh) in cases {
        if !ts.has_family(fam) {
            println!("| {fam} | kurulu değil |");
            continue;
        }
        let mut style = TextStyle::new(fam, size, 400);
        style.line_height = lh;
        let before = ts.layout(text, &style.clone().with_optical(OpticalCenter::Metrics), None);
        let after = ts.layout(text, &style, None);
        for s in SCALES {
            // Üç hücre merkezinde en kötü (mutlak en büyük) değer.
            let worst = |ts: &mut TextSystem, l: &TextLayout, legacy: bool| {
                let mut w = (0.0f32, 0.0f32);
                for c in CENTERS {
                    let (dx, dy) = offset(ts, l, Point::new(c.0, c.1), s, legacy).unwrap_or_default();
                    if dx.abs() > w.0.abs() {
                        w.0 = dx;
                    }
                    if dy.abs() > w.1.abs() {
                        w.1 = dy;
                    }
                }
                w
            };
            let b = worst(&mut ts, &before, true);
            let a = worst(&mut ts, &after, false);
            let shown = if text.is_ascii() { text.to_owned() } else { format!("U+{:X}", text.chars().next().map_or(0, u32::from)) };
            println!("| {fam} {size}px | {shown} | {s} | {:+.2} | {:+.2} | {:+.2} | {:+.2} |", b.0, b.1, a.0, a.1);
        }
    }
}
