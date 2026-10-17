//! Renk seçici: doygunluk/değer alanı + ton şeridi + hex alanı + palet önerileri.
//!
//! Palet önerileri seçim yapılırken hep açıktır (kullanıcı isteği): seçili rengin uyumları
//! (tamamlayıcı, komşular, üçlü, açık/koyu tonlar), çağıranın verdiği son kullanılanlar ve
//! temanın vurgu rengi. Sürüklerken `Changed`, bırakınca/Enter'da `Committed` gelir.
//!
//! Alan ve şerit `Painter::gradient_rect` ile (birkaç quad; yarıçap ve AA SDF'ten).

use std::time::Instant;

use nuxut_core::{Color, Corners, Point, Rect, Size};
use nuxut_motion::Animated;
use nuxut_render::{Border, RectStyle, TextSystem};
use nuxut_theme::Theme;

use crate::input::{Button, CursorShape, KeyEvent, PointerEvent, PointerKind};
use crate::style::{DISABLED_CONTENT, FOCUS_RING, HAIRLINE, Hsv, luminance};

/// Örnek ile zemin arasındaki parlaklık farkı bunun altındaysa örneğe hafif sınır çizilir.
const LOW_CONTRAST: f32 = 0.06;
use crate::text_input::{TextInput, TextInputEvent};
use crate::ui::{DamageState, Response, Ui};

/// Önerilerde gösterilen en fazla son kullanılan renk.
pub const MAX_RECENT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorEvent {
    /// Sürükleme sırasında (canlı önizleme).
    Changed(Color),
    /// Seçim bitti (bırakma, Enter, öneriye tıklama).
    Committed(Color),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    None,
    Field,
    Hue,
}

#[derive(Debug)]
pub struct ColorPicker {
    hsv: Hsv,
    rect: Rect,
    hex: TextInput,
    recent: Vec<Color>,
    suggestions: Vec<Color>,
    swatch_rects: Vec<Rect>,
    hovered_swatch: Option<usize>,
    drag: Drag,
    /// Tutamaç konumları (yaylı: tıklamada akar, sürüklerken doğrudan).
    field_handle: Animated<Point>,
    hue_handle: Animated<f32>,
    damage: DamageState,
    /// Son yerleşimin bölgeleri (imleç için).
    geom: Option<Geometry>,
}

impl ColorPicker {
    pub fn new(theme: &Theme, color: Color) -> Self {
        let hsv = Hsv::from_color(color);
        let mut p = Self {
            hsv,
            rect: Rect::ZERO,
            hex: TextInput::new(theme).with_max_bytes(9),
            recent: Vec::new(),
            suggestions: Vec::new(),
            swatch_rects: Vec::new(),
            hovered_swatch: None,
            drag: Drag::None,
            field_handle: Animated::new(Point::new(hsv.s, 1.0 - hsv.v), theme.motion.spatial_fast),
            hue_handle: Animated::new(hsv.h / 360.0, theme.motion.spatial_fast),
            damage: DamageState::new(),
            geom: None,
        };
        p.sync_hex();
        p.rebuild_suggestions(theme);
        p
    }

    pub fn color(&self) -> Color {
        self.hsv.to_color()
    }

    /// Dışarıdan renk (ör. tema değişti). Tutamaçlar yayla gider.
    pub fn set_color(&mut self, theme: &Theme, color: Color, now: Instant) {
        if color == self.color() {
            return;
        }
        let hsv = Hsv::from_color(color);
        // Gri renkte ton bilinmez: eski tonu koru (tutamaç zıplamasın).
        self.hsv = if hsv.s <= f32::EPSILON { Hsv { h: self.hsv.h, ..hsv } } else { hsv };
        self.field_handle.set_target(Point::new(self.hsv.s, 1.0 - self.hsv.v), now);
        self.hue_handle.set_target(self.hsv.h / 360.0, now);
        self.sync_hex();
        self.rebuild_suggestions(theme);
        self.damage.mark();
    }

    /// Son kullanılan renkler (en yeni başta, en fazla [`MAX_RECENT`]).
    pub fn set_recent(&mut self, theme: &Theme, recent: &[Color]) {
        self.recent = recent.iter().take(MAX_RECENT).copied().collect();
        self.rebuild_suggestions(theme);
        self.damage.mark();
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.hex.set_theme(theme, now);
        self.field_handle.set_motion(theme.motion.spatial_fast, now);
        self.hue_handle.set_motion(theme.motion.spatial_fast, now);
        self.rebuild_suggestions(theme);
        self.damage.mark();
    }

    /// Hex alanı odağı (uygulama klavye odağını yönetir).
    pub fn set_hex_focused(&mut self, focused: bool, now: Instant) {
        self.hex.set_focused(focused, now);
    }

    fn sync_hex(&mut self) {
        let hex = self.color().to_hex();
        if !self.hex.is_focused() {
            self.hex.set_text(&hex);
        }
    }

    fn rebuild_suggestions(&mut self, theme: &Theme) {
        let h = self.hsv;
        let tone = |s: f32, v: f32| Hsv::new(h.h, s, v).to_color();
        let mut out = vec![
            h.with_hue(h.h + 180.0).to_color(),
            h.with_hue(h.h - 30.0).to_color(),
            h.with_hue(h.h + 30.0).to_color(),
            h.with_hue(h.h + 120.0).to_color(),
            h.with_hue(h.h - 120.0).to_color(),
            tone(h.s * 0.45, (h.v + 0.25).min(1.0)),
            tone((h.s * 1.1).min(1.0), h.v * 0.6),
            theme.colors.primary,
        ];
        for c in &self.recent {
            if !out.contains(c) {
                out.push(*c);
            }
        }
        out.dedup();
        self.suggestions = out;
        self.swatch_rects.clear();
    }

    pub fn suggestions(&self) -> &[Color] {
        &self.suggestions
    }

    /// Tercih edilen boyut: genişlik verilir, yükseklik ondan türetilir.
    pub fn preferred_size(&self, theme: &Theme, width: f32) -> Size {
        let field_h = (width * 0.62).round();
        let d = theme.space(7);
        let gap = theme.space(1) * 1.5;
        let per_row = ((width + gap) / (d + gap)).floor().max(1.0) as usize;
        let swatch_rows = self.suggestions.len().div_ceil(per_row).max(1) as f32;
        let h = field_h + theme.space(3) + theme.space(4) + theme.space(4) + theme.space(12) + theme.space(4)
            + swatch_rows * (d + gap);
        Size::new(width, h)
    }

    pub fn set_rect(&mut self, theme: &Theme, rect: Rect) {
        if self.rect == rect {
            return;
        }
        self.rect = rect;
        self.damage.mark();
        let g = self.geometry(theme);
        self.hex.set_rect(g.hex);
        self.geom = Some(g);
        self.swatch_rects.clear();
    }

    fn geometry(&self, theme: &Theme) -> Geometry {
        let r = self.rect;
        let field = Rect::new(r.x, r.y, r.width, (r.width * 0.62).round());
        let hue = Rect::new(r.x, field.bottom() + theme.space(3), r.width, theme.space(4));
        let row_y = hue.bottom() + theme.space(4);
        let preview = Rect::new(r.x, row_y, theme.space(12), theme.space(12));
        let hex = Rect::new(preview.right() + theme.space(3), row_y, r.width - preview.width - theme.space(3), theme.space(12));
        let swatches = Rect::new(r.x, hex.bottom() + theme.space(4), r.width, r.bottom() - hex.bottom() - theme.space(4));
        Geometry { field, hue, preview, hex, swatches }
    }

    fn layout_swatches(&mut self, theme: &Theme) {
        if !self.swatch_rects.is_empty() {
            return;
        }
        let area = self.geometry(theme).swatches;
        let d = theme.space(7);
        let gap = theme.space(1) * 1.5;
        let per_row = ((area.width + gap) / (d + gap)).floor().max(1.0) as usize;
        self.swatch_rects = (0..self.suggestions.len())
            .map(|i| {
                let (row, col) = (i / per_row, i % per_row);
                Rect::new(area.x + col as f32 * (d + gap), area.y + row as f32 * (d + gap), d, d)
            })
            .collect();
    }

    fn set_from_field(&mut self, field: Rect, p: Point) {
        let s = ((p.x - field.x) / field.width.max(1.0)).clamp(0.0, 1.0);
        let v = 1.0 - ((p.y - field.y) / field.height.max(1.0)).clamp(0.0, 1.0);
        self.hsv.s = s;
        self.hsv.v = v;
        self.field_handle.jump_to(Point::new(s, 1.0 - v));
    }

    fn set_from_hue(&mut self, hue: Rect, x: f32) {
        let t = ((x - hue.x) / hue.width.max(1.0)).clamp(0.0, 1.0);
        self.hsv.h = t * 360.0;
        self.hue_handle.jump_to(t);
    }

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<ColorEvent> {
        let g = self.geometry(theme);
        self.layout_swatches(theme);
        let p = event.position;
        // Hex alanı tıklaması imleci koyar.
        let _ = self.hex.pointer(text, theme, event, now);
        match event.kind {
            PointerKind::Press(Button::Left) => {
                if g.field.contains(p) {
                    self.drag = Drag::Field;
                    self.set_from_field(g.field, p);
                } else if g.hue.expand(theme.space(1)).contains(p) {
                    self.drag = Drag::Hue;
                    self.set_from_hue(g.hue, p.x);
                } else if let Some(i) = self.swatch_rects.iter().position(|r| r.contains(p)) {
                    let c = self.suggestions[i];
                    self.set_color(theme, c, now);
                    return Response::event(ColorEvent::Committed(c));
                } else {
                    return Response::none();
                }
                self.after_change(theme);
                Response::event(ColorEvent::Changed(self.color()))
            }
            PointerKind::Motion if self.drag != Drag::None => {
                match self.drag {
                    Drag::Field => self.set_from_field(g.field, p),
                    Drag::Hue => self.set_from_hue(g.hue, p.x),
                    Drag::None => {}
                }
                self.after_change(theme);
                Response::event(ColorEvent::Changed(self.color()))
            }
            PointerKind::Release(Button::Left) if self.drag != Drag::None => {
                self.drag = Drag::None;
                self.rebuild_suggestions(theme);
                Response::event(ColorEvent::Committed(self.color()))
            }
            PointerKind::Motion | PointerKind::Leave => {
                let hovered = if event.kind == PointerKind::Leave { None } else { self.swatch_rects.iter().position(|r| r.contains(p)) };
                if hovered != self.hovered_swatch {
                    self.hovered_swatch = hovered;
                    self.damage.mark();
                    return Response::repaint();
                }
                Response::none()
            }
            _ => Response::none(),
        }
    }

    fn after_change(&mut self, _theme: &Theme) {
        self.sync_hex();
        self.damage.mark();
    }

    /// Hex alanı odaktayken tuşlar. Geçerli hex + Enter → `Committed`.
    pub fn key(&mut self, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<ColorEvent> {
        let r = self.hex.key(event, now);
        match &r.event {
            Some(TextInputEvent::Changed) => {
                // Yazarken geçerli olan her değer canlı uygulanır.
                let t = self.hex.text().trim().to_owned();
                if (t.len() == 7 || t.len() == 9 || t.len() == 6)
                    && let Ok(c) = Color::from_hex(&t)
                {
                    let hsv = Hsv::from_color(c);
                    self.hsv = if hsv.s <= f32::EPSILON { Hsv { h: self.hsv.h, ..hsv } } else { hsv };
                    self.field_handle.set_target(Point::new(self.hsv.s, 1.0 - self.hsv.v), now);
                    self.hue_handle.set_target(self.hsv.h / 360.0, now);
                    self.damage.mark();
                    return Response::event(ColorEvent::Changed(c));
                }
                Response::repaint()
            }
            Some(TextInputEvent::Submit) => {
                let hex = self.color().to_hex();
                self.hex.set_text(&hex);
                self.rebuild_suggestions(theme);
                Response::event(ColorEvent::Committed(self.color()))
            }
            _ => r.discard(),
        }
    }

    /// Hex alanının yapıştırma isteği için (bkz. `TextInput::paste`).
    pub fn paste(&mut self, text: &str) {
        let _ = self.hex.paste(text);
    }

    /// İmleç: SV alanı `Crosshair`, ton şeridi ve öneriler `Pointer`, hex alanı `Text`.
    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        match self.drag {
            Drag::Field => return Some(CursorShape::Crosshair),
            Drag::Hue => return Some(CursorShape::Pointer),
            Drag::None => {}
        }
        let g = self.geom?;
        if g.field.contains(p) {
            Some(CursorShape::Crosshair)
        } else if g.hue.expand(g.hue.height * 0.25).contains(p) || self.swatch_rects.iter().any(|r| r.contains(p)) {
            Some(CursorShape::Pointer)
        } else {
            self.hex.cursor(p)
        }
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let now = ui.now;
        let theme = ui.theme;
        let c = &theme.colors;
        let g = self.geometry(theme);
        self.layout_swatches(theme);
        // Doygunluk/değer alanı: x = doygunluk, y = 1 − değer. HSV tanımı gereği beyaz → ton
        // yatay, üstüne saydam → siyah dikey (bunlar tasarım rengi değil, renk uzayının uçları).
        let field_r = theme.shape.medium();
        let hue = Hsv::new(self.hsv.h, 1.0, 1.0).to_color();
        let white = ui.fade(Color::WHITE); // renk-matematiği: HSV doygunluk/parlaklık uçları
        let black = ui.fade(Color::BLACK); // renk-matematiği: HSV doygunluk/parlaklık uçları
        let clear = Color::TRANSPARENT;
        let hue_c = ui.fade(hue);
        ui.painter.gradient_rect(g.field, field_r, [white, hue_c, hue_c, white]);
        ui.painter.gradient_rect(g.field, field_r, [clear, clear, black, black]);
        // Alanın kenarı yalnızca kenar kipi açıksa.
        if let Some(outline) = theme.outline() {
            let frame = RectStyle { fill: Color::TRANSPARENT, radii: field_r, border: Some(Border { width: HAIRLINE, color: ui.fade(outline) }) };
            ui.painter.rect(g.field, &frame);
        }

        // Ton şeridi: altı ana tonun arası doğrusal (HSV tonu bu segmentlerde doğrusaldır).
        let seg = g.hue.width / 6.0;
        let r = g.hue.height * 0.5;
        for i in 0..6 {
            let a = ui.fade(Hsv::new(i as f32 * 60.0, 1.0, 1.0).to_color());
            let b = ui.fade(Hsv::new((i + 1) as f32 * 60.0, 1.0, 1.0).to_color());
            let radii = match i {
                0 => Corners::new(r, 0.0, 0.0, r),
                5 => Corners::new(0.0, r, r, 0.0),
                _ => Corners::ZERO,
            };
            // Komşu segmentler yarım piksel örtüşür (dikiş görünmesin).
            let rect = Rect::new(g.hue.x + seg * i as f32, g.hue.y, seg + if i < 5 { 0.5 } else { 0.0 }, g.hue.height);
            ui.painter.gradient_rect(rect, radii, [a, b, b, a]);
        }

        // Tutamaçlar: içi seçili renk, iki halka (açık + koyu) her zeminde görünür.
        let color = self.color();
        let fh = self.field_handle.value(now);
        let fp = Point::new(g.field.x + fh.x.clamp(0.0, 1.0) * g.field.width, g.field.y + fh.y.clamp(0.0, 1.0) * g.field.height);
        draw_handle(ui, fp, theme.space(3), color);
        let hp = Point::new(g.hue.x + self.hue_handle.value(now).clamp(0.0, 1.0) * g.hue.width, g.hue.center().y);
        draw_handle(ui, hp, theme.space(3), Hsv::new(self.hsv.h, 1.0, 1.0).to_color());

        // Önizleme + hex.
        ui.painter.rect(g.preview, &RectStyle::filled(ui.fade(color), Corners::uniform(g.preview.height * 0.5)));
        self.hex.draw(ui);

        // Öneriler.
        for (i, (r, col)) in self.swatch_rects.iter().zip(&self.suggestions).enumerate() {
            let selected = *col == color;
            let hovered = self.hovered_swatch == Some(i);
            let grow = theme.space(1) * 0.5;
            let k = if hovered || selected { grow } else { 0.0 };
            let rr = r.expand(k);
            let disc = |r: Rect| Corners::uniform(r.height * 0.5);
            if selected {
                // Boşluklu halka: `on_surface` halka, arada `surface` boşluğu.
                let gap = grow * 1.5;
                let ring = rr.expand(gap + FOCUS_RING);
                ui.painter.rect(ring, &RectStyle::filled(ui.fade(c.on_surface), disc(ring)));
                let hole = rr.expand(gap);
                ui.painter.rect(hole, &RectStyle::filled(ui.fade(c.surface), disc(hole)));
            }
            // Kenar kipi açıksa kenarlık; kapalıyken zemine çok yakın renklerde sınırı hafif bir
            // ton taşır (erişilebilirlik: örnek kaybolmasın).
            let low_contrast = (luminance(*col) - luminance(c.surface_container)).abs() < LOW_CONTRAST;
            let border = theme
                .outline()
                .or_else(|| low_contrast.then(|| c.on_surface_variant.multiply_alpha(DISABLED_CONTENT)))
                .map(|color| Border { width: HAIRLINE, color: ui.fade(color) });
            ui.painter.rect(rr, &RectStyle { fill: ui.fade(*col), radii: disc(rr), border });
        }

        let animating = !self.field_handle.is_settled(now) | !self.hue_handle.is_settled(now);
        if animating {
            ui.request_frame();
        }
        self.damage.report(ui, self.rect.expand(theme.space(3) + FOCUS_RING), animating);
    }
}

fn draw_handle(ui: &mut Ui, p: Point, radius: f32, fill: Color) {
    let theme = ui.theme;
    let outer = Rect::new(p.x - radius, p.y - radius, radius * 2.0, radius * 2.0);
    // `surface` kenarlığı: halka renkli alandan ve şeritten ayrışsın.
    let edge = outer.expand(HAIRLINE * 1.5);
    ui.painter.rect(edge, &RectStyle::filled(ui.fade(theme.colors.surface), Corners::uniform(edge.width * 0.5)));
    // Koyu dış halka + açık iç halka: hem açık hem koyu renk üzerinde seçilir.
    let ring = RectStyle {
        fill: ui.fade(fill),
        radii: Corners::uniform(radius),
        border: Some(Border { width: FOCUS_RING, color: ui.fade(theme.colors.on_surface) }),
    };
    if theme.shadow.enabled {
        let sh = nuxut_render::ShadowStyle { color: ui.fade(theme.shadow.color), blur: theme.space(1), offset: Point::new(0.0, HAIRLINE), spread: 0.0 };
        ui.painter.shadow(outer, Corners::uniform(radius), &sh);
    }
    ui.painter.rect(outer, &ring);
}

#[derive(Debug, Clone, Copy)]
struct Geometry {
    field: Rect,
    hue: Rect,
    preview: Rect,
    hex: Rect,
    swatches: Rect,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn picker() -> (ColorPicker, TextSystem, Theme, Instant) {
        let theme = Theme::default();
        let text = TextSystem::new();
        let t0 = Instant::now();
        let mut p = ColorPicker::new(&theme, Color::from_hex("#9ec5ff").unwrap());
        let s = p.preferred_size(&theme, 280.0);
        p.set_rect(&theme, Rect::new(0.0, 0.0, s.width, s.height));
        (p, text, theme, t0)
    }

    #[test]
    fn field_drag_changes_saturation_value() {
        let (mut p, mut text, theme, t0) = picker();
        let g = p.geometry(&theme);
        let r = p.pointer(&mut text, &theme, &press(g.field.right() - 1.0, g.field.y + 1.0), t0);
        assert!(matches!(r.event, Some(ColorEvent::Changed(_))));
        assert!(p.hsv.s > 0.99 && p.hsv.v > 0.99);
        let _ = p.pointer(&mut text, &theme, &motion(g.field.x - 50.0, g.field.bottom() + 50.0), t0);
        assert_eq!((p.hsv.s, p.hsv.v), (0.0, 0.0));
        let r = p.pointer(&mut text, &theme, &release(0.0, 0.0), t0);
        assert!(matches!(r.event, Some(ColorEvent::Committed(_))));
    }

    #[test]
    fn hue_strip_and_hex_follow() {
        let (mut p, mut text, theme, t0) = picker();
        let g = p.geometry(&theme);
        p.hsv.s = 1.0;
        p.hsv.v = 1.0;
        let _ = p.pointer(&mut text, &theme, &press(g.hue.x, g.hue.center().y), t0);
        assert_eq!(p.color().to_hex(), "#ff0000");
        assert_eq!(p.hex.text(), "#ff0000");
    }

    #[test]
    fn hex_typing_applies_live() {
        let (mut p, _text, theme, t0) = picker();
        p.set_hex_focused(true, t0);
        let _ = p.key(&theme, &key(crate::input::Key::End), t0);
        for _ in 0..7 {
            let _ = p.key(&theme, &key(crate::input::Key::Backspace), t0);
        }
        let mut last = None;
        for ch in "#00ff80".chars() {
            last = p.key(&theme, &typed(ch), t0).event;
        }
        assert_eq!(last, Some(ColorEvent::Changed(Color::from_hex("#00ff80").unwrap())));
        assert_eq!(p.key(&theme, &key(crate::input::Key::Enter), t0).event, Some(ColorEvent::Committed(p.color())));
    }

    #[test]
    fn suggestions_include_harmonies_and_recent() {
        let (mut p, mut text, theme, t0) = picker();
        let red = Color::from_hex("#ff0000").unwrap();
        p.set_color(&theme, red, t0);
        assert_eq!(p.suggestions()[0].to_hex(), "#00ffff", "tamamlayıcı ilk sırada");
        let recent = Color::from_hex("#123456").unwrap();
        p.set_recent(&theme, &[recent]);
        assert!(p.suggestions().contains(&recent));
        // Öneriye tıklama seçer ve onaylar.
        p.layout_swatches(&theme);
        let r0 = p.swatch_rects[0].center();
        let r = p.pointer(&mut text, &theme, &press(r0.x, r0.y), t0);
        assert_eq!(r.event.map(|e| matches!(e, ColorEvent::Committed(_))), Some(true));
    }
}
