//! `nuxut-palette generate <görsel> [--scheme S] [--mode dark|light] [--contrast C] [--high-contrast]
//!                        [--color #rrggbb] [--state-dir D] [--templates D] [--no-export] [--dry-run]`

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use nuxut_core::Color;
use nuxut_palette::{ContrastMode, Mode, Palette, Variant, export};

const USAGE: &str = "\
kullanım: nuxut-palette generate <görsel> [seçenekler]
          nuxut-palette generate --color #rrggbb [seçenekler]
          nuxut-palette generate nuxut:<id>[/dark|/light] --color #rrggbb [seçenekler]
            (yerleşik duvar kağıdı: adres [meta] image'a yazılır, kaynak renk --color)

seçenekler:
  --scheme S       tonal-spot (varsayılan), vibrant, expressive, neutral, fidelity,
                   content, monochrome, rainbow, fruit-salad
  --mode M         dark (varsayılan) | light
  --contrast C     -1.0 … 1.0 (varsayılan 0; MCU kontrast düzeyi)
  --high-contrast  yüksek karşıtlık kipi (WCAG AA: metin ≥ 4.5:1, metin dışı ≥ 3:1)
  --state-dir D    çıktı kökü (varsayılan $XDG_STATE_HOME/nuxut)
  --templates D    kullanıcı şablon dizini (varsayılan ~/.config/nuxut/templates)
  --no-export      yalnızca palette.toml yaz
  --dry-run        hiçbir dosya yazma; palette.toml'u stdout'a bas";

struct Args {
    image: Option<PathBuf>,
    color: Option<Color>,
    variant: Variant,
    mode: Mode,
    contrast: f64,
    high_contrast: bool,
    state_dir: Option<PathBuf>,
    templates: Option<PathBuf>,
    export: bool,
    dry_run: bool,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    match it.next().as_deref() {
        Some("generate") => {}
        Some("-h" | "--help") | None => return Err(String::new()),
        Some(other) => return Err(format!("bilinmeyen komut '{other}'")),
    }
    let mut a = Args {
        image: None,
        color: None,
        variant: Variant::default(),
        mode: Mode::default(),
        contrast: 0.0,
        high_contrast: false,
        state_dir: None,
        templates: None,
        export: true,
        dry_run: false,
    };
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} bir değer bekliyor"));
        match arg.as_str() {
            "--scheme" => a.variant = value("--scheme")?.parse()?,
            "--mode" => a.mode = value("--mode")?.parse()?,
            "--contrast" => {
                a.contrast = value("--contrast")?.parse::<f64>().map_err(|e| format!("--contrast: {e}"))?;
                if !(-1.0..=1.0).contains(&a.contrast) {
                    return Err("--contrast -1.0 … 1.0 aralığında olmalı".into());
                }
            }
            "--color" => a.color = Some(Color::from_hex(&value("--color")?).map_err(|e| format!("--color: {e}"))?),
            "--state-dir" => a.state_dir = Some(value("--state-dir")?.into()),
            "--templates" => a.templates = Some(value("--templates")?.into()),
            "--high-contrast" => a.high_contrast = true,
            "--no-export" => a.export = false,
            "--dry-run" => a.dry_run = true,
            "-h" | "--help" => return Err(String::new()),
            s if s.starts_with('-') => return Err(format!("bilinmeyen seçenek '{s}'")),
            _ if a.image.is_none() => a.image = Some(arg.into()),
            _ => return Err(format!("fazladan argüman '{arg}'")),
        }
    }
    // Yerleşik adres (`nuxut:<id>[/dark|light]`) çözülmez: tohumu shell'in `nuxut-wallpapers`
    // crate'i verir, `--color` ile gelir; adres yalnızca `[meta] image`'a yazılır. Görsel dosyası
    // ile `--color` birlikte verilemez (hangisinin kaynak olduğu belirsiz kalırdı).
    let address = a.image.as_deref().is_some_and(nuxut_palette::export::is_builtin_address);
    match (&a.image, a.color.is_some()) {
        (Some(_), false) if address => return Err("yerleşik adres için --color (yerleşikin tohumu) gerekli".into()),
        (Some(_), true) if !address => return Err("ya bir görsel ya da --color verin".into()),
        (None, false) => return Err("ya bir görsel ya da --color verin".into()),
        _ => {}
    }
    Ok(a)
}

/// Görseli duvar kağıdı profilli yalıtılmış işçide çözer (işçi yoksa ADR-0007 K2-b: temel raster
/// süreç içinde, ilk kare; JPEG XL/AVIF/SVG hata).
fn source_from_file(p: &std::path::Path) -> Result<Color, nuxut_palette::PaletteError> {
    let sandbox = nuxut_image_sandbox::Sandbox::new(nuxut_image_sandbox::SandboxConfig::wallpaper());
    nuxut_palette::source_color_from_file(p, &sandbox, nuxut_image::file_key("wallpaper", p))
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("hata: {e}\n");
            }
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    // Dosya yolu mutlak yapılır; yerleşik adres olduğu gibi kalır (yol değil).
    let image = args.image.as_ref().map(|p| {
        if nuxut_palette::export::is_builtin_address(p) { p.clone() } else { std::path::absolute(p).unwrap_or_else(|_| p.clone()) }
    });
    let source = match (&image, args.color) {
        (_, Some(c)) => c,
        (Some(p), None) => match source_from_file(p) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("hata: {}: {e}", p.display());
                return ExitCode::FAILURE;
            }
        },
        (None, None) => unreachable!("parse en az birini garanti eder"),
    };
    let level = if args.high_contrast { ContrastMode::High } else { ContrastMode::Standard };
    let palette = Palette::generate_with(source, args.variant, args.mode, args.contrast, level);
    let toml = export::palette_toml(&palette, image.as_deref());
    if args.dry_run {
        print!("{toml}");
        return ExitCode::SUCCESS;
    }
    let Some(state) = args.state_dir.or_else(export::state_dir) else {
        eprintln!("hata: durum dizini bulunamadı (XDG_STATE_HOME/HOME yok)");
        return ExitCode::FAILURE;
    };
    let mut failed = false;
    let path = state.join(nuxut_theme::PALETTE_FILE);
    if let Err(e) = export::generated_palette(&palette, image.as_deref()).save(&path) {
        eprintln!("hata: {}: {e}", path.display());
        return ExitCode::FAILURE;
    }
    println!("kaynak {} → {}", source.to_hex(), path.display());
    println!("  (tema bu paleti framework.toml [colors] source = \"wallpaper\" iken kullanır)");
    if args.export {
        let user = args.templates.or_else(export::user_template_dir);
        for out in export::export_templates(&palette, image.as_deref(), &state.join("exports"), user.as_deref()) {
            match out.result {
                Ok(p) => println!("  {} → {}", out.name, p.display()),
                Err(e) => {
                    failed = true;
                    eprintln!("  {}: hata: {e}", out.name);
                }
            }
        }
    }
    if failed { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Args, String> {
        parse(std::iter::once("generate").chain(args.iter().copied()).map(String::from))
    }

    /// Yerleşik adres `--color` ister ve yol olarak kalır; dosya ile `--color` birlikte olmaz.
    #[test]
    fn builtin_address_needs_color_and_stays_verbatim() {
        assert!(p(&["nuxut:pair-flight"]).is_err());
        let a = p(&["nuxut:pair-flight/dark", "--color", "#6079a1"]).unwrap();
        assert_eq!(a.image.as_deref(), Some(std::path::Path::new("nuxut:pair-flight/dark")));
        assert!(p(&["/tmp/d.png", "--color", "#6079a1"]).is_err());
        assert!(p(&["/tmp/d.png"]).is_ok());
        assert!(p(&["--color", "#6079a1"]).is_ok());
        assert!(p(&[]).is_err());
    }
}
