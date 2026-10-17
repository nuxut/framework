//! Çerçeve demosu: ekran kenarlarında ince bir çerçeve, üstte kalın kenar (bar) ve bar'daki
//! "Panel" düğmesine tıklayınca bar'dan yayla büyüyerek çıkan bir panel. Panel ve çerçeve tek
//! bir blob alanıdır: birleşim yeri dairesel filetoyla yumuşar, hareket hızı paneli esnetir.
//!
//! ```sh
//! cargo run -p nuxut --example frame
//! NUXUT_DEMO_AUTO=1 cargo run -p nuxut --example frame   # panel her 3 sn'de kendiliğinden açılır/kapanır
//! NUXUT_DEMO_FULL_FRAMES=1 …   # hasar takibini kapat (CPU karşılaştırması)
//! NUXUT_DEMO_SIDE=1 …          # shell'in kenar çekmecesi: sağ kenardan, iç alanın tam boyunda
//! ```
//! Escape (yüzey odaktayken) çıkar.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use nuxut::ui::{Align, Interaction, Label, Row, TextRole, text_style};
use nuxut::{
    Anchor, Animated, App, BlobField, BlobShape, Button, Context, Corners, Deform, FrameShape, Key,
    KeyEvent, KeyState, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, Point,
    PointerEvent, PointerKind, Rect, RectStyle, ShadowStyle, Size, SurfaceId, Theme, TimerId,
};

/// Panelin içeriği.
struct PanelContent {
    title: Label,
    subtitle: Label,
    rows: Vec<(Label, Label)>,
}

struct Screen {
    output: OutputId,
    size: Size,
    panel: Animated<Rect>,
    deform: Deform,
    open: bool,
    chip: Interaction,
    pointer: Option<Point>,
    button_down: bool,
    brand: Label,
    chip_label: Label,
    workspaces: Vec<Label>,
    content: PanelContent,
    /// Önceki karede panelin etkilediği bölge (hasar: eski ∪ yeni).
    last_panel_damage: Option<Rect>,
    /// Sonraki kare tamamen yeniden çizilsin (boyut/tema değişimi).
    damage_all: bool,
}

/// Çerçeve geometrisi (tamamı temadan türetilir).
struct Geometry {
    frame: FrameShape,
    bar: Rect,
    chip: Rect,
    panel_open: Rect,
    panel_closed: Rect,
}

impl Screen {
    fn new(theme: &Theme, output: OutputId) -> Self {
        let body = text_style(theme, TextRole::Body);
        let caption = text_style(theme, TextRole::Caption);
        let title = text_style(theme, TextRole::Title);
        let rows = [
            ("Ses", "%64"),
            ("Parlaklık", "%80"),
            ("Ağ", "virtio-net · bağlı"),
            ("Bluetooth", "kapalı"),
        ]
        .into_iter()
        .map(|(k, v)| (Label::new(k, body.clone()), Label::new(v, caption.clone())))
        .collect();
        Self {
            output,
            size: Size::ZERO,
            panel: Animated::new(Rect::ZERO, theme.motion.spatial),
            deform: Deform::new(theme.blob.deform_spring, theme.blob.deform),
            open: false,
            chip: Interaction::new(theme),
            pointer: None,
            button_down: false,
            brand: Label::new("nuxut", title.clone()),
            chip_label: Label::new("Panel", body.clone()),
            workspaces: (1..=5).map(|i| Label::new(i.to_string(), body.clone())).collect(),
            content: PanelContent {
                title: Label::new("Hızlı ayarlar", title),
                subtitle: Label::new("Bar'dan büyüyerek çıkan blob panel", caption),
                rows,
            },
            last_panel_damage: None,
            damage_all: true,
        }
    }

    fn geometry(&self, theme: &Theme) -> Geometry {
        let (w, h) = (self.size.width, self.size.height);
        let edge = theme.space(2);
        let bar_h = theme.space(10);
        let inner = Rect::new(edge, bar_h, (w - edge * 2.0).max(0.0), (h - bar_h - edge).max(0.0));
        let frame = FrameShape { outer: Rect::new(0.0, 0.0, w, h), inner, inner_radius: theme.shape.radius_large };
        let bar = Rect::new(0.0, 0.0, w, bar_h);
        let chip_w = theme.space(18);
        let chip_h = bar_h - theme.space(3);
        let chip = Rect::new(w - edge - theme.space(2) - chip_w, (bar_h - chip_h) * 0.5, chip_w, chip_h);
        let (pw, ph) = (theme.space(96), theme.space(74));
        let px = (chip.center().x - pw * 0.5).min(w - edge - pw).max(edge);
        if side_drawer() {
            // Shell'in kenar çekmecesi gibi: sağ kenara yapışık, iç alanın tam boyunda (üstte
            // bar'a, altta çerçeveye birleşir); tohum kenar şeridinin 1 px içinde.
            let panel_open = Rect::new(inner.right() - pw, inner.y, pw, inner.height);
            let panel_closed = Rect::new(inner.right() + 1.0, inner.y, 0.0, inner.height);
            return Geometry { frame, bar, chip, panel_open, panel_closed };
        }
        let panel_open = Rect::new(px, inner.y, pw, ph);
        // Kapalıyken bar'ın içinde, düğmenin altında küçük bir tohum: açılırken oradan büyür.
        let panel_closed = Rect::new(panel_open.center().x - pw * 0.25, inner.y - 1.0, pw * 0.5, 0.0);
        Geometry { frame, bar, chip, panel_open, panel_closed }
    }

    fn toggle(&mut self, theme: &Theme, now: Instant) {
        self.open = !self.open;
        let g = self.geometry(theme);
        self.panel.set_target(if self.open { g.panel_open } else { g.panel_closed }, now);
    }

    fn input_rects(&self, theme: &Theme) -> Vec<Rect> {
        let g = self.geometry(theme);
        let f = g.frame;
        let mut rects = vec![
            g.bar,
            Rect::new(0.0, f.inner.y, f.inner.x, f.inner.height),
            Rect::new(f.inner.right(), f.inner.y, f.outer.right() - f.inner.right(), f.inner.height),
            Rect::new(0.0, f.inner.bottom(), f.outer.width, f.outer.bottom() - f.inner.bottom()),
        ];
        if self.open {
            rects.push(g.panel_open);
        }
        rects
    }
}

#[derive(Default)]
struct FrameDemo {
    screens: HashMap<SurfaceId, Screen>,
    auto: Option<TimerId>,
}

const AUTO_PERIOD: Duration = Duration::from_secs(3);

impl App for FrameDemo {
    type Message = ();

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        let spec = LayerSurfaceSpec {
            layer: Layer::Top,
            anchor: Anchor::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardMode::OnDemand,
            ..LayerSurfaceSpec::new("nuxut-frame-demo", output.id)
        };
        let id = cx.create_layer_surface(spec);
        self.screens.insert(id, Screen::new(cx.theme(), output.id));
        if self.auto.is_none() && std::env::var_os("NUXUT_DEMO_AUTO").is_some() {
            self.auto = Some(cx.set_timer(Duration::from_millis(800)));
        }
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.screens.retain(|&id, s| {
            let keep = s.output != output;
            if !keep {
                cx.destroy_surface(id);
            }
            keep
        });
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, _scale: f32) {
        let theme = cx.theme().clone();
        let Some(s) = self.screens.get_mut(&surface) else { return };
        s.size = size;
        s.damage_all = true;
        let g = s.geometry(&theme);
        s.panel.jump_to(if s.open { g.panel_open } else { g.panel_closed });
        let rects = s.input_rects(&theme);
        cx.set_input_region(surface, &rects);
        cx.request_frame(surface);
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        let theme = cx.theme().clone();
        let Some(s) = self.screens.get_mut(&surface) else { return };
        let now = Instant::now();
        match event.kind {
            PointerKind::Leave => s.pointer = None,
            PointerKind::Press(Button::Left) => s.button_down = true,
            PointerKind::Release(Button::Left) => {
                s.button_down = false;
                if s.geometry(&theme).chip.contains(event.position) {
                    s.toggle(&theme, now);
                    let rects = s.input_rects(&theme);
                    cx.set_input_region(surface, &rects);
                }
            }
            _ => {}
        }
        if event.kind != PointerKind::Leave {
            s.pointer = Some(event.position);
        }
        let chip = s.geometry(&theme).chip;
        s.chip.pointer(chip, s.pointer, s.button_down, now);
        cx.request_frame(surface);
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: KeyEvent) {
        if event.state == KeyState::Pressed && event.key == Key::Escape {
            cx.quit();
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.auto != Some(id) {
            return;
        }
        let theme = cx.theme().clone();
        let now = Instant::now();
        let ids: Vec<SurfaceId> = self.screens.keys().copied().collect();
        for sid in ids {
            if let Some(s) = self.screens.get_mut(&sid) {
                s.toggle(&theme, now);
                let rects = s.input_rects(&theme);
                cx.set_input_region(sid, &rects);
            }
            cx.request_frame(sid);
        }
        self.auto = Some(cx.set_timer(AUTO_PERIOD));
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        for (&id, s) in &mut self.screens {
            s.panel.set_spring(theme.motion.spatial, now);
            s.deform.set_params(theme.blob.deform_spring, theme.blob.deform, now);
            s.chip.set_theme(&theme, now);
            s.damage_all = true;
            let g = s.geometry(&theme);
            s.panel.set_target(if s.open { g.panel_open } else { g.panel_closed }, now);
            cx.request_frame(id);
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let theme = cx.theme().clone();
        let Some(s) = self.screens.get_mut(&surface) else { return };
        let g = s.geometry(&theme);
        let colors = theme.colors;

        // Panel: yay konumu + hızdan esneme.
        let raw = s.panel.value(now);
        // Kapanırken yay sıfırı aşarsa (negatif yükseklik) panel bar'ın içine gömülür:
        // sıfır yükseklikli şekil, iç duvarı o kadar geri çeker (blob çökmesi) — puding.
        let side = side_drawer();
        let panel = if side {
            Rect::new(raw.x.min(raw.right()), raw.y, raw.width.max(0.0), raw.height.max(0.0))
        } else if raw.height >= 0.0 {
            Rect::new(raw.x, raw.y, raw.width.max(0.0), raw.height)
        } else {
            Rect::new(raw.x, raw.y + raw.height, raw.width.max(0.0), 0.0)
        };
        // Kenara yapışık panel yalnızca büyüme ekseninde esner; çapası kenarın ortası.
        let v = s.panel.velocity(now);
        let (velocity, anchor) = if side {
            (Point::new(v.x + v.width * 0.5, 0.0), Point::new(1.0, 0.5))
        } else {
            (Point::new(0.0, v.y + v.height * 0.5), Point::new(0.5, 0.0))
        };
        s.deform.update(velocity, now);
        let r = theme.shape.radius_large;
        let shadow = theme.shadow.enabled.then(|| ShadowStyle {
            color: theme.shadow.color,
            blur: theme.shadow.blur,
            offset: Point::new(0.0, theme.shadow.offset_y),
            spread: 0.0,
        });
        // Esneme dahil şekil; içerik görünen dikdörtgenine yerleşir (iç boşluk sabit kalır).
        let shape = BlobShape::new_rect(panel).with_deform(s.deform.matrix(now), anchor);
        // Tamamen kapanıp oturmuş panel alandan çıkarılır (zaten görünmez; boşuna hesaplanmaz).
        let settled_closed = !s.open && s.panel.is_settled(now);
        let shapes = if settled_closed { Vec::new() } else { vec![shape] };
        let shapes_empty = shapes.is_empty() || panel.height < 0.5;
        let field = BlobField {
            frame: Some(g.frame),
            shapes,
            smoothing: theme.blob.smoothing,
            corner_radius: r,
            fill: colors.surface,
            shadow,
        };
        // Çerçeveye yapışık kenarlar esnemede yerinde kalır; içerik alanın gördüğüne yerleşir.
        let visual = field.visual_rect(&shape);

        // Hasar: panelin eski ve yeni etki alanı + düğme. Çerçeve ve bar metni değişmez.
        // NUXUT_DEMO_FULL_FRAMES=1: hasar bildirme (karşılaştırma ölçümü için).
        if std::mem::take(&mut s.damage_all) || std::env::var_os("NUXUT_DEMO_FULL_FRAMES").is_some() {
            painter.damage_all();
        }
        let panel_damage = field.shapes.first().map(|sh| field.damage_rect(sh));
        for r in [s.last_panel_damage, panel_damage].into_iter().flatten() {
            painter.damage(r);
        }
        s.last_panel_damage = panel_damage;
        painter.damage(g.chip);
        painter.blob(&field);

        // Bar içeriği.
        let pad = theme.space(4);
        let brand_size = s.brand.measure_in(painter);
        s.brand.draw(painter, Point::new(pad, (g.bar.height - brand_size.height) * 0.5), colors.primary);

        let ws_sizes: Vec<Size> = s.workspaces.iter_mut().map(|l| l.measure_in(painter)).collect();
        let row = Row::new(theme.space(4), Align::Center);
        let row_size = row.measure(&ws_sizes);
        let row_bounds = Rect::new((g.bar.width - row_size.width) * 0.5, 0.0, row_size.width, g.bar.height);
        for (i, (label, rect)) in s.workspaces.iter_mut().zip(row.arrange(row_bounds, &ws_sizes)).enumerate() {
            let color = if i == 0 { colors.primary } else { colors.on_surface_variant };
            label.draw(painter, rect.origin(), color);
        }

        let chip_radii = Corners::uniform(theme.shape.radius_small);
        let chip_fill = if s.open { colors.surface_bright } else { colors.surface_container };
        painter.rect(g.chip, &RectStyle::filled(chip_fill, chip_radii));
        s.chip.draw_layer(painter, g.chip, chip_radii, &theme, now);
        let cl = s.chip_label.measure_in(painter);
        let chip_text = Point::new(g.chip.center().x - cl.width * 0.5, g.chip.center().y - cl.height * 0.5);
        s.chip_label.draw(painter, chip_text, if s.open { colors.primary } else { colors.on_surface });

        // Panel içeriği: panel yeterince açıldıkça belirir, panel sınırına kırpılır.
        let open_h = g.panel_open.height;
        let reveal = ((visual.height - open_h * 0.35) / (open_h * 0.5)).clamp(0.0, 1.0);
        if reveal > 0.0 && !shapes_empty {
            painter.push_clip(visual);
            let inset = theme.space(5);
            let mut y = visual.y + inset;
            let x = visual.x + inset;
            let inner_w = visual.width - inset * 2.0;
            let t = s.content.title.measure_in(painter);
            s.content.title.draw(painter, Point::new(x, y), colors.on_surface.multiply_alpha(reveal));
            y += t.height + theme.space(1);
            let st = s.content.subtitle.measure_in(painter);
            s.content.subtitle.draw(painter, Point::new(x, y), colors.on_surface_variant.multiply_alpha(reveal));
            y += st.height + theme.space(4);
            let row_h = theme.space(11);
            let card_r = Corners::uniform(theme.shape.radius_medium);
            for (key, value) in &mut s.content.rows {
                let card = Rect::new(x, y, inner_w, row_h);
                painter.rect(card, &RectStyle::filled(colors.surface_container.multiply_alpha(reveal), card_r));
                let ks = key.measure_in(painter);
                key.draw(painter, Point::new(card.x + theme.space(4), card.center().y - ks.height * 0.5), colors.on_surface.multiply_alpha(reveal));
                let vs = value.measure_in(painter);
                value.draw(
                    painter,
                    Point::new(card.right() - theme.space(4) - vs.width, card.center().y - vs.height * 0.5),
                    colors.on_surface_variant.multiply_alpha(reveal),
                );
                y += row_h + theme.space(2);
            }
            painter.pop_clip();
        }

        let animating = !s.panel.is_settled(now) | !s.deform.is_settled(now) | s.chip.is_animating(now);
        if animating {
            cx.request_frame(surface);
        }
    }
}

fn side_drawer() -> bool {
    std::env::var_os("NUXUT_DEMO_SIDE").is_some()
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if let Err(e) = nuxut::run(FrameDemo::default()) {
        eprintln!("frame demosu: {e}");
        std::process::exit(1);
    }
}
