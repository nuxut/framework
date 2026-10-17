//! Linux dışı platformlar (ADR-0003): config dosyası izleme henüz yok. Platform katmanı
//! (winit) geldiğinde platformun kendi izleme API'siyle doldurulur. O zamana kadar değişiklik
//! raporlanmaz; config uygulama başlangıcında okunur.

use std::io;
use std::path::{Path, PathBuf};

/// İzleyici bu platformda kullanılamıyor: [`new`](Self::new) `Unsupported` döner.
#[derive(Debug)]
pub struct ConfigWatcher {
    _private: (),
}

impl ConfigWatcher {
    pub fn new(_dir: &Path, _names: &[&str]) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "config izleme bu platformda yok"))
    }

    pub fn changed_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
}
