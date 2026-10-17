//! Metin ölçeğinin GTK ve Qt'ye aktarımı (docs/design/accessibility.md §3): saf hesap, yazım
//! yok. Yazımı shell look yapar (dconf, `settings.ini`, kdeglobals, ortam); bu modül yalnızca
//! değerleri ve yuvarlama kurallarını verir. Çıktı deterministiktir.
//!
//! Girdi framework'ün metin ölçeğidir: `[typography] size` (mantıksal px) ve rol çarpanları.
//! Ölçek `f = size / 13` ([`nuxut_theme::TEXT_SIZE_REFERENCE`]).
//!
//! **Model (çift ölçeklemeyi önler):**
//!
//! - **GTK:** yazı tipi adı (`font-name`, `gtk-font-name`) REFERANS boyda yazılır (`13 px ×
//!   rol çarpanı`, pt), ölçek ayrıca `org.gnome.desktop.interface text-scaling-factor = f` ve
//!   `settings.ini gtk-xft-dpi = 96·1024·f` ile verilir. GTK ikisini çarpar; böylece sistem
//!   fontunu kullanmayan GTK/Electron/Firefox/Chromium metni de aynı oranda büyür (GNOME
//!   "Büyük metin" modeli). `f` adın içine ayrıca katılmaz.
//! - **Qt (kdeglobals yolu):** Qt'de canlı bir metin ölçeği anahtarı yok; ölçek kdeglobals
//!   QFont dizgelerinin pt boyuna KATILIR (`size × rol çarpanı`, pt) ve KDE platform teması onu
//!   canlı uygular. `QT_FONT_DPI` bu yolda AYARLANMAZ (ikisi birlikte çift ölçekler).
//! - **Qt (qt6ct yolu):** yazı tipleri yazılmaz (shell ADR-0004 §17-4); tek aktarım
//!   `QT_FONT_DPI = 96·f` (oturum ortamı, uygulama yeniden başlayınca). `f = 1` iken değişken
//!   hiç yazılmaz.
//! - `QT_SCALE_FACTOR` KULLANILMAZ: metni değil tüm arayüzü (ikon, boşluk) ölçekler.

use nuxut_theme::{TEXT_SIZE_REFERENCE, Typography};

/// `text-scaling-factor` aralığı (gsettings-desktop-schemas: `range d 0.5 3.0`).
pub const GTK_FACTOR_MIN: f64 = 0.5;
pub const GTK_FACTOR_MAX: f64 = 3.0;
/// GTK/Qt'nin referans mantıksal DPI'ı (1 pt = 96/72 px).
pub const DPI_REFERENCE: f64 = 96.0;
/// `gtk-xft-dpi` birimi: DPI × 1024.
pub const XFT_DPI_UNIT: f64 = 1024.0;
/// Mantıksal px → pt (96 dpi).
const PX_TO_PT: f32 = 0.75;
/// Yazı tipi boyu sınırı (px), shell `look::fonts::px_to_pt` ile aynı.
const PX_MIN: f32 = 4.0;
const PX_MAX: f32 = 200.0;

/// Doğrulanmış metin ölçeği: 0.01 adıma yuvarlanmış, GTK aralığına kırpılmış oran.
///
/// Yuvarlama: `f = round(size / 13, 0.01)`, sonra `[0.5, 3.0]`. Sonlu olmayan girdi 1.0.
/// 0.01 adımı dconf'a `1.0769230769…` gibi kayan nokta gürültüsü yazılmasını önler; etkin boyda
/// hata ≤ %0.5 (yarım pt'nin altında).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextScale {
    factor: f64,
}

impl TextScale {
    pub const IDENTITY: Self = Self { factor: 1.0 };

    /// Taban boydan (mantıksal px).
    pub fn from_size(size_px: f32) -> Self {
        let raw = f64::from(size_px) / f64::from(TEXT_SIZE_REFERENCE);
        if !raw.is_finite() || raw <= 0.0 {
            return Self::IDENTITY;
        }
        let rounded = (raw * 100.0).round() / 100.0;
        Self { factor: rounded.clamp(GTK_FACTOR_MIN, GTK_FACTOR_MAX) }
    }

    /// Temanın tipografisinden.
    pub fn of(t: &Typography) -> Self {
        Self::from_size(t.size)
    }

    pub fn factor(self) -> f64 {
        self.factor
    }

    pub fn is_identity(self) -> bool {
        self.factor == 1.0
    }

    /// `org.gnome.desktop.interface text-scaling-factor` (dconf, `d`).
    pub fn gtk_text_scaling_factor(self) -> f64 {
        self.factor
    }

    /// `settings.ini [Settings] gtk-xft-dpi` (tamsayı, DPI × 1024): `round(96 · 1024 · f)`.
    /// `f = 1` → 98304. GSettings/portal okunabilen GTK'da `text-scaling-factor`'dan türetilen
    /// değer bunun yerine geçer (toplanmaz); ikisi aynı `f`'ten yazıldığı için tutarlıdır.
    pub fn gtk_xft_dpi(self) -> i32 {
        (DPI_REFERENCE * XFT_DPI_UNIT * self.factor).round() as i32
    }

    /// `QT_FONT_DPI` (tamsayı DPI): `round(96 · f)`. Yalnızca yazı tipleri Qt'ye yazılmayan yolda
    /// ([`QtFonts::NotWritten`]) ve `f ≠ 1` iken `Some`; aksi hâlde değişken yazılmaz/kaldırılır.
    pub fn qt_font_dpi(self, fonts: QtFonts) -> Option<u32> {
        match fonts {
            QtFonts::Written => None,
            QtFonts::NotWritten => {
                let dpi = (DPI_REFERENCE * self.factor).round() as u32;
                (dpi != DPI_REFERENCE as u32).then_some(dpi)
            }
        }
    }
}

/// Qt yolunda yazı tipleri yazılıyor mu (kdeglobals QFont dizgeleri) yoksa yalnızca ortam mı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QtFonts {
    /// kdeglobals (`QT_QPA_PLATFORMTHEME=kde`): ölçek pt boyuna katılır.
    Written,
    /// qt6ct/qt5ct (yazı tipi yazılmaz): ölçek yalnızca `QT_FONT_DPI` ile.
    NotWritten,
}

/// pt'yi 0.5 adıma yuvarlar (GTK/KDE yazı tipi seçicilerinin adımı; `10`, `10.5`).
pub fn round_half_pt(pt: f32) -> f32 {
    (pt * 2.0).round() / 2.0
}

fn px_to_pt(px: f32) -> f32 {
    let px = if px.is_finite() { px.clamp(PX_MIN, PX_MAX) } else { TEXT_SIZE_REFERENCE };
    round_half_pt(px * PX_TO_PT)
}

/// GTK yazı tipi adının boyu (pt, 0.5 adım): REFERANS boy × rol çarpanı — ölçek
/// `text-scaling-factor`'dadır. Varsayılan rol: `13 × 0.75 = 9.75` → 10.
pub fn gtk_font_pt(role_scale: f32) -> f32 {
    px_to_pt(TEXT_SIZE_REFERENCE * sanitize_scale(role_scale))
}

/// Qt (kdeglobals) yazı tipi boyu (pt, 0.5 adım): GERÇEK boy × rol çarpanı (ölçek katılmış).
/// `size = 16` → `12`.
pub fn qt_font_pt(size_px: f32, role_scale: f32) -> f32 {
    px_to_pt(size_px * sanitize_scale(role_scale))
}

fn sanitize_scale(s: f32) -> f32 {
    if s.is_finite() && s > 0.0 { s } else { 1.0 }
}

/// GTK hedefleri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GtkText {
    /// dconf `org.gnome.desktop.interface text-scaling-factor`.
    pub text_scaling_factor: f64,
    /// `settings.ini gtk-xft-dpi` (GTK 3 ve 4).
    pub xft_dpi: i32,
    /// `font-name` / `gtk-font-name` boyu (ui rolü, referans boy).
    pub ui_pt: f32,
    /// `document-font-name` boyu.
    pub document_pt: f32,
    /// `monospace-font-name` boyu.
    pub mono_pt: f32,
}

/// Qt hedefleri.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QtText {
    /// kdeglobals `font` / `menuFont` / `toolBarFont` boyu (ui rolü, ölçek katılmış).
    pub ui_pt: f32,
    /// kdeglobals `fixed` boyu.
    pub mono_pt: f32,
    /// `QT_FONT_DPI`; `None` = yazma (ve Nuxut'un önceki değerini kaldır).
    pub font_dpi: Option<u32>,
}

/// GTK için tüm değerler.
pub fn gtk(t: &Typography) -> GtkText {
    let s = TextScale::of(t);
    GtkText {
        text_scaling_factor: s.gtk_text_scaling_factor(),
        xft_dpi: s.gtk_xft_dpi(),
        ui_pt: gtk_font_pt(t.ui.scale),
        document_pt: gtk_font_pt(t.document.scale),
        mono_pt: gtk_font_pt(t.mono.scale),
    }
}

/// Qt için tüm değerler (`fonts`: hangi Qt yolu etkin).
pub fn qt(t: &Typography, fonts: QtFonts) -> QtText {
    QtText {
        ui_pt: qt_font_pt(t.size, t.ui.scale),
        mono_pt: qt_font_pt(t.size, t.mono.scale),
        font_dpi: TextScale::of(t).qt_font_dpi(fonts),
    }
}

#[cfg(test)]
mod tests {
    use nuxut_theme::Theme;

    use super::*;

    fn typo(text: &str) -> Typography {
        Theme::parse(text).unwrap().0.typography
    }

    #[test]
    fn reference_is_the_default_size() {
        let t = Theme::default().typography;
        assert_eq!(t.size, TEXT_SIZE_REFERENCE);
        assert_eq!(t.text_scale(), 1.0);
        assert!(TextScale::of(&t).is_identity());
        let g = gtk(&t);
        assert_eq!(g.text_scaling_factor, 1.0);
        assert_eq!(g.xft_dpi, 98_304);
        assert_eq!(g.ui_pt, 10.0, "13 px = 9.75 pt → 10");
        let q = qt(&t, QtFonts::Written);
        assert_eq!(q.ui_pt, 10.0);
        assert_eq!(q.font_dpi, None);
        assert_eq!(qt(&t, QtFonts::NotWritten).font_dpi, None, "f = 1: QT_FONT_DPI yazılmaz");
    }

    #[test]
    fn larger_text_scales_gtk_by_factor_and_qt_by_size() {
        let t = typo("[typography]\nsize = 16\n");
        let s = TextScale::of(&t);
        assert_eq!(s.factor(), 1.23, "16/13 = 1.2307… → 0.01 adım");
        let g = gtk(&t);
        assert_eq!(g.text_scaling_factor, 1.23);
        assert_eq!(g.xft_dpi, 120_914, "96·1024·1.23 = 120913.92");
        assert_eq!(g.ui_pt, 10.0, "GTK adı referans boyda kalır (çift ölçek yok)");
        let q = qt(&t, QtFonts::Written);
        assert_eq!(q.ui_pt, 12.0, "16 px = 12 pt, ölçek katılmış");
        assert_eq!(q.font_dpi, None, "kdeglobals yolunda QT_FONT_DPI yok");
        assert_eq!(qt(&t, QtFonts::NotWritten).font_dpi, Some(118), "96·1.23 = 118.08");
        // Etkin boylar uyumlu: GTK 10 pt × 1.23 = 12.3 pt ≈ Qt 12 pt (≤ yarım pt).
        assert!((f64::from(g.ui_pt) * g.text_scaling_factor - f64::from(q.ui_pt)).abs() <= 0.5);
    }

    #[test]
    fn rounding_and_clamping() {
        assert_eq!(TextScale::from_size(13.0 * 1.25).factor(), 1.25);
        assert_eq!(TextScale::from_size(14.0).factor(), 1.08, "1.0769… → 1.08");
        assert_eq!(TextScale::from_size(12.0).factor(), 0.92);
        assert_eq!(TextScale::from_size(4.0).factor(), GTK_FACTOR_MIN, "0.31 → 0.5");
        assert_eq!(TextScale::from_size(128.0).factor(), GTK_FACTOR_MAX, "9.85 → 3.0");
        for bad in [f32::NAN, f32::INFINITY, 0.0, -13.0] {
            assert!(TextScale::from_size(bad).is_identity(), "{bad}");
        }
        assert_eq!(TextScale::from_size(26.0).gtk_xft_dpi(), 196_608);
        assert_eq!(TextScale::from_size(26.0).qt_font_dpi(QtFonts::NotWritten), Some(192));
        // Yarım pt yuvarlaması.
        assert_eq!(round_half_pt(9.75), 10.0);
        assert_eq!(round_half_pt(10.2), 10.0);
        assert_eq!(round_half_pt(10.3), 10.5);
        assert_eq!(qt_font_pt(f32::NAN, 1.0), 10.0);
        assert_eq!(qt_font_pt(13.0, f32::NAN), 10.0);
        assert_eq!(qt_font_pt(1000.0, 1.0), 150.0, "200 px sınırı");
    }

    #[test]
    fn role_scales_enter_both_targets_once() {
        let t = typo("[typography]\nsize = 16\n[typography.mono]\nscale = 0.9\n[typography.document]\nscale = 1.2\n");
        let g = gtk(&t);
        assert_eq!(g.mono_pt, 9.0, "13·0.9·0.75 = 8.775 → 9");
        assert_eq!(g.document_pt, 11.5, "13·1.2·0.75 = 11.7 → 11.5");
        let q = qt(&t, QtFonts::Written);
        assert_eq!(q.mono_pt, 11.0, "16·0.9·0.75 = 10.8 → 11");
        assert_eq!(g.text_scaling_factor, 1.23, "rol çarpanı ölçeğe girmez");
    }
}
