//! Avatar: daire içinde baş harfler (adın karmasından türetilen, temanın vurgu renginin tonu
//! döndürülmüş zemin) ya da görsel yuvası; isteğe bağlı çevrimiçi noktası.
//!
//! Görsel `Painter::image` ile daireye kırpılır.

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect};
use nuxut_motion::Animated;
use nuxut_render::{Border, ImageFit, ImageStyle, RectStyle, TextStyle, Texture};
use nuxut_theme::Theme;

use crate::label::Label;
use crate::style::{FOCUS_RING, Hsv, content_on};
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvatarSize {
    Small,
    #[default]
    Medium,
    Large,
    /// Profil kartı boyu (Ayarlar → Kullanıcı hesabı): 96 px, `space(24)`.
    XLarge,
}

impl AvatarSize {
    pub fn side(self, theme: &Theme) -> f32 {
        match self {
            Self::Small => theme.space(8),
            Self::Medium => theme.space(10),
            Self::Large => theme.space(16),
            Self::XLarge => theme.space(24),
        }
    }

    /// Baş harf stili. Küçük boylar tip ölçeğinin rolünü olduğu gibi kullanır; `XLarge`
    /// `Title`'ı (Large ile aynı aile/ağırlık) dairenin `Large`'a oranıyla büyütür — baş
    /// harfin daireye oranı boydan boya aynı kalır, tema yazı boyutunu izler.
    fn text_style(self, theme: &Theme) -> TextStyle {
        match self {
            Self::Small => text_style(theme, TextRole::Caption),
            Self::Medium => text_style(theme, TextRole::Label),
            Self::Large => text_style(theme, TextRole::Title),
            Self::XLarge => {
                let mut style = text_style(theme, TextRole::Title);
                style.size = (style.size * self.side(theme) / Self::Large.side(theme)).round();
                style
            }
        }
    }
}

#[derive(Clone)]
pub struct Avatar {
    name: String,
    initials: Label,
    size: AvatarSize,
    image: Option<Texture>,
    /// Doku henüz yüklenmemişti (hazır olduğu karede bölge yeniden çizilir).
    image_pending: bool,
    presence: bool,
    presence_scale: Animated<f32>,
    center: Point,
    damage: DamageState,
}

/// Kişi adı kişisel veridir: `Debug` yalnızca uzunluk yazar.
impl std::fmt::Debug for Avatar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avatar")
            .field("name", &crate::redact::Len(&self.name))
            .field("size", &self.size)
            .field("image", &self.image.is_some())
            .field("presence", &self.presence)
            .field("center", &self.center)
            .finish_non_exhaustive()
    }
}

/// Addan en fazla iki baş harf ("Ayşe Yılmaz" → "AY", "nuxut" → "N").
pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric()));
    let first = words.next();
    let last = words.next_back();
    first.into_iter().chain(last).flat_map(char::to_uppercase).collect()
}

/// Ad → kararlı ton kayması (FNV-1a).
fn name_hue(name: &str) -> f32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    (h % 360) as f32
}

impl Avatar {
    pub fn new(theme: &Theme, name: &str, size: AvatarSize) -> Self {
        Self {
            name: name.to_owned(),
            initials: Label::new(initials(name), size.text_style(theme)),
            size,
            image: None,
            image_pending: false,
            presence: false,
            presence_scale: Animated::new(0.0, theme.motion.spatial_fast),
            center: Point::ZERO,
            damage: DamageState::new(),
        }
    }

    /// Profil görseli (daireye kırpılır; doku yüklenene kadar baş harfler görünür).
    pub fn with_image(mut self, image: Option<Texture>) -> Self {
        self.image = image;
        self
    }

    pub fn set_image(&mut self, image: Option<Texture>) {
        if self.image != image {
            self.image = image;
            self.damage.mark();
        }
    }

    pub fn set_presence(&mut self, online: bool, now: Instant) {
        if self.presence != online {
            self.presence = online;
            self.presence_scale.set_target(if online { 1.0 } else { 0.0 }, now);
            self.damage.mark();
        }
    }

    pub fn set_name(&mut self, name: &str) {
        if self.name != name {
            name.clone_into(&mut self.name);
            self.initials.set_text(&initials(name));
            self.damage.mark();
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.initials.set_style(self.size.text_style(theme));
        self.presence_scale.set_motion(theme.motion.spatial_fast, now);
        self.damage.mark();
    }

    /// Zemin rengi: `primary`'nin doygunluk/değeri, adın karmasıyla döndürülmüş ton.
    pub fn background(&self, theme: &Theme) -> Color {
        let p = Hsv::from_color(theme.colors.primary);
        let h = Hsv::new(p.h + name_hue(&self.name), p.s.max(0.25), p.v);
        h.to_color()
    }

    pub fn size(&self) -> AvatarSize {
        self.size
    }

    pub fn set_center(&mut self, center: Point) {
        if self.center != center {
            self.center = center;
            self.damage.mark();
        }
    }

    pub fn rect(&self, theme: &Theme) -> Rect {
        let s = self.size.side(theme);
        Rect::new(self.center.x - s * 0.5, self.center.y - s * 0.5, s, s)
    }

    /// Görsel varsa çağıranın çizeceği daire kutusu.
    pub fn image_rect(&self, theme: &Theme) -> Option<Rect> {
        self.image.is_some().then(|| self.rect(theme))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let r = self.rect(theme);
        let radii = Corners::uniform(r.width * 0.5);
        let ready = self.image.as_ref().filter(|t| t.is_ready()).cloned();
        if let Some(tex) = &ready {
            ui.painter.image(r, tex, &ImageStyle::new(ImageFit::Cover).with_radii(radii).with_opacity(ui.alpha()));
        } else {
            // Görsel yokken ya da yüklenirken: baş harfler.
            let bg = self.background(theme);
            ui.painter.rect(r, &RectStyle::filled(ui.fade(bg), radii));
            let m = self.initials.measure_in(ui.painter);
            self.initials.draw(ui.painter, Point::new(r.center().x - m.width * 0.5, r.center().y - m.height * 0.5), ui.fade(content_on(theme, bg)));
        }
        let pending = self.image.is_some() && ready.is_none();
        if self.image_pending && !pending {
            self.damage.mark();
        }
        self.image_pending = pending;
        let k = self.presence_scale.value(now).max(0.0);
        if k > 0.01 {
            let d = r.width * 0.3 * k;
            // Dairenin 45° noktasına oturur, zemin rengiyle kesik halka.
            let c = Point::new(r.center().x + r.width * 0.354, r.center().y + r.width * 0.354);
            let dot = Rect::new(c.x - d * 0.5, c.y - d * 0.5, d, d);
            let style = RectStyle {
                fill: ui.fade(theme.colors.primary),
                radii: Corners::uniform(d * 0.5),
                border: Some(Border { width: FOCUS_RING, color: ui.fade(theme.colors.surface_container) }),
            };
            ui.painter.rect(dot, &style);
        }
        let animating = !self.presence_scale.is_settled(now) | pending;
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, r.expand(r.width * 0.2), animating);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_is_redacted() {
        let theme = Theme::default();
        crate::redact::assert_redacted(&Avatar::new(&theme, "gizli Kişi", AvatarSize::Medium));
    }

    #[test]
    fn initials_from_names() {
        assert_eq!(initials("Ayşe Yılmaz"), "AY");
        assert_eq!(initials("nuxut"), "N");
        assert_eq!(initials("  ışık  deniz   öz "), "IÖ");
        assert_eq!(initials(""), "");
        assert_eq!(initials("(Deniz) Kaya"), "DK");
    }

    #[test]
    fn xlarge_side_and_initials_scale() {
        let theme = Theme::default();
        assert_eq!(AvatarSize::XLarge.side(&theme), 96.0);
        assert_eq!(AvatarSize::XLarge.side(&theme), theme.space(24));
        // Mevcut boylar değişmedi.
        assert_eq!(AvatarSize::Small.text_style(&theme), text_style(&theme, TextRole::Caption));
        assert_eq!(AvatarSize::Medium.text_style(&theme), text_style(&theme, TextRole::Label));
        assert_eq!(AvatarSize::Large.text_style(&theme), text_style(&theme, TextRole::Title));
        // XLarge: Large'ın aile/ağırlığı, dairenin oranında büyük metin.
        let (l, xl) = (AvatarSize::Large.text_style(&theme), AvatarSize::XLarge.text_style(&theme));
        assert_eq!((xl.family.as_str(), xl.weight), (l.family.as_str(), l.weight));
        let ratio = |s: AvatarSize, t: &TextStyle| t.size / s.side(&theme);
        assert!((ratio(AvatarSize::XLarge, &xl) - ratio(AvatarSize::Large, &l)).abs() < 0.01);
        assert!(xl.size > l.size);
        // Tema yazı boyutunu izler.
        let mut big = theme.clone();
        big.typography.size *= 1.5;
        assert!(AvatarSize::XLarge.text_style(&big).size > xl.size);
    }

    #[test]
    fn background_is_stable_and_varies() {
        let theme = Theme::default();
        let a = Avatar::new(&theme, "Ayşe", AvatarSize::Medium);
        let b = Avatar::new(&theme, "Mehmet", AvatarSize::Medium);
        assert_eq!(a.background(&theme), Avatar::new(&theme, "Ayşe", AvatarSize::Small).background(&theme));
        assert_ne!(a.background(&theme), b.background(&theme));
    }
}
