//! Çoklu seçim modeli ([`Selection`]): dosya ızgarası ve listelerin ortak seçim kuralları
//! (GTK/Nautilus, Windows Gezgini ve macOS Finder'daki ortak davranış).
//!
//! - **Tık:** yalnız o öğe; **Ctrl+tık** öğeyi ekler/çıkarır; **Shift+tık** çapadan öğeye aralık
//!   (önceki seçimin yerine); **Ctrl+Shift+tık** aralığı ekler.
//! - **Ok tuşları:** imleç gider, seçim yalnız imleç; **Shift+ok** çapadan imlece aralık;
//!   **Ctrl+ok** imleç gider, seçim değişmez (Ctrl+Space imleçteki öğeyi ekler/çıkarır).
//! - **Ctrl+A** hepsi. Tekli kipte (`multiple == false`) her işlem tek öğe seçer.
//!
//! Depolama bit kümesi: 100 000 öğede ~12 KB; sayım tutulur (`count` O(1)).

use crate::input::Modifiers;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    bits: Vec<u64>,
    len: usize,
    count: usize,
    cursor: usize,
    anchor: usize,
    multiple: bool,
}

impl Selection {
    /// Boş seçim. `multiple`: çoklu seçim açık mı.
    pub fn new(multiple: bool) -> Self {
        Self { multiple, ..Self::default() }
    }

    /// Yeni veri: seçim boşalır, imleç ve çapa başa.
    pub fn reset(&mut self, len: usize) {
        self.len = len;
        self.bits.clear();
        self.bits.resize(len.div_ceil(64), 0);
        self.count = 0;
        self.cursor = 0;
        self.anchor = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn is_multiple(&self) -> bool {
        self.multiple
    }

    /// Çoklu kip kapanınca seçim imleçteki öğeye iner.
    pub fn set_multiple(&mut self, multiple: bool) -> bool {
        if self.multiple == multiple {
            return false;
        }
        self.multiple = multiple;
        if !multiple && self.count > 1 {
            let c = self.cursor;
            self.only(c);
            return true;
        }
        false
    }

    /// İmleç (klavye odağındaki öğe). Boş veride 0.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn is_selected(&self, i: usize) -> bool {
        i < self.len && self.bits[i / 64] & (1 << (i % 64)) != 0
    }

    /// Seçili indisler, artan sırada.
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.bits.iter().enumerate().flat_map(|(w, &word)| {
            let mut word = word;
            std::iter::from_fn(move || {
                if word == 0 {
                    return None;
                }
                let b = word.trailing_zeros() as usize;
                word &= word - 1;
                Some(w * 64 + b)
            })
        })
    }

    fn set_bit(&mut self, i: usize, on: bool) {
        let (w, m) = (i / 64, 1u64 << (i % 64));
        let was = self.bits[w] & m != 0;
        if was != on {
            self.bits[w] ^= m;
            if on {
                self.count += 1;
            } else {
                self.count -= 1;
            }
        }
    }

    fn clear_bits(&mut self) {
        self.bits.iter_mut().for_each(|w| *w = 0);
        self.count = 0;
    }

    fn only(&mut self, i: usize) {
        self.clear_bits();
        self.set_bit(i, true);
        self.cursor = i;
        self.anchor = i;
    }

    fn range(&mut self, to: usize, add: bool) {
        if !add {
            self.clear_bits();
        }
        let (a, b) = if self.anchor <= to { (self.anchor, to) } else { (to, self.anchor) };
        for i in a..=b.min(self.len - 1) {
            self.set_bit(i, true);
        }
        self.cursor = to;
    }

    /// Durumu karşılaştırmak için (değişti mi).
    /// Girdi başına bir kopya (100 000 öğede ~12 KB); kesin karşılaştırma. İmleç ayrı okunur.
    fn snapshot(&self) -> Vec<u64> {
        self.bits.clone()
    }

    /// Olaysız: yalnız `i` seçili (aralık dışıysa sona kırpılır).
    pub fn select_only(&mut self, i: usize) -> bool {
        if self.len == 0 {
            return false;
        }
        let before = self.snapshot();
        self.only(i.min(self.len - 1));
        before != self.snapshot()
    }

    /// Seçimi boşaltır (imleç yerinde).
    pub fn clear(&mut self) -> bool {
        let changed = self.count > 0;
        self.clear_bits();
        changed
    }

    /// Hepsi (yalnız çoklu kipte).
    pub fn select_all(&mut self) -> bool {
        if !self.multiple || self.len == 0 || self.count == self.len {
            return false;
        }
        for i in 0..self.len {
            self.set_bit(i, true);
        }
        true
    }

    /// İmleçteki öğeyi ekler/çıkarır (Ctrl+Space; tekli kipte seçer).
    pub fn toggle_cursor(&mut self) -> bool {
        if self.len == 0 {
            return false;
        }
        let c = self.cursor;
        if !self.multiple {
            return self.select_only(c);
        }
        let on = !self.is_selected(c);
        self.set_bit(c, on);
        self.anchor = c;
        true
    }

    /// İşaretçi tıklaması (değiştiricilerle). Seçim kümesi değiştiyse `true` (imleç `cursor()`).
    pub fn click(&mut self, i: usize, modifiers: Modifiers) -> bool {
        if i >= self.len {
            return false;
        }
        let before = self.snapshot();
        match (self.multiple, modifiers.ctrl, modifiers.shift) {
            (true, _, true) => self.range(i, modifiers.ctrl),
            (true, true, false) => {
                let on = !self.is_selected(i);
                self.set_bit(i, on);
                self.cursor = i;
                self.anchor = i;
            }
            _ => self.only(i),
        }
        before != self.snapshot()
    }

    /// Klavye gezinmesi `i`'ye (değiştiricilerle). Seçim kümesi değiştiyse `true` (imleç `cursor()`).
    pub fn navigate(&mut self, i: usize, modifiers: Modifiers) -> bool {
        if self.len == 0 {
            return false;
        }
        let i = i.min(self.len - 1);
        let before = self.snapshot();
        match (self.multiple, modifiers.ctrl, modifiers.shift) {
            (true, _, true) => self.range(i, false),
            (true, true, false) => self.cursor = i,
            _ => self.only(i),
        }
        before != self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Modifiers = Modifiers { ctrl: false, shift: false, alt: false, logo: false, caps_lock: false, num_lock: false };
    const CTRL: Modifiers = Modifiers { ctrl: true, ..NONE };
    const SHIFT: Modifiers = Modifiers { shift: true, ..NONE };
    const BOTH: Modifiers = Modifiers { ctrl: true, shift: true, ..NONE };

    fn sel(n: usize) -> Selection {
        let mut s = Selection::new(true);
        s.reset(n);
        s
    }

    fn items(s: &Selection) -> Vec<usize> {
        s.iter().collect()
    }

    #[test]
    fn click_variants() {
        let mut s = sel(200);
        assert!(s.click(3, NONE));
        assert_eq!(items(&s), [3]);
        assert!(s.click(130, CTRL));
        assert_eq!(items(&s), [3, 130]);
        assert_eq!(s.anchor(), 130);
        assert!(s.click(128, SHIFT), "çapadan aralık, öncekinin yerine");
        assert_eq!(items(&s), [128, 129, 130]);
        assert_eq!(s.anchor(), 130, "Shift çapayı oynatmaz");
        s.click(5, CTRL);
        assert!(s.click(7, BOTH), "Ctrl+Shift aralığı ekler");
        assert_eq!(items(&s), [5, 6, 7, 128, 129, 130]);
        assert!(s.click(5, CTRL), "Ctrl+tık çıkarır");
        assert!(!s.is_selected(5));
        assert_eq!(s.count(), 5);
        assert!(!s.click(500, NONE), "aralık dışı");
        assert!(s.click(1, NONE));
        assert!(!s.click(1, NONE), "aynı tık değişmez");
    }

    #[test]
    fn keyboard_variants() {
        let mut s = sel(10);
        s.select_only(2);
        assert!(s.navigate(3, SHIFT));
        assert!(s.navigate(5, SHIFT));
        assert_eq!(items(&s), [2, 3, 4, 5]);
        assert!(s.navigate(1, SHIFT), "çapanın öbür yanına");
        assert_eq!(items(&s), [1, 2]);
        assert!(!s.navigate(8, CTRL), "Ctrl+ok yalnız imleç: seçim değişmez");
        assert_eq!(items(&s), [1, 2]);
        assert_eq!(s.cursor(), 8);
        assert!(s.toggle_cursor());
        assert_eq!(items(&s), [1, 2, 8]);
        assert!(s.navigate(9, NONE));
        assert_eq!(items(&s), [9]);
        assert!(s.select_all());
        assert_eq!(s.count(), 10);
        assert!(!s.select_all());
        assert!(s.clear());
        assert!(s.is_empty());
    }

    #[test]
    fn single_mode_never_selects_more() {
        let mut s = Selection::new(false);
        s.reset(10);
        s.click(1, NONE);
        s.click(4, CTRL);
        s.click(7, SHIFT);
        s.navigate(2, SHIFT);
        assert!(!s.select_all());
        s.toggle_cursor();
        assert_eq!(items(&s), [2]);
        let mut m = sel(10);
        m.click(1, NONE);
        m.click(4, SHIFT);
        assert!(m.set_multiple(false));
        assert_eq!(items(&m), [4], "imleçte kalır");
    }

    #[test]
    fn reset_and_word_boundaries() {
        let mut s = sel(130);
        s.click(63, NONE);
        s.click(64, BOTH);
        s.click(129, CTRL);
        assert_eq!(items(&s), [63, 64, 129]);
        s.reset(5);
        assert!(s.is_empty());
        assert_eq!(s.cursor(), 0);
        let mut e = sel(0);
        assert!(!e.navigate(3, NONE));
        assert!(!e.toggle_cursor());
    }
}
