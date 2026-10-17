//! Basit ve güvenli şablon motoru: yalnızca `{{ yol }}` değiştirme. Döngü, koşul, dosya
//! ekleme, komut yok; bilinmeyen değişken hata verir (yarım dosya yazılmaz).
//!
//! Değişkenler (matugen ile uyumlu adlar):
//! - `{{colors.<rol>.<kip>.<biçim>}}` — `rol`: Material rolü (`primary`, `surface_container`…),
//!   `term0`…`term15`, `source_color`; `kip`: `default` (seçili) | `dark` | `light`;
//!   `biçim`: `hex` (#rrggbb), `hex_stripped`, `hex_argb` (#ffrrggbb, qt6ct), `rgb`, `rgba`,
//!   `hypr` (rgba(rrggbbff)), `kde` (r,g,b), `red`, `green`, `blue`.
//! - `{{mode}}`, `{{scheme}}`, `{{contrast}}`, `{{source}}` (#rrggbb).
//! - `{{image}}`: `[meta] image` ile aynı değer — görselin dosya yolu; yerleşik duvar kağıdında
//!   `nuxut:<id>[/dark|light]` adresi (yol DEĞİL, olduğu gibi); görsel yoksa boş.
//! - `{{image_path}}`: yalnızca gerçek dosya yolu; yerleşik adreste ve görsel yoksa boş. Dış
//!   araca yol veren şablonlar bunu kullanır (adres dosya olarak açılamaz).
//!
//! `{{` yazmak için `{{"{{"}}`.

use std::path::Path;

use nuxut_core::Color;

use crate::{Mode, Palette};

/// Kabul edilen en büyük şablon.
pub const MAX_TEMPLATE_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("şablon çok büyük ({0} bayt)")]
    TooLarge(usize),
    #[error("satır {line}: kapanmamış '{{{{'")]
    Unclosed { line: usize },
    #[error("satır {line}: bilinmeyen değişken '{name}'")]
    Unknown { line: usize, name: String },
}

/// Şablonu bağlamla doldurur.
pub fn render(template: &str, palette: &Palette, image: Option<&Path>) -> Result<String, TemplateError> {
    if template.len() > MAX_TEMPLATE_BYTES {
        return Err(TemplateError::TooLarge(template.len()));
    }
    let mut out = String::with_capacity(template.len() + template.len() / 4);
    let mut rest = template;
    let mut line = 1;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        line += rest[..start].matches('\n').count();
        let after = &rest[start + 2..];
        let end = after.find("}}").ok_or(TemplateError::Unclosed { line })?;
        let expr = after[..end].trim();
        if expr.contains('\n') {
            return Err(TemplateError::Unclosed { line });
        }
        let value = lookup(expr, palette, image).ok_or_else(|| TemplateError::Unknown { line, name: expr.to_owned() })?;
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

fn lookup(expr: &str, p: &Palette, image: Option<&Path>) -> Option<String> {
    match expr {
        "\"{{\"" => return Some("{{".into()),
        "mode" => return Some(p.mode.name().into()),
        "scheme" => return Some(p.variant.name().into()),
        "contrast" => return Some(format!("{:.2}", p.contrast)),
        "source" => return Some(p.source.to_hex()[..7].to_owned()),
        "image" => return Some(image.map(crate::export::image_value).unwrap_or_default()),
        "image_path" => {
            let path = image.filter(|i| !crate::export::is_builtin_address(i));
            return Some(path.map(|i| i.display().to_string()).unwrap_or_default());
        }
        _ => {}
    }
    let mut parts = expr.split('.');
    let (Some("colors"), Some(role), Some(mode), Some(fmt), None) =
        (parts.next(), parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let roles = match mode {
        "default" => p.active(),
        "dark" => p.roles(Mode::Dark),
        "light" => p.roles(Mode::Light),
        _ => return None,
    };
    format(roles.get(role)?, fmt)
}

fn format(c: Color, fmt: &str) -> Option<String> {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let (r, g, bl) = (b(c.r), b(c.g), b(c.b));
    Some(match fmt {
        "hex" => format!("#{r:02x}{g:02x}{bl:02x}"),
        "hex_stripped" => format!("{r:02x}{g:02x}{bl:02x}"),
        "hex_argb" => format!("#ff{r:02x}{g:02x}{bl:02x}"),
        "rgb" => format!("rgb({r}, {g}, {bl})"),
        "rgba" => format!("rgba({r}, {g}, {bl}, 1.0)"),
        "hypr" => format!("rgba({r:02x}{g:02x}{bl:02x}ff)"),
        "kde" => format!("{r},{g},{bl}"),
        "red" => r.to_string(),
        "green" => g.to_string(),
        "blue" => bl.to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Variant;

    fn palette() -> Palette {
        Palette::generate(Color::from_hex("#4285f4").unwrap(), Variant::TonalSpot, Mode::Dark, 0.0)
    }

    #[test]
    fn substitutes_variables() {
        let p = palette();
        let out = render("a {{ colors.source_color.default.hex }} b {{mode}}/{{scheme}} {{\"{{\"}}x}}", &p, None).unwrap();
        assert_eq!(out, "a #4285f4 b dark/tonal-spot {{x}}");
        let fmts = render(
            "{{colors.source_color.dark.hex_argb}} {{colors.source_color.light.hypr}} {{colors.source_color.default.kde}} {{colors.source_color.default.rgb}}",
            &p,
            None,
        )
        .unwrap();
        assert_eq!(fmts, "#ff4285f4 rgba(4285f4ff) 66,133,244 rgb(66, 133, 244)");
    }

    /// Yerleşik adres `{{image}}`'a olduğu gibi gider, `{{image_path}}` boş; dosya yolu ikisinde de.
    #[test]
    fn image_variables_carry_builtin_address_transparently() {
        let p = palette();
        let t = "[{{image}}|{{image_path}}]";
        let addr = Path::new("nuxut:pair-flight/dark");
        assert_eq!(render(t, &p, Some(addr)).unwrap(), "[nuxut:pair-flight/dark|]");
        assert_eq!(render(t, &p, Some(Path::new("/home/x/d.png"))).unwrap(), "[/home/x/d.png|/home/x/d.png]");
        assert_eq!(render(t, &p, None).unwrap(), "[|]");
        let meta = crate::export::generated_palette(&p, Some(addr)).meta;
        assert_eq!(meta.image.as_deref(), Some("nuxut:pair-flight/dark"));
    }

    #[test]
    fn rejects_unknown_and_unclosed() {
        let p = palette();
        assert_eq!(
            render("x\n{{ colors.nope.default.hex }}", &p, None),
            Err(TemplateError::Unknown { line: 2, name: "colors.nope.default.hex".into() })
        );
        assert!(matches!(render("{{ colors.primary.default.hex", &p, None), Err(TemplateError::Unclosed { .. })));
        assert!(matches!(render("{{ env.HOME }}", &p, None), Err(TemplateError::Unknown { .. })));
        assert!(matches!(render("{{ colors.primary.default.hex.extra }}", &p, None), Err(TemplateError::Unknown { .. })));
        assert!(matches!(render(&"x".repeat(MAX_TEMPLATE_BYTES + 1), &p, None), Err(TemplateError::TooLarge(_))));
    }

    #[test]
    fn output_is_never_reinterpreted() {
        // Değer içinde "{{" çıksa bile ikinci tur yapılmaz (tek geçiş).
        let p = palette();
        assert_eq!(render("{{\"{{\"}}mode}}", &p, None).unwrap(), "{{mode}}");
    }
}
