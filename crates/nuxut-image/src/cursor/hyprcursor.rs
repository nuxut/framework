//! Hyprcursor temaları (salt okuma, önizleme): `manifest.hl`/`manifest.toml` + şekil başına
//! `<cursors_directory>/<şekil>.hlc` ZIP arşivi (`meta.hl`/`meta.toml` + PNG/SVG görseller).
//!
//! Biçim (hyprcursor `docs/MAKING_THEMES.md`): manifestte `cursors_directory`; metada
//! `hotspot_x`/`hotspot_y` (0…1, görsele göre), `define_override` (eşanlamlı adlar) ve
//! `define_size = <boyut>, <dosya>[, <gecikme ms>]` (hyprlang'de satır başına bir; toml'da tek
//! değer, `;` ayrılmış). Boyut 0 ya da `.svg` uzantısı ölçeklenebilir görseldir.
//!
//! **Güven (ADR-0007):** raster (PNG) her zaman tercih edilir; yalnızca PNG imzası taşıyan
//! veri çözülür (başka raster biçimi yok). Yalnızca SVG'si olan şekil, `.hlc` dosyası sistem
//! kökenliyse (ev dizini dışındaki bir arama dizininde, kök sahipli ve başkalarınca yazılamaz:
//! [`crate::icon::system_origin`], ikon temalarıyla aynı kural) `Trust::System` ile süreç
//! içinde çizilir; kullanıcı dizinindeki (`~/.icons`, `~/.local/share/icons`) SVG şekil
//! `Untrusted` kalır: [`crate::Loader`]'da yalıtılmış çözücü varsa (`Loader::with_sandbox`) SVG
//! baytları işçide tek kare olarak çözülür, yoksa `Unsupported("SVG")` döner (önizlemede "yok").
//! İşçi kullanılamıyorsa (`Unavailable`) SVG yine süreç içine düşmez (ADR-0007 K2-b). Raster
//! (PNG) kareler bugünkü gibi süreç içinde, `Untrusted` sınırlarla çözülür.
//!
//! **Sınırlar:** manifest/meta en çok [`MAX_META_BYTES`]; `.hlc` en çok
//! [`zip::MAX_ARCHIVE_BYTES`](super::zip::MAX_ARCHIVE_BYTES) belleğe okunur; en çok
//! [`MAX_SIZE_DEFS`] `define_size`, [`MAX_OVERRIDES`] eşanlamlı; eşanlamlı haritası için
//! dizinde en çok [`MAX_SHAPES`] arşive bakılır (yalnızca adla bulunamayan rolde, tema başına
//! bir kez). Görsel kenarı [`super::MAX_CURSOR_SIDE`], kare/piksel bütçesi Xcursor ile aynı.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::zip::{self, Archive};
use super::{CursorImage, MAX_CURSOR_FRAMES, MAX_CURSOR_PIXELS, MAX_CURSOR_SIDE, ZERO_DELAY};
use crate::{DecodeOptions, Fit, ImageError, Limits, MAX_FRAME_DELAY, TargetSize, Trust};

/// Manifest ve meta dosyasının en büyük boyutu.
pub const MAX_META_BYTES: usize = 64 << 10;
/// Bir şekildeki en fazla `define_size`.
pub const MAX_SIZE_DEFS: usize = 256;
/// Bir şekildeki en fazla `define_override`.
pub const MAX_OVERRIDES: usize = 64;
/// Eşanlamlı haritası kurulurken bakılan en fazla `.hlc`.
pub const MAX_SHAPES: usize = 1024;

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Ayrıştırılmış manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// `name` (görünen ad; yoksa `None`).
    pub name: Option<String>,
    /// Şekil arşivlerinin dizini (tema dizinine göre tek bileşen).
    pub cursors_directory: String,
}

/// `theme_dir/manifest.hl` ya da `manifest.toml` (bu sırayla). Geçersiz `cursors_directory`
/// (yol, `..`, gizli) ya da okunamayan dosya `None`.
pub fn read_manifest(theme_dir: &Path) -> Option<Manifest> {
    let text = ["manifest.hl", "manifest.toml"].iter().find_map(|f| read_text(&theme_dir.join(f)))?;
    let mut name = None;
    let mut dir = None;
    for (k, v) in key_values(&text) {
        match k {
            "name" if name.is_none() => name = Some(clean(v)).filter(|n| !n.is_empty()),
            "cursors_directory" if dir.is_none() => dir = Some(v.to_owned()),
            _ => {}
        }
    }
    let dir = dir.filter(|d| crate::icon::valid_name(d))?;
    Some(Manifest { name, cursors_directory: dir })
}

/// Bir şekil boyutu tanımı.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SizeDef {
    /// Piksel boyutu; 0 = ölçeklenebilir.
    size: u32,
    file: String,
    delay_ms: u32,
}

impl SizeDef {
    fn is_svg(&self) -> bool {
        self.size == 0 || self.file.to_ascii_lowercase().ends_with(".svg")
    }
}

/// Ayrıştırılmış şekil metası.
#[derive(Debug, Clone, PartialEq, Default)]
struct Meta {
    hotspot: (f32, f32),
    overrides: Vec<String>,
    sizes: Vec<SizeDef>,
}

fn parse_meta(text: &str) -> Meta {
    let mut m = Meta::default();
    let frac = |v: &str| v.parse::<f32>().ok().filter(|f| f.is_finite()).map_or(0.0, |f| f.clamp(0.0, 1.0));
    for (k, v) in key_values(text) {
        match k {
            "hotspot_x" => m.hotspot.0 = frac(v),
            "hotspot_y" => m.hotspot.1 = frac(v),
            "define_override" => {
                for o in v.split(';').map(str::trim).filter(|o| crate::icon::valid_name(o)) {
                    if m.overrides.len() < MAX_OVERRIDES {
                        m.overrides.push(o.to_owned());
                    }
                }
            }
            "define_size" => {
                for def in v.split(';') {
                    let mut parts = def.split(',').map(str::trim);
                    let (Some(size), Some(file)) = (parts.next(), parts.next()) else { continue };
                    let Ok(size) = size.parse::<u32>() else { continue };
                    let delay_ms = parts.next().and_then(|d| d.parse::<u32>().ok()).unwrap_or(0);
                    if size <= MAX_CURSOR_SIDE && !file.is_empty() && m.sizes.len() < MAX_SIZE_DEFS {
                        m.sizes.push(SizeDef { size, file: file.to_owned(), delay_ms });
                    }
                }
            }
            _ => {}
        }
    }
    m
}

/// `anahtar = değer` satırları (hyprlang ve düz toml): boş, `#` yorumu ve `[bölüm]` satırları
/// atlanır; değerden satır sonu yorumu ve çevreleyen tırnaklar atılır.
fn key_values(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            return None;
        }
        let (k, v) = line.split_once('=')?;
        let v = v.trim();
        let v = match v.strip_prefix('"') {
            Some(rest) => rest.split('"').next().unwrap_or(""),
            None => v.split(" #").next().unwrap_or(v).trim(),
        };
        Some((k.trim(), v))
    })
}

fn clean(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(crate::icon::installed::MAX_NAME_CHARS).collect::<String>().trim().to_owned()
}

/// Küçük metin dosyası (en çok [`MAX_META_BYTES`], `O_NONBLOCK`, normal dosya).
fn read_text(path: &Path) -> Option<String> {
    let file = crate::decode::open_regular(path).ok()?;
    let mut buf = Vec::new();
    file.take(MAX_META_BYTES as u64 + 1).read_to_end(&mut buf).ok()?;
    (buf.len() <= MAX_META_BYTES).then(|| String::from_utf8_lossy(&buf).into_owned())
}

/// Bir hyprcursor teması dizini (tema zincirindeki bir arama dizini kaydı).
#[derive(Debug)]
pub(crate) struct HyprTheme {
    shapes: PathBuf,
    /// Tüm arama dizinleri (güven kökeni denetimi için).
    search_dirs: Vec<PathBuf>,
    /// Eşanlamlı → şekil dosyası; ilk gerektiğinde kurulur.
    overrides: std::sync::OnceLock<HashMap<String, PathBuf>>,
}

impl HyprTheme {
    pub(crate) fn open(theme_dir: &Path, search_dirs: &[PathBuf]) -> Option<Self> {
        let manifest = read_manifest(theme_dir)?;
        let shapes = theme_dir.join(&manifest.cursors_directory);
        std::fs::metadata(&shapes).is_ok_and(|m| m.is_dir()).then(|| Self {
            shapes,
            search_dirs: search_dirs.to_vec(),
            overrides: std::sync::OnceLock::new(),
        })
    }

    /// Adlardan ilkinin şekil dosyası: önce `<ad>.hlc`, sonra arşivlerin `define_override`'ı.
    pub(crate) fn find(&self, names: &[&str]) -> Option<PathBuf> {
        let is_file = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file());
        let valid = || names.iter().copied().filter(|n| crate::icon::valid_name(n));
        if let Some(p) = valid().map(|n| self.shapes.join(format!("{n}.hlc"))).find(|p| is_file(p)) {
            return Some(p);
        }
        let map = self.overrides.get_or_init(|| self.override_map());
        valid().find_map(|n| map.get(n).cloned())
    }

    fn override_map(&self) -> HashMap<String, PathBuf> {
        let mut map = HashMap::new();
        let Ok(rd) = std::fs::read_dir(&self.shapes) else { return map };
        let mut files: Vec<PathBuf> = rd
            .take(MAX_SHAPES)
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "hlc"))
            .collect();
        files.sort();
        for path in files {
            let Ok((archive, _)) = open_archive(&path, &self.search_dirs) else { continue };
            let Some(meta) = read_meta(&archive) else { continue };
            for o in meta.overrides {
                map.entry(o).or_insert_with(|| path.clone());
            }
        }
        map
    }

    /// Şeklin boyutları: raster `define_size`'lar (artan, tekil) ve ölçeklenebilir görsel var mı.
    pub(crate) fn sizes(&self, names: &[&str]) -> Option<(Vec<u32>, bool)> {
        let path = self.find(names)?;
        let (archive, _) = open_archive(&path, &self.search_dirs).ok()?;
        let meta = read_meta(&archive)?;
        let mut fixed: Vec<u32> = meta.sizes.iter().filter(|s| !s.is_svg()).map(|s| s.size).collect();
        fixed.sort_unstable();
        fixed.dedup();
        Some((fixed, meta.sizes.iter().any(SizeDef::is_svg)))
    }

    pub(crate) fn load(&self, names: &[&str], size: u32, animate: bool, isolated: Option<Isolated<'_>>) -> Result<Vec<CursorImage>, ImageError> {
        let path = self.find(names).ok_or_else(|| ImageError::NotFound(String::new()))?;
        let (archive, trust) = open_archive(&path, &self.search_dirs)?;
        load_shape(&archive, trust, size, animate, isolated)
    }
}

/// `.hlc` dosyasını sınırlı okur; güveni aynı fd'den belirlenir.
fn open_archive(path: &Path, search_dirs: &[PathBuf]) -> Result<(Archive, Trust), ImageError> {
    let file = crate::decode::open_regular(path)?;
    let io = |source| ImageError::Io { path: PathBuf::new(), source };
    let len = file.metadata().map_err(io)?.len();
    if len > zip::MAX_ARCHIVE_BYTES {
        return Err(ImageError::FileTooLarge { size: len, limit: zip::MAX_ARCHIVE_BYTES });
    }
    let trust = if crate::icon::system_origin(&file, search_dirs, dirs::home_dir().as_deref()) {
        Trust::System
    } else {
        Trust::Untrusted
    };
    let mut data = Vec::new();
    file.take(zip::MAX_ARCHIVE_BYTES + 1).read_to_end(&mut data).map_err(io)?;
    Ok((Archive::parse(data)?, trust))
}

fn read_meta(archive: &Archive) -> Option<Meta> {
    ["meta.hl", "meta.toml"]
        .iter()
        .find(|n| archive.contains(n))
        .and_then(|n| archive.read(n, MAX_META_BYTES).ok())
        .map(|b| parse_meta(&String::from_utf8_lossy(&b)))
}

/// Güvenilmeyen baytları yalıtılmış çözen işlev ([`crate::Loader`]'ın işçisi; ADR-0007).
pub(crate) type Isolated<'a> = &'a dyn Fn(std::sync::Arc<[u8]>, &DecodeOptions) -> Result<crate::Image, ImageError>;

/// Arşivden `size`'a en yakın raster boyutun karelerini çözer; raster yoksa SVG'yi: sistem
/// kökenliyse süreç içinde, değilse yalnızca `isolated` verilmişse onda (yoksa
/// `Unsupported("SVG")`).
pub(crate) fn load_shape(
    archive: &Archive,
    trust: Trust,
    size: u32,
    animate: bool,
    isolated: Option<Isolated<'_>>,
) -> Result<Vec<CursorImage>, ImageError> {
    let meta = read_meta(archive).ok_or_else(|| ImageError::InvalidData("hyprcursor: meta yok".into()))?;
    let want = size.max(1);
    let best = meta
        .sizes
        .iter()
        .filter(|s| !s.is_svg())
        .map(|s| s.size)
        .reduce(|best, n| if n.abs_diff(want) < best.abs_diff(want) { n } else { best });
    let limits = Limits { max_width: MAX_CURSOR_SIDE, max_height: MAX_CURSOR_SIDE, ..Limits::ICON }.with_trust(trust);
    let mut out: Vec<CursorImage> = Vec::new();
    let mut pixels: u64 = 0;
    let Some(best) = best else {
        // Yalnızca ölçeklenebilir görsel: güvenilmeyen kaynakta asla süreç içinde çözülmez;
        // işçi varsa onda (tek kare), yoksa "yok".
        let isolated = match (trust, isolated) {
            (Trust::System, _) => None,
            (_, Some(f)) => Some(f),
            (_, None) => return Err(ImageError::Unsupported("SVG")),
        };
        let def = meta.sizes.iter().find(|s| s.is_svg()).ok_or(ImageError::Empty)?;
        let bytes = archive.read(&def.file, Limits::ICON.max_file_bytes as usize)?;
        let side = want.min(MAX_CURSOR_SIDE);
        let opts = DecodeOptions { limits, target: Some(TargetSize::new(side, side, Fit::Contain)), animate: false };
        let image = match isolated {
            Some(f) => f(bytes.into(), &opts)?,
            None => crate::decode_bytes(&bytes, &opts)?.into_first_frame(),
        };
        return Ok(vec![frame(image, side, meta.hotspot, 0)]);
    };
    // Güvenilmeyen arşiv (K1-b, K5): yalnızca ilk kare; işçi varsa raster de onda çözülür,
    // yoksa süreç içinde yalnızca PNG (K2-b; `decode_bytes` güven kapısı).
    let untrusted = trust == Trust::Untrusted;
    let limit = if animate && !untrusted { MAX_CURSOR_FRAMES } else { 1 };
    let opts = DecodeOptions { limits, target: None, animate: false };
    for def in meta.sizes.iter().filter(|s| !s.is_svg() && s.size == best).take(limit) {
        let decoded = archive.read(&def.file, Limits::ICON.max_file_bytes as usize).and_then(|bytes| {
            if !bytes.starts_with(&PNG_MAGIC) {
                return Err(ImageError::UnknownFormat);
            }
            match isolated {
                Some(f) if untrusted => f(bytes.into(), &opts),
                _ => crate::decode_bytes(&bytes, &opts).map(crate::Decoded::into_first_frame),
            }
        });
        match decoded {
            Ok(image) => {
                let (w, h) = image.size();
                let total = pixels + u64::from(w) * u64::from(h);
                if total > MAX_CURSOR_PIXELS || out.first().is_some_and(|f| f.image.size() != (w, h)) {
                    break;
                }
                pixels = total;
                out.push(frame(image, best, meta.hotspot, def.delay_ms));
            }
            Err(e) if out.is_empty() => return Err(e),
            Err(e) => {
                tracing::debug!("hyprcursor animasyonu {} karede kesildi: {e}", out.len());
                break;
            }
        }
    }
    if out.is_empty() { Err(ImageError::Empty) } else { Ok(out) }
}

fn frame(image: crate::Image, nominal: u32, hotspot: (f32, f32), delay_ms: u32) -> CursorImage {
    let (w, h) = image.size();
    let hot = |f: f32, side: u32| ((f * side as f32).round() as u32).min(side.saturating_sub(1));
    let delay = match delay_ms {
        0 => ZERO_DELAY,
        ms => Duration::from_millis(u64::from(ms)).min(MAX_FRAME_DELAY),
    };
    CursorImage { hotspot: (hot(hotspot.0, w), hot(hotspot.1, h)), image, nominal, delay }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::cursor::zip::build_zip;

    /// `side`×`side` tek renk PNG (opak).
    pub(crate) fn png(side: u32, rgb: [u8; 3]) -> Vec<u8> {
        let mut buf = Vec::new();
        let img = image::RgbaImage::from_pixel(side, side, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        buf
    }

    pub(crate) fn hlc(meta: &str, files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut all: Vec<(&str, &[u8])> = vec![("meta.hl", meta.as_bytes())];
        all.extend(files.iter().map(|(n, b)| (*n, b.as_slice())));
        build_zip(&all, true)
    }

    #[test]
    fn meta_parses_hyprlang_and_toml() {
        let hl = "# yorum\nresize_algorithm = bilinear\nhotspot_x = 0.25\nhotspot_y = 2.0\n\
                  define_override = arrow;left_ptr\ndefine_override = ../x\n\
                  define_size = 32, a32.png\ndefine_size = 64, a64.png, 120\ndefine_size = 0, a.svg\n\
                  define_size = bozuk\ndefine_size = 99999, dev.png\n";
        let m = parse_meta(hl);
        assert_eq!(m.hotspot, (0.25, 1.0));
        assert_eq!(m.overrides, ["arrow", "left_ptr"]);
        assert_eq!(m.sizes.len(), 3);
        assert_eq!(m.sizes[1], SizeDef { size: 64, file: "a64.png".into(), delay_ms: 120 });
        assert!(m.sizes[2].is_svg());
        let toml = "[General]\nhotspot_x = 0.5 # orta\ndefine_override = \"default;arrow\"\n\
                    define_size = \"24,a.png;48,b.png,50\"\n";
        let t = parse_meta(toml);
        assert_eq!((t.hotspot.0, t.overrides.len(), t.sizes.len()), (0.5, 2, 2));
        assert_eq!(t.sizes[1].delay_ms, 50);
        // Sınırlar: çok sayıda tanım kırpılır.
        let many: String = (0..1000).map(|i| format!("define_size = 24, f{i}.png\ndefine_override = o{i}\n")).collect();
        let m = parse_meta(&many);
        assert_eq!((m.sizes.len(), m.overrides.len()), (MAX_SIZE_DEFS, MAX_OVERRIDES));
    }

    #[test]
    fn manifest_rejects_escaping_directory() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path();
        std::fs::write(dir.join("manifest.hl"), "name = Bibata Hypr\ncursors_directory = hyprcursors\n").unwrap();
        assert_eq!(
            read_manifest(dir).unwrap(),
            Manifest { name: Some("Bibata Hypr".into()), cursors_directory: "hyprcursors".into() }
        );
        for bad in ["../x", "/etc", ".gizli", ""] {
            std::fs::write(dir.join("manifest.hl"), format!("cursors_directory = {bad}\n")).unwrap();
            assert!(read_manifest(dir).is_none(), "{bad}");
        }
        std::fs::remove_file(dir.join("manifest.hl")).unwrap();
        std::fs::write(dir.join("manifest.toml"), "[General]\ncursors_directory = \"c\"\n").unwrap();
        assert_eq!(read_manifest(dir).unwrap().cursors_directory, "c");
        // Dev manifest okunmaz.
        std::fs::write(dir.join("manifest.toml"), "#".repeat(MAX_META_BYTES + 1)).unwrap();
        assert!(read_manifest(dir).is_none());
    }

    #[test]
    fn picks_nearest_raster_frames_and_hotspot() {
        let meta = "hotspot_x = 0.5\nhotspot_y = 0.25\ndefine_size = 0, a.svg\n\
                    define_size = 24, s1.png, 100\ndefine_size = 24, s2.png, 0\ndefine_size = 48, b.png\n";
        let a = Archive::parse(hlc(
            meta,
            &[("s1.png", png(24, [255, 0, 0])), ("s2.png", png(24, [0, 255, 0])), ("b.png", png(48, [0, 0, 255])), ("a.svg", b"<svg/>".to_vec())],
        ))
        .unwrap();
        let f = load_shape(&a, Trust::System, 30, true, None).unwrap();
        assert_eq!(f.len(), 2);
        assert_eq!((f[0].nominal, f[0].image.size(), f[0].hotspot), (24, (24, 24), (12, 6)));
        assert_eq!((f[0].delay, f[1].delay), (Duration::from_millis(100), ZERO_DELAY));
        assert_eq!(&f[1].image.pixels()[..4], &[0, 255, 0, 255]);
        assert_eq!(load_shape(&a, Trust::Untrusted, 40, false, None).unwrap()[0].nominal, 48);
    }

    /// Bulgu 4c: güvenilmeyen arşivin raster kareleri — işçi varsa onda, her durumda tek kare.
    #[test]
    fn untrusted_raster_frames_go_to_the_sandbox_and_are_single_frame() {
        let meta = "define_size = 24, s1.png, 100\ndefine_size = 24, s2.png, 100\n";
        let a = Archive::parse(hlc(meta, &[("s1.png", png(24, [255, 0, 0])), ("s2.png", png(24, [0, 255, 0]))])).unwrap();
        // İşçisiz: süreç içinde PNG, yalnızca ilk kare.
        let f = load_shape(&a, Trust::Untrusted, 24, true, None).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(&f[0].image.pixels()[..4], &[255, 0, 0, 255]);
        // İşçiyle: raster de işçiye gider, tek çağrı (tek kare).
        let calls = std::cell::Cell::new(0);
        let fake = |bytes: std::sync::Arc<[u8]>, opts: &DecodeOptions| {
            calls.set(calls.get() + 1);
            assert!(bytes.starts_with(&PNG_MAGIC));
            assert_eq!(opts.trust(), Trust::Untrusted);
            crate::Image::from_premultiplied(24, 24, vec![0; 24 * 24 * 4])
        };
        let f = load_shape(&a, Trust::Untrusted, 24, true, Some(&fake)).unwrap();
        assert_eq!((f.len(), calls.get()), (1, 1));
        // Sistem kökenli arşiv işçiye gitmez, animasyon korunur.
        assert_eq!(load_shape(&a, Trust::System, 24, true, Some(&fake)).unwrap().len(), 2);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn svg_only_shape_needs_system_trust() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;
        let a = Archive::parse(hlc("define_size = 0, a.svg\n", &[("a.svg", svg.to_vec())])).unwrap();
        assert!(matches!(load_shape(&a, Trust::Untrusted, 32, false, None), Err(ImageError::Unsupported(_))));
        let f = load_shape(&a, Trust::System, 32, false, None).unwrap();
        assert_eq!((f[0].nominal, f[0].image.size()), (32, (32, 32)));
    }

    #[test]
    fn untrusted_svg_shape_goes_only_to_the_isolated_decoder() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect width="10" height="10" fill="red"/></svg>"#;
        let a = Archive::parse(hlc("hotspot_x = 0.5\ndefine_size = 0, a.svg\n", &[("a.svg", svg.to_vec())])).unwrap();
        let calls = std::cell::Cell::new(0);
        let fake = |bytes: std::sync::Arc<[u8]>, opts: &DecodeOptions| {
            calls.set(calls.get() + 1);
            assert_eq!(opts.trust(), Trust::Untrusted, "işçiye güvenilmeyen sınırlarla gider");
            assert!(bytes.starts_with(b"<svg"));
            // İşçinin yaptığı: güven kapısız çözüm (burada süreç içinde, yalnızca test).
            crate::decode_bytes(&bytes, &opts.with_trust(Trust::System)).map(crate::Decoded::into_first_frame)
        };
        let f = load_shape(&a, Trust::Untrusted, 24, true, Some(&fake)).unwrap();
        assert_eq!((f.len(), f[0].nominal, f[0].image.size(), f[0].hotspot.0), (1, 24, (24, 24), 12));
        assert_eq!(calls.get(), 1);
        // Sistem kökenli şekil işçiye gitmez.
        load_shape(&a, Trust::System, 24, false, Some(&fake)).unwrap();
        assert_eq!(calls.get(), 1);
        // İşçinin hatası (ör. Unavailable) aynen döner; süreç içine düşülmez.
        let down = |_: std::sync::Arc<[u8]>, _: &DecodeOptions| Err(ImageError::Sandbox(crate::SandboxFailure::Unavailable));
        assert!(matches!(load_shape(&a, Trust::Untrusted, 24, false, Some(&down)), Err(ImageError::Sandbox(crate::SandboxFailure::Unavailable))));
    }

    #[test]
    fn hostile_shapes_are_errors() {
        // PNG olmayan raster, eksik dosya, meta yok, dev görsel, boyut uyuşmayan kare.
        let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
        let cases: Vec<Vec<u8>> = vec![
            hlc("define_size = 24, a.png\n", &[("a.png", gif)]),
            hlc("define_size = 24, yok.png\n", &[]),
            build_zip(&[("a.png", &png(24, [0, 0, 0]))], false),
            hlc("define_size = 24, a.png\n", &[("a.png", png(MAX_CURSOR_SIDE + 1, [0, 0, 0]))]),
            hlc("", &[]),
        ];
        for (i, c) in cases.into_iter().enumerate() {
            let a = Archive::parse(c).unwrap();
            assert!(load_shape(&a, Trust::Untrusted, 24, true, None).is_err(), "durum {i}");
        }
        let a = Archive::parse(hlc(
            "define_size = 24, a.png\ndefine_size = 24, b.png\n",
            &[("a.png", png(24, [0, 0, 0])), ("b.png", png(20, [0, 0, 0]))],
        ))
        .unwrap();
        assert_eq!(load_shape(&a, Trust::System, 24, true, None).unwrap().len(), 1, "farklı boyutlu kare animasyonu keser");
    }

    #[test]
    fn theme_finds_by_name_then_override() {
        let root = tempfile::tempdir().unwrap();
        let t = root.path().join("H");
        std::fs::create_dir_all(t.join("hc")).unwrap();
        std::fs::write(t.join("manifest.hl"), "cursors_directory = hc\n").unwrap();
        let shape = |o: &str| hlc(&format!("define_override = {o}\ndefine_size = 24, a.png\ndefine_size = 32, b.png\n"), &[("a.png", png(24, [1, 2, 3])), ("b.png", png(32, [1, 2, 3]))]);
        std::fs::write(t.join("hc/left_ptr.hlc"), shape("default;arrow")).unwrap();
        std::fs::write(t.join("hc/hand2.hlc"), shape("pointer")).unwrap();
        std::fs::write(t.join("hc/bozuk.hlc"), b"PK bozuk").unwrap();
        let h = HyprTheme::open(&t, &[root.path().to_owned()]).unwrap();
        assert_eq!(h.find(&["default", "left_ptr"]).unwrap(), t.join("hc/left_ptr.hlc"));
        assert_eq!(h.find(&["pointer"]).unwrap(), t.join("hc/hand2.hlc"), "eşanlamlıdan");
        assert!(h.find(&["text", "../hc/left_ptr"]).is_none());
        assert_eq!(h.sizes(&["pointer"]).unwrap(), (vec![24, 32], false));
        assert_eq!(h.load(&["arrow"], 32, false, None).unwrap()[0].nominal, 32);
    }
}
