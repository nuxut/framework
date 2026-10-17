//! `index.theme` ayrıştırma (Icon Theme Spec §4) ve dizin–boyut eşleşmesi (§6).

use std::collections::HashMap;

/// Ayrıştırılmış basit INI: bölüm → anahtar → değer. Yerelleştirilmiş anahtarlar (`Name[tr]`)
/// atlanır; yinelenen anahtarda ilk değer kalır.
pub(crate) fn parse_ini(text: &str) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut section: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = Some(name.trim().to_owned());
            out.entry(name.trim().to_owned()).or_default();
            continue;
        }
        let (Some(sec), Some((k, v))) = (&section, line.split_once('=')) else { continue };
        let k = k.trim();
        if k.contains('[') {
            continue;
        }
        out.entry(sec.clone()).or_default().entry(k.to_owned()).or_insert_with(|| v.trim().to_owned());
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirType {
    Fixed,
    Scalable,
    Threshold,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubDir {
    pub path: String,
    pub size: u32,
    pub scale: u32,
    pub kind: DirType,
    pub min_size: u32,
    pub max_size: u32,
    pub threshold: u32,
}

impl SubDir {
    /// Spec: DirectoryMatchesSize.
    pub fn matches(&self, size: u32, scale: u32) -> bool {
        if self.scale != scale {
            return false;
        }
        match self.kind {
            DirType::Fixed => self.size == size,
            DirType::Scalable => (self.min_size..=self.max_size).contains(&size),
            DirType::Threshold => {
                self.size.saturating_sub(self.threshold) <= size && size <= self.size + self.threshold
            }
        }
    }

    /// Spec: DirectorySizeDistance (fiziksel piksel cinsinden).
    pub fn distance(&self, size: u32, scale: u32) -> u32 {
        let want = (size * scale) as i64;
        let (lo, hi) = match self.kind {
            DirType::Fixed => {
                let s = (self.size * self.scale) as i64;
                (s, s)
            }
            DirType::Scalable => ((self.min_size * self.scale) as i64, (self.max_size * self.scale) as i64),
            DirType::Threshold => (
                (self.size.saturating_sub(self.threshold) * self.scale) as i64,
                ((self.size + self.threshold) * self.scale) as i64,
            ),
        };
        if want < lo {
            (lo - want) as u32
        } else if want > hi {
            (want - hi) as u32
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ThemeIndex {
    pub inherits: Vec<String>,
    pub dirs: Vec<SubDir>,
}

impl ThemeIndex {
    pub fn parse(text: &str) -> Self {
        let ini = parse_ini(text);
        let Some(head) = ini.get("Icon Theme") else { return Self::default() };
        let list = |key: &str| -> Vec<String> {
            head.get(key)
                .map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let inherits = list("Inherits");
        let mut names = list("Directories");
        for d in list("ScaledDirectories") {
            if !names.contains(&d) {
                names.push(d);
            }
        }
        let dirs = names
            .into_iter()
            .filter_map(|path| {
                let sec = ini.get(&path)?;
                let num = |k: &str| sec.get(k).and_then(|v| v.parse::<u32>().ok());
                let size = num("Size")?;
                if size == 0 {
                    return None;
                }
                let kind = match sec.get("Type").map(String::as_str) {
                    Some("Fixed") => DirType::Fixed,
                    Some("Scalable") => DirType::Scalable,
                    _ => DirType::Threshold,
                };
                Some(SubDir {
                    path,
                    size,
                    scale: num("Scale").unwrap_or(1).max(1),
                    kind,
                    min_size: num("MinSize").unwrap_or(size),
                    max_size: num("MaxSize").unwrap_or(size),
                    threshold: num("Threshold").unwrap_or(2),
                })
            })
            .collect();
        Self { inherits, dirs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "\
[Icon Theme]
Name=Test
Name[tr]=Deneme
Inherits=parent, hicolor
Directories=16x16/apps,scalable/apps
ScaledDirectories=16x16@2/apps

[16x16/apps]
Size=16
Type=Fixed

[16x16@2/apps]
Size=16
Scale=2
Type=Fixed

[scalable/apps]
Size=48
MinSize=8
MaxSize=512
Type=Scalable
";

    #[test]
    fn parses_index() {
        let t = ThemeIndex::parse(INDEX);
        assert_eq!(t.inherits, ["parent", "hicolor"]);
        assert_eq!(t.dirs.len(), 3);
        assert_eq!(t.dirs[2].path, "16x16@2/apps");
        assert_eq!(t.dirs[2].scale, 2);
        assert!(t.dirs[0].matches(16, 1));
        assert!(!t.dirs[0].matches(16, 2));
        assert!(t.dirs[1].matches(300, 1));
        assert_eq!(t.dirs[0].distance(24, 1), 8);
        assert_eq!(t.dirs[1].distance(600, 1), 88);
    }
}
