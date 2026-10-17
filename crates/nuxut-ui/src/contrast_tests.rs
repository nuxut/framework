//! Yüksek karşıtlık (t124, docs/design/accessibility.md §1 "Bileşen karışımları"): bileşenlerin
//! KENDİ karıştırdığı renkler de eşikte mi?
//!
//! Kanıt kalıbı: çok sayıda kaynak renkten üretilen palet (+ rastgele "kötü" paletler) × koyu/açık
//! için yüksek karşıtlıklı tema kurulur; bileşenlerin renk seçen saf fonksiyonlarından
//! (`variant_colors`, `ChipColors`, `ToggleColors`, `SliderColors`, `DayColors`, `tile_colors`,
//! `agenda::palette`, `week::event_fill`) ve satır içi karışımların kullandığı `style`
//! yardımcılarından (aynı argümanlarla) çıkan her ön plan, GERÇEKTEN üstüne çizildiği zeminin
//! birleşimine (alfalı dolgular üç tema yüzeyinin üstüne sırayla bindirilir, sonuç 8 bit
//! nicelenir) karşı ölçülür. Metin ≥ 4.5, metin dışı (iz, halka, ayraç, işaret) ≥ 3, devre dışı
//! ≥ 3 (karar). Standart kipte yardımcılar eski ifadeyi bayt bayt verir.

// Dosyanın tamamı test (lib.rs `#[cfg(test)] mod`); gömülü renk bekçisi bu satırdan sonrasını atlar.
#[cfg(test)]
use nuxut_core::Color;
use nuxut_theme::contrast::{NON_TEXT_MIN, TEXT_MIN};
use nuxut_theme::{Palette as ThemePalette, STATE_HOVER, STATE_PRESSED, Theme};

use crate::button::{ButtonVariant, variant_colors};
use crate::calendar::DayColors;
use crate::chip::ChipColors;
use crate::slider::SliderColors;
use crate::style::{self, DISABLED_CONTAINER, DISABLED_CONTENT, TONAL_MIX, over};
use crate::toggle::ToggleColors;

fn q(c: Color) -> Color {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
    Color::rgba(b(c.r), b(c.g), b(c.b), 1.0)
}

/// `fg`'nin, `fills` yığınının (alttan üste) her tema yüzeyi üstündeki birleşimine en kötü oranı;
/// ölçüm 8 bit nicelenmiş renkle (ekrandaki piksel).
fn ratio(theme: &Theme, fg: Color, fills: &[Color]) -> f32 {
    style::backdrops(theme, fills)
        .iter()
        .map(|bg| style::contrast_ratio(q(over(fg, *bg)), q(*bg)))
        .fold(f32::INFINITY, f32::min)
}

#[track_caller]
fn at_least(theme: &Theme, what: &str, fg: Color, fills: &[Color], min: f32) {
    let r = ratio(theme, fg, fills);
    assert!(r >= min, "{what}: {r:.3} < {min} (fg {}, fills {:?})", fg.to_hex(), fills.iter().map(|c| c.to_hex()).collect::<Vec<_>>());
}

fn lcg(x: &mut u32) -> u8 {
    *x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    (*x >> 24) as u8
}

/// Yüksek karşıtlıklı temalar: üretilmiş paletler (kaynak × varyant × kip) + rastgele "kötü"
/// paletler (koyu/açık yüzeyler, rastgele ön planlar; `high_contrast` dönüşümünden geçer).
fn themes() -> Vec<(String, Theme)> {
    use nuxut_palette::{Mode, Palette, Variant};
    let mut out = Vec::new();
    let base = |mode: &str| Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"high\"\n")).unwrap().0;
    for mode in ["dark", "light"] {
        out.push((format!("statik {mode}"), base(mode)));
    }
    let mut sources: Vec<Color> = ["#000000", "#ffffff", "#808080", "#ff0000", "#00ff00", "#0000ff", "#ffff00", "#4285f4", "#c0602a", "#ffd400", "#e0006c", "#9ec5ff"]
        .iter()
        .map(|h| Color::from_hex(h).unwrap())
        .collect();
    let mut x: u32 = 0x5eed_1234;
    for _ in 0..24 {
        sources.push(Color::from_rgba8(lcg(&mut x), lcg(&mut x), lcg(&mut x), 255));
    }
    for src in &sources {
        for v in Variant::ALL {
            for (mode, name) in [(Mode::Dark, "dark"), (Mode::Light, "light")] {
                let p = Palette::generate(*src, v, mode, 0.0);
                let mut t = base(name);
                let mut colors = ThemePalette::for_mode(t.mode);
                colors.apply(&nuxut_palette::export::generated_palette(&p, None).colors);
                t.colors = colors.high_contrast();
                out.push((format!("{} {v} {name}", src.to_hex()), t));
            }
        }
    }
    // Rastgele kötü paletler: yüzeyler kipin ucunda, ön planlar tamamen rastgele.
    for k in 0..60 {
        let dark = k % 2 == 0;
        let mut t = base(if dark { "dark" } else { "light" });
        let mut rnd = |lo: u8, hi: u8| {
            let span = (hi - lo) as u32 + 1;
            let ch = |x: &mut u32| lo + (lcg(x) as u32 % span) as u8;
            Color::from_rgba8(ch(&mut x), ch(&mut x), ch(&mut x), 255)
        };
        let (slo, shi) = if dark { (0, 60) } else { (215, 255) };
        let mut c = t.colors;
        c.surface = rnd(slo, shi);
        c.surface_container = rnd(slo, shi);
        c.surface_bright = rnd(slo, shi);
        c.on_surface = rnd(0, 255);
        c.on_surface_variant = rnd(0, 255);
        c.primary = rnd(0, 255);
        c.on_primary = rnd(0, 255);
        c.error = rnd(0, 255);
        c.on_error = rnd(0, 255);
        c.outline = rnd(0, 255);
        t.colors = c.high_contrast();
        // Palet düzeyi garanti tutmayan (orta tonlu çift; accessibility.md §1 "en iyi çaba") paletler
        // bileşen testinin konusu değil.
        if palette_holds(&t.colors) {
            out.push((format!("rastgele #{k}"), t));
        }
    }
    out
}

/// Palet düzeyi garantiler (nuxut-palette testleri bunları kanıtlar; burada yalnız süzgeç).
fn palette_holds(p: &ThemePalette) -> bool {
    let r = style::contrast_ratio;
    [p.surface, p.surface_container, p.surface_bright].iter().all(|s| {
        [p.on_surface, p.on_surface_variant, p.primary, p.error].iter().all(|fg| r(*fg, *s) >= TEXT_MIN) && r(p.outline, *s) >= NON_TEXT_MIN
    }) && r(p.on_primary, p.primary) >= TEXT_MIN
        && r(p.on_error, p.error) >= TEXT_MIN
}

const LAYERS: [f32; 3] = [0.0, STATE_HOVER, STATE_PRESSED];

#[test]
fn buttons_and_chips() {
    for (name, t) in themes() {
        for v in [ButtonVariant::Filled, ButtonVariant::Tonal, ButtonVariant::Outlined, ButtonVariant::Text, ButtonVariant::Destructive] {
            let (container, content, layer) = variant_colors(&t, v, true);
            for op in LAYERS {
                let l = style::state_layer(&t, layer, &[container], op);
                at_least(&t, &format!("{name}: düğme {v:?} {op}"), content, &[container, l], TEXT_MIN);
            }
            let (container, content, _) = variant_colors(&t, v, false);
            at_least(&t, &format!("{name}: devre dışı düğme {v:?}"), content, &[container], NON_TEXT_MIN);
        }
        for (enabled, selected) in [(true, false), (true, true), (false, false), (false, true)] {
            let ChipColors { fill, content, label } = ChipColors::new(&t, enabled, selected);
            let min = if enabled { TEXT_MIN } else { NON_TEXT_MIN };
            for op in LAYERS {
                let l = style::state_layer_under(&t, t.colors.on_surface, op, &[fill], &[content, label]);
                at_least(&t, &format!("{name}: chip {enabled}/{selected} ikon {op}"), content, &[fill, l], min);
                at_least(&t, &format!("{name}: chip {enabled}/{selected} etiket {op}"), label, &[fill, l], min);
            }
        }
        // Chip / alan kenarı ve ayraçlar: bileşen sınırı ≥ 3.
        at_least(&t, &format!("{name}: kenar"), t.outline().expect("yüksek kipte kenar her zaman"), &[], NON_TEXT_MIN);
        at_least(&t, &format!("{name}: ayraç"), style::divider(&t), &[], NON_TEXT_MIN);
    }
}

#[test]
fn toggles_sliders_and_tracks() {
    for (name, t) in themes() {
        let c = &t.colors;
        for on in [0.0, 1.0] {
            for op in LAYERS {
                let ToggleColors { track, border, thumb } = ToggleColors::new(&t, true, on, op);
                at_least(&t, &format!("{name}: anahtar topuzu t={on} {op}"), thumb, &[track], NON_TEXT_MIN);
                if on == 1.0 {
                    at_least(&t, &format!("{name}: açık ray"), track, &[], NON_TEXT_MIN);
                } else {
                    at_least(&t, &format!("{name}: kapalı ray kenarı"), border.expect("yüksek kipte kenar"), &[], NON_TEXT_MIN);
                }
            }
            let d = ToggleColors::new(&t, false, on, 0.0);
            at_least(&t, &format!("{name}: devre dışı anahtar"), d.thumb, &[d.track], NON_TEXT_MIN);
        }
        for op in LAYERS {
            let s = SliderColors::new(&t, true, op);
            at_least(&t, &format!("{name}: kaydırıcı izi {op}"), s.rest, &[], NON_TEXT_MIN);
            at_least(&t, &format!("{name}: kaydırıcı dolgusu {op}"), s.fill, &[], NON_TEXT_MIN);
            at_least(&t, &format!("{name}: kaydırıcı topuzu {op}"), s.knob, &[s.fill], NON_TEXT_MIN);
        }
        let s = SliderColors::new(&t, false, 0.0);
        at_least(&t, &format!("{name}: devre dışı kaydırıcı"), s.fill, &[s.rest], NON_TEXT_MIN);
        // İlerleme / gösterge / döner izi; dolgu `primary` (ya da özel renk) yüzeylere ≥ 3.
        at_least(&t, &format!("{name}: iz"), style::track(&t), &[], NON_TEXT_MIN);
        at_least(&t, &format!("{name}: dolgu"), c.primary, &[], NON_TEXT_MIN);
        for hex in ["#202020", "#e0e0e0", "#808080", "#ffeb3b", "#1a237e"] {
            let custom = crate::Tint::Color(Color::from_hex(hex).unwrap()).resolve(&t);
            at_least(&t, &format!("{name}: özel gösterge rengi {hex}"), custom, &[], NON_TEXT_MIN);
        }
        // Kaydırma göstergesi (metin dışı işaret).
        at_least(&t, &format!("{name}: kaydırma göstergesi"), style::faded_mark(&t, c.on_surface_variant, 0.6, &[]), &[], NON_TEXT_MIN);
    }
}

#[test]
fn calendar_agenda_and_week() {
    use crate::agenda::{AgendaItem, palette};
    for (name, t) in themes() {
        let c = &t.colors;
        for in_month in [true, false] {
            for today in [false, true] {
                for selected in [false, true] {
                    for hovered in [false, true] {
                        let d = DayColors::new(&t, in_month, today, selected, hovered);
                        let fills = [d.fill.unwrap_or(Color::TRANSPARENT), d.layer];
                        at_least(&t, &format!("{name}: gün {in_month}/{today}/{selected}/{hovered}"), d.text, &fills, TEXT_MIN);
                        if let Some(ring) = d.ring {
                            at_least(&t, &format!("{name}: bugün halkası"), ring, &[], NON_TEXT_MIN);
                        }
                    }
                }
            }
        }
        // Ay dışı gün (soluk) hover'da da okunur ve bugün dışındaki günden ayırt edilir.
        let out = DayColors::new(&t, false, false, false, false).text;
        assert!(out.a <= 1.0 && ratio(&t, out, &[]) <= ratio(&t, c.on_surface, &[]) + 1e-3, "{name}: ay dışı soluk değil");

        let past = AgendaItem::new("x", "y").past(true);
        for op in LAYERS {
            let l = style::state_layer_under(&t, c.on_surface, op, &[], &[c.on_surface, c.on_surface_variant, c.primary]);
            let (main, sub, time, stripe) = palette(&t, &past, &[l]);
            for (what, fg) in [("ana", main), ("ikincil", sub), ("zaman", time)] {
                at_least(&t, &format!("{name}: geçmiş etkinlik {what} {op}"), fg, &[l], TEXT_MIN);
            }
            at_least(&t, &format!("{name}: geçmiş etkinlik şeridi"), stripe, &[l], NON_TEXT_MIN);
        }
        let mut x: u32 = 7;
        for _ in 0..12 {
            let event = Color::from_rgba8(lcg(&mut x), lcg(&mut x), lcg(&mut x), 255);
            let fill = crate::week::event_fill(&t, event);
            for op in LAYERS {
                let l = style::state_layer_under(&t, c.on_surface, op, &[fill], &[c.on_surface, c.on_surface_variant]);
                at_least(&t, &format!("{name}: etkinlik metni {} {op}", event.to_hex()), c.on_surface, &[fill, l], TEXT_MIN);
                at_least(&t, &format!("{name}: etkinlik saati {} {op}", event.to_hex()), c.on_surface_variant, &[fill, l], TEXT_MIN);
            }
        }
    }
}

#[test]
fn rows_fields_tiles_focus() {
    for (name, t) in themes() {
        let c = &t.colors;
        let tonal = style::tonal_container(&t);
        // Tonal kap (seçili satır, gezinme göstergesi, bağlı cihaz) + hover.
        for under in [Color::TRANSPARENT, tonal] {
            let accent = style::accent_on(&t, under);
            for op in LAYERS {
                let l = style::state_layer_under(&t, c.on_surface, op, &[under], &[c.on_surface, c.on_surface_variant, accent]);
                for (what, fg) in [("metin", c.on_surface), ("ikincil", c.on_surface_variant), ("vurgu", accent)] {
                    at_least(&t, &format!("{name}: satır {what} {} {op}", under.a), fg, &[under, l], TEXT_MIN);
                }
            }
            at_least(&t, &format!("{name}: satır içi odak halkası"), style::focus_ring(&t, &[under]), &[under], NON_TEXT_MIN);
            at_least(&t, &format!("{name}: devre dışı satır"), style::disabled_content(&t, c.on_surface_variant, &[under]), &[under], NON_TEXT_MIN);
        }
        at_least(&t, &format!("{name}: odak halkası"), style::focus_ring(&t, &[]), &[], NON_TEXT_MIN);
        // Metinli rozet (Pill): opak zemini üstünde Caption vurgu metni ≥ 4.5 — normalde tonal
        // kap, seçili (tonal) satırda `surface`; altındaki katman hapın altında kalır (zemin opak).
        for (under, on_tonal) in [(Color::TRANSPARENT, false), (tonal, true)] {
            let (pill_bg, pill_fg) = crate::badge::pill_colors(&t, on_tonal);
            at_least(&t, &format!("{name}: hap metni {on_tonal}"), pill_fg, &[under, pill_bg], TEXT_MIN);
        }
        // Metin alanı / açılır seçim: hover'lı alan üstünde metin ve yer tutucu, seçim vurgusu.
        let field = style::field_fill(&t);
        for h in [0.0, 1.0] {
            let hover = style::state_layer_under(&t, c.on_surface, STATE_HOVER * h, &[field], &[c.on_surface, c.on_surface_variant]);
            let fill = field.lerp(&c.on_surface, hover.a);
            at_least(&t, &format!("{name}: alan metni {h}"), c.on_surface, &[fill], TEXT_MIN);
            at_least(&t, &format!("{name}: yer tutucu {h}"), c.on_surface_variant, &[fill], TEXT_MIN);
            let sel = style::state_layer_under(&t, c.primary, 0.4, &[fill], &[c.on_surface]);
            at_least(&t, &format!("{name}: seçili metin {h}"), c.on_surface, &[fill, sel], TEXT_MIN);
            at_least(&t, &format!("{name}: alan odak çizgisi {h}"), style::focus_ring(&t, &[fill]), &[fill], NON_TEXT_MIN);
            at_least(&t, &format!("{name}: devre dışı alan {h}"), style::disabled_content(&t, c.on_surface, &[fill]), &[fill], NON_TEXT_MIN);
        }
        // Menü satırı: hover/basılı katman (üstünde ana, ikincil ve `primary` geri ikonu).
        for op in LAYERS {
            let l = style::state_layer_under(&t, c.on_surface, op, &[], &[c.on_surface, c.on_surface_variant, c.primary]);
            for fg in [c.on_surface, c.on_surface_variant, c.primary] {
                at_least(&t, &format!("{name}: menü {op}"), fg, &[l], TEXT_MIN);
            }
        }
        at_least(&t, &format!("{name}: devre dışı menü öğesi"), style::disabled_content(&t, c.on_surface, &[]), &[], NON_TEXT_MIN);
        // Bölümlü düğmede tek devre dışı seçenek: ray üstünde ve (seçiliyse) devre dışı kap üstünde.
        let cap = style::disabled_container(&t);
        for under in [&[c.surface_bright][..], &[c.surface_bright, cap][..]] {
            at_least(&t, &format!("{name}: devre dışı bölüm {}", under.len()), style::disabled_content(&t, c.on_surface, under), under, NON_TEXT_MIN);
        }
        // Döşeme: kapalı/açık × hover; "›" ayracı.
        for on in [0.0, 1.0] {
            let (fill, content, sub) = crate::tile::tile_colors(&t, true, on);
            let layer = c.on_surface.lerp(&c.on_primary, on);
            for op in LAYERS {
                let l = style::state_layer_under(&t, layer, op, &[fill], &[content, sub]);
                at_least(&t, &format!("{name}: döşeme {on} {op}"), content, &[fill, l], TEXT_MIN);
                at_least(&t, &format!("{name}: döşeme alt {on} {op}"), sub, &[fill, l], TEXT_MIN);
            }
            at_least(&t, &format!("{name}: döşeme ayracı {on}"), style::hairline(&t, content, style::DIVIDER_TONE * 2.0, &[fill]), &[fill], NON_TEXT_MIN);
        }
        // Rastgele renk örneği üstünde içerik (renk seçici, avatar).
        let mut x: u32 = 99;
        for _ in 0..8 {
            let bg = Color::from_rgba8(lcg(&mut x), lcg(&mut x), lcg(&mut x), 255);
            at_least(&t, &format!("{name}: örnek içeriği {}", bg.to_hex()), style::content_on(&t, bg), &[bg], TEXT_MIN);
        }
    }
}

/// Standart kipte yardımcılar eski ifadeyi aynen verir (mevcut çıktı değişmez).
#[test]
fn standard_mode_is_unchanged() {
    for mode in ["dark", "light"] {
        let (t, _) = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\n")).unwrap();
        assert!(!t.contrast.is_high());
        let c = &t.colors;
        assert_eq!(style::tonal_container(&t), c.surface_container.lerp(&c.primary, TONAL_MIX));
        assert_eq!(style::track(&t), style::tonal_container(&t));
        assert_eq!(style::disabled_content(&t, c.on_surface, &[]), c.on_surface.multiply_alpha(DISABLED_CONTENT));
        assert_eq!(style::disabled_container(&t), c.on_surface.multiply_alpha(DISABLED_CONTAINER));
        assert_eq!(style::state_layer(&t, c.on_surface, &[], STATE_HOVER), c.on_surface.multiply_alpha(STATE_HOVER));
        assert_eq!(style::faded_text(&t, c.on_surface, DISABLED_CONTENT, &[]), c.on_surface.multiply_alpha(DISABLED_CONTENT));
        assert_eq!(style::focus_ring(&t, &[c.primary]), c.primary);
        assert_eq!(style::accent_on(&t, style::tonal_container(&t)), c.primary);
        let low = Color::from_hex("#777777").unwrap();
        assert_eq!(style::legible(&t, low, &[low], 7.0), low);
        assert_eq!(style::content_on(&t, c.primary), c.on_primary);
        for v in [ButtonVariant::Filled, ButtonVariant::Tonal, ButtonVariant::Text] {
            let (_, content, _) = variant_colors(&t, v, false);
            assert_eq!(content, c.on_surface.multiply_alpha(DISABLED_CONTENT));
        }
        let s = SliderColors::new(&t, true, STATE_HOVER);
        assert_eq!(s.rest, style::tonal_container(&t).lerp(&c.on_surface, STATE_HOVER));
        assert_eq!(s.fill, c.primary.lerp(&c.on_primary, STATE_HOVER));
        let d = DayColors::new(&t, false, false, false, true);
        assert_eq!(d.text, c.on_surface.multiply_alpha(DISABLED_CONTENT));
        assert_eq!(d.layer, c.on_surface.multiply_alpha(STATE_HOVER));
    }
}

/// Yardımcıların davranışı: en az değişim, eşiği geçen renk aynen, hue korunur.
#[test]
fn helpers_change_as_little_as_needed() {
    let (t, _) = Theme::parse("[colors]\ncontrast = \"high\"\n").unwrap();
    let c = &t.colors;
    // Eşiği geçen tam ton aynen.
    assert_eq!(style::legible(&t, c.on_surface, &style::surfaces(&t), TEXT_MIN), c.on_surface);
    // Soluk metin: yalnız opaklık artar, renk aynı; eşiğin hemen üstünde.
    let f = style::faded_text(&t, c.on_surface, DISABLED_CONTENT, &[]);
    assert_eq!((f.r, f.g, f.b), (c.on_surface.r, c.on_surface.g, c.on_surface.b));
    assert!(f.a > DISABLED_CONTENT && f.a < 1.0, "{}", f.a);
    let r = ratio(&t, f, &[]);
    assert!((TEXT_MIN..TEXT_MIN + 0.4).contains(&r), "{r}");
    // Katman: eşik zaten tutuyorsa dokunulmaz.
    assert_eq!(style::state_layer(&t, c.on_surface, &[], STATE_HOVER), c.on_surface.multiply_alpha(STATE_HOVER));
    // Ulaşılamayan opak renk uca doğru en az karışır.
    let mid = Color::from_hex("#7a7a7a").unwrap();
    let bg = Color::from_hex("#808080").unwrap();
    let out = style::legible(&t, mid, &[bg], NON_TEXT_MIN);
    assert!(style::contrast_ratio(out, bg) >= NON_TEXT_MIN);
    assert!(style::contrast_ratio(out, bg) < NON_TEXT_MIN + 0.3);
}

/// Bilgi: kaç temada hover katmanı ya da tonal karışım sınırlandı (belge için; başarısız olmaz).
#[test]
fn report_how_often_mixes_are_limited() {
    let all = themes();
    let mut layer = 0;
    let (mut kept, mut least) = (0.0_f32, 1.0_f32);
    let mut tonal = 0;
    let mut fade = 0;
    for (_, t) in &all {
        let c = &t.colors;
        let a = style::state_layer_under(t, c.primary, STATE_PRESSED, &[], &[c.primary]).a;
        if a < STATE_PRESSED - 1e-4 {
            kept += a / STATE_PRESSED;
            least = least.min(a / STATE_PRESSED);
            layer += 1;
        }
        if style::tonal_container(t) != c.surface_container.lerp(&c.primary, TONAL_MIX) {
            tonal += 1;
        }
        if style::faded_text(t, c.on_surface, DISABLED_CONTENT, &[]).a > DISABLED_CONTENT {
            fade += 1;
        }
    }
    eprintln!(
        "yüksek karşıtlık bileşen karışımları: {} tema; basılı katman (primary içerik) sınırlı {layer} (ortalama %{:.0}, en az %{:.0} korundu), tonal karışım sınırlı {tonal}, soluk metin opaklaştı {fade}",
        all.len(),
        100.0 * kept / layer.max(1) as f32,
        100.0 * least
    );
}

/// Glif ızgarası (`GlyphGrid`): seçili/seçisiz × hover/basılı hücrede sembol glifi ≥ 4.5; odak
/// halkası ve yüksek karşıtlıktaki seçim kenarı hem seçili kaba hem yüzeylere ≥ 3; başlık metni.
#[test]
fn glyph_grid_cells() {
    use crate::glyph_grid::{CellColors, header_color};
    for (name, t) in themes() {
        for selected in [false, true] {
            for op in LAYERS {
                let CellColors { fill, layer, glyph, ring } = CellColors::new(&t, selected, op);
                at_least(&t, &format!("{name}: glif {selected} {op}"), glyph, &[fill, layer], TEXT_MIN);
                at_least(&t, &format!("{name}: glif halkası (kap) {selected}"), ring, &[fill], NON_TEXT_MIN);
                at_least(&t, &format!("{name}: glif halkası (yüzey) {selected}"), ring, &[], NON_TEXT_MIN);
            }
        }
        at_least(&t, &format!("{name}: glif bölüm başlığı"), header_color(&t), &[], TEXT_MIN);
    }
}
