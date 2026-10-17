//! Ekran göstergesi (OSD): ses/parlaklık değişince kenardan blob olarak büyüyen küçük kapsül —
//! ikon + ilerleme. Menu ile aynı hareket (`grow`): shell kapsülün şeklini
//! [`Osd::shape`] ile kendi blob alanına ekler (çerçeveden çıkar), içeriği
//! [`Osd::draw_content`] çizer; bağımsız kullanımda [`Osd::draw`].
//!
//! Değer `spatial_fast` yayla akar; gizlenme zaman aşımı çağıranın zamanlayıcısıdır
//! (her `show`'da yeniden kurar, dolunca `hide`). İkon değere göre değişebilir
//! (`IconName::volume`, `IconName::brightness`).
//!
//! Aşım ([`Osd::set_max`], ör. %150 ses): ray `0..max`'ı gösterir; %100'de raydan taşan ince
//! işaret (dolgunun arkasında). Dolgunun %100'ü aşan kısmı `error` tonunda (yayla belirir).
//! İki parça tek kapsüldür: aynı hap dolgu çizilir, aşım tonu yalnız işaretin sağına kırpılarak
//! üstüne biner.
//!
//! Sessiz ([`Osd::set_muted`]): içerik (ikon, ray, yüzde) `DISABLED_CONTENT` opaklığına `effects`
//! hareketiyle soluklaşır (kesilebilir; hareket kapalıyken anında). Kapsülün şekli solmaz.

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::BlobShape;
use nuxut_theme::Theme;

use crate::grow::Grow;
use crate::icon::{Icon, IconView};
use crate::icon_names::IconName;
use crate::menu::MenuAnchor;
use crate::bar::{Overflow, PillBar};
use crate::style::DISABLED_CONTENT;
use crate::label::Label;
use crate::typography::{TextRole, text_style};
use crate::ui::{DamageState, Ui};

/// [`Osd::set_max`] üst sınırı (%200).
pub const OSD_MAX_OVERFLOW: f32 = 2.0;
/// [`Osd::set_value_text`] metninin en uzun hâli (bayt).
pub const OSD_MAX_VALUE_TEXT: usize = 32;

#[derive(Debug, Clone)]
pub struct Osd {
    grow: Grow,
    icon: IconView,
    value: Animated<f32>,
    /// Rayın temsil ettiği üst değer (1..=2); değişince ölçek yayla akar.
    max: Animated<f32>,
    /// Aşım tonunun karışımı: 0 = değer ≤ 1, 1 = aşımda.
    over: Animated<f32>,
    /// Sessizlik soluklaşması: 0 = tam opak içerik, 1 = `DISABLED_CONTENT`.
    muted: Animated<f32>,
    /// Hedef yüzde ("64"; tabular rakamlar, genişlik "100"e göre ayrılır).
    percent: Label,
    percent_width: Option<f32>,
    /// Çağıranın değer metni (ör. büyüteç "2,0×"); `None` = yüzde.
    value_text: Option<String>,
    damage: DamageState,
}

impl Osd {
    pub fn new(theme: &Theme) -> Self {
        Self {
            grow: Grow::new(theme),
            icon: IconView::with_role(theme, Icon::named(IconName::VolumeHigh), TextRole::Icon),
            value: Animated::new(0.0, theme.motion.spatial_fast),
            max: Animated::new(1.0, theme.motion.spatial_fast),
            over: Animated::new(0.0, theme.motion.effects),
            muted: Animated::new(0.0, theme.motion.effects),
            percent: Label::new("", Self::percent_style(theme)),
            percent_width: None,
            value_text: None,
            damage: DamageState::new(),
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.grow.set_theme(theme, now);
        self.icon.set_theme(theme, now);
        self.value.set_motion(theme.motion.spatial_fast, now);
        self.max.set_motion(theme.motion.spatial_fast, now);
        self.over.set_motion(theme.motion.effects, now);
        self.muted.set_motion(theme.motion.effects, now);
        self.percent.set_style(Self::percent_style(theme));
        self.percent_width = None;
        self.damage.mark();
    }

    fn percent_style(theme: &Theme) -> nuxut_render::TextStyle {
        text_style(theme, TextRole::Label).with_tabular_numbers().with_max_lines(1)
    }

    /// Kapsülün boyutu: `space(64) × space(12)`.
    pub fn size(theme: &Theme) -> Size {
        Size::new(theme.space(64), theme.space(12))
    }

    pub fn is_open(&self) -> bool {
        self.grow.is_open()
    }

    pub fn is_visible(&self) -> bool {
        self.grow.is_visible()
    }

    /// Gösterir ya da (açıksa) yalnızca değeri ve ikonu günceller. `anchor`: kapsülün çıktığı
    /// kenar (ör. çerçevenin alt kenarı ortası, `gap <= 0` → kenara yapışık blob).
    /// `value` 0..=[`max`](Self::set_max) (1 = %100).
    pub fn show(&mut self, theme: &Theme, icon: Icon, value: f32, anchor: MenuAnchor, bounds: Rect, now: Instant) {
        let v = if value.is_finite() { value.clamp(0.0, self.max.target()) } else { 0.0 };
        let over = if v > 1.0 { 1.0 } else { 0.0 };
        if !self.grow.is_open() {
            // Kapalıyken değer ilk karede yerinde başlar (kapsül büyürken dolmaz).
            self.value.jump_to(v);
            self.muted.jump_to(self.muted.target());
            self.max.jump_to(self.max.target());
            self.over.jump_to(over);
            self.grow.open(anchor, bounds, Self::size(theme), now);
        } else {
            self.value.set_target(v, now);
            self.over.set_target(over, now);
        }
        self.icon.set_icon(icon);
        self.refresh_label();
        self.damage.mark();
    }

    /// Değer etiketi: `Some` → yüzde yerine bu metin (ör. büyüteç "2,0×"; en çok
    /// [`OSD_MAX_VALUE_TEXT`] bayt, tek satır); `None` → yüzde (varsayılan). Ray yine `show`'un
    /// 0..=max değerini gösterir. Etiket alanı "100" ile bu metnin genişinden ayrılır. Sonraki
    /// `show`'larda da geçerlidir; değiştiyse `true`.
    pub fn set_value_text(&mut self, text: Option<String>) -> bool {
        let text = text.map(|t| crate::style::clip_text(&t, OSD_MAX_VALUE_TEXT).to_owned());
        if self.value_text == text {
            return false;
        }
        self.value_text = text;
        self.percent_width = None;
        self.refresh_label();
        self.damage.mark();
        true
    }

    pub fn value_text(&self) -> Option<&str> {
        self.value_text.as_deref()
    }

    /// Etiketi hedef değerden (ya da çağıranın metninden) yazar.
    fn refresh_label(&mut self) {
        match &self.value_text {
            Some(t) => self.percent.set_text(t),
            None => self.percent.set_text(&format!("{}", (self.value.target() * 100.0).round() as u16)),
        };
    }

    /// Rayın üst değeri: 1..=[`OSD_MAX_OVERFLOW`] (geçersiz → 1; varsayılan 1). Açıkken ölçek
    /// yayla akar (dolgu zıplamaz); değer yeni aralığa kırpılır.
    pub fn set_max(&mut self, max: f32, now: Instant) {
        let max = if max.is_finite() { max.clamp(1.0, OSD_MAX_OVERFLOW) } else { 1.0 };
        if max == self.max.target() {
            return;
        }
        if self.grow.is_open() {
            self.max.set_target(max, now);
        } else {
            self.max.jump_to(max);
        }
        let v = self.value.target();
        if v > max {
            if self.grow.is_open() {
                self.value.set_target(max, now);
            } else {
                self.value.jump_to(max);
            }
            self.refresh_label();
            let over = if max > 1.0 { 1.0 } else { 0.0 };
            if self.grow.is_open() {
                self.over.set_target(over, now);
            } else {
                self.over.jump_to(over);
            }
        }
        self.damage.mark();
    }

    pub fn max(&self) -> f32 {
        self.max.target()
    }

    /// Sessiz durumu: içerik `DISABLED_CONTENT` opaklığına yayla soluklaşır (açılınca geri gelir).
    /// Kapalıyken (görünmezken) anında geçer; `show`'dan önce çağrılırsa kapsül ilk karesinden soluk
    /// açılır (açılış anında bekleyen soluklaşma da oturtulur).
    /// Değişiklik yoksa hiçbir şey yapmaz (kare istemez).
    pub fn set_muted(&mut self, muted: bool, now: Instant) {
        let target = if muted { 1.0 } else { 0.0 };
        if self.muted.target() == target {
            return;
        }
        if self.grow.is_visible() {
            self.muted.set_target(target, now);
        } else {
            self.muted.jump_to(target);
        }
        self.damage.mark();
    }

    pub fn is_muted(&self) -> bool {
        self.muted.target() > 0.5
    }

    /// Bu karedeki içerik opaklığı çarpanı (sessizlik soluklaşması). Sessiz hâl devre dışı gibi
    /// ele alınır: yüksek karşıtlıkta en soluk içerik (`on_surface_variant` yüzde) yine ≥ 3:1
    /// kalır (`style::disabled_content`).
    fn content_alpha(&self, now: Instant, theme: &Theme) -> f32 {
        let m = self.muted.value(now).clamp(0.0, 1.0);
        let floor = if theme.contrast.is_high() {
            crate::style::disabled_content(theme, theme.colors.on_surface_variant, &[]).a.max(DISABLED_CONTENT)
        } else {
            DISABLED_CONTENT
        };
        1.0 + (floor - 1.0) * m
    }

    pub fn hide(&mut self, now: Instant) {
        if self.grow.close(now) {
            self.damage.mark();
        }
    }

    /// Blob şekli (shell kendi alanına ekler; karede bir kez). Kapalıyken `None`.
    pub fn shape(&mut self, now: Instant, theme: &Theme) -> Option<BlobShape> {
        self.grow.shape(now, Self::size(theme).height * 0.5)
    }

    pub fn input_rect(&self) -> Option<Rect> {
        None
    }

    /// Yalnızca içerik (şekil shell'in alanında).
    pub fn draw_content(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let shape = self.shape(ui.now, theme);
        self.draw_inner(ui, shape);
    }

    /// Bağımsız: şekil (gölgeli) + içerik.
    pub fn draw(&mut self, ui: &mut Ui) {
        let shape = self.shape(ui.now, ui.theme);
        self.grow.draw_standalone(ui, shape);
        self.draw_inner(ui, shape);
    }

    fn draw_inner(&mut self, ui: &mut Ui, shape: Option<BlobShape>) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let Some(shape) = shape else {
            self.damage.report_hidden(ui);
            return;
        };
        let visual = shape.visual_rect();
        let reveal = self.grow.reveal(visual);
        if reveal > 0.0 && visual.width > 1.0 && visual.height > 1.0 {
            let target = self.grow.target();
            ui.painter.push_clip(visual);
            ui.with_alpha(reveal * self.content_alpha(now, theme), |ui| {
                let pad = theme.space(4);
                let iw = self.icon.size().width;
                let mid = target.center().y;
                self.icon.draw_centered(ui.painter, Point::new(target.x + pad + iw * 0.5, mid), ui.fade(c.on_surface));
                // İlerleme: kalın hap ray, `primary` dolum.
                let x0 = target.x + pad + iw + theme.space(3);
                // Yüzde sağda, sabit genişlikte (rakam değişince ray oynamaz).
                // Çağıranın metni varsa alan onun genişliğine de yeter.
                let custom = self.value_text.is_some();
                let pw = match self.percent_width {
                    Some(w) => w,
                    None => {
                        let base = Label::new("100", Self::percent_style(theme)).measure_in(ui.painter).width;
                        let w = if custom { base.max(self.percent.measure_in(ui.painter).width) } else { base };
                        *self.percent_width.insert(w)
                    }
                };
                let ps = self.percent.measure_in(ui.painter);
                let px = target.right() - pad - ps.width;
                self.percent.draw(ui.painter, Point::new(px, mid - ps.height * 0.5), ui.fade(c.on_surface_variant));
                let w = (target.right() - pad - pw - theme.space(3) - x0).max(0.0);
                let h = theme.space(2);
                let track = Rect::new(x0, mid - h * 0.5, w, h);
                let b = bar(w, h, self.value.value(now), self.max.value(now));
                let over = self.over.value(now).clamp(0.0, 1.0);
                PillBar {
                    track,
                    horizontal: true,
                    fill: b.fill,
                    rest: ui.fade(crate::style::track(theme)),
                    fill_color: ui.fade(c.primary),
                    overflow: b.mark.map(|mark| Overflow {
                        mark,
                        tick: ui.fade(c.outline),
                        color: ui.fade(c.primary.lerp(&c.error, over)),
                    }),
                }
                .draw(ui.painter);
            });
            ui.painter.pop_clip();
        }
        let animating = self.grow.is_animating(now)
            | !self.value.is_settled(now)
            | !self.max.is_settled(now)
            | !self.over.is_settled(now)
            | !self.muted.is_settled(now)
            | self.icon.animate(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, visual.expand(1.0), animating);
        self.grow.settle(now);
    }
}

/// Ray geometrisi (ray başından x): dolgu genişliği ve `max > 1` iken %100 işareti.
/// Aşım bölgesi `mark..fill`'dir (`fill > mark` iken).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Bar {
    fill: f32,
    mark: Option<f32>,
}

fn bar(width: f32, height: f32, value: f32, max: f32) -> Bar {
    let max = if max.is_finite() { max.max(1.0) } else { 1.0 };
    let v = if value.is_finite() { value.clamp(0.0, max) } else { 0.0 };
    // Hap en az kendi yüksekliği kadar (yuvarlak uç).
    let fill = if v > 0.0 { (width * v / max).max(height) } else { 0.0 };
    // max 1'e yayla dönerken işaret ray ucuna varınca kaybolur.
    let mark = (max > 1.0 + 1e-3).then(|| width / max);
    Bar { fill, mark }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use nuxut_render::{Recorder, TextSystem};

    fn shown(theme: &Theme, t0: Instant) -> Osd {
        let mut o = Osd::new(theme);
        show_value(&mut o, theme, 0.5, t0);
        o
    }

    fn show_value(o: &mut Osd, theme: &Theme, v: f32, now: Instant) {
        let anchor = MenuAnchor { side: crate::menu::MenuSide::Above, ..MenuAnchor::below(Rect::new(500.0, 790.0, 0.0, 0.0)) };
        o.show(theme, Icon::named(IconName::VolumeHigh), v, anchor, Rect::new(0.0, 0.0, 1000.0, 800.0), now);
    }

    /// Çağıranın değer metni yüzdenin yerine geçer (ray değeri aynı), sonraki `show`'larda
    /// kalır, alanı metne göre genişler; `None` yüzdeye döner.
    #[test]
    fn custom_value_text_replaces_percent() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut o = shown(&theme, t0);
        assert_eq!(o.percent.text(), "50");
        assert!(o.set_value_text(Some("2,0×".into())));
        assert!(!o.set_value_text(Some("2,0×".into())), "aynı metin");
        assert_eq!(o.percent.text(), "2,0×");
        show_value(&mut o, &theme, 0.25, t0);
        assert_eq!(o.percent.text(), "2,0×", "show metni ezmez");
        assert_eq!(o.value.target(), 0.25, "ray çağıranın ilerlemesi");
        assert_eq!(o.value_text(), Some("2,0×"));
        // Alan: "100"den geniş metinde metnin genişliği.
        let wide = "12,5× büyütme";
        o.set_value_text(Some(wide.into()));
        let mut text = TextSystem::new();
        let mut rec = Recorder::new();
        let draw = |o: &mut Osd, text: &mut TextSystem, rec: &mut Recorder| {
            rec.record(text, (1000, 800), 1.0, |p| {
                let mut ui = Ui::new(p, &theme, t0 + Duration::from_secs(5));
                o.draw(&mut ui);
            });
        };
        draw(&mut o, &mut text, &mut rec);
        let base = Label::new("100", Osd::percent_style(&theme)).measure(&mut text).width;
        let want = Label::new(wide, Osd::percent_style(&theme)).measure(&mut text).width;
        assert!(want > base);
        assert_eq!(o.percent_width, Some(want));
        // Geri yüzde: alan yine "100"e göre.
        assert!(o.set_value_text(None));
        assert_eq!(o.percent.text(), "25");
        draw(&mut o, &mut text, &mut rec);
        assert_eq!(o.percent_width, Some(base));
        // Bayt sınırı.
        o.set_value_text(Some("ğ".repeat(100)));
        assert!(o.value_text().unwrap().len() <= OSD_MAX_VALUE_TEXT);
    }

    #[test]
    fn overflow_clamps_and_labels_real_value() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut o = shown(&theme, t0);
        show_value(&mut o, &theme, 1.35, t0);
        assert_eq!(o.value.target(), 1.0, "varsayılan max 1: kırpılır");
        assert_eq!(o.percent.text(), "100");
        o.set_max(1.5, t0);
        show_value(&mut o, &theme, 1.35, t0);
        assert_eq!(o.percent.text(), "135", "gerçek değer");
        show_value(&mut o, &theme, 9.0, t0);
        assert_eq!(o.value.target(), 1.5);
        o.set_max(7.0, t0);
        assert_eq!(o.max(), OSD_MAX_OVERFLOW, "üst sınır");
        o.set_max(f32::NAN, t0);
        assert_eq!(o.max(), 1.0);
        assert_eq!(o.value.target(), 1.0, "max düşünce değer kırpılır");
        assert_eq!(o.percent.text(), "100");
        assert_eq!(o.over.target(), 0.0);
    }

    #[test]
    fn max_change_springs_instead_of_jumping() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut o = shown(&theme, t0);
        o.set_max(1.5, t0);
        assert_eq!(o.max.value(t0), 1.0, "açıkken ölçek yayla akar");
        assert!((o.max.value(t0 + Duration::from_secs(3)) - 1.5).abs() < 1e-3);
    }

    #[test]
    fn bar_geometry() {
        // max 1: işaret ve aşım yok, dolgu orantılı.
        let b = bar(200.0, 8.0, 0.5, 1.0);
        assert_eq!(b, Bar { fill: 100.0, mark: None });
        assert_eq!(bar(200.0, 8.0, 1.4, 1.0), Bar { fill: 200.0, mark: None });
        // max 1.5: %100 işareti 2/3'te; 1.35 → 0.9'a dolar, işaretin sağı aşım.
        let b = bar(300.0, 8.0, 1.35, 1.5);
        assert_eq!(b.mark, Some(200.0));
        assert!((b.fill - 270.0).abs() < 1e-3 && b.fill > 200.0);
        // %100 altı aşımsız.
        assert!(bar(300.0, 8.0, 0.6, 1.5).fill < 200.0);
        // Sıfır dolgu yok; küçük değer hap yüksekliği kadar.
        assert_eq!(bar(300.0, 8.0, 0.0, 1.5).fill, 0.0);
        assert_eq!(bar(300.0, 8.0, 0.001, 1.5).fill, 8.0);
    }

    #[test]
    fn grows_from_edge_and_value_springs() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut o = Osd::new(&theme);
        let bounds = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let anchor = MenuAnchor { side: crate::menu::MenuSide::Above, ..MenuAnchor::below(Rect::new(500.0, 790.0, 0.0, 0.0)) };
        o.show(&theme, Icon::named(IconName::volume(40, false)), 0.4, anchor, bounds, t0);
        assert!(o.shape(t0, &theme).is_some());
        assert_eq!(o.value.value(t0), 0.4, "ilk gösterimde değer yerinde");
        o.show(&theme, Icon::named(IconName::volume(80, false)), 0.8, anchor, bounds, t0);
        assert_eq!(o.value.value(t0), 0.4, "açıkken yayla akar");
        let t1 = t0 + Duration::from_secs(3);
        assert!((o.value.value(t1) - 0.8).abs() < 1e-3);
        assert_eq!(o.percent.text(), "80", "yüzde hedefi gösterir");
        o.hide(t1);
        assert!(!o.is_open());
    }

    #[test]
    fn mute_fades_content_with_motion_and_settles() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut o = shown(&theme, t0);
        o.damage = DamageState::default();
        o.set_muted(false, t0);
        assert!(!o.damage.is_dirty(), "değişiklik yok: hasar ve kare yok");
        o.set_muted(true, t0);
        assert!(o.is_muted() && o.damage.is_dirty());
        assert_eq!(o.content_alpha(t0, &theme), 1.0, "açıkken yayla soluklaşır (zıplamaz)");
        let mid = o.content_alpha(t0 + Duration::from_millis(60), &theme);
        assert!(mid < 1.0 && mid > DISABLED_CONTENT, "ara değer: {mid}");
        assert!(!o.muted.is_settled(t0 + Duration::from_millis(60)), "sürerken kare istenir");
        let t1 = t0 + Duration::from_secs(3);
        assert!((o.content_alpha(t1, &theme) - DISABLED_CONTENT).abs() < 1e-3);
        assert!(o.muted.is_settled(t1), "oturunca kare yok");
        // Sesi açınca geri gelir.
        o.set_muted(false, t1);
        assert!((o.content_alpha(t1 + Duration::from_secs(3), &theme) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn mute_is_instant_when_hidden_or_motion_off() {
        let theme = Theme::default();
        let t0 = Instant::now();
        // Kapalıyken: anında; ardından açılış soluk başlar.
        let mut o = Osd::new(&theme);
        o.set_muted(true, t0);
        assert_eq!(o.content_alpha(t0, &theme), DISABLED_CONTENT);
        show_value(&mut o, &theme, 0.3, t0);
        assert_eq!(o.content_alpha(t0, &theme), DISABLED_CONTENT, "sessiz açılış ilk karesinden soluk");
        // Hareket kapalı: açıkken de anında.
        let (still, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        let mut o = shown(&still, t0);
        o.set_muted(true, t0);
        assert_eq!(o.content_alpha(t0, &theme), DISABLED_CONTENT);
        assert!(o.muted.is_settled(t0));
    }
}
