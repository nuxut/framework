//! Kullanıcının ikon teması adı: GTK ve Qt ayar dosyalarından.

use std::path::Path;

use super::index::parse_ini;

/// Sıra: `NUXUT_ICON_THEME` → GTK 4 → GTK 3 (`settings.ini`, `gtk-icon-theme-name`) →
/// qt6ct/qt5ct (`icon_theme`) → `kdeglobals` (`[Icons] Theme`) → `~/.gtkrc-2.0`.
/// Hiçbiri yoksa `None` (çağıran `hicolor` kullanır). Yalnızca dosya okur; gsettings/dconf
/// sorgulanmaz.
pub fn detect_theme_name() -> Option<String> {
    if let Ok(v) = std::env::var("NUXUT_ICON_THEME")
        && valid(&v)
    {
        return Some(v);
    }
    let config = dirs::config_dir()?;
    detect_in(&config, dirs::home_dir().as_deref())
}

pub(crate) fn detect_in(config: &Path, home: Option<&Path>) -> Option<String> {
    let ini = |rel: &str, section: &str, key: &str| -> Option<String> {
        let text = std::fs::read_to_string(config.join(rel)).ok()?;
        let v = parse_ini(&text).get(section)?.get(key)?.trim_matches('"').to_owned();
        valid(&v).then_some(v)
    };
    ini("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name")
        .or_else(|| ini("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"))
        .or_else(|| ini("qt6ct/qt6ct.conf", "Appearance", "icon_theme"))
        .or_else(|| ini("qt5ct/qt5ct.conf", "Appearance", "icon_theme"))
        .or_else(|| ini("kdeglobals", "Icons", "Theme"))
        .or_else(|| {
            let text = std::fs::read_to_string(home?.join(".gtkrc-2.0")).ok()?;
            text.lines().find_map(|l| {
                let (k, v) = l.split_once('=')?;
                let v = v.trim().trim_matches('"').to_owned();
                (k.trim() == "gtk-icon-theme-name" && valid(&v)).then_some(v)
            })
        })
}

fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && !name.contains('/') && !name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gtk_then_qt() {
        let dir = tempfile::tempdir().unwrap();
        let c = dir.path();
        std::fs::create_dir_all(c.join("qt6ct")).unwrap();
        std::fs::write(c.join("qt6ct/qt6ct.conf"), "[Appearance]\nicon_theme=QtTheme\n").unwrap();
        assert_eq!(detect_in(c, None).as_deref(), Some("QtTheme"));
        std::fs::create_dir_all(c.join("gtk-3.0")).unwrap();
        std::fs::write(c.join("gtk-3.0/settings.ini"), "[Settings]\ngtk-icon-theme-name=Papirus-Dark\n").unwrap();
        assert_eq!(detect_in(c, None).as_deref(), Some("Papirus-Dark"));
        std::fs::write(c.join("gtk-3.0/settings.ini"), "[Settings]\ngtk-icon-theme-name=../etc\n").unwrap();
        assert_eq!(detect_in(c, None).as_deref(), Some("QtTheme"));
    }
}
