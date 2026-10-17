//! Birleşik görünüm (shell ADR-0004) için saf GTK üreticileri: [`Scheme`] → CSS metni.
//!
//! - [`gtk4_css`]: GTK 4 / libadwaita. libadwaita ≥ 1.6 renkleri **CSS özel değişkenlerinden**
//!   okur (`--window-bg-color` …); `@define-color` bu roller için yok sayılır (ADR-0004 B1,
//!   libadwaita 1.9.4'te doğrulandı). Dosya iki kipi de taşır: önce seçili kip `:root`
//!   (`@media` desteklemeyen eski GTK 4 için), sonra `@media (prefers-color-scheme: light|dark)`
//!   blokları — koyu/açık geçişi libadwaita'da yeniden başlatma istemez (B4).
//! - [`gtk3_css`]: GTK 3 (adw-gtk3 adları + klasik `theme_*` adları) `@define-color`; GTK 3
//!   kip sorgusu bilmez, yalnızca etkin kip yazılır.
//!
//! Değişken adları kurulu libadwaita 1.9.4 ikilisinden (`strings libadwaita-1.so`) ve
//! dots-hyprland matugen şablonundan doğrulandı. Eşleme ADR-0004 §5.1 tablosudur. Çıktı
//! deterministiktir; yol, saat ya da kullanıcı verisi içermez.

use std::fmt::Write as _;

use nuxut_core::Color;

use crate::{Mode, Role, Scheme};

/// Bir GTK renginin kaynağı.
#[derive(Debug, Clone, Copy)]
enum Src {
    Role(Role),
    /// `shadow`, kipine göre saydamlıkla (koyu 0.36, açık 0.07 — libadwaita varsayılanları).
    Shade,
}

use Role as R;
use Src::{Role as C, Shade};

/// libadwaita adı (alt çizgili; GTK 4'te `--` + tireli) → kaynak. GTK 3 adw-gtk3 aynı adları
/// `@define-color` ile kullanır.
const ADW: &[(&str, Src)] = &[
    ("accent_bg_color", C(R::Primary)),
    ("accent_fg_color", C(R::OnPrimary)),
    ("accent_color", C(R::Primary)),
    ("destructive_bg_color", C(R::Error)),
    ("destructive_fg_color", C(R::OnError)),
    ("destructive_color", C(R::Error)),
    ("success_bg_color", C(R::Success)),
    ("success_fg_color", C(R::OnSuccess)),
    ("success_color", C(R::Success)),
    ("warning_bg_color", C(R::Warning)),
    ("warning_fg_color", C(R::OnWarning)),
    ("warning_color", C(R::Warning)),
    ("error_bg_color", C(R::Error)),
    ("error_fg_color", C(R::OnError)),
    ("error_color", C(R::Error)),
    ("window_bg_color", C(R::Surface)),
    ("window_fg_color", C(R::OnSurface)),
    ("view_bg_color", C(R::SurfaceContainerLow)),
    ("view_fg_color", C(R::OnSurface)),
    ("headerbar_bg_color", C(R::SurfaceContainer)),
    ("headerbar_fg_color", C(R::OnSurface)),
    ("headerbar_border_color", C(R::OutlineVariant)),
    ("headerbar_backdrop_color", C(R::Surface)),
    ("headerbar_shade_color", Shade),
    ("sidebar_bg_color", C(R::SurfaceContainerLow)),
    ("sidebar_fg_color", C(R::OnSurface)),
    ("sidebar_backdrop_color", C(R::SurfaceContainerLow)),
    ("sidebar_border_color", C(R::OutlineVariant)),
    ("sidebar_shade_color", Shade),
    ("secondary_sidebar_bg_color", C(R::SurfaceContainer)),
    ("secondary_sidebar_fg_color", C(R::OnSurface)),
    ("secondary_sidebar_backdrop_color", C(R::SurfaceContainer)),
    ("secondary_sidebar_border_color", C(R::OutlineVariant)),
    ("secondary_sidebar_shade_color", Shade),
    ("card_bg_color", C(R::SurfaceContainer)),
    ("card_fg_color", C(R::OnSurface)),
    ("thumbnail_bg_color", C(R::SurfaceContainer)),
    ("thumbnail_fg_color", C(R::OnSurface)),
    ("dialog_bg_color", C(R::SurfaceContainerHigh)),
    ("dialog_fg_color", C(R::OnSurface)),
    ("popover_bg_color", C(R::SurfaceContainerHigh)),
    ("popover_fg_color", C(R::OnSurface)),
    ("overview_bg_color", C(R::SurfaceContainerLowest)),
    ("overview_fg_color", C(R::OnSurface)),
    ("shade_color", Shade),
];

/// Yalnızca GTK 3: klasik tema adları ve kenarlar.
const GTK3_ONLY: &[(&str, Src)] = &[
    ("borders", C(R::OutlineVariant)),
    ("unfocused_borders", C(R::OutlineVariant)),
    ("link_color", C(R::Primary)),
    ("theme_bg_color", C(R::Surface)),
    ("theme_fg_color", C(R::OnSurface)),
    ("theme_base_color", C(R::SurfaceContainerLow)),
    ("theme_text_color", C(R::OnSurface)),
    ("theme_selected_bg_color", C(R::Primary)),
    ("theme_selected_fg_color", C(R::OnPrimary)),
    ("insensitive_bg_color", C(R::SurfaceContainer)),
    ("insensitive_fg_color", C(R::OnSurfaceVariant)),
    ("insensitive_base_color", C(R::Surface)),
    ("theme_unfocused_bg_color", C(R::Surface)),
    ("theme_unfocused_fg_color", C(R::OnSurface)),
    ("theme_unfocused_base_color", C(R::SurfaceContainerLow)),
    ("theme_unfocused_text_color", C(R::OnSurface)),
    ("theme_unfocused_selected_bg_color", C(R::PrimaryContainer)),
    ("theme_unfocused_selected_fg_color", C(R::OnPrimaryContainer)),
];

/// `shadow` saydamlığı (libadwaita `--shade-color` varsayılanları).
pub const SHADE_ALPHA_DARK: f32 = 0.36;
pub const SHADE_ALPHA_LIGHT: f32 = 0.07;

/// GTK 4 / libadwaita `gtk.css` içeriği: CSS özel değişkenleri, iki kip. `active` kip
/// `@media` desteklemeyen GTK için varsayılan `:root` bloğuna yazılır.
pub fn gtk4_css(dark: &Scheme, light: &Scheme, active: Mode) -> String {
    let mut out = String::from(
        "/* Nuxut — birleşik görünüm, GTK 4 / libadwaita (≥ 1.6, CSS değişkenleri) · elle düzenleme bir sonraki uygulamada ezilir */\n",
    );
    let block = |out: &mut String, scheme: &Scheme, mode: Mode, indent: &str| {
        let _ = writeln!(out, "{indent}:root {{");
        for (name, src) in ADW {
            let _ = writeln!(out, "{indent}  --{}: {};", name.replace('_', "-"), value(scheme, mode, *src));
        }
        let _ = writeln!(out, "{indent}}}");
    };
    let pick = |m: Mode| if m == Mode::Dark { dark } else { light };
    block(&mut out, pick(active), active, "");
    for mode in [Mode::Light, Mode::Dark] {
        let _ = writeln!(out, "\n@media (prefers-color-scheme: {}) {{", mode.name());
        block(&mut out, pick(mode), mode, "  ");
        out.push_str("}\n");
    }
    out
}

/// GTK 3 `gtk.css` içeriği (adw-gtk3 + klasik adlar), `mode` kipinin şeması ile.
pub fn gtk3_css(scheme: &Scheme, mode: Mode) -> String {
    let mut out = format!("/* Nuxut — birleşik görünüm, GTK 3 ({}) · elle düzenleme bir sonraki uygulamada ezilir */\n", mode.name());
    for (name, src) in ADW.iter().chain(GTK3_ONLY) {
        let _ = writeln!(out, "@define-color {name} {};", value(scheme, mode, *src));
    }
    out
}

fn value(scheme: &Scheme, mode: Mode, src: Src) -> String {
    match src {
        Src::Role(r) => hex(scheme.get(r)),
        Src::Shade => {
            let a = if mode == Mode::Dark { SHADE_ALPHA_DARK } else { SHADE_ALPHA_LIGHT };
            let [r, g, b] = rgb8(scheme.shadow);
            format!("rgba({r}, {g}, {b}, {a})")
        }
    }
}

fn rgb8(c: Color) -> [u8; 3] {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [b(c.r), b(c.g), b(c.b)]
}

fn hex(c: Color) -> String {
    let [r, g, b] = rgb8(c);
    format!("#{r:02x}{g:02x}{b:02x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Variant;

    fn schemes() -> (Scheme, Scheme) {
        let src = Color::from_hex("#4285f4").unwrap();
        (
            Scheme::generate(src, Variant::TonalSpot, Mode::Dark, 0.0),
            Scheme::generate(src, Variant::TonalSpot, Mode::Light, 0.0),
        )
    }

    #[test]
    fn gtk4_uses_custom_properties_not_define_color() {
        let (d, l) = schemes();
        let css = gtk4_css(&d, &l, Mode::Dark);
        assert!(!css.contains("@define-color"));
        assert!(css.contains(&format!("  --window-bg-color: {};", hex(d.surface))));
        assert!(css.contains(&format!("    --window-bg-color: {};", hex(l.surface))));
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
        assert!(css.contains("@media (prefers-color-scheme: light)"));
        assert_eq!(css.matches(":root {").count(), 3);
        assert!(css.contains("--headerbar-shade-color: rgba(0, 0, 0, 0.36);"));
        assert!(css.contains("--headerbar-shade-color: rgba(0, 0, 0, 0.07);"));
        // Varsayılan blok seçili kipi izler.
        let light_first = gtk4_css(&d, &l, Mode::Light);
        let first_root = |s: &str| s.split("@media").next().unwrap().to_owned();
        assert!(first_root(&light_first).contains(&hex(l.surface)));
    }

    #[test]
    fn gtk3_has_adw_and_classic_names() {
        let (d, _) = schemes();
        let css = gtk3_css(&d, Mode::Dark);
        for name in ["accent_bg_color", "window_bg_color", "theme_selected_bg_color", "borders", "success_color"] {
            assert!(css.contains(&format!("@define-color {name} ")), "{name}");
        }
        assert!(!css.contains("--"), "GTK 3 CSS değişkeni bilmez");
    }

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (n, _) in ADW.iter().chain(GTK3_ONLY) {
            assert!(seen.insert(*n), "{n}");
        }
    }
}
