//! XCursor imleç temaları: tema çözümü ve sınırlı Xcursor ayrıştırıcısı (Ayarlar'daki imleç
//! teması önizlemesi; imleci compositor çizer, burada yalnızca görüntüsü okunur).
//!
//! **Tema çözümü** (libXcursor ile aynı sıra): `<arama dizini>/<tema>/cursors/<ad>`; tema
//! zincirinin her temasında önce o temanın kendi dosyaları (tüm eşanlamlı adlar), bulunamazsa
//! `index.theme` `Inherits` zinciri derinlik öncelikli. Zincir en çok [`MAX_INHERIT_DEPTH`]
//! derin ve [`MAX_CHAIN_THEMES`] tema; döngüler ziyaret kümesiyle kesilir. Arama dizinleri
//! `XCURSOR_PATH` (varsa) ya da ikon arama dizinleri + `/usr/share/pixmaps`.
//!
//! **Ayrıştırıcı** (Xcursor dosya biçimi, küçük uçlu): dosya sistem dizininden bile gelse
//! güvenilmez sayılır. Dosya boyutu [`MAX_CURSOR_FILE_BYTES`], TOC girdisi [`MAX_TOC_ENTRIES`],
//! görüntü kenarı [`MAX_CURSOR_SIDE`], kare sayısı [`MAX_CURSOR_FRAMES`], çözülen toplam piksel
//! [`MAX_CURSOR_PIXELS`] ile sınırlı; tüm konum/uzunluk aritmetiği taşmasız (`checked_*`),
//! her parça dosya sınırına karşı denetlenir, kısa/bozuk başlık hata döner (panik yok). Dosyanın
//! tamamı okunmaz: başlık + TOC, sonra yalnızca seçilen boyutun kareleri (`seek`). Dosya
//! `O_NONBLOCK` açılır ve normal dosya olduğu fd'den doğrulanır (FIFO/aygıt engellemez);
//! symlink döngüsü `ELOOP` ile "bulunamadı" sayılır.
//!
//! **Hyprcursor** ([`hyprcursor`]): `manifest.hl` + `.hlc` ZIP arşivleri. Bir rol için zincirin
//! hiçbir temasında XCursor dosyası yoksa zincirdeki temaların hyprcursor şekillerine bakılır
//! (iki biçimi de olan temada XCursor tercih edilir: raster, daha ucuz, güven sorunu yok).
//! SVG şekillerin güven kuralı modül belgesindedir (ADR-0007).

pub mod hyprcursor;
pub mod zip;

use std::collections::HashSet;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use crate::{Image, ImageError, MAX_FRAME_DELAY};

/// Okunmayı kabul edilen en büyük Xcursor dosyası (Bibata `watch`: ~9 MB, tüm boyutlar × kareler).
pub const MAX_CURSOR_FILE_BYTES: u64 = 64 << 20;
/// Başlıktaki TOC girdisi sınırı (gerçek temalarda ≤ ~800).
pub const MAX_TOC_ENTRIES: u32 = 4096;
/// Bir imleç görüntüsünün en büyük kenarı (piksel).
pub const MAX_CURSOR_SIDE: u32 = 512;
/// Animasyonlu imleçte çözülen en fazla kare.
pub const MAX_CURSOR_FRAMES: usize = 128;
/// Bir imleç için çözülen tüm karelerin toplam pikseli.
pub const MAX_CURSOR_PIXELS: u64 = 8_000_000;
/// `Inherits` zincirinin en büyük derinliği.
pub const MAX_INHERIT_DEPTH: usize = 8;
/// Zincirde ziyaret edilen en fazla tema.
pub const MAX_CHAIN_THEMES: usize = 32;

const MAGIC: [u8; 4] = *b"Xcur";
const FILE_HEADER_LEN: u32 = 16;
const TOC_ENTRY_LEN: u64 = 12;
const IMAGE_TYPE: u32 = 0xfffd_0002;
const IMAGE_HEADER_LEN: u32 = 36;
/// Gecikmesi 0 yazılmış kare (bazı temalar) bu kadar sayılır.
pub(crate) const ZERO_DELAY: Duration = Duration::from_millis(50);

/// Önizlenebilir imleç rolleri. Her rolün dosya adı eşanlamlıları [`names`](Self::names)
/// tablosundadır (CSS/cursor-shape adı → X11 core adı → Qt/KDE adı → eski karma adlar).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorRole {
    Default,
    Pointer,
    Text,
    Wait,
    Progress,
    Help,
    Crosshair,
    Move,
    Grab,
    Grabbing,
    NotAllowed,
    /// Yatay yeniden boyutlandırma (↔).
    ResizeEw,
    /// Dikey yeniden boyutlandırma (↕).
    ResizeNs,
    /// Sol üst – sağ alt köşegeni (⤡).
    ResizeNwse,
    /// Sağ üst – sol alt köşegeni (⤢).
    ResizeNesw,
}

impl CursorRole {
    /// Tüm roller.
    pub const ALL: [Self; 15] = [
        Self::Default,
        Self::Pointer,
        Self::Text,
        Self::Wait,
        Self::Progress,
        Self::Help,
        Self::Crosshair,
        Self::Move,
        Self::Grab,
        Self::Grabbing,
        Self::NotAllowed,
        Self::ResizeEw,
        Self::ResizeNs,
        Self::ResizeNwse,
        Self::ResizeNesw,
    ];

    /// Tema seçicisi için varsayılan önizleme seti (en tanıdık dokuz imleç).
    pub const PREVIEW: [Self; 9] = [
        Self::Default,
        Self::Pointer,
        Self::Text,
        Self::Wait,
        Self::Crosshair,
        Self::Move,
        Self::NotAllowed,
        Self::ResizeEw,
        Self::ResizeNwse,
    ];

    /// Dosya adı eşanlamlıları, tercih sırasıyla.
    pub fn names(self) -> &'static [&'static str] {
        match self {
            Self::Default => &["default", "left_ptr", "arrow", "top_left_arrow"],
            Self::Pointer => &[
                "pointer",
                "hand2",
                "pointing_hand",
                "hand1",
                "hand",
                "e29285e634086352946a0e7090d73106",
                "9d800788f1b08800ae810202380a0822",
            ],
            Self::Text => &["text", "xterm", "ibeam"],
            Self::Wait => &["wait", "watch", "0426c94ea35c87780ff01dc239897213"],
            Self::Progress => &[
                "progress",
                "left_ptr_watch",
                "half-busy",
                "3ecb610c1bf2410f44200f48c40d3599",
                "08e8e1c95fe2fc01f976f1e063a24ccd",
                "00000000000000020006000e7e9ffc3f",
            ],
            Self::Help => &[
                "help",
                "question_arrow",
                "whats_this",
                "left_ptr_help",
                "5c6cd98b3f3ebcb1f9c7f1c204630408",
                "d9ce0ab605698f320427677b458ad60b",
            ],
            Self::Crosshair => &["crosshair", "cross", "tcross", "cross_reverse", "diamond_cross"],
            Self::Move => &[
                "move",
                "fleur",
                "size_all",
                "all-scroll",
                "4498f0e0c1937ffe01fd06f973665830",
                "9081237383d90e509aa00f00170e968f",
            ],
            Self::Grab => &["grab", "openhand", "fleur", "5aca4d189052212118709018842178c0"],
            Self::Grabbing => &["grabbing", "closedhand", "dnd-none", "208530c400c041818281048008011002"],
            Self::NotAllowed => &[
                "not-allowed",
                "crossed_circle",
                "forbidden",
                "no-drop",
                "circle",
                "03b6e0fcb3499374a867c041f52298f0",
            ],
            Self::ResizeEw => &[
                "ew-resize",
                "sb_h_double_arrow",
                "h_double_arrow",
                "size_hor",
                "col-resize",
                "split_h",
                "028006030e0e7ebffc7f7070c0600140",
                "14fef782d02440884392942c11205230",
            ],
            Self::ResizeNs => &[
                "ns-resize",
                "sb_v_double_arrow",
                "v_double_arrow",
                "size_ver",
                "row-resize",
                "split_v",
                "00008160000006810000408080010102",
                "2870a09082c103050810ffdffffe0204",
            ],
            Self::ResizeNwse => &["nwse-resize", "bd_double_arrow", "size_fdiag", "c7088f0f3e6c8088236ef8e1e3e70000"],
            Self::ResizeNesw => &["nesw-resize", "fd_double_arrow", "size_bdiag", "fcf1c3c7cd4491d801f1e1c78f100000"],
        }
    }
}

impl From<nuxut_core::input::CursorShape> for CursorRole {
    fn from(shape: nuxut_core::input::CursorShape) -> Self {
        use nuxut_core::input::CursorShape as S;
        match shape {
            S::Default => Self::Default,
            S::Pointer => Self::Pointer,
            S::Text => Self::Text,
            S::Grab => Self::Grab,
            S::Grabbing => Self::Grabbing,
            S::NotAllowed => Self::NotAllowed,
            S::Move => Self::Move,
            S::Crosshair => Self::Crosshair,
            S::Progress => Self::Progress,
            S::Wait => Self::Wait,
            S::Help => Self::Help,
            S::ColResize | S::ResizeE | S::ResizeW => Self::ResizeEw,
            S::RowResize | S::ResizeN | S::ResizeS => Self::ResizeNs,
            S::ResizeNw | S::ResizeSe => Self::ResizeNwse,
            S::ResizeNe | S::ResizeSw => Self::ResizeNesw,
        }
    }
}

/// Bir imleç görüntüsü (Xcursor ya da hyprcursor; animasyonun bir karesi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorImage {
    /// Premultiplied RGBA8, dosyadaki boyutunda (küçültülmemiş).
    pub image: Image,
    /// Dosyadaki nominal boyut (seçilen boyut; görüntü kenarından farklı olabilir).
    pub nominal: u32,
    /// Sıcak nokta (görüntü içinde, piksel).
    pub hotspot: (u32, u32),
    /// Karenin süresi (tek karede anlamsız).
    pub delay: Duration,
}

/// `size` nominal boyutuna en yakın imleç görüntüsünü (ve `animate` ise aynı boyutun tüm
/// karelerini) okur. `len` akışın toplam uzunluğudur; her konum ona karşı denetlenir.
///
/// En yakın boyut libXcursor gibi seçilir: |nominal − size| en küçük, eşitlikte TOC'ta önce
/// gelen. Animasyonda ilk kareden sonra bozuk bir kare gelirse animasyon orada kesilir (o ana
/// kadarki kareler döner); ilk kare bozuksa hata.
pub fn parse_xcursor<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    size: u32,
    animate: bool,
) -> Result<Vec<CursorImage>, ImageError> {
    if len > MAX_CURSOR_FILE_BYTES {
        return Err(ImageError::FileTooLarge { size: len, limit: MAX_CURSOR_FILE_BYTES });
    }
    if len < u64::from(FILE_HEADER_LEN) {
        return Err(invalid("kısa başlık"));
    }
    reader.seek(SeekFrom::Start(0)).map_err(io)?;
    let head: [u8; 16] = read_array(reader)?;
    if head[..4] != MAGIC {
        return Err(ImageError::UnknownFormat);
    }
    let header_len = le(&head, 4);
    let ntoc = le(&head, 12);
    if header_len < FILE_HEADER_LEN || u64::from(header_len) > len {
        return Err(invalid("başlık uzunluğu"));
    }
    if ntoc == 0 {
        return Err(ImageError::Empty);
    }
    if ntoc > MAX_TOC_ENTRIES {
        return Err(invalid("TOC çok büyük"));
    }
    let toc_bytes = u64::from(ntoc).checked_mul(TOC_ENTRY_LEN).ok_or_else(|| invalid("TOC"))?;
    let toc_end = u64::from(header_len).checked_add(toc_bytes).ok_or_else(|| invalid("TOC"))?;
    if toc_end > len {
        return Err(invalid("TOC dosyayı aşıyor"));
    }
    reader.seek(SeekFrom::Start(u64::from(header_len))).map_err(io)?;
    // ntoc ≤ MAX_TOC_ENTRIES: en çok 48 KiB.
    let mut toc = vec![0u8; toc_bytes as usize];
    reader.read_exact(&mut toc).map_err(io)?;
    let images: Vec<(u32, u32)> = toc
        .chunks_exact(TOC_ENTRY_LEN as usize)
        .filter(|e| le(e, 0) == IMAGE_TYPE)
        .map(|e| (le(e, 4), le(e, 8)))
        .collect();
    let want = size.max(1);
    let best = images
        .iter()
        .map(|&(nominal, _)| nominal)
        .reduce(|best, n| if n.abs_diff(want) < best.abs_diff(want) { n } else { best })
        .ok_or(ImageError::Empty)?;
    let limit = if animate { MAX_CURSOR_FRAMES } else { 1 };
    let mut out: Vec<CursorImage> = Vec::new();
    let mut pixels_used: u64 = 0;
    for &(_, position) in images.iter().filter(|(n, _)| *n == best).take(limit) {
        match read_image(reader, len, position, best, &mut pixels_used) {
            Ok(img) => {
                // Animasyon kareleri aynı boyutta olmalı (oynatıcı ilk karenin boyutunu kullanır).
                if out.first().is_some_and(|f| f.image.size() != img.image.size()) {
                    break;
                }
                out.push(img);
            }
            Err(e) if out.is_empty() => return Err(e),
            Err(e) => {
                tracing::debug!("imleç animasyonu {} karede kesildi: {e}", out.len());
                break;
            }
        }
    }
    Ok(out)
}

/// Bir görüntü parçasını okur ve doğrular.
fn read_image<R: Read + Seek>(
    reader: &mut R,
    len: u64,
    position: u32,
    nominal: u32,
    pixels_used: &mut u64,
) -> Result<CursorImage, ImageError> {
    let start = u64::from(position);
    let header_end = start.checked_add(u64::from(IMAGE_HEADER_LEN)).ok_or_else(|| invalid("konum"))?;
    if header_end > len {
        return Err(invalid("görüntü başlığı dosyayı aşıyor"));
    }
    reader.seek(SeekFrom::Start(start)).map_err(io)?;
    let h: [u8; 36] = read_array(reader)?;
    let (chunk_len, kind, subtype) = (le(&h, 0), le(&h, 4), le(&h, 8));
    let (width, height, xhot, yhot, delay_ms) = (le(&h, 16), le(&h, 20), le(&h, 24), le(&h, 28), le(&h, 32));
    if chunk_len < IMAGE_HEADER_LEN || kind != IMAGE_TYPE || subtype != nominal {
        return Err(invalid("görüntü başlığı"));
    }
    if width == 0 || height == 0 {
        return Err(ImageError::Empty);
    }
    if width > MAX_CURSOR_SIDE || height > MAX_CURSOR_SIDE {
        return Err(ImageError::TooLarge { width, height });
    }
    if xhot > width || yhot > height {
        return Err(invalid("sıcak nokta görüntü dışında"));
    }
    let count = u64::from(width) * u64::from(height);
    let total = pixels_used.checked_add(count).ok_or_else(|| invalid("piksel"))?;
    if total > MAX_CURSOR_PIXELS {
        return Err(ImageError::TooLarge { width, height });
    }
    let data_start = start.checked_add(u64::from(chunk_len)).ok_or_else(|| invalid("konum"))?;
    let data_end = data_start.checked_add(count * 4).ok_or_else(|| invalid("konum"))?;
    if data_end > len {
        return Err(invalid("pikseller dosyayı aşıyor"));
    }
    reader.seek(SeekFrom::Start(data_start)).map_err(io)?;
    // width, height ≤ MAX_CURSOR_SIDE: en çok 1 MiB.
    let mut px = vec![0u8; (count * 4) as usize];
    reader.read_exact(&mut px).map_err(io)?;
    // Küçük uçlu ARGB32 (premultiplied) → premultiplied RGBA8; renk kanalı alfayı aşamaz.
    for p in px.chunks_exact_mut(4) {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        p.copy_from_slice(&[r.min(a), g.min(a), b.min(a), a]);
    }
    *pixels_used = total;
    let delay = match delay_ms {
        0 => ZERO_DELAY,
        ms => Duration::from_millis(u64::from(ms)).min(MAX_FRAME_DELAY),
    };
    Ok(CursorImage { image: Image::from_premultiplied(width, height, px)?, nominal, hotspot: (xhot, yhot), delay })
}

/// Dosyadan okur (`O_NONBLOCK`, normal dosya doğrulaması, boyut sınırı).
pub fn read_xcursor_file(path: &Path, size: u32, animate: bool) -> Result<Vec<CursorImage>, ImageError> {
    let file = crate::decode::open_regular(path)?;
    let len = file.metadata().map_err(|source| ImageError::Io { path: path.to_owned(), source })?.len();
    parse_xcursor(&mut BufReader::new(file), len, size, animate)
}

/// Xcursor dosyasındaki nominal boyutlar (artan, tekil); yalnızca başlık + TOC okunur.
pub fn xcursor_sizes<R: Read + Seek>(reader: &mut R, len: u64) -> Result<Vec<u32>, ImageError> {
    if len > MAX_CURSOR_FILE_BYTES {
        return Err(ImageError::FileTooLarge { size: len, limit: MAX_CURSOR_FILE_BYTES });
    }
    if len < u64::from(FILE_HEADER_LEN) {
        return Err(invalid("kısa başlık"));
    }
    reader.seek(SeekFrom::Start(0)).map_err(io)?;
    let head: [u8; 16] = read_array(reader)?;
    if head[..4] != MAGIC {
        return Err(ImageError::UnknownFormat);
    }
    let (header_len, ntoc) = (le(&head, 4), le(&head, 12));
    if header_len < FILE_HEADER_LEN || ntoc > MAX_TOC_ENTRIES {
        return Err(invalid("başlık"));
    }
    let toc_bytes = u64::from(ntoc) * TOC_ENTRY_LEN;
    if u64::from(header_len) + toc_bytes > len {
        return Err(invalid("TOC dosyayı aşıyor"));
    }
    reader.seek(SeekFrom::Start(u64::from(header_len))).map_err(io)?;
    let mut toc = vec![0u8; toc_bytes as usize];
    reader.read_exact(&mut toc).map_err(io)?;
    let mut sizes: Vec<u32> = toc
        .chunks_exact(TOC_ENTRY_LEN as usize)
        .filter(|e| le(e, 0) == IMAGE_TYPE && (1..=MAX_CURSOR_SIDE).contains(&le(e, 4)))
        .map(|e| le(e, 4))
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    Ok(sizes)
}

/// Bir temanın sunduğu imleç boyutları (Ayarlar boyut seçicisi).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CursorSizes {
    /// Hazır çizilmiş nominal boyutlar (fiziksel piksel), artan.
    pub fixed: Vec<u32>,
    /// Ölçeklenebilir (hyprcursor SVG) şekil var: her boyut keskin çizilir.
    pub scalable: bool,
}

/// Bir XCursor teması. `Send + Sync`; zincir ilk kullanımda bir kez çözülür.
pub struct CursorTheme {
    name: String,
    base_dirs: Vec<PathBuf>,
    chain: OnceLock<Vec<String>>,
    /// Zincirdeki hyprcursor temaları (zincir sırasıyla); ilk gerektiğinde açılır.
    hypr: OnceLock<Vec<hyprcursor::HyprTheme>>,
}

/// Arama dizinleri ev dizinini (kullanıcı adını) taşır: yalnızca tema adı ve sayı yazılır.
impl std::fmt::Debug for CursorTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CursorTheme").field("name", &self.name).field("base_dirs", &self.base_dirs.len()).finish()
    }
}

impl CursorTheme {
    /// Standart arama dizinleriyle ([`cursor_base_dirs`]).
    pub fn new(name: impl Into<String>) -> Self {
        Self::with_dirs(name, cursor_base_dirs())
    }

    /// Özel arama dizinleri (öncelik sırasıyla; testler için).
    pub fn with_dirs(name: impl Into<String>, base_dirs: Vec<PathBuf>) -> Self {
        Self { name: name.into(), base_dirs, chain: OnceLock::new(), hypr: OnceLock::new() }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tema ve `Inherits` zinciri, arama sırasıyla (derinlik öncelikli, sınırlı, döngüsüz).
    /// Geçersiz tema adı (yol, `..`, gizli) boş zincir verir.
    pub fn chain(&self) -> &[String] {
        self.chain.get_or_init(|| {
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            self.walk(&self.name, 0, &mut seen, &mut out);
            out
        })
    }

    fn walk(&self, theme: &str, depth: usize, seen: &mut HashSet<String>, out: &mut Vec<String>) {
        if depth > MAX_INHERIT_DEPTH || out.len() >= MAX_CHAIN_THEMES || !crate::icon::valid_name(theme) {
            return;
        }
        if !seen.insert(theme.to_owned()) {
            return;
        }
        out.push(theme.to_owned());
        // libXcursor gibi: `index.theme`'i olan ilk arama dizini zinciri belirler.
        let inherits = self.base_dirs.iter().find_map(|b| crate::icon::theme_inherits(&b.join(theme))).unwrap_or_default();
        for parent in inherits {
            self.walk(&parent, depth + 1, seen, out);
        }
    }

    /// Zincirde XCursor dosyası (`cursors/` dizini) olan bir tema var mı.
    pub fn has_xcursor(&self) -> bool {
        self.chain().iter().any(|t| self.base_dirs.iter().any(|b| b.join(t).join("cursors").is_dir()))
    }

    /// Zincirde hyprcursor teması (geçerli manifest + şekil dizini) var mı.
    pub fn has_hyprcursor(&self) -> bool {
        !self.hypr().is_empty()
    }

    /// Zincirin hyprcursor temaları: her zincir teması için manifesti olan ilk arama dizini.
    fn hypr(&self) -> &[hyprcursor::HyprTheme] {
        self.hypr.get_or_init(|| {
            self.chain()
                .iter()
                .filter_map(|t| self.base_dirs.iter().find_map(|b| hyprcursor::HyprTheme::open(&b.join(t), &self.base_dirs)))
                .collect()
        })
    }

    /// Temanın boyutları, `Default` rolünün dosyasından (Xcursor TOC'u ya da hyprcursor metası;
    /// görseller çözülmez). Rol bulunamazsa boş.
    pub fn sizes(&self) -> CursorSizes {
        if let Some(path) = self.find(CursorRole::Default) {
            let fixed = crate::decode::open_regular(&path)
                .and_then(|f| {
                    let len = f.metadata().map_err(io)?.len();
                    xcursor_sizes(&mut BufReader::new(f), len)
                })
                .unwrap_or_default();
            return CursorSizes { fixed, scalable: false };
        }
        self.hypr()
            .iter()
            .find_map(|h| h.sizes(CursorRole::Default.names()))
            .map(|(fixed, scalable)| CursorSizes { fixed, scalable })
            .unwrap_or_default()
    }

    /// Rolün dosyası: zincirdeki her temada tüm eşanlamlılar denenir (temanın kendi `left_ptr`'ı,
    /// üst temanın `default`'undan önce gelir).
    pub fn find(&self, role: CursorRole) -> Option<PathBuf> {
        self.find_names(role.names())
    }

    /// İlk bulunan ad (tema zinciri dış döngü, adlar iç döngü). Geçersiz adlar atlanır.
    pub fn find_names(&self, names: &[&str]) -> Option<PathBuf> {
        let names: Vec<&str> = names.iter().copied().filter(|n| crate::icon::valid_name(n)).collect();
        for theme in self.chain() {
            let dirs: Vec<PathBuf> =
                self.base_dirs.iter().map(|b| b.join(theme).join("cursors")).filter(|d| d.is_dir()).collect();
            for name in &names {
                // `metadata` bağlantıyı izler: kırık bağlantı ve döngü (`ELOOP`) yok sayılır.
                if let Some(p) = dirs.iter().map(|d| d.join(name)).find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file())) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// Rolün görüntüsü: `size` (fiziksel piksel nominal boyut) için en yakın boyut; `animate`
    /// ise aynı boyutun tüm kareleri (en çok [`MAX_CURSOR_FRAMES`]), değilse yalnızca ilk kare.
    ///
    /// Önce zincirin XCursor dosyaları; hiçbirinde yoksa zincirin hyprcursor şekilleri (ilk
    /// başarılı; hepsi hata verirse ilk hata, hiç şekil yoksa `NotFound`).
    ///
    /// Kullanıcı dizinindeki yalnızca-SVG hyprcursor şekli burada `Unsupported("SVG")` döner;
    /// yalıtılmış çözücülü [`crate::Loader`] (`LoadRequest::cursor`) onu işçide çözer.
    pub fn load(&self, role: CursorRole, size: u32, animate: bool) -> Result<Vec<CursorImage>, ImageError> {
        self.load_with(role, size, animate, None)
    }

    /// [`load`](Self::load); güvenilmeyen SVG şekil `isolated` verilmişse onda çözülür.
    pub(crate) fn load_with(
        &self,
        role: CursorRole,
        size: u32,
        animate: bool,
        isolated: Option<hyprcursor::Isolated<'_>>,
    ) -> Result<Vec<CursorImage>, ImageError> {
        if let Some(path) = self.find(role) {
            return read_xcursor_file(&path, size, animate);
        }
        let mut first_err = None;
        for h in self.hypr() {
            match h.load(role.names(), size, animate, isolated) {
                Ok(frames) => return Ok(frames),
                Err(ImageError::NotFound(_)) => {}
                Err(e) => {
                    first_err.get_or_insert(e);
                }
            }
        }
        Err(first_err.unwrap_or_else(|| ImageError::NotFound(String::new())))
    }
}

/// İmleç arama dizinleri: `XCURSOR_PATH` (`:` ayrılmış, `~/` açılır) varsa o; yoksa ikon arama
/// dizinleri ([`crate::icon::default_base_dirs`]) + `/usr/share/pixmaps`.
pub fn cursor_base_dirs() -> Vec<PathBuf> {
    if let Some(v) = std::env::var_os("XCURSOR_PATH").filter(|v| !v.is_empty()) {
        let home = dirs::home_dir();
        return v
            .to_string_lossy()
            .split(':')
            .filter(|s| !s.is_empty())
            .filter_map(|s| match s.strip_prefix("~/") {
                Some(rest) => home.as_ref().map(|h| h.join(rest)),
                None => Some(PathBuf::from(s)),
            })
            .collect();
    }
    let mut dirs = crate::icon::default_base_dirs();
    dirs.push(PathBuf::from("/usr/share/pixmaps"));
    dirs
}

fn le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn read_array<const N: usize, R: Read>(r: &mut R) -> Result<[u8; N], ImageError> {
    let mut b = [0u8; N];
    r.read_exact(&mut b).map_err(io)?;
    Ok(b)
}

fn io(source: std::io::Error) -> ImageError {
    ImageError::Io { path: PathBuf::new(), source }
}

fn invalid(what: &str) -> ImageError {
    ImageError::InvalidData(format!("xcursor: {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Test dosyası kurucusu: (nominal, w, h, xhot, yhot, gecikme, piksel) görüntüleri.
    struct Img {
        nominal: u32,
        w: u32,
        h: u32,
        hot: (u32, u32),
        delay: u32,
        argb: u32,
    }

    fn img(nominal: u32, side: u32, argb: u32) -> Img {
        Img { nominal, w: side, h: side, hot: (1, 1), delay: 30, argb }
    }

    fn build(images: &[Img]) -> Vec<u8> {
        let mut out = Vec::new();
        let ntoc = images.len() as u32 + 1; // + bir yorum girdisi (atlanmalı)
        out.extend_from_slice(&MAGIC);
        for v in [16u32, 0x1_0000, ntoc] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        let mut pos = 16 + ntoc * 12;
        let mut chunks = Vec::new();
        // Yorum girdisi: tür 0xfffe0001, konum önemsiz.
        for v in [0xfffe_0001u32, 1, 0] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for i in images {
            for v in [IMAGE_TYPE, i.nominal, pos] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            let mut c = Vec::new();
            for v in [36u32, IMAGE_TYPE, i.nominal, 1, i.w, i.h, i.hot.0, i.hot.1, i.delay] {
                c.extend_from_slice(&v.to_le_bytes());
            }
            for _ in 0..(i.w * i.h) {
                c.extend_from_slice(&i.argb.to_le_bytes());
            }
            pos += c.len() as u32;
            chunks.push(c);
        }
        for c in chunks {
            out.extend_from_slice(&c);
        }
        out
    }

    fn parse(bytes: &[u8], size: u32, animate: bool) -> Result<Vec<CursorImage>, ImageError> {
        parse_xcursor(&mut Cursor::new(bytes), bytes.len() as u64, size, animate)
    }

    #[test]
    fn picks_nearest_size_and_converts_pixels() {
        // Yarı saydam kırmızı (premultiplied): A=0x80, R=0x80.
        let bytes = build(&[img(24, 4, 0x8080_0000), img(32, 6, 0xff00_ff00), img(48, 8, 0xff00_00ff)]);
        let got = parse(&bytes, 30, false).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].nominal, got[0].image.size()), (32, (6, 6)));
        assert_eq!(&got[0].image.pixels()[..4], &[0, 255, 0, 255]);
        let small = parse(&bytes, 1, false).unwrap();
        assert_eq!(small[0].nominal, 24);
        assert_eq!(&small[0].image.pixels()[..4], &[0x80, 0, 0, 0x80]);
        // Eşit uzaklık: TOC'ta önce gelen (libXcursor).
        assert_eq!(parse(&bytes, 28, false).unwrap()[0].nominal, 24);
        assert_eq!(parse(&bytes, 1000, false).unwrap()[0].nominal, 48);
    }

    #[test]
    fn animation_frames_of_chosen_size() {
        let mut frames: Vec<Img> = (0..5).map(|i| Img { delay: i * 10, ..img(24, 3, 0xff10_2030) }).collect();
        frames.push(img(48, 6, 0xff00_0000));
        let bytes = build(&frames);
        let got = parse(&bytes, 24, true).unwrap();
        assert_eq!(got.len(), 5);
        assert_eq!(got[0].delay, ZERO_DELAY, "0 ms → varsayılan");
        assert_eq!(got[3].delay, Duration::from_millis(30));
        assert_eq!(parse(&bytes, 24, false).unwrap().len(), 1);
    }

    #[test]
    fn channels_never_exceed_alpha() {
        let bytes = build(&[img(24, 2, 0x10ff_ffff)]);
        assert_eq!(&parse(&bytes, 24, false).unwrap()[0].image.pixels()[..4], &[0x10, 0x10, 0x10, 0x10]);
    }

    #[test]
    fn hostile_files_are_errors_not_panics() {
        let good = build(&[img(24, 4, 0xff00_0000), img(24, 4, 0xff00_0000)]);
        // Her kesik önek: panik yok; ilk görüntü bütün değilse hata, bütünse yalnızca o kare.
        let first_end = 16 + 3 * 12 + 36 + 4 * 4 * 4;
        for n in 0..good.len() {
            match parse(&good[..n], 24, true) {
                Ok(frames) => assert!(n >= first_end && frames.len() == 1, "kesik {n}"),
                Err(_) => assert!(n < first_end, "kesik {n}"),
            }
        }
        // Yanlış sihirli sayı, sıfır TOC, dev TOC, başlık uzunluğu dosyayı aşıyor.
        let mut bad = good.clone();
        bad[0] = b'Y';
        assert!(matches!(parse(&bad, 24, false), Err(ImageError::UnknownFormat)));
        let set = |b: &mut Vec<u8>, at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let mut zero = good.clone();
        set(&mut zero, 12, 0);
        assert!(matches!(parse(&zero, 24, false), Err(ImageError::Empty)));
        for ntoc in [MAX_TOC_ENTRIES + 1, u32::MAX] {
            let mut huge = good.clone();
            set(&mut huge, 12, ntoc);
            assert!(parse(&huge, 24, false).is_err());
        }
        let mut fits = good.clone();
        set(&mut fits, 12, 2000); // sınırda ama dosyayı aşan TOC
        assert!(parse(&fits, 24, false).is_err());
        let mut hl = good.clone();
        set(&mut hl, 4, u32::MAX);
        assert!(parse(&hl, 24, false).is_err());
        // İlk görüntü girdisi (TOC[1]) konumu: sahte offset (dosya dışı, taşma sınırı).
        for pos in [u32::MAX, u32::MAX - 10, good.len() as u32 - 10] {
            let mut b = good.clone();
            set(&mut b, 16 + 12 + 8, pos);
            assert!(parse(&b, 24, false).is_err(), "konum {pos}");
        }
        // Görüntü başlığı: sıfır boyut, dev boyut, sıcak nokta dışarıda, yanlış tür, sahte
        // parça uzunluğu (pikselleri dosya dışına iter).
        let first = 16 + 3 * 12;
        for (field, v) in [(16, 0u32), (20, 0), (16, MAX_CURSOR_SIDE + 1), (16, u32::MAX), (24, 99), (4, 7), (0, 3), (0, u32::MAX)] {
            let mut b = good.clone();
            set(&mut b, first + field, v);
            assert!(parse(&b, 24, false).is_err(), "alan {field} = {v}");
        }
        // Dosya boyutu sınırı (akış uzunluğuna bakılır, okunmaz).
        let r = parse_xcursor(&mut Cursor::new(&good), MAX_CURSOR_FILE_BYTES + 1, 24, false);
        assert!(matches!(r, Err(ImageError::FileTooLarge { .. })));
        // Yalnızca yorum girdileri.
        let mut only_comment = build(&[]);
        only_comment.truncate(16 + 12);
        assert!(parse(&only_comment, 24, false).is_err());
    }

    #[test]
    fn broken_later_frame_truncates_animation() {
        let mut good = build(&[img(24, 4, 0xff00_0000), img(24, 4, 0xff00_0000), img(24, 4, 0xff00_0000)]);
        // Üçüncü görüntünün konumunu dosya dışına it.
        let at = 16 + 3 * 12 + 8;
        good[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse(&good, 24, true).unwrap().len(), 2);
    }

    #[test]
    fn pixel_budget_bounds_animation() {
        // Kare sınırı: fazlası okunmaz.
        let frames: Vec<Img> = (0..MAX_CURSOR_FRAMES + 3).map(|_| img(24, 2, 0xff00_0000)).collect();
        assert_eq!(parse(&build(&frames), 24, true).unwrap().len(), MAX_CURSOR_FRAMES);
        // Piksel bütçesi: 512² karelerden en çok 8M / 262144 = 30 kare.
        let big: Vec<Img> = (0..32).map(|_| img(512, MAX_CURSOR_SIDE, 0xff00_0000)).collect();
        let n = (MAX_CURSOR_PIXELS / u64::from(MAX_CURSOR_SIDE * MAX_CURSOR_SIDE)) as usize;
        assert_eq!(parse(&build(&big), 512, true).unwrap().len(), n);
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn theme_chain_synonyms_and_loops() {
        let root = tempfile::tempdir().unwrap();
        let (user, sys) = (root.path().join("user"), root.path().join("sys"));
        let cur = build(&[img(24, 4, 0xff00_0000)]);
        // Child: yalnızca left_ptr (eşanlamlı) ve kendi index'i; Parent: default, pointer, xterm.
        write(&user.join("Child/index.theme"), b"[Icon Theme]\nInherits=\"Parent\";Loop\n");
        write(&user.join("Child/cursors/left_ptr"), &cur);
        write(&sys.join("Parent/index.theme"), b"[Icon Theme]\nInherits=Child,Missing\n");
        for n in ["default", "hand2", "xterm"] {
            write(&sys.join(format!("Parent/cursors/{n}")), &cur);
        }
        write(&sys.join("Loop/index.theme"), b"[Icon Theme]\nInherits=Loop,Child\n");
        let t = CursorTheme::with_dirs("Child", vec![user.clone(), sys.clone()]);
        assert_eq!(t.chain(), ["Child", "Parent", "Missing", "Loop"]);
        assert!(t.has_xcursor());
        // Temanın kendi eşanlamlısı üst temanın birincil adından önce.
        assert_eq!(t.find(CursorRole::Default).unwrap(), user.join("Child/cursors/left_ptr"));
        assert_eq!(t.find(CursorRole::Pointer).unwrap(), sys.join("Parent/cursors/hand2"));
        assert!(t.find(CursorRole::Wait).is_none());
        assert_eq!(t.load(CursorRole::Text, 24, false).unwrap()[0].image.size(), (4, 4));
        assert!(matches!(t.load(CursorRole::Wait, 24, false), Err(ImageError::NotFound(_))));
        // Geçersiz tema adları zincir üretmez.
        for bad in ["../x", "/etc", ".hidden", ""] {
            assert!(CursorTheme::with_dirs(bad, vec![sys.clone()]).chain().is_empty(), "{bad}");
        }
    }

    #[test]
    fn load_request_runs_cursor_jobs() {
        let root = tempfile::tempdir().unwrap();
        let frames: Vec<Img> = (0..3).map(|_| img(48, 40, 0xff00_0000)).collect();
        write(&root.path().join("T/cursors/watch"), &build(&frames));
        let theme = std::sync::Arc::new(CursorTheme::with_dirs("T", vec![root.path().to_owned()]));
        let run = |animate, target| {
            let mut r = crate::LoadRequest::cursor(theme.clone(), CursorRole::Wait, 48, animate);
            r.options.target = target;
            r.run().unwrap()
        };
        assert!(matches!(run(false, None), crate::Decoded::Still(ref i) if i.size() == (40, 40)));
        match run(true, Some(crate::TargetSize::new(20, 20, crate::Fit::Contain))) {
            crate::Decoded::Animated(a) => assert_eq!((a.frames.len(), a.size()), (3, (20, 20))),
            other => panic!("{other:?}"),
        }
        let r = crate::LoadRequest::cursor(theme, CursorRole::Text, 48, false);
        assert!(format!("{r:?}").contains("Text"));
        assert!(r.run().is_err());
    }

    #[test]
    fn deep_chain_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        for i in 0..40 {
            write(&root.path().join(format!("t{i}/index.theme")), format!("[Icon Theme]\nInherits=t{}\n", i + 1).as_bytes());
        }
        let t = CursorTheme::with_dirs("t0", vec![root.path().to_owned()]);
        assert_eq!(t.chain().len(), MAX_INHERIT_DEPTH + 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_loops_and_fifos_are_not_found() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let cursors = root.path().join("T/cursors");
        std::fs::create_dir_all(&cursors).unwrap();
        symlink(cursors.join("default"), cursors.join("default")).unwrap();
        symlink(cursors.join("b"), cursors.join("left_ptr")).unwrap();
        symlink(cursors.join("left_ptr"), cursors.join("b")).unwrap();
        let made = std::process::Command::new("mkfifo").arg(cursors.join("arrow")).status().is_ok_and(|s| s.success());
        // Kendine dönen tema dizini.
        symlink(root.path().join("Loop"), root.path().join("Loop")).unwrap();
        let t = CursorTheme::with_dirs("T", vec![root.path().to_owned()]);
        assert!(t.find(CursorRole::Default).is_none());
        if made {
            assert!(read_xcursor_file(&cursors.join("arrow"), 24, false).is_err(), "FIFO engellemez");
        }
        assert!(CursorTheme::with_dirs("Loop", vec![root.path().to_owned()]).find(CursorRole::Default).is_none());
        // Symlink ile eşanlamlı (gerçek temalardaki gibi) çalışır.
        std::fs::write(cursors.join("real"), build(&[img(24, 2, 0xff00_0000)])).unwrap();
        symlink(cursors.join("real"), cursors.join("hand2")).unwrap();
        assert!(t.load(CursorRole::Pointer, 24, false).is_ok());
    }

    #[test]
    fn roles_cover_shapes_and_have_unique_primary_names() {
        let mut seen = HashSet::new();
        for r in CursorRole::ALL {
            assert!(!r.names().is_empty() && r.names().iter().all(|n| crate::icon::valid_name(n)));
            assert!(seen.insert(r.names()[0]), "{r:?}");
        }
        assert_eq!(CursorRole::from(nuxut_core::input::CursorShape::ResizeSe), CursorRole::ResizeNwse);
    }

    #[test]
    fn sizes_from_toc_and_hyprcursor_fallback() {
        let root = tempfile::tempdir().unwrap();
        let base = root.path().to_owned();
        write(&base.join("X/cursors/left_ptr"), &build(&[img(32, 4, 0xff00_0000), img(24, 3, 0xff00_0000), img(24, 3, 0xff00_0000)]));
        let x = CursorTheme::with_dirs("X", vec![base.clone()]);
        assert_eq!(x.sizes(), CursorSizes { fixed: vec![24, 32], scalable: false });
        assert!(!x.has_hyprcursor());
        // Hyprcursor teması X'ten miras alır: X'te olmayan `pointer` hyprcursor'dan gelir,
        // `default` ise X'in XCursor dosyasından (XCursor önce).
        use hyprcursor::tests::{hlc, png};
        write(&base.join("H/index.theme"), b"[Icon Theme]\nInherits=X\n");
        write(&base.join("H/manifest.hl"), b"cursors_directory = hc\n");
        let shape = hlc("define_override = hand2\ndefine_size = 48, a.png\ndefine_size = 0, a.svg\n", &[("a.png", png(48, [9, 9, 9]))]);
        write(&base.join("H/hc/pointer.hlc"), &shape);
        let h = CursorTheme::with_dirs("H", vec![base.clone()]);
        assert!(h.has_hyprcursor() && h.has_xcursor());
        assert_eq!(h.load(CursorRole::Pointer, 24, false).unwrap()[0].nominal, 48);
        assert_eq!(h.load(CursorRole::Default, 24, false).unwrap()[0].image.size(), (3, 3));
        assert!(matches!(h.load(CursorRole::Text, 24, false), Err(ImageError::NotFound(_))));
        // Yalnızca hyprcursor: boyutlar metadan.
        write(&base.join("O/manifest.hl"), b"cursors_directory = hc\n");
        write(&base.join("O/hc/left_ptr.hlc"), &shape);
        let o = CursorTheme::with_dirs("O", vec![base.clone()]);
        assert_eq!(o.sizes(), CursorSizes { fixed: vec![48], scalable: true });
        // Bozuk şekil: NotFound değil, hata.
        write(&base.join("O/hc/xterm.hlc"), b"PK\x03\x04bozuk");
        assert!(matches!(o.load(CursorRole::Text, 24, false), Err(ImageError::InvalidData(_))));
    }

    #[test]
    fn toc_sizes_are_bounded() {
        let mut b = build(&[img(24, 2, 0xff00_0000)]);
        b[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(xcursor_sizes(&mut Cursor::new(&b), b.len() as u64).is_err());
        assert!(xcursor_sizes(&mut Cursor::new(&b[..10]), 10).is_err());
    }

    /// Sistemde kurulu temalar (varsa): her önizleme imleci panik olmadan çözülür.
    #[test]
    fn installed_themes_parse() {
        for theme in crate::icon::cursor_themes_in(&[PathBuf::from("/usr/share/icons")]).iter().filter(|t| t.xcursor) {
            let t = CursorTheme::with_dirs(theme.id.clone(), vec![PathBuf::from("/usr/share/icons")]);
            for role in CursorRole::PREVIEW {
                if let Ok(frames) = t.load(role, 48, true) {
                    assert!(!frames.is_empty() && frames.len() <= MAX_CURSOR_FRAMES, "{} {role:?}", theme.id);
                }
            }
            assert!(t.load(CursorRole::Default, 24, false).is_ok(), "{}", theme.id);
        }
    }
}
