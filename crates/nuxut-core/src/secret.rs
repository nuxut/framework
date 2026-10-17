//! Düşerken sıfırlanan metin ([`SecretText`]): klavyeden (`KeyEvent::text`) ve girdi
//! yönteminden (IME commit/preedit) gelen, parola olabilecek kısa ömürlü dizgiler.
//!
//! Tüketici kaynağını kırmamak için `String` gibi davranır: `Deref<Target = str>` (`&t`, `t.len()`,
//! `t.chars()`, `Option::as_deref`), `From<String>`/`From<&str>`/`From<char>`, `str`/`&str`/`String`
//! ile karşılaştırma. Farkları:
//!
//! - **`Drop`'ta sıfırlanır** (`zeroize`: derleyicinin silemeyeceği yazım; ayrılmış kapasitenin
//!   tamamı, kullanılmayan kısım dahil).
//! - **`Debug` içeriği yazmaz** (yalnızca karakter sayısı; ARCHITECTURE değişmez 9). `Display` yok:
//!   metin `format!`/log'a yanlışlıkla giremez; gerçekten gerekiyorsa `as_str()` açıkça çağrılır.
//! - **Kısaltma** ([`truncate_zeroized`](SecretText::truncate_zeroized)) atılan kuyruğu da sıfırlar.
//!
//! `Clone` vardır (olay tipleri `Clone`); her kopya kendi düşüşünde sıfırlanır. `String`'e geri
//! dönüş (`into_string`) bilerek yoktur: sıfırlanmayan bir kopya ancak `to_owned()` ile, açıkça
//! yapılır.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;

use zeroize::Zeroize;

#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretText(String);

impl SecretText {
    pub const fn new() -> Self {
        Self(String::new())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Ayrılmış kapasite (bayt; test ve ölçüm için).
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    /// İçeriği sıfırlar ve boşaltır (kapasite korunur, tamamı sıfır).
    pub fn clear(&mut self) {
        self.0.zeroize();
    }

    /// En çok `max` bayta (karakter sınırına aşağı yuvarlanarak) kısaltır. Sığıyorsa dokunmaz;
    /// kısalırsa korunan kısım yeni bir tampona kopyalanır ve eski tamponun **tamamı** sıfırlanır
    /// (`String::truncate` atılan baytları bellekte bırakırdı).
    pub fn truncate_zeroized(&mut self, max: usize) {
        if self.0.len() <= max {
            return;
        }
        let mut end = max;
        while !self.0.is_char_boundary(end) {
            end -= 1;
        }
        let kept = Self(self.0[..end].to_owned());
        // Eski değer düşerken (`Drop`) sıfırlanır.
        *self = kept;
    }
}

impl Drop for SecretText {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl Deref for SecretText {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SecretText {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for SecretText {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// Taşıma: kopya yok, gelen tampon sahiplenilir ve düşerken sıfırlanır.
impl From<String> for SecretText {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for SecretText {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<&String> for SecretText {
    fn from(s: &String) -> Self {
        Self(s.clone())
    }
}

impl From<char> for SecretText {
    fn from(c: char) -> Self {
        let mut s = String::with_capacity(c.len_utf8());
        s.push(c);
        Self(s)
    }
}

impl PartialEq<str> for SecretText {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for SecretText {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for SecretText {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl PartialEq<SecretText> for str {
    fn eq(&self, other: &SecretText) -> bool {
        self == other.0
    }
}

impl PartialEq<SecretText> for &str {
    fn eq(&self, other: &SecretText) -> bool {
        *self == other.0
    }
}

impl PartialEq<SecretText> for String {
    fn eq(&self, other: &SecretText) -> bool {
        *self == other.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} karakter gizli>", self.0.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn behaves_like_str() {
        let t = SecretText::from("ğüş");
        assert_eq!(t.len(), 6);
        assert_eq!(&*t, "ğüş");
        let owned = String::from("ğüş");
        assert_eq!(t, "ğüş");
        assert_eq!("ğüş", t);
        assert_eq!(t, owned);
        assert_eq!(owned, t);
        let o: Option<SecretText> = Some("a".into());
        assert_eq!(o.as_deref(), Some("a"));
        assert_eq!(SecretText::from('ş').as_str(), "ş");
        fn takes_str(_: &str) {}
        takes_str(&t);
    }

    #[test]
    fn debug_hides_content() {
        let s = format!("{:?}", SecretText::from("parola"));
        assert!(!s.contains("parola") && s.contains("6 karakter gizli"), "{s}");
    }

    #[test]
    fn clear_zeroes_and_keeps_capacity() {
        let mut t = SecretText::from(String::from("gizli"));
        let cap = t.capacity();
        t.clear();
        assert!(t.is_empty());
        assert_eq!(t.capacity(), cap, "zeroize kapasiteyi bırakır; baytlar sıfırlandı");
    }

    #[test]
    fn truncate_cuts_on_char_boundary() {
        let mut t = SecretText::from("aşb");
        t.truncate_zeroized(2); // 'ş' ortası → 1
        assert_eq!(t, "a");
        assert_eq!(t.capacity(), 1, "yeni, tam boy tampon");
        let mut u = SecretText::from("ab");
        u.truncate_zeroized(10);
        assert_eq!(u, "ab");
    }
}
