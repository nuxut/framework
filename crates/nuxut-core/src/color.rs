//! sRGB renk. Saklama düz (straight) alpha ile; GPU'ya giderken `premultiplied()` kullanılır.

use std::fmt;
use std::str::FromStr;

use crate::geometry::lerp;

#[derive(Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::rgba(r, g, b, 1.0)
    }

    pub const fn from_rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0)
    }

    /// `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa` (baştaki `#` opsiyonel).
    pub fn from_hex(s: &str) -> Result<Self, ColorParseError> {
        let hex = s.trim().trim_start_matches('#');
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ColorParseError(s.to_owned()));
        }
        let nibble = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map(|v| v * 17);
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16);
        let parsed = match hex.len() {
            3 => (nibble(0), nibble(1), nibble(2), Ok(255)),
            4 => (nibble(0), nibble(1), nibble(2), nibble(3)),
            6 => (byte(0), byte(2), byte(4), Ok(255)),
            8 => (byte(0), byte(2), byte(4), byte(6)),
            _ => return Err(ColorParseError(s.to_owned())),
        };
        match parsed {
            (Ok(r), Ok(g), Ok(b), Ok(a)) => Ok(Self::from_rgba8(r, g, b, a)),
            _ => Err(ColorParseError(s.to_owned())),
        }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub fn multiply_alpha(self, f: f32) -> Self {
        Self { a: self.a * f, ..self }
    }

    pub fn premultiplied(&self) -> [f32; 4] {
        [self.r * self.a, self.g * self.a, self.b * self.a, self.a]
    }

    pub fn lerp(&self, to: &Self, t: f32) -> Self {
        Self::rgba(
            lerp(self.r, to.r, t),
            lerp(self.g, to.g, t),
            lerp(self.b, to.b, t),
            lerp(self.a, to.a, t),
        )
    }

    /// İki rengi `t` oranında karıştırır (state layer, hover tonu gibi).
    pub fn mix(&self, over: &Self, t: f32) -> Self {
        self.lerp(&over.with_alpha(self.a), t.clamp(0.0, 1.0))
    }

    pub fn to_hex(&self) -> String {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        if self.a >= 1.0 {
            format!("#{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b))
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", c(self.r), c(self.g), c(self.b), c(self.a))
        }
    }
}

impl fmt::Debug for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Color({})", self.to_hex())
    }
}

impl FromStr for Color {
    type Err = ColorParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorParseError(pub String);

impl fmt::Display for ColorParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "geçersiz renk '{}' (beklenen: #rgb, #rgba, #rrggbb veya #rrggbbaa)", self.0)
    }
}

impl std::error::Error for ColorParseError {}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_hex_forms() {
        assert_eq!(Color::from_hex("#fff").unwrap(), Color::WHITE);
        assert_eq!(Color::from_hex("000000").unwrap(), Color::BLACK);
        assert_eq!(Color::from_hex("#ff000080").unwrap().to_hex(), "#ff000080");
        assert_eq!(Color::from_hex("#f008").unwrap().to_hex(), "#ff000088");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Color::from_hex("#ggg").is_err());
        assert!(Color::from_hex("#12345").is_err());
        assert!(Color::from_hex("").is_err());
        // Çok baytlı karakterler dilimlemede panik üretmemeli.
        assert!(Color::from_hex("#ğğğ").is_err());
    }

    #[test]
    fn premultiply() {
        let c = Color::rgba(1.0, 0.5, 0.0, 0.5).premultiplied();
        assert_eq!(c, [0.5, 0.25, 0.0, 0.5]);
    }
}
