//! Kullanıcı çeviri dizini izleme (ADR-0010 §3): `<kök>/<domain>/*.ftl` değişince runtime
//! katalogu yeniden kurar ([`crate::reload`]) ve `App::locale_changed` gönderir.
//!
//! Üç parça, ikisi saf:
//! - [`Debounce`]: olay patlamasını tek yeniden kurmaya indirir (son olaydan [`DEBOUNCE`]
//!   sonra; sürekli olay akışında en geç [`MAX_WAIT`]). Saat dışarıdan verilir.
//! - [`ChangeTracker`]: "katalog girdisi gerçekten değişti mi" kararı. Önce ucuz meta özeti
//!   (ad, boy, mtime, inode); meta aynıysa dosya okunmaz. Meta değiştiyse içerik özeti
//!   karşılaştırılır: içerik aynıysa (editör aynı metni yeniden kaydetti) yeniden kurma yok.
//! - `UserDirWatcher` (Linux, inotify): olay tabanlı, yoklama yok; boşta sıfır CPU.
//!   Dizin yoksa en yakın var olan üst dizini (en çok [`MAX_ANCESTORS`] düzey yukarı) izler,
//!   dizin oluşunca aşağı iner. Symlink izlenmez (`IN_DONT_FOLLOW`; symlink dizine inilmez).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

/// Son olaydan sonra yeniden kurmadan önce beklenen süre.
pub const DEBOUNCE: Duration = Duration::from_millis(200);
/// Sürekli olay akışında ilk olaydan sonra en geç bu kadar beklenir.
pub const MAX_WAIT: Duration = Duration::from_secs(1);
/// Domain dizininden yukarı en çok bu kadar üst dizin izlenir: `locales`, `nuxut`,
/// `$XDG_DATA_HOME`. Veri dizini de yoksa izlenmez (açılışta bir kez debug logu).
pub const MAX_ANCESTORS: usize = 3;

/// [`Debounce::poll`] sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebouncePoll {
    /// Bekleyen olay yok.
    Idle,
    /// Henüz erken; bu ana kadar bekle.
    Wait(Instant),
    /// Şimdi yeniden kur (durum sıfırlandı).
    Fire,
}

/// Sondan tetiklenen (trailing) birleştirici; üst sınırlı.
#[derive(Debug, Clone, Copy)]
pub struct Debounce {
    delay: Duration,
    max_wait: Duration,
    first: Option<Instant>,
    last: Option<Instant>,
}

impl Default for Debounce {
    fn default() -> Self {
        Self::new(DEBOUNCE, MAX_WAIT)
    }
}

impl Debounce {
    pub const fn new(delay: Duration, max_wait: Duration) -> Self {
        Self {
            delay,
            max_wait,
            first: None,
            last: None,
        }
    }

    /// İlgili bir olay geldi; güncel son tarihi döndürür.
    pub fn event(&mut self, now: Instant) -> Instant {
        self.first.get_or_insert(now);
        self.last = Some(now);
        self.deadline().unwrap_or(now)
    }

    /// Bekleyen olay varsa tetiklenme anı: `min(son + gecikme, ilk + üst sınır)`.
    pub fn deadline(&self) -> Option<Instant> {
        let (first, last) = (self.first?, self.last?);
        Some((last + self.delay).min(first + self.max_wait))
    }

    pub fn is_pending(&self) -> bool {
        self.first.is_some()
    }

    /// Zamanı geldiyse [`DebouncePoll::Fire`] (ve sıfırlar).
    pub fn poll(&mut self, now: Instant) -> DebouncePoll {
        match self.deadline() {
            None => DebouncePoll::Idle,
            Some(d) if now < d => DebouncePoll::Wait(d),
            Some(_) => {
                self.first = None;
                self.last = None;
                DebouncePoll::Fire
            }
        }
    }
}

/// Kullanıcı dosyalarının özeti; karar [`ChangeTracker::take_change`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeTracker {
    root: std::path::PathBuf,
    domains: Vec<String>,
    meta: u64,
    content: u64,
}

impl ChangeTracker {
    /// Taban özeti şimdi alınır (dosyalar bir kez okunur; dizin yoksa iş yok).
    pub fn new(root: &Path, domains: &[&str]) -> Self {
        let domains: Vec<String> = domains.iter().map(|d| (*d).to_owned()).collect();
        let meta = meta_digest(root, &domains);
        let content = content_digest(root, &domains);
        Self {
            root: root.to_owned(),
            domains,
            meta,
            content,
        }
    }

    /// Son karardan bu yana katalog girdisi değişti mi? Meta aynıysa dosya okunmaz; meta
    /// değişip içerik aynıysa `false` (taban güncellenir).
    pub fn take_change(&mut self) -> bool {
        let meta = meta_digest(&self.root, &self.domains);
        if meta == self.meta {
            return false;
        }
        self.meta = meta;
        let content = content_digest(&self.root, &self.domains);
        if content == self.content {
            return false;
        }
        self.content = content;
        true
    }
}

/// Domain başına dil dosyaları, ada göre sıralı (yükleyiciyle aynı kurallar).
fn sorted_languages(root: &Path, domain: &str) -> Vec<String> {
    let mut langs = crate::user::languages(&root.join(domain));
    langs.sort_unstable();
    langs
}

fn meta_digest(root: &Path, domains: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    for domain in domains {
        domain.hash(&mut h);
        let dir = root.join(domain);
        for lang in sorted_languages(root, domain) {
            lang.hash(&mut h);
            match std::fs::symlink_metadata(dir.join(format!("{lang}.ftl"))) {
                Ok(m) => {
                    m.len().hash(&mut h);
                    m.modified().ok().hash(&mut h);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        (m.ino(), m.dev(), m.ctime(), m.ctime_nsec()).hash(&mut h);
                    }
                }
                Err(e) => e.kind().hash(&mut h),
            }
        }
    }
    h.finish()
}

fn content_digest(root: &Path, domains: &[String]) -> u64 {
    let mut h = DefaultHasher::new();
    for domain in domains {
        domain.hash(&mut h);
        let dir = root.join(domain);
        for lang in sorted_languages(root, domain) {
            lang.hash(&mut h);
            // Yükleyicinin güvenli okuyucusu (O_NOFOLLOW, ≤ 1 MiB): reddin nedeni de özete girer.
            crate::user::read(&dir.join(format!("{lang}.ftl"))).hash(&mut h);
        }
    }
    h.finish()
}

#[cfg(target_os = "linux")]
pub use linux::UserDirWatcher;

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::io;
    use std::os::fd::{AsFd, BorrowedFd};
    use std::path::{Path, PathBuf};

    use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

    use super::{ChangeTracker, MAX_ANCESTORS};

    /// İzlenen dizinin rolü.
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Role {
        /// `<kök>/<domain>`: `*.ftl` olayları ilgilidir.
        Domain,
        /// Henüz var olmayan bir alt dizinin üstü: yalnızca bu adlar ilgilidir.
        Ancestor(Vec<OsString>),
    }

    /// Kullanıcı çeviri dizinini inotify ile izler. Tanımlayıcı engellemesizdir; bir event
    /// loop'a [`fd`](Self::fd) ile bağlanır, uyanınca [`process_events`](Self::process_events)
    /// okunur; birleştirmeden sonra [`take_change`](Self::take_change) kararı verir.
    #[derive(Debug)]
    pub struct UserDirWatcher {
        inotify: Inotify,
        root: PathBuf,
        domains: Vec<String>,
        watches: HashMap<WatchDescriptor, (PathBuf, Role)>,
        buffer: Vec<u8>,
        tracker: ChangeTracker,
        /// `set_domains` sırasında eski domain'lerde bekleyen değişiklik.
        pending: bool,
    }

    impl UserDirWatcher {
        /// `root/<domain>` dizinlerini izler. Dizin oluşturulmaz; geçersiz domain adları
        /// (boş, `/`, `.` ile başlayan) atlanır. Taban özeti burada alınır.
        pub fn new(root: &Path, domains: &[&str]) -> io::Result<Self> {
            let domains: Vec<&str> = domains
                .iter()
                .copied()
                .filter(|d| valid_domain(d))
                .collect();
            let inotify = Inotify::init()?;
            let mut watcher = Self {
                inotify,
                root: root.to_owned(),
                domains: domains.iter().map(|d| (*d).to_owned()).collect(),
                watches: HashMap::new(),
                buffer: vec![0; 4096],
                tracker: ChangeTracker::new(root, &domains),
                pending: false,
            };
            watcher.rearm();
            Ok(watcher)
        }

        pub fn fd(&self) -> BorrowedFd<'_> {
            self.inotify.as_fd()
        }

        /// Şu an izlenen dizinler (tanı ve testler için; sıralı).
        pub fn watched_dirs(&self) -> Vec<PathBuf> {
            let mut dirs: Vec<PathBuf> = self.watches.values().map(|(p, _)| p.clone()).collect();
            dirs.sort();
            dirs
        }

        /// Bekleyen olayları tüketir (bloklamaz); ilgili bir olay olduysa `true`. Dizin
        /// oluştu/silindi/taşındıysa izleme yeniden kurulur.
        pub fn process_events(&mut self) -> bool {
            let mut relevant = false;
            let mut rearm = false;
            let mut dead: Vec<WatchDescriptor> = Vec::new();
            loop {
                let events = match self.inotify.read_events(&mut self.buffer) {
                    Ok(events) => events,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!("çeviri dizini izleme okunamadı: {e}");
                        break;
                    }
                };
                let mut any = false;
                for event in events {
                    any = true;
                    if event.mask.contains(EventMask::Q_OVERFLOW) {
                        relevant = true;
                        rearm = true;
                        continue;
                    }
                    let Some((_, role)) = self.watches.get(&event.wd) else {
                        continue;
                    };
                    if event.mask.intersects(
                        EventMask::IGNORED | EventMask::DELETE_SELF | EventMask::MOVE_SELF,
                    ) {
                        // İz ölü ya da taşınmış dizini gösteriyor: aynı yolda yeniden oluşan
                        // dizine eski iz yeniden kullanılmasın.
                        if !dead.contains(&event.wd) {
                            dead.push(event.wd.clone());
                        }
                        relevant = true;
                        rearm = true;
                        continue;
                    }
                    let Some(name) = event.name else { continue };
                    match role {
                        Role::Domain => {
                            if !event.mask.contains(EventMask::ISDIR) && is_ftl(name) {
                                relevant = true;
                            }
                        }
                        Role::Ancestor(names) => {
                            if names.iter().any(|n| n.as_os_str() == name) {
                                relevant = true;
                                rearm = true;
                            }
                        }
                    }
                }
                if !any {
                    break;
                }
            }
            for wd in dead {
                if self.watches.remove(&wd).is_some() {
                    // IGNORED sonrası EINVAL döner; önemsiz.
                    let _ = self.inotify.watches().remove(wd);
                }
            }
            if rearm {
                self.rearm();
            }
            relevant
        }

        /// Birleştirmeden sonra: katalog girdisi gerçekten değişti mi ([`ChangeTracker`]).
        pub fn take_change(&mut self) -> bool {
            std::mem::take(&mut self.pending) | self.tracker.take_change()
        }

        /// İzlenen domain kümesini değiştirir (çalışma başladıktan sonra kaydedilen domain;
        /// `nuxut_i18n::set_register_hook`). Geçersiz adlar atlanır. Küme değiştiyse izler
        /// yeniden kurulur ve `true` döner. Yeni domain'in o anki dosyaları taban sayılır (kayıt
        /// sonrası katalog zaten yeniden kurulur); eski domain'lerde birleştirmeyi bekleyen
        /// değişiklik kaybolmaz (sonraki [`take_change`](Self::take_change) `true`).
        pub fn set_domains(&mut self, domains: &[&str]) -> bool {
            let mut next: Vec<String> = domains.iter().copied().filter(|d| valid_domain(d)).map(str::to_owned).collect();
            next.sort();
            next.dedup();
            let mut current = self.domains.clone();
            current.sort();
            if next == current {
                return false;
            }
            self.pending |= self.tracker.take_change();
            let refs: Vec<&str> = next.iter().map(String::as_str).collect();
            self.tracker = ChangeTracker::new(&self.root, &refs);
            self.domains = next;
            self.rearm();
            true
        }

        /// İzlenen domain'ler (sıralı).
        pub fn domains(&self) -> Vec<String> {
            let mut d = self.domains.clone();
            d.sort();
            d
        }

        /// İstenen izleme kümesini kurar: var olanı yeniden kullanır, gereksizi kaldırır.
        fn rearm(&mut self) {
            let wanted = plan(&self.root, &self.domains);
            let mut next: HashMap<WatchDescriptor, (PathBuf, Role)> = HashMap::new();
            for (path, role) in wanted {
                let existing = self
                    .watches
                    .iter()
                    .find(|(_, (p, r))| {
                        *p == path && std::mem::discriminant(r) == std::mem::discriminant(&role)
                    })
                    .map(|(wd, _)| wd.clone());
                let wd = match existing {
                    Some(wd) => wd,
                    None => {
                        let top = path.as_path()
                            == top_dir(&self.root).as_deref().unwrap_or(Path::new(""));
                        match self.inotify.watches().add(&path, mask(&role, top)) {
                            Ok(wd) => wd,
                            Err(e) => {
                                // Yol loga yazılmaz (kullanıcı dizini, değişmez 9).
                                tracing::debug!("çeviri dizini izlenemedi: {e}");
                                continue;
                            }
                        }
                    }
                };
                next.insert(wd, (path, role));
            }
            for wd in self.watches.keys() {
                if !next.contains_key(wd) {
                    // Silinmiş dizinin izi zaten düşmüştür (EINVAL); önemsiz.
                    let _ = self.inotify.watches().remove(wd.clone());
                }
            }
            self.watches = next;
            if self.watches.is_empty() && !self.domains.is_empty() {
                tracing::debug!("çeviri dizini ve üst dizinleri yok; izlenmiyor");
            }
        }
    }

    fn mask(role: &Role, top: bool) -> WatchMask {
        let base = WatchMask::DELETE_SELF | WatchMask::MOVE_SELF | WatchMask::ONLYDIR;
        // En üst düzey (veri dizini) ortamdan gelir ve symlink olabilir; altındaki her düzey
        // gerçek dizin olmalı.
        let follow = if top {
            WatchMask::empty()
        } else {
            WatchMask::DONT_FOLLOW
        };
        let entries = match role {
            Role::Domain => {
                WatchMask::CLOSE_WRITE
                    | WatchMask::MOVED_TO
                    | WatchMask::MOVED_FROM
                    | WatchMask::CREATE
                    | WatchMask::DELETE
            }
            Role::Ancestor(_) => {
                WatchMask::CREATE | WatchMask::MOVED_TO | WatchMask::MOVED_FROM | WatchMask::DELETE
            }
        };
        base | follow | entries
    }

    fn is_ftl(name: &OsStr) -> bool {
        name.to_str()
            .is_some_and(|n| n.ends_with(".ftl") && !n.starts_with('.'))
    }

    fn valid_domain(d: &str) -> bool {
        !d.is_empty() && !d.contains(['/', '\\']) && !d.starts_with('.')
    }

    /// Domain dizininin [`MAX_ANCESTORS`] üstü: izlemenin en üst sınırı.
    fn top_dir(root: &Path) -> Option<PathBuf> {
        root.ancestors().nth(MAX_ANCESTORS - 1).map(Path::to_owned)
    }

    /// Her domain için: sınırdan aşağı inerek gerçek dizin olan en derin düzey. Domain dizini
    /// varsa onun kendisi (`Domain`), yoksa o üst dizin ve beklenen alt ad (`Ancestor`).
    fn plan(root: &Path, domains: &[String]) -> Vec<(PathBuf, Role)> {
        let mut out: Vec<(PathBuf, Role)> = Vec::new();
        let Some(top) = top_dir(root) else { return out };
        // En üst düzey ortamdan gelir: symlink izlenir. Altı symlink_metadata ile.
        if !std::fs::metadata(&top).is_ok_and(|m| m.is_dir()) {
            return out;
        }
        for domain in domains {
            let target = root.join(domain);
            let Ok(rel) = target.strip_prefix(&top) else {
                continue;
            };
            let mut base = top.clone();
            let mut missing: Option<OsString> = None;
            for comp in rel.components() {
                let next = base.join(comp);
                if std::fs::symlink_metadata(&next).is_ok_and(|m| m.file_type().is_dir()) {
                    base = next;
                } else {
                    missing = Some(comp.as_os_str().to_owned());
                    break;
                }
            }
            let role = match missing {
                None => Role::Domain,
                Some(name) => Role::Ancestor(vec![name]),
            };
            match out.iter_mut().find(|(p, _)| *p == base) {
                Some((_, Role::Ancestor(names))) => {
                    if let Role::Ancestor(new) = role {
                        for n in new {
                            if !names.contains(&n) {
                                names.push(n);
                            }
                        }
                    }
                }
                Some((_, Role::Domain)) => {}
                None => out.push((base, role)),
            }
        }
        out
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn plan_walks_down_to_deepest_real_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path().join("nuxut/locales");
            let domains = vec!["shell".to_owned(), "settings".to_owned()];
            // Hiçbiri yok: veri dizini, beklenen ad "nuxut".
            let p = plan(&root, &domains);
            assert_eq!(
                p,
                vec![(tmp.path().to_owned(), Role::Ancestor(vec!["nuxut".into()]))]
            );

            std::fs::create_dir_all(root.join("shell")).unwrap();
            let p = plan(&root, &domains);
            assert_eq!(
                p,
                vec![
                    (root.join("shell"), Role::Domain),
                    (root.clone(), Role::Ancestor(vec!["settings".into()]))
                ]
            );
        }

        #[test]
        fn plan_does_not_descend_into_symlink() {
            let tmp = tempfile::tempdir().unwrap();
            let real = tmp.path().join("elsewhere");
            std::fs::create_dir_all(&real).unwrap();
            std::fs::create_dir_all(tmp.path().join("nuxut")).unwrap();
            std::os::unix::fs::symlink(&real, tmp.path().join("nuxut/locales")).unwrap();
            let root = tmp.path().join("nuxut/locales");
            let p = plan(&root, &["shell".to_owned()]);
            assert_eq!(
                p,
                vec![(
                    tmp.path().join("nuxut"),
                    Role::Ancestor(vec!["locales".into()])
                )]
            );
        }

        #[test]
        fn invalid_domains_are_skipped() {
            assert!(valid_domain("shell"));
            assert!(!valid_domain(""));
            assert!(!valid_domain("../x"));
            assert!(!valid_domain(".hidden"));
            assert!(is_ftl(OsStr::new("tr.ftl")));
            assert!(!is_ftl(OsStr::new(".tr.ftl.swp")));
            assert!(!is_ftl(OsStr::new("tr.ftl~")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debounce_coalesces_burst_into_single_fire() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut d = Debounce::default();
        assert_eq!(d.poll(t0), DebouncePoll::Idle);
        assert_eq!(d.event(ms(0)), ms(200));
        assert_eq!(d.event(ms(50)), ms(250));
        assert_eq!(d.event(ms(120)), ms(320));
        assert_eq!(d.poll(ms(200)), DebouncePoll::Wait(ms(320)));
        assert_eq!(d.poll(ms(320)), DebouncePoll::Fire);
        // Tek tetik: sonra boşta.
        assert_eq!(d.poll(ms(400)), DebouncePoll::Idle);
        assert!(!d.is_pending());
    }

    #[test]
    fn debounce_caps_continuous_stream() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut d = Debounce::default();
        let mut deadline = t0;
        for i in 0..20 {
            deadline = d.event(ms(i * 100));
        }
        // Her 100 ms'de olay: son tarih ilk olay + 1 s'de sabitlenir.
        assert_eq!(deadline, ms(1000));
        assert_eq!(d.poll(ms(1000)), DebouncePoll::Fire);
    }

    #[test]
    fn tracker_detects_content_not_mere_touch() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shell");
        let mut t = ChangeTracker::new(tmp.path(), &["shell"]);
        assert!(!t.take_change(), "değişiklik yok");

        std::fs::create_dir_all(&dir).unwrap();
        assert!(!t.take_change(), "boş dizin katalogu değiştirmez");
        std::fs::write(dir.join("tr.ftl"), "a = A\n").unwrap();
        assert!(t.take_change());
        assert!(!t.take_change());

        // Aynı içerik yeniden yazıldı (yeni inode): meta değişir, içerik aynı.
        let tmp_file = dir.join(".tr.ftl.tmp");
        std::fs::write(&tmp_file, "a = A\n").unwrap();
        std::fs::rename(&tmp_file, dir.join("tr.ftl")).unwrap();
        assert!(!t.take_change(), "aynı içerik: yeniden kurma yok");

        std::fs::write(dir.join("tr.ftl"), "a = B\n").unwrap();
        assert!(t.take_change());
        // Dil dışı dosya sayılmaz.
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        assert!(!t.take_change());
        std::fs::remove_file(dir.join("tr.ftl")).unwrap();
        assert!(t.take_change());
    }

    #[cfg(unix)]
    #[test]
    fn tracker_ignores_symlinked_file_content() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shell");
        std::fs::create_dir_all(&dir).unwrap();
        let outside = tmp.path().join("outside.ftl");
        std::fs::write(&outside, "a = A\n").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("tr.ftl")).unwrap();
        let mut t = ChangeTracker::new(tmp.path(), &["shell"]);
        // Bağın hedefi değişse de yükleyici bağı okumaz: katalog değişmez.
        std::fs::write(&outside, "a = B\n").unwrap();
        assert!(!t.take_change());
    }
}
