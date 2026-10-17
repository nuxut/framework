//! Sözde dil (`NUXUT_LANG=qps`, yalnızca geliştirme): metin parçaları aksanlanır, mesaj
//! köşeli parantezle sarılıp %35 uzatılır (`[Ŝéţţîñĝš···]`). Çevrilmemiş gömülü metin
//! (aksansız kalır) ve taşmalar (uzatılmış metin sığmaz) ekranda görünür.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

const LOWER: [char; 26] = [
    'å', 'ƀ', 'ç', 'ð', 'é', 'ƒ', 'ĝ', 'ĥ', 'î', 'ĵ', 'ķ', 'ļ', 'ɱ', 'ñ', 'ö', 'þ', 'ǫ', 'ŕ', 'š', 'ţ', 'û', 'ṽ', 'ŵ', 'ẋ',
    'ý', 'ž',
];
const UPPER: [char; 26] = [
    'Å', 'Ɓ', 'Ç', 'Ð', 'É', 'Ƒ', 'Ĝ', 'Ĥ', 'Î', 'Ĵ', 'Ķ', 'Ļ', 'Ṁ', 'Ñ', 'Ö', 'Þ', 'Ǫ', 'Ŕ', 'Ŝ', 'Ţ', 'Û', 'Ṽ', 'Ŵ', 'Ẋ',
    'Ý', 'Ž',
];

/// Uzatma oranı (%35).
const EXPANSION: f32 = 0.35;
/// Dolgu karakteri.
const PAD: char = '·';

/// `FluentBundle::set_transform` kancası: yalnızca metin parçaları (değişkenler değil).
pub(crate) fn transform(s: &str) -> Cow<'_, str> {
    if !s.bytes().any(|b| b.is_ascii_alphabetic()) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| match c {
                'a'..='z' => LOWER[(c as u8 - b'a') as usize],
                'A'..='Z' => UPPER[(c as u8 - b'A') as usize],
                c => c,
            })
            .collect(),
    )
}

/// Tüm mesaj: `[` + metin + %35 dolgu + `]`.
pub(crate) fn wrap(s: &str) -> String {
    let n = s.graphemes(true).count();
    let pad = (n as f32 * EXPANSION).ceil() as usize;
    let mut out = String::with_capacity(s.len() + pad * PAD.len_utf8() + 2);
    out.push('[');
    out.push_str(s);
    out.extend(std::iter::repeat_n(PAD, pad));
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_and_expands() {
        assert_eq!(transform("Settings"), "Ŝéţţîñĝš");
        assert!(matches!(transform("12 · %"), Cow::Borrowed(_)));
        let w = wrap("Ŝéţţîñĝš");
        assert_eq!(w, "[Ŝéţţîñĝš···]");
        assert_eq!(wrap(""), "[]");
    }
}
