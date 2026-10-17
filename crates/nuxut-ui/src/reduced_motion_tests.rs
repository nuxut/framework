//! Azaltılmış hareket (t124, docs/design/accessibility.md §2): Ayarlar → Hareket "Kapalı"
//! (`[motion] preset = "instant"`) ya da `[motion] enabled = false` iken her bileşen durum
//! değişikliğinin İLK karesinde hedef hâli çizer ve ek kare istemez (boşta sıfır CPU).
//!
//! Kanıt kalıbı ([`assert_instant`]): değişiklikten hemen sonraki kare (a) `wants_frame` yanlış,
//! (b) çizim kaydı on saniye sonraki (kesinlikle oturmuş) kareyle aynı (tür, dikdörtgen, kırpma).
//! Renk/opaklık kayıtta yok; onlar `Animated` üzerinden akar ve `wants_frame` yanlışlığı onların
//! da oturduğunu gösterir (bileşenler yalnızca `is_settled` olmayan bir değer varken kare ister).

use std::time::{Duration, Instant};

use nuxut_core::{Point, Rect, Size};
use nuxut_render::{RecordedDraw, Recorder, TextSystem};
use nuxut_theme::Theme;

use crate::input::test_util::*;
use crate::input::{PointerEvent, PointerKind};
use crate::ui::Ui;
use crate::{
    Avatar, AvatarSize, Badge, Button, ButtonVariant, Chip, ChipKind, Dialog, DialogAction, DialogActionKind, Disclosure, Gauge, Icon,
    IconButton, IconButtonVariant, IconName, Menu, MenuAnchor, MenuItem, MenuModel, MenuSide, NotificationStack, Orientation, Osd,
    NavItem, NavigationRail, PageStack, Popover, Progress, ScrollView, Segment, SegmentedButton, Select, Slider, Snackbar, Spinner, TextInput,
    Toggle, Tooltip,
};

const SURFACE: (u32, u32) = (1000, 800);
const BOUNDS: Rect = Rect::new(0.0, 0.0, 1000.0, 800.0);
/// Hiçbir hareketin bu kadar sürmediği kesin "oturmuş" an.
const LATER: Duration = Duration::from_secs(10);

/// Ayarlar → Hareket "Kapalı".
fn off() -> Theme {
    let (theme, warnings) = Theme::parse("[motion]\npreset = \"instant\"\n").unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert!(theme.reduced_motion());
    theme
}

struct Harness {
    rec: Recorder,
    text: TextSystem,
    theme: Theme,
}

impl Harness {
    fn new() -> Self {
        Self { rec: Recorder::new(), text: TextSystem::new(), theme: off() }
    }

    /// Bir kare çizer: (çizim kaydı, kare istendi mi).
    fn frame(&mut self, now: Instant, f: &mut dyn FnMut(&mut Ui)) -> (Vec<RecordedDraw>, bool) {
        let Self { rec, text, theme } = self;
        let mut wants = false;
        rec.record(text, SURFACE, 1.0, |p| {
            let mut ui = Ui::new(p, theme, now);
            f(&mut ui);
            wants = ui.wants_frame();
        });
        (rec.draws(), wants)
    }

    /// Değişiklik `now`'da yapıldı: o kare hedefte ve ek kare yok.
    fn assert_instant(&mut self, what: &str, now: Instant, f: &mut dyn FnMut(&mut Ui)) {
        let (first, wants) = self.frame(now, f);
        assert!(!wants, "{what}: hareket kapalıyken kare istendi");
        let (settled, wants) = self.frame(now + LATER, f);
        assert!(!wants, "{what}: boşta kare istendi");
        assert_eq!(first, settled, "{what}: ilk kare hedef hâl değil");
    }
}

#[test]
fn theme_off_preset_and_master_switch_are_reduced() {
    for text in ["[motion]\npreset = \"instant\"\n", "[motion]\nenabled = false\n", "[motion]\npreset = \"lively\"\nenabled = false\n"] {
        let (t, _) = Theme::parse(text).unwrap();
        assert!(t.reduced_motion(), "{text}");
        for m in [t.motion.spatial, t.motion.spatial_fast, t.motion.effects] {
            assert!(m.is_instant(), "{text}: {m:?}");
        }
        assert_eq!(t.blob.deform, 0.0, "blob esnemesi kapalı");
        assert!(t.blob.deform_spring.is_instant());
    }
    for p in ["calm", "lively", "snappy"] {
        let (t, _) = Theme::parse(&format!("[motion]\npreset = \"{p}\"\n")).unwrap();
        assert!(!t.reduced_motion(), "{p}");
    }
    // Rol ezmesi ve stil "Kapalı"yı delmez: ana anahtar her şeyi keser.
    let (t, _) = Theme::parse("[motion]\npreset = \"instant\"\nstyle = \"curve\"\n[motion.spatial]\nresponse = 2.0\nduration_ms = 900\n").unwrap();
    assert!(t.reduced_motion() && t.motion.spatial.is_instant());
}

#[test]
fn buttons_toggles_and_selection() {
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();

    let mut b = Button::new(&theme, ButtonVariant::Filled).with_label(&theme, "Tamam");
    b.set_rect(Rect::new(10.0, 10.0, 120.0, 40.0));
    let _ = b.pointer(&motion(20.0, 20.0), t0);
    let _ = b.pointer(&press(20.0, 20.0), t0);
    h.assert_instant("Button basış (state layer + sıkışma)", t0, &mut |ui| b.draw(ui));

    let mut ib = IconButton::new(&theme, Icon::named(IconName::Settings), IconButtonVariant::Standard);
    ib.set_rect(Rect::new(200.0, 10.0, 40.0, 40.0));
    ib.set_selected(true, t0);
    h.assert_instant("IconButton seçimi", t0, &mut |ui| ib.draw(ui));

    let mut tg = Toggle::new(&theme, false);
    tg.set_rect(Rect::new(300.0, 10.0, 52.0, 32.0));
    assert!(tg.set_on(true, t0));
    h.assert_instant("Toggle topuz kayması", t0, &mut |ui| tg.draw(ui));

    let mut s = Slider::new(&theme, Orientation::Horizontal, 0.1);
    s.set_rect(Rect::new(10.0, 100.0, 300.0, 40.0));
    s.set_value(0.9, t0);
    h.assert_instant("Slider değeri", t0, &mut |ui| s.draw(ui));

    let mut seg = SegmentedButton::new(&theme, vec![Segment::new("Açık"), Segment::new("Koyu"), Segment::new("Sistem")], 0);
    let size = seg.preferred_size(&mut h.text, &theme);
    seg.set_rect(&theme, Rect::new(10.0, 200.0, size.width, size.height));
    seg.set_selected(2, t0);
    h.assert_instant("SegmentedButton gösterge kayması", t0, &mut |ui| seg.draw(ui));

    let mut chip = Chip::new(&theme, ChipKind::Filter, "Wi-Fi");
    let size = chip.preferred_size(&mut h.text, &theme);
    chip.set_rect(Rect::new(10.0, 300.0, size.width, size.height));
    chip.set_selected(true, t0);
    h.assert_instant("Chip seçimi", t0, &mut |ui| chip.draw(ui));

    let mut badge = Badge::new(&theme, None);
    badge.set_count(Some(3), t0);
    h.assert_instant("Badge belirmesi", t0, &mut |ui| badge.draw(ui));

    let mut av = Avatar::new(&theme, "Ada Lovelace", AvatarSize::Medium);
    av.set_presence(true, t0);
    h.assert_instant("Avatar çevrimiçi noktası", t0, &mut |ui| av.draw(ui));

    let mut rail = NavigationRail::new(
        &theme,
        vec![NavItem::new(Icon::named(IconName::Settings), "Ayarlar"), NavItem::new(Icon::named(IconName::VolumeHigh), "Ses")],
        0,
    );
    rail.set_rect(&theme, Rect::new(0.0, 400.0, 80.0, 300.0));
    let _ = rail.key(&theme, &key(crate::input::Key::Down), t0);
    h.assert_instant("NavigationRail gösterge kayması", t0, &mut |ui| rail.draw(ui));

    let mut input = TextInput::new(&theme);
    input.set_rect(Rect::new(400.0, 400.0, 300.0, 48.0));
    input.set_focused(true, t0);
    let _ = h.frame(t0, &mut |ui| input.draw(ui));
    for c in "merhaba".chars() {
        let _ = input.key(&typed(c), t0);
    }
    // İmleç yanıp sönmez (Nuxut'ta yanıp sönme yok); kayan imleç konumu anında.
    h.assert_instant("TextInput imleç kayması", t0, &mut |ui| input.draw(ui));
}

#[test]
fn select_menu_opens_in_one_frame() {
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();
    let mut s = Select::new(&theme, vec!["Açık".into(), "Koyu".into(), "Sistem".into()], 0);
    s.set_rect(&theme, Rect::new(10.0, 40.0, 240.0, 48.0));
    let _ = s.key(&mut h.text, &theme, &key(crate::input::Key::Down), t0);
    assert!(s.is_open());
    h.assert_instant("Select açılış", t0, &mut |ui| s.draw_overlay(ui));
}

#[test]
fn progress_gauge_and_spinner() {
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();

    let mut p = Progress::new(&theme, 0.1);
    p.set_rect(Rect::new(10.0, 10.0, 300.0, 8.0));
    p.set_value(0.8, t0);
    h.assert_instant("Progress değeri", t0, &mut |ui| p.draw(ui));

    let mut g = Gauge::new(&theme, 0.1);
    g.set_rect(Rect::new(10.0, 40.0, 80.0, 80.0));
    g.set_value(0.95, t0);
    h.assert_instant("Gauge değeri", t0, &mut |ui| g.draw(ui));

    // Belirsiz spinner: sonsuz döngü yerine durağan gösterge; hiç kare istemez, zamanla değişmez.
    let mut sp = Spinner::new(&theme);
    sp.set_rect(Rect::new(200.0, 40.0, 40.0, 40.0));
    let (first, wants) = h.frame(t0, &mut |ui| sp.draw(ui));
    assert!(!wants, "belirsiz spinner hareket kapalıyken kare istemez");
    assert!(!first.is_empty(), "durağan gösterge çizilir (boş değil)");
    for dt in [16, 333, 1_000, 7_777] {
        let (later, wants) = h.frame(t0 + Duration::from_millis(dt), &mut |ui| sp.draw(ui));
        assert!(!wants);
        assert_eq!(later, first, "{dt} ms: durağan");
    }
    sp.set_progress(&theme, Some(0.6), t0);
    h.assert_instant("Spinner belirli kip", t0, &mut |ui| sp.draw(ui));
}

#[test]
fn spinner_spins_when_motion_is_on() {
    // Karşı kanıt: hareket açıkken belirsiz spinner döner ve kare ister.
    let theme = Theme::default();
    assert!(!theme.reduced_motion());
    let mut h = Harness { rec: Recorder::new(), text: TextSystem::new(), theme: theme.clone() };
    let mut sp = Spinner::new(&theme);
    sp.set_rect(Rect::new(200.0, 40.0, 40.0, 40.0));
    let t0 = Instant::now();
    let (spinning, wants) = h.frame(t0, &mut |ui| sp.draw(ui));
    assert!(wants, "döner: sonraki kare istenir");
    assert_eq!(spinning.len(), 1, "tek dönen yay");
    let (_, wants) = h.frame(t0 + LATER, &mut |ui| sp.draw(ui));
    assert!(wants, "sonsuz döngü");
    // Durağan alternatif: iz + üç parça (tek yay "%25 ilerleme" gibi okunurdu).
    let mut off = Harness::new();
    let mut still = Spinner::new(&off.theme);
    still.set_rect(Rect::new(200.0, 40.0, 40.0, 40.0));
    let (still_draws, _) = off.frame(t0, &mut |ui| still.draw(ui));
    assert_eq!(still_draws.len(), 4);
    // Canlı tema değişimi: hareket kapanınca aynı spinner durağanlaşır.
    sp.set_theme(&off.theme, t0);
    let (_, wants) = off.frame(t0, &mut |ui| sp.draw(ui));
    assert!(!wants, "set_theme ile Kapalı'ya geçiş");
}

fn menu_model() -> MenuModel {
    MenuModel::new(vec![
        MenuItem::action(1, "Aç"),
        MenuItem::submenu(2, "Daha fazla", vec![MenuItem::action(21, "Alt öğe")]),
        MenuItem::check(3, "Bildirimler", true),
    ])
}

#[test]
fn overlays_open_and_close_in_one_frame() {
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();
    let anchor = MenuAnchor::below(Rect::new(400.0, 0.0, 32.0, 40.0));

    // Menü: blob morf (tohumdan büyüme + hız esnemesi) yok; açılış ve kapanış tek kare.
    let mut m = Menu::new(&theme);
    m.open(&mut h.text, &theme, menu_model(), anchor, BOUNDS, t0);
    h.assert_instant("Menu açılış", t0, &mut |ui| m.draw(ui));
    // Alt menü sayfa geçişi (kayma + çapraz solma).
    let t1 = t0 + LATER * 2;
    let _ = m.key(&mut h.text, &theme, &key(crate::input::Key::Down), t1);
    let _ = m.key(&mut h.text, &theme, &key(crate::input::Key::Right), t1);
    h.assert_instant("Menu alt sayfa geçişi", t1, &mut |ui| m.draw(ui));
    let t2 = t1 + LATER * 2;
    m.close(t2);
    h.assert_instant("Menu kapanış", t2, &mut |ui| m.draw(ui));
    assert!(!m.is_visible(), "kapanış ilk karede bitti");

    let mut p = Popover::new(&theme);
    p.open(&theme, anchor, Size::new(200.0, 120.0), BOUNDS, t0);
    h.assert_instant("Popover açılış", t0, &mut |ui| {
        p.draw_background(ui);
        p.draw_content(ui, |_, _| {});
    });
    p.set_content_size(&theme, Size::new(260.0, 300.0), t1);
    h.assert_instant("Popover içerik boyutu morfu", t1, &mut |ui| {
        p.draw_background(ui);
        p.draw_content(ui, |_, _| {});
    });

    let mut tip = Tooltip::new(&theme);
    tip.show("Wi-Fi ayarları", Rect::new(100.0, 100.0, 40.0, 40.0), MenuSide::Below, BOUNDS, t0);
    h.assert_instant("Tooltip belirme", t0, &mut |ui| tip.draw(ui));
    tip.hide(t1);
    h.assert_instant("Tooltip sönme", t1, &mut |ui| tip.draw(ui));

    let mut d = Dialog::new(&theme);
    d.open(
        &mut h.text,
        &theme,
        "Silinsin mi?",
        vec![DialogAction::new("İptal", DialogActionKind::Default), DialogAction::new("Sil", DialogActionKind::Destructive)],
        Size::new(300.0, 80.0),
        BOUNDS,
        t0,
    );
    h.assert_instant("Dialog açılış (scrim + büyüme)", t0, &mut |ui| d.draw(ui, |_, _| {}));
    d.close(t1);
    h.assert_instant("Dialog kapanış", t1, &mut |ui| d.draw(ui, |_, _| {}));

    let mut sb = Snackbar::new(&theme);
    sb.show(&mut h.text, &theme, "Kaydedildi", Some("Geri al"), BOUNDS, t0);
    h.assert_instant("Snackbar yükselme", t0, &mut |ui| sb.draw(ui));
    sb.hide(t1);
    h.assert_instant("Snackbar inme", t1, &mut |ui| sb.draw(ui));

    let mut osd = Osd::new(&theme);
    let osd_anchor = MenuAnchor { side: MenuSide::Above, ..MenuAnchor::below(Rect::new(500.0, 790.0, 0.0, 0.0)) };
    osd.show(&theme, Icon::named(IconName::VolumeHigh), 0.3, osd_anchor, BOUNDS, t0);
    h.assert_instant("OSD açılış", t0, &mut |ui| osd.draw(ui));
    osd.show(&theme, Icon::named(IconName::VolumeHigh), 0.9, osd_anchor, BOUNDS, t1);
    h.assert_instant("OSD değer akışı", t1, &mut |ui| osd.draw(ui));
    osd.hide(t2);
    h.assert_instant("OSD kapanış", t2, &mut |ui| osd.draw(ui));
}

#[test]
fn page_and_layout_transitions() {
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();

    let mut stack = PageStack::new(&theme, "kök", Size::new(300.0, 200.0));
    stack.push("wifi", Size::new(300.0, 420.0), t0);
    assert_eq!(stack.current_size(t0).height, 420.0, "sayfa geçişi anında");
    h.assert_instant("PageStack push", t0, &mut |ui| stack.draw(ui, BOUNDS, |_, _, _| {}));
    let t1 = t0 + LATER * 2;
    let _ = stack.pop(Size::new(300.0, 200.0), t1);
    h.assert_instant("PageStack pop", t1, &mut |ui| stack.draw(ui, BOUNDS, |_, _, _| {}));

    let mut d = Disclosure::new(&theme, "Gelişmiş");
    d.set_rect(Rect::new(0.0, 0.0, 400.0, 56.0));
    d.set_open(true, true, t0);
    assert_eq!(d.height(t0), d.height(t0 + LATER), "açılma anında");
    h.assert_instant("Disclosure açılma", t0, &mut |ui| d.draw(ui));

    let mut ns: NotificationStack<u32> = NotificationStack::new(&theme);
    let _ = ns.layout(&[(1, 80.0)], 8.0, t0);
    let _ = ns.layout(&[(2, 60.0), (1, 80.0)], 8.0, t1);
    assert!(!ns.is_animating(t1), "bildirim yığını yeniden dizilişi anında");
}

fn wheel(steps: f32) -> PointerEvent {
    PointerEvent::new(Point::new(10.0, 10.0), PointerKind::Scroll { delta: Point::new(0.0, 0.0), steps: Point::new(0.0, steps) })
}

fn touch(dy: f32) -> PointerEvent {
    PointerEvent::new(Point::new(10.0, 10.0), PointerKind::Scroll { delta: Point::new(0.0, dy), steps: Point::ZERO })
}

#[test]
fn scrolling_has_no_animation_or_inertia() {
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();
    let mut s = ScrollView::new(&theme);
    s.set_geometry(Rect::new(0.0, 0.0, 300.0, 400.0), 4000.0, t0);

    // Tekerlek: hedefe tek karede.
    let _ = s.pointer(&theme, &wheel(2.0), t0);
    assert_eq!(s.offset(t0), theme.space(24));
    h.assert_instant("ScrollView tekerlek", t0, &mut |ui| s.draw(ui, |_, _, _| {}));

    // Dokunmatik yüzey: içerik parmağı izler (doğrudan işleme, hareket değil); bırakınca
    // fırlatma (atalet) YOK — olduğu yerde durur.
    let t1 = t0 + LATER;
    let mut t = t1;
    for _ in 0..10 {
        t += Duration::from_millis(8);
        let _ = s.pointer(&theme, &touch(40.0), t);
    }
    let released_at = s.offset(t);
    s.scroll_end(t);
    assert_eq!(s.offset(t), released_at, "bırakınca sıçrama yok");
    h.assert_instant("ScrollView fırlatma yok", t, &mut |ui| s.draw(ui, |_, _, _| {}));
    assert_eq!(s.offset(t + LATER), released_at, "atalet yok: konum değişmez");

    // Kenar taşması: bırakınca geri sekme anında.
    let t2 = t + LATER;
    s.scroll_to(0.0, t2);
    let mut t = t2;
    for _ in 0..5 {
        t += Duration::from_millis(8);
        let _ = s.pointer(&theme, &touch(-60.0), t);
    }
    assert!(s.offset(t) < 0.0, "lastik taşma (parmağı izler)");
    s.scroll_end(t);
    assert_eq!(s.offset(t), 0.0, "geri sekme anında");
    h.assert_instant("ScrollView geri sekme", t, &mut |ui| s.draw(ui, |_, _, _| {}));

    // Programatik kaydırma (ensure_visible / scroll_to).
    let t3 = t + LATER;
    s.ensure_visible(2000.0, 2050.0, t3);
    assert_eq!(s.offset(t3), 1650.0);
    h.assert_instant("ScrollView ensure_visible", t3, &mut |ui| s.draw(ui, |_, _, _| {}));
}

#[test]
fn fling_still_glides_when_motion_is_on() {
    // Karşı kanıt: hareket açıkken bırakılan dokunmatik hareket kayarak yavaşlar.
    let theme = Theme::default();
    let t0 = Instant::now();
    let mut s = ScrollView::new(&theme);
    s.set_geometry(Rect::new(0.0, 0.0, 300.0, 400.0), 4000.0, t0);
    let mut t = t0;
    for _ in 0..10 {
        t += Duration::from_millis(8);
        let _ = s.pointer(&theme, &touch(40.0), t);
    }
    let released_at = s.offset(t);
    s.scroll_end(t);
    assert!(s.offset(t + Duration::from_millis(200)) > released_at + 1.0, "fırlatma sürer");
}

#[test]
fn glyph_grid_selection_and_reveal_are_instant() {
    use crate::{GlyphGrid, GlyphItem, GlyphSection};
    let mut h = Harness::new();
    let theme = h.theme.clone();
    let t0 = Instant::now();
    let mut g = GlyphGrid::new(&theme);
    g.set_rect(&theme, Rect::new(10.0, 10.0, 440.0, 220.0), t0);
    let items = |n: usize| (0..n).map(|i| GlyphItem::new(format!("{}", i % 10), format!("rakam {i}"))).collect::<Vec<_>>();
    g.set_sections(&theme, vec![GlyphSection::new("A", items(300)), GlyphSection::new("B", items(300))], t0);
    g.set_focus_visible(true);
    // Klavye gezinmesi: vurgu kayması + seçimi görünür kılan kaydırma.
    let _ = g.key(&key(crate::input::Key::Down), t0);
    h.assert_instant("GlyphGrid seçim vurgusu", t0, &mut |ui| g.draw(ui));
    let t1 = t0 + LATER;
    for _ in 0..12 {
        let _ = g.key(&key(crate::input::Key::Down), t1);
    }
    h.assert_instant("GlyphGrid seçimi görünür kılma", t1, &mut |ui| g.draw(ui));
    let t2 = t1 + LATER;
    assert!(g.reveal_section(1, t2));
    h.assert_instant("GlyphGrid bölüme atlama", t2, &mut |ui| g.draw(ui));
}
