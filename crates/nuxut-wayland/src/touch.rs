//! Dokunmatik ekran (`wl_touch`): birincil temasın işaretçi öykünmesi.
//!
//! Ham olaylar (`App::touch`) tüm temaslar için gelir. Uygulama bir temasın `Down`'ında `true`
//! dönerse (sahiplendi) o temas öykünülmez. Sahiplenilmeyen ilk temas **birincil** olur ve
//! `PointerSource::Touch` kaynaklı işaretçi olaylarına çevrilir:
//!
//! - `Down` → `Enter`, `Motion`, `Press(Left)`
//! - `Motion` → `Motion`
//! - `Up` → `Release(Left)`, `Leave`
//! - `Cancel` → `Leave` (bırakma yok: bileşenler eylem üretmez)
//!
//! Birincil temas sürerken gelen diğer sahiplenilmemiş temaslar öykünülmez (bileşen modeli tek
//! işaretçilidir). Birincil temas [`TOUCH_TAP_DELAY`] boyunca kımıldamazsa runtime aynı konumda
//! bir `Motion` üretir ([`TouchEmulator::hold_tick`]): kaydırma görünümleri bekletilen basmayı
//! çocuklarına bununla iletir (zamanlayıcısız bileşen modeli).
//!
//! Bu modül Wayland bilmez: olay dizileri birim testlerle sınanır.

use nuxut_core::Point;
use nuxut_core::input::{Button, PointerEvent, PointerKind, TOUCH_SLOP, TouchId};

use crate::SurfaceId;

/// Aynı anda izlenen en çok temas; fazlası yok sayılır (bozuk/kötü niyetli compositor'a karşı
/// sınır; gerçek ekranlar 10 temas bildirir).
pub(crate) const MAX_CONTACTS: usize = 16;

#[derive(Debug, Clone, Copy)]
struct Contact {
    id: TouchId,
    surface: SurfaceId,
    position: Point,
    down_at: Point,
    /// Uygulama `App::touch` ile sahiplendi: öykünülmez.
    claimed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct TouchEmulator {
    contacts: Vec<Contact>,
    primary: Option<TouchId>,
    /// Birincil temasın kuşağı: bekleme zamanlayıcısı eski temas için ateşlenirse yok sayılır.
    generation: u64,
}

/// Öykünmenin ürettiği işaretçi olayları (yüzeyiyle). Çoğu çağrı en çok üç olay üretir.
pub(crate) type Emitted = Vec<(SurfaceId, PointerEvent)>;

impl TouchEmulator {
    /// Yeni temas. `claimed`: uygulama ham olayı sahiplendi. Birincil olduysa bekleme
    /// zamanlayıcısının kuşağını döner (zamanlayıcı bu kuşakla [`hold_tick`] çağırır).
    pub(crate) fn down(
        &mut self,
        surface: SurfaceId,
        id: TouchId,
        position: Point,
        claimed: bool,
        out: &mut Emitted,
    ) -> Option<u64> {
        // Aynı kimlik bitmeden yeniden geldiyse (bozuk dizi) eskisini kapat.
        if self.contacts.iter().any(|c| c.id == id) {
            self.end(id, false, out);
        }
        if self.contacts.len() >= MAX_CONTACTS {
            return None;
        }
        self.contacts.push(Contact { id, surface, position, down_at: position, claimed });
        if claimed || self.primary.is_some() {
            return None;
        }
        self.primary = Some(id);
        self.generation = self.generation.wrapping_add(1);
        out.push((surface, PointerEvent::touch(position, PointerKind::Enter)));
        out.push((surface, PointerEvent::touch(position, PointerKind::Motion)));
        out.push((surface, PointerEvent::touch(position, PointerKind::Press(Button::Left))));
        Some(self.generation)
    }

    /// Temas hareketi. Temasın yüzeyini döner (ham olay için); bilinmeyen temas `None`.
    pub(crate) fn motion(&mut self, id: TouchId, position: Point, out: &mut Emitted) -> Option<SurfaceId> {
        let primary = self.primary == Some(id);
        let c = self.contacts.iter_mut().find(|c| c.id == id)?;
        c.position = position;
        if primary {
            out.push((c.surface, PointerEvent::touch(position, PointerKind::Motion)));
        }
        Some(c.surface)
    }

    /// Temas kalktı. Temasın (yüzeyi, son konumu) döner.
    pub(crate) fn up(&mut self, id: TouchId, out: &mut Emitted) -> Option<(SurfaceId, Point)> {
        self.end(id, true, out)
    }

    /// Compositor tüm dizileri iptal etti. Ham `Cancel` için her temasın (kimlik, yüzey, konum).
    pub(crate) fn cancel(&mut self, out: &mut Emitted) -> Vec<(TouchId, SurfaceId, Point)> {
        let all: Vec<_> = self.contacts.iter().map(|c| (c.id, c.surface, c.position)).collect();
        for (id, ..) in &all {
            self.end(*id, false, out);
        }
        all
    }

    /// Bekleme zamanlayıcısı: birincil temas hâlâ basılı ve eşiği aşmadıysa aynı konumda bir
    /// `Motion` (kaydırma görünümü bekletilen basmayı çocuğa iletir).
    pub(crate) fn hold_tick(&mut self, generation: u64, out: &mut Emitted) {
        if generation != self.generation {
            return;
        }
        let Some(c) = self.primary.and_then(|p| self.contacts.iter().find(|c| c.id == p)) else {
            return;
        };
        if distance(c.position, c.down_at) <= TOUCH_SLOP {
            out.push((c.surface, PointerEvent::touch(c.position, PointerKind::Motion)));
        }
    }

    /// Yüzey yok edildi: üzerindeki temaslar sessizce düşer (olay üretilmez).
    pub(crate) fn surface_removed(&mut self, surface: SurfaceId) {
        if let Some(p) = self.primary
            && self.contacts.iter().any(|c| c.id == p && c.surface == surface)
        {
            self.primary = None;
        }
        self.contacts.retain(|c| c.surface != surface);
    }

    /// Dokunma yeteneği kalktı: her şey sıfırlanır.
    pub(crate) fn reset(&mut self) {
        self.contacts.clear();
        self.primary = None;
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.contacts.len()
    }

    fn end(&mut self, id: TouchId, release: bool, out: &mut Emitted) -> Option<(SurfaceId, Point)> {
        let i = self.contacts.iter().position(|c| c.id == id)?;
        let c = self.contacts.swap_remove(i);
        if self.primary == Some(id) {
            self.primary = None;
            if !c.claimed {
                if release {
                    out.push((c.surface, PointerEvent::touch(c.position, PointerKind::Release(Button::Left))));
                }
                out.push((c.surface, PointerEvent::touch(c.position, PointerKind::Leave)));
            }
        }
        Some((c.surface, c.position))
    }
}

fn distance(a: Point, b: Point) -> f32 {
    let d = a - b;
    (d.x * d.x + d.y * d.y).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuxut_core::input::PointerSource;

    fn s() -> SurfaceId {
        SurfaceId::for_test(1)
    }

    fn p(x: f32, y: f32) -> Point {
        Point::new(x, y)
    }

    fn kinds(out: &Emitted) -> Vec<PointerKind> {
        assert!(out.iter().all(|(_, e)| e.source == PointerSource::Touch));
        out.iter().map(|(_, e)| e.kind).collect()
    }

    #[test]
    fn single_tap_is_enter_press_release_leave() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        assert!(t.down(s(), TouchId(0), p(10.0, 20.0), false, &mut out).is_some());
        t.motion(TouchId(0), p(12.0, 21.0), &mut out);
        t.up(TouchId(0), &mut out);
        assert_eq!(
            kinds(&out),
            [
                PointerKind::Enter,
                PointerKind::Motion,
                PointerKind::Press(Button::Left),
                PointerKind::Motion,
                PointerKind::Release(Button::Left),
                PointerKind::Leave,
            ]
        );
        // Bırakma son konumda.
        assert_eq!(out[4].1.position, p(12.0, 21.0));
        assert_eq!(t.active(), 0);
    }

    #[test]
    fn cancel_leaves_without_release() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        t.down(s(), TouchId(3), p(1.0, 1.0), false, &mut out);
        out.clear();
        let raw = t.cancel(&mut out);
        assert_eq!(kinds(&out), [PointerKind::Leave]);
        assert_eq!(raw, [(TouchId(3), s(), p(1.0, 1.0))]);
        assert_eq!(t.active(), 0);
    }

    #[test]
    fn second_finger_is_not_emulated_until_primary_lifts() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        t.down(s(), TouchId(0), p(0.0, 0.0), false, &mut out);
        out.clear();
        assert!(t.down(s(), TouchId(1), p(50.0, 0.0), false, &mut out).is_none());
        assert_eq!(t.motion(TouchId(1), p(60.0, 0.0), &mut out), Some(s()));
        assert!(out.is_empty());
        t.up(TouchId(0), &mut out);
        assert_eq!(kinds(&out), [PointerKind::Release(Button::Left), PointerKind::Leave]);
        out.clear();
        // İkinci parmak hâlâ basılı: birincil olmaz (dizinin ortasında basma üretilmez).
        t.motion(TouchId(1), p(70.0, 0.0), &mut out);
        t.up(TouchId(1), &mut out);
        assert!(out.is_empty());
        // Sonraki temas yeniden birincil.
        assert!(t.down(s(), TouchId(1), p(0.0, 0.0), false, &mut out).is_some());
    }

    #[test]
    fn claimed_contact_is_not_emulated() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        assert!(t.down(s(), TouchId(0), p(0.0, 0.0), true, &mut out).is_none());
        assert_eq!(t.motion(TouchId(0), p(5.0, 0.0), &mut out), Some(s()));
        assert_eq!(t.up(TouchId(0), &mut out), Some((s(), p(5.0, 0.0))));
        assert!(out.is_empty());
        // Sahiplenilmiş temas sürerken sahiplenilmeyen temas birincil olur.
        t.down(s(), TouchId(0), p(0.0, 0.0), true, &mut out);
        assert!(t.down(s(), TouchId(1), p(9.0, 9.0), false, &mut out).is_some());
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn hold_tick_repeats_position_only_while_still() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        let g = t.down(s(), TouchId(0), p(10.0, 10.0), false, &mut out).unwrap();
        out.clear();
        t.motion(TouchId(0), p(12.0, 12.0), &mut out);
        out.clear();
        t.hold_tick(g, &mut out);
        assert_eq!(kinds(&out), [PointerKind::Motion]);
        assert_eq!(out[0].1.position, p(12.0, 12.0));
        out.clear();
        // Eşiği aştı: tik yok (sürükleme zaten Motion üretiyor).
        t.motion(TouchId(0), p(10.0, 30.0), &mut out);
        out.clear();
        t.hold_tick(g, &mut out);
        assert!(out.is_empty());
        // Eski kuşağın tiki yeni teması etkilemez.
        t.up(TouchId(0), &mut out);
        t.down(s(), TouchId(0), p(0.0, 0.0), false, &mut out);
        out.clear();
        t.hold_tick(g, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn unknown_and_removed_surfaces_are_dropped() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        assert_eq!(t.motion(TouchId(9), p(0.0, 0.0), &mut out), None);
        assert_eq!(t.up(TouchId(9), &mut out), None);
        t.down(s(), TouchId(0), p(0.0, 0.0), false, &mut out);
        out.clear();
        t.surface_removed(s());
        assert_eq!(t.up(TouchId(0), &mut out), None);
        assert!(out.is_empty());
        assert!(t.down(s(), TouchId(1), p(0.0, 0.0), false, &mut out).is_some());
    }

    #[test]
    fn duplicate_down_closes_previous_and_contacts_are_bounded() {
        let mut t = TouchEmulator::default();
        let mut out = Vec::new();
        t.down(s(), TouchId(0), p(0.0, 0.0), false, &mut out);
        out.clear();
        t.down(s(), TouchId(0), p(4.0, 4.0), false, &mut out);
        assert_eq!(
            kinds(&out),
            [PointerKind::Leave, PointerKind::Enter, PointerKind::Motion, PointerKind::Press(Button::Left)]
        );
        for i in 1..40 {
            t.down(s(), TouchId(i), p(0.0, 0.0), false, &mut out);
        }
        assert_eq!(t.active(), MAX_CONTACTS);
    }
}
