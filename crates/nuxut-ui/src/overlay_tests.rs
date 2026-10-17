//! Kaplama katmanı: kırpılı kapsayıcının içindeki tetikleyiciden açılan ipucu/menü kırpılmaz,
//! en üstte çizilir; girdi bölgesi bildirilir; kapanınca katman boşalır.

use std::time::{Duration, Instant};

use nuxut_core::{Point, Rect};
use nuxut_render::{DrawKind, PxRect, RecordedDraw, Recorder, TextSystem};
use nuxut_theme::Theme;

use crate::input::test_util::*;
use crate::menu::MenuSide;
use crate::select::{Select, SelectEvent};
use crate::tooltip::Tooltip;
use crate::ui::{OverlayRegions, Ui};

const SURFACE: (u32, u32) = (400, 400);
/// Kırpılı kapsayıcı (ör. Ayarlar'da bir kartın kaydırma alanı).
const VIEWPORT: Rect = Rect::new(0.0, 0.0, 400.0, 100.0);

fn viewport_clip() -> Option<PxRect> {
    Some(PxRect::new(0, 0, 400, 100))
}

/// Kapsayıcının kırpması içinde `f` ile çizer; kare sonundaki kaplama bölgelerini döndürür.
fn frame(
    rec: &mut Recorder,
    text: &mut TextSystem,
    theme: &Theme,
    now: Instant,
    f: impl FnOnce(&mut Ui),
) -> (OverlayRegions, bool) {
    let mut out = (OverlayRegions::default(), false);
    rec.record(text, SURFACE, 1.0, |p| {
        let mut ui = Ui::new(p, theme, now);
        ui.painter.push_clip(VIEWPORT);
        f(&mut ui);
        ui.painter.pop_clip();
        assert!(!ui.painter.in_overlay(), "kaplama kapatıldı");
        out = (ui.take_overlay_regions(), ui.wants_frame());
    });
    out
}

/// Temel katman (kapsayıcının kırpmasıyla) önce, kaplama en sonda tek blok: kaplamadaki
/// çizimler ya kırpmasız ya da yalnızca kendi şekline kırpılı — kapsayıcıya değil (kapsayıcının
/// dışına taşan bir kırpma).
fn overlay_tail(draws: &[RecordedDraw]) -> &[RecordedDraw] {
    let first = draws.iter().position(|d| d.clip != viewport_clip()).unwrap_or(draws.len());
    let tail = &draws[first..];
    let escapes = |c: Option<PxRect>| c.is_none_or(|c| c.bottom() > VIEWPORT.bottom() as i32);
    assert!(tail.iter().all(|d| escapes(d.clip)), "kaplama temel katmanla karışmaz: {draws:?}");
    assert!(tail.first().is_none_or(|d| d.clip.is_none()), "kaplama kırpmasız başlar");
    tail
}

#[test]
fn select_menu_in_clipped_container_is_unclipped_and_takes_clicks() {
    let theme = Theme::default();
    let mut text = TextSystem::new();
    let mut rec = Recorder::new();
    let t0 = Instant::now();
    let mut s = Select::new(&theme, vec!["Açık".into(), "Koyu".into(), "Sistem".into()], 0);
    // Alan kapsayıcının alt kenarına yakın: menü kapsayıcının dışına taşar.
    s.set_rect(&theme, Rect::new(10.0, 40.0, 240.0, 48.0));
    let _ = s.key(&mut text, &theme, &key(crate::input::Key::Down), t0);
    assert!(s.is_open());
    let menu = s.input_rect().unwrap();
    assert!(menu.bottom() > VIEWPORT.bottom(), "menü kapsayıcıdan taşıyor: {menu:?}");

    // Açılış yayı sürerken kare istenir; oturunca istenmez.
    let (_, wants) = frame(&mut rec, &mut text, &theme, t0, |ui| {
        s.draw(ui);
        s.draw_overlay(ui);
    });
    assert!(wants, "açılış yay ile canlanır");
    let t1 = t0 + Duration::from_secs(5);
    let (regions, wants) = frame(&mut rec, &mut text, &theme, t1, |ui| {
        s.draw(ui);
        s.draw_overlay(ui);
    });
    assert!(!wants, "oturunca kare istenmez");
    let quads = rec.draws();
    let tail = overlay_tail(&quads);
    assert!(!tail.is_empty(), "menü kaplamada");
    let below = Point::new(menu.center().x, menu.bottom() - 4.0);
    assert!(below.y > VIEWPORT.bottom());
    assert!(
        tail.iter().any(|d| d.kind == DrawKind::Blob && d.rect.contains(below)),
        "kapsayıcının dışındaki kısım da çizilir"
    );

    // Girdi: kapsayıcının dışındaki menü noktası kaplama bölgesinde; uygulama olayı menüye verir.
    assert_eq!(regions.rects(), &[menu]);
    let last_item = Point::new(menu.center().x, menu.bottom() - theme.space(6));
    assert!(regions.contains(last_item) && !VIEWPORT.contains(last_item));
    let _ = s.pointer(&mut text, &theme, &motion(last_item.x, last_item.y), t1);
    let _ = s.pointer(&mut text, &theme, &press(last_item.x, last_item.y), t1);
    let r = s.pointer(&mut text, &theme, &release(last_item.x, last_item.y), t1);
    assert_eq!(r.event, Some(SelectEvent::Changed(2)), "tıklama menüye gitti");
    assert!(!s.is_open(), "seçim menüyü kapatır");

    // Kapanış yayı bitince (oturduğu karede şekil sıfır boyda son kez çizilip kapanır)
    // kaplama katmanı boş, bölge yok.
    let t2 = t1 + Duration::from_secs(5);
    frame(&mut rec, &mut text, &theme, t2, |ui| {
        s.draw(ui);
        s.draw_overlay(ui);
    });
    let (regions, wants) = frame(&mut rec, &mut text, &theme, t2, |ui| {
        s.draw(ui);
        s.draw_overlay(ui);
    });
    assert!(regions.is_empty());
    assert!(!wants);
    let d = rec.draws();
    assert!(d.iter().all(|q| q.clip == viewport_clip()), "kapalıyken kaplamada çizim yok: {d:?}");
}

#[test]
fn tooltip_in_clipped_container_is_unclipped_and_idle_while_waiting() {
    let theme = Theme::default();
    let mut text = TextSystem::new();
    let mut rec = Recorder::new();
    let t0 = Instant::now();
    let mut tip = Tooltip::new(&theme);
    // Kapsayıcının alt kenarındaki satır; `bounds` boş → yüzeyin tamamına sığar.
    let anchor = Rect::new(20.0, 70.0, 200.0, 30.0);
    let at = tip.hover("Satırın açıklaması", anchor, MenuSide::Below, Rect::ZERO, t0);

    if let Some(at) = at {
        // Beklerken: çizim yok, kare yok, yalnızca uyanma anı.
        let mut deadline = None;
        rec.record(&mut text, SURFACE, 1.0, |p| {
            let mut ui = Ui::new(p, &theme, t0);
            ui.painter.push_clip(VIEWPORT);
            tip.draw(&mut ui);
            ui.painter.pop_clip();
            assert!(!ui.wants_frame());
            deadline = ui.wake_deadline();
        });
        assert_eq!(deadline, Some(at));
        assert!(rec.is_empty());
    }
    tip.show("Satırın açıklaması", anchor, MenuSide::Below, Rect::ZERO, t0);
    let t1 = t0 + Duration::from_secs(5);
    let (regions, wants) = frame(&mut rec, &mut text, &theme, t1, |ui| tip.draw(ui));
    assert!(!wants && regions.is_empty(), "ipucu girdi almaz, oturunca kare yok");
    let quads = rec.draws();
    let tail = overlay_tail(&quads);
    let bubble = tail.iter().rev().find(|d| d.kind == DrawKind::Quad).expect("ipucu kaplamada").rect;
    assert!(bubble.y >= anchor.bottom(), "tetikleyicinin altında, kapsayıcının dışında: {bubble:?}");
    assert!(bubble.bottom() <= SURFACE.1 as f32, "yüzeye sığar");

    tip.hide(t1);
    let t2 = t1 + Duration::from_secs(5);
    frame(&mut rec, &mut text, &theme, t2, |ui| tip.draw(ui));
    assert!(rec.is_empty(), "kapanınca katman boş");
}
