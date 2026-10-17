//! Not satırı: ikon + sarılan kısa metin. [`NoticeKind::Warning`] dikkat isteyen durum
//! (ör. ses 100 %'ün üstünde; `error` ikon), [`NoticeKind::Info`] yumuşak bilgi notu
//! (ör. "saat ağdan eşitleniyor"; `on_surface_variant` ikon ve metin), [`NoticeKind::Success`]
//! olumlu sonuç (ör. "kaydedildi"; [`Palette::success`] ikon). Renkler yalnızca tema
//! rollerinden; dolgu/çerçeve yok (satırın içinde durur, iç boşluğu çağıran verir).
//!
//! Boyut: [`NoticeSize::Regular`] (Body metin, `Icon` ikon) ya da [`NoticeSize::Compact`]
//! (Caption metin, `IconSmall` ikon, sıkı boşluk; ör. Ayarlar'da form satırının altındaki
//! geçersiz kılma uyarısı). İki boyutta ikon/metin oranı aynıdır (tip ölçeğinde üç adım).

use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_render::TextSystem;
use nuxut_theme::{Palette, Theme};

use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::label::Label;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Ui};

/// Notun düzeyi: ikon ve renk rolleri buradan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NoticeKind {
    /// Uyarı: `Warning` ikonu `error` renginde, metin `on_surface`.
    #[default]
    Warning,
    /// Bilgi: `Info` ikonu ve metin `on_surface_variant` (yumuşak, dikkat çekmez).
    Info,
    /// Başarı: `CheckCircle` ikonu [`Palette::success`] renginde (yüzeylere karşı ≥ 4.5:1,
    /// yüksek karşıtlıkta da), metin `on_surface`.
    Success,
}

impl NoticeKind {
    pub fn icon(self) -> IconName {
        match self {
            Self::Warning => IconName::Warning,
            Self::Info => IconName::Info,
            Self::Success => IconName::CheckCircle,
        }
    }

    /// `Success`'te [`Palette::success`] hesaplanır (saf ama ucuz değil; [`Notice`] önbelleğe
    /// alır).
    pub fn icon_color(self, colors: &Palette) -> Color {
        match self {
            Self::Warning => colors.error,
            Self::Info => colors.on_surface_variant,
            Self::Success => colors.success(),
        }
    }

    pub fn text_color(self, colors: &Palette) -> Color {
        match self {
            Self::Warning | Self::Success => colors.on_surface,
            Self::Info => colors.on_surface_variant,
        }
    }
}

/// Notun boyutu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NoticeSize {
    /// Body metin, `Icon` ikon, `space(3)` boşluk (panel/sayfa içi not).
    #[default]
    Regular,
    /// Caption metin, `IconSmall` ikon, `space(2)` boşluk (form satırı altı yardımcı/uyarı).
    Compact,
}

impl NoticeSize {
    pub fn text_role(self) -> TextRole {
        match self {
            Self::Regular => TextRole::Body,
            Self::Compact => TextRole::Caption,
        }
    }

    pub fn icon_role(self) -> TextRole {
        match self {
            Self::Regular => TextRole::Icon,
            Self::Compact => TextRole::IconSmall,
        }
    }

    /// İkonla metin arası.
    pub fn gap(self, theme: &Theme) -> f32 {
        match self {
            Self::Regular => theme.space(3),
            Self::Compact => theme.space(2),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Notice {
    kind: NoticeKind,
    size: NoticeSize,
    icon: IconView,
    text: Label,
    rect: Rect,
    damage: DamageState,
    /// Son çizimdeki palet ve ikon rengi (`Success` her karede yeniden türetilmesin).
    icon_color: Option<(NoticeKind, Palette, Color)>,
}

impl Notice {
    pub fn new(theme: &Theme, kind: NoticeKind, text: &str) -> Self {
        Self {
            kind,
            size: NoticeSize::Regular,
            icon: IconView::new(theme, Icon::named(kind.icon())),
            text: Label::new(text, text_style(theme, TextRole::Body)),
            rect: Rect::ZERO,
            damage: DamageState::new(),
            icon_color: None,
        }
    }

    /// Kurucu: boyut ([`NoticeSize::Compact`] form satırı altı için).
    pub fn with_size(mut self, theme: &Theme, size: NoticeSize) -> Self {
        self.set_size(theme, size);
        self
    }

    /// `Notice::new(..).with_size(theme, NoticeSize::Compact)` kısayolu.
    pub fn compact(self, theme: &Theme) -> Self {
        self.with_size(theme, NoticeSize::Compact)
    }

    pub fn size(&self) -> NoticeSize {
        self.size
    }

    /// Boyutu değiştirir (metin ve ikon rolleri, boşluk). Değiştiyse `true`; ölçü değişir,
    /// çağıran yeniden `measure` etmeli.
    pub fn set_size(&mut self, theme: &Theme, size: NoticeSize) -> bool {
        if self.size == size {
            return false;
        }
        self.size = size;
        self.icon.set_role(theme, size.icon_role());
        self.text.set_style(text_style(theme, size.text_role()));
        self.damage.mark();
        true
    }

    pub fn kind(&self) -> NoticeKind {
        self.kind
    }

    /// Düzey değişince ikon ve renkler değişir; ölçü aynı kalır (ikonlar aynı boyda).
    pub fn set_kind(&mut self, kind: NoticeKind) -> bool {
        if self.kind == kind {
            return false;
        }
        self.kind = kind;
        self.icon.set_icon(Icon::named(kind.icon()));
        self.damage.mark();
        true
    }

    pub fn text(&self) -> &str {
        self.text.text()
    }

    pub fn set_text(&mut self, text: &str) -> bool {
        let changed = self.text.set_text(text);
        if changed {
            self.damage.mark();
        }
        changed
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.icon.set_theme(theme, now);
        self.text.set_style(text_style(theme, self.size.text_role()));
        self.damage.mark();
    }

    /// `width` içinde gereken boyut: metin ikon + boşluktan kalan genişlikte sarılır; yükseklik
    /// ikon ile metnin büyüğü.
    pub fn measure(&mut self, text: &mut TextSystem, theme: &Theme, width: f32) -> Size {
        let is = self.icon.size();
        let tw = (width - is.width - self.size.gap(theme)).max(1.0);
        self.text.set_max_width(Some(tw));
        let m = self.text.measure(text);
        Size::new(width, m.height.max(is.height).ceil())
    }

    pub fn set_rect(&mut self, rect: Rect) {
        if self.rect != rect {
            self.rect = rect;
            self.damage.mark();
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn icon_color(&mut self, colors: &Palette) -> Color {
        match self.icon_color {
            Some((k, p, c)) if k == self.kind && p == *colors => c,
            _ => {
                let c = self.kind.icon_color(colors);
                self.icon_color = Some((self.kind, *colors, c));
                c
            }
        }
    }

    /// İkon ilk satırın ortasına hizalanır (çok satırlı metinde üstte kalır).
    pub fn draw(&mut self, ui: &mut Ui) {
        let t = ui.theme;
        let rect = self.rect;
        self.damage.report(ui, rect, false);
        let is = self.icon.size();
        let gap = self.size.gap(t);
        let tw = (self.rect.width - is.width - gap).max(1.0);
        self.text.set_max_width(Some(tw));
        let m = self.text.measure_in(ui.painter);
        let h = m.height.max(is.height);
        let top = self.rect.y + (self.rect.height - h).max(0.0) * 0.5;
        let style = self.text.style();
        let line = style.size * style.line_height;
        let icon_cy = top + if m.height > is.height { line.min(m.height) * 0.5 } else { h * 0.5 };
        let icon_color = self.icon_color(&t.colors);
        self.icon.draw_centered(ui.painter, Point::new(self.rect.x + is.width * 0.5, icon_cy), ui.fade(icon_color));
        let ty = top + (h - m.height) * 0.5;
        self.text.draw(ui.painter, Point::new(self.rect.x + is.width + gap, ty), ui.fade(self.kind.text_color(&t.colors)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_kind_uses_soft_theme_roles() {
        let theme = Theme::default();
        let c = &theme.colors;
        assert_eq!(NoticeKind::default(), NoticeKind::Warning);
        assert_eq!(NoticeKind::Warning.icon(), IconName::Warning);
        assert_eq!(NoticeKind::Info.icon(), IconName::Info);
        assert_eq!(NoticeKind::Warning.icon_color(c), c.error);
        assert_eq!(NoticeKind::Warning.text_color(c), c.on_surface);
        assert_eq!(NoticeKind::Info.icon_color(c), c.on_surface_variant);
        assert_eq!(NoticeKind::Info.text_color(c), c.on_surface_variant);
        assert_ne!(NoticeKind::Info.icon_color(c), c.error, "bilgi notu kırmızı değil");
    }

    #[test]
    fn measure_wraps_and_kind_keeps_size() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let msg = "Saat ağdan eşitleniyor; elle ayar için otomatik eşitlemeyi kapatın.";
        let mut n = Notice::new(&theme, NoticeKind::Info, msg);
        let wide = n.measure(&mut text, &theme, 2000.0);
        let narrow = n.measure(&mut text, &theme, 220.0);
        assert_eq!(narrow.width, 220.0);
        assert!(wide.height >= n.icon.size().height);
        assert!(narrow.height > wide.height, "dar alanda metin sarılır: {narrow:?} / {wide:?}");
        assert!(n.set_kind(NoticeKind::Warning));
        assert!(!n.set_kind(NoticeKind::Warning));
        assert_eq!(n.measure(&mut text, &theme, 220.0), narrow, "düzey ölçüyü değiştirmez");
        assert!(n.set_text("Kısa"));
        assert_eq!(n.text(), "Kısa");
        assert!(n.measure(&mut text, &theme, 220.0).height <= narrow.height);
    }

    /// Başarı notu: `CheckCircle`, türetilmiş `success` ikon rengi, `on_surface` metin; iki kipte
    /// standart ve yüksek karşıtlıkta metin ≥ 4.5, ikon ≥ 3 (üç yüzeye karşı).
    #[test]
    fn success_kind_contrast() {
        use nuxut_theme::contrast::{NON_TEXT_MIN, TEXT_MIN, contrast_ratio};
        assert_eq!(NoticeKind::Success.icon(), IconName::CheckCircle);
        for mode in ["dark", "light"] {
            for contrast in ["standard", "high"] {
                let (t, w) = Theme::parse(&format!("[colors]\nmode = \"{mode}\"\ncontrast = \"{contrast}\"\n")).unwrap();
                assert!(w.is_empty(), "{w:?}");
                let c = &t.colors;
                let icon = NoticeKind::Success.icon_color(c);
                let text = NoticeKind::Success.text_color(c);
                assert_eq!(icon, c.success());
                assert_eq!(text, c.on_surface);
                assert_ne!(icon, NoticeKind::Warning.icon_color(c), "uyarıdan ayrışır");
                for bg in [c.surface, c.surface_container, c.surface_bright] {
                    assert!(contrast_ratio(text, bg) >= TEXT_MIN, "{mode}/{contrast} metin");
                    assert!(contrast_ratio(icon, bg) >= NON_TEXT_MIN, "{mode}/{contrast} ikon");
                }
                // Önbellek paletle değişir.
                let mut n = Notice::new(&t, NoticeKind::Success, "Kaydedildi");
                assert_eq!(n.icon_color(c), icon);
                assert!(n.set_kind(NoticeKind::Info));
                assert_eq!(n.icon_color(c), c.on_surface_variant);
            }
        }
    }

    /// Sıkı boyut: Caption metin, `IconSmall` ikon; aynı metin daha kısa ve dar sarılmaz.
    #[test]
    fn compact_size_measures_smaller() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let msg = "Bu değer ~/.config/hypr/input.conf içinde geçersiz kılınıyor.";
        let mut regular = Notice::new(&theme, NoticeKind::Warning, msg);
        let mut compact = Notice::new(&theme, NoticeKind::Warning, msg).compact(&theme);
        assert_eq!(regular.size(), NoticeSize::Regular);
        assert_eq!(compact.size(), NoticeSize::Compact);
        assert_eq!(compact.text.style().size, text_style(&theme, TextRole::Caption).size);
        assert_eq!(compact.icon.size().height, text_style(&theme, TextRole::IconSmall).size);
        assert!(compact.icon.size().height < regular.icon.size().height);
        assert!(NoticeSize::Compact.gap(&theme) < NoticeSize::Regular.gap(&theme));
        let r = regular.measure(&mut text, &theme, 2000.0);
        let c = compact.measure(&mut text, &theme, 2000.0);
        assert!(c.height < r.height, "{c:?} / {r:?}");
        // Dar alanda da sarılır; düzey ölçüyü değiştirmez.
        let narrow = compact.measure(&mut text, &theme, 200.0);
        assert!(narrow.height > c.height);
        compact.set_kind(NoticeKind::Success);
        assert_eq!(compact.measure(&mut text, &theme, 200.0), narrow);
        // set_size geri döner; tema değişimi boyutu korur.
        assert!(!compact.set_size(&theme, NoticeSize::Compact));
        compact.set_theme(&theme, Instant::now());
        assert_eq!(compact.measure(&mut text, &theme, 200.0), narrow);
        assert!(compact.set_size(&theme, NoticeSize::Regular));
        let mut back = Notice::new(&theme, NoticeKind::Success, msg);
        assert_eq!(compact.measure(&mut text, &theme, 200.0), back.measure(&mut text, &theme, 200.0));
        assert_eq!(Notice::new(&theme, NoticeKind::Info, "x").with_size(&theme, NoticeSize::Compact).size(), NoticeSize::Compact);
    }
}
