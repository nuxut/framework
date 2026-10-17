//! Uygulama sınıfı (Wayland `app_id`, Hyprland `class`) → uygulama logosu glifi (Nerd Font) ve
//! glifleri taşıyan kurulu Nerd Font'un bulunması.
//!
//! - **Tablo veridir:** `app_glyphs.txt` (derlemeye gömülü; satır biçimi dosyanın başında).
//!   İlk kullanımda bir kez ayrıştırılır ([`AppGlyphs::builtin`]); arama karma tablodur.
//!   Uygulama kendi eşlemelerini ekleyebilir ([`AppGlyphs::insert`], ör. kullanıcı config'i).
//! - **Eşleme sırası:** tam ad → noktalı adın parçaları sondan başa (`org.gnome.Nautilus` →
//!   `nautilus`), her biri olduğu gibi ve `-bin`/`-git`/`-wayland`… ekleri atılmış → önekler.
//!   Büyük/küçük harf duyarsız.
//! - **Font:** [`find_nerd_font`] TextSystem'in font veritabanında (Linux'ta fontconfig'in font
//!   dizinleri ve ayarı) arar: `Symbols Nerd Font` → tercih edilen aile (ör. temanın `mono`'su)
//!   Nerd Font ise → kurulu herhangi bir Nerd Font. [`NerdFont`] sonucu bir kez çözüp saklar
//!   (aile varlığı ve aile listesi TextSystem'de de önbellekli); font yoksa glif yerine numara/
//!   baş harf gibi bir yedek gösterilmeli.

use std::collections::HashMap;
use std::sync::LazyLock;

use nuxut_render::TextSystem;

/// Yalnız sembol glifleri taşıyan Nerd Font ailesi (tercih edilen).
pub const NERD_SYMBOLS_FAMILY: &str = "Symbols Nerd Font";
/// Daha uzun sınıf adları eşlenmez (kötü niyetli/bozuk girdiye karşı sınır).
pub const MAX_APP_CLASS: usize = 256;

const BUILTIN: &str = include_str!("app_glyphs.txt");
const SUFFIXES: &[&str] = &["-bin", "-git", "-wayland", "-stable", "-beta", "-nightly", "-desktop"];

/// Sınıf → glif tablosu.
#[derive(Debug, Clone, Default)]
pub struct AppGlyphs {
    exact: HashMap<String, char>,
    /// Sırayla denenir (ilk eşleşen kazanır).
    prefixes: Vec<(String, char)>,
}

/// Tablo verisinde hata (satır numarası 1'den).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGlyphParseError {
    pub line: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for AppGlyphParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "uygulama glif tablosu, satır {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for AppGlyphParseError {}

static BUILTIN_TABLE: LazyLock<AppGlyphs> =
    LazyLock::new(|| AppGlyphs::parse(BUILTIN).expect("gömülü app_glyphs.txt geçerli (testli)"));

impl AppGlyphs {
    /// Gömülü tablo (ilk çağrıda bir kez ayrıştırılır).
    pub fn builtin() -> &'static AppGlyphs {
        &BUILTIN_TABLE
    }

    /// `app_glyphs.txt` biçimindeki veriyi ayrıştırır. Sınıflar küçük harfe çevrilir; aynı
    /// sınıf ikinci kez gelirse hata.
    pub fn parse(src: &str) -> Result<Self, AppGlyphParseError> {
        let mut t = Self::default();
        for (i, line) in src.lines().enumerate() {
            let err = |reason| AppGlyphParseError { line: i + 1, reason };
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut words = line.split_whitespace();
            let first = words.next().ok_or(err("boş"))?;
            let prefix = first == "prefix";
            let hex = if prefix { words.next().ok_or(err("kod noktası yok"))? } else { first };
            let glyph = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32).ok_or(err("geçersiz kod noktası"))?;
            if !is_private_use(glyph) {
                return Err(err("kod noktası özel kullanım alanında değil"));
            }
            let _name = words.next().ok_or(err("glif adı yok"))?;
            let mut any = false;
            for class in words {
                any = true;
                let class = class.to_lowercase();
                if prefix {
                    t.prefixes.push((class, glyph));
                } else if t.exact.insert(class, glyph).is_some() {
                    return Err(err("yinelenen sınıf"));
                }
            }
            if !any {
                return Err(err("sınıf yok"));
            }
        }
        Ok(t)
    }

    /// Eşleme ekler ya da değiştirir (ör. kullanıcı config'i). Sınıf küçük harfe çevrilir.
    pub fn insert(&mut self, class: &str, glyph: char) {
        let class = class.trim().to_lowercase();
        if !class.is_empty() && class.len() <= MAX_APP_CLASS {
            self.exact.insert(class, glyph);
        }
    }

    /// Sınıfın glifi; tanınmazsa `None`.
    pub fn glyph(&self, class: &str) -> Option<char> {
        let class = class.trim();
        if class.is_empty() || class.len() > MAX_APP_CLASS {
            return None;
        }
        let lower = class.to_lowercase();
        let lookup = |k: &str| self.exact.get(k).copied();
        if let Some(g) = lookup(&lower) {
            return Some(g);
        }
        for part in lower.rsplit('.') {
            let trimmed = SUFFIXES.iter().find_map(|s| part.strip_suffix(s)).unwrap_or(part);
            if let Some(g) = lookup(part).or_else(|| lookup(trimmed)) {
                return Some(g);
            }
        }
        let last = lower.rsplit('.').next().unwrap_or(&lower);
        self.prefixes.iter().find(|(p, _)| lower.starts_with(p.as_str()) || last.starts_with(p.as_str())).map(|&(_, g)| g)
    }

    pub fn len(&self) -> usize {
        self.exact.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exact.is_empty()
    }
}

/// Gömülü tabloda sınıfın glifi ([`AppGlyphs::builtin`]`.glyph`).
pub fn app_glyph(class: &str) -> Option<char> {
    AppGlyphs::builtin().glyph(class)
}

fn is_private_use(c: char) -> bool {
    ('\u{e000}'..='\u{f8ff}').contains(&c) || ('\u{f0000}'..='\u{ffffd}').contains(&c)
}

/// Aile adı bir Nerd Font mu (yamalı fontların adlandırması: "… Nerd Font", "… NF", "… NFM").
pub fn is_nerd_font_family(name: &str) -> bool {
    let n = name.trim();
    n.contains("Nerd Font") || [" NF", " NFM", " NFP"].iter().any(|s| n.ends_with(s) || n.contains(&format!("{s} ")))
}

/// Kurulu ailelerden en uygun Nerd Font: önce düz "… Nerd Font", sonra "Propo", sonra "Mono",
/// sonra diğer adlandırmalar ("NF"); eşitlikte ada göre (büyük/küçük harf duyarsız).
fn best_nerd_font<'a>(families: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let rank = |n: &str| {
        if n.ends_with("Nerd Font") {
            0
        } else if n.ends_with("Nerd Font Propo") {
            1
        } else if n.ends_with("Nerd Font Mono") {
            2
        } else {
            3
        }
    };
    families
        .into_iter()
        .filter(|f| is_nerd_font_family(f))
        .min_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.to_lowercase().cmp(&b.to_lowercase())))
        .map(str::to_owned)
}

/// Kurulu Nerd Font ailesi: `Symbols Nerd Font` → `preferred` (Nerd Font ise) → kurulu Nerd
/// Font'lardan en uygunu; yoksa `None`. Sonucu [`NerdFont`] ile saklayın (aile varlığı ve aile
/// listesi TextSystem'de önbellekli; liste yalnız ilk iki adım bulamazsa okunur).
pub fn find_nerd_font(text: &mut TextSystem, preferred: &str) -> Option<String> {
    if text.has_family(NERD_SYMBOLS_FAMILY) {
        return Some(NERD_SYMBOLS_FAMILY.to_owned());
    }
    if is_nerd_font_family(preferred) && text.has_family(preferred) {
        return Some(preferred.to_owned());
    }
    best_nerd_font(text.families().iter().map(|f| f.name.as_str()))
}

/// Bir kez çözülen Nerd Font ailesi (ör. bileşen başına ya da uygulama başına bir tane).
/// Tercih edilen aile değişince ([`resolve`](Self::resolve) farklı `preferred`) yeniden çözülür.
#[derive(Debug, Clone, Default)]
pub struct NerdFont {
    /// (çözüldüğü tercih, sonuç)
    resolved: Option<(String, Option<String>)>,
}

impl NerdFont {
    pub fn new() -> Self {
        Self::default()
    }

    /// Glif ailesi; kurulu Nerd Font yoksa `None`. İlk çağrıda (ya da `preferred` değişince)
    /// çözer, sonra saklananı döndürür.
    pub fn resolve(&mut self, text: &mut TextSystem, preferred: &str) -> Option<&str> {
        if self.resolved.as_ref().is_none_or(|(p, _)| p != preferred) {
            self.resolved = Some((preferred.to_owned(), find_nerd_font(text, preferred)));
        }
        self.resolved.as_ref().and_then(|(_, f)| f.as_deref())
    }

    /// Çözüldü mü (sonuç ne olursa olsun)?
    pub fn is_resolved(&self) -> bool {
        self.resolved.is_some()
    }

    /// Font kurulumu değişti: sonraki `resolve` yeniden arar.
    pub fn reset(&mut self) {
        self.resolved = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_table_parses_and_is_well_formed() {
        let t = AppGlyphs::builtin();
        assert!(t.len() > 200, "{}", t.len());
        assert!(!t.prefixes.is_empty());
        // Her veri satırı: hex, nf- adı, en az bir sınıf; tüm sınıflar küçük harf.
        for line in BUILTIN.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
            let w: Vec<&str> = line.split_whitespace().collect();
            let name = if w[0] == "prefix" { w[2] } else { w[1] };
            assert!(name.starts_with("nf-"), "{line}");
            assert!(w.iter().all(|s| *s == s.to_lowercase()), "{line}");
        }
    }

    #[test]
    fn known_classes() {
        assert_eq!(app_glyph("firefox"), Some('\u{f0239}'));
        assert_eq!(app_glyph("kitty"), Some('\u{f018d}'));
        assert_eq!(app_glyph("org.gnome.Nautilus"), Some('\u{f024b}'), "son parça, küçük harf");
        assert_eq!(app_glyph("com.mitchellh.ghostty"), Some('\u{f018d}'));
        assert_eq!(app_glyph("code-oss"), Some('\u{f0a1e}'));
        assert_eq!(app_glyph("Spotify"), Some('\u{f04c7}'));
        assert_eq!(app_glyph("discord-bin"), Some('\u{f066f}'), "-bin eki");
        assert_eq!(app_glyph("org.telegram.desktop"), app_glyph("telegram"), "son parça 'desktop' değil tam ad");
        assert_eq!(app_glyph("  kitty "), Some('\u{f018d}'), "boşluk kırpılır");
    }

    #[test]
    fn prefixes_and_unknown() {
        assert_eq!(app_glyph("steam_app_570"), Some('\u{f0297}'));
        assert_eq!(app_glyph("libreoffice-writer"), Some('\u{f37c}'), "tablo önekten önce");
        assert_eq!(app_glyph("jetbrains-goland"), Some('\u{e808}'));
        assert_eq!(app_glyph("firefox-aurora"), Some('\u{f0239}'));
        assert_eq!(app_glyph("some-unknown-app"), None);
        assert_eq!(app_glyph(""), None);
        assert_eq!(app_glyph(&"a".repeat(1000)), None);
    }

    #[test]
    fn parse_errors_and_overrides() {
        let bad = |src: &str| AppGlyphs::parse(src).unwrap_err();
        assert_eq!(bad("zzzz nf-x a").reason, "geçersiz kod noktası");
        assert_eq!(bad("41 nf-x a").reason, "kod noktası özel kullanım alanında değil");
        assert_eq!(bad("f000 nf-x").reason, "sınıf yok");
        assert_eq!(bad("# yorum\nf000 nf-x a\nf001 nf-y A").line, 3, "yinelenen (büyük/küçük harf duyarsız)");
        assert_eq!(bad("prefix f000").reason, "glif adı yok");
        let mut t = AppGlyphs::parse("f000 nf-x foo\nprefix f001 nf-y bar-").unwrap();
        assert_eq!(t.glyph("bar-baz"), Some('\u{f001}'));
        t.insert("Bar-Baz", '\u{f002}');
        assert_eq!(t.glyph("bar-baz"), Some('\u{f002}'), "eklenen eşleme önekten önce");
        t.insert(&"x".repeat(MAX_APP_CLASS + 1), '\u{f003}');
        assert_eq!(t.len(), 2, "sınırı aşan sınıf eklenmez");
    }

    #[test]
    fn nerd_font_names() {
        for n in ["Symbols Nerd Font", "JetBrainsMono Nerd Font", "Hack Nerd Font Mono", "FiraCode NF", "Iosevka NFM", "CaskaydiaCove NF SemiBold"] {
            assert!(is_nerd_font_family(n), "{n}");
        }
        for n in ["JetBrains Mono", "Noto Sans", "NFoo", "Inter"] {
            assert!(!is_nerd_font_family(n), "{n}");
        }
    }

    #[test]
    fn best_installed_nerd_font_order() {
        let all = ["Noto Sans", "Hack Nerd Font Mono", "iosevka Nerd Font Propo", "FiraCode Nerd Font", "Agave Nerd Font", "Lilex NF"];
        assert_eq!(best_nerd_font(all).as_deref(), Some("Agave Nerd Font"), "düz ad önce, sonra alfabetik");
        assert_eq!(best_nerd_font(["Hack Nerd Font Mono", "iosevka Nerd Font Propo"]).as_deref(), Some("iosevka Nerd Font Propo"));
        assert_eq!(best_nerd_font(["Lilex NF", "Hack Nerd Font Mono"]).as_deref(), Some("Hack Nerd Font Mono"));
        assert_eq!(best_nerd_font(["Noto Sans", "Inter"]), None);
    }

    #[test]
    fn resolver_caches_until_preference_changes() {
        let mut text = TextSystem::new();
        let mut nf = NerdFont::new();
        assert!(!nf.is_resolved());
        let first = nf.resolve(&mut text, "JetBrainsMono Nerd Font").map(str::to_owned);
        assert!(nf.is_resolved());
        // Sonuç ortama bağlı (fontsuz CI'da None); tutarlı ve önbellekli olmalı.
        assert_eq!(nf.resolve(&mut text, "JetBrainsMono Nerd Font").map(str::to_owned), first);
        if let Some(f) = &first {
            assert!(is_nerd_font_family(f) && text.has_family(f));
        }
        nf.reset();
        assert!(!nf.is_resolved());
    }
}
