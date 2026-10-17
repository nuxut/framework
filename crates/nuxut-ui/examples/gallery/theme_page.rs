//! Sayfa 8: imleç ve ikon teması önizlemeleri (`CursorPreview`, `IconThemePreview`) — kurulu
//! temalar, Ayarlar kartlarındaki akışın aynısıyla: şerit ilk çizildiğinde görünen yuvaları
//! ister (`take_load_request`), önce `PreviewCache`'e bakılır, yoksa düşük öncelikli
//! `Loader`'da çözülür, sonuç `MessageSender` ile olay döngüsüne gelir, sonraki karede dokuya
//! yüklenir. Animasyonlu imleçler (bekleme/ilerleme) yalnızca işaretçi şeridin üzerindeyken
//! oynar. `Loader` yalıtılmış çözücüyle kurulur: kullanıcı dizinindeki SVG ikon temaları ve
//! yalnızca-SVG hyprcursor şekilleri işçide çizilir (ADR-0007); işçi yoksa yer tutucu.
//!
//! "Uygulama ikonları" kartı: `app_id` → `.desktop` → tema ikonu (`AppIcons`, aynı yalıtılmış
//! çözücüyle); çözülürken ve bulunamayanda genel pencere ikonu. Sonuç `Msg::AppIcon` ile gelir.

use std::sync::Arc;
use std::time::Instant;

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_image::preview::{cursor_request, icon_request, outcome};
use nuxut_image::{
    AppIconOptions, AppIconTextures, AppIcons, CursorRole, CursorTheme, IconRole, IconTheme, IsolatedDecoder, LoadRequest, Loader,
    LoaderOptions, PreviewCache, PreviewImage, PreviewKey,
};
use nuxut_image_sandbox::{Sandbox, SandboxConfig};
use nuxut_render::{ImageData, Painter, TextureOptions};
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{
    CursorPreview, Icon, IconName, IconThemePreview, IconView, Label, PreviewFrame, PreviewSlot, TextRole, Ui, text_style,
};
use nuxut_wayland::MessageSender;

use super::Card;

/// Gösterilen en fazla tema (her tür).
const MAX_CARDS: usize = 5;

/// "Uygulama ikonları" kartındaki örnek `app_id`'ler: tam ad, küçük harf (`Chromium`),
/// ters-DNS (`copyq` → `com.github.hluk.copyq`), masaüstü girdisi olmayan (`kitty` — tema
/// adı) ve bilinmeyen (genel pencere ikonu).
const APP_IDS: [&str; 8] =
    ["firefox", "brave-browser", "Chromium", "code-oss", "copyq", "org.gnome.Nautilus", "kitty", "bilinmeyen.uygulama"];

/// Galeri olay döngüsü mesajları (sayfa 8).
pub enum Msg {
    Loaded(Loaded),
    /// Bir uygulama ikonu çözüldü: yeniden çiz.
    AppIcon,
}

/// `app_id` → tema ikonu kartı.
struct AppCard {
    card: Card,
    entries: Vec<(IconView, Label)>,
    /// İlk çizimde kurulur (bildirim için olay döngüsü göndericisi gerekir).
    icons: Option<AppIcons>,
    textures: AppIconTextures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Cursor(usize),
    Icon(usize),
}

/// Çözme thread'inden gelen sonuç.
pub struct Loaded {
    which: Which,
    slot: usize,
    key: PreviewKey,
    result: Option<PreviewImage>,
    /// Kalıcı sonuç (önbelleğe yazılır); `false` = geçici (işçinin hız sınırı), yuva "yok"
    /// gösterir ama sonraki açılışta yeniden denenir.
    cache: bool,
}

struct CursorCard {
    card: Card,
    caption: Label,
    theme: Arc<CursorTheme>,
    preview: CursorPreview,
}

struct IconCard {
    card: Card,
    caption: Label,
    theme: Arc<IconTheme>,
    preview: IconThemePreview,
}

pub struct ThemePage {
    cursors: Vec<CursorCard>,
    icons: Vec<IconCard>,
    apps: AppCard,
    sandbox: Option<Arc<dyn IsolatedDecoder>>,
    loader: Loader,
    cache: Arc<PreviewCache>,
    pending: Vec<Loaded>,
    scale: f32,
}

impl ThemePage {
    pub fn new(theme: &Theme) -> Self {
        let caption = |t: &str| Label::new(t, text_style(theme, TextRole::Caption));
        let cursors = nuxut_image::icon::installed_cursor_themes()
            .into_iter()
            .take(MAX_CARDS)
            .map(|t| {
                let ct = CursorTheme::new(t.id.clone());
                let sizes = ct.sizes();
                let mut info = Vec::new();
                if t.xcursor {
                    info.push("XCursor".to_owned());
                }
                if t.hyprcursor {
                    info.push("hyprcursor".to_owned());
                }
                if !sizes.fixed.is_empty() {
                    info.push(sizes.fixed.iter().map(u32::to_string).collect::<Vec<_>>().join(", ") + " px");
                }
                if sizes.scalable {
                    info.push("ölçeklenebilir".to_owned());
                }
                CursorCard {
                    card: Card::new(theme, &t.name),
                    caption: caption(&info.join(" · ")),
                    theme: Arc::new(ct),
                    preview: CursorPreview::new(theme, CursorRole::PREVIEW.len()),
                }
            })
            .collect();
        let sandbox = image_sandbox();
        let icons = nuxut_image::icon::installed_icon_themes()
            .into_iter()
            .filter(|t| !t.hidden)
            .take(MAX_CARDS)
            .map(|t| {
                let info = if t.inherits.is_empty() { t.id.clone() } else { format!("{} · yedek: {}", t.id, t.inherits.join(", ")) };
                IconCard {
                    card: Card::new(theme, &t.name),
                    caption: caption(&info),
                    theme: Arc::new(IconTheme::new(t.id)),
                    preview: IconThemePreview::new(theme, IconRole::PREVIEW.len()),
                }
            })
            .collect();
        Self {
            cursors,
            icons,
            apps: AppCard {
                card: Card::new(theme, "Uygulama ikonları (app_id)"),
                entries: APP_IDS
                    .iter()
                    .map(|id| (IconView::with_role(theme, Icon::named(IconName::Window), TextRole::IconLarge), caption(id)))
                    .collect(),
                icons: None,
                textures: AppIconTextures::new(),
            },
            loader: preview_loader(sandbox.clone()),
            sandbox,
            cache: Arc::new(PreviewCache::default()),
            pending: Vec::new(),
            scale: 1.0,
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        self.scale = scale.max(1.0);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for c in &mut self.cursors {
            c.preview.set_theme(theme, now);
        }
        for c in &mut self.icons {
            c.preview.set_theme(theme, now);
        }
        for (view, caption) in &mut self.apps.entries {
            view.set_theme(theme, now);
            caption.set_style(text_style(theme, TextRole::Caption));
        }
        // framework.toml [desktop] icon_theme; yoksa kullanıcının GTK/Qt ayarı.
        if let Some(icons) = &self.apps.icons {
            icons.set_theme_name(theme.desktop.icon_theme.as_deref());
        }
    }

    pub fn layout(&mut self, theme: &Theme, size: Size) {
        let s = |n| theme.space(n);
        let m = s(5);
        let col_w = (size.width - m * 3.0) * 0.5;
        let pad = s(5);
        let place = |card: &mut Card, caption: &mut Label, preview_h: f32, x: f32, y: f32| -> (Rect, f32) {
            let h = pad + s(8) + s(5) + preview_h + pad;
            card.rect = Rect::new(x, y, col_w, h);
            caption.set_max_width(Some(col_w - pad * 2.0));
            (Rect::new(x + pad, y + pad + s(8) + s(5), col_w - pad * 2.0, preview_h), y + h + m)
        };
        let mut y = m;
        for c in &mut self.cursors {
            let h = c.preview.preferred_height(theme);
            let (r, next) = place(&mut c.card, &mut c.caption, h, m, y);
            c.preview.set_rect(r);
            y = next;
        }
        // Sağ sütunun başı: uygulama ikonları (ikon + altında app_id, eşit hücreler).
        let x = m * 2.0 + col_w;
        let slot = self.apps.entries.first().map_or(0.0, |(v, _)| v.size().height);
        let h = pad + s(8) + s(3) + slot + s(2) + s(6) + pad;
        self.apps.card.rect = Rect::new(x, m, col_w, h);
        let cell = (col_w - pad * 2.0) / self.apps.entries.len().max(1) as f32;
        for (_, caption) in &mut self.apps.entries {
            caption.set_max_width(Some(cell - s(1)));
        }
        let mut y = m + h + m;
        for c in &mut self.icons {
            let h = c.preview.preferred_height(theme);
            let (r, next) = place(&mut c.card, &mut c.caption, h, m * 2.0 + col_w, y);
            c.preview.set_rect(r);
            y = next;
        }
    }

    pub fn pointer(&mut self, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        for c in &mut self.cursors {
            repaint |= c.preview.pointer(e, now).repaint;
        }
        for c in &mut self.icons {
            repaint |= c.preview.pointer(e, now).repaint;
        }
        repaint
    }

    pub fn cursor_at(&self, _p: Point) -> CursorShape {
        CursorShape::Default
    }

    /// Çözme thread'inden gelen sonuç: önbelleğe yazılır, sonraki karede yüklenir.
    pub fn loaded(&mut self, msg: Loaded) {
        if msg.cache {
            self.cache.insert(msg.key.clone(), msg.result.clone());
        }
        self.pending.push(msg);
    }

    /// Bekleyen sonuçları dokuya yükler ve yuvalara verir.
    fn apply_pending(&mut self, painter: &mut Painter, now: Instant) {
        for msg in std::mem::take(&mut self.pending) {
            let slot = match &msg.result {
                Some(d) => PreviewSlot::Ready(frames(painter, d)),
                None => PreviewSlot::Missing,
            };
            match msg.which {
                Which::Cursor(i) => self.cursors[i].preview.set_slot(msg.slot, slot, now),
                Which::Icon(i) => self.icons[i].preview.set_slot(msg.slot, slot, now),
            }
        }
    }

    /// Görünür olan yeni yuvalar için önbelleğe bakar, yoksa çözme işi verir.
    fn request(&mut self, sender: &MessageSender<Msg>) {
        let mut jobs: Vec<(Which, usize, PreviewKey, LoadRequest)> = Vec::new();
        for (ci, c) in self.cursors.iter_mut().enumerate() {
            let Some(range) = c.preview.take_load_request() else { continue };
            let px = c.preview.load_px(self.scale);
            for slot in range {
                let role = CursorRole::PREVIEW[slot];
                let key = PreviewKey::Cursor { theme: c.theme.name().to_owned(), role, px };
                // Animasyonlu imleç tüm kareleriyle (yalnızca üzerindeyken oynar).
                let animate = matches!(role, CursorRole::Wait | CursorRole::Progress);
                jobs.push((Which::Cursor(ci), slot, key, cursor_request(c.theme.clone(), role, px, animate)));
            }
        }
        for (ci, c) in self.icons.iter_mut().enumerate() {
            let Some(range) = c.preview.take_load_request() else { continue };
            let (size, scale) = (c.preview.item_size().round() as u32, self.scale.ceil() as u32);
            for slot in range {
                let role = IconRole::PREVIEW[slot];
                let key = PreviewKey::Icon { theme: c.theme.theme_name().to_owned(), role, px: size * scale };
                jobs.push((Which::Icon(ci), slot, key, icon_request(c.theme.clone(), role, size, scale)));
            }
        }
        for (which, slot, key, request) in jobs {
            if let Some(result) = self.cache.get(&key) {
                self.pending.push(Loaded { which, slot, key, result, cache: false });
                continue;
            }
            let tx = sender.clone();
            // Tema+rol anahtarı: aynı yuvanın yeni isteği eskisini iptal eder (iptal edilen iş
            // sonuç göndermez; yenisi gelir).
            let _ = self.loader.load_keyed(key.load_key(), request, move |r| {
                if matches!(r, Err(nuxut_image::ImageError::Cancelled)) {
                    return;
                }
                let (result, cache) = match outcome(r) {
                    Some(entry) => (entry, true),
                    None => (None, false),
                };
                let _ = tx.send(Msg::Loaded(Loaded { which, slot, key, result, cache }));
            });
        }
    }

    pub fn draw(&mut self, ui: &mut Ui, sender: &MessageSender<Msg>) {
        let theme = ui.theme;
        let now = ui.now;
        if !self.pending.is_empty() {
            self.apply_pending(ui.painter, now);
        }
        let pad = theme.space(5);
        for c in &mut self.cursors {
            c.card.draw(ui);
            let at = Point::new(c.card.rect.x + pad, c.card.rect.y + pad + theme.space(8));
            c.caption.draw(ui.painter, at, theme.colors.on_surface_variant);
            c.preview.draw(ui);
        }
        for c in &mut self.icons {
            c.card.draw(ui);
            let at = Point::new(c.card.rect.x + pad, c.card.rect.y + pad + theme.space(8));
            c.caption.draw(ui.painter, at, theme.colors.on_surface_variant);
            c.preview.draw(ui);
        }
        self.draw_apps(ui, sender);
        self.request(sender);
        // Önbellekten hemen gelenler bir sonraki karede yüklenir.
        if !self.pending.is_empty() {
            ui.request_frame();
        }
    }
}


impl ThemePage {
    /// Uygulama ikonları kartı: her `app_id` için hazırsa doku (`Icon::image`), yoksa genel
    /// pencere ikonu. Çözücü ilk çizimde kurulur (bildirim olay döngüsüne `Msg::AppIcon`).
    fn draw_apps(&mut self, ui: &mut Ui, sender: &MessageSender<Msg>) {
        let theme = ui.theme;
        let scale = self.scale.ceil() as u32;
        let apps = &mut self.apps;
        let icons = apps.icons.get_or_insert_with(|| {
            let tx = sender.clone();
            let options = AppIconOptions { sandbox: self.sandbox.clone(), theme: theme.desktop.icon_theme.clone(), dirs: None };
            AppIcons::new(options, move || {
                let _ = tx.send(Msg::AppIcon);
            })
        });
        apps.card.draw(ui);
        let pad = theme.space(5);
        let r = apps.card.rect;
        let cell = (r.width - pad * 2.0) / apps.entries.len().max(1) as f32;
        let top = r.y + pad + theme.space(8) + theme.space(3);
        for (i, (view, caption)) in apps.entries.iter_mut().enumerate() {
            let side = view.size().height;
            let icon = match apps.textures.get(ui.painter, icons, APP_IDS[i], side.round() as u32, scale) {
                Some(t) => Icon::image(t),
                None => Icon::named(IconName::Window),
            };
            // Yeni doku ya da henüz GPU'da değil: kart yeniden boyanır, yükleme bitene dek kare istenir.
            if view.set_icon(icon) || view.is_pending() {
                ui.damage(r);
                if view.is_pending() {
                    ui.request_frame();
                }
            }
            let cx = r.x + pad + cell * (i as f32 + 0.5);
            view.draw_centered(ui.painter, Point::new(cx, top + side * 0.5), theme.colors.on_surface_variant);
            let m = caption.measure_in(ui.painter);
            caption.draw(ui.painter, Point::new(cx - m.width * 0.5, top + side + theme.space(2)), theme.colors.on_surface_variant);
        }
    }
}

/// Paylaşılan yalıtılmış çözücü (önizleme yükleyicisi ve uygulama ikonları). İkili bulunamazsa
/// (`NUXUT_IMAGE_WORKER` → galeri ikilisinin yanı → `~/.local/lib/nuxut` → `/usr/lib/nuxut`)
/// yalıtım kurulmaz: kullanıcı dizinindeki SVG'ler sessizce yer tutucu kalır. İşçi tembel
/// başlar (ilk güvenilmeyen SVG'de), 30 sn boşta kalınca kapanır.
fn image_sandbox() -> Option<Arc<dyn IsolatedDecoder>> {
    let worker = nuxut_image_sandbox::locate::locate()?;
    Some(Arc::new(Sandbox::new(SandboxConfig { worker: Some(worker), ..SandboxConfig::default() })))
}

/// Düşük öncelikli önizleme yükleyicisi; kullanıcı dizinindeki SVG tema dosyaları yalıtılmış
/// işçide çözülür.
fn preview_loader(sandbox: Option<Arc<dyn IsolatedDecoder>>) -> Loader {
    let loader = Loader::with_options(LoaderOptions { threads: 2, low_priority: true });
    match sandbox {
        Some(sandbox) => loader.with_sandbox(sandbox),
        None => loader,
    }
}

/// Önizleme görseli → yüklenmiş kareler.
fn frames(painter: &mut Painter, d: &PreviewImage) -> Vec<PreviewFrame> {
    d.frames
        .iter()
        .map(|f| {
            let (width, height) = f.image.size();
            let data = ImageData { width, height, pixels: Arc::from(f.image.pixels()) };
            PreviewFrame { texture: painter.upload_texture(data, TextureOptions::default()), delay: f.delay }
        })
        .collect()
}
