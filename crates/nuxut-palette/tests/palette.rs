//! Palet üretiminin deterministikliği, rol eşlemesi ve dışa aktarım.

use nuxut_core::Color;
#[cfg(feature = "image")]
use nuxut_image::Image;
use nuxut_palette::{Mode, Palette, THEME_ROLES, Variant, export};

/// Çoğunluğu turuncu, biraz mavi ve gri içeren yapay "duvar kağıdı".
#[cfg(feature = "image")]
fn wallpaper() -> Image {
    let (w, h) = (96u32, 64u32);
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let c = if x < 60 { [230, 120, 40, 255] } else if y < 20 { [40, 90, 200, 255] } else { [90, 90, 90, 255] };
            px.extend_from_slice(&c);
        }
    }
    Image::from_straight(w, h, px).unwrap()
}

#[test]
#[cfg(feature = "image")]
fn extraction_and_generation_are_deterministic() {
    use nuxut_palette::source_color;
    let a = source_color(&wallpaper());
    let b = source_color(&wallpaper());
    assert_eq!(a, b);
    // Baskın renk turuncu ailesinden seçilmeli.
    assert!(a.r > a.b, "{a:?}");
    for v in Variant::ALL {
        let p1 = Palette::generate(a, v, Mode::Dark, 0.0);
        let p2 = Palette::generate(a, v, Mode::Dark, 0.0);
        assert_eq!(p1, p2, "{v}");
        assert_eq!(export::palette_toml(&p1, None), export::palette_toml(&p2, None));
    }
}

#[test]
fn known_source_maps_to_stable_roles() {
    let p = Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0);
    let toml = export::palette_toml(&p, None);
    let v: toml::Table = toml.parse().unwrap();
    let colors = v["colors"].as_table().unwrap();
    assert_eq!(colors.len(), THEME_ROLES.len());
    for (k, _) in THEME_ROLES {
        assert!(Color::from_hex(colors[k].as_str().unwrap()).is_ok(), "{k}");
    }
    // Tonal-spot, #4285f4, koyu: primary T80 (MCU referansı #adc6ff), on_primary T20 (anlık görüntü:
    // material-colors sürümü değişirse bilinçli güncellenir).
    assert_eq!(colors["primary"].as_str(), Some("#adc6ff"));
    assert_eq!(colors["on_primary"].as_str(), Some("#102f60"));
    assert_eq!(v["meta"]["scheme"].as_str(), Some("tonal-spot"));
}

#[test]
fn contrast_between_text_and_surface() {
    // WCAG göreli parlaklık oranı; on_surface/surface ≥ 7, on_primary/primary ≥ 4.5.
    let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let lum = |c: Color| 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
    let ratio = |a: Color, b: Color| {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    };
    for mode in [Mode::Dark, Mode::Light] {
        for v in Variant::ALL {
            let p = Palette::generate(Color::from_hex("#c0602a").unwrap(), v, mode, 0.0);
            let r = p.active();
            let get = |k| r.get(k).unwrap();
            assert!(ratio(get("on_surface"), get("surface")) >= 7.0, "{v} {mode:?}");
            assert!(ratio(get("on_primary"), get("primary")) >= 4.5, "{v} {mode:?}");
        }
    }
}

#[test]
fn exports_builtins_and_user_templates() {
    let p = Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::Vibrant, Mode::Light, 0.0);
    let dir = tempfile::tempdir().unwrap();
    let user = dir.path().join("templates");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(user.join("kitty.conf"), "background {{colors.surface.default.hex}}\n").unwrap();
    std::fs::write(user.join("waybar.css"), "@define-color p {{colors.primary.dark.hex}};\n").unwrap();
    std::fs::write(user.join("broken.txt"), "{{ colors.nope.default.hex }}").unwrap();
    std::fs::write(user.join(".hidden"), "x").unwrap();
    let out = dir.path().join("exports");
    let results = export::export_templates(&p, None, &out, Some(&user));
    let ok: Vec<_> = results.iter().filter(|r| r.result.is_ok()).map(|r| r.name.as_str()).collect();
    let bad: Vec<_> = results.iter().filter(|r| r.result.is_err()).map(|r| r.name.as_str()).collect();
    assert_eq!(bad, ["broken.txt"]);
    assert_eq!(ok.len(), export::BUILTIN_TEMPLATES.len() + export::BUILTIN_GENERATED.len() + 1);
    let kitty = std::fs::read_to_string(out.join("kitty.conf")).unwrap();
    assert_eq!(kitty, format!("background {}\n", p.light.get("surface").unwrap().to_hex()));
    assert!(!out.join("broken.txt").exists(), "hatalı şablon yarım dosya bırakmamalı");
    assert!(!out.join(".hidden").exists());
    let hypr = std::fs::read_to_string(out.join("hyprland.conf")).unwrap();
    assert!(hypr.contains("col.active_border = rgba("), "{hypr}");
    let qt = std::fs::read_to_string(out.join("qt6ct-colors.conf")).unwrap();
    let active = qt.lines().find(|l| l.starts_with("active_colors=")).unwrap();
    assert_eq!(active.split(", ").count(), 22);
}

#[test]
fn theme_layer_reads_generated_palette() {
    let p = Palette::generate(Color::from_hex("#c0602a").unwrap(), Variant::Expressive, Mode::Dark, 0.0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nuxut").join(nuxut_theme::PALETTE_FILE);
    export::generated_palette(&p, Some(std::path::Path::new("/tmp/w.png"))).save(&path).unwrap();
    let loaded = nuxut_theme::GeneratedPalette::load(&path).unwrap();
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let g = loaded.value;
    assert_eq!(g.colors.primary, p.active().get("primary"));
    assert_eq!(g.meta.scheme.as_deref(), Some("expressive"));
    assert_eq!(g.meta.image.as_deref(), Some("/tmp/w.png"));
    assert_eq!(g.material.len(), p.active().len());
    assert!(g.material.len() <= nuxut_theme::MAX_MATERIAL_ROLES);
    // Metin çıktısı dosyayla aynı tabloyu verir.
    let text: toml::Table = export::palette_toml(&p, None).parse().unwrap();
    assert_eq!(text["colors"]["primary"].as_str(), Some(p.active().get("primary").unwrap().to_hex().as_str()));
}

fn contrast_ratio(a: Color, b: Color) -> f32 {
    let lin = |c: f32| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let lum = |c: Color| 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn every_base_pair_meets_wcag_aa_in_both_modes() {
    use nuxut_palette::{Role as R, Scheme};
    // Temel roller: onX / X ≥ 4.5 (WCAG AA normal metin), her varyant, iki kip, farklı kaynaklar.
    const PAIRS: [(R, R); 18] = [
        (R::OnPrimary, R::Primary),
        (R::OnPrimaryContainer, R::PrimaryContainer),
        (R::OnSecondary, R::Secondary),
        (R::OnSecondaryContainer, R::SecondaryContainer),
        (R::OnTertiary, R::Tertiary),
        (R::OnTertiaryContainer, R::TertiaryContainer),
        (R::OnError, R::Error),
        (R::OnErrorContainer, R::ErrorContainer),
        (R::OnSuccess, R::Success),
        (R::OnSuccessContainer, R::SuccessContainer),
        (R::OnWarning, R::Warning),
        (R::OnWarningContainer, R::WarningContainer),
        (R::OnSurface, R::Surface),
        (R::OnSurface, R::SurfaceContainerHighest),
        (R::OnSurfaceVariant, R::Surface),
        (R::OnBackground, R::Background),
        (R::InverseOnSurface, R::InverseSurface),
        (R::OnPrimaryFixed, R::PrimaryFixed),
    ];
    for src in ["#4285f4", "#c0602a", "#33aa55", "#ffd400", "#808080", "#e0006c"] {
        let src = Color::from_hex(src).unwrap();
        for v in Variant::ALL {
            for mode in [Mode::Dark, Mode::Light] {
                let s = Scheme::generate(src, v, mode, 0.0);
                for (fg, bg) in PAIRS {
                    let r = contrast_ratio(s.get(fg), s.get(bg));
                    assert!(r >= 4.5, "{} / {} = {r:.2} ({v}, {mode:?}, {})", fg.snake(), bg.snake(), src.to_hex());
                }
            }
        }
    }
}

#[test]
fn scheme_matches_role_map_and_theme_layer() {
    // Adlandırılmış yapı, şablonların/palette.toml'un gördüğü haritayla birebir aynı.
    let p = Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0);
    for mode in [Mode::Dark, Mode::Light] {
        let s = p.scheme(mode);
        for (name, c) in s.iter_snake() {
            assert_eq!(p.roles(mode).get(name), Some(c), "{name}");
        }
        assert_eq!(s, &nuxut_palette::Scheme::generate(p.source, p.variant, mode, p.contrast));
    }
    // Tema katmanının aldığı 11 rol yapıdaki değerlerle aynı.
    let s = p.scheme(Mode::Dark);
    for (ours, m3) in THEME_ROLES {
        let c = p.theme_colors().into_iter().find(|(n, _)| *n == ours).unwrap().1;
        assert_eq!(Some(c), s.by_name(m3), "{ours}");
    }
    assert_eq!(s.primary.to_hex(), "#adc6ff");
    // camelCase adları da çözülür.
    assert_eq!(s.by_name("onPrimaryContainer"), Some(s.on_primary_container));
}

#[test]
fn gtk_outputs_match_golden() {
    use nuxut_palette::look;
    let p = Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0);
    let cases = [
        ("gtk4.css", look::gtk4_css(p.scheme(Mode::Dark), p.scheme(Mode::Light), Mode::Dark)),
        ("gtk3-dark.css", look::gtk3_css(p.scheme(Mode::Dark), Mode::Dark)),
        ("gtk3-light.css", look::gtk3_css(p.scheme(Mode::Light), Mode::Light)),
    ];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let bless = std::env::var_os("NUXUT_BLESS").is_some();
    for (name, got) in cases {
        let path = dir.join(name);
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        // nuxut-apply (edit::managed) ve shell yönetilen dosyayı ilk satırdaki bu başlıkla tanır.
        assert!(got.starts_with("/* Nuxut — "), "{name}: yönetilen dosya başlığı yok");
        let want = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{name} yok: NUXUT_BLESS=1 ile üretin"));
        assert_eq!(got, want, "{name} altın dosyadan farklı (bilinçli değişiklikse NUXUT_BLESS=1)");
    }
    // Dışa aktarım aynı üreticiyi kullanır.
    let out = tempfile::tempdir().unwrap();
    export::export_templates(&p, None, out.path(), None);
    assert_eq!(std::fs::read_to_string(out.path().join("gtk4.css")).unwrap(), std::fs::read_to_string(dir.join("gtk4.css")).unwrap());
}

#[test]
fn static_light_palette_matches_derivation() {
    // nuxut-theme'deki statik açık palet, koyu varsayılanın primary'sinden tonal-spot açık şema.
    let seed = nuxut_theme::Palette::default().primary;
    let p = Palette::generate(seed, Variant::TonalSpot, Mode::Light, 0.0);
    let mut derived = nuxut_theme::Palette::default();
    derived.apply(&export::generated_palette(&p, None).colors);
    assert_eq!(derived, nuxut_theme::Palette::light());
    assert_eq!(Mode::from(nuxut_theme::ThemeMode::Light), Mode::Light);
    assert_eq!(nuxut_theme::ThemeMode::from(Mode::Light), nuxut_theme::ThemeMode::Light);
    assert_eq!(nuxut_theme::ColorMode::from(Mode::Dark), nuxut_theme::ColorMode::Dark);
}

#[test]
fn generated_palette_carries_both_modes() {
    let p = Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0);
    let g = export::generated_palette(&p, None);
    assert_eq!(g.dark.unwrap().primary, p.dark.get("primary"));
    assert_eq!(g.light.unwrap().primary, p.light.get("primary"));
    assert_eq!(g.colors, g.dark.unwrap());
    let back: nuxut_theme::GeneratedPalette = toml::from_str(&export::palette_toml(&p, None)).unwrap();
    assert_eq!(back.colors_for(nuxut_theme::ThemeMode::Light), (&g.light.unwrap(), false));
    // `auto` için kayıtlı kip ve yüzey eşiği üretilen şemalarla tutarlı.
    assert_eq!(back.recorded_mode(), Some(nuxut_theme::ThemeMode::Dark));
    for (patch, mode) in [(g.dark.unwrap(), nuxut_theme::ThemeMode::Dark), (g.light.unwrap(), nuxut_theme::ThemeMode::Light)] {
        assert_eq!(nuxut_theme::ThemeMode::of_surface(patch.surface.unwrap()), mode);
    }
}

/// Belirlenimci kaynak renkler: uçlar (siyah, beyaz, gri, saf ana renkler, pastel/koyu tonlar)
/// + LCG ile 80 rastgele renk.
fn many_sources() -> Vec<Color> {
    let mut out: Vec<Color> = [
        "#000000", "#ffffff", "#808080", "#ff0000", "#00ff00", "#0000ff", "#ffff00", "#00ffff", "#ff00ff", "#4285f4",
        "#c0602a", "#ffd400", "#e0006c", "#1a1a2e", "#f5f0e1", "#3b0a45", "#9ec5ff",
    ]
    .iter()
    .map(|h| Color::from_hex(h).unwrap())
    .collect();
    let mut x: u32 = 0x2545_f491;
    for _ in 0..80 {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push(Color::from_rgba8((x >> 24) as u8, (x >> 16) as u8, (x >> 8) as u8, 255));
    }
    out
}

/// Yüksek karşıtlık kipinin tüm garantileri (docs/design/accessibility.md §1): her kaynak ×
/// varyant × kip × MCU kontrast düzeyi.
#[test]
fn high_contrast_meets_wcag_aa_everywhere() {
    use nuxut_palette::{ContrastMode, ON_PAIRS, Role as R, SURFACE_ROLES, Scheme};
    const TEXT: f32 = 4.5;
    const NON_TEXT: f32 = 3.0;
    let mut checked = 0usize;
    for src in many_sources() {
        for v in Variant::ALL {
            for mode in [Mode::Dark, Mode::Light] {
                for level in [-1.0, 0.0, 1.0] {
                    let s = Scheme::generate_with(src, v, mode, level, ContrastMode::High);
                    let ctx = || format!("({}, {v}, {mode:?}, {level})", src.to_hex());
                    let check = |fg: R, bg: R, min: f32| {
                        let r = contrast_ratio(s.get(fg), s.get(bg));
                        assert!(r >= min, "{} / {} = {r:.2} {}", fg.snake(), bg.snake(), ctx());
                    };
                    for bg in SURFACE_ROLES {
                        for fg in [R::OnSurface, R::OnBackground, R::OnSurfaceVariant, R::Primary, R::Secondary, R::Tertiary, R::Error, R::Success, R::Warning, R::Outline] {
                            check(fg, bg, TEXT);
                        }
                        check(R::OutlineVariant, bg, NON_TEXT);
                    }
                    for (fg, bg) in ON_PAIRS {
                        check(fg, bg, TEXT);
                    }
                    check(R::InverseOnSurface, R::InverseSurface, TEXT);
                    check(R::InversePrimary, R::InverseSurface, TEXT);
                    for (on, bgs) in [
                        (R::OnPrimaryFixed, [R::PrimaryFixed, R::PrimaryFixedDim]),
                        (R::OnPrimaryFixedVariant, [R::PrimaryFixed, R::PrimaryFixedDim]),
                        (R::OnSecondaryFixed, [R::SecondaryFixed, R::SecondaryFixedDim]),
                        (R::OnSecondaryFixedVariant, [R::SecondaryFixed, R::SecondaryFixedDim]),
                        (R::OnTertiaryFixed, [R::TertiaryFixed, R::TertiaryFixedDim]),
                        (R::OnTertiaryFixedVariant, [R::TertiaryFixed, R::TertiaryFixedDim]),
                    ] {
                        for bg in bgs {
                            check(on, bg, TEXT);
                        }
                    }
                    assert_eq!(s, s.high_contrast(), "idempotent {}", ctx());
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 97 * 9 * 2 * 3);
}

/// Yüksek karşıtlık dinamik rengi korur: yüzeyler aynı, vurguların hue'su yerinde.
#[test]
fn high_contrast_keeps_dynamic_identity() {
    use material_colors::color::Argb;
    use material_colors::hct::Hct;
    use nuxut_palette::{ContrastMode, Role as R, SURFACE_ROLES, Scheme};
    let hct = |c: Color| Hct::new(Argb::new(255, (c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8));
    for src in many_sources() {
        for v in Variant::ALL {
            for mode in [Mode::Dark, Mode::Light] {
                let std = Scheme::generate(src, v, mode, 0.0);
                let high = Scheme::generate_with(src, v, mode, 0.0, ContrastMode::High);
                assert_eq!(Scheme::generate_with(src, v, mode, 0.0, ContrastMode::Standard), std);
                for r in SURFACE_ROLES {
                    assert_eq!(std.get(r), high.get(r), "{} {v} {mode:?}", r.snake());
                }
                for r in [R::Primary, R::Secondary, R::Tertiary, R::PrimaryContainer] {
                    let (a, b) = (hct(std.get(r)), hct(high.get(r)));
                    // Akromatik (kroma < 8) rengin hue'su anlamsız; renkli kimlik ölçülür.
                    if a.get_chroma() >= 8.0 && b.get_chroma() >= 8.0 {
                        let d = (a.get_hue() - b.get_hue()).rem_euclid(360.0);
                        assert!(d.min(360.0 - d) < 8.0, "{} hue kaydı {d:.1} ({} {v} {mode:?})", r.snake(), src.to_hex());
                    }
                }
            }
        }
    }
}

/// Tema katmanı (framework.toml `contrast = "high"`): 11 rol, statik ve üretilmiş paletler.
#[test]
fn theme_layer_high_contrast_on_generated_and_static_palettes() {
    use nuxut_theme::{Palette as ThemePalette, PalettePatch, ThemeMode};
    let check = |p: &ThemePalette, ctx: &str| {
        let h = p.high_contrast();
        for s in [h.surface, h.surface_container, h.surface_bright] {
            for fg in [h.on_surface, h.on_surface_variant, h.primary, h.error] {
                assert!(contrast_ratio(fg, s) >= 4.5, "{ctx}: {fg:?} / {s:?}");
            }
            assert!(contrast_ratio(h.outline, s) >= 3.0, "{ctx}: outline");
        }
        assert!(contrast_ratio(h.on_primary, h.primary) >= 4.5, "{ctx}");
        assert!(contrast_ratio(h.on_error, h.error) >= 4.5, "{ctx}");
        assert_eq!(h, h.high_contrast(), "{ctx}");
    };
    for m in [ThemeMode::Dark, ThemeMode::Light] {
        check(&ThemePalette::for_mode(m), &format!("statik {}", m.name()));
    }
    for src in many_sources() {
        for v in Variant::ALL {
            let p = Palette::generate(src, v, Mode::Dark, 0.0);
            for mode in [Mode::Dark, Mode::Light] {
                let mut t = ThemePalette::for_mode(mode.into());
                let patch = PalettePatch {
                    surface: p.roles(mode).get("surface"),
                    surface_container: p.roles(mode).get("surface_container"),
                    surface_bright: p.roles(mode).get("surface_container_high"),
                    on_surface: p.roles(mode).get("on_surface"),
                    on_surface_variant: p.roles(mode).get("on_surface_variant"),
                    primary: p.roles(mode).get("primary"),
                    on_primary: p.roles(mode).get("on_primary"),
                    error: p.roles(mode).get("error"),
                    on_error: p.roles(mode).get("on_error"),
                    outline: p.roles(mode).get("outline_variant"),
                    shadow: p.roles(mode).get("shadow"),
                };
                t.apply(&patch);
                check(&t, &format!("{} {v} {mode:?}", src.to_hex()));
            }
        }
    }
}

/// Nuxut renk temaları (docs/design/color-themes.md §Kontrast): dış uygulamalara giden tam rol
/// kümesi her tema × kip × karşıtlıkta `on_X`/`X` ≥ 4.5; vurgu rolleri yüzeylere ≥ 3 (yüksekte
/// 4.5). Temanın 11 token'ı karşılık gelen rollerle aynı (Nuxut ile dış uygulamalar aynı rengi
/// görür); Nuxut mavisi açık kipi tohumun tonal-spot şemasıyla bayt bayt aynı.
#[test]
fn color_themes_export_meets_contrast_and_matches_tokens() {
    use nuxut_palette::{ContrastMode, ON_PAIRS, Role as R, SURFACE_ROLES};
    use nuxut_theme::{ColorRole, ColorThemeId, ThemeMode};
    for id in ColorThemeId::ALL {
        let theme = id.theme();
        for level in [ContrastMode::Standard, ContrastMode::High] {
            let p = Palette::from_color_theme(theme, Mode::Dark, level);
            assert_eq!(p.source, theme.seed);
            for mode in [Mode::Dark, Mode::Light] {
                let s = p.scheme(mode);
                let tag = format!("{} {mode:?} {}", id.name(), level.name());
                for (fg, bg) in ON_PAIRS {
                    let r = contrast_ratio(s.get(fg), s.get(bg));
                    assert!(r >= 4.5, "{tag}: {} / {} {r:.2}", fg.snake(), bg.snake());
                }
                let min = if level.is_high() { 4.5 } else { 3.0 };
                for surface in [R::Surface, R::SurfaceContainer, R::SurfaceContainerHigh] {
                    for fg in [R::Primary, R::Secondary, R::Tertiary, R::Error] {
                        let r = contrast_ratio(s.get(fg), s.get(surface));
                        assert!(r >= min, "{tag}: {} / {} {r:.2}", fg.snake(), surface.snake());
                    }
                    let r = contrast_ratio(s.on_surface, s.get(surface));
                    assert!(r >= 4.5, "{tag}: on_surface {r:.2}");
                }
                assert!(SURFACE_ROLES.contains(&R::Surface));
                if !level.is_high() {
                    let tokens = theme.palette(ThemeMode::from(mode));
                    for (ours, m3) in THEME_ROLES {
                        let want = tokens.get(ColorRole::from_name(ours).unwrap());
                        assert_eq!(s.get(R::from_name(m3).unwrap()), want, "{tag}: {ours}");
                    }
                    assert_eq!(p.roles(mode).get("source_color"), Some(theme.seed));
                    assert!(p.roles(mode).get("term15").is_some());
                }
            }
        }
    }
    let blue = Palette::from_color_theme(ColorThemeId::Blue.theme(), Mode::Light, ContrastMode::Standard);
    let tonal = Palette::generate(ColorThemeId::Blue.theme().seed, Variant::TonalSpot, Mode::Light, 0.0);
    assert_eq!(blue.scheme(Mode::Light).primary, tonal.scheme(Mode::Light).primary);
    assert_eq!(blue.scheme(Mode::Light), tonal.scheme(Mode::Light));
}
