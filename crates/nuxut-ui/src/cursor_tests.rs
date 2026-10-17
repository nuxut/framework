//! Bileşenlerin `cursor(p)` sözleşmesi (DESIGN.md "İmleç"): temsilî noktalar.

use std::time::Instant;

use nuxut_core::{Point, Rect, Size};
use nuxut_motion::{CubicBezier, Spring};
use nuxut_render::TextSystem;
use nuxut_theme::Theme;

use crate::input::CursorShape::{self, *};
use crate::input::test_util::*;
use crate::*;

fn env() -> (Theme, TextSystem, Instant) {
    (Theme::default(), TextSystem::new(), Instant::now())
}

#[test]
fn pick_takes_topmost_hit() {
    assert_eq!(CursorShape::pick([None, Some(Text), Some(Pointer)]), Text);
}

#[test]
fn buttons_toggle_segmented() {
    let (theme, _, t0) = env();
    let mut b = Button::new(&theme, ButtonVariant::Filled).with_label(&theme, "Tamam");
    b.set_rect(Rect::new(0.0, 0.0, 100.0, 40.0));
    assert_eq!(b.cursor(Point::new(10.0, 10.0)), Some(Pointer));
    assert_eq!(b.cursor(Point::new(200.0, 10.0)), None);
    b.set_enabled(false, t0);
    assert_eq!(b.cursor(Point::new(10.0, 10.0)), Some(Default));

    let mut t = Toggle::new(&theme, false);
    t.set_rect(Rect::new(0.0, 0.0, 52.0, 32.0));
    assert_eq!(t.cursor(Point::new(5.0, 5.0)), Some(Pointer));

    let mut s = SegmentedButton::new(&theme, vec![Segment::new("A"), Segment::new("B")], 0);
    s.set_rect(&theme, Rect::new(0.0, 0.0, 200.0, 40.0));
    assert_eq!(s.cursor(Point::new(150.0, 20.0)), Some(Pointer));
}

#[test]
fn slider_drag_is_grabbing() {
    let (theme, _, t0) = env();
    let mut s = Slider::new(&theme, Orientation::Horizontal, 0.5);
    s.set_rect(Rect::new(0.0, 0.0, 200.0, 44.0));
    assert_eq!(s.cursor(Point::new(20.0, 22.0)), Some(Pointer));
    let _ = s.pointer(&press(20.0, 22.0), t0);
    assert_eq!(s.cursor(Point::new(500.0, 500.0)), Some(Grabbing), "sürükleme dışarıda da sürer");
}

#[test]
fn text_input_is_text_even_for_password() {
    let (theme, _, _) = env();
    let mut i = TextInput::new(&theme).password(true);
    i.set_rect(Rect::new(0.0, 0.0, 200.0, 48.0));
    assert_eq!(i.cursor(Point::new(10.0, 10.0)), Some(Text));
    i.set_enabled(false);
    assert_eq!(i.cursor(Point::new(10.0, 10.0)), Some(Default));
}

#[test]
fn menu_items_pointer_separator_default() {
    let (theme, mut text, t0) = env();
    let mut m = Menu::new(&theme);
    let model = MenuModel::new(vec![MenuItem::action(1, "Aç"), MenuItem::separator(), MenuItem::action(2, "Çık")]);
    m.open(&mut text, &theme, model, MenuAnchor::below(Rect::new(100.0, 0.0, 40.0, 40.0)), Rect::new(0.0, 40.0, 800.0, 600.0), t0);
    let f = m.input_rect().unwrap();
    let pad = theme.space(2);
    assert_eq!(m.cursor(Point::new(f.x + 20.0, f.y + pad + theme.space(5))), Some(Pointer));
    assert_eq!(m.cursor(Point::new(f.x + 20.0, f.y + pad + theme.space(10) + theme.space(1))), Some(Default));
    assert_eq!(m.cursor(Point::new(f.x - 50.0, f.y)), None);
}

#[test]
fn select_field_and_open_list() {
    let (theme, mut text, t0) = env();
    let mut s = Select::new(&theme, vec!["A".into(), "B".into()], 0);
    s.set_rect(&theme, Rect::new(0.0, 0.0, 200.0, 48.0));
    s.set_bounds(Rect::new(0.0, 0.0, 800.0, 600.0));
    assert_eq!(s.cursor(Point::new(10.0, 10.0)), Some(Pointer));
    let _ = s.key(&mut text, &theme, &key(crate::input::Key::Down), t0);
    let list = s.input_rect().unwrap();
    assert_eq!(s.cursor(Point::new(list.x + 20.0, list.y + theme.space(2) + theme.space(5))), Some(Pointer));
}

#[test]
fn popover_inside_is_default() {
    let (theme, _, t0) = env();
    let mut p = Popover::new(&theme);
    p.open(&theme, MenuAnchor::below(Rect::new(10.0, 10.0, 40.0, 40.0)), Size::new(100.0, 100.0), Rect::new(0.0, 0.0, 800.0, 600.0), t0);
    let r = p.input_rect().unwrap();
    assert_eq!(p.cursor(r.center()), Some(Default));
    assert_eq!(p.cursor(Point::new(700.0, 500.0)), None);
}

#[test]
fn notification_body_pointer_swipe_grabbing() {
    let (theme, mut text, t0) = env();
    let mut c = NotificationCard::new(
        &theme,
        NotificationContent { app_name: "Uygulama".into(), title: "Başlık".into(), body: vec![BodySpan::plain("Gövde")], ..NotificationContent::default() },
    );
    let h = c.measure(&mut text, &theme, 320.0);
    c.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 320.0, h));
    let p = Point::new(60.0, h * 0.6);
    assert_eq!(c.cursor(p), Some(Pointer));
    let _ = c.pointer(&theme, &press(p.x, p.y), t0);
    let _ = c.pointer(&theme, &motion(p.x + 60.0, p.y), t0);
    assert_eq!(c.cursor(Point::new(p.x + 60.0, p.y)), Some(Grabbing));
}

#[test]
fn color_picker_regions() {
    let (theme, _, _) = env();
    let mut cp = ColorPicker::new(&theme, theme.colors.primary);
    let s = cp.preferred_size(&theme, 280.0);
    cp.set_rect(&theme, Rect::new(0.0, 0.0, s.width, s.height));
    assert_eq!(cp.cursor(Point::new(20.0, 20.0)), Some(Crosshair));
    let field_h = (280.0f32 * 0.62).round();
    let hue_y = field_h + theme.space(3) + theme.space(2);
    assert_eq!(cp.cursor(Point::new(100.0, hue_y)), Some(Pointer));
    let hex_y = field_h + theme.space(3) + theme.space(4) + theme.space(4) + theme.space(6);
    assert_eq!(cp.cursor(Point::new(theme.space(12) + theme.space(10), hex_y)), Some(Text));
}

#[test]
fn calendar_days_pointer_header_default() {
    let (theme, _, _) = env();
    let mut g = CalendarGrid::new(&theme, Date::new(2026, 10, 4), "tr_TR");
    let s = g.preferred_size(&theme, 280.0);
    g.set_rect(&theme, Rect::new(0.0, 0.0, s.width, s.height));
    let grid_top = theme.space(12) + theme.space(7);
    assert_eq!(g.cursor(Point::new(20.0, grid_top + 10.0)), Some(Pointer));
    assert_eq!(g.cursor(Point::new(20.0, 10.0)), Some(Default));
}

#[test]
fn curve_editor_drag_grabbing() {
    let (theme, _, _) = env();
    let mut e = CurveEditor::new(CurveValue::Spring(Spring::new(0.4, 0.7)));
    e.set_rect(Rect::new(0.0, 0.0, 300.0, 210.0));
    let _ = e.pointer(&theme, &press(150.0, 100.0));
    assert_eq!(e.cursor(Point::new(150.0, 100.0)), Some(Grabbing));
    let _ = e.pointer(&theme, &release(150.0, 100.0));
    let mut b = CurveEditor::new(CurveValue::Bezier(CubicBezier::EASE));
    b.set_rect(Rect::new(0.0, 0.0, 300.0, 210.0));
    assert_eq!(b.cursor(Point::new(150.0, 100.0)), None, "çizim öncesi bölge bilinmez");
}

#[test]
fn reorder_handle_grab_remove_pointer() {
    let (theme, _, t0) = env();
    let mut l = ReorderList::new(&theme, vec![ReorderItem::new("Saat"), ReorderItem::new("Pil")]);
    l.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 200.0), t0);
    let _ = l.pointer(&theme, &motion(150.0, 10.0), t0);
    assert_eq!(l.cursor(Point::new(10.0, 10.0)), Some(Grab));
    assert_eq!(l.cursor(Point::new(150.0, 10.0)), Some(Pointer));
    let x_center = 300.0 - theme.space(2) - theme.space(4);
    assert_eq!(l.cursor(Point::new(x_center, theme.space(6))), Some(Pointer));
    let _ = l.pointer(&theme, &press(10.0, 10.0), t0);
    assert_eq!(l.cursor(Point::new(10.0, 10.0)), Some(Grabbing));
}

#[test]
fn list_rows_pointer() {
    let (theme, _, t0) = env();
    let mut l = List::new(&theme).with_items(vec![ListItem::new(&theme, "Wi-Fi"), ListItem::new(&theme, "Ses")]);
    l.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 300.0), t0);
    let _ = l.pointer(&theme, &motion(10.0, 10.0), t0);
    assert_eq!(l.cursor(Point::new(10.0, 10.0)), Some(Pointer));
    assert_eq!(l.cursor(Point::new(10.0, 290.0)), Some(Default));
    assert_eq!(l.cursor(Point::new(400.0, 10.0)), None);
}

#[test]
fn shell_components() {
    let (theme, mut text, t0) = env();
    let mut g = TileGrid::new(&theme, vec![TileItem::new(Icon::named(crate::IconName::Wifi), "Wi-Fi").expandable(true)]);
    g.layout(&theme, Point::ZERO, 400.0);
    let r = g.tiles()[0].rect();
    assert_eq!(g.cursor(Point::new(r.x + 10.0, r.y + 10.0)), Some(Pointer));
    assert_eq!(g.cursor(Point::new(r.right() - 5.0, r.y + 10.0)), Some(Pointer), "› alanı");
    assert_eq!(g.cursor(Point::new(r.right() + 50.0, r.y + 10.0)), None);

    let mut l = ResultList::new(&theme);
    l.set_rect(&theme, Rect::new(0.0, 0.0, 300.0, 400.0), t0);
    l.set_items(&theme, vec![ResultItem::new("Nova"), ResultItem::new("Lira")], t0);
    assert_eq!(l.cursor(Point::new(10.0, 10.0)), Some(Pointer));
    assert_eq!(l.cursor(Point::new(10.0, 390.0)), Some(Default), "boş alan");
    assert_eq!(l.cursor(Point::new(400.0, 10.0)), None);

    let mut d = DeviceRow::new(&theme, "ag", DeviceKind::Wifi { strength: 40, secured: true }, DeviceState::Unavailable);
    d.set_rect(&mut text, &theme, Rect::new(0.0, 0.0, 300.0, 56.0));
    assert_eq!(d.cursor(Point::new(10.0, 10.0)), Some(Default), "kullanılamaz");
    d.set_state(DeviceState::Available, t0);
    assert_eq!(d.cursor(Point::new(10.0, 10.0)), Some(Pointer));

    let mut h = PageHeader::new(&theme, "Wi-Fi");
    h.set_rect(Rect::new(0.0, 0.0, 120.0, 48.0));
    assert_eq!(h.cursor(Point::new(10.0, 10.0)), Some(Pointer));
}
