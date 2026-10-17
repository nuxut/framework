//! Raster ve SVG çözme. Her yol önce boyutu sınırlara karşı denetler, sonra çözer; animasyon
//! kareleri tek tek küçültülerek biriktirilir (tam boyutlu kare dizisi bellekte tutulmaz).

use std::io::{Cursor, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::{DecodeOptions, Image, ImageError, Limits, Trust, resize};

/// Animasyon karesi.
#[derive(Debug, Clone)]
pub struct Frame {
    pub image: Image,
    /// Bu karenin ekranda kalma süresi.
    pub delay: Duration,
}

/// Animasyonlu görsel. Kareler aynı boyuttadır ve sonsuz döngüyle oynatılır.
#[derive(Debug, Clone)]
pub struct Animation {
    pub frames: Vec<Frame>,
}

impl Animation {
    pub fn size(&self) -> (u32, u32) {
        self.frames[0].image.size()
    }

    /// Tek döngünün süresi.
    pub fn duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }

    pub fn byte_size(&self) -> usize {
        self.frames.iter().map(|f| f.image.byte_size()).sum()
    }
}

/// Çözme sonucu.
#[derive(Debug)]
pub enum Decoded {
    Still(Image),
    /// En az iki kare, hepsi bellekte.
    Animated(Animation),
    /// Bellek bütçesini aşan animasyon: kareler arka planda akıtılır.
    Streamed(AnimationStream),
}

impl Decoded {
    /// Durağan görsel ya da animasyonun ilk karesi.
    pub fn first_frame(&self) -> &Image {
        match self {
            Self::Still(i) => i,
            Self::Animated(a) => &a.frames[0].image,
            Self::Streamed(s) => &s.first_frame().image,
        }
    }

    pub fn into_first_frame(self) -> Image {
        match self {
            Self::Still(i) => i,
            Self::Animated(mut a) => a.frames.swap_remove(0).image,
            Self::Streamed(s) => s.first_frame().image.clone(),
        }
    }

    pub fn size(&self) -> (u32, u32) {
        self.first_frame().size()
    }

    pub fn byte_size(&self) -> usize {
        match self {
            Self::Still(i) => i.byte_size(),
            Self::Animated(a) => a.byte_size(),
            Self::Streamed(s) => s.first_frame().image.byte_size() * (STREAM_AHEAD + 1),
        }
    }
}

/// Dosyayı okur ve çözer. Biçim içerikten tespit edilir; `System` güvende `.svg` uzantısı da
/// SVG sayılır, `Untrusted`'ta uzantıya hiç bakılmaz (güven kapısı: [`untrusted_gate`]).
pub fn decode_file(path: &Path, opts: &DecodeOptions) -> Result<Decoded, ImageError> {
    decode_open_file(open_regular(path)?, path, opts)
}

/// Dosyayı bekletmeden açar ve fd'den normal dosya olduğunu doğrular: kullanıcının yazabildiği
/// bir dizine bırakılmış FIFO/aygıt, açılışta ya da okumada thread'i sonsuza dek bloklayamaz
/// (`O_NONBLOCK`; normal dosyada etkisiz). Tema sembolik bağları izlenir.
pub(crate) fn open_regular(path: &Path) -> Result<std::fs::File, ImageError> {
    let io = |source| ImageError::Io { path: path.to_owned(), source };
    #[cfg(unix)]
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(path)
            .map_err(io)?
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path).map_err(io)?;
    if !file.metadata().map_err(io)?.is_file() {
        return Err(io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "normal dosya değil")));
    }
    Ok(file)
}

/// [`decode_file`], önceden açılmış dosyadan: güveni denetlenen dosya ile çözülen dosya aynı
/// fd'dir (denetim ile okuma arasında yol başka dosyaya döndürülemez). `path` yalnızca
/// uzantı ve hata iletisi içindir.
pub(crate) fn decode_open_file(file: std::fs::File, path: &Path, opts: &DecodeOptions) -> Result<Decoded, ImageError> {
    let bytes = read_limited(file, path, opts.limits.max_file_bytes)?;
    if opts.trust() == Trust::System {
        let svg_ext = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg"));
        if svg_ext || looks_like_svg(&bytes) {
            return crate::svg::decode_svg(&bytes, opts).map(Decoded::Still);
        }
    }
    decode_bytes(&bytes, opts)
}

/// Bellekteki baytları çözer.
pub fn decode_bytes(bytes: &[u8], opts: &DecodeOptions) -> Result<Decoded, ImageError> {
    if bytes.len() as u64 > opts.limits.max_file_bytes {
        return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit: opts.limits.max_file_bytes });
    }
    // Güvenilmeyen kaynakta süreç içinde yalnızca temel raster ve yalnızca ilk kare (K5).
    let first_only;
    let opts = if opts.trust() == Trust::Untrusted {
        untrusted_gate(bytes)?;
        first_only = DecodeOptions { animate: false, ..*opts };
        &first_only
    } else {
        opts
    };
    if looks_like_jxl(bytes) {
        return decode_jxl(bytes, opts).map(Decoded::Still);
    }
    if opts.trust() == Trust::System && looks_like_svg(bytes) {
        return crate::svg::decode_svg(bytes, opts).map(Decoded::Still);
    }
    let format = image::guess_format(bytes).map_err(|_| ImageError::UnknownFormat)?;
    let lim = image_limits(&opts.limits);
    match format {
        ImageFormat::Gif if opts.animate => {
            let mut d = GifDecoder::new(Cursor::new(bytes))?;
            let canvas = gate(&mut d, opts, &lim)?;
            animated(d.into_frames(), opts, bytes, format, canvas)
        }
        ImageFormat::WebP if opts.animate => {
            let mut d = WebPDecoder::new(Cursor::new(bytes))?;
            let canvas = gate(&mut d, opts, &lim)?;
            if d.has_animation() { animated(d.into_frames(), opts, bytes, format, canvas) } else { still(d, opts) }
        }
        ImageFormat::Png => {
            let mut d = PngDecoder::new(Cursor::new(bytes))?;
            let canvas = gate(&mut d, opts, &lim)?;
            if opts.animate && d.is_apng()? {
                return animated(d.apng()?.into_frames(), opts, bytes, format, canvas);
            }
            // Paletli PNG küçültülecekse RGB(A)'ya açılmadan en dar biçimden (indexed.rs).
            let oriented = !matches!(d.orientation(), Ok(image::metadata::Orientation::NoTransforms));
            if let Some(img) = crate::indexed::decode_png_indexed(bytes, opts, oriented)? {
                return Ok(Decoded::Still(img));
            }
            still(d, opts)
        }
        _ => {
            // Sınırlar kapıda verilir: okuyucuya önceden verilirse aynı ihlal başka bir hata
            // türüyle (Decode) dönerdi. `into_decoder` yalnızca başlığı okur.
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            reader.no_limits();
            let mut d = reader.into_decoder()?;
            gate(&mut d, opts, &lim)?;
            still(d, opts)
        }
    }
}

pub(crate) fn read_limited(file: std::fs::File, path: &Path, limit: u64) -> Result<Vec<u8>, ImageError> {
    let io = |source| ImageError::Io { path: path.to_owned(), source };
    let size = file.metadata().map_err(io)?.len();
    if size > limit {
        return Err(ImageError::FileTooLarge { size, limit });
    }
    let mut bytes = Vec::with_capacity(size as usize);
    // Dosya okurken büyüyebilir; sınırın bir fazlası okunursa reddedilir.
    file.take(limit + 1).read_to_end(&mut bytes).map_err(io)?;
    if bytes.len() as u64 > limit {
        return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit });
    }
    Ok(bytes)
}

fn image_limits(l: &Limits) -> image::Limits {
    let mut lim = image::Limits::default();
    lim.max_image_width = Some(l.max_width);
    lim.max_image_height = Some(l.max_height);
    // Çözücünün kendi ara tamponları da bütçeden (başlık tahmini kaçarsa son savunma).
    lim.max_alloc = Some(l.max_decode_bytes);
    lim
}

/// Çözümün tepe bellek tahmini (bayt), başlıktan: çözücünün tamponu (kanal × bit derinliği),
/// 8 bit'e dönüşüm kopyası (16 bit/float kaynak) ve çıktı (hedefe küçültülmüş RGBA8; hedefsiz
/// RGBA8 kaynakta yerinde). `per_pixel`: biçime özgü çözücü tamponu (JPEG XL); `None` →
/// `color`'dan.
pub(crate) fn decode_estimate(color: image::ColorType, (w, h): (u32, u32), per_pixel: Option<u64>, target: Option<crate::TargetSize>) -> u64 {
    let px = u64::from(w) * u64::from(h);
    let decoded = px.saturating_mul(per_pixel.unwrap_or(u64::from(color.bytes_per_pixel())));
    let wide = color.bytes_per_pixel() > color.channel_count();
    let convert = if wide { px.saturating_mul(4) } else { 0 };
    let out = match target.map(|t| resize::plan((w, h), t)) {
        Some(p) if !p.is_identity((w, h)) => u64::from(p.dst.0) * u64::from(p.dst.1) * 4,
        _ if color == image::ColorType::Rgba8 => 0,
        _ => px.saturating_mul(4),
    };
    decoded.saturating_add(convert).saturating_add(out)
}

/// Sınır kapısı (her biçim, her yol): boyutu çözmeden önce [`Limits::check_dimensions`] ve
/// bellek bütçesini ([`Limits::check_budget`], [`decode_estimate`]) denetler, çözücüye bellek
/// sınırını verir. Çözüm sonrası sonuç ve her animasyon karesi yeniden aynı kapıdan geçer
/// (`verify`).
fn gate(d: &mut impl ImageDecoder, opts: &DecodeOptions, lim: &image::Limits) -> Result<(u32, u32), ImageError> {
    let (w, h) = d.dimensions();
    opts.limits.check_dimensions(w, h)?;
    opts.limits.check_budget((w, h), decode_estimate(d.color_type(), (w, h), None, opts.target))?;
    d.set_limits(lim.clone())?;
    Ok((w, h))
}

/// Çözücünün verdiği tampon başlıktakiyle aynı boyutta ve sınır içinde mi.
fn verify(got: (u32, u32), expected: (u32, u32), limits: &Limits) -> Result<(), ImageError> {
    limits.check_dimensions(got.0, got.1)?;
    if got != expected {
        return Err(ImageError::InvalidData(format!("çözücü {}x{} verdi, başlık {}x{}", got.0, got.1, expected.0, expected.1)));
    }
    Ok(())
}

fn still(mut d: impl ImageDecoder, opts: &DecodeOptions) -> Result<Decoded, ImageError> {
    let orientation = d.orientation().ok();
    let dims = d.dimensions();
    let mut img = DynamicImage::from_decoder(d)?;
    verify((img.width(), img.height()), dims, &opts.limits)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(Decoded::Still(from_dynamic(img, opts)?))
}

/// Çözülmüş görseli premultiplied RGBA8'e çevirir ve hedefe küçültür. Küçültme gerekiyorsa
/// RGBA'dan dar biçimler (RGB, gri, gri+alfa; 16 bit/float olanlar önce 8 bite) ÖNCE kendi
/// kanal sayısıyla küçültülür, SONRA RGBA'ya genişletilir: tam boyutlu RGBA kopyası (tepe
/// bellek, gride 4 kat) oluşmaz. Sonuç tam boyutlu RGBA yoluyla piksel piksel aynıdır (kanallar
/// bağımsız örneklenir; test: `narrow_formats_match_rgba_path`). Paletli PNG/GIF çözücüden
/// zaten RGB(A)8 gelir.
fn from_dynamic(img: DynamicImage, opts: &DecodeOptions) -> Result<Image, ImageError> {
    if let Some(target) = opts.target {
        let src = (img.width(), img.height());
        let dst = resize::plan(src, target);
        if !dst.is_identity(src) {
            match img {
                DynamicImage::ImageRgb8(rgb) => return resize::resize_rgb(rgb.as_raw(), src, dst),
                DynamicImage::ImageLuma8(l) => return resize::resize_luma(l.as_raw(), src, dst),
                DynamicImage::ImageLumaA8(la) => return resize::resize_luma_alpha(la.into_raw(), src, dst),
                DynamicImage::ImageLuma16(_) => return resize::resize_luma(img.into_luma8().as_raw(), src, dst),
                DynamicImage::ImageLumaA16(_) => {
                    return resize::resize_luma_alpha(img.into_luma_alpha8().into_raw(), src, dst);
                }
                DynamicImage::ImageRgb16(_) | DynamicImage::ImageRgb32F(_) => {
                    return resize::resize_rgb(img.into_rgb8().as_raw(), src, dst);
                }
                _ => {}
            }
        }
    }
    full_rgba(img, opts)
}

/// Tam boyutlu RGBA8 yolu: genişlet, premultiply et, küçült.
fn full_rgba(img: DynamicImage, opts: &DecodeOptions) -> Result<Image, ImageError> {
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    let image = Image::from_straight(w, h, rgba.into_raw())?;
    resize::fit(image, opts.target)
}

/// GIF/WebP/APNG kareleri. Kareler birleştirilmiş tam tuval olarak gelir; her biri hemen
/// küçültülür. Toplam bellek `max_animation_bytes`'ı ya da çözme işi `max_animation_pixels`'i
/// aşarsa kareler bellekte tutulmaz: [`AnimationStream`] arka planda sırayla çözer. Kare
/// sayısı `max_frames`'i aşarsa yalnızca ilk kare kullanılır.
fn animated(
    frames: image::Frames<'_>,
    opts: &DecodeOptions,
    source: &[u8],
    format: ImageFormat,
    canvas: (u32, u32),
) -> Result<Decoded, ImageError> {
    let mut out: Vec<Frame> = Vec::new();
    let mut bytes: u64 = 0;
    // Her kare tam tuvale birleştirilir: iş = kare × tuval.
    let per_frame = u64::from(canvas.0) * u64::from(canvas.1);
    let mut work: u64 = 0;
    for frame in frames {
        let frame = convert_frame(frame?, opts, canvas)?;
        bytes += frame.image.byte_size() as u64;
        work = work.saturating_add(per_frame);
        out.push(frame);
        if out.len() > opts.limits.max_frames {
            tracing::warn!("animasyon {} kareyi aşıyor; yalnızca ilk kare kullanılıyor", opts.limits.max_frames);
            out.truncate(1);
            break;
        }
        if bytes > opts.limits.max_animation_bytes || work > opts.limits.max_animation_pixels {
            let first = out.swap_remove(0);
            let stream = AnimationStream::spawn(Arc::from(source), format, *opts, first)?;
            return Ok(Decoded::Streamed(stream));
        }
    }
    match out.len() {
        0 => Err(ImageError::Empty),
        1 => Ok(Decoded::Still(out.pop().map(|f| f.image).ok_or(ImageError::Empty)?)),
        _ => Ok(Decoded::Animated(Animation { frames: out })),
    }
}

/// Bir karenin ekranda kalabileceği en uzun süre (GIF 655 sn, APNG 65535 sn yazabilir):
/// oynatıcı zamanlayıcısı bundan uzağa kurulmaz.
pub const MAX_FRAME_DELAY: Duration = Duration::from_secs(60);

fn convert_frame(frame: image::Frame, opts: &DecodeOptions, canvas: (u32, u32)) -> Result<Frame, ImageError> {
    let (num, den) = frame.delay().numer_denom_ms();
    let ms = num.checked_div(den).unwrap_or(0);
    // Tarayıcılarla aynı: ≤ 10 ms gecikme 100 ms sayılır (eski GIF'ler 0 yazar).
    let delay = Duration::from_millis(effective_delay_ms(u64::from(ms))).min(MAX_FRAME_DELAY);
    let buffer = frame.into_buffer();
    let (w, h) = buffer.dimensions();
    verify((w, h), canvas, &opts.limits)?;
    let image = resize::fit(Image::from_straight(w, h, buffer.into_raw())?, opts.target)?;
    Ok(Frame { image, delay })
}

/// Sahipli kaynaktan kare yineleyicisi (akış thread'i için).
fn open_frames(bytes: Arc<[u8]>, format: ImageFormat, opts: &DecodeOptions) -> Result<(image::Frames<'static>, (u32, u32)), ImageError> {
    let lim = image_limits(&opts.limits);
    let cursor = Cursor::new(bytes);
    Ok(match format {
        ImageFormat::Gif => {
            let mut d = GifDecoder::new(cursor)?;
            let canvas = gate(&mut d, opts, &lim)?;
            (d.into_frames(), canvas)
        }
        ImageFormat::WebP => {
            let mut d = WebPDecoder::new(cursor)?;
            let canvas = gate(&mut d, opts, &lim)?;
            (d.into_frames(), canvas)
        }
        ImageFormat::Png => {
            let mut d = PngDecoder::new(cursor)?;
            let canvas = gate(&mut d, opts, &lim)?;
            (d.apng()?.into_frames(), canvas)
        }
        _ => return Err(ImageError::UnknownFormat),
    })
}

/// Bellekte tutulamayacak kadar büyük animasyon: kareler arka plan thread'inde sırayla
/// çözülür ve küçük bir kuyruğa (2 kare) konur. Tüketici kare almazsa thread kuyruk dolunca
/// bekler — görünmez/duraklatılmış animasyon CPU harcamaz. Düşünce thread durur.
pub struct AnimationStream {
    first: Frame,
    rx: Option<Receiver<Frame>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for AnimationStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AnimationStream({:?})", self.first.image)
    }
}

/// Akış kuyruğunun uzunluğu (bellekte en fazla bu kadar + gösterilen kare).
const STREAM_AHEAD: usize = 2;

impl AnimationStream {
    fn spawn(bytes: Arc<[u8]>, format: ImageFormat, opts: DecodeOptions, first: Frame) -> Result<Self, ImageError> {
        let (tx, rx) = sync_channel(STREAM_AHEAD);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("nuxut-anim".into())
            .spawn(move || stream_loop(&bytes, format, &opts, &tx, &flag))
            .map_err(|e| ImageError::Decode(format!("akış thread'i: {e}")))?;
        Ok(Self { first, rx: Some(rx), stop, thread: Some(thread) })
    }

    /// İlk kare (akış ikinci kareden devam eder, döngü başında yine ilk kareyi verir).
    pub fn first_frame(&self) -> &Frame {
        &self.first
    }

    pub fn size(&self) -> (u32, u32) {
        self.first.image.size()
    }

    /// Hazırsa sıradaki kare; yoksa `None` (çözücü yetişemedi ya da durdu).
    pub fn try_next(&self) -> Option<Frame> {
        self.rx.as_ref()?.try_recv().ok()
    }
}

impl Drop for AnimationStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Alıcı düşünce bekleyen `send` hata döner ve thread çıkar.
        self.rx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn stream_loop(bytes: &Arc<[u8]>, format: ImageFormat, opts: &DecodeOptions, tx: &SyncSender<Frame>, stop: &AtomicBool) {
    // İlk geçişte ilk kare zaten tüketicide; atlanır.
    let mut skip_first = true;
    loop {
        let (frames, canvas) = match open_frames(bytes.clone(), format, opts) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!("animasyon akışı açılamadı: {e}");
                return;
            }
        };
        let mut produced = 0usize;
        for frame in frames {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let frame = match frame.map_err(ImageError::from).and_then(|f| convert_frame(f, opts, canvas)) {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("animasyon karesi çözülemedi: {e}");
                    break;
                }
            };
            produced += 1;
            if std::mem::take(&mut skip_first) {
                continue;
            }
            if tx.send(frame).is_err() {
                return;
            }
            if produced >= opts.limits.max_frames {
                break;
            }
        }
        if produced < 2 {
            return;
        }
    }
}
pub(crate) fn looks_like_jxl(b: &[u8]) -> bool {
    b.starts_with(&[0xFF, 0x0A]) || b.starts_with(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A])
}

/// SVG sınıfı (güven kapısının gözüyle): baştaki BOM ve boşluktan sonra `<` ile başlayan her şey
/// (SVG/XML; `<svg` 4 KiB'den sonra gelse de) ya da gzip (svgz).
pub(crate) fn svg_like(b: &[u8]) -> bool {
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
    let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let b = &b[start..];
    b.starts_with(b"<") || b.starts_with(&[0x1F, 0x8B]) || looks_like_svg(b)
}

/// Güven kapısı (`Trust::Untrusted`, süreç içi yol): yalnızca içerik baytlarına bakar ve
/// yalnızca temel raster biçimleri ([`crate::SniffedFormat::in_process_fallback`]: PNG/JPEG/
/// WebP/GIF/QOI) geçirir (ADR-0007 K1-b/K2-b). SVG, JXL ve AVIF süreç içinde hiç
/// ayrıştırılmaz (`Unsupported`); bunlar yalnızca yalıtılmış işçide çözülür. İmzası tanınmayan
/// içerik `UnknownFormat`.
pub(crate) fn untrusted_gate(b: &[u8]) -> Result<(), ImageError> {
    use crate::SniffedFormat as F;
    match crate::sniff_format(b) {
        F::Svg => Err(ImageError::Unsupported("SVG")),
        F::Jxl => Err(ImageError::Unsupported("JXL")),
        F::Avif => Err(ImageError::Unsupported("AVIF")),
        F::Other => Err(ImageError::UnknownFormat),
        f => {
            debug_assert!(f.in_process_fallback());
            Ok(())
        }
    }
}

/// Baştaki BOM/boşluk/XML bildirimi/yorumlar sonrası `<svg` ilk 4 KiB içinde mi?
pub(crate) fn looks_like_svg(b: &[u8]) -> bool {
    let head = &b[..b.len().min(4096)];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(head);
    let start = head.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(head.len());
    head[start..].starts_with(b"<") && head.windows(4).any(|w| w == b"<svg")
}

#[cfg(feature = "jxl")]
fn decode_jxl(bytes: &[u8], opts: &DecodeOptions) -> Result<Image, ImageError> {
    let mut d = jxl_oxide::integration::JxlDecoder::new(Cursor::new(bytes))?;
    let lim = image_limits(&opts.limits);
    // jxl-oxide f32 düzlemlerle çalışır: renk türünün bayt sayısı değil, ölçülmüş tahmin.
    let (w, h) = d.dimensions();
    opts.limits.check_dimensions(w, h)?;
    opts.limits.check_budget((w, h), decode_estimate(d.color_type(), (w, h), Some(crate::JXL_BYTES_PER_PIXEL), opts.target))?;
    let dims = gate(&mut d, opts, &lim)?;
    let img = DynamicImage::from_decoder(d)?;
    verify((img.width(), img.height()), dims, &opts.limits)?;
    from_dynamic(img, opts)
}

#[cfg(not(feature = "jxl"))]
fn decode_jxl(_: &[u8], _: &DecodeOptions) -> Result<Image, ImageError> {
    Err(ImageError::Decode("JPEG XL desteği derlenmedi (`jxl` özelliği)".into()))
}

/// Bayt dizisi animasyonlu bir GIF/WebP/APNG mi. Hiçbir piksel çözülmez: GIF'te blok yapısı
/// yürünerek görüntü tanımlayıcıları sayılır, WebP/APNG'de başlık okunur. Sınır kapısından
/// geçer (dosya boyutu, başlıktaki tuval boyutu); ihlal hatadır, başka biçim `Ok(false)`.
/// Başlığı kesik GIF/PNG `InvalidData`; gövdesi bozuk/kesik dosyada sayılabilen kadar (asıl
/// çözme hatayı ayrıca verir).
pub fn is_animated(bytes: &[u8], limits: &Limits) -> Result<bool, ImageError> {
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit: limits.max_file_bytes });
    }
    match image::guess_format(bytes) {
        Ok(ImageFormat::Gif) => {
            let (w, h) = gif_canvas(bytes).ok_or_else(|| ImageError::InvalidData("GIF başlığı kesik".into()))?;
            limits.check_dimensions(w, h)?;
            Ok(gif_image_count(bytes, 2) >= 2)
        }
        // Animasyon yalnızca genişletilmiş biçimde (VP8X: bayrak + tuval); basit VP8/VP8L durağandır.
        Ok(ImageFormat::WebP) => match webp_vp8x(bytes) {
            Some((animated, w, h)) => {
                limits.check_dimensions(w, h)?;
                Ok(animated)
            }
            None => Ok(false),
        },
        Ok(ImageFormat::Png) => {
            let (w, h) = png_size(bytes).ok_or_else(|| ImageError::InvalidData("PNG başlığı kesik".into()))?;
            limits.check_dimensions(w, h)?;
            Ok(png_has_actl(bytes))
        }
        _ => Ok(false),
    }
}

/// PNG IHDR boyutu (imza + IHDR uzunluk/türü sonrası).
fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(12..16)? != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(b.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(b.get(20..24)?.try_into().ok()?);
    Some((w, h))
}

/// APNG mi: ilk `IDAT`'tan önce `acTL` parçası var mı (yalnızca parça başlıkları yürünür).
fn png_has_actl(b: &[u8]) -> bool {
    let mut i = 8usize;
    while let Some(head) = b.get(i..i + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        match &head[4..8] {
            b"acTL" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        // uzunluk + tür + veri + CRC
        match i.checked_add(12).and_then(|j| j.checked_add(len)) {
            Some(next) => i = next,
            None => return false,
        }
    }
    false
}


/// WebP `VP8X` parçası (RIFF başlığından hemen sonra): animasyon bayrağı ve tuval boyutu.
fn webp_vp8x(b: &[u8]) -> Option<(bool, u32, u32)> {
    if b.get(12..16)? != b"VP8X" {
        return None;
    }
    let flags = *b.get(20)?;
    let u24 = |i: usize| -> Option<u32> { Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, 0]) + 1) };
    Some((flags & 0x02 != 0, u24(24)?, u24(27)?))
}


/// GIF mantıksal ekran boyutu (başlık + ekran tanımlayıcısı).
fn gif_canvas(b: &[u8]) -> Option<(u32, u32)> {
    let w = u16::from_le_bytes([*b.get(6)?, *b.get(7)?]);
    let h = u16::from_le_bytes([*b.get(8)?, *b.get(9)?]);
    Some((u32::from(w), u32::from(h)))
}

/// GIF'teki görüntü tanımlayıcısı sayısı, en çok `max`'a kadar.
fn gif_image_count(b: &[u8], max: usize) -> usize {
    gif_scan(b, max).0
}

/// GIF blok yapısını yürür: görüntü tanımlayıcısı sayısı (en çok `max`) ve gecikmelerinin
/// toplamı (ms, [`effective_delay_ms`] ile). Yalnızca blok yapısı yürünür (ayırma yok, LZW
/// çözülmez, her bayt en fazla bir kez). Kesik/bozuk yapıda o ana kadarki değerler.
fn gif_scan(b: &[u8], max: usize) -> (usize, u64) {
    // Alt bloklar: uzunluk baytı + veri, 0 ile biter. Sonraki konumu döndürür.
    fn skip_sub_blocks(b: &[u8], mut i: usize) -> Option<usize> {
        loop {
            let n = usize::from(*b.get(i)?);
            i += 1;
            if n == 0 {
                return Some(i);
            }
            i += n;
        }
    }
    let table = |flags: u8| if flags & 0x80 != 0 { 3usize << ((flags & 0x07) + 1) } else { 0 };
    let Some(&lsd_flags) = b.get(10) else { return (0, 0) };
    let mut i = 13 + table(lsd_flags);
    let (mut count, mut total_ms) = (0usize, 0u64);
    // Grafik denetim uzantısının gecikmesi (santisaniye) yalnızca sonraki görüntüye uygulanır.
    let mut pending_cs = 0u16;
    while count < max {
        match b.get(i) {
            // Uzantı: etiket + alt bloklar. Grafik denetimi (F9, 4 bayt): bayraklar + gecikme.
            Some(0x21) => {
                if b.get(i + 1) == Some(&0xF9) && b.get(i + 2) == Some(&4) {
                    let (Some(&lo), Some(&hi)) = (b.get(i + 4), b.get(i + 5)) else { break };
                    pending_cs = u16::from_le_bytes([lo, hi]);
                }
                match skip_sub_blocks(b, i + 2) {
                    Some(next) => i = next,
                    None => break,
                }
            }
            // Görüntü tanımlayıcısı: 9 bayt + yerel renk tablosu + LZW kod boyu + alt bloklar.
            Some(0x2C) => {
                let Some(&flags) = b.get(i + 9) else { break };
                count += 1;
                total_ms += effective_delay_ms(u64::from(pending_cs) * 10);
                pending_cs = 0;
                match skip_sub_blocks(b, i + 10 + table(flags) + 1) {
                    Some(next) => i = next,
                    None => break,
                }
            }
            _ => break,
        }
    }
    (count, total_ms)
}

/// Kare gecikmesi kuralı (tarayıcılarla ve FFmpeg'in GIF çözücüsüyle aynı): ≤ 10 ms gecikme
/// 100 ms sayılır (eski GIF'ler 0 yazar). Üst sınır burada yok ([`MAX_FRAME_DELAY`] oynatıcının).
const fn effective_delay_ms(ms: u64) -> u64 {
    if ms <= 10 { 100 } else { ms }
}

/// Animasyonun kare sayısı ve toplam süresi (bir tur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationInfo {
    pub frames: u64,
    pub duration: Duration,
}

/// Animasyonlu GIF/APNG/WebP'nin kare sayısı ve süresi — yalnızca kap yapısı yürünür (piksel
/// çözülmez, ayırma yok, her bayt en fazla bir kez; dosya sınırı içindeki girdide ucuz).
/// Gecikmeler oynatıcının kuralıyla (≤ 10 ms → 100 ms) ama üst sınırsız toplanır: kodlayıcı
/// (duvar kağıdı içe aktarma) kareyi kırpmaz, bütçe gerçek çıktıyı ölçmeli. Animasyon değilse
/// ya da yapı tanınmazsa `None`. Bozuk yapıda o ana kadar sayılan değerler döner (alt sınır).
pub fn animation_info(bytes: &[u8]) -> Option<AnimationInfo> {
    let (frames, ms) = match image::guess_format(bytes).ok()? {
        ImageFormat::Gif => {
            let (n, ms) = gif_scan(bytes, usize::MAX);
            (n as u64, ms)
        }
        ImageFormat::Png => apng_scan(bytes)?,
        ImageFormat::WebP => webp_scan(bytes)?,
        _ => return None,
    };
    (frames >= 2).then(|| AnimationInfo { frames, duration: Duration::from_millis(ms) })
}

/// APNG: `fcTL` parçaları (kare başına bir) ve gecikmeleri (`delay_num/delay_den` sn; payda 0
/// → 100). `acTL` yoksa `None`.
fn apng_scan(b: &[u8]) -> Option<(u64, u64)> {
    let (mut i, mut actl, mut frames, mut ms) = (8usize, false, 0u64, 0u64);
    while let Some(head) = b.get(i..i + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        match &head[4..8] {
            b"acTL" => actl = true,
            b"fcTL" => {
                // Veri: sıra(4) w(4) h(4) x(4) y(4) delay_num(2) delay_den(2) dispose blend.
                let Some(d) = b.get(i + 8 + 20..i + 8 + 24) else { break };
                let num = u64::from(u16::from_be_bytes([d[0], d[1]]));
                let den = match u16::from_be_bytes([d[2], d[3]]) {
                    0 => 100,
                    v => u64::from(v),
                };
                frames += 1;
                ms += effective_delay_ms(num * 1000 / den);
            }
            b"IEND" => break,
            _ => {}
        }
        match i.checked_add(12).and_then(|j| j.checked_add(len)) {
            Some(next) => i = next,
            None => break,
        }
    }
    actl.then_some((frames, ms))
}

/// Animasyonlu WebP: RIFF parçaları içinde `ANMF` (kare başına bir; verinin 12. baytında 24 bit
/// süre, ms). `VP8X` animasyon bayrağı yoksa `None`.
fn webp_scan(b: &[u8]) -> Option<(u64, u64)> {
    if !webp_vp8x(b)?.0 {
        return None;
    }
    let (mut i, mut frames, mut ms) = (12usize, 0u64, 0u64);
    while let Some(head) = b.get(i..i + 8) {
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if &head[..4] == b"ANMF" {
            let Some(d) = b.get(i + 8 + 12..i + 8 + 15) else { break };
            frames += 1;
            ms += effective_delay_ms(u64::from(u32::from_le_bytes([d[0], d[1], d[2], 0])));
        }
        // Parça verisi çift uzunluğa doldurulur.
        match i.checked_add(8).and_then(|j| j.checked_add(len + (len & 1))) {
            Some(next) => i = next,
            None => break,
        }
    }
    Some((frames, ms))
}

/// Bayt dizisi `decode_bytes`'ın tanıdığı bir görsel mi (biçim imzası; çözmeden).
pub fn is_image(bytes: &[u8]) -> bool {
    looks_like_jxl(bytes) || looks_like_svg(bytes) || image::guess_format(bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fit, TargetSize};

    /// Sert kenarlı, gürültülü desen (Lanczos taşmasını ve yuvarlamayı zorlar).
    fn value(x: u32, y: u32, salt: u32) -> u8 {
        let n = x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ salt.wrapping_mul(83_492_791);
        if (x / 5 + y / 3).is_multiple_of(2) { (n % 256) as u8 } else { 255 - ((x * 7 + y) % 256) as u8 }
    }

    fn samples(w: u32, h: u32) -> Vec<(&'static str, DynamicImage)> {
        let alpha = |x: u32, y: u32| if (x + y).is_multiple_of(11) { 0 } else { value(x, y, 9) };
        let wide = |v: u8, x: u32| (u16::from(v) * 256) | (x % 256) as u16;
        vec![
            ("L8", DynamicImage::ImageLuma8(image::GrayImage::from_fn(w, h, |x, y| image::Luma([value(x, y, 1)])))),
            (
                "LA8",
                DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_fn(w, h, |x, y| image::LumaA([value(x, y, 2), alpha(x, y)]))),
            ),
            (
                "L16",
                DynamicImage::ImageLuma16(image::ImageBuffer::from_fn(w, h, |x, y| image::Luma([wide(value(x, y, 3), x)]))),
            ),
            (
                "LA16",
                DynamicImage::ImageLumaA16(image::ImageBuffer::from_fn(w, h, |x, y| {
                    image::LumaA([wide(value(x, y, 4), x), wide(alpha(x, y), y)])
                })),
            ),
            (
                "RGB8",
                DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
                    image::Rgb([value(x, y, 5), value(x, y, 6), value(x, y, 7)])
                })),
            ),
            (
                "RGB16",
                DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(w, h, |x, y| {
                    image::Rgb([wide(value(x, y, 5), x), wide(value(x, y, 6), y), wide(value(x, y, 7), x + y)])
                })),
            ),
            (
                "RGB32F",
                DynamicImage::ImageRgb32F(image::ImageBuffer::from_fn(w, h, |x, y| {
                    image::Rgb([f32::from(value(x, y, 5)) / 255.0, f32::from(value(x, y, 6)) / 255.0, 1.25])
                })),
            ),
        ]
    }

    #[test]
    fn narrow_formats_match_rgba_path() {
        for (w, h) in [(97, 61), (300, 40), (16, 512)] {
            for fit in [Fit::Contain, Fit::Cover, Fit::Fill] {
                let opts = DecodeOptions { target: Some(TargetSize::new(37, 23, fit)), ..DecodeOptions::default() };
                for (name, img) in samples(w, h) {
                    let narrow = from_dynamic(img.clone(), &opts).unwrap();
                    let full = full_rgba(img, &opts).unwrap();
                    assert_eq!(narrow.size(), full.size(), "{name} {w}x{h} {fit:?}");
                    assert!(narrow.pixels() == full.pixels(), "{name} {w}x{h} {fit:?}: pikseller farklı");
                }
            }
        }
    }

    /// Paletli PNG fikstürü: `bits` bit indeks, `entries` girişli palet (gri ya da renkli),
    /// isteğe bağlı tRNS. Desen: indeks köşegen + kare dama (Lanczos taşması da sınanır).
    fn indexed_png(w: u32, h: u32, bits: u8, gray: bool, trns: Option<Vec<u8>>) -> Vec<u8> {
        let entries = 1usize << bits;
        let palette: Vec<u8> = (0..entries)
            .flat_map(|i| {
                let v = (i * 255 / (entries - 1).max(1)) as u8;
                if gray { [v, v, v] } else { [v, 255 - v, (i * 37 % 256) as u8] }
            })
            .collect();
        let row_bytes = (w as usize * usize::from(bits)).div_ceil(8);
        let mut data = vec![0u8; row_bytes * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let idx = ((x / 3 + y / 5) + usize::from((x / 7 + y / 7) % 2 == 0) * entries / 2) % entries;
                let bit = x * usize::from(bits);
                data[y * row_bytes + bit / 8] |= (idx as u8) << (8 - usize::from(bits) - bit % 8);
            }
        }
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::from_u8(bits).unwrap());
        enc.set_palette(palette);
        if let Some(t) = trns {
            enc.set_trns(t);
        }
        enc.write_header().unwrap().write_image_data(&data).unwrap();
        out
    }

    /// Paletli yol (en dar biçim) image'ın genişletme yoluyla piksel piksel aynı: bit
    /// derinlikleri × gri/renkli × tRNS yok/kısa/tam/paletten uzun (yok sayılır) × sığdırma.
    #[test]
    fn indexed_png_matches_expanded_path() {
        let mut checked = 0;
        for bits in [1u8, 2, 4, 8] {
            let n = 1usize << bits;
            let trns_cases = [None, Some(vec![0u8]), Some((0..n).map(|i| (i * 255 / n) as u8).collect()), Some(vec![128; n + 1]), Some(vec![255; n])];
            for gray in [true, false] {
                for trns in trns_cases.clone() {
                    let bytes = indexed_png(61, 43, bits, gray, trns.clone());
                    for fit in [Fit::Contain, Fit::Cover, Fit::Fill] {
                        let opts = DecodeOptions { target: Some(TargetSize::new(19, 11, fit)), ..DecodeOptions::default() };
                        let fast = crate::indexed::decode_png_indexed(&bytes, &opts, false).unwrap();
                        let img = DynamicImage::from_decoder(PngDecoder::new(Cursor::new(&bytes[..])).unwrap()).unwrap();
                        let slow = from_dynamic(img, &opts).unwrap();
                        let name = format!("{bits} bit gri={gray} trns={:?} {fit:?}", trns.as_ref().map(Vec::len));
                        // Renkli palet olağan yoldan (kazanç yok, bkz. indexed.rs).
                        assert_eq!(fast.is_some(), gray, "{name}: yol seçimi");
                        if let Some(fast) = fast {
                            assert_eq!(fast.size(), slow.size(), "{name}");
                            assert!(fast.pixels() == slow.pixels(), "{name}: pikseller farklı");
                        }
                        // Genel giriş de bu yolu kullanır.
                        let Decoded::Still(via) = decode_bytes(&bytes, &opts).unwrap() else { panic!("durağan") };
                        assert!(via.pixels() == slow.pixels(), "{name}: decode_bytes");
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 4 * 2 * 5 * 3);
    }

    /// Ölçüm (elle): önce fikstür, sonra her yol ayrı süreçte (VmHWM süreç başına tepe RSS):
    /// `NUXUT_BENCH=gen|old|new NUXUT_BENCH_KIND=gray|gray-trns|color-trns NUXUT_BENCH_FILE=<yol> cargo
    /// test --release -p nuxut-image --lib bench_indexed_png -- --ignored --nocapture`.
    /// 6000×6000 8 bit paletli PNG → 1920 kenar; `new` genel girişten (`decode_bytes`) geçer.
    #[test]
    #[ignore = "ölçüm; elle çalıştırılır"]
    fn bench_indexed_png() {
        let path = std::env::var("NUXUT_BENCH").unwrap_or_else(|_| "new".into());
        let file = std::env::var("NUXUT_BENCH_FILE").expect("NUXUT_BENCH_FILE");
        if path == "gen" {
            let n = 1usize << 8;
            let kind = std::env::var("NUXUT_BENCH_KIND").unwrap_or_else(|_| "gray".into());
            let ramp = || Some((0..n).map(|i| (i * 255 / n) as u8).collect());
            let (gray, trns) = match kind.as_str() {
                "gray" => (true, None),
                "gray-trns" => (true, ramp()),
                _ => (false, ramp()),
            };
            std::fs::write(&file, indexed_png(6000, 6000, 8, gray, trns)).unwrap();
            return;
        }
        let bytes = std::fs::read(&file).unwrap();
        let hwm = || {
            std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with("VmHWM")).unwrap().to_owned()
        };
        let before = hwm();
        let opts = DecodeOptions { target: Some(TargetSize::new(1920, 1920, Fit::Contain)), ..DecodeOptions::default() };
        let t = std::time::Instant::now();
        let img = if path == "old" {
            let d = DynamicImage::from_decoder(PngDecoder::new(Cursor::new(&bytes[..])).unwrap()).unwrap();
            from_dynamic(d, &opts).unwrap()
        } else {
            let Decoded::Still(img) = decode_bytes(&bytes, &opts).unwrap() else { panic!("durağan") };
            img
        };
        let dt = t.elapsed();
        println!("bench {path}: {:?} {:.1} ms; önce {before}, sonra {}", img.size(), dt.as_secs_f64() * 1e3, hwm());
    }

    #[test]
    fn indexed_png_path_only_when_shrinking_plain_palette() {
        let bytes = indexed_png(40, 30, 8, true, None);
        let small = DecodeOptions { target: Some(TargetSize::new(10, 10, Fit::Contain)), ..DecodeOptions::default() };
        assert!(crate::indexed::decode_png_indexed(&bytes, &small, false).unwrap().is_some());
        // Küçültme yok, EXIF yönlü: olağan yol.
        let none = DecodeOptions { target: None, ..DecodeOptions::default() };
        let big = DecodeOptions { target: Some(TargetSize::new(400, 400, Fit::Contain)), ..DecodeOptions::default() };
        assert!(crate::indexed::decode_png_indexed(&bytes, &none, false).unwrap().is_none());
        assert!(crate::indexed::decode_png_indexed(&bytes, &big, false).unwrap().is_none());
        assert!(crate::indexed::decode_png_indexed(&bytes, &small, true).unwrap().is_none());
        // Paletli olmayan PNG.
        let mut rgb = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::new(40, 30)).write_to(&mut Cursor::new(&mut rgb), ImageFormat::Png).unwrap();
        assert!(crate::indexed::decode_png_indexed(&rgb, &small, false).unwrap().is_none());
        // Bozuk veri: hata, panik yok.
        let mut cut = bytes.clone();
        cut.truncate(cut.len() - 30);
        assert!(crate::indexed::decode_png_indexed(&cut, &small, false).is_err());
    }

    #[test]
    fn narrow_formats_without_resize_are_untouched() {
        // Hedef yoksa ya da görsel zaten küçükse tam RGBA yolu (küçültme yok).
        for target in [None, Some(TargetSize::new(4096, 4096, Fit::Cover))] {
            let opts = DecodeOptions { target, ..DecodeOptions::default() };
            for (name, img) in samples(20, 10) {
                let out = from_dynamic(img.clone(), &opts).unwrap();
                assert_eq!(out, full_rgba(img, &opts).unwrap(), "{name}");
                assert_eq!(out.size(), (20, 10));
            }
        }
    }
}
