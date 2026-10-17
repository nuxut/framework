use nuxut_render::{GenericFamily, TextStyle};
use nuxut_theme::{FontRole, Theme};

/// Metin rolleri (ADR-0006). Boyutlar `typography.size` tabanından tip ölçeğiyle türetilir;
/// aile/ağırlık/ölçek temanın font rolünden gelir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    /// Ana metin: `typography.size` (`ui` rolü).
    Body,
    /// Küçük/ikincil metin.
    Caption,
    /// Düğme, sekme, rozet gibi kısa etiketler (gövde boyu, bir kademe kalın).
    Label,
    /// Başlık.
    Title,
    /// Büyük gösterge (saat gibi; `display` rolü).
    Display,
    /// Uzun okuma metni (takvim/etkinlik açıklaması; `document` rolü). Bildirim gövdesi `Body`
    /// kullanır (karar 2026-10-14).
    Document,
    /// Eş aralıklı (`mono` rolü).
    Mono,
    /// İkon (ikon fontu, satır yüksekliği 1).
    Icon,
    IconSmall,
    IconLarge,
}

/// Tip ölçeği oranı (major second ≈ 1.125); roller bunun kuvvetleridir.
const TYPE_SCALE: f32 = 1.125;

impl TextRole {
    fn step(self) -> i32 {
        match self {
            Self::Caption => -1,
            Self::Body | Self::Document | Self::Mono | Self::Label => 0,
            Self::Title => 2,
            Self::IconSmall => 2,
            Self::Icon => 3,
            Self::IconLarge => 5,
            Self::Display => 5,
        }
    }

    pub fn is_icon(self) -> bool {
        matches!(self, Self::Icon | Self::IconSmall | Self::IconLarge)
    }

    /// Temadaki font rolü ve kurulu değilse düşülecek genel aile.
    fn font(self, theme: &Theme) -> (&FontRole, GenericFamily) {
        let t = &theme.typography;
        match self {
            Self::Body | Self::Caption | Self::Label | Self::Title => (&t.ui, GenericFamily::SansSerif),
            Self::Display => (&t.display, GenericFamily::SansSerif),
            Self::Document => (&t.document, GenericFamily::Serif),
            Self::Mono => (&t.mono, GenericFamily::Monospace),
            Self::Icon | Self::IconSmall | Self::IconLarge => (&t.icon, GenericFamily::SansSerif),
        }
    }
}

/// Tema tipografisinden rol için `TextStyle`.
pub fn text_style(theme: &Theme, role: TextRole) -> TextStyle {
    let t = &theme.typography;
    let (font, fallback) = role.font(theme);
    let family = if role.is_icon() { t.icon_family() } else { font.family.as_str() };
    let size = (t.size * TYPE_SCALE.powi(role.step()) * font.scale).round();
    let weight = match role {
        TextRole::Title | TextRole::Label => (font.weight + 100).min(1000),
        _ => font.weight,
    };
    let mut style = TextStyle::new(family, size, weight).with_fallback(fallback);
    if role.is_icon() {
        style.line_height = 1.0;
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_scale_from_theme() {
        let theme = Theme::default();
        assert_eq!(text_style(&theme, TextRole::Body).size, 13.0);
        assert!(text_style(&theme, TextRole::Caption).size < 13.0);
        assert!(text_style(&theme, TextRole::Title).size > 13.0);
        assert_eq!(text_style(&theme, TextRole::Title).weight, 600);
        let icon = text_style(&theme, TextRole::Icon);
        assert_eq!(icon.family, theme.typography.icon_family());
        assert_eq!(icon.line_height, 1.0);
        assert!(icon.size > 13.0);
    }

    #[test]
    fn roles_use_their_font_role() {
        let (mut theme, _) = Theme::parse(
            "[typography.document]\nfamily = \"Belge\"\nscale = 1.2\n[typography.display]\nfamily = \"Gösterge\"\nweight = 300\n",
        )
        .unwrap();
        let doc = text_style(&theme, TextRole::Document);
        assert_eq!((doc.family.as_str(), doc.size, doc.fallback), ("Belge", (13.0f32 * 1.2).round(), GenericFamily::Serif));
        let display = text_style(&theme, TextRole::Display);
        assert_eq!((display.family.as_str(), display.weight), ("Gösterge", 300));
        let mono = text_style(&theme, TextRole::Mono);
        assert_eq!((mono.family.as_str(), mono.fallback), (theme.typography.mono.family.as_str(), GenericFamily::Monospace));
        theme.typography.icon_set = nuxut_theme::IconSet::Nerd;
        assert_eq!(text_style(&theme, TextRole::Icon).family, theme.typography.mono.family);
    }
}
