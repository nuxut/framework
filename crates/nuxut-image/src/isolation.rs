//! Güvenilmeyen görselin yalıtılmış çözümü için sözleşme (ADR-0007).
//!
//! Yalıtılmış işçinin kendisi `nuxut-image-sandbox` crate'indedir (süreç, seccomp, havuz);
//! bu crate yalnızca arayüzü ([`IsolatedDecoder`]), hata türünü ([`SandboxFailure`]), biçim
//! koklamayı ([`sniff_format`]) ve işçinin içinde çağrılan çözümü ([`decode_isolated`]) taşır.
//! Bağımlılık yönü: `nuxut-image-sandbox` → `nuxut-image` (tersi yok); [`crate::Loader`] işçiyi
//! bu trait üzerinden kullanır.

use std::sync::Arc;

use crate::{DecodeOptions, Image, ImageError, Trust};

/// Bu platformda/oturumda güvenilmeyen görselin nasıl çözülebildiği.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Isolation {
    /// Ayrı, kısıtlı (seccomp + landlock + rlimit) yardımcı süreçte.
    Process,
    /// İşçi yok, kısıtlanamıyor ya da Linux dışı platform: SVG/JXL/AVIF reddedilir, yalnızca
    /// temel raster (PNG/JPEG/WebP/GIF/QOI) süreç içine düşer, ilk kare (K2-b, K5).
    Unavailable,
}

/// Yalıtılmış çözümün başarısızlık türü. Hiçbir varyant bayt, yol ya da ad taşımaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SandboxFailure {
    /// İstek duvar saati süresini aştı; işçi öldürüldü.
    #[error("zaman aşımı")]
    Timeout,
    /// İşçi bir sinyalle ya da beklenmedik çıkışla öldü (seccomp, rlimit, panik) ya da girdi
    /// negatif önbellekte (daha önce işçiyi öldürdü).
    #[error("işçi öldü")]
    Killed,
    /// İşçinin yanıtı sözleşmeye uymuyor (boyut, uzunluk, fazla/eksik bayt); işçi öldürüldü.
    #[error("protokol ihlali")]
    Protocol,
    /// İşçi ikilisi yok, el sıkışma tutmadı ya da kısıtlamalar kurulamadı.
    #[error("yalıtım kullanılamıyor")]
    Unavailable,
    /// Hız sınırı ya da susturulmuş anahtar: istek çözülmeden reddedildi.
    #[error("hız sınırı")]
    RateLimited,
}

/// Anahtarsız istek: `Loader`'da anahtar başına birleştirme/iptal uygulanmaz. Yalıtılmış
/// çözücüye anahtarsız gelen güvenilmeyen iş susturmadan kaçamaz: `Loader` kaynaktan bir anahtar
/// türetir (yol/ad özeti), türetilemeyen (bayt) iş ve doğrudan çağrılar [`ANON_KEY`] kovasına
/// düşer.
pub const NO_KEY: u64 = 0;

/// Anahtarı türetilemeyen güvenilmeyen işlerin ortak susturma kovası (ADR-0007 bulgu 4d).
pub const ANON_KEY: u64 = u64::MAX;

/// Kaynak kimliğinden kararlı bir anahtar (FNV-1a, 64 bit; [`NO_KEY`] ve [`ANON_KEY`] olmaz).
/// Yalnızca süreç içi kova kimliğidir; loglanmaz.
pub fn derive_key(parts: &[&[u8]]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for &b in *p {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        // Parça sınırı: ("ab","c") ≠ ("a","bc").
        h ^= 0xFF;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    match h {
        NO_KEY | ANON_KEY => 1,
        h => h,
    }
}

/// Bu süreç yalıtılmış işçi mi (kilitler kurulduktan sonra `worker_main` açar).
static WORKER_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// **Yalnızca** `nuxut_image_sandbox::worker_main` çağırır, seccomp/landlock kurulduktan
/// SONRA. Açıldıktan sonra [`decode_isolated`] çalışır; geri alınamaz.
#[doc(hidden)]
pub fn enter_worker_mode() {
    WORKER_MODE.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// İşçi kipi açık mı.
pub fn in_worker_mode() -> bool {
    WORKER_MODE.load(std::sync::atomic::Ordering::SeqCst)
}

/// Kullanıcı dosyası için kararlı anahtar: `domain` (tüketici/amaç, ör. `"wallpaper"`,
/// `"avatar"`) + yol. [`IsolatedDecoder::decode_file`]'ın `key`'i (susturma kovası) için
/// önerilen kalıp; aynı dosya aynı alanda aynı anahtarı verir, bozuk bir dosyanın art arda
/// başarısızlığı başka dosyaları susturmaz. Loglanmaz.
pub fn file_key(domain: &str, path: &std::path::Path) -> u64 {
    #[cfg(unix)]
    let bytes = std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec();
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    derive_key(&[domain.as_bytes(), &bytes])
}

/// Süren bir çözümü durdurma bayrağı. Kopyalar aynı bayrağı paylaşır; [`cancel`](Self::cancel)
/// sonrası [`IsolatedDecoder::decode_file`] en geç bir yoklama diliminde (≤ 50 ms) işçiyi
/// öldürür ve `ImageError::Cancelled` döner. İptal başarısızlık sayılmaz (negatif önbelleğe ve
/// susturma sayacına girmez).
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<std::sync::atomic::AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// [`IsolatedDecoder::decode_file`] sonucu: tek kare ve kaynağın animasyonlu olup olmadığı
/// (ADR-0007 K5: yalnızca ilk kare verilir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFile {
    pub image: Image,
    /// Kaynak animasyonluydu (GIF/WebP/APNG, ≥ 2 kare); `image` ilk karedir.
    pub animated_source: bool,
    /// Animasyonlu kaynağın kare sayısı ve bir tur süresi (kap yapısından, piksel çözülmeden;
    /// [`crate::animation_info`]). Yalıtılmış çözücüde yapı **işçide** yürünür ve yanıtla gelir
    /// (tel v4); ana süreç güvenilmeyen kabı ayrıştırmaz. Durağan kaynakta ya da yapı
    /// tanınmadığında `None`. Ele geçirilmiş işçi yalan söyleyebilir: yalnızca ön denetim
    /// içindir (ör. `nuxut-video` içe aktarma bütçesi; kesin sınır kodlayıcının sayacı).
    pub animation: Option<crate::AnimationInfo>,
}

impl DecodedFile {
    /// Süreç içi çözücüler (test/araç, K2-b yedeği) için: kaynak baytlarından animasyon
    /// bayrağı ve bilgisi (`limits` kapısıyla).
    pub fn from_source(image: Image, bytes: &[u8], limits: &crate::Limits) -> Self {
        let animated_source = matches!(crate::is_animated(bytes, limits), Ok(true));
        let animation = if animated_source { crate::animation_info(bytes) } else { None };
        Self { image, animated_source, animation }
    }
}

/// Kullanıcının verdiği yolu [`IsolatedDecoder::decode_file`] için açar: `O_NONBLOCK` (FIFO/
/// aygıt açılışta bloklamaz), fd'den `fstat` ile normal dosya olduğu doğrulanır. Sembolik bağ
/// izlenir. Dosya bir kez açılır; anahtar ve akış aynı fd'dendir (TOCTOU yok).
pub fn open_user_file(path: &std::path::Path) -> Result<std::fs::File, ImageError> {
    crate::decode::open_regular(path)
}

/// Güvenilmeyen baytları yalıtılmış olarak çözen nesne (`nuxut_image_sandbox::Sandbox`).
/// [`crate::Loader::with_sandbox`] ile kurulur.
pub trait IsolatedDecoder: Send + Sync + std::fmt::Debug {
    fn isolation(&self) -> Isolation;
    /// Kabul edilen en büyük girdi; `Loader` güvenilmeyen dosyayı bundan fazla belleğe okumaz.
    fn max_input_bytes(&self) -> u64 {
        u64::MAX
    }
    /// `bytes`'ı çözer; sonuç her zaman tek karedir (animasyonun ilk karesi). `key` çağıranın
    /// kaynak kimliği (tepside öğe, bildirimde uygulama; [`NO_KEY`] = anahtarsız): aynı
    /// anahtarda bekleyen eski iş iptal edilir, art arda başarısızlık anahtarı susturur.
    fn decode(&self, key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions) -> Result<Image, ImageError>;

    /// Açık bir kullanıcı dosyasını ([`open_user_file`]) çözer; sonuç tek kare + animasyon
    /// bayrağı. Duvar kağıdı, kilit avatarı (`~/.face`), içe aktarılan durağan görsel gibi
    /// büyük ya da kullanıcı dosyaları için: `Sandbox` dosyayı belleğe almaz, işçiye parça
    /// parça akıtır; negatif önbellek anahtarı dosya kimliğidir (`dev, ino, len, mtime, ctime`;
    /// içerik iki kez okunmaz). `cancel` kurulunca ≤ 50 ms'de durur (`ImageError::Cancelled`).
    /// `key` için [`file_key`].
    ///
    /// Varsayılan gövde (test/araç çözücüleri): dosyayı `max_input_bytes`'a kadar okur ve
    /// [`decode`](Self::decode) çağırır.
    fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag) -> Result<DecodedFile, ImageError> {
        if cancel.is_cancelled() {
            return Err(ImageError::Cancelled);
        }
        let limit = opts.limits.max_file_bytes.min(self.max_input_bytes());
        let bytes = crate::decode::read_limited(file, std::path::Path::new(""), limit)?;
        if cancel.is_cancelled() {
            return Err(ImageError::Cancelled);
        }
        let bytes: Arc<[u8]> = bytes.into();
        let image = self.decode(key, bytes.clone(), opts)?;
        Ok(DecodedFile::from_source(image, &bytes, &opts.limits))
    }
}

/// İçerik baytlarından tespit edilen biçim (uzantıya bakılmaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SniffedFormat {
    /// SVG/XML (baştaki BOM/boşluktan sonra `<`) ya da gzip (svgz).
    Svg,
    Jxl,
    Avif,
    Png,
    Jpeg,
    WebP,
    Gif,
    Qoi,
    /// İmzası tanınmayan ya da başka bir biçim.
    Other,
}

impl SniffedFormat {
    /// ADR-0007 K2-b: işçi kullanılamıyorken (`Unavailable`) süreç içine düşebilen temel
    /// raster biçimler. SVG/JXL/AVIF asla.
    pub const fn in_process_fallback(self) -> bool {
        matches!(self, Self::Png | Self::Jpeg | Self::WebP | Self::Gif | Self::Qoi)
    }

    /// Protokol kodu (işçi isteğindeki tür baytı).
    pub const fn code(self) -> u8 {
        match self {
            Self::Svg => 1,
            Self::Jxl => 2,
            Self::Avif => 3,
            Self::Png => 4,
            Self::Jpeg => 5,
            Self::WebP => 6,
            Self::Gif => 7,
            Self::Qoi => 8,
            Self::Other => 9,
        }
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            1 => Self::Svg,
            2 => Self::Jxl,
            3 => Self::Avif,
            4 => Self::Png,
            5 => Self::Jpeg,
            6 => Self::WebP,
            7 => Self::Gif,
            8 => Self::Qoi,
            9 => Self::Other,
            _ => return None,
        })
    }
}

/// Biçimi yalnızca içerikten tespit eder; SVG sınıfı güven kapısınınkiyle aynıdır.
pub fn sniff_format(bytes: &[u8]) -> SniffedFormat {
    if crate::decode::svg_like(bytes) {
        return SniffedFormat::Svg;
    }
    if crate::decode::looks_like_jxl(bytes) {
        return SniffedFormat::Jxl;
    }
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => SniffedFormat::Png,
        Ok(image::ImageFormat::Jpeg) => SniffedFormat::Jpeg,
        Ok(image::ImageFormat::WebP) => SniffedFormat::WebP,
        Ok(image::ImageFormat::Gif) => SniffedFormat::Gif,
        Ok(image::ImageFormat::Qoi) => SniffedFormat::Qoi,
        Ok(image::ImageFormat::Avif) => SniffedFormat::Avif,
        _ => SniffedFormat::Other,
    }
}

/// **Yalnızca yalıtılmış işçinin içinde** çağrılır: güven kapısı uygulanmaz (SVG ayrıştırılır),
/// sınırlar ve hedef `opts`'tan gelir, sonuç her zaman ilk karedir. İşçi kipi
/// ([`enter_worker_mode`]) açık değilse `Unsupported` döner: süreç içinde güvenilmeyen baytla
/// kazayla çağrılamaz. Süreç içi yol için [`crate::decode_bytes`] kullanın.
#[doc(hidden)]
pub fn decode_isolated(bytes: &[u8], opts: &DecodeOptions) -> Result<Image, ImageError> {
    if !in_worker_mode() {
        return Err(ImageError::Unsupported("decode_isolated outside worker"));
    }
    let opts = DecodeOptions { animate: false, ..opts.with_trust(Trust::System) };
    if bytes.len() as u64 > opts.limits.max_file_bytes {
        return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit: opts.limits.max_file_bytes });
    }
    match sniff_format(bytes) {
        // `<svg` 4 KiB'den sonra gelse de SVG ayrıştırıcısına gider; gzip (svgz) desteklenmez.
        SniffedFormat::Svg if !bytes.starts_with(&[0x1F, 0x8B]) => crate::svg::decode_svg(bytes, &opts),
        SniffedFormat::Svg => Err(ImageError::UnknownFormat),
        _ => crate::decode_bytes(bytes, &opts).map(crate::Decoded::into_first_frame),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_matches_gate_and_signatures() {
        assert_eq!(sniff_format(b"  <svg/>"), SniffedFormat::Svg);
        assert_eq!(sniff_format(&[0x1F, 0x8B, 0, 0]), SniffedFormat::Svg);
        assert_eq!(sniff_format(&[0xFF, 0x0A, 1]), SniffedFormat::Jxl);
        assert_eq!(sniff_format(b"\x89PNG\r\n\x1a\n...."), SniffedFormat::Png);
        assert_eq!(sniff_format(b"GIF89a...."), SniffedFormat::Gif);
        assert_eq!(sniff_format(b"qoif...."), SniffedFormat::Qoi);
        assert_eq!(sniff_format(b"hello"), SniffedFormat::Other);
        for f in [SniffedFormat::Svg, SniffedFormat::Jxl, SniffedFormat::Avif, SniffedFormat::Other] {
            assert!(!f.in_process_fallback());
        }
        for c in 0..=255u8 {
            if let Some(f) = SniffedFormat::from_code(c) {
                assert_eq!(f.code(), c);
            }
        }
    }

    #[test]
    fn failure_display_carries_no_content() {
        for f in [SandboxFailure::Timeout, SandboxFailure::Killed, SandboxFailure::Protocol, SandboxFailure::Unavailable, SandboxFailure::RateLimited] {
            let e = ImageError::Sandbox(f);
            assert!(!e.to_string().is_empty());
            assert_eq!(format!("{e:?}"), e.to_string());
        }
    }
}
