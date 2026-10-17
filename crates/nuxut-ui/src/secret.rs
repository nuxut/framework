//! Sabit kapasiteli, sıfırlanan metin tamponu ([`SecretBuffer`]). [`TextInput`](crate::TextInput)
//! metnini (her kipte) burada tutar; parola kipinde güvenlik garantileri bunun üzerine kurulur:
//!
//! - **Yeniden tahsis yok:** kapasite oluşturulurken ayrılır ve hiç büyümez; sınırı aşan
//!   değişiklik reddedilir. Böylece büyürken eski bir kopya bellekte kalmaz.
//! - **Silinen bayt sıfırlanır:** silme/kesme, kuyruğu kaydırıp boşalan baytları sıfırlar.
//! - **Düşünce sıfırlanır:** `Drop`'ta tüm tampon `zeroize` ile (derleyici yazmayı silemez).
//! - **Kopya yok:** `Clone`/`Copy` yok; `Debug` yalnızca uzunluğu yazar.
//! - **Kilitli bellek ([`SecretBuffer::locked`], parola kipi):** tampon yığında değil, kendi
//!   anonim `mmap` eşlemesinde durur — sayfa hizalı, başka hiçbir ayırmayla sayfa paylaşmaz
//!   (her kilitli tampon ayrı eşleme). Unix'te `mlock` (takasa yazılmaz), Linux'ta ayrıca
//!   `MADV_DONTDUMP` (çekirdek dökümüne girmez). Eşleme kendine ait olduğundan `mlock`/`madvise`
//!   başka veriyi etkilemez ve düşerken `munlock` başka kilitli bir tamponu çözemez.
//!   Kilit başarısızsa (`RLIMIT_MEMLOCK`, `ENOMEM`…) alan aynen çalışır; uyarı süreç başına
//!   yalnızca bir kez loglanır (yalnızca errno, içerik asla). `unsafe` yok: `memmap2`'nin güvenli
//!   API'si (`map_anon`, `lock`/`unlock`, `advise(Advice::DontDump)`).
//!
//! Sınırlar (kapsam dışı): girdi yöntemi (IME) ve compositor süreçlerinin kendi kopyaları,
//! `KeyEvent::text`'in çağıranda yaşayan kısa ömürlü kopyası (`SecretText`, kilitsiz yığın),
//! kilitlenemeyen tamponun takası. `fork` edilen çocuk eşlemeyi `exec`'e dek kopya-yazımda
//! paylaşır (`MADV_DONTFORK`/`WIPEONFORK` memmap2'de `unsafe`; kapsam dışı).

use std::fmt;
use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};

use memmap2::MmapMut;
use zeroize::Zeroize;

/// Tamponun baytları: düz yığın (normal kip) ya da kendi kilitli eşlemesi (parola kipi).
enum Storage {
    Heap(Box<[u8]>),
    Locked(LockedPages),
}

/// Kendi anonim eşlemesi (sayfa hizalı, sayfaları paylaşılmaz). `locked`: `mlock` başarılı oldu.
struct LockedPages {
    map: MmapMut,
    locked: bool,
}

pub struct SecretBuffer {
    storage: Storage,
    /// Kullanılabilir kapasite (eşlemenin sayfa yuvarlaması dahil değil).
    capacity: usize,
    len: usize,
}

impl fmt::Debug for SecretBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretBuffer({} bayt)", self.len)
    }
}

/// Kilit/eşleme uyarısı süreç başına bir kez.
static WARNED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static WARN_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
thread_local! {
    /// Test kancası: `true` iken `mlock` başarısız sayılır (EPERM gibi).
    static FORCE_LOCK_FAILURE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Yalnızca errno/tür loglanır; tampon içeriği hiçbir zaman.
fn warn_once(what: &str, err: &std::io::Error) {
    if WARNED.swap(true, Ordering::Relaxed) {
        return;
    }
    #[cfg(test)]
    WARN_COUNT.fetch_add(1, Ordering::Relaxed);
    let errno = err.raw_os_error().unwrap_or(0);
    tracing::warn!(
        errno,
        kind = ?err.kind(),
        "parola tamponu: {what} başarısız; alan çalışır ama tampon takasa yazılabilir \
         (bu uyarı süreç başına bir kez)"
    );
}

impl LockedPages {
    fn new(capacity: usize) -> std::io::Result<Self> {
        let map = MmapMut::map_anon(capacity.max(1))?;
        // Çekirdek dökümüne girmesin. `RLIMIT_MEMLOCK`'tan bağımsızdır; başarısızlık yalnızca
        // uyarıdır.
        #[cfg(target_os = "linux")]
        if let Err(e) = map.advise(memmap2::Advice::DontDump) {
            warn_once("madvise(MADV_DONTDUMP)", &e);
        }
        let locked = lock(&map);
        Ok(Self { map, locked })
    }
}

#[cfg(unix)]
fn lock(map: &MmapMut) -> bool {
    #[cfg(test)]
    if FORCE_LOCK_FAILURE.with(|f| f.get()) {
        warn_once("mlock", &std::io::Error::from_raw_os_error(1));
        return false;
    }
    match map.lock() {
        Ok(()) => true,
        Err(e) => {
            warn_once("mlock", &e);
            false
        }
    }
}

/// Unix dışında kilit yok (eşleme yine kendine ait, sıfırlama aynı).
#[cfg(not(unix))]
fn lock(_: &MmapMut) -> bool {
    false
}

impl Drop for LockedPages {
    fn drop(&mut self) {
        // Önce sıfırla (sayfanın tamamı, yuvarlama payı dahil), sonra kilidi çöz; eşleme ardından
        // `MmapMut`'un düşüşünde `munmap` edilir. Eşleme kendine ait: `munlock` başka tampona
        // dokunmaz.
        self.map[..].zeroize();
        #[cfg(unix)]
        if self.locked {
            let _ = self.map.unlock();
        }
    }
}

impl SecretBuffer {
    /// Yığında, kilitsiz tampon (normal metin alanı).
    pub fn with_capacity(capacity: usize) -> Self {
        Self { storage: Storage::Heap(vec![0u8; capacity].into_boxed_slice()), capacity, len: 0 }
    }

    /// Parola tamponu: kendi sayfa hizalı eşlemesinde, mümkünse `mlock` + `MADV_DONTDUMP`.
    /// Eşleme açılamazsa (bellek yok) yığına düşer; her iki başarısızlık da yalnızca bir kez
    /// uyarı loglar, alan çalışmaya devam eder.
    pub fn locked(capacity: usize) -> Self {
        match LockedPages::new(capacity) {
            Ok(p) => Self { storage: Storage::Locked(p), capacity, len: 0 },
            Err(e) => {
                warn_once("mmap", &e);
                Self::with_capacity(capacity)
            }
        }
    }

    /// Tampon kendi eşlemesinde mi ([`locked`](Self::locked) ile ve eşleme açılabildi).
    pub fn is_isolated(&self) -> bool {
        matches!(self.storage, Storage::Locked(_))
    }

    /// Tampon belleğe kilitli mi (`mlock` başarılı; takasa yazılmaz).
    pub fn is_locked(&self) -> bool {
        matches!(&self.storage, Storage::Locked(p) if p.locked)
    }

    fn bytes(&self) -> &[u8] {
        match &self.storage {
            Storage::Heap(b) => b,
            Storage::Locked(p) => &p.map[..self.capacity],
        }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        match &mut self.storage {
            Storage::Heap(b) => b,
            Storage::Locked(p) => &mut p.map[..self.capacity],
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// İçerik (her zaman geçerli UTF-8: yalnızca `&str` dilimleri karakter sınırlarına yazılır).
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes()[..self.len]).unwrap_or_default()
    }

    /// `range`'i `s` ile değiştirir. Kapasite aşılırsa ya da aralık karakter sınırında değilse
    /// hiçbir şey değişmez ve `false` döner.
    pub fn replace_range(&mut self, range: Range<usize>, s: &str) -> bool {
        let (a, b) = (range.start, range.end);
        let cur = self.as_str();
        if a > b || b > self.len || !cur.is_char_boundary(a) || !cur.is_char_boundary(b) {
            return false;
        }
        let new_len = self.len - (b - a) + s.len();
        if new_len > self.capacity {
            return false;
        }
        let len = self.len;
        let bytes = self.bytes_mut();
        // Kuyruğu yeni yerine kaydır, araya yeni metni yaz.
        bytes.copy_within(b..len, a + s.len());
        bytes[a..a + s.len()].copy_from_slice(s.as_bytes());
        // Kısalınca boşalan baytlar sıfırlanır.
        if new_len < len {
            bytes[new_len..len].zeroize();
        }
        self.len = new_len;
        true
    }

    /// Tüm içeriği `s` ile değiştirir (sığmazsa `false`, içerik değişmez).
    pub fn set(&mut self, s: &str) -> bool {
        if s.len() > self.capacity {
            return false;
        }
        let end = self.len;
        self.replace_range(0..end, s)
    }

    /// İçeriği sıfırlar.
    pub fn clear(&mut self) {
        self.wipe();
    }

    fn wipe(&mut self) {
        self.bytes_mut().zeroize();
        self.len = 0;
    }

    /// İçeriğin tek kopyası, düşünce sıfırlanan bir `Vec` içinde (ör. PAM/D-Bus'a parola
    /// vermek için). Tam kapasiteyle ayrılır (`Vec::with_capacity(len)`): ara `String` yok,
    /// büyürken yeniden ayırma (ve geride kalan sıfırlanmamış kopya) yok.
    ///
    /// Sınır: dönen bellek **kilitli değildir** (`mlock`/`MADV_DONTDUMP` yok, sıradan yığın);
    /// takasa ya da çekirdek dökümüne düşebilir. Çağıran onu olabildiğince kısa tutmalı, başka
    /// tipe (`String`, `Vec` klonu) kopyalamamalı ve iş bitince hemen düşürmelidir.
    pub fn to_zeroizing(&self) -> zeroize::Zeroizing<Vec<u8>> {
        let src = &self.bytes()[..self.len];
        let mut out = Vec::with_capacity(src.len());
        out.extend_from_slice(src);
        zeroize::Zeroizing::new(out)
    }

    /// Test kancası: düşmeden önceki temizliği uygular ve ham tamponu döndürür (tamamı sıfır
    /// olmalı). `Drop` aynı `wipe`'ı çağırır.
    #[cfg(test)]
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        self.bytes()
    }

    #[cfg(test)]
    pub(crate) fn wipe_and_inspect(&mut self) -> &[u8] {
        self.wipe();
        self.bytes()
    }

    #[cfg(test)]
    fn addr(&self) -> usize {
        self.bytes().as_ptr() as usize
    }
}

impl Drop for SecretBuffer {
    fn drop(&mut self) {
        self.wipe();
        // Kilitli eşleme kendi düşüşünde sayfanın tamamını sıfırlar, sonra `munlock` + `munmap`.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_in_place_and_zeroes_tail() {
        for mut b in [SecretBuffer::with_capacity(16), SecretBuffer::locked(16)] {
            assert!(b.set("parola123"));
            assert!(b.replace_range(6..9, ""));
            assert_eq!(b.as_str(), "parola");
            // Silinen baytlar sıfır.
            assert!(b.raw_bytes()[6..9].iter().all(|x| *x == 0));
            assert!(b.replace_range(0..0, "çok "));
            assert_eq!(b.as_str(), "çok parola");
            assert!(!b.replace_range(1..2, "x"), "karakter sınırı dışı");
        }
    }

    #[test]
    fn capacity_is_fixed() {
        for mut b in [SecretBuffer::with_capacity(4), SecretBuffer::locked(4)] {
            assert!(b.set("abcd"));
            assert!(!b.replace_range(4..4, "e"));
            assert_eq!(b.as_str(), "abcd");
            assert_eq!(b.capacity(), 4);
            assert_eq!(b.raw_bytes().len(), 4, "sayfa yuvarlaması kapasiteye girmez");
        }
    }

    #[test]
    fn debug_hides_content_and_wipe_zeroes() {
        for mut b in [SecretBuffer::with_capacity(32), SecretBuffer::locked(32)] {
            b.set("gizli-sır");
            let dbg = format!("{b:?}");
            assert!(!dbg.contains("gizli") && dbg.contains("bayt"), "{dbg}");
            assert!(b.wipe_and_inspect().iter().all(|x| *x == 0));
            assert!(b.is_empty());
        }
    }

    #[test]
    fn to_zeroizing_is_exact_single_copy() {
        for mut b in [SecretBuffer::with_capacity(64), SecretBuffer::locked(64)] {
            assert!(b.set("gizli-şifre"));
            let z = b.to_zeroizing();
            assert_eq!(&z[..], "gizli-şifre".as_bytes());
            assert_eq!(z.len(), b.len());
            assert_eq!(z.capacity(), z.len(), "tam kapasite: yeniden ayırma ya da artık yok");
            assert_ne!(z.as_ptr() as usize, b.addr(), "ayrı kopya");
            // Kaynak değişmez.
            assert_eq!(b.as_str(), "gizli-şifre");
            b.clear();
            let e = b.to_zeroizing();
            assert!(e.is_empty());
            assert_eq!(e.capacity(), 0);
        }
    }

    /// Enjekte edilen `mlock` hatası: alan aynen çalışır, kilitli değildir ama yine kendi
    /// eşlemesindedir.
    #[test]
    fn lock_failure_keeps_field_working() {
        FORCE_LOCK_FAILURE.with(|f| f.set(true));
        let mut b = SecretBuffer::locked(1024);
        FORCE_LOCK_FAILURE.with(|f| f.set(false));
        assert!(!b.is_locked());
        assert!(b.is_isolated());
        assert!(b.set("parola"));
        assert!(b.replace_range(0..1, "P"));
        assert_eq!(b.as_str(), "Parola");
        assert!(b.wipe_and_inspect().iter().all(|x| *x == 0));
        assert!(WARN_COUNT.load(Ordering::Relaxed) <= 1, "uyarı en fazla bir kez");
    }

    #[cfg(target_os = "linux")]
    mod linux {
        use super::*;
        use std::process::Command;

        /// `/proc/self/smaps`'te `addr`'ı içeren eşleme: (başlangıç, bitiş, Locked kB,
        /// VmFlags, KernelPageSize kB).
        struct Mapping {
            start: usize,
            end: usize,
            locked_kb: usize,
            flags: Vec<String>,
            page_kb: usize,
        }

        fn mapping_of(addr: usize) -> Option<Mapping> {
            let smaps = std::fs::read_to_string("/proc/self/smaps").ok()?;
            let mut cur: Option<Mapping> = None;
            for line in smaps.lines() {
                let first = line.split_whitespace().next().unwrap_or("");
                if let Some((a, b)) = first.split_once('-')
                    && let (Ok(a), Ok(b)) = (usize::from_str_radix(a, 16), usize::from_str_radix(b, 16))
                {
                    if cur.as_ref().is_some_and(|m| m.start <= addr && addr < m.end) {
                        return cur;
                    }
                    cur = Some(Mapping { start: a, end: b, locked_kb: 0, flags: Vec::new(), page_kb: 0 });
                    continue;
                }
                let Some(m) = cur.as_mut() else { continue };
                let kb = |rest: &str| rest.split_whitespace().next().and_then(|v| v.parse().ok()).unwrap_or(0);
                if let Some(rest) = line.strip_prefix("Locked:") {
                    m.locked_kb = kb(rest);
                } else if let Some(rest) = line.strip_prefix("KernelPageSize:") {
                    m.page_kb = kb(rest);
                } else if let Some(rest) = line.strip_prefix("VmFlags:") {
                    m.flags = rest.split_whitespace().map(str::to_owned).collect();
                }
            }
            cur.filter(|m| m.start <= addr && addr < m.end)
        }

        fn vm_lck_kb() -> usize {
            let status = std::fs::read_to_string("/proc/self/status").unwrap();
            status
                .lines()
                .find_map(|l| l.strip_prefix("VmLck:"))
                .and_then(|r| r.split_whitespace().next()?.parse().ok())
                .unwrap()
        }

        /// Kilitli tampon sayfa hizalı, kendi eşlemesinde (başka tamponla sayfa paylaşmaz) ve
        /// `MADV_DONTDUMP` bayraklı (`dd`). `dd` `RLIMIT_MEMLOCK`'tan bağımsızdır.
        #[test]
        fn locked_buffers_own_page_aligned_mappings() {
            let a = SecretBuffer::locked(1024);
            let b = SecretBuffer::locked(1024);
            let ma = mapping_of(a.addr()).expect("eşleme var");
            let mb = mapping_of(b.addr()).expect("eşleme var");
            let page = ma.page_kb * 1024;
            assert!(page > 0);
            assert_eq!(a.addr() % page, 0, "sayfa hizalı");
            assert_eq!(b.addr() % page, 0, "sayfa hizalı");
            // Çekirdek komşu anonim eşlemeleri tek VMA'da birleştirebilir; birleşmiş olsa da
            // sayfalar ayrık olmalı.
            let pages = |x: &SecretBuffer| x.addr() / page;
            assert_ne!(pages(&a), pages(&b), "iki tampon aynı sayfada olamaz");
            assert!(ma.flags.iter().any(|f| f == "dd"), "MADV_DONTDUMP: {:?}", ma.flags);
            assert!(mb.flags.iter().any(|f| f == "dd"), "MADV_DONTDUMP: {:?}", mb.flags);
            // Yığın tamponu kilitli eşlemenin sayfalarında olamaz.
            let h = SecretBuffer::with_capacity(1024);
            assert_ne!(pages(&h), pages(&a));
            assert_ne!(pages(&h), pages(&b));
        }

        /// Ölçüm `VmLck`/`Locked` süreç geneli olduğundan paralel testlerden etkilenmesin diye
        /// alt süreçte (bu test ikilisi, yalnızca tek test) koşar.
        fn run_child(test: &str, env: &str) -> String {
            let out = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", test, "--nocapture", "--test-threads=1"])
                .env(env, "1")
                .output()
                .expect("alt süreç");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(
                out.status.success(),
                "alt test başarısız:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
            stdout
        }

        #[test]
        fn mlock_accounts_and_releases() {
            let out = run_child("secret::tests::linux::mlock_child", "NUXUT_MLOCK_CHILD");
            if out.contains("ATLANDI") {
                eprintln!("{}", out.lines().find(|l| l.contains("ATLANDI")).unwrap_or(""));
            } else {
                assert!(out.contains("MLOCK-TAMAM"), "{out}");
                eprintln!("{}", out.lines().find(|l| l.contains("MLOCK-TAMAM")).unwrap_or(""));
            }
        }

        #[test]
        fn mlock_child() {
            if std::env::var_os("NUXUT_MLOCK_CHILD").is_none() {
                return;
            }
            let before = vm_lck_kb();
            let b = SecretBuffer::locked(1024);
            if !b.is_locked() {
                let soft = rustix::process::getrlimit(rustix::process::Resource::Memlock).current;
                println!("ATLANDI: mlock reddedildi (RLIMIT_MEMLOCK soft = {soft:?} bayt)");
                return;
            }
            let m = mapping_of(b.addr()).unwrap();
            assert!(m.flags.iter().any(|f| f == "lo"), "VM_LOCKED: {:?}", m.flags);
            assert!(m.flags.iter().any(|f| f == "dd"), "VM_DONTDUMP: {:?}", m.flags);
            assert!(m.locked_kb >= m.page_kb, "Locked: {} kB", m.locked_kb);
            let during = vm_lck_kb();
            assert!(during >= before + m.page_kb, "VmLck {before} → {during} kB");
            drop(b);
            let after = vm_lck_kb();
            assert_eq!(after, before, "Drop sonrası VmLck geri iner");
            println!("MLOCK-TAMAM VmLck {before} → {during} → {after} kB");
        }

        /// Gerçek `RLIMIT_MEMLOCK = 0`: kilit reddedilir, alan çalışır, uyarı bir kez.
        #[test]
        fn mlock_rlimit_zero_logs_once() {
            let out = run_child("secret::tests::linux::rlimit_zero_child", "NUXUT_RLIMIT_CHILD");
            if out.contains("ATLANDI") {
                eprintln!("{}", out.lines().find(|l| l.contains("ATLANDI")).unwrap_or(""));
            } else {
                assert!(out.contains("RLIMIT-TAMAM"), "{out}");
                eprintln!("{}", out.lines().find(|l| l.contains("RLIMIT-TAMAM")).unwrap_or(""));
            }
        }

        #[test]
        fn rlimit_zero_child() {
            if std::env::var_os("NUXUT_RLIMIT_CHILD").is_none() {
                return;
            }
            use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
            let hard = getrlimit(Resource::Memlock).maximum;
            setrlimit(Resource::Memlock, Rlimit { current: Some(0), maximum: hard }).unwrap();
            let mut bufs: Vec<SecretBuffer> = (0..3).map(|_| SecretBuffer::locked(1024)).collect();
            if bufs.iter().any(SecretBuffer::is_locked) {
                // CAP_IPC_LOCK sınırı aşar.
                println!("ATLANDI: RLIMIT_MEMLOCK=0 iken mlock başarılı (CAP_IPC_LOCK?)");
                return;
            }
            for b in &mut bufs {
                assert!(b.is_isolated());
                assert!(b.set("parola"));
                assert!(b.replace_range(6..6, "!"));
                assert_eq!(b.as_str(), "parola!");
            }
            assert_eq!(WARN_COUNT.load(Ordering::Relaxed), 1, "uyarı süreç başına bir kez");
            drop(bufs);
            println!("RLIMIT-TAMAM");
        }
    }
}
