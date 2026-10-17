//! Tümü Nuxut'un dosyası: ilk satırdaki başlıkla tanınır.
//!
//! Başlık: `<yorum> Nuxut — <ne> · elle düzenleme bir sonraki uygulamada ezilir` (shell
//! `HEADER_PREFIX` = `# Nuxut — `). CSS gibi `#` yorumu olmayan biçimler için `/* … */` ve `//`,
//! Lua (Hyprland `.lua`) için `--` de tanınır. Başlıksız (yabancı) dosya ilk yazımda tümüyle
//! snapshot'a girer.

/// Başlık işareti (yorum ön ekinden sonra).
pub const HEADER_MARK: &str = "Nuxut — ";

/// Yorum biçimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `# …` (conf, ini, systemd, environment.d, Hyprland).
    Hash,
    /// `/* … */` (CSS).
    Block,
    /// `// …`
    Slash,
    /// `-- …` (Lua).
    DoubleDash,
}

/// Tanınan satır yorumu ön ekleri (başlık işaretinden önce).
const PREFIXES: [&str; 4] = ["# ", "/* ", "// ", "-- "];

/// Başlık satırı (satır sonu dahil). `what`: dosyanın ne olduğu (ör. "Hyprland ayarları").
pub fn header(style: CommentStyle, what: &str) -> String {
    let body = format!("{HEADER_MARK}{what} · elle düzenleme bir sonraki uygulamada ezilir");
    match style {
        CommentStyle::Hash => format!("# {body}\n"),
        CommentStyle::Block => format!("/* {body} */\n"),
        CommentStyle::Slash => format!("// {body}\n"),
        CommentStyle::DoubleDash => format!("-- {body}\n"),
    }
}

/// İlk satır bir Nuxut başlığı mı?
pub fn is_managed(bytes: &[u8]) -> bool {
    let first = bytes.split(|&b| b == b'\n').next().unwrap_or_default();
    let Ok(first) = std::str::from_utf8(first) else {
        return false;
    };
    let t = first.trim_start();
    PREFIXES.iter().any(|p| {
        t.strip_prefix(p)
            .is_some_and(|rest| rest.starts_with(HEADER_MARK))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_detection() {
        for s in [
            CommentStyle::Hash,
            CommentStyle::Block,
            CommentStyle::Slash,
            CommentStyle::DoubleDash,
        ] {
            let h = header(s, "GTK");
            assert!(is_managed(format!("{h}a\n").as_bytes()), "{h}");
        }
        assert!(is_managed(b"# Nuxut \xe2\x80\x94 x"));
        assert!(is_managed(b"-- Nuxut \xe2\x80\x94 x\nreturn {}\n"));
        assert!(!is_managed(b"a\n# Nuxut \xe2\x80\x94 x"));
        assert!(!is_managed(b"a\n-- Nuxut \xe2\x80\x94 x"));
        assert!(!is_managed(b"# Nuxut x"));
        // Lua: `--[[` blok yorumu ve boşluksuz `--Nuxut` başlık sayılmaz.
        assert!(!is_managed(b"--[[ Nuxut \xe2\x80\x94 x ]]"));
        assert!(!is_managed(b"--Nuxut \xe2\x80\x94 x"));
        assert!(!is_managed(b"- Nuxut \xe2\x80\x94 x"));
        assert!(!is_managed(b""));
        assert!(!is_managed(&[0xff, b'\n']));
    }
}
