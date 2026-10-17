//! Yüzey başına uygulama ikonu dokuları: [`AppIcons`]'ın çözdüğü görsel ilk istendiği karede
//! bir kez yüklenir, sonraki karelerde aynı doku döner. Dokular renderer'a (yüzeye) aittir;
//! çözücü paylaşılır, bu önbellek yüzey başına tutulur.
//!
//! ```ignore
//! // draw içinde:
//! let icon = match textures.get(painter, &app_icons, &window.class, 24, scale.ceil() as u32) {
//!     Some(t) => Icon::image(t),
//!     None => Icon::named(IconName::Window), // çözülüyor ya da bulunamadı
//! };
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use nuxut_render::{ImageData, Painter, Texture, TextureOptions};

use super::{AppIcon, AppIcons, Key, MAX_APP_ICON_SCALE, MAX_APP_ICON_SIZE, desktop};

/// Yüzey başına tutulan en fazla doku (en eski kullanılan atılır).
pub const MAX_APP_ICON_TEXTURES: usize = 128;

#[derive(Debug)]
struct Entry {
    texture: Texture,
    used: u64,
}

/// Bir yüzeyin uygulama ikonu dokuları.
#[derive(Debug, Default)]
pub struct AppIconTextures {
    /// Dokuların ait olduğu çözücü nesli; değişince (tema değişti) hepsi düşer.
    generation: u64,
    map: HashMap<Key, Entry>,
    tick: u64,
}

impl AppIconTextures {
    pub fn new() -> Self {
        Self::default()
    }

    /// `app_id`'nin dokusu; çözülüyorsa ya da bulunamadıysa `None` (çağıran genel ikonu
    /// çizer; çözüm bitince `AppIcons`'ın `notify`'ı yeniden çizim ister). Yalnızca çizimde
    /// (painter gerekir) çağrılır; IO yapmaz.
    pub fn get(&mut self, painter: &mut Painter, icons: &AppIcons, app_id: &str, size: u32, scale: u32) -> Option<Texture> {
        let generation = icons.generation();
        if generation != self.generation {
            self.map.clear();
            self.generation = generation;
        }
        let id = desktop::clean_app_id(app_id)?;
        let key = Key { app_id: id.to_owned(), size: size.clamp(1, MAX_APP_ICON_SIZE), scale: scale.clamp(1, MAX_APP_ICON_SCALE) };
        self.tick += 1;
        if let Some(e) = self.map.get_mut(&key) {
            e.used = self.tick;
            return Some(e.texture.clone());
        }
        let AppIcon::Ready(image) = icons.get(id, key.size, key.scale) else { return None };
        let (width, height) = image.size();
        let data = ImageData { width, height, pixels: Arc::from(image.pixels()) };
        // Görsel zaten hedef boyuta küçültüldü: mipmap gereksiz.
        let texture = painter.upload_texture(data, TextureOptions { mipmaps: false });
        if self.map.len() >= MAX_APP_ICON_TEXTURES
            && let Some(old) = self.map.iter().min_by_key(|(_, e)| e.used).map(|(k, _)| k.clone())
        {
            self.map.remove(&old);
        }
        self.map.insert(key, Entry { texture: texture.clone(), used: self.tick });
        Some(texture)
    }

    /// Tüm dokuları bırakır (ör. yüzey kapanırken).
    pub fn clear(&mut self) {
        self.map.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}
