//! Çıktılar: `palette.toml` (tema katmanı) ve şablon dışa aktarımı. Tüm yazmalar atomiktir
//! (aynı dizinde geçici dosya + `rename`) ve yalnızca verilen hedef dizine yapılır.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use nuxut_theme::{GeneratedPalette, PaletteMeta, PalettePatch};

use crate::template::{self, MAX_TEMPLATE_BYTES};
use crate::{Mode, Palette, PaletteError};

/// Yerleşik şablonlar: (çıktı dosya adı, içerik).
pub const BUILTIN_TEMPLATES: [(&str, &str); 5] = [
    ("qt6ct-colors.conf", include_str!("../templates/qt6ct-colors.conf")),
    ("kdeglobals-colors", include_str!("../templates/kdeglobals-colors")),
    ("kitty.conf", include_str!("../templates/kitty.conf")),
    ("foot.ini", include_str!("../templates/foot.ini")),
    ("hyprland.conf", include_str!("../templates/hyprland.conf")),
];

/// Paletten çıktı metni üreten saf fonksiyon.
pub type Generator = fn(&Palette) -> String;

/// Şablon yerine saf üreticiyle yazılan yerleşik çıktılar ([`crate::look`]); aynı adlı
/// kullanıcı şablonu bunların da yerine geçer.
pub const BUILTIN_GENERATED: [(&str, Generator); 2] = [
    ("gtk3.css", |p| crate::look::gtk3_css(p.scheme(p.mode), p.mode)),
    ("gtk4.css", |p| crate::look::gtk4_css(p.scheme(crate::Mode::Dark), p.scheme(crate::Mode::Light), p.mode)),
];

/// Kullanıcı şablon dizininde işlenecek en fazla dosya.
const MAX_USER_TEMPLATES: usize = 64;

/// `$XDG_STATE_HOME/nuxut` (yoksa `~/.local/state/nuxut`).
pub fn state_dir() -> Option<PathBuf> {
    dirs::state_dir().map(|d| d.join("nuxut"))
}

/// `~/.config/nuxut/templates`.
pub fn user_template_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("nuxut").join("templates"))
}

/// Yerleşik duvar kağıdı adresinin öneki (`nuxut:<id>[/dark|light]`; çözücü shell deposundaki
/// `nuxut-wallpapers` crate'i). Framework adresi ayrıştırmaz, yalnızca saydam taşır.
pub const BUILTIN_SCHEME: &str = "nuxut:";

/// `image` bir yerleşik adres mi (dosya yolu değil). Adres yol olarak açılmaz,
/// kanonikleştirilmez; `[meta] image` ve şablonun `{{image}}`'ı onu olduğu gibi alır,
/// `{{image_path}}` boş kalır.
pub fn is_builtin_address(image: &Path) -> bool {
    image.to_str().is_some_and(|s| s.starts_with(BUILTIN_SCHEME))
}

/// `[meta] image` / `{{image}}` değeri: adres olduğu gibi, yol `display()` ile.
pub(crate) fn image_value(image: &Path) -> String {
    match image.to_str() {
        Some(s) if is_builtin_address(image) => s.to_owned(),
        _ => image.display().to_string(),
    }
}

/// Tema katmanının okuduğu biçim (`nuxut_theme::GeneratedPalette`): `[meta]`, `[colors]` (framework.toml
/// ile aynı 11 rol, seçili kip; `[dark]`/`[light]` iki kip ayrı) ve `[material]` (seçili kipin tüm rolleri + `term0`…`term15`).
/// `image`: dosya yolu ya da yerleşik adres ([`is_builtin_address`]); `[meta] image`'a olduğu
/// gibi yazılır.
pub fn generated_palette(palette: &Palette, image: Option<&Path>) -> GeneratedPalette {
    let patch = |mode: Mode| {
        let colors = palette.theme_colors_for(mode);
        let get = |k: &str| colors.iter().find(|(n, _)| *n == k).map(|(_, c)| *c);
        PalettePatch {
            surface: get("surface"),
            surface_container: get("surface_container"),
            surface_bright: get("surface_bright"),
            on_surface: get("on_surface"),
            on_surface_variant: get("on_surface_variant"),
            primary: get("primary"),
            on_primary: get("on_primary"),
            error: get("error"),
            on_error: get("on_error"),
            outline: get("outline"),
            shadow: get("shadow"),
        }
    };
    GeneratedPalette {
        meta: PaletteMeta {
            source: Some(palette.source.to_hex()),
            scheme: Some(palette.variant.name().to_owned()),
            mode: Some(palette.mode.name().to_owned()),
            contrast: Some(palette.contrast as f32),
            image: image.map(image_value),
        },
        colors: patch(palette.mode),
        dark: Some(patch(Mode::Dark)),
        light: Some(patch(Mode::Light)),
        material: palette.active().iter().map(|(k, c)| (k.to_owned(), c)).collect(),
    }
}

/// `palette.toml` metni (`--dry-run` ve testler için; dosyaya `GeneratedPalette::save` yazar).
pub fn palette_toml(palette: &Palette, image: Option<&Path>) -> String {
    toml::to_string(&generated_palette(palette, image)).unwrap_or_default()
}


/// `dir/name`'e atomik yazar. `name` düz bir dosya adı olmalı (ayırıcı, `..`, gizli dosya
/// yok); dizin yoksa yaratılır.
pub fn write_atomic(dir: &Path, name: &str, contents: &[u8]) -> Result<PathBuf, PaletteError> {
    if !is_plain_name(name) {
        return Err(PaletteError::BadName(name.to_owned()));
    }
    let io = |path: &Path| {
        let path = path.to_owned();
        move |source| PaletteError::Io { path, source }
    };
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    let target = dir.join(name);
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(&target)(e));
    }
    Ok(target)
}

fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// Tek bir şablonun sonucu.
pub struct Exported {
    pub name: String,
    pub result: Result<PathBuf, PaletteError>,
}

/// Yazılan yol (ev dizini) gösterilmez.
impl std::fmt::Debug for Exported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let result: Result<&str, &PaletteError> = self.result.as_ref().map(|_| "<yol gizli>");
        f.debug_struct("Exported").field("name", &self.name).field("result", &result).finish()
    }
}

/// Yerleşik şablonları ve `user_dir`'deki kullanıcı şablonlarını işleyip `out_dir`'e yazar
/// (kullanıcı şablonu aynı adlı yerleşiğin yerine geçer). Bir şablonun hatası diğerlerini
/// durdurmaz.
pub fn export_templates(palette: &Palette, image: Option<&Path>, out_dir: &Path, user_dir: Option<&Path>) -> Vec<Exported> {
    /// Bir çıktının kaynağı: şablon metni ya da saf üretici.
    enum Job {
        Template(Result<String, PaletteError>),
        Generated(Generator),
    }
    let mut jobs: Vec<(String, Job)> = BUILTIN_GENERATED
        .iter()
        .map(|(n, f)| ((*n).to_owned(), Job::Generated(*f)))
        .chain(BUILTIN_TEMPLATES.iter().map(|(n, t)| ((*n).to_owned(), Job::Template(Ok((*t).to_owned())))))
        .collect();
    if let Some(dir) = user_dir {
        for (name, text) in read_user_templates(dir) {
            match jobs.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = Job::Template(text),
                None => jobs.push((name, Job::Template(text))),
            }
        }
    }
    jobs.into_iter()
        .map(|(name, job)| {
            let result = match job {
                Job::Generated(f) => write_atomic(out_dir, &name, f(palette).as_bytes()),
                Job::Template(text) => text.and_then(|t| {
                    let out =
                        template::render(&t, palette, image).map_err(|e| PaletteError::Template(name.clone(), e))?;
                    write_atomic(out_dir, &name, out.as_bytes())
                }),
            };
            Exported { name, result }
        })
        .collect()
}

fn read_user_templates(dir: &Path) -> Vec<(String, Result<String, PaletteError>)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<_> = rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path())))
        .filter(|(n, _)| is_plain_name(n))
        .map(|(name, path)| {
            let text = match std::fs::metadata(&path) {
                Ok(m) if m.len() as usize > MAX_TEMPLATE_BYTES => {
                    Err(PaletteError::Template(name.clone(), template::TemplateError::TooLarge(m.len() as usize)))
                }
                _ => std::fs::read_to_string(&path).map_err(|source| PaletteError::Io { path: path.clone(), source }),
            };
            (name, text)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    if out.len() > MAX_USER_TEMPLATES {
        tracing::warn!("{} kullanıcı şablonu var; ilk {MAX_USER_TEMPLATES} işleniyor", out.len());
        out.truncate(MAX_USER_TEMPLATES);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_names() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../x", "a/b", ".hidden", "", "x y", "/etc/passwd"] {
            assert!(matches!(write_atomic(dir.path(), bad, b"1"), Err(PaletteError::BadName(_))), "{bad}");
        }
        let p = write_atomic(dir.path(), "ok.conf", b"1").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"1");
        write_atomic(dir.path(), "ok.conf", b"2").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"2");
        // Geçici dosya kalmaz.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
