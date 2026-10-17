//! Marka ikonlarını (Kara Sağan) hicolor düzeninde dışa aktarır:
//!
//! ```sh
//! cargo run -p nuxut-ui --example brand_icons -- <çıktı-dizini>
//! ```
//!
//! Üretilen (`<çıktı-dizini>` = hicolor kökü, ör. `~/.local/share/icons/hicolor`):
//! - `scalable/apps/nuxut.svg` — renkli, karolu uygulama ikonu (gömülü SVG'nin birebir kopyası);
//! - `symbolic/apps/nuxut-symbolic.svg` — tek renkli (`currentColor`; masaüstü yeniden boyar);
//! - `{48,64,128,256}x{…}/apps/nuxut.png` — renkli SVG'den, nuxut-image'in SVG yolu (resvg).
//!
//! Uygulama ikonu bir dış dosyadır ve sabit renklidir (renkler SVG'de, bu dosyada renk sabiti
//! yok); uygulama içinde marka her zaman tema rolleriyle `BrandMark` olarak çizilir.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use nuxut_image::{Fit, TargetSize, render_svg};
use nuxut_ui::{BRAND_SVG, BRAND_SYMBOLIC_SVG};

const ICON_NAME: &str = "nuxut";
const PNG_SIZES: [u32; 4] = [48, 64, 128, 256];

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn run(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let svg = root.join("scalable/apps").join(format!("{ICON_NAME}.svg"));
    write(&svg, BRAND_SVG)?;
    out.push(svg);
    let symbolic = root.join("symbolic/apps").join(format!("{ICON_NAME}-symbolic.svg"));
    write(&symbolic, BRAND_SYMBOLIC_SVG)?;
    out.push(symbolic);
    for size in PNG_SIZES {
        let img = render_svg(BRAND_SVG, TargetSize::new(size, size, Fit::Contain)).map_err(|e| format!("SVG {size}px: {e}"))?;
        if img.size() != (size, size) {
            return Err(format!("beklenmeyen raster boyutu {:?} ≠ {size}", img.size()));
        }
        let png = root.join(format!("{size}x{size}/apps")).join(format!("{ICON_NAME}.png"));
        if let Some(dir) = png.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        image::save_buffer(&png, &img.to_straight(), size, size, image::ColorType::Rgba8).map_err(|e| format!("{}: {e}", png.display()))?;
        out.push(png);
    }
    Ok(out)
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(root), None) = (args.next(), args.next()) else {
        eprintln!("kullanım: brand_icons <çıktı-dizini>   (hicolor kökü)");
        return ExitCode::from(2);
    };
    match run(Path::new(&root)) {
        Ok(files) => {
            for f in files {
                println!("{}", f.display());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("brand_icons: {e}");
            ExitCode::FAILURE
        }
    }
}
