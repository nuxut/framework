//! Platformdan bağımsız girdi olayları (`nuxut_core::input`; runtime aynı tipleri verir).

pub use nuxut_core::input::*;

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use nuxut_core::Point;

    pub fn motion(x: f32, y: f32) -> PointerEvent {
        PointerEvent::new(Point::new(x, y), PointerKind::Motion)
    }
    pub fn press(x: f32, y: f32) -> PointerEvent {
        PointerEvent::new(Point::new(x, y), PointerKind::Press(Button::Left))
    }
    pub fn release(x: f32, y: f32) -> PointerEvent {
        PointerEvent::new(Point::new(x, y), PointerKind::Release(Button::Left))
    }
    pub fn key(key: Key) -> KeyEvent {
        KeyEvent { key, state: KeyState::Pressed, text: None, modifiers: Modifiers::default(), xkb: None }
    }
    pub fn key_mod(key: Key, modifiers: Modifiers) -> KeyEvent {
        KeyEvent { key, state: KeyState::Pressed, text: None, modifiers, xkb: None }
    }
    pub fn typed(c: char) -> KeyEvent {
        KeyEvent { key: Key::Char(c), state: KeyState::Pressed, text: Some(c.into()), modifiers: Modifiers::default(), xkb: None }
    }
}
