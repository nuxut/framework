//! Bileşenlerin erişilebilirlik düğümleri (ADR-0011): D-Bus'sız, `accesskit_consumer::Tree`
//! üzerinde. Tüketici geçersiz ağaçta (yetim düğüm, bilinmeyen çocuk) panikler; bu testler hem
//! eşlemeyi hem ağacın geçerliliğini doğrular.

use std::time::Instant;

use crate::access::Toggled;
use accesskit_consumer::{NodeRef, Tree};
use nuxut_core::Rect;
use nuxut_render::TextSystem;
use nuxut_theme::Theme;

use crate::access::{AccessAction, AccessRequest, AccessTree, Live, Role, child_id, root_child};
use crate::{
    BodySpan, Button, ButtonVariant, EmptyState, Icon, IconButton, IconButtonVariant, IconName, IconView, Label, List, ListItem, ListTrailing,
    NotificationCard, NotificationContent, NotificationEvent, Orientation, ResultItem, ResultList, Segment, SegmentedButton, Slider, TextInput, Toggle,
};

fn find<'a>(node: NodeRef<'a>, pred: &dyn Fn(&NodeRef<'a>) -> bool) -> Option<NodeRef<'a>> {
    if pred(&node) {
        return Some(node);
    }
    node.children().find_map(|c| find(c, pred))
}

fn by_label<'a>(tree: &'a Tree, label: &str) -> NodeRef<'a> {
    find(tree.state().root(), &|n| n.label().as_deref() == Some(label)).unwrap_or_else(|| panic!("düğüm yok: {label}"))
}

fn by_role(tree: &Tree, role: Role) -> NodeRef<'_> {
    find(tree.state().root(), &|n| n.role() == role).unwrap_or_else(|| panic!("rol yok: {role:?}"))
}

fn rect(x: f32, y: f32) -> Rect {
    Rect::new(x, y, 120.0, 40.0)
}

#[test]
fn controls_map_to_roles_names_states() {
    let theme = Theme::default();
    let now = Instant::now();
    let mut t = AccessTree::new();
    t.root().label("Ayarlar");

    let mut ok = Button::new(&theme, ButtonVariant::Filled).with_label(&theme, "Kaydet");
    ok.set_rect(rect(0.0, 0.0));
    ok.access(&mut t, "save");

    let mut close = IconButton::new(&theme, Icon::named(IconName::Close), IconButtonVariant::Standard);
    close.set_rect(rect(0.0, 40.0));
    close.access(&mut t, "close").label("Kapat");

    let mut sw = Toggle::new(&theme, true);
    sw.set_rect(rect(0.0, 80.0));
    sw.set_enabled(false, now);
    sw.access(&mut t, "wifi").label("Wi-Fi");

    let mut vol = Slider::new(&theme, Orientation::Horizontal, 0.25).with_steps(20);
    vol.set_rect(rect(0.0, 120.0));
    vol.access(&mut t, "vol", 0.0..=100.0).label("Ses");

    let mut seg = SegmentedButton::new(&theme, vec![Segment::new("Açık"), Segment::new("Koyu"), Segment::new("Otomatik")], 1);
    seg.set_rect(&theme, Rect::new(0.0, 160.0, 300.0, 40.0));
    let group = seg.access(&mut t, "mode", true).label("Görünüm").id();

    let icon_label = Label::new("Pil", crate::typography::text_style(&theme, crate::typography::TextRole::Body));
    icon_label.access(&mut t, "battery-label", rect(0.0, 200.0));
    IconView::new(&theme, Icon::named(IconName::Battery50)).access(&mut t, "battery", "Pil %50", rect(0.0, 240.0));

    assert!(t.unlabeled().is_empty(), "adsız denetim: {:?}", t.unlabeled());
    let tree = Tree::new(t.finish(), true);

    let save = by_label(&tree, "Kaydet");
    assert_eq!(save.role(), Role::Button);
    assert!(save.is_clickable(&accesskit_consumer::common_filter));

    assert_eq!(by_label(&tree, "Kapat").role(), Role::Button);

    let wifi = by_label(&tree, "Wi-Fi");
    assert_eq!((wifi.role(), wifi.toggled(), wifi.is_disabled()), (Role::Switch, Some(Toggled::True), true));

    let ses = by_label(&tree, "Ses");
    assert_eq!(ses.role(), Role::Slider);
    assert_eq!(ses.numeric_value(), Some(25.0));
    assert_eq!(ses.numeric_value_step(), Some(5.0));

    let radios: Vec<_> = by_label(&tree, "Görünüm").children().collect();
    assert_eq!(radios.len(), 3);
    assert!(radios.iter().all(|r| r.role() == Role::RadioButton));
    assert_eq!(radios[1].toggled(), Some(Toggled::True));
    assert_eq!(radios[0].toggled(), Some(Toggled::False));
    // Odaktaki bölümlü düğmede odak seçili seçenekte.
    assert_eq!(tree.state().focus().unwrap().label().as_deref(), Some("Koyu"));

    assert_eq!(find(tree.state().root(), &|n| n.role() == Role::Label).unwrap().value().as_deref(), Some("Pil"));
    assert_eq!(by_role(&tree, Role::Image).label().as_deref(), Some("Pil %50"));

    // AT eylemleri bileşen durumunu klavye/tıklamayla aynı yoldan değiştirir.
    let req = |node, action| AccessRequest { node, action };
    assert_eq!(seg.access_action(group, req(SegmentedButton::segment_id(group, 2), AccessAction::Click), now).event, Some(2));
    assert_eq!(seg.selected(), 2);
    assert_eq!(vol.access_action(AccessAction::Increment, 0.0..=100.0, now).event, Some(0.3));
    assert_eq!(vol.access_action(AccessAction::SetValue(80.0), 0.0..=100.0, now).event, Some(0.8));
    let mut on = Toggle::new(&theme, false);
    assert_eq!(on.access_action(AccessAction::Click, now).event, Some(true));
    assert!(on.is_on());
}

#[test]
fn icon_button_without_name_is_reported() {
    let theme = Theme::default();
    let mut t = AccessTree::new();
    let b = IconButton::new(&theme, Icon::named(IconName::Close), IconButtonVariant::Standard);
    let id = b.access(&mut t, "x").id();
    assert_eq!(t.unlabeled(), vec![id]);
}

#[test]
fn text_input_exposes_text_caret_and_masks_password() {
    let theme = Theme::default();
    let now = Instant::now();
    let mut t = AccessTree::new();

    let mut search = TextInput::new(&theme).with_placeholder("Ara");
    search.set_text("firefox");
    search.set_rect(Rect::new(0.0, 0.0, 300.0, 40.0));
    search.set_focused(true, now);
    search.access(&mut t, "search").label("Uygulama ara");

    let mut pw = TextInput::new(&theme).password(true).with_reveal_toggle(&theme, true);
    pw.set_reveal_labels("Parolayı göster", "Parolayı gizle");
    pw.set_text("hunter2");
    pw.set_rect(Rect::new(0.0, 50.0, 300.0, 40.0));
    let pw_id = pw.access(&mut t, "pw").label("Parola").id();

    let update = t.finish();
    for (_, n) in &update.nodes {
        assert!(!n.value().unwrap_or_default().contains("hunter"), "parola ağaçta");
    }
    let tree = Tree::new(update, true);
    let s = by_label(&tree, "Uygulama ara");
    assert_eq!(s.role(), Role::TextInput);
    assert_eq!(s.value().as_deref(), Some("firefox"));
    assert!(s.supports_text_ranges());
    assert_eq!(tree.state().focus().unwrap().label().as_deref(), Some("Uygulama ara"));
    assert_eq!(s.data().placeholder(), Some("Ara"));

    let p = by_label(&tree, "Parola");
    assert_eq!(p.role(), Role::PasswordInput);
    assert_eq!(p.value().as_deref(), Some("•••••••"));
    let reveal = by_label(&tree, "Parolayı göster");
    assert_eq!(reveal.toggled(), Some(Toggled::False));

    assert!(pw.access_action(pw_id, AccessRequest { node: child_id(pw_id, "reveal"), action: AccessAction::Click }));
    assert!(pw.is_revealed());
    // Gösterilirken de maske.
    let mut t = AccessTree::new();
    pw.access(&mut t, "pw").label("Parola");
    let u = t.finish();
    assert!(u.nodes.iter().all(|(_, n)| !n.value().unwrap_or_default().contains("hunter")));
}

#[test]
fn list_rows_and_results_listbox() {
    let theme = Theme::default();
    let now = Instant::now();
    let mut t = AccessTree::new();

    let mut list = List::new(&theme)
        .with_items(vec![
            ListItem::new(&theme, "Wi-Fi").with_trailing(&theme, ListTrailing::Toggle(true)),
            ListItem::new(&theme, "Ses").with_subtitle(&theme, "Hoparlör").with_trailing(&theme, ListTrailing::Text("%64".into())),
            ListItem::new(&theme, "Ekran").with_trailing(&theme, ListTrailing::Chevron),
        ])
        .with_groups(&theme, &[(0, "Bağlantı")]);
    list.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 600.0), now);
    // Yerleşim çizimde; ağaç çizimden sonra kurulur (runtime sırası).
    let _ = list.place_visible(&theme, now);
    let keys = ["wifi", "audio", "display"];
    list.access(&mut t, "list", |i| keys[i]).label("Sistem");

    let mut results = ResultList::new(&theme);
    results.set_items(&theme, vec![ResultItem::new("Firefox").subtitle("Web tarayıcısı"), ResultItem::new("Dosyalar")], now);
    results.set_rect(&theme, Rect::new(0.0, 0.0, 400.0, 300.0), now);
    results.set_selected(&theme, 1, now);
    let ids = ["firefox.desktop", "files.desktop"];
    let lb = results.access(&mut t, "results", now, |i| ids[i]).id();
    let sel = results.access_selected(lb, |i| ids[i]).unwrap();
    t.push("search", Role::SearchInput).label("Ara").active_descendant(sel).focused();

    let tree = Tree::new(t.finish(), true);
    let wifi = by_label(&tree, "Wi-Fi");
    assert_eq!((wifi.role(), wifi.toggled()), (Role::Switch, Some(Toggled::True)));
    let audio = by_label(&tree, "Ses");
    assert_eq!(audio.role(), Role::Button);
    assert_eq!(audio.description().as_deref(), Some("Hoparlör, %64"));
    assert_eq!(by_label(&tree, "Bağlantı").role(), Role::Heading);
    assert_eq!(by_label(&tree, "Sistem").role(), Role::List);

    // Etkin torun: odak arama alanında, AT seçili satırı okur.
    let focus = tree.state().focus().unwrap();
    assert_eq!((focus.role(), focus.label().as_deref()), (Role::ListBoxOption, Some("Dosyalar")));
    assert_eq!(focus.is_selected(), Some(true));
    assert_eq!(by_label(&tree, "Firefox").description().as_deref(), Some("Web tarayıcısı"));

    let r = results.access_action(&theme, lb, AccessRequest { node: child_id(lb, ids[0]), action: AccessAction::Click }, ids_key(&ids), now);
    assert_eq!(r.event, Some(crate::ResultEvent::Activated(0)));
}

fn ids_key<'a>(ids: &'a [&'a str]) -> impl Fn(usize) -> &'a str {
    move |i| ids[i]
}

#[test]
fn notification_and_empty_state() {
    let theme = Theme::default();
    let now = Instant::now();
    let mut text = TextSystem::new();
    let mut card = NotificationCard::new(
        &theme,
        NotificationContent {
            app_name: "Mesajlar".into(),
            time: "şimdi".into(),
            title: "Ayşe".into(),
            body: vec![BodySpan::plain("Geliyor musun?")],
            actions: vec!["Yanıtla".into()],
            ..NotificationContent::default()
        },
    );
    let h = card.measure(&mut text, &theme, 360.0);
    card.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 360.0, h));

    let mut empty = EmptyState::new(&theme, "Bildirim yok").with_body(&theme, "Her şey yolunda").with_action(&theme, "Ayarlar");
    empty.set_rect(Rect::new(0.0, 200.0, 360.0, 300.0));
    empty.layout(&mut text, &theme);

    let mut t = AccessTree::new();
    let card_id = card.access(&mut t, 42u32, "Kapat").live(Live::Polite).id();
    let empty_id = empty.access(&mut t, "empty").id();
    assert!(t.unlabeled().is_empty());
    let tree = Tree::new(t.finish(), true);

    let c = by_label(&tree, "Ayşe");
    assert_eq!(c.role(), Role::Group);
    assert_eq!(c.description().as_deref(), Some("Mesajlar, şimdi, Geliyor musun?"));
    assert_eq!(c.live(), Live::Polite);
    let kids: Vec<_> = c.children().map(|n| n.label().unwrap_or_default()).collect();
    assert_eq!(kids, vec!["Yanıtla", "Kapat"]);

    let e = by_label(&tree, "Bildirim yok");
    assert_eq!(e.description().as_deref(), Some("Her şey yolunda"));
    assert_eq!(e.children().next().unwrap().label().as_deref(), Some("Ayarlar"));

    let req = |node| AccessRequest { node, action: AccessAction::Click };
    assert_eq!(card.access_action(card_id, req(child_id(card_id, ("action", 0usize))), now), Some(NotificationEvent::Action(0)));
    assert_eq!(card.access_action(card_id, req(card_id), now), Some(NotificationEvent::Activated));
    assert_eq!(card.access_action(card_id, req(child_id(card_id, "close")), now), Some(NotificationEvent::Dismissed));
    assert!(card.is_dismissed());
    assert!(empty.is_access_action(empty_id, req(child_id(empty_id, "action"))));
    assert!(!empty.is_access_action(empty_id, req(root_child("x"))));
}

#[test]
fn tiles_are_switches_with_expand() {
    use crate::{TileEvent, TileGrid, TileItem};
    let theme = Theme::default();
    let now = Instant::now();
    let mut grid = TileGrid::new(&theme, vec![TileItem::new(Icon::named(IconName::Wifi), "Wi-Fi").subtitle("Ev").on(true).expandable(true)]);
    grid.layout(&theme, nuxut_core::Point::new(0.0, 0.0), 400.0);
    let mut t = AccessTree::new();
    grid.access(&mut t, "tiles", |_| "wifi").label("Hızlı anahtarlar");
    assert!(t.unlabeled().is_empty());
    let tree = Tree::new(t.finish(), true);
    let w = by_label(&tree, "Wi-Fi");
    assert_eq!((w.role(), w.toggled(), w.description().as_deref()), (Role::Switch, Some(Toggled::True), Some("Ev")));
    let tile = &mut grid.tiles_mut()[0];
    assert_eq!(tile.access_action(AccessAction::Click, now).event, Some(TileEvent::Toggled(false)));
    assert_eq!(tile.access_action(AccessAction::Expand, now).event, Some(TileEvent::Expand));
}

#[test]
fn select_is_combobox_with_value() {
    let theme = Theme::default();
    let mut s = crate::Select::new(&theme, vec!["Açık".into(), "Koyu".into()], 1);
    s.set_rect(&theme, Rect::new(0.0, 0.0, 200.0, 40.0));
    let mut t = AccessTree::new();
    s.access(&mut t, "mode").label("Tema");
    let tree = Tree::new(t.finish(), true);
    let c = by_label(&tree, "Tema");
    assert_eq!((c.role(), c.value().as_deref()), (Role::ComboBox, Some("Koyu")));
}

#[test]
fn file_grid_and_path_bar() {
    use crate::{FileGrid, FileGridEvent, FileItem, PathBar, PathBarEvent, PathSegment};
    let theme = Theme::default();
    let now = Instant::now();
    let mut text = TextSystem::new();
    let mut t = AccessTree::new();
    t.root().label("Dosya seç");

    let mut bar = PathBar::new(&theme, ["Ev", "Belgeler", "Projeler", "Nuxut", "framework"].iter().map(|s| PathSegment::new(*s)).collect());
    bar.set_rect(Rect::new(0.0, 0.0, 200.0, 40.0));
    bar.layout(&mut text, &theme);
    assert!(bar.first_visible() > 0);
    let nav = bar.access(&mut t, "path", false, "Üst klasörler").label("Konum").id();

    let mut grid = FileGrid::new(&theme, true);
    grid.set_rect(&theme, Rect::new(0.0, 40.0, 460.0, 300.0), now);
    let names = ["a.txt", "b.png", "c"];
    grid.set_items(names.iter().map(|n| FileItem::new(*n, Icon::named(IconName::File))).collect(), now);
    grid.select_only(1, now);
    let lb = grid.access(&mut t, "files", true, now, |i| names[i]).label("Dosyalar").id();

    let tree = Tree::new(t.finish(), true);
    let list = by_label(&tree, "Dosyalar");
    assert_eq!(list.role(), Role::ListBox);
    assert!(list.is_multiselectable());
    let b = by_label(&tree, "b.png");
    assert_eq!((b.role(), b.is_selected(), b.is_focused()), (Role::ListBoxOption, Some(true), true));
    assert_eq!(by_label(&tree, "a.txt").is_selected(), Some(false));
    assert_eq!(by_label(&tree, "Konum").role(), Role::Navigation);
    assert_eq!(by_label(&tree, "Üst klasörler").role(), Role::Button);
    assert_eq!(by_label(&tree, "framework").is_selected(), Some(true), "geçerli klasör");

    let req = |node, action| AccessRequest { node, action };
    let r = grid.access_action(lb, req(child_id(lb, names[2]), AccessAction::Click), |i| names[i], now);
    assert_eq!(r.event, Some(FileGridEvent::SelectionChanged));
    assert_eq!(grid.selection().iter().collect::<Vec<_>>(), [2]);
    let first = bar.first_visible();
    assert_eq!(bar.access_action(nav, req(child_id(nav, "more"), AccessAction::Click)).event, Some(PathBarEvent::Navigate(first - 1)));
    assert_eq!(bar.access_action(nav, req(child_id(nav, 4usize), AccessAction::Click)).event, Some(PathBarEvent::Navigate(4)));
}
