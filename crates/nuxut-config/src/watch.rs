use std::ffi::OsString;
use std::io;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

use inotify::{Inotify, WatchMask};

/// Config dizinini izler ve ilgili dosyalar değişince haber verir.
///
/// Dosyanın kendisi değil **dizin** izlenir: editörler çoğunlukla geçici dosyaya yazıp
/// yeniden adlandırır (inode değişir), dosya izleyicisi bu durumda kopar. Tanımlayıcı
/// engellemesizdir; bir event loop'a `fd()` ile bağlanır, uyanınca `changed_files()` okunur.
#[derive(Debug)]
pub struct ConfigWatcher {
    inotify: Inotify,
    dir: PathBuf,
    names: Vec<OsString>,
    buffer: Vec<u8>,
}

impl ConfigWatcher {
    /// `dir` içinde adı `names` olan dosyaları izler. Dizin yoksa oluşturulur (boş dizin
    /// zararsızdır; dosya oluşturulmaz).
    pub fn new(dir: &Path, names: &[&str]) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let inotify = Inotify::init()?;
        inotify.watches().add(
            dir,
            WatchMask::CLOSE_WRITE
                | WatchMask::MOVED_TO
                | WatchMask::MOVED_FROM
                | WatchMask::CREATE
                | WatchMask::DELETE,
        )?;
        Ok(Self {
            inotify,
            dir: dir.to_owned(),
            names: names.iter().map(OsString::from).collect(),
            buffer: vec![0; 4096],
        })
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.inotify.as_fd()
    }

    /// Bekleyen olayları tüketir ve değişen izlenen dosyaların yollarını döndürür
    /// (tekrarsız). Bekleyen olay yoksa boş döner, bloklamaz.
    pub fn changed_files(&mut self) -> Vec<PathBuf> {
        let mut changed: Vec<PathBuf> = Vec::new();
        loop {
            let events = match self.inotify.read_events(&mut self.buffer) {
                Ok(events) => events,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => {
                    tracing::warn!("config izleme okunamadı: {e}");
                    break;
                }
            };
            let mut any = false;
            for event in events {
                any = true;
                let Some(name) = event.name else { continue };
                if self.names.iter().any(|n| n.as_os_str() == name) {
                    let path = self.dir.join(name);
                    if !changed.contains(&path) {
                        changed.push(path);
                    }
                }
            }
            if !any {
                break;
            }
        }
        changed
    }
}
