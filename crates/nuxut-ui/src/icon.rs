//! İkonlar (ADR-0006): semantik ad (`Icon::named(IconName::Wifi)`) seçili ikon setinin glifiyle
//! çizilir, ya da GPU dokusu (`Icon::image` — uygulama/tepsi pixmap'i, dosya ikonu;
//! `Icon::symbolic` — tek renkli ikon, içerik rengiyle boyanır).
//!
//! Dolu/çizgi: `material` setinde değişken fontun `FILL` ekseni; `nerd` setinde çizgi ve dolu
//! glif arasında opaklık geçişi. [`IconView::set_filled`] dolguyu `effects` hareketiyle geçirir
//! (ör. seçilen ikon dolar). Dokulu ikonda etkisizdir.
//!
//! İkon fontu kurulu değilse: `material` → kurulu Nerd Font varsa `nerd` seti; hiçbiri yoksa
//! ikon çizilmez (ligatür adı metin olarak görünmez), kutu boyutu korunur.

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Color, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{ImageFit, ImageStyle, OpticalCenter, Painter, TextSystem, Texture};
use nuxut_theme::{IconSet, Theme};

use crate::access::{AccessNode, AccessTree, Role};

use crate::icon_names::{IconName, glyph};
use crate::label::Label;
use crate::style::clip_text;
use crate::typography::{TextRole, text_style};

/// Dolgu bu adımlarla yeniden şekillendirilir (her karede değil; glyph önbellek anahtarı
/// dolguya bağlı).
const FILL_STEPS: f32 = 10.0;

/// Bir ikon kaynağı.
#[derive(Clone, PartialEq)]
pub enum Icon {
    /// Semantik ad; glif ikon setinden gelir. `None`: bilinmeyen ad (boş çizilir).
    Named(Option<IconName>),
    /// Doku. `symbolic`: tek renkli; alfa maskesi içerik rengiyle boyanır.
    Image { texture: Texture, symbolic: bool },
    /// Metin glifi (emoji, tek harf, sembol): ikon kutusunda, arayüz fontuyla (yoksa sistem
    /// yedeği; renkli emoji kendi renkleriyle) mürekkebi ortalanmış çizilir. Glif kutudan
    /// genişse kutuya sığacak kadar küçültülür. En fazla [`MAX_GLYPH_BYTES`] bayt; boş dize
    /// boş çizilir (kutu korunur). Kurucu: [`Icon::glyph`].
    Glyph(String),
}

// Glif metni kullanıcı içeriği olabilir (son kullanılan emojiler): Debug yalnızca uzunluk yazar.
impl std::fmt::Debug for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Named(name) => f.debug_tuple("Named").field(name).finish(),
            Self::Image { texture, symbolic } => {
                f.debug_struct("Image").field("texture", texture).field("symbolic", symbolic).finish()
            }
            Self::Glyph(text) => f.debug_tuple("Glyph").field(&crate::redact::Len(text)).finish(),
        }
    }
}

/// [`Icon::Glyph`] metninin en uzun hâli (bayt; ZWJ'li aile/bayrak emojileri sığar).
pub const MAX_GLYPH_BYTES: usize = 64;

impl Icon {
    pub fn named(name: IconName) -> Self {
        Self::Named(Some(name))
    }

    /// Geçiş kısa yolu: semantik ad ya da eski Material ligatür adı (`"volume_up"`) dizesi.
    /// Bilinmeyen ad boş çizilir. Yeni kod [`Icon::named`] kullanır.
    pub fn symbol(name: &str) -> Self {
        Self::Named(name.parse().ok())
    }

    /// Renkli görsel ikon (kendi renkleriyle çizilir).
    pub fn image(texture: Texture) -> Self {
        Self::Image { texture, symbolic: false }
    }

    /// Tek renkli (sembolik) ikon: temanın içerik rengiyle boyanır.
    pub fn symbolic(texture: Texture) -> Self {
        Self::Image { texture, symbolic: true }
    }

    /// İkon yerine metin glifi ([`Icon::Glyph`]; ör. emoji seçicide `"😀"`). Uzun girdi
    /// karakter sınırında [`MAX_GLYPH_BYTES`]'a kırpılır.
    pub fn glyph(text: &str) -> Self {
        Self::Glyph(clip_text(text, MAX_GLYPH_BYTES).to_owned())
    }
}

impl From<IconName> for Icon {
    fn from(name: IconName) -> Self {
        Self::named(name)
    }
}

/// Glif ikonunun hangi setle çizildiği (font kurulu mu, ilk ölçümde belirlenir).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolved {
    Unknown,
    Set(IconSet),
    /// Ne setin fontu ne yedek kurulu: çizilmez.
    Hidden,
}

/// Önbellekli çizilebilir ikon.
#[derive(Debug, Clone)]
pub struct IconView {
    icon: Icon,
    /// Glif ikonu için şekillendirilmiş etiket (`nerd`'de çizgi glifi); dokuda yalnızca boyut kaynağı.
    label: Label,
    /// `nerd` setinde dolu glif (çizgiyle aynıysa `None`).
    solid: Option<Box<Label>>,
    role: TextRole,
    fill: Animated<f32>,
    /// Layout'un şekillendirildiği (nicelenmiş) dolgu (`material`).
    shaped_fill: f32,
    /// Son `animate`'teki dolgu (`nerd` opaklık geçişi).
    fill_now: f32,
    /// Temanın seçtiği set ve aileleri.
    theme_set: IconSet,
    material_family: String,
    nerd_family: String,
    resolved: Resolved,
    /// [`Icon::Glyph`] için arayüz fontu ailesi.
    glyph_family: String,
    /// Kutu kenarı: rolün font boyutu ya da `with_size` (glif küçültülse de değişmez).
    slot: f32,
    /// `with_size`: rol boyutunun yerine sabit kutu kenarı (dosya ızgarası gibi büyük ikonlar).
    size_override: Option<f32>,
}

impl IconView {
    pub fn new(theme: &Theme, icon: Icon) -> Self {
        Self::with_role(theme, icon, TextRole::Icon)
    }

    /// `role`: [`TextRole::Icon`] (varsayılan) ya da [`TextRole::IconSmall`] / [`TextRole::IconLarge`].
    pub fn with_role(theme: &Theme, icon: Icon, role: TextRole) -> Self {
        let mut view = Self {
            icon,
            label: Label::new("", text_style(theme, role)),
            solid: None,
            role,
            fill: Animated::new(0.0, theme.motion.effects),
            shaped_fill: 0.0,
            fill_now: 0.0,
            theme_set: theme.typography.icon_set,
            material_family: String::new(),
            nerd_family: String::new(),
            resolved: Resolved::Unknown,
            glyph_family: String::new(),
            slot: 0.0,
            size_override: None,
        };
        view.adopt_theme(theme);
        view
    }

    pub fn role(&self) -> TextRole {
        self.role
    }

    /// Rol boyutu yerine sabit kutu kenarı (mantıksal px; glif bu boyda şekillenir, doku bu
    /// kutuya sığar). Rolün ailesi korunur; tema değişince de geçerli kalır. Çağıran boyu
    /// tema ölçüsünden türetir (ör. `theme.space(12)`).
    pub fn with_size(mut self, theme: &Theme, size: f32) -> Self {
        self.size_override = Some(size.max(1.0));
        self.adopt_theme(theme);
        self
    }

    /// Boyut rolünü değiştirir (ikon, dolgu ve set çözümü korunur). Değiştiyse `true`.
    pub fn set_role(&mut self, theme: &Theme, role: TextRole) -> bool {
        if self.role == role {
            return false;
        }
        self.role = role;
        self.adopt_theme(theme);
        true
    }

    /// Başlangıç dolgusu (animasyonsuz).
    pub fn filled(mut self, filled: bool) -> Self {
        let f = if filled { 1.0 } else { 0.0 };
        self.fill.jump_to(f);
        self.fill_now = f;
        self.reshape(f);
        self
    }

    pub fn icon(&self) -> &Icon {
        &self.icon
    }

    /// İkon değiştiyse `true`.
    pub fn set_icon(&mut self, icon: Icon) -> bool {
        if self.icon == icon {
            return false;
        }
        self.icon = icon;
        self.refresh_glyphs();
        true
    }

    /// Dolu/çizgi geçişi (`effects` hareketiyle).
    pub fn set_filled(&mut self, filled: bool, now: Instant) {
        self.fill.set_target(if filled { 1.0 } else { 0.0 }, now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.fill.set_motion(theme.motion.effects, now);
        self.adopt_theme(theme);
    }

    fn adopt_theme(&mut self, theme: &Theme) {
        let t = &theme.typography;
        let set = t.icon_set;
        let material = if set == IconSet::Material { t.icon_family() } else { nuxut_theme::MATERIAL_SYMBOLS_FAMILY };
        let nerd = if set == IconSet::Nerd { t.icon_family() } else { t.mono.family.as_str() };
        if (set, material, nerd) != (self.theme_set, self.material_family.as_str(), self.nerd_family.as_str()) {
            self.theme_set = set;
            self.material_family = material.to_owned();
            self.nerd_family = nerd.to_owned();
            self.resolved = Resolved::Unknown;
        }
        let style = text_style(theme, self.role);
        t.ui.family.clone_into(&mut self.glyph_family);
        self.slot = self.size_override.unwrap_or(style.size);
        self.label.set_style(style.with_fill(self.shaped_fill));
        self.refresh_glyphs();
    }

    /// Çizimde kullanılacak set (çözülmemişse temanınki).
    fn set(&self) -> IconSet {
        match self.resolved {
            Resolved::Set(s) => s,
            Resolved::Unknown | Resolved::Hidden => self.theme_set,
        }
    }

    /// Etiket metinlerini ve ailelerini ad + sete göre kurar.
    fn refresh_glyphs(&mut self) {
        if let Icon::Glyph(text) = &self.icon {
            // Kutu boyunda, arayüz fontu (yedek zinciri renkli emojiyi bulur), mürekkep ortalı.
            // Küçültme gerekirse ilk ölçümde (`fit_glyph`).
            let mut style = self.label.style().clone();
            style.family.clone_from(&self.glyph_family);
            style.size = self.slot;
            style.fill = 0.0;
            style.optical = OpticalCenter::Ink;
            self.label.set_style(style);
            self.label.set_text(clip_text(text, MAX_GLYPH_BYTES));
            self.solid = None;
            return;
        }
        let set = self.set();
        let name = match self.icon {
            Icon::Named(name) if self.resolved != Resolved::Hidden => name,
            _ => None,
        };
        let family = match set {
            IconSet::Material => &self.material_family,
            IconSet::Nerd => &self.nerd_family,
        };
        let mut style = self.label.style().clone();
        // Glifden ada dönüldüyse glif ayarları geri alınır.
        let (size, optical, fill) = (self.slot, OpticalCenter::Auto, self.shaped_fill);
        if style.family != *family || style.size != size || style.optical != optical || style.fill != fill {
            style.family = family.clone();
            (style.size, style.optical, style.fill) = (size, optical, fill);
            self.label.set_style(style.clone());
        }
        self.label.set_text(name.map_or("", |n| glyph(set, n, false)));
        self.solid = name.filter(|_| set == IconSet::Nerd).and_then(|n| {
            let solid = glyph(set, n, true);
            (solid != glyph(set, n, false)).then(|| Box::new(Label::new(solid, style.clone().with_fill(0.0))))
        });
    }

    /// İlk ölçümde ikon fontunun kurulu olup olmadığına göre seti belirler.
    fn resolve(&mut self, text: &mut TextSystem) {
        if self.resolved != Resolved::Unknown || !matches!(self.icon, Icon::Named(_)) {
            return;
        }
        let preferred = match self.theme_set {
            IconSet::Material => &self.material_family,
            IconSet::Nerd => &self.nerd_family,
        };
        self.resolved = if text.has_family(preferred) {
            Resolved::Set(self.theme_set)
        } else if self.theme_set == IconSet::Material && text.has_family(&self.nerd_family) {
            Resolved::Set(IconSet::Nerd)
        } else {
            Resolved::Hidden
        };
        self.refresh_glyphs();
    }

    /// Glif kutudan genişse (renkli emojinin ilerlemesi font boyundan büyüktür) kutuya
    /// sığacak kadar küçültür. Küçültülmüş glif tekrar ölçülünce sığar: bir kez çalışır.
    fn fit_glyph(&mut self, measured: Size) {
        if !matches!(self.icon, Icon::Glyph(_)) || measured.width <= self.slot + 0.5 || measured.width <= 0.0 {
            return;
        }
        let mut style = self.label.style().clone();
        style.size = (style.size * self.slot / measured.width).floor().max(1.0);
        self.label.set_style(style);
    }

    fn reshape(&mut self, fill: f32) {
        if self.set() != IconSet::Material || matches!(self.icon, Icon::Glyph(_)) {
            return;
        }
        let q = (fill.clamp(0.0, 1.0) * FILL_STEPS).round() / FILL_STEPS;
        if q != self.shaped_fill {
            self.shaped_fill = q;
            let style = self.label.style().clone().with_fill(q);
            self.label.set_style(style);
        }
    }

    /// Dolgu animasyonunu ilerletir; sürüyorsa `true` (sahibi kare istemeli).
    pub fn animate(&mut self, now: Instant) -> bool {
        let v = self.fill.value(now);
        self.fill_now = v.clamp(0.0, 1.0);
        self.reshape(v);
        !self.fill.is_settled(now)
    }

    /// Doku henüz GPU'ya yüklenmediyse `true` (sahibi bir kare daha istemeli).
    pub fn is_pending(&self) -> bool {
        matches!(&self.icon, Icon::Image { texture, .. } if !texture.is_ready())
    }

    /// İkonun kutusu: kare, kenar = rolün font boyutu (glif ya da doku boyutu ne olursa olsun).
    pub fn size(&self) -> Size {
        Size::new(self.slot, self.slot)
    }

    pub fn measure(&mut self, text: &mut TextSystem) -> Size {
        match self.icon {
            Icon::Named(_) => {
                self.resolve(text);
                self.label.measure(text);
            }
            Icon::Glyph(_) => {
                let m = self.label.measure(text);
                self.fit_glyph(m);
            }
            Icon::Image { .. } => {}
        }
        self.size()
    }

    /// `center` etrafında ortalanmış çizer. `color`: glif/sembolik ikonun rengi; renkli
    /// dokuda yalnızca opaklığı (solma) kullanılır.
    pub fn draw_centered(&mut self, painter: &mut Painter, center: Point, color: Color) {
        match &self.icon {
            Icon::Named(_) => {
                self.resolve(painter.text_system());
                if self.label.text().is_empty() {
                    return;
                }
                let at = |label: &mut Label, painter: &mut Painter| {
                    let m = label.measure_in(painter);
                    Point::new(center.x - m.width * 0.5, center.y - m.height * 0.5)
                };
                match &mut self.solid {
                    // nerd: çizgi ve dolu glif arasında opaklık geçişi.
                    Some(solid) => {
                        let f = self.fill_now;
                        if f < 1.0 {
                            let p = at(&mut self.label, painter);
                            self.label.draw(painter, p, color.with_alpha(color.a * (1.0 - f)));
                        }
                        if f > 0.0 {
                            let p = at(solid.as_mut(), painter);
                            solid.draw(painter, p, color.with_alpha(color.a * f));
                        }
                    }
                    None => {
                        let p = at(&mut self.label, painter);
                        self.label.draw(painter, p, color);
                    }
                }
            }
            Icon::Image { texture, symbolic } => {
                let s = self.size();
                let rect = Rect::new(center.x - s.width * 0.5, center.y - s.height * 0.5, s.width, s.height);
                let style = ImageStyle::new(ImageFit::Contain).with_opacity(color.a);
                let style = if *symbolic { style.with_tint(color.with_alpha(1.0)) } else { style };
                painter.image(rect, texture, &style);
            }
            Icon::Glyph(_) => {
                if self.label.text().is_empty() {
                    return;
                }
                let m = self.label.measure_in(painter);
                self.fit_glyph(m);
                let m = self.label.measure_in(painter);
                self.label.draw(painter, Point::new(center.x - m.width * 0.5, center.y - m.height * 0.5), color);
            }
        }
    }
}

/// İkon yer tutucusu: ikon kutusu (`size` kenarlı kare, `center` ortalı) boyunda hafif
/// yuvarlak kare. Yüklenmemiş ya da hatalı ikonun yerini tutar; yerleşim ikon gelince
/// zıplamaz. `color` çağıranın solmasıyla ([`crate::style::icon_placeholder`]).
pub(crate) fn draw_icon_placeholder(painter: &mut Painter, theme: &Theme, center: Point, size: f32, color: Color) {
    let rect = Rect::new(center.x - size * 0.5, center.y - size * 0.5, size, size);
    let radius = theme.shape.radius_small.min(size * 0.5) * 0.5;
    painter.rect(rect, &nuxut_render::RectStyle::filled(color, nuxut_core::Corners::uniform(radius)));
}

// Erişilebilirlik (ADR-0011).
impl IconView {
    /// Anlam taşıyan ikon (durum simgesi, pil): `Role::Image`, yalnız ad — çağıranın
    /// `t!()` ile verdiği `label` (glif/ligatür adı okunmaz). Süs ikonu (düğme/satır içindeki)
    /// için çağrılmaz: ağaçta yer almaz.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash, label: &str, bounds: Rect) -> AccessNode<'t> {
        tree.push(key, Role::Image).label(label).bounds(bounds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_constructor_clips_on_char_boundary() {
        assert_eq!(Icon::glyph("😀"), Icon::Glyph("😀".to_owned()));
        let long = "😀".repeat(40);
        let Icon::Glyph(g) = Icon::glyph(&long) else { panic!("glif bekleniyordu") };
        assert!(g.len() <= MAX_GLYPH_BYTES && g.chars().all(|c| c == '😀'));
    }

    #[test]
    fn glyph_uses_icon_slot_and_ui_font() {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let named = IconView::new(&theme, Icon::named(IconName::Search));
        let mut view = IconView::new(&theme, Icon::glyph("😀"));
        assert_eq!(view.size(), named.size(), "glif ikon kutusunu kullanır");
        assert_eq!(view.label.style().family, theme.typography.ui.family);
        assert_eq!(view.label.style().optical, OpticalCenter::Ink);
        assert_eq!(view.label.text(), "😀");
        assert!(view.solid.is_none());
        // Kutudan geniş glif sığacak kadar küçülür; kutu değişmez.
        let mut wide = IconView::new(&theme, Icon::glyph("WWWW"));
        let slot = wide.size();
        assert_eq!(wide.measure(&mut text), slot);
        let m = wide.label.measure(&mut text);
        assert!(m.width <= slot.width + 0.5, "{} > {}", m.width, slot.width);
        assert!(wide.label.style().size < slot.width);
        // Rol değişince kutu ve glif rolü izler.
        assert!(view.set_role(&theme, TextRole::IconLarge));
        assert_eq!(view.size().width, text_style(&theme, TextRole::IconLarge).size);
        assert_eq!(view.label.style().family, theme.typography.ui.family);
        // Dolgu animasyonu glifte etkisiz.
        view.reshape(1.0);
        assert_eq!(view.label.style().fill, 0.0);
    }

    #[test]
    fn switching_between_glyph_and_name_restores_style() {
        let theme = Theme::default();
        let mut view = IconView::new(&theme, Icon::named(IconName::Search));
        let before = view.label.style().clone();
        let mut wide = IconView::new(&theme, Icon::glyph("WWWW"));
        wide.measure(&mut TextSystem::new());
        assert!(wide.set_icon(Icon::named(IconName::Search)));
        assert_eq!(wide.label.style(), &before, "addan glife ve geri: ikon stili aynen");
        assert!(view.set_icon(Icon::glyph("")));
        assert!(view.label.text().is_empty(), "boş glif boş çizilir");
        assert_eq!(view.size().width, before.size);
    }
}
