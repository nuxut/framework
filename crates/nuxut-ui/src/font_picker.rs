//! Aranabilir font seçici ([`FontPicker`]): arama + filtre (tümü / eş aralıklı) + sanal aile
//! listesi + ağırlık seçimi + önizleme. Her satır ailenin adını (tema fontunda; simge
//! fontlarında da okunur) ve örnek metni **o ailenin kendi fontuyla** gösterir; aile adı
//! veriden (`TextSystem::families()`) gelir, temadan değil.
//!
//! - **Liste:** [`ResultList`] (yalnızca görünen satırlar şekillendirilir, layout'lar LRU
//!   önbellekte). Seçili aile ✓ ile işaretli.
//! - **Ağırlık:** statik ailede yalnızca bulunan ağırlıklar (Choice çipleri); değişken ailede
//!   (`FamilyInfo::weight_range`) ailenin `wght` aralığında kaydırıcı, 100'lük adımlar.
//! - **Yalnızca metin aileleri:** `latin == false` aileler (sembol ve Latin dışı yazı fontları)
//!   listelenmez.
//! - **Boşta iş yok:** yalnızca alt bileşenler canlanırken kare istenir.

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_render::{FamilyInfo, TextStyle, TextSystem};
use nuxut_theme::Theme;

use crate::chip::{Chip, ChipEvent, ChipGroup, ChipKind};
use crate::icon::Icon;
use crate::icon_names::IconName;
use crate::input::{CursorShape, Key, KeyEvent, KeyState, PointerEvent};
use crate::label::Label;
use crate::results::{ResultEvent, ResultItem, ResultList};
use crate::search::SearchField;
use crate::segmented::{Segment, SegmentedButton};
use crate::slider::{Orientation, Slider};
use crate::style::clip_text;
use crate::text_input::TextInputEvent;
use crate::typography::{TextRole, text_style};
use crate::ui::{Response, Ui};

/// Ağırlık adlarının kapsadığı CSS aralığı ve kaydırıcı adımı.
const WEIGHT_MIN: u16 = 100;
const WEIGHT_MAX: u16 = 900;
const WEIGHT_STEP: u16 = 100;

/// Çağıranın dilindeki metinler.
#[derive(Debug, Clone, PartialEq)]
pub struct FontPickerLabels {
    /// Arama yer tutucusu ("Font ara…").
    pub search: String,
    /// Filtre: tüm aileler.
    pub all: String,
    /// Filtre: yalnızca eş aralıklı aileler.
    pub monospace: String,
    /// Satırlarda ve önizlemede örnek metin.
    pub sample: String,
    /// 100, 200 … 900 ağırlıklarının adları ("İnce" … "Siyah").
    pub weight_names: [String; 9],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontEvent {
    /// Aile ya da ağırlık değişti. Aile değişince ağırlık eldekine en yakın bulunana oturur.
    Changed { family: String, weight: u16 },
}

#[derive(Debug)]
pub struct FontPicker {
    labels: FontPickerLabels,
    families: Vec<FamilyInfo>,
    /// Listede görünen ailelerin `families` içindeki indisleri (filtre + arama sonrası).
    shown: Vec<usize>,
    selected: Option<(String, u16)>,
    /// Kaydırıcının kurulduğu `wght` aralığı (adım sayısı ona göre).
    slider_range: Option<(u16, u16)>,
    search: SearchField,
    filter: SegmentedButton,
    /// `false`: filtre gizli, her zaman "Tümü" (liste çağıranca önceden süzülmüş, ör. ikon fontları).
    show_filter: bool,
    list: ResultList,
    weights: ChipGroup,
    /// `weights` çiplerinin ağırlıkları (aynı sıra).
    chip_weights: Vec<u16>,
    slider: Slider,
    weight_label: Label,
    preview: Label,
    rect: Rect,
    /// Ağırlık bölgesi (çipler ya da kaydırıcı satırı).
    weight_rect: Rect,
    preview_origin: Point,
}

impl FontPicker {
    pub fn new(theme: &Theme, families: &[FamilyInfo], labels: FontPickerLabels, selected: Option<(&str, u16)>) -> Self {
        let filter = SegmentedButton::new(theme, vec![Segment::new(labels.all.clone()), Segment::new(labels.monospace.clone())], 0);
        let mut p = Self {
            search: SearchField::new(theme, &labels.search),
            filter,
            show_filter: true,
            list: ResultList::new(theme),
            weights: ChipGroup::new(Vec::new(), true),
            chip_weights: Vec::new(),
            slider: Slider::new(theme, Orientation::Horizontal, 0.0).with_steps(u32::from((WEIGHT_MAX - WEIGHT_MIN) / WEIGHT_STEP)),
            weight_label: Label::new("", text_style(theme, TextRole::Label).with_tabular_numbers().with_max_lines(1)),
            preview: Label::new(clip_text(&labels.sample, 256), text_style(theme, TextRole::Title).with_max_lines(1)),
            labels,
            families: families.to_vec(),
            shown: Vec::new(),
            selected: selected.map(|(f, w)| (f.to_owned(), w)),
            slider_range: None,
            rect: Rect::ZERO,
            weight_rect: Rect::ZERO,
            preview_origin: Point::ZERO,
        };
        p.search.set_focused(true, Instant::now());
        p.refilter(theme, Instant::now());
        p
    }

    /// "Tümü / Eş aralıklı" filtresini gösterir ya da gizler (gizliyken tüm aileler; varsayılan
    /// açık). Liste çağıranca zaten süzülmüşse (ör. ikon fontu rolü) kapatılır.
    pub fn with_filter(mut self, theme: &Theme, on: bool) -> Self {
        self.show_filter = on;
        self.refilter(theme, Instant::now());
        self
    }

    /// Aile listesi değişti (font kuruldu/kaldırıldı). Seçim korunur.
    pub fn set_families(&mut self, theme: &Theme, families: &[FamilyInfo], now: Instant) {
        self.families = families.to_vec();
        self.refilter(theme, now);
    }

    pub fn selected(&self) -> Option<(&str, u16)> {
        self.selected.as_ref().map(|(f, w)| (f.as_str(), *w))
    }

    /// Dışarıdan seçim (config yüklendi); olay üretmez.
    pub fn set_selected(&mut self, theme: &Theme, family: &str, weight: u16, now: Instant) {
        self.selected = Some((family.to_owned(), weight));
        self.sync_selection(theme, now);
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.search.set_theme(theme, now);
        self.filter.set_theme(theme, now);
        self.list.set_theme(theme, now);
        self.weights.set_theme(theme, now);
        self.slider.set_theme(theme, now);
        self.weight_label.set_style(text_style(theme, TextRole::Label).with_tabular_numbers().with_max_lines(1));
        self.update_preview(theme);
    }

    /// IME ve yapıştırma tesisatı (arama alanı).
    pub fn search_mut(&mut self) -> &mut SearchField {
        &mut self.search
    }

    /// Önerilen boyut (Popover içeriği için).
    pub fn preferred_size(theme: &Theme) -> Size {
        Size::new(theme.space(100), theme.space(130))
    }

    fn family(&self) -> Option<&FamilyInfo> {
        let (name, _) = self.selected.as_ref()?;
        self.families.iter().find(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// Seçili aile değişkense `wght` aralığı.
    fn range(&self) -> Option<(u16, u16)> {
        self.family().and_then(|f| f.weight_range)
    }

    /// Seçili ailenin listedeki satırı (filtrede görünüyorsa).
    fn selected_row(&self) -> Option<usize> {
        let (name, _) = self.selected.as_ref()?;
        self.shown.iter().position(|&i| self.families[i].name.eq_ignore_ascii_case(name))
    }

    fn monospace_only(&self) -> bool {
        self.show_filter && self.filter.selected() == 1
    }

    /// Filtre + arama → görünen aileler; liste yeniden kurulur, seçili aile işaretlenir.
    fn refilter(&mut self, theme: &Theme, now: Instant) {
        let query: Vec<char> = self.search.text().trim().chars().flat_map(char::to_lowercase).collect();
        let mono = self.monospace_only();
        let sample = clip_text(&self.labels.sample, 256).to_owned();
        let mut items = Vec::new();
        self.shown.clear();
        for (i, f) in self.families.iter().enumerate() {
            if !f.latin || (mono && !f.monospace) {
                continue;
            }
            let Some(m) = subsequence(&f.name, &query) else { continue };
            self.shown.push(i);
            items.push(ResultItem::new(f.name.clone()).subtitle(sample.clone()).subtitle_font(f.name.clone()).match_chars(&m));
        }
        self.list.set_items(theme, items, now);
        self.sync_selection(theme, now);
    }

    /// Seçili ailenin satırını ✓ ile işaretle, görünür kıl; ağırlık denetimlerini kur.
    fn sync_selection(&mut self, theme: &Theme, now: Instant) {
        let sel = self.family().map(|f| f.name.clone());
        let mut row = None;
        for (r, &i) in self.shown.iter().enumerate() {
            let on = sel.as_deref().is_some_and(|s| self.families[i].name == s);
            if on {
                row = Some(r);
            }
            self.list.set_item_icon(r, on.then(|| Icon::named(IconName::Check)));
        }
        if let Some(r) = row {
            self.list.set_selected(theme, r, now);
        }
        self.rebuild_weights(theme, now);
        self.update_preview(theme);
    }

    fn rebuild_weights(&mut self, theme: &Theme, now: Instant) {
        let weight = self.selected.as_ref().map_or(400, |(_, w)| *w);
        let available: Vec<u16> = self.family().map(|f| f.weights.clone()).unwrap_or_default();
        if let Some((lo, hi)) = self.range() {
            self.chip_weights.clear();
            self.weights = ChipGroup::new(Vec::new(), true);
            if self.slider_range != Some((lo, hi)) {
                self.slider_range = Some((lo, hi));
                let steps = u32::from((hi.saturating_sub(lo) / WEIGHT_STEP).max(1));
                let rect = self.slider.rect();
                self.slider = Slider::new(theme, Orientation::Horizontal, 0.0).with_steps(steps);
                self.slider.set_rect(rect);
            }
            let v = f32::from(weight.clamp(lo, hi) - lo) / f32::from((hi - lo).max(1));
            let _ = self.slider.set_value(v, now);
        } else if self.chip_weights != available {
            self.chip_weights = available;
            let chips = self
                .chip_weights
                .iter()
                .map(|&w| Chip::new(theme, ChipKind::Choice, &self.weight_name(w)).selected(w == weight))
                .collect();
            self.weights = ChipGroup::new(chips, true);
        } else {
            for (c, &w) in self.weights.chips_mut().iter_mut().zip(&self.chip_weights) {
                c.set_selected(w == weight, now);
            }
        }
        let name = self.weight_name(weight);
        self.weight_label.set_text(&name);
    }

    fn weight_name(&self, w: u16) -> String {
        if w.is_multiple_of(WEIGHT_STEP) && (WEIGHT_MIN..=WEIGHT_MAX).contains(&w) {
            self.labels.weight_names[usize::from(w / WEIGHT_STEP - 1)].clone()
        } else {
            w.to_string()
        }
    }

    fn update_preview(&mut self, theme: &Theme) {
        let base = text_style(theme, TextRole::Title).with_max_lines(1);
        let style = match &self.selected {
            Some((f, w)) => TextStyle { family: clip_text(f, 256).to_owned(), weight: *w, ..base },
            None => base,
        };
        self.preview.set_style(style);
    }

    /// Yerleşim: arama / filtre / liste (esnek) / ağırlık / önizleme.
    pub fn set_rect(&mut self, text: &mut TextSystem, theme: &Theme, rect: Rect, now: Instant) {
        self.rect = rect;
        let gap = theme.space(3);
        let sh = self.search.preferred_size(theme, rect.width).height;
        self.search.set_rect(theme, Rect::new(rect.x, rect.y, rect.width, sh));
        let mut y = rect.y + sh + gap;
        if self.show_filter {
            let fs = self.filter.preferred_size(text, theme);
            self.filter.set_rect(theme, Rect::new(rect.x, y, fs.width.min(rect.width), fs.height));
            y += fs.height + gap;
        }
        // Alttan yukarı: önizleme, ağırlık, sonra liste kalan alanı alır.
        let ph = self.preview.measure(text).height.max(theme.space(10));
        self.preview.set_max_width(Some(rect.width));
        let preview_y = rect.bottom() - ph;
        self.preview_origin = Point::new(rect.x, preview_y);
        let wh = if self.range().is_some() {
            self.slider.preferred_size(theme, rect.width).height.max(theme.space(10))
        } else {
            let probe = self.weights.layout(text, theme, Point::new(rect.x, 0.0), rect.width);
            probe.max(theme.space(10))
        };
        let wy = preview_y - gap - wh;
        self.weight_rect = Rect::new(rect.x, wy, rect.width, wh);
        if self.range().is_some() {
            let lw = theme.space(24);
            let ss = self.slider.preferred_size(theme, rect.width - lw - gap);
            self.slider.set_rect(Rect::new(rect.x + lw + gap, wy + (wh - ss.height) * 0.5, rect.width - lw - gap, ss.height));
        } else {
            let _ = self.weights.layout(text, theme, Point::new(rect.x, wy), rect.width);
        }
        let list = Rect::new(rect.x, y, rect.width, (wy - gap - y).max(0.0));
        let changed = self.list.rect() != list;
        self.list.set_rect(theme, list, now);
        // Seçim alan boyu bilinmeden kurulmuş olabilir: yerleşim değişince yeniden görünür kıl.
        if changed && let Some(r) = self.selected_row() {
            self.list.set_selected(theme, r, now);
        }
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    fn choose_family(&mut self, text: &mut TextSystem, theme: &Theme, row: usize, now: Instant) -> Response<FontEvent> {
        let Some(&i) = self.shown.get(row) else { return Response::none() };
        let f = &self.families[i];
        let want = self.selected.as_ref().map_or(400, |(_, w)| *w);
        let name = f.name.clone();
        let weight = match f.weight_range {
            Some((lo, hi)) => snap(want, lo, hi),
            None => nearest(&f.weights, want),
        };
        if self.selected.as_ref().is_some_and(|(n, w)| *n == name && *w == weight) {
            return Response::repaint();
        }
        self.selected = Some((name.clone(), weight));
        self.sync_selection(theme, now);
        let r = self.rect;
        self.set_rect(text, theme, r, now);
        Response::event(FontEvent::Changed { family: name, weight })
    }

    fn choose_weight(&mut self, theme: &Theme, weight: u16, now: Instant) -> Response<FontEvent> {
        let Some((name, w)) = &mut self.selected else { return Response::repaint() };
        if *w == weight {
            return Response::repaint();
        }
        *w = weight;
        let ev = FontEvent::Changed { family: name.clone(), weight };
        self.rebuild_weights(theme, now);
        self.update_preview(theme);
        Response::event(ev)
    }

    pub fn pointer(&mut self, text: &mut TextSystem, theme: &Theme, event: &PointerEvent, now: Instant) -> Response<FontEvent> {
        let mut out = Response::none();
        let r = self.search.pointer(text, theme, event, now);
        out.repaint |= r.repaint;
        if r.event == Some(TextInputEvent::Changed) {
            self.refilter(theme, now);
            return Response::repaint();
        }
        let r = if self.show_filter { self.filter.pointer(event, now) } else { Response::none() };
        out.repaint |= r.repaint;
        if r.event.is_some() {
            self.refilter(theme, now);
            return Response::repaint();
        }
        let r = self.list.pointer(theme, event, now);
        out.repaint |= r.repaint;
        if let Some(ResultEvent::Activated(row)) = r.event {
            return self.choose_family(text, theme, row, now);
        }
        if let Some((lo, hi)) = self.range() {
            let r = self.slider.pointer(event, now);
            out.repaint |= r.repaint;
            if let Some(v) = r.event {
                return self.choose_weight(theme, slider_weight(v, lo, hi), now);
            }
        } else {
            let r = self.weights.pointer(text, theme, event, now);
            out.repaint |= r.repaint;
            if let Some((i, ChipEvent::Toggled(_) | ChipEvent::Clicked)) = r.event
                && let Some(&w) = self.chip_weights.get(i)
            {
                return self.choose_weight(theme, w, now);
            }
        }
        out
    }

    /// Arama alanı odaktayken gezinme tuşları listeye, diğerleri aramaya gider.
    pub fn key(&mut self, text: &mut TextSystem, theme: &Theme, event: &KeyEvent, now: Instant) -> Response<FontEvent> {
        let nav = matches!(event.key, Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Enter);
        if nav {
            if event.state == KeyState::Released {
                return Response::none();
            }
            let r = self.list.key(theme, event, now);
            return match r.event {
                Some(ResultEvent::Activated(row)) => self.choose_family(text, theme, row, now),
                _ => r.discard(),
            };
        }
        let r = self.search.key(event, now);
        if r.event == Some(TextInputEvent::Changed) {
            self.refilter(theme, now);
            return Response::repaint();
        }
        r.discard()
    }

    pub fn cursor(&self, p: Point) -> Option<CursorShape> {
        if !self.rect.contains(p) {
            return None;
        }
        let w = if self.range().is_some() { self.slider.cursor(p) } else { self.weights.cursor(p) };
        self.search
            .cursor(p)
            .or_else(|| self.show_filter.then(|| self.filter.cursor(p)).flatten())
            .or_else(|| self.list.cursor(p))
            .or(w)
            .or(Some(CursorShape::Default))
    }

    pub fn draw(&mut self, ui: &mut Ui) {
        let theme = ui.theme;
        let c = &theme.colors;
        self.search.draw(ui);
        if self.show_filter {
            self.filter.draw(ui);
        }
        self.list.draw(ui);
        if self.selected.is_some() {
            if self.range().is_some() {
                let ls = self.weight_label.measure_in(ui.painter);
                let at = Point::new(self.weight_rect.x, self.weight_rect.center().y - ls.height * 0.5);
                self.weight_label.draw(ui.painter, at, ui.fade(c.on_surface_variant));
                self.slider.draw(ui);
            } else {
                self.weights.draw(ui);
            }
            self.preview.draw(ui.painter, self.preview_origin, ui.fade(c.on_surface));
        }
    }
}

fn slider_weight(v: f32, lo: u16, hi: u16) -> u16 {
    let w = f32::from(lo) + v.clamp(0.0, 1.0) * f32::from(hi.saturating_sub(lo));
    snap(w.round() as u16, lo, hi)
}

/// Değişken ailede ağırlık: 100'ün katına yuvarlanır, ailenin aralığına kırpılır.
fn snap(want: u16, lo: u16, hi: u16) -> u16 {
    let stepped = ((f32::from(want) / f32::from(WEIGHT_STEP)).round() as u16).saturating_mul(WEIGHT_STEP);
    stepped.clamp(lo, hi.max(lo))
}

/// `weights` içinde `want`'a en yakın (eşitlikte ağırı); boşsa `want`.
fn nearest(weights: &[u16], want: u16) -> u16 {
    weights.iter().copied().min_by_key(|&w| (w.abs_diff(want), u16::MAX - w)).unwrap_or(want)
}

/// Büyük/küçük harf duyarsız alt dizi eşleşmesi: eşleşen karakter indeksleri (boş sorgu: hepsi).
fn subsequence(name: &str, query: &[char]) -> Option<Vec<u32>> {
    let mut out = Vec::with_capacity(query.len());
    let mut q = query.iter().peekable();
    for (i, ch) in name.chars().enumerate() {
        let Some(&&want) = q.peek() else { break };
        if ch.to_lowercase().eq(std::iter::once(want)) {
            out.push(i as u32);
            q.next();
        }
    }
    q.peek().is_none().then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::test_util::*;

    fn fam(name: &str, weights: &[u16], mono: bool) -> FamilyInfo {
        FamilyInfo { name: name.into(), weights: weights.to_vec(), weight_range: None, italic: false, monospace: mono, latin: true }
    }

    fn labels() -> FontPickerLabels {
        FontPickerLabels {
            search: "Font ara".into(),
            all: "Tümü".into(),
            monospace: "Eş aralıklı".into(),
            sample: "Aa Ğğ Şş 0123".into(),
            weight_names: ["İnce", "Çok hafif", "Hafif", "Normal", "Orta", "Yarı kalın", "Kalın", "Çok kalın", "Siyah"].map(String::from),
        }
    }

    fn picker() -> (FontPicker, TextSystem, Theme, Instant) {
        let theme = Theme::default();
        let mut text = TextSystem::new();
        let t0 = Instant::now();
        let families = vec![
            fam("Alfa Sans", &[300, 400, 700], false),
            fam("Beta Mono", &[400, 700], true),
            fam("Gama Serif", &[400], false),
        ];
        let mut p = FontPicker::new(&theme, &families, labels(), Some(("Alfa Sans", 400)));
        p.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 400.0, 520.0), t0);
        (p, text, theme, t0)
    }

    #[test]
    fn search_and_filter() {
        let (mut p, mut text, theme, t0) = picker();
        assert_eq!(p.shown.len(), 3);
        for ch in "mno".chars() {
            let _ = p.key(&mut text, &theme, &typed(ch), t0);
        }
        assert_eq!(p.shown, vec![1], "alt dizi: Beta MoNO");
        assert_eq!(subsequence("Beta Mono", &['m', 'n', 'o']), Some(vec![5, 7, 8]));
        let _ = p.key(&mut text, &theme, &key(Key::Escape), t0);
        assert_eq!(p.shown.len(), 3, "Esc aramayı temizler");
        p.filter.set_selected(1, t0);
        p.refilter(&theme, t0);
        assert_eq!(p.shown, vec![1], "eş aralıklı filtre");
    }

    #[test]
    fn hidden_filter_takes_no_space_and_shows_all() {
        let (mut p, mut text, theme, t0) = picker();
        let with = p.list.rect();
        p.filter.set_selected(1, t0);
        let mut p = p.with_filter(&theme, false);
        assert_eq!(p.shown.len(), 3, "gizli filtre süzmez");
        p.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 400.0, 520.0), t0);
        assert!(p.list.rect().y < with.y, "liste filtrenin yerine çıkar");
        let fr = p.filter.rect().center();
        let _ = p.pointer(&mut text, &theme, &press(fr.x, fr.y), t0);
        let _ = p.pointer(&mut text, &theme, &release(fr.x, fr.y), t0);
        assert_eq!(p.shown.len(), 3, "eski filtre konumu tıklanamaz");
    }

    #[test]
    fn choosing_family_snaps_weight_and_marks_row() {
        let (mut p, mut text, theme, t0) = picker();
        assert_eq!(p.list.selected(), Some(0), "seçili aile vurgulu");
        let _ = p.choose_weight(&theme, 300, t0);
        let _ = p.key(&mut text, &theme, &key(Key::Down), t0);
        let ev = p.key(&mut text, &theme, &key(Key::Enter), t0).event;
        assert_eq!(ev, Some(FontEvent::Changed { family: "Beta Mono".into(), weight: 400 }), "300 → en yakın 400");
        assert_eq!(p.chip_weights, vec![400, 700], "yalnız ailede bulunan ağırlıklar");
        let ev = p.choose_weight(&theme, 700, t0).event;
        assert_eq!(ev, Some(FontEvent::Changed { family: "Beta Mono".into(), weight: 700 }));
        assert_eq!(p.weight_label.text(), "Kalın");
    }

    #[test]
    fn variable_range_and_symbol_fonts() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let families = vec![
            FamilyInfo { weight_range: Some((300, 800)), ..fam("Esnek", &[400], false) },
            FamilyInfo { latin: false, ..fam("Simgeler", &[400], false) },
        ];
        let mut p = FontPicker::new(&theme, &families, labels(), None);
        assert_eq!(p.shown, vec![0], "latin olmayan aile gizli");
        p.set_selected(&theme, "Esnek", 950, t0);
        assert_eq!(p.range(), Some((300, 800)));
        assert!(p.chip_weights.is_empty(), "değişkende çip yok, kaydırıcı var");
        p.selected = Some(("Başka".into(), 900));
        let ev = p.choose_family(&mut TextSystem::new(), &theme, 0, t0).event;
        assert_eq!(ev, Some(FontEvent::Changed { family: "Esnek".into(), weight: 800 }), "aralığa oturur");
    }

    #[test]
    fn helpers() {
        assert_eq!(nearest(&[300, 500], 400), 500, "eşitlikte ağır");
        assert_eq!(nearest(&[], 450), 450);
        assert_eq!(slider_weight(0.0, 100, 900), 100);
        assert_eq!(slider_weight(0.51, 100, 900), 500);
        assert_eq!(slider_weight(2.0, 100, 900), 900);
        assert_eq!(slider_weight(1.0, 300, 800), 800);
        assert_eq!(snap(950, 100, 900), 900);
        assert_eq!(snap(240, 250, 700), 250, "aralık dışı uca kırpılır");
    }
}
