//! Premultiplied RGBA8 görsel.

use crate::ImageError;

/// Çözülmüş görsel: satırlar sıkı, **premultiplied** RGBA8.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

impl Image {
    /// Zaten premultiplied pikseller.
    pub fn from_premultiplied(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, ImageError> {
        let expected = (width as usize).checked_mul(height as usize).and_then(|n| n.checked_mul(4));
        if width == 0 || height == 0 {
            return Err(ImageError::Empty);
        }
        if expected != Some(pixels.len()) {
            return Err(ImageError::InvalidData(format!("{width}x{height} için {} bayt", pixels.len())));
        }
        Ok(Self { width, height, pixels })
    }

    /// Düz (straight) alpha RGBA8; yerinde premultiply edilir.
    pub fn from_straight(width: u32, height: u32, mut pixels: Vec<u8>) -> Result<Self, ImageError> {
        premultiply(&mut pixels);
        Self::from_premultiplied(width, height, pixels)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }

    pub fn byte_size(&self) -> usize {
        self.pixels.len()
    }

    /// Tek piksel, düz alpha olarak (test ve palet örneklemesi için).
    pub fn straight_pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.pixels[i..i + 4];
        unpremultiply_px([p[0], p[1], p[2], p[3]])
    }

    /// Düz alpha RGBA8 kopyası (palet çıkarma gibi okuyucular için).
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.chunks_exact_mut(4) {
            let p = unpremultiply_px([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&p);
        }
        out
    }
}

#[cfg(feature = "render")]
impl Image {
    /// Doku yükleme biçimi (`Painter::upload_texture`); `Arc<[u8]>`ye tek kopya.
    pub fn into_image_data(self) -> nuxut_render::ImageData {
        nuxut_render::ImageData { width: self.width, height: self.height, pixels: self.pixels.into() }
    }
}

#[cfg(feature = "render")]
impl From<Image> for nuxut_render::ImageData {
    fn from(i: Image) -> Self {
        i.into_image_data()
    }
}

pub(crate) fn premultiply(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        if a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
}

fn unpremultiply_px(p: [u8; 4]) -> [u8; 4] {
    let a = u16::from(p[3]);
    match a {
        0 => [0, 0, 0, 0],
        255 => p,
        _ => {
            let un = |c: u8| ((u16::from(c) * 255 + a / 2) / a).min(255) as u8;
            [un(p[0]), un(p[1]), un(p[2]), p[3]]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premultiplies_and_back() {
        let img = Image::from_straight(1, 1, vec![255, 128, 0, 128]).unwrap();
        assert_eq!(img.pixels(), &[128, 64, 0, 128]);
        let back = img.straight_pixel(0, 0);
        assert_eq!(back[3], 128);
        assert!(back[0] >= 254 && (127..=129).contains(&back[1]));
    }

    #[test]
    fn rejects_wrong_length() {
        assert!(Image::from_premultiplied(2, 2, vec![0; 15]).is_err());
        assert!(matches!(Image::from_premultiplied(0, 2, vec![]), Err(ImageError::Empty)));
    }
}
