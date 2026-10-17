//! Uygulama kimliğinden (`app_id`, bildirimin `desktop-entry` ipucu, SNI `Id`) ikon adına:
//! `.desktop` dosyasının `Icon=` alanı.

use std::path::PathBuf;

use super::index::parse_ini;

/// `app_id` için `<data_dirs>/applications/<app_id>.desktop`'taki `Icon=` değeri. Tam ad
/// yoksa küçük harfli ad denenir. Bulunamazsa `None` (çağıran `app_id`'yi ikon adı olarak
/// deneyebilir).
pub fn desktop_icon_name(app_id: &str) -> Option<String> {
    let id = app_id.trim().trim_end_matches(".desktop");
    if id.is_empty() || id.len() > 255 || id.contains('/') || id.starts_with('.') {
        return None;
    }
    let lower = id.to_lowercase();
    for dir in application_dirs() {
        for name in [id, lower.as_str()] {
            let Ok(text) = std::fs::read_to_string(dir.join(format!("{name}.desktop"))) else { continue };
            if let Some(icon) = parse_ini(&text).get("Desktop Entry").and_then(|s| s.get("Icon"))
                && !icon.is_empty()
            {
                return Some(icon.clone());
            }
        }
    }
    None
}

fn application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(d) = dirs::data_dir() {
        dirs.push(d.join("applications"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty());
    for d in data_dirs.as_deref().unwrap_or("/usr/local/share:/usr/share").split(':').filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(d).join("applications"));
    }
    dirs
}
