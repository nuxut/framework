//! Klavye odak sırası: [`FocusScope`].
//!
//! Uygulama odaklanabilir bileşenlerini bir sıraya koyar (`0..len`); Tab / Shift+Tab sırayı
//! gezer (devre dışı öğeler atlanır, uçlarda dolanır). Odak halkası yalnızca klavyeyle
//! görünür (DESIGN.md "Odak"): fareyle odak verilince halka gizlenir. Diğer tuşlar (Enter,
//! Space, oklar) odaktaki bileşenin kendi `key`'ine gider.
//!
//! ```ignore
//! if let Some(change) = scope.key(&event) {
//!     for i in [change.from, Some(change.to)].into_iter().flatten() {
//!         widgets[i].set_focus_visible(scope.ring_visible_for(i));
//!     }
//! } else if let Some(i) = scope.focused() {
//!     widgets[i].key(&event, now);
//! }
//! ```

use crate::input::{Key, KeyEvent, KeyState};

/// Odak değişimi (eski ve yeni indis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusChange {
    pub from: Option<usize>,
    pub to: usize,
}

#[derive(Debug, Clone, Default)]
pub struct FocusScope {
    enabled: Vec<bool>,
    current: Option<usize>,
    ring: bool,
}

impl FocusScope {
    pub fn new(len: usize) -> Self {
        Self { enabled: vec![true; len], current: None, ring: false }
    }

    pub fn len(&self) -> usize {
        self.enabled.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }

    /// Öğe sayısı değişti; odak sınır dışına düştüyse kalkar.
    pub fn set_len(&mut self, len: usize) {
        self.enabled.resize(len, true);
        if self.current.is_some_and(|c| c >= len) {
            self.current = None;
        }
    }

    /// Devre dışı öğe Tab ile atlanır; odaktaysa odak kalkar.
    pub fn set_enabled(&mut self, index: usize, enabled: bool) {
        if let Some(e) = self.enabled.get_mut(index) {
            *e = enabled;
            if !enabled && self.current == Some(index) {
                self.current = None;
            }
        }
    }

    pub fn focused(&self) -> Option<usize> {
        self.current
    }

    /// Odak halkası görünür mü (yalnızca klavyeyle odaklanınca).
    pub fn ring_visible(&self) -> bool {
        self.ring && self.current.is_some()
    }

    /// `index` öğesi halka çizmeli mi.
    pub fn ring_visible_for(&self, index: usize) -> bool {
        self.ring && self.current == Some(index)
    }

    /// Fareyle odak (tıklanan alan): halka gizlenir.
    pub fn focus_pointer(&mut self, index: usize) -> Option<FocusChange> {
        self.ring = false;
        self.set(index)
    }

    /// Programatik odak (ör. diyalog açılınca ilk alan); halka klavye kipindeyse görünür.
    pub fn focus(&mut self, index: usize) -> Option<FocusChange> {
        self.set(index)
    }

    pub fn clear(&mut self) -> Option<usize> {
        self.current.take()
    }

    fn set(&mut self, index: usize) -> Option<FocusChange> {
        if !self.enabled.get(index).copied().unwrap_or(false) || self.current == Some(index) {
            return None;
        }
        let from = self.current.replace(index);
        Some(FocusChange { from, to: index })
    }

    /// Tab / Shift+Tab: sıradaki etkin öğeye geçer, halkayı görünür yapar ve olayı tüketir
    /// (`Some`). Diğer tuşlar `None` (odaktaki bileşene iletilmeli).
    pub fn key(&mut self, event: &KeyEvent) -> Option<FocusChange> {
        if event.key != Key::Tab || event.state == KeyState::Released || event.modifiers.ctrl || event.modifiers.alt {
            return None;
        }
        let n = self.enabled.len();
        if n == 0 || !self.enabled.iter().any(|e| *e) {
            return None;
        }
        let back = event.modifiers.shift;
        let mut i = match (self.current, back) {
            (Some(c), false) => (c + 1) % n,
            (Some(c), true) => (c + n - 1) % n,
            (None, false) => 0,
            (None, true) => n - 1,
        };
        while !self.enabled[i] {
            i = if back { (i + n - 1) % n } else { (i + 1) % n };
        }
        let was_ring = self.ring;
        self.ring = true;
        match self.set(i) {
            Some(c) => Some(c),
            // Aynı öğe (tek etkin öğe) — halka yeni görünür olduysa yine bildir.
            None if !was_ring => Some(FocusChange { from: Some(i), to: i }),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Modifiers;
    use crate::input::test_util::*;

    fn tab(shift: bool) -> KeyEvent {
        key_mod(Key::Tab, Modifiers { shift, ..Default::default() })
    }

    #[test]
    fn tab_cycles_and_skips_disabled() {
        let mut s = FocusScope::new(4);
        s.set_enabled(2, false);
        let order: Vec<usize> = (0..4).map(|_| s.key(&tab(false)).unwrap().to).collect();
        assert_eq!(order, vec![0, 1, 3, 0]);
        assert_eq!(s.key(&tab(true)).unwrap().to, 3);
        assert!(s.ring_visible_for(3));
    }

    #[test]
    fn pointer_focus_hides_ring() {
        let mut s = FocusScope::new(3);
        let _ = s.key(&tab(false));
        assert!(s.ring_visible());
        assert_eq!(s.focus_pointer(2), Some(FocusChange { from: Some(0), to: 2 }));
        assert!(!s.ring_visible());
        assert!(s.key(&key(Key::Enter)).is_none(), "diğer tuşlar bileşene gider");
    }

    #[test]
    fn shrinking_drops_focus() {
        let mut s = FocusScope::new(3);
        let _ = s.focus(2);
        s.set_len(2);
        assert_eq!(s.focused(), None);
        assert!(FocusScope::new(0).key(&tab(false)).is_none());
    }
}
