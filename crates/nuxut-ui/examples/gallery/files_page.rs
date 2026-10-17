//! Sayfa 11: dosya seçici bileşenleri — yol çubuğu (`PathBar`: geniş, dar/katlanmış, düzenleme
//! kipi) ve dosya ızgarası (`FileGrid`: çoklu seçim, küçük resim, 2 satır etiket). Durumlar
//! (`NUXUT_GALLERY_PAGE=11 NUXUT_GALLERY_STATE=…`):
//! - (yok): geniş çubukta klavye odağı (halka) ikinci parçada; ızgarada 3 öğe seçili, imleçte halka;
//! - `edit`: üçüncü çubuk düzenleme kipinde (Ctrl+L eşdeğeri).
//!
//! Tab odağı çubuklar → ızgara arasında gezdirir; Ctrl+L odaktaki çubuğu düzenler.

use std::time::Instant;

use nuxut_core::input::CursorShape;
use nuxut_core::{Point, Rect, Size};
use nuxut_render::{TextSystem, Texture};
use nuxut_theme::Theme;
use nuxut_ui::input as ui_input;
use nuxut_ui::{FileGrid, FileGridEvent, FileItem, Icon, IconName, Label, PathBar, PathBarEvent, PathSegment, TextRole, Ui, text_style};

use super::Card;

const PATH: [&str; 7] = ["Ev", "Belgeler", "Projeler", "Nuxut", "framework", "crates", "nuxut-ui"];

fn segments() -> Vec<PathSegment> {
    PATH.iter()
        .enumerate()
        .map(|(i, n)| {
            let s = PathSegment::new(*n);
            if i == 0 { s.with_icon(Icon::named(IconName::Home)) } else { s }
        })
        .collect()
}

/// Örnek dizin: klasörler, uzun adlı belgeler, resimler (küçük resimleri varlıklar gelince).
fn files() -> Vec<FileItem> {
    let mut v: Vec<FileItem> = ["Belgeler", "İndirilenler", "Müzik", "Resimler", "Videolar"]
        .iter()
        .map(|n| FileItem::new(*n, Icon::named(IconName::Folder)))
        .collect();
    v.push(FileItem::new("2026 yıllık bütçe planı ve harcama raporu taslağı.ods", Icon::named(IconName::File)));
    v.push(FileItem::new("notlar.md", Icon::named(IconName::File)));
    for i in 1..=6 {
        v.push(FileItem::new(format!("tatil-{i:02}.jpg"), Icon::named(IconName::Image)));
    }
    for i in 0..40 {
        v.push(FileItem::new(format!("kayıt_{i:03}.log"), Icon::named(IconName::File)));
    }
    v
}

/// İlk resim dosyasının indisi (`files` sırası).
const FIRST_IMAGE: usize = 7;
const GRID: usize = 3;

pub struct FilesPage {
    cards: Vec<Card>,
    bars: Vec<PathBar>,
    grid: FileGrid,
    /// 0..3 çubuklar, `GRID` ızgara.
    focus: usize,
    modifiers: ui_input::Modifiers,
    status: Label,
    size: Size,
}

impl FilesPage {
    pub fn new(theme: &Theme) -> Self {
        let usb = vec![PathSegment::new("KINGSTON").with_icon(Icon::named(IconName::Usb)), PathSegment::new("Fotoğraflar")];
        let mut grid = FileGrid::new(theme, true);
        grid.set_items(files(), Instant::now());
        Self {
            cards: vec![Card::new(theme, "Yol çubuğu"), Card::new(theme, "Dosya ızgarası")],
            bars: vec![PathBar::new(theme, segments()), PathBar::new(theme, segments()), PathBar::new(theme, usb)],
            grid,
            focus: 0,
            modifiers: ui_input::Modifiers::default(),
            status: Label::new("", text_style(theme, TextRole::Caption)),
            size: Size::ZERO,
        }
    }

    /// Galeri varlıkları yüklenince: resim dosyalarına küçük resim (sonuncusu yüklenmemiş kalır →
    /// ikon görünür).
    pub fn set_assets(&mut self, photo: Option<&Texture>, thumb: Option<&Texture>) {
        let pics = [photo, thumb, photo, thumb, photo];
        for (k, t) in pics.into_iter().enumerate() {
            self.grid.set_thumbnail(FIRST_IMAGE + k, t.cloned());
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        for b in &mut self.bars {
            b.set_theme(theme, now);
        }
        self.grid.set_theme(theme, now);
        self.status.set_style(text_style(theme, TextRole::Caption));
    }

    pub fn layout(&mut self, text: &mut TextSystem, theme: &Theme, size: Size) {
        let m = theme.space(5);
        self.size = size;
        let w = (size.width - m * 2.0).min(theme.space(200));
        self.cards[0].rect = Rect::new(m, m, w, theme.space(60));
        let b = self.cards[0].body(theme);
        let h = self.bars[0].preferred_size(theme, b.width).height;
        let widths = [b.width, theme.space(70), theme.space(110)];
        let mut y = b.y;
        for (bar, bw) in self.bars.iter_mut().zip(widths) {
            bar.set_rect(Rect::new(b.x, y, bw.min(b.width), h));
            bar.layout(text, theme);
            y += h + theme.space(4);
        }
        let top = self.cards[0].rect.bottom() + m;
        self.cards[1].rect = Rect::new(m, top, w, (size.height - top - m).max(0.0));
        let g = self.cards[1].body(theme);
        self.grid.set_rect(theme, Rect::new(g.x, g.y, g.width, (g.height - theme.space(8)).max(0.0)), Instant::now());
    }

    pub fn apply_state(&mut self, theme: &Theme, state: &str, now: Instant) {
        self.bars[0].set_focus_visible(true);
        let _ = self.bars[0].key(&key(ui_input::Key::Home), now);
        let _ = self.bars[0].key(&key(ui_input::Key::Right), now);
        if state == "edit" {
            self.bars[2].begin_edit(theme, "/run/media/kullanici/KINGSTON/Fotoğraflar", now);
        }
        // Izgara: resimlerden üçü seçili (Ctrl+tık eşdeğeri), imleç sonuncusunda, halka görünür.
        self.grid.select_only(FIRST_IMAGE, now);
        let shift = ui_input::Modifiers { shift: true, ..Default::default() };
        let _ = self.grid.key(&with_mods(ui_input::Key::Right, shift), now);
        let _ = self.grid.key(&with_mods(ui_input::Key::Right, shift), now);
        self.grid.set_focus_visible(true);
        self.describe();
    }

    fn describe(&mut self) {
        let s = self.grid.selection();
        let text = format!("{} öğe, {} seçili, {} sütun, {} etiket şekillendirildi", self.grid.len(), s.count(), self.grid.columns(), self.grid.shaped_cells());
        self.status.set_text(&text);
    }

    pub fn modifiers_changed(&mut self, m: ui_input::Modifiers) {
        self.modifiers = m;
    }

    pub fn key(&mut self, theme: &Theme, e: &ui_input::KeyEvent, now: Instant) -> bool {
        self.modifiers = e.modifiers;
        if e.state != ui_input::KeyState::Pressed {
            return false;
        }
        if e.key == ui_input::Key::Tab {
            self.focus = (self.focus + 1) % (GRID + 1);
            for (i, b) in self.bars.iter_mut().enumerate() {
                b.set_focus_visible(i == self.focus);
            }
            self.grid.set_focus_visible(self.focus == GRID);
            return true;
        }
        if self.focus == GRID {
            let r = self.grid.key(e, now);
            self.grid_event(r.event);
            return r.repaint;
        }
        if e.key == ui_input::Key::Char('l') && e.modifiers.ctrl {
            self.bars[self.focus].begin_edit(theme, "/home/kullanici/Belgeler", now);
            return true;
        }
        let r = self.bars[self.focus].key(e, now);
        self.report(r.event.as_ref());
        r.repaint
    }

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, e: &ui_input::PointerEvent, now: Instant) -> bool {
        let mut repaint = false;
        for i in 0..self.bars.len() {
            let r = self.bars[i].pointer(text, theme, e, now);
            repaint |= r.repaint;
            if r.event.is_some() {
                self.focus = i;
            }
            self.report(r.event.as_ref());
        }
        let r = self.grid.pointer(theme, e, self.modifiers, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            self.focus = GRID;
        }
        self.grid_event(r.event);
        repaint
    }

    fn grid_event(&mut self, event: Option<FileGridEvent>) {
        match event {
            Some(FileGridEvent::Activated(i)) => {
                let name = self.grid.item(i).map(|it| it.label.clone()).unwrap_or_default();
                self.status.set_text(&format!("Aç: {name}"));
            }
            Some(FileGridEvent::SelectionChanged) => self.describe(),
            None => {}
        }
    }

    fn report(&mut self, event: Option<&PathBarEvent>) {
        match event {
            Some(PathBarEvent::Navigate(i)) => self.status.set_text(&format!("Git: {i}")),
            Some(PathBarEvent::Edit(ev)) => self.status.set_text(&format!("Düzenleme: {ev:?}")),
            None => false,
        };
    }

    pub fn cursor_at(&self, p: Point) -> CursorShape {
        CursorShape::pick(self.bars.iter().map(|b| b.cursor(p)).chain([self.grid.cursor_shape(p)]))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        for c in &mut self.cards {
            c.draw(ui);
        }
        for b in &mut self.bars {
            b.draw(ui);
        }
        self.grid.draw(ui);
        let card = &self.cards[1];
        let at = Point::new(card.body(theme).x, card.rect.bottom() - theme.space(10));
        ui.damage(Rect::new(at.x, at.y, card.rect.width, theme.space(6)));
        self.status.draw(ui.painter, at, theme.colors.on_surface_variant);
    }
}

fn key(k: ui_input::Key) -> ui_input::KeyEvent {
    with_mods(k, Default::default())
}

fn with_mods(k: ui_input::Key, modifiers: ui_input::Modifiers) -> ui_input::KeyEvent {
    ui_input::KeyEvent { key: k, state: ui_input::KeyState::Pressed, text: None, modifiers, xkb: None }
}
