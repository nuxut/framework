//! Sınırlı, salt okunur ZIP okuyucu (hyprcursor `.hlc` şekil arşivleri).
//!
//! Arşiv güvenilmez sayılır. Desteklenen: tek diskli, ZIP64 olmayan, şifresiz, `stored` (0) ya
//! da `deflate` (8) girdiler. Sınırlar: arşiv belleğe en çok [`MAX_ARCHIVE_BYTES`] okunur (çağıran
//! denetler), en çok [`MAX_ENTRIES`] girdi, girdi adı en çok [`MAX_NAME_BYTES`], açılmış girdi en
//! çok [`MAX_ENTRY_BYTES`] (merkez dizinindeki boyut bildirimine güvenilmez: açma bu sınırla
//! durur, sonra bildirilen boyut ve CRC-32 doğrulanır). Tüm konum aritmetiği taşmasız ve dosya
//! sınırına karşı denetlenir; bozuk girdi hata döner (panik yok).

use crate::ImageError;

/// Belleğe okunan en büyük arşiv (gerçek `.hlc`: birkaç KB – birkaç yüz KB).
pub const MAX_ARCHIVE_BYTES: u64 = 16 << 20;
/// Merkez dizinindeki en fazla girdi (gerçek şekil: meta + birkaç boyut × kare).
pub const MAX_ENTRIES: usize = 512;
/// Açılmış bir girdinin en büyük boyutu.
pub const MAX_ENTRY_BYTES: usize = 4 << 20;
/// Girdi adının en büyük bayt uzunluğu.
pub const MAX_NAME_BYTES: usize = 255;

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;
const EOCD_LEN: usize = 22;
const CENTRAL_LEN: usize = 46;
const LOCAL_LEN: usize = 30;
const FLAG_ENCRYPTED: u16 = 1;

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    local: u32,
}

/// Bellekteki bir arşiv: yalnızca merkez dizini ayrıştırılmıştır; girdiler istendikçe açılır.
pub struct Archive {
    data: Vec<u8>,
    entries: Vec<Entry>,
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Archive({} bayt, {} girdi)", self.data.len(), self.entries.len())
    }
}

impl Archive {
    /// Merkez dizinini okur. Ad olarak UTF-8 olmayan ya da `/`, `\`, `..`, NUL içeren girdiler
    /// atlanır (hyprcursor arşivi düzdür); aynı ad ikinci kez gelirse ilki geçerlidir.
    pub fn parse(data: Vec<u8>) -> Result<Self, ImageError> {
        if data.len() as u64 > MAX_ARCHIVE_BYTES {
            return Err(ImageError::FileTooLarge { size: data.len() as u64, limit: MAX_ARCHIVE_BYTES });
        }
        let eocd = find_eocd(&data).ok_or_else(|| invalid("dizin sonu yok"))?;
        let (disk, cd_disk) = (u16le(&data, eocd + 4), u16le(&data, eocd + 6));
        let (on_disk, total) = (u16le(&data, eocd + 8), u16le(&data, eocd + 10));
        let (cd_size, cd_offset) = (u32le(&data, eocd + 12), u32le(&data, eocd + 16));
        if disk != 0 || cd_disk != 0 || on_disk != total {
            return Err(ImageError::Unsupported("çok diskli zip"));
        }
        if total == u16::MAX || cd_size == u32::MAX || cd_offset == u32::MAX {
            return Err(ImageError::Unsupported("zip64"));
        }
        if usize::from(total) > MAX_ENTRIES {
            return Err(invalid("çok fazla girdi"));
        }
        let cd_start = cd_offset as usize;
        let cd_end = cd_start.checked_add(cd_size as usize).ok_or_else(|| invalid("dizin"))?;
        if cd_end > eocd {
            return Err(invalid("dizin dosyayı aşıyor"));
        }
        let mut entries: Vec<Entry> = Vec::with_capacity(usize::from(total));
        let mut at = cd_start;
        for _ in 0..total {
            let fixed_end = at.checked_add(CENTRAL_LEN).ok_or_else(|| invalid("dizin"))?;
            if fixed_end > cd_end || u32le(&data, at) != CENTRAL_SIG {
                return Err(invalid("dizin girdisi"));
            }
            let name_len = usize::from(u16le(&data, at + 28));
            let extra_len = usize::from(u16le(&data, at + 30));
            let comment_len = usize::from(u16le(&data, at + 32));
            let next = fixed_end
                .checked_add(name_len)
                .and_then(|v| v.checked_add(extra_len))
                .and_then(|v| v.checked_add(comment_len))
                .ok_or_else(|| invalid("dizin"))?;
            if next > cd_end {
                return Err(invalid("dizin girdisi dosyayı aşıyor"));
            }
            let raw_name = &data[fixed_end..fixed_end + name_len];
            let entry = Entry {
                name: String::new(),
                flags: u16le(&data, at + 8),
                method: u16le(&data, at + 10),
                crc: u32le(&data, at + 16),
                compressed: u32le(&data, at + 20),
                size: u32le(&data, at + 24),
                local: u32le(&data, at + 42),
            };
            at = next;
            let Some(name) = std::str::from_utf8(raw_name).ok().filter(|n| valid_entry_name(n)) else { continue };
            if entries.iter().any(|e| e.name == name) {
                continue;
            }
            entries.push(Entry { name: name.to_owned(), ..entry });
        }
        Ok(Self { data, entries })
    }

    /// Girdi adları, dizindeki sırayla.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// Girdi var mı.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.iter().any(|e| e.name == name)
    }

    /// Girdiyi açar (`limit` ≤ [`MAX_ENTRY_BYTES`] en büyük açılmış boyut).
    pub fn read(&self, name: &str, limit: usize) -> Result<Vec<u8>, ImageError> {
        let limit = limit.min(MAX_ENTRY_BYTES);
        let e = self.entries.iter().find(|e| e.name == name).ok_or_else(|| ImageError::NotFound(String::new()))?;
        if e.flags & FLAG_ENCRYPTED != 0 {
            return Err(ImageError::Unsupported("şifreli zip"));
        }
        if e.size as usize > limit {
            return Err(ImageError::FileTooLarge { size: u64::from(e.size), limit: limit as u64 });
        }
        let local = e.local as usize;
        let fixed_end = local.checked_add(LOCAL_LEN).ok_or_else(|| invalid("yerel başlık"))?;
        if fixed_end > self.data.len() || u32le(&self.data, local) != LOCAL_SIG {
            return Err(invalid("yerel başlık"));
        }
        let name_len = usize::from(u16le(&self.data, local + 26));
        let extra_len = usize::from(u16le(&self.data, local + 28));
        let start = fixed_end
            .checked_add(name_len)
            .and_then(|v| v.checked_add(extra_len))
            .ok_or_else(|| invalid("yerel başlık"))?;
        let end = start.checked_add(e.compressed as usize).ok_or_else(|| invalid("veri"))?;
        if end > self.data.len() {
            return Err(invalid("veri dosyayı aşıyor"));
        }
        let raw = &self.data[start..end];
        let out = match e.method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, limit).map_err(|_| invalid("deflate"))?,
            _ => return Err(ImageError::Unsupported("zip yöntemi")),
        };
        if out.len() != e.size as usize {
            return Err(invalid("boyut uyuşmuyor"));
        }
        if crc32fast::hash(&out) != e.crc {
            return Err(invalid("CRC"));
        }
        Ok(out)
    }
}

/// Dizin sonu kaydını sondan arar (yorum en çok 65535 bayt).
fn find_eocd(data: &[u8]) -> Option<usize> {
    if data.len() < EOCD_LEN {
        return None;
    }
    let last = data.len() - EOCD_LEN;
    let first = last.saturating_sub(usize::from(u16::MAX));
    (first..=last).rev().find(|&i| {
        // Yorum uzunluğu kaydın dosya sonuna kadar uzandığını doğrular (sahte imzayı eler).
        u32le(data, i) == EOCD_SIG && i + EOCD_LEN + usize::from(u16le(data, i + 20)) == data.len()
    })
}

/// Düz arşiv adı: boş değil, ayırıcı/`..`/NUL/denetim karakteri yok.
fn valid_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && !name.contains("..")
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control())
}

fn u16le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn invalid(what: &str) -> ImageError {
    ImageError::InvalidData(format!("zip: {what}"))
}

/// Test arşivi yazıcısı (`stored` ya da `deflate`); hyprcursor testleri de kullanır.
#[cfg(test)]
pub(crate) fn build_zip(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, body) in files {
        let (method, data) =
            if deflate { (8u16, miniz_oxide::deflate::compress_to_vec(body, 6)) } else { (0u16, body.to_vec()) };
        let crc = crc32fast::hash(body);
        let offset = out.len() as u32;
        out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
        for v in [20u16, 0, method, 0, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, data.len() as u32, body.len() as u32] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&data);
        central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
        for v in [20u16, 20, 0, method, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [crc, data.len() as u32, body.len() as u32] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        for v in [name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&v.to_le_bytes());
        }
        central.extend_from_slice(&0u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&EOCD_SIG.to_le_bytes());
    for v in [0u16, 0, files.len() as u16, files.len() as u16] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stored_and_deflated_entries() {
        for deflate in [false, true] {
            let zip = build_zip(&[("meta.hl", b"hotspot_x = 0.5\n"), ("a.png", &[7u8; 1000])], deflate);
            let a = Archive::parse(zip).unwrap();
            assert_eq!(a.names().collect::<Vec<_>>(), ["meta.hl", "a.png"]);
            assert_eq!(a.read("meta.hl", 64).unwrap(), b"hotspot_x = 0.5\n");
            assert_eq!(a.read("a.png", MAX_ENTRY_BYTES).unwrap(), vec![7u8; 1000]);
            assert!(matches!(a.read("a.png", 999), Err(ImageError::FileTooLarge { .. })));
            assert!(matches!(a.read("yok", 10), Err(ImageError::NotFound(_))));
        }
    }

    #[test]
    fn unsafe_names_are_skipped() {
        let zip = build_zip(&[("../x", b"1"), ("d/x", b"2"), ("ok", b"3"), ("ok", b"4"), ("", b"5")], false);
        let a = Archive::parse(zip).unwrap();
        assert_eq!(a.names().collect::<Vec<_>>(), ["ok"]);
        assert_eq!(a.read("ok", 10).unwrap(), b"3", "ilk girdi geçerli");
    }

    #[test]
    fn hostile_archives_are_errors_not_panics() {
        let good = build_zip(&[("meta.hl", b"define_size = 24, a.png\n"), ("a.png", &[1u8; 300])], true);
        // Her kesik önek ve her tek bayt bozulması: panik yok.
        for n in 0..good.len() {
            if let Ok(a) = Archive::parse(good[..n].to_vec()) {
                let _ = a.read("a.png", MAX_ENTRY_BYTES);
            }
        }
        for i in 0..good.len() {
            for v in [0u8, 0xff, 0x7f] {
                let mut b = good.clone();
                b[i] = v;
                if let Ok(a) = Archive::parse(b) {
                    for n in ["meta.hl", "a.png"] {
                        // Bozuk veri ya CRC/boyutla yakalanır ya da (bozulma veriye değmediyse) doğru döner.
                        if let Ok(d) = a.read(n, MAX_ENTRY_BYTES) {
                            assert!(d == b"define_size = 24, a.png\n" || d == [1u8; 300], "bayt {i}");
                        }
                    }
                }
            }
        }
        // Dekompresyon bombası: 64 MiB sıfır, bildirilen boyut sınırın altında yalan.
        let bomb_body = vec![0u8; 64 << 20];
        let mut bomb = build_zip(&[("a.png", &bomb_body)], true);
        let cd = bomb.len() - EOCD_LEN - (CENTRAL_LEN + 5);
        bomb[cd + 24..cd + 28].copy_from_slice(&100u32.to_le_bytes());
        let a = Archive::parse(bomb).unwrap();
        assert!(a.read("a.png", MAX_ENTRY_BYTES).is_err());
        // Girdi sayısı sınırı, zip64 işaretleri, şifreli girdi, bilinmeyen yöntem.
        let set16 = |b: &mut Vec<u8>, at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let eocd = good.len() - EOCD_LEN;
        let mut many = good.clone();
        set16(&mut many, eocd + 8, 600);
        set16(&mut many, eocd + 10, 600);
        assert!(Archive::parse(many).is_err());
        let mut z64 = good.clone();
        set16(&mut z64, eocd + 8, u16::MAX);
        set16(&mut z64, eocd + 10, u16::MAX);
        assert!(matches!(Archive::parse(z64), Err(ImageError::Unsupported(_))));
        let cd0 = u32le(&good, eocd + 16) as usize;
        let mut enc = good.clone();
        set16(&mut enc, cd0 + 8, FLAG_ENCRYPTED);
        assert!(matches!(Archive::parse(enc).unwrap().read("meta.hl", 100), Err(ImageError::Unsupported(_))));
        let mut method = good.clone();
        set16(&mut method, cd0 + 10, 12);
        assert!(Archive::parse(method).unwrap().read("meta.hl", 100).is_err());
        // Çok büyük arşiv belleğe alınmaz.
        assert!(matches!(
            Archive::parse(vec![0u8; MAX_ARCHIVE_BYTES as usize + 1]),
            Err(ImageError::FileTooLarge { .. })
        ));
        assert!(Archive::parse(Vec::new()).is_err());
    }
}
