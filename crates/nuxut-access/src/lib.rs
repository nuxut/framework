//! Erişilebilirlik ağacı (ADR-0011): bileşenlerden AccessKit düğümleri.
//!
//! Uygulama her yüzey için [`AccessTree`]'yi `App::accessibility`'de doldurur; runtime bunu
//! AT-SPI'ye (Orca) taşır. Ağaç **tam** kurulur (görünür her düğüm), fark ve olaylar
//! AccessKit'te üretilir. Runtime ağacı yalnızca bir yardımcı teknoloji etkinken ister; kapalıyken
//! bu modülün hiçbir kodu çalışmaz.
//!
//! Kimlik: düğüm kimliği (ebeveyn kimliği, uygulama anahtarı) karmasıdır ([`child_id`]); aynı
//! anahtar her kurulumda aynı kimliği verir. Liste öğeleri indisle değil öğenin kendi kimliğiyle
//! anahtarlanmalı (masaüstü kimliği, bildirim id'si…): liste yeniden sıralanınca Orca odak ve
//! olayları karıştırmasın.
//!
//! Gizlilik (değişmez 9): ad/değer/açıklama kullanıcı içeriği taşıyabilir; bu modülün `Debug`
//! uygulamaları yalnız sayı ve rol yazar. Parola alanı yalnız maske verir.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

pub use accesskit;
pub use accesskit::{Action, Live, Node, NodeId, Role, Toggled};
use accesskit::{ActionData, TextPosition, TextSelection, TreeId, TreeInfo, TreeUpdate};
use nuxut_core::Rect;
use unicode_segmentation::UnicodeSegmentation;

/// Her yüzey ağacının kök düğümü (`Role::Window`).
pub const ROOT_ID: NodeId = NodeId(0);

/// AT-SPI'de görünen araç takımı adı.
pub const TOOLKIT_NAME: &str = "nuxut";

/// Parola maskesinin karakteri (görsel maskeyle aynı).
pub const MASK_CHAR: char = '•';

/// Ad, açıklama ve (metin alanı dışı) değerin en uzun hâli (bayt; grafem sınırında kesilir).
/// Bildirim gövdesi gibi dış içerik ağaçtan topluca dökülemesin; ekran okuyucu için fazlası
/// zaten okunmaz. Düzenlenen metin (`text_content`) sınırsızdır (kullanıcı onu yazıyor).
pub const MAX_ACCESS_TEXT: usize = 512;

/// Pano geçmişi gibi hassas listeler için kısa önizleme varsayılanı (grafem).
pub const PREVIEW_GRAPHEMES: usize = 40;

/// `s`'i en çok `max` bayta, grafem sınırında keser; kesildiyse sonuna "…" ekler.
pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let mut end = 0;
    for (i, g) in s.grapheme_indices(true) {
        if i + g.len() > max.saturating_sub('…'.len_utf8()) {
            break;
        }
        end = i + g.len();
    }
    format!("{}…", &s[..end])
}

/// Hassas metnin kısa önizlemesi: ilk satır, boşluklar sadeleşmiş, en çok `graphemes` grafem
/// ("…" ile). Pano geçmişi öğesi ağaca tam metin yerine bununla konur (ADR-0011 tehdit modeli).
pub fn preview(s: &str, graphemes: usize) -> String {
    let line = s.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let flat: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (n, g) in flat.graphemes(true).enumerate() {
        if n >= graphemes {
            out.push('…');
            return out;
        }
        out.push_str(g);
    }
    if s.lines().filter(|l| !l.trim().is_empty()).count() > 1 {
        out.push('…');
    }
    out
}

/// `parent` altındaki `key` anahtarlı düğümün kimliği. Kurulumda kullanılan kimlikle aynıdır:
/// uygulama `App::accessibility_action`'da gelen isteği bununla karşılaştırır.
pub fn child_id(parent: NodeId, key: impl Hash) -> NodeId {
    // `DefaultHasher::new()` sabit anahtarlı SipHash: süreç içinde (ve sürümler arası aynı
    // derlemede) belirlenimci.
    let mut h = DefaultHasher::new();
    parent.0.hash(&mut h);
    key.hash(&mut h);
    match h.finish() {
        0 => NodeId(1),
        id => NodeId(id),
    }
}

/// Kökün doğrudan çocuğunun kimliği (`child_id(ROOT_ID, key)`).
pub fn root_child(key: impl Hash) -> NodeId {
    child_id(ROOT_ID, key)
}

/// Adı olması gereken (etkileşimli) roller: adsız kalırsa Orca yalnız rolü okur.
fn needs_label(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::Switch
            | Role::CheckBox
            | Role::RadioButton
            | Role::Slider
            | Role::TextInput
            | Role::PasswordInput
            | Role::SearchInput
            | Role::ComboBox
            | Role::MenuItem
            | Role::ListBoxOption
            | Role::Tab
            | Role::Link
            | Role::Image
    )
}

/// Bir yüzeyin erişilebilirlik ağacı kurucusu.
pub struct AccessTree {
    nodes: Vec<(NodeId, Node)>,
    index: HashMap<NodeId, usize>,
    /// Açık kapların indisleri; boşsa ebeveyn köktür.
    parents: Vec<usize>,
    focus: Option<NodeId>,
    /// Yinelenen kimlikli düğüm buraya yazılır ve atılır (release; debug'da panik).
    scratch: Node,
    duplicates: usize,
}

impl Default for AccessTree {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for AccessTree {
    // Değişmez 9: düğüm içeriği yok, yalnız sayılar.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessTree")
            .field("nodes", &self.nodes.len())
            .field("focus", &self.focus.map(|n| n.0))
            .field("duplicates", &self.duplicates)
            .finish()
    }
}

impl AccessTree {
    /// Yalnız kök (`Role::Window`, adsız) düğümlü ağaç.
    pub fn new() -> Self {
        let mut index = HashMap::new();
        index.insert(ROOT_ID, 0);
        Self {
            nodes: vec![(ROOT_ID, Node::new(Role::Window))],
            index,
            parents: Vec::new(),
            focus: None,
            scratch: Node::new(Role::Unknown),
            duplicates: 0,
        }
    }

    /// Kök düğüm (yüzeyin penceresi): adını uygulama verir (`tree.root().label(&t!(..))`).
    pub fn root(&mut self) -> AccessNode<'_> {
        AccessNode { tree: self, slot: Some(0), id: ROOT_ID }
    }

    /// Şu anki ebeveyn (kök ya da [`AccessTree::within`] kabı).
    pub fn parent(&self) -> NodeId {
        self.parents.last().map_or(ROOT_ID, |&i| self.nodes[i].0)
    }

    /// Şu anki ebeveyn altında `key` anahtarlı düğümün kimliği (eklemeden).
    pub fn id_for(&self, key: impl Hash) -> NodeId {
        child_id(self.parent(), key)
    }

    /// Şu anki ebeveynin altına `role` rollü düğüm ekler. Aynı ebeveyn altında aynı anahtar iki
    /// kez verilirse debug'da panik; release'te ikinci düğüm atılır (AT-SPI nesne yolu çakışması
    /// accesskit_unix'te abort olurdu).
    pub fn push(&mut self, key: impl Hash, role: Role) -> AccessNode<'_> {
        let id = self.id_for(key);
        self.push_id(id, role)
    }

    fn push_id(&mut self, id: NodeId, role: Role) -> AccessNode<'_> {
        if self.index.contains_key(&id) {
            self.duplicates += 1;
            debug_assert!(false, "erişilebilirlik: yinelenen düğüm kimliği (aynı ebeveyn altında aynı anahtar)");
            self.scratch = Node::new(role);
            return AccessNode { tree: self, slot: None, id };
        }
        let parent = self.parents.last().copied().unwrap_or(0);
        self.nodes[parent].1.push_child(id);
        self.nodes.push((id, Node::new(role)));
        let slot = self.nodes.len() - 1;
        self.index.insert(id, slot);
        AccessNode { tree: self, slot: Some(slot), id }
    }

    /// `f` içinde eklenen düğümler `parent`'ın çocukları olur. `parent` bu ağaçta olmalı
    /// (yoksa düğümler şu anki ebeveynde kalır).
    pub fn within<R>(&mut self, parent: NodeId, f: impl FnOnce(&mut Self) -> R) -> R {
        let Some(&slot) = self.index.get(&parent) else {
            debug_assert!(false, "erişilebilirlik: within için bilinmeyen ebeveyn");
            return f(self);
        };
        self.parents.push(slot);
        let r = f(self);
        self.parents.pop();
        r
    }

    /// Mevcut bir düğümü yeniden düzenlemek için.
    pub fn get(&mut self, id: NodeId) -> Option<AccessNode<'_>> {
        let slot = *self.index.get(&id)?;
        Some(AccessNode { tree: self, slot: Some(slot), id })
    }

    /// Düğümü salt okunur verir (test, denetim).
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.index.get(&id).map(|&i| &self.nodes[i].1)
    }

    /// Kök dahil tüm düğümler, ekleme sırasıyla (ör. tıklanabilir düğümlerin sınırlarından
    /// AT eylemi → işaretçi noktası eşlemesi kurmak için).
    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().map(|(id, n)| (*id, n))
    }

    /// Klavye odağındaki düğüm. Verilmezse kök (yüzeyde odaklı bileşen yok).
    pub fn set_focus(&mut self, id: NodeId) {
        self.focus = Some(id);
    }

    pub fn focus(&self) -> NodeId {
        self.focus.unwrap_or(ROOT_ID)
    }

    /// Kök dahil düğüm sayısı.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Yalnız kök var mı.
    pub fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Atılan yinelenen düğüm sayısı (release).
    pub fn duplicates(&self) -> usize {
        self.duplicates
    }

    /// Adı olması gerektiği hâlde adsız (etkileşimli) düğümler: simgeli düğme, anahtar,
    /// kaydırıcı… Testlerde boş olduğu doğrulanır.
    pub fn unlabeled(&self) -> Vec<NodeId> {
        let labeled: std::collections::HashSet<NodeId> = self
            .nodes
            .iter()
            .filter(|(_, n)| n.label().is_some_and(|l| !l.trim().is_empty()))
            .map(|(id, _)| *id)
            .collect();
        self.nodes
            .iter()
            .filter(|(id, n)| {
                needs_label(n.role())
                    && !labeled.contains(id)
                    && !n.labelled_by().iter().any(|l| labeled.contains(l) || self.is_value_label(*l))
            })
            .map(|(id, _)| *id)
            .collect()
    }

    fn is_value_label(&self, id: NodeId) -> bool {
        self.node(id).is_some_and(|n| n.role() == Role::Label && n.value().is_some())
    }

    /// Tam ağaç güncellemesi (AccessKit). Runtime çağırır.
    pub fn finish(self) -> TreeUpdate {
        let focus = self.focus.filter(|f| self.index.contains_key(f)).unwrap_or(ROOT_ID);
        let mut tree = TreeInfo::new(ROOT_ID);
        tree.toolkit_name = Some(TOOLKIT_NAME.into());
        tree.toolkit_version = Some(env!("CARGO_PKG_VERSION").into());
        TreeUpdate { nodes: self.nodes, tree: Some(tree), tree_id: TreeId::ROOT, focus }
    }
}

/// Ağaçtaki bir düğümün düzenleyicisi (zincirlenir).
pub struct AccessNode<'t> {
    tree: &'t mut AccessTree,
    /// `None`: yinelenen kimlik, düğüm atılacak.
    slot: Option<usize>,
    id: NodeId,
}

impl fmt::Debug for AccessNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessNode").field("id", &self.id.0).finish()
    }
}

impl<'t> AccessNode<'t> {
    pub fn id(&self) -> NodeId {
        self.id
    }

    /// Ham AccessKit düğümü (bu modülün kapsamadığı özellikler için).
    pub fn node_mut(&mut self) -> &mut Node {
        match self.slot {
            Some(i) => &mut self.tree.nodes[i].1,
            None => &mut self.tree.scratch,
        }
    }

    /// Erişilebilir ad. Boş dizge adı temizler.
    pub fn label(mut self, label: &str) -> Self {
        let label = label.trim();
        if label.is_empty() {
            self.node_mut().clear_label();
        } else {
            self.node_mut().set_label(clip(label, MAX_ACCESS_TEXT));
        }
        self
    }

    /// Ad yoksa verilen adı koyar (bileşenin görünen metni yoksa çağıranın adı).
    pub fn label_or(mut self, label: &str) -> Self {
        if self.node_mut().label().is_none() {
            self = self.label(label);
        }
        self
    }

    /// Ek açıklama (ipucu, alt metin).
    pub fn description(mut self, text: &str) -> Self {
        if text.trim().is_empty() {
            self.node_mut().clear_description();
        } else {
            self.node_mut().set_description(clip(text.trim(), MAX_ACCESS_TEXT));
        }
        self
    }

    /// Metin değeri (ör. "%64", "Bağlı"). `Role::Label`'da görünen metin budur.
    pub fn value(mut self, text: &str) -> Self {
        self.node_mut().set_value(clip(text, MAX_ACCESS_TEXT));
        self
    }

    /// Sayısal değer (kaydırıcı, ilerleme). `step` 0 ise adımsız.
    pub fn numeric(mut self, value: f64, min: f64, max: f64, step: f64) -> Self {
        // Bileşen değerleri f32: 0,05 → 0.05000000074 okunmasın (6 ondalık yeter).
        let tidy = |v: f64| (v * 1e6).round() / 1e6;
        let node = self.node_mut();
        node.set_numeric_value(tidy(value));
        node.set_min_numeric_value(tidy(min));
        node.set_max_numeric_value(tidy(max));
        if step > 0.0 {
            node.set_numeric_value_step(tidy(step));
        }
        self
    }

    pub fn toggled(mut self, on: bool) -> Self {
        self.node_mut().set_toggled(if on { Toggled::True } else { Toggled::False });
        self
    }

    pub fn selected(mut self, on: bool) -> Self {
        self.node_mut().set_selected(on);
        self
    }

    pub fn expanded(mut self, on: bool) -> Self {
        self.node_mut().set_expanded(on);
        self
    }

    /// `false` iken devre dışı işaretler ve etkinleştirme eylemlerini kaldırmaz (AccessKit
    /// devre dışı düğümün eylemini AT'ye zaten göstermez).
    pub fn enabled(mut self, on: bool) -> Self {
        if !on {
            self.node_mut().set_disabled();
        }
        self
    }

    /// Yüzeye göre mantıksal sınır (son `set_rect`).
    pub fn bounds(mut self, r: Rect) -> Self {
        if r.width > 0.0 && r.height > 0.0 {
            self.node_mut().set_bounds(to_rect(r));
        }
        self
    }

    pub fn action(mut self, action: Action) -> Self {
        self.node_mut().add_action(action);
        self
    }

    /// Tıklanabilir + odaklanabilir (düğme, anahtar, satır).
    pub fn clickable(self) -> Self {
        self.action(Action::Click).action(Action::Focus)
    }

    /// Canlı bölge: ad/değer değişince AT duyurur (OSD, bildirim, sonuç sayısı).
    pub fn live(mut self, live: Live) -> Self {
        self.node_mut().set_live(live);
        self
    }

    /// Klavye kısayolu metni (ör. "Super+Alt+S").
    pub fn shortcut(mut self, text: &str) -> Self {
        self.node_mut().set_keyboard_shortcut(text);
        self
    }

    /// Kümedeki konum (1 tabanlı) ve küme boyutu: "3 / 12".
    pub fn position(mut self, pos: usize, of: usize) -> Self {
        let node = self.node_mut();
        node.set_position_in_set(pos);
        node.set_size_of_set(of);
        self
    }

    /// Ad başka düğümden gelir (form satırının etiketi).
    pub fn labelled_by(mut self, id: NodeId) -> Self {
        self.node_mut().push_labelled_by(id);
        self
    }

    /// Odak bu düğümde kalırken AT'ye okunan etkin öğe (arama alanı + sonuç listesi).
    pub fn active_descendant(mut self, id: NodeId) -> Self {
        self.node_mut().set_active_descendant(id);
        self
    }

    /// Bu düğüm klavye odağında.
    pub fn focused(self) -> Self {
        self.focused_if(true)
    }

    pub fn focused_if(self, on: bool) -> Self {
        if on && self.slot.is_some() {
            self.tree.focus = Some(self.id);
        }
        self
    }

    /// Düzenlenebilir metnin içeriği: tek `TextRun` çocuğu + imleç/seçim. `caret`, `anchor`
    /// bayt ofseti (karakter sınırında). `mask` doğruysa (parola) metin yerine her grafem için
    /// [`MASK_CHAR`] verilir: uzunluk dışında bilgi yok (karar 6, 2026-10-13).
    pub fn text_content(mut self, text: &str, caret: usize, anchor: usize, mask: bool) -> Self {
        let bounds = self.node_mut().bounds();
        let run_id = child_id(self.id, "text-run");
        let (shown, lengths, caret_i, anchor_i) = text_run(text, caret, anchor, mask);
        if !mask {
            self.node_mut().set_value(text);
        } else {
            self.node_mut().set_value(shown.as_str());
        }
        if self.slot.is_none() {
            return self;
        }
        let mut run = Node::new(Role::TextRun);
        run.set_value(shown);
        run.set_character_lengths(lengths);
        if let Some(b) = bounds {
            run.set_bounds(b);
        }
        let pos = |i| TextPosition { node: run_id, character_index: i };
        self.node_mut().set_text_selection(TextSelection { anchor: pos(anchor_i), focus: pos(caret_i) });
        let me = self.id;
        if !self.tree.index.contains_key(&run_id) {
            self.tree.within(me, |t| {
                t.push_id(run_id, Role::TextRun).node_mut().clone_from(&run);
            });
        }
        self
    }
}

/// (gösterilen metin, grafem başına UTF-8 uzunluğu, imleç ve çapa grafem indisi).
fn text_run(text: &str, caret: usize, anchor: usize, mask: bool) -> (String, Vec<u8>, usize, usize) {
    let mut shown = String::new();
    let mut lengths = Vec::new();
    let (mut caret_i, mut anchor_i) = (None, None);
    for (i, (start, g)) in text.grapheme_indices(true).enumerate() {
        if caret_i.is_none() && caret <= start {
            caret_i = Some(i);
        }
        if anchor_i.is_none() && anchor <= start {
            anchor_i = Some(i);
        }
        let piece: &str = if mask { "\u{2022}" } else { g };
        // Uzunluk u8: 255 baytı aşan grafem (aşırı birleştirici dizisi) bölünmez, kırpılır.
        let len = piece.len().min(u8::MAX as usize);
        shown.push_str(&piece[..floor_char(piece, len)]);
        lengths.push(floor_char(piece, len) as u8);
    }
    let n = lengths.len();
    (shown, lengths, caret_i.unwrap_or(n), anchor_i.unwrap_or(n))
}

fn floor_char(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn to_rect(r: Rect) -> accesskit::Rect {
    accesskit::Rect {
        x0: f64::from(r.x),
        y0: f64::from(r.y),
        x1: f64::from(r.x + r.width),
        y1: f64::from(r.y + r.height),
    }
}

/// Uygulamaya iletilen AT eylemi. Metin yazan eylemler (`SetValue` dizge,
/// `ReplaceSelectedText`) **hiç** iletilmez (ADR-0011 karar 6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AccessAction {
    /// Etkinleştir (düğmeye bas, anahtarı çevir, satırı aç).
    Click,
    /// Klavye odağını bu düğüme taşı.
    Focus,
    Increment,
    Decrement,
    /// Sayısal değer (kaydırıcı); düğümün `min..=max` aralığında olduğu doğrulanmalı.
    SetValue(f64),
    Expand,
    Collapse,
    /// Düğümü kaydırma alanında görünür kıl.
    ScrollIntoView,
}

/// AT'den gelen istek: hangi düğüm, hangi eylem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AccessRequest {
    pub node: NodeId,
    pub action: AccessAction,
}

impl AccessRequest {
    /// AccessKit isteğinden; desteklenmeyen ya da metin yazan eylemde `None`.
    pub fn from_accesskit(req: &accesskit::ActionRequest) -> Option<Self> {
        let action = match (req.action, &req.data) {
            (Action::Click, _) => AccessAction::Click,
            (Action::Focus, _) => AccessAction::Focus,
            (Action::Increment, _) => AccessAction::Increment,
            (Action::Decrement, _) => AccessAction::Decrement,
            (Action::SetValue, Some(ActionData::NumericValue(v))) if v.is_finite() => AccessAction::SetValue(*v),
            (Action::Expand, _) => AccessAction::Expand,
            (Action::Collapse, _) => AccessAction::Collapse,
            (Action::ScrollIntoView, _) => AccessAction::ScrollIntoView,
            _ => return None,
        };
        Some(Self { node: req.target_node, action })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_scoped() {
        assert_eq!(child_id(ROOT_ID, "a"), child_id(ROOT_ID, "a"));
        assert_ne!(child_id(ROOT_ID, "a"), child_id(ROOT_ID, "b"));
        let p = root_child("list");
        assert_ne!(child_id(p, "a"), child_id(ROOT_ID, "a"));
        assert_ne!(child_id(ROOT_ID, 0u64), ROOT_ID);
    }

    #[test]
    fn tree_builds_parents_and_focus() {
        let mut t = AccessTree::new();
        t.root().label("Bar");
        let list = t.push("list", Role::List).label("Liste").id();
        let item = t.within(list, |t| t.push("x", Role::ListItem).label("X").focused().id());
        assert_eq!(item, child_id(list, "x"));
        assert_eq!(t.node(list).unwrap().children(), &[item]);
        assert_eq!(t.node(ROOT_ID).unwrap().children(), &[list]);
        let u = t.finish();
        assert_eq!(u.focus, item);
        assert_eq!(u.nodes.len(), 3);
        assert_eq!(u.tree.unwrap().toolkit_name.as_deref(), Some(TOOLKIT_NAME));
    }

    #[test]
    fn unlabeled_finds_nameless_controls() {
        let mut t = AccessTree::new();
        let b = t.push("b", Role::Button).id();
        t.push("ok", Role::Button).label("Tamam");
        let lbl = t.push("l", Role::Label).value("Ses").id();
        t.push("s", Role::Slider).labelled_by(lbl);
        t.push("g", Role::Group);
        assert_eq!(t.unlabeled(), vec![b]);
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn duplicate_is_dropped_in_release() {
        let mut t = AccessTree::new();
        t.push("a", Role::Button).label("A");
        t.push("a", Role::Button).label("B");
        assert_eq!(t.duplicates(), 1);
        assert_eq!(t.len(), 2);
    }

    #[test]
    #[should_panic(expected = "yinelenen")]
    #[cfg(debug_assertions)]
    fn duplicate_panics_in_debug() {
        let mut t = AccessTree::new();
        t.push("a", Role::Button);
        t.push("a", Role::Button);
    }

    #[test]
    fn text_run_maps_bytes_to_graphemes_and_masks() {
        let (s, l, c, a) = text_run("aé🇹🇷b", 3, 1, false);
        assert_eq!(s, "aé🇹🇷b");
        assert_eq!(l, vec![1, 2, 8, 1]);
        assert_eq!((c, a), (2, 1));
        let (s, l, c, _) = text_run("gizli", 5, 5, true);
        assert_eq!(s, "•••••");
        assert_eq!(l, vec![3; 5]);
        assert_eq!(c, 5);
    }

    #[test]
    fn password_text_never_reaches_tree() {
        let mut t = AccessTree::new();
        let id = t.push("pw", Role::PasswordInput).label("Parola").text_content("hunter2", 7, 7, true).id();
        let u = t.finish();
        for (_, n) in &u.nodes {
            assert!(!n.value().unwrap_or_default().contains("hunter"));
            assert!(!n.label().unwrap_or_default().contains("hunter"));
        }
        let field = u.nodes.iter().find(|(i, _)| *i == id).unwrap();
        assert_eq!(field.1.value(), Some("•••••••"));
    }

    #[test]
    fn long_text_is_clipped_and_previews_are_short() {
        let long = "ğ".repeat(1000);
        let mut t = AccessTree::new();
        let id = t.push("n", Role::Group).label(&long).description(&long).value(&long).id();
        let n = t.node(id).unwrap();
        for s in [n.label().unwrap(), n.description().unwrap(), n.value().unwrap()] {
            assert!(s.len() <= MAX_ACCESS_TEXT && s.ends_with('…'));
        }
        assert_eq!(clip("kısa", 10), "kısa");
        assert_eq!(preview("  gizli   anahtar\nikinci satır", 40), "gizli anahtar…");
        assert_eq!(preview("abcdef", 3), "abc…");
        assert_eq!(preview("tek", 40), "tek");
    }

    #[test]
    fn text_actions_are_never_forwarded() {
        let base = accesskit::ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node: NodeId(5),
            data: Some(ActionData::Value("parola".into())),
        };
        assert_eq!(AccessRequest::from_accesskit(&base), None);
        let replace = accesskit::ActionRequest { action: Action::ReplaceSelectedText, ..base.clone() };
        assert_eq!(AccessRequest::from_accesskit(&replace), None);
        let num = accesskit::ActionRequest { data: Some(ActionData::NumericValue(0.5)), ..base.clone() };
        assert_eq!(AccessRequest::from_accesskit(&num).unwrap().action, AccessAction::SetValue(0.5));
        let nan = accesskit::ActionRequest { data: Some(ActionData::NumericValue(f64::NAN)), ..base };
        assert_eq!(AccessRequest::from_accesskit(&nan), None);
    }
}
