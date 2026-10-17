//! Aç/kapa anahtarı (switch). Topuz `spatial` yayıyla kayar (az sönümlü: hedefi hafifçe aşıp
//! oturur), açıkken büyür, hover'da fark edilir biçimde, basılıyken daha çok büyür; renkler
//! `effects` yayıyla geçer. Hover/basılı katmanı ray dolgusuna karışır; klavye odağında topuz tam
//! boy olur ve ortasında nokta belirir (outline yok). Bileşen `rect` dışına hiçbir şey taşmaz.

use std::hash::Hash;
use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle};
use nuxut_theme::{STATE_HOVER, Theme};

use crate::access::{AccessAction, AccessNode, AccessTree, Role};
use crate::input::{CursorShape, KeyEvent, PointerEvent};
use crate::pressable::Pressable;
use crate::style::{DISABLED_CONTENT, FOCUS_RING, contrast_ratio, disabled_container, disabled_content, most_contrasting, pill, state_layer_under};
use crate::ui::{Response, Ui};

#[derive(Debug, Clone)]
pub struct Toggle {
    core: Pressable,
    on: bool,
    /// Topuz konumu 0 (kapalı) .. 1 (açık).
    position: Animated<f32>,
    /// Renk karışımı 0 (kapalı) .. 1 (açık).
    tint: Animated<f32>,
}

impl Toggle {
    pub fn new(theme: &Theme, on: bool) -> Self {
        let v = if on { 1.0 } else { 0.0 };
        Self {
            core: Pressable::new(theme),
            on,
            position: Animated::new(v, theme.motion.spatial),
            tint: Animated::new(v, theme.motion.effects),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Dışarıdan durum (ör. D-Bus'tan gelen gerçek durum). Değiştiyse yayla geçer.
    pub fn set_on(&mut self, on: bool, now: Instant) -> bool {
        if self.on == on {
            return false;
        }
        self.on = on;
        let v = if on { 1.0 } else { 0.0 };
        self.position.set_target(v, now);
        self.tint.set_target(v, now);
        self.core.damage.mark();
        true
    }

    pub fn set_enabled(&mut self, enabled: bool, now: Instant) -> bool {
        self.core.set_enabled(enabled, now)
    }

    pub fn set_focus_visible(&mut self, on: bool) -> bool {
        self.core.set_focus_visible(on)
    }

    /// Görünüş durumunu doğrudan verir (galeri, ekran görüntüsü testleri).
    pub fn set_visual_state(&mut self, hovered: bool, pressed: bool, now: Instant) {
        self.core.set_visual_state(hovered, pressed, now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.core.set_theme(theme, now);
        self.position.set_motion(theme.motion.spatial, now);
        self.tint.set_motion(theme.motion.effects, now);
    }

    /// Ray: `space(13) × space(8)`.
    pub fn preferred_size(&self, theme: &Theme) -> Size {
        Size::new(theme.space(13), theme.space(8))
    }

    pub fn set_rect(&mut self, rect: Rect) {
        self.core.set_rect(rect);
    }

    pub fn rect(&self) -> Rect {
        self.core.rect
    }

    /// Tıklamada yeni durum: `event = Some(açık_mı)`.
    pub fn pointer(&mut self, event: &PointerEvent, now: Instant) -> Response<bool> {
        let r = self.core.pointer(event, now);
        self.after(r, now)
    }

    pub fn key(&mut self, event: &KeyEvent, now: Instant) -> Response<bool> {
        let r = self.core.key(event, now);
        self.after(r, now)
    }

    fn after(&mut self, r: Response, now: Instant) -> Response<bool> {
        if r.event.is_some() {
            self.set_on(!self.on, now);
        }
        let on = self.on;
        r.map(|()| on)
    }

    /// İmleç (DESIGN.md "İmleç").
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        self.core.cursor(p)
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let rect = self.core.rect;
        let t = self.tint.value(now).clamp(0.0, 1.0);
        let p = self.position.value(now);
        let enabled = self.core.enabled;
        let radii = pill(rect);

        // Ray: kapalıyken dolgulu `surface_bright` (kenar yalnızca kenar kipi açıksa), açıkken `primary`.
        let ToggleColors { track, border, thumb } = ToggleColors::new(theme, enabled, t, self.core.interaction.opacity(now));
        let style = RectStyle {
            fill: ui.fade(track),
            radii,
            border: border.map(|color| Border { width: FOCUS_RING, color: ui.fade(color) }),
        };
        ui.painter.rect(rect, &style);

        let hover = (self.core.interaction.opacity(now) / STATE_HOVER).min(1.0);
        let focus = self.core.focus_visible && enabled;
        let tr = thumb_rect(theme, rect, t, p, hover, self.core.squish(now), focus);
        ui.painter.rect(tr, &RectStyle::filled(ui.fade(thumb), Corners::uniform(tr.width * 0.5)));
        // Klavye odağı: topuz tam boy + ortasında nokta (outline yok; şekil + renk). Nokta
        // `primary`; topuz rengine karşı okunmuyorsa (kapalıyken açık gri topuz) en kontrastlı token.
        if focus {
            let dot = tr.expand(-tr.width * 0.32);
            ui.painter.rect(dot, &RectStyle::filled(ui.fade(focus_dot(theme, thumb)), Corners::uniform(dot.width * 0.5)));
        }

        let animating = !self.position.is_settled(now) | !self.tint.is_settled(now);
        if animating {
            ui.request_frame();
        }
        // Hasar yalnız bileşen dikdörtgeni: hiçbir katman dışarı taşmaz.
        self.core.finish(ui, rect);
    }
}

/// Topuz: kapalı küçük, açık büyük; hover'da (`hover` 0..1) fark edilir biçimde, basılıyken
/// (`squish`) daha çok büyür; klavye odağında en az tam boy (açık topuz boyu). Her durumda —
/// konum yayı (`p`) hedefi aşsa da — rayın içinde kalır.
fn thumb_rect(theme: &Theme, rect: Rect, t: f32, p: f32, hover: f32, squish: f32, focus: bool) -> Rect {
    let h = rect.height;
    let pad = theme.space(1);
    let full = h - pad * 2.0;
    let small = full * 0.6;
    let mut base = small + (full - small) * t;
    if focus {
        base = base.max(full);
    }
    let d = (base + (h - base) * (0.25 * hover.clamp(0.0, 1.0) + 0.45 * squish.max(0.0))).min(h);
    let travel = rect.width - pad * 2.0 - full;
    let cx = (rect.x + pad + full * 0.5 + travel * p).clamp(rect.x + d * 0.5, rect.right() - d * 0.5);
    Rect::new(cx - d * 0.5, rect.center().y - d * 0.5, d, d)
}

/// Odak noktası: `primary`, topuza karşı 3:1'i tutmuyorsa en kontrastlı token.
/// Anahtar renkleri (saf; yüksek karşıtlık testleri bunun üstünden). `t` 0 = kapalı, 1 = açık
/// tonu; `layer` hover/basılı opaklığı.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ToggleColors {
    pub track: Color,
    /// Kapalı rayın kenarı (kenar kipi açıksa; yüksek karşıtlıkta her zaman).
    pub border: Option<Color>,
    pub thumb: Color,
}

impl ToggleColors {
    pub(crate) fn new(theme: &Theme, enabled: bool, t: f32, layer: f32) -> Self {
        let c = &theme.colors;
        if !enabled {
            let track = disabled_container(theme);
            let border_alpha = 1.0 - t;
            let border = theme
                .outline()
                .filter(|_| border_alpha > 0.0)
                .map(|o| o.multiply_alpha(border_alpha * DISABLED_CONTENT));
            return Self { track, border, thumb: disabled_content(theme, c.on_surface, &[track]) };
        }
        // Ray: kapalıyken dolgulu `surface_bright`, açıkken `primary`. Hover/basılı: state layer
        // rayın dolgusuna karıştırılır (aynı hap, tek kenar; dışarı taşmaz, kenarda ikinci
        // yumuşatma halkası oluşmaz); katman rengi içerik renginden, topuz ona karşı okunur kalır.
        let content = c.on_surface.lerp(&c.on_primary, t);
        let base = c.surface_bright.lerp(&c.primary, t);
        let thumb = c.on_surface_variant.lerp(&c.on_primary, t);
        // Yüksek karşıtlıkta katman opaklığı topuz okunur kalacak kadarla sınırlı (`state_layer_under`).
        let layer = if high_contrast(theme) { state_layer_under(theme, content.with_alpha(1.0), layer, &[base], &[thumb]).a } else { layer };
        let track = base.lerp(&content, layer);
        let border_alpha = 1.0 - t;
        let border = theme.outline().filter(|_| border_alpha > 0.0).map(|o| o.multiply_alpha(border_alpha));
        Self { track, border, thumb }
    }
}

fn high_contrast(theme: &Theme) -> bool {
    theme.contrast.is_high()
}

fn focus_dot(theme: &Theme, thumb: Color) -> Color {
    let c = &theme.colors;
    if contrast_ratio(c.primary, thumb) >= 3.0 { c.primary } else { most_contrasting(thumb, &[c.primary, c.on_primary, c.on_surface, c.surface]) }
}

// Erişilebilirlik (ADR-0011).
impl Toggle {
    /// `Role::Switch` + açık/kapalı. Ad görünmez (satırın başlığı): çağıran `.label(..)` verir.
    pub fn access<'t>(&self, tree: &'t mut AccessTree, key: impl Hash) -> AccessNode<'t> {
        tree.push(key, Role::Switch).bounds(self.core.rect).enabled(self.core.enabled).toggled(self.on).clickable()
    }

    /// AT eylemi: `Click` anahtarı çevirir (`event = Some(yeni durum)`, tıklamayla aynı).
    pub fn access_action(&mut self, action: AccessAction, now: Instant) -> Response<bool> {
        if action != AccessAction::Click || !self.core.enabled {
            return Response::none();
        }
        self.core.pulse(now);
        self.after(Response::event(()), now)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::input::test_util::*;

    #[test]
    fn click_toggles_and_springs() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = Toggle::new(&theme, false);
        t.set_rect(Rect::new(0.0, 0.0, 52.0, 32.0));
        let _ = t.pointer(&press(5.0, 5.0), t0);
        assert_eq!(t.pointer(&release(5.0, 5.0), t0).event, Some(true));
        assert!(t.is_on());
        // Az sönümlü yay: topuz hedefi bir an aşar.
        let max = (1..80).map(|k| t.position.value(t0 + Duration::from_millis(k * 10))).fold(0.0, f32::max);
        assert!(max > 1.0, "puding taşması beklenir: {max}");
        assert!(t.position.is_settled(t0 + Duration::from_secs(4)));
    }

    #[test]
    fn thumb_never_leaves_track() {
        let theme = Theme::default();
        let r = Rect::new(10.0, 20.0, theme.space(13), theme.space(8));
        let inside = |b: Rect| b.x >= r.x - 1e-3 && b.right() <= r.right() + 1e-3 && b.y >= r.y - 1e-3 && b.bottom() <= r.bottom() + 1e-3;
        for t in [0.0, 1.0] {
            for p in [-0.2, 0.0, 1.0, 1.2] {
                for (hover, squish) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (1.0, 1.5)] {
                    for focus in [false, true] {
                        let b = thumb_rect(&theme, r, t, p, hover, squish, focus);
                        assert!(inside(b), "t={t} p={p} hover={hover} squish={squish} focus={focus}: {b:?}");
                    }
                }
            }
        }
        // Hover topuzu büyütür, basılı daha çok.
        let w = |hover, squish| thumb_rect(&theme, r, 0.0, 0.0, hover, squish, false).width;
        assert!(w(0.0, 0.0) < w(1.0, 0.0) && w(1.0, 0.0) < w(1.0, 1.0));
        assert!(w(1.0, 0.0) - w(0.0, 0.0) >= theme.space(1), "hover farkı fark edilir");
        // Odakta kapalı topuz tam boy.
        let full = thumb_rect(&theme, r, 1.0, 1.0, 0.0, 0.0, false).width;
        assert!((thumb_rect(&theme, r, 0.0, 0.0, 0.0, 0.0, true).width - full).abs() < 1e-3);
    }

    #[test]
    fn focus_dot_and_on_thumb_contrast_across_palettes() {
        use nuxut_palette::{Mode, Palette, Variant};
        let mut palettes = vec![Theme::default().colors];
        for seed in ["#4285f4", "#c0602a", "#2e7d32", "#9c27b0", "#ffeb3b"] {
            for mode in [Mode::Dark, Mode::Light] {
                for v in Variant::ALL {
                    let p = Palette::generate(Color::from_hex(seed).unwrap(), v, mode, 0.0);
                    let mut colors = Theme::default().colors;
                    colors.apply(&nuxut_palette::export::generated_palette(&p, None).colors);
                    palettes.push(colors);
                }
            }
        }
        for colors in palettes {
            let theme = Theme { colors, ..Theme::default() };
            let c = &theme.colors;
            // Açık: on_primary topuz primary rayda.
            assert!(contrast_ratio(c.on_primary, c.primary) >= 3.0);
            // Odak noktası her iki durumda topuza karşı.
            for thumb in [c.on_surface_variant, c.on_primary] {
                let r = contrast_ratio(focus_dot(&theme, thumb), thumb);
                assert!(r >= 3.0, "odak noktası {r:.2} ({})", c.primary.to_hex());
            }
        }
    }

    #[test]
    fn external_state_does_not_emit() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut t = Toggle::new(&theme, true);
        assert!(!t.set_on(true, t0));
        assert!(t.set_on(false, t0));
    }
}
