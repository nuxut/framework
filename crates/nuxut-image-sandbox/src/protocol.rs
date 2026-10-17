//! İşçi ↔ ebeveyn tel biçimi (ADR-0007 §Protokol). Tüm tamsayılar küçük sonlu (LE).
//!
//! - El sıkışma (işçi → ebeveyn, kısıtlamalar KURULDUKTAN sonra): [`Handshake`] — `MAGIC | u16
//!   sürüm | u8 bayraklar (seccomp, test kancaları) | u8 landlock düzeyi | u16 miras fd sayısı |
//!   u8 profil | u8 n | n bayt crate sürümü`. Ebeveyn kilit bayraklarını ve profili
//!   [`Handshake::accept`] ile zorunlu tutar.
//! - İstek (ebeveyn → işçi): [`REQUEST_HEADER_LEN`] baytlık başlık + `payload_len` bayt girdi
//!   (en fazla işçi profilinin `max_input_bytes`'ı).
//! - Yanıt (işçi → ebeveyn): [`RESPONSE_HEADER_LEN`] baytlık başlık — `u8 durum | u8 hata | u16
//!   bayraklar | u32 w | u32 h | u32 kare | u32 süre_ms` (v4: animasyon bilgisi); durum `OK` ise ardından
//!   tam `w·h·4` bayt premultiplied RGBA.
//!
//! Ebeveyn işçiyi **ele geçirilmiş** sayar: yanıt başlığı [`Response::validate`] ile isteğin üst
//! sınırına göre denetlenir, piksel tamponu yalnızca denetimden sonra ayrılır.

use nuxut_image::{Fit, ImageError, SniffedFormat, TargetSize};

use crate::WorkerProfile;

/// El sıkışma imzası.
pub const MAGIC: [u8; 4] = *b"NXIW";
/// Tel biçimi sürümü; değişen her ayrıntıda artar. 2: el sıkışmada kilit bayrakları. 3: işçi
/// profili (el sıkışma), istek bayrakları (`CROP`) ve çözücü bütçesi, yanıt bayrakları
/// (`ANIMATED_SOURCE`). 4: yanıtta animasyon bilgisi (kare sayısı, bir tur süresi; işçide kap
/// yapısından — `nuxut-video` içe aktarma bütçesi ana süreçte kap yürümesin).
pub const PROTOCOL_VERSION: u16 = 4;
/// El sıkışmadaki crate sürümü (ikili ile kütüphane aynı derlemeden mi).
pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");
/// İkon profilinin girdi sınırı (`MAX_SVG_BYTES`); geriye uyum adı. Profil başına sınır
/// [`crate::ProfileCaps::max_input_bytes`].
pub const MAX_INPUT_BYTES: u32 = 8 << 20;
/// El sıkışmadaki sürüm dizesinin üst sınırı.
pub const MAX_VERSION_LEN: usize = 32;

pub const REQUEST_HEADER_LEN: usize = 48;
/// `u8 durum | u8 hata | u16 bayraklar | u32 w | u32 h | u32 kare | u32 süre_ms`.
pub const RESPONSE_HEADER_LEN: usize = 20;

/// El sıkışma bayrağı: seccomp filtresi kuruldu.
pub const FLAG_SECCOMP: u8 = 1;
/// El sıkışma bayrağı: işçi test kancalarıyla derlendi (`sandbox-test-hooks`).
pub const FLAG_TEST_HOOKS: u8 = 2;

/// İstek bayrağı: `Cover` hedefe ortadan kırp ([`TargetSize::crop`]); yalnızca `Cover` ile.
pub const REQ_CROP: u8 = 1;
/// Yanıt bayrağı: kaynak animasyonluydu, ilk kare verildi.
pub const RESP_ANIMATED_SOURCE: u16 = 1;

/// Landlock düzeyi: kısmi (eski çekirdek, bazı haklar yok) ya da tam.
pub const LANDLOCK_PARTIAL: u8 = 1;
pub const LANDLOCK_FULL: u8 = 2;

/// Sabit el sıkışma başlığı: `MAGIC | u16 sürüm | u8 bayraklar | u8 landlock | u16 miras fd |
/// u8 profil | u8 n`, ardından `n` bayt crate sürümü.
pub const HANDSHAKE_HEADER_LEN: usize = 12;

/// İşçinin kendi kilidi hakkındaki beyanı (kilit kurulduktan SONRA yazılır).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handshake {
    pub flags: u8,
    pub landlock: u8,
    /// Kilitten önce açık olan, stdin/stdout/stderr dışındaki fd sayısı (yalnızca sayı).
    pub inherited_fds: u16,
    /// İşçinin uyguladığı profil ([`WorkerProfile::code`]).
    pub profile: u8,
}

/// El sıkışmanın reddedilme nedeni (log için; içerik yok).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeReject {
    NoSeccomp,
    NoLandlock,
    /// Kancalı (test) işçi, kancasız ebeveyne bağlanmaya çalıştı.
    TestHooks,
    /// İşçinin beyan ettiği profil ebeveyninkiyle aynı değil.
    Profile,
}

impl Handshake {
    pub fn encode(&self) -> Vec<u8> {
        let v = CRATE_VERSION.as_bytes();
        let n = v.len().min(MAX_VERSION_LEN);
        let mut out = Vec::with_capacity(HANDSHAKE_HEADER_LEN + n);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        out.push(self.flags);
        out.push(self.landlock);
        out.extend_from_slice(&self.inherited_fds.to_le_bytes());
        out.push(self.profile);
        out.push(n as u8);
        out.extend_from_slice(&v[..n]);
        out
    }

    /// Sabit başlığı çözer: imza ve sürüm tutmazsa `None`. Dönen `usize` ardından gelecek
    /// sürüm dizesinin uzunluğudur (≤ [`MAX_VERSION_LEN`]).
    pub fn decode_header(b: &[u8; HANDSHAKE_HEADER_LEN]) -> Option<(Self, usize)> {
        let version = u16::from_le_bytes([b[4], b[5]]);
        let n = b[11] as usize;
        if b[..4] != MAGIC || version != PROTOCOL_VERSION || n > MAX_VERSION_LEN {
            return None;
        }
        Some((Self { flags: b[6], landlock: b[7], inherited_fds: u16::from_le_bytes([b[8], b[9]]), profile: b[10] }, n))
    }

    /// Ebeveynin kabul kuralı (bulgu 2a): seccomp zorunlu, landlock en az kısmi, profil
    /// ebeveyninkiyle aynı; kancalı işçi yalnızca kendisi de kancalı derlenmiş ebeveyne
    /// bağlanabilir. `allow_unconfined` yalnızca kancalı test ebeveyninde, landlock katmanını
    /// tek başına sınamak içindir.
    pub fn accept(&self, profile: WorkerProfile, parent_has_hooks: bool, allow_unconfined: bool) -> Result<(), HandshakeReject> {
        if self.flags & FLAG_TEST_HOOKS != 0 && !parent_has_hooks {
            return Err(HandshakeReject::TestHooks);
        }
        if self.profile != profile.code() {
            return Err(HandshakeReject::Profile);
        }
        let unconfined_ok = allow_unconfined && parent_has_hooks;
        if self.flags & FLAG_SECCOMP == 0 && !unconfined_ok {
            return Err(HandshakeReject::NoSeccomp);
        }
        if self.landlock == 0 {
            return Err(HandshakeReject::NoLandlock);
        }
        Ok(())
    }
}

/// İstek başlığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub payload_len: u32,
    pub kind: SniffedFormat,
    /// `crop` alanı tel üzerinde [`REQ_CROP`] bayrağıdır.
    pub target: Option<TargetSize>,
    /// Test kancası (`sandbox-test-hooks`); dağıtılan işçi 0 dışını protokol ihlali sayar.
    pub hook: u8,
    pub max_width: u32,
    pub max_height: u32,
    /// Kaynak (çözülmüş tek kare) piksel sınırı.
    pub max_pixels: u64,
    /// Yanıt (çıktı) piksel sınırı: ebeveynin ayıracağı en büyük tampon / 4.
    pub max_out_pixels: u64,
    /// Çözücü bellek bütçesi ([`nuxut_image::Limits::max_decode_bytes`]).
    pub max_decode_bytes: u64,
}

fn fit_code(target: Option<TargetSize>) -> u8 {
    match target.map(|t| t.fit) {
        None => 0,
        Some(Fit::Cover) => 1,
        Some(Fit::Contain) => 2,
        Some(Fit::Fill) => 3,
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn le64(b: &[u8], at: usize) -> u64 {
    let mut x = [0u8; 8];
    x.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(x)
}

impl Request {
    pub fn encode(&self) -> [u8; REQUEST_HEADER_LEN] {
        let mut b = [0u8; REQUEST_HEADER_LEN];
        b[0..4].copy_from_slice(&self.payload_len.to_le_bytes());
        b[4] = self.kind.code();
        b[5] = fit_code(self.target);
        b[6] = self.hook;
        b[7] = if self.target.is_some_and(|t| t.crops()) { REQ_CROP } else { 0 };
        let (tw, th) = self.target.map_or((0, 0), |t| (t.width, t.height));
        b[8..12].copy_from_slice(&tw.to_le_bytes());
        b[12..16].copy_from_slice(&th.to_le_bytes());
        b[16..20].copy_from_slice(&self.max_width.to_le_bytes());
        b[20..24].copy_from_slice(&self.max_height.to_le_bytes());
        b[24..32].copy_from_slice(&self.max_pixels.to_le_bytes());
        b[32..40].copy_from_slice(&self.max_out_pixels.to_le_bytes());
        b[40..48].copy_from_slice(&self.max_decode_bytes.to_le_bytes());
        b
    }

    /// Başlığı çözer; herhangi bir alan geçersizse `None` (işçi protokol ihlaliyle çıkar).
    /// `max_input`: işçi profilinin girdi sınırı.
    pub fn decode(b: &[u8; REQUEST_HEADER_LEN], max_input: u32) -> Option<Self> {
        let payload_len = le32(b, 0);
        let kind = SniffedFormat::from_code(b[4])?;
        let flags = b[7];
        if payload_len > max_input || flags & !REQ_CROP != 0 {
            return None;
        }
        let (tw, th) = (le32(b, 8), le32(b, 12));
        let fit = match b[5] {
            0 => None,
            1 => Some(Fit::Cover),
            2 => Some(Fit::Contain),
            3 => Some(Fit::Fill),
            _ => return None,
        };
        let crop = flags & REQ_CROP != 0;
        let target = match fit {
            None if tw == 0 && th == 0 && !crop => None,
            Some(fit) if tw > 0 && th > 0 && (!crop || fit == Fit::Cover) => Some(TargetSize { crop, ..TargetSize::new(tw, th, fit) }),
            _ => return None,
        };
        Some(Self {
            payload_len,
            kind,
            target,
            hook: b[6],
            max_width: le32(b, 16),
            max_height: le32(b, 20),
            max_pixels: le64(b, 24),
            max_out_pixels: le64(b, 32),
            max_decode_bytes: le64(b, 40),
        })
    }
}

/// Test kancası sonuçlarının ebeveyndeki `ImageError::Unsupported` iletileri.
pub const HOOK_DENIED: &str = "test_hook_denied";
pub const HOOK_ALLOWED: &str = "test_hook_allowed";

/// Yanıt durumu.
pub const STATUS_OK: u8 = 0;
pub const STATUS_ERROR: u8 = 1;

/// Çözme hatası sınıfı (metin taşımaz).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    Empty = 1,
    UnknownFormat = 2,
    /// `w`/`h` alanları kaynak boyutunu taşır.
    TooLarge = 3,
    FileTooLarge = 4,
    Decode = 5,
    Svg = 6,
    Unsupported = 7,
    Resize = 8,
    InvalidData = 9,
    /// Test kancası: işçi içinden denenen erişim (dosya/ağ/süreç) çekirdekçe reddedildi.
    HookDenied = 10,
    /// Test kancası: erişim BAŞARILI oldu — yalıtım delik.
    HookAllowed = 11,
}

impl ErrorCode {
    pub fn from_code(c: u8) -> Option<Self> {
        Some(match c {
            1 => Self::Empty,
            2 => Self::UnknownFormat,
            3 => Self::TooLarge,
            4 => Self::FileTooLarge,
            5 => Self::Decode,
            6 => Self::Svg,
            7 => Self::Unsupported,
            8 => Self::Resize,
            9 => Self::InvalidData,
            10 => Self::HookDenied,
            11 => Self::HookAllowed,
            _ => return None,
        })
    }

    /// İşçinin içindeki çözme hatasını sınıfa indirger (ileti metni süreç sınırını geçmez).
    pub fn from_error(e: &ImageError) -> (Self, u32, u32) {
        match e {
            ImageError::Empty => (Self::Empty, 0, 0),
            ImageError::UnknownFormat => (Self::UnknownFormat, 0, 0),
            ImageError::TooLarge { width, height } => (Self::TooLarge, *width, *height),
            ImageError::FileTooLarge { .. } => (Self::FileTooLarge, 0, 0),
            ImageError::Svg(_) => (Self::Svg, 0, 0),
            ImageError::Unsupported(_) => (Self::Unsupported, 0, 0),
            ImageError::Resize(_) => (Self::Resize, 0, 0),
            ImageError::InvalidData(_) => (Self::InvalidData, 0, 0),
            _ => (Self::Decode, 0, 0),
        }
    }

    /// Ebeveyn tarafında `ImageError`'a dönüşüm. `input_len` FileTooLarge için.
    pub fn into_error(self, w: u32, h: u32, input_len: u64, limit: u64) -> ImageError {
        match self {
            Self::Empty => ImageError::Empty,
            Self::UnknownFormat => ImageError::UnknownFormat,
            Self::TooLarge => ImageError::TooLarge { width: w, height: h },
            Self::FileTooLarge => ImageError::FileTooLarge { size: input_len, limit },
            Self::Svg => ImageError::Svg("yalıtılmış işçide SVG hatası".into()),
            Self::Unsupported => ImageError::Unsupported("işçide desteklenmeyen biçim"),
            Self::Resize => ImageError::Resize("yalıtılmış işçide küçültme hatası".into()),
            Self::InvalidData => ImageError::InvalidData("yalıtılmış işçide geçersiz veri".into()),
            Self::Decode => ImageError::Decode("yalıtılmış işçide çözme hatası".into()),
            Self::HookDenied => ImageError::Unsupported(HOOK_DENIED),
            Self::HookAllowed => ImageError::Unsupported(HOOK_ALLOWED),
        }
    }
}

/// İşçi yanıtı sözleşmeye uymuyor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolViolation;

/// Yanıt başlığı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response {
    pub status: u8,
    pub error: u8,
    /// Yanıt bayrakları ([`RESP_ANIMATED_SOURCE`]); hata yanıtında 0.
    pub flags: u16,
    pub width: u32,
    pub height: u32,
    /// Animasyonlu kaynağın kare sayısı (yalnızca `ANIMATED_SOURCE` ile; 0 = bilinmiyor, aksi
    /// hâlde ≥ 2). `u32`'ye doyurulur.
    pub frames: u32,
    /// Bir tur süresi, ms (`frames` 0 ise 0). `u32`'ye doyurulur (~49 gün).
    pub duration_ms: u32,
}

impl Response {
    pub fn ok(width: u32, height: u32) -> Self {
        Self { status: STATUS_OK, error: 0, flags: 0, width, height, frames: 0, duration_ms: 0 }
    }

    pub fn with_flags(self, flags: u16) -> Self {
        Self { flags, ..self }
    }

    /// Animasyonlu kaynak: bayrak + (biliniyorsa) kare sayısı ve süre.
    pub fn with_animation(self, info: Option<nuxut_image::AnimationInfo>) -> Self {
        let (frames, duration_ms) = info.map_or((0, 0), |i| {
            (u32::try_from(i.frames).unwrap_or(u32::MAX), u32::try_from(i.duration.as_millis()).unwrap_or(u32::MAX))
        });
        Self { flags: self.flags | RESP_ANIMATED_SOURCE, frames, duration_ms, ..self }
    }

    pub fn error(code: ErrorCode, w: u32, h: u32) -> Self {
        Self { status: STATUS_ERROR, error: code as u8, flags: 0, width: w, height: h, frames: 0, duration_ms: 0 }
    }

    pub fn encode(&self) -> [u8; RESPONSE_HEADER_LEN] {
        let mut b = [0u8; RESPONSE_HEADER_LEN];
        b[0] = self.status;
        b[1] = self.error;
        b[2..4].copy_from_slice(&self.flags.to_le_bytes());
        b[4..8].copy_from_slice(&self.width.to_le_bytes());
        b[8..12].copy_from_slice(&self.height.to_le_bytes());
        b[12..16].copy_from_slice(&self.frames.to_le_bytes());
        b[16..20].copy_from_slice(&self.duration_ms.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; RESPONSE_HEADER_LEN]) -> Self {
        Self {
            status: b[0],
            error: b[1],
            flags: u16::from_le_bytes([b[2], b[3]]),
            width: le32(b, 4),
            height: le32(b, 8),
            frames: le32(b, 12),
            duration_ms: le32(b, 16),
        }
    }

    /// Kaynak animasyonluydu (ilk kare verildi).
    pub fn animated_source(&self) -> bool {
        self.flags & RESP_ANIMATED_SOURCE != 0
    }

    /// İşçinin beyan ettiği animasyon bilgisi ([`validate`](Self::validate) sonrası anlamlı).
    pub fn animation(&self) -> Option<nuxut_image::AnimationInfo> {
        (self.animated_source() && self.frames >= 2).then(|| nuxut_image::AnimationInfo {
            frames: u64::from(self.frames),
            duration: std::time::Duration::from_millis(u64::from(self.duration_ms)),
        })
    }

    /// Animasyon alanları tutarlı mı: yalnızca `ANIMATED_SOURCE` ile dolu; kare sayısı 0
    /// (bilinmiyor, süre de 0) ya da ≥ 2.
    fn animation_fields_ok(&self) -> bool {
        if !self.animated_source() || self.frames == 0 {
            return self.frames == 0 && self.duration_ms == 0;
        }
        self.frames >= 2
    }

    /// Ele geçirilmiş işçiye karşı denetim. `Ok(Some(n))`: ardından tam `n` bayt piksel gelir;
    /// `Ok(None)`: hata yanıtı (pikselsiz); `Err`: protokol ihlali.
    pub fn validate(&self, req: &Request) -> Result<Option<usize>, ProtocolViolation> {
        match self.status {
            STATUS_ERROR => {
                if ErrorCode::from_code(self.error).is_none() || self.flags != 0 || self.frames != 0 || self.duration_ms != 0 {
                    return Err(ProtocolViolation);
                }
                Ok(None)
            }
            STATUS_OK => {
                let (w, h) = (self.width, self.height);
                if self.error != 0
                    || self.flags & !RESP_ANIMATED_SOURCE != 0
                    || !self.animation_fields_ok()
                    || w == 0
                    || h == 0
                    || w > req.max_width
                    || h > req.max_height
                {
                    return Err(ProtocolViolation);
                }
                let px = u64::from(w) * u64::from(h);
                if px > req.max_pixels.min(req.max_out_pixels) {
                    return Err(ProtocolViolation);
                }
                if let Some(t) = req.target {
                    let fits = match t.fit {
                        Fit::Contain | Fit::Fill => w <= t.width && h <= t.height,
                        // Kırpılmış Cover: iki eksen de kutuda.
                        Fit::Cover if t.crops() => w <= t.width && h <= t.height,
                        // Cover kutuyu kaplar: bir eksen kutuda, diğeri taşabilir.
                        Fit::Cover => w <= t.width || h <= t.height,
                    };
                    if !fits {
                        return Err(ProtocolViolation);
                    }
                }
                usize::try_from(px * 4).map(Some).map_err(|_| ProtocolViolation)
            }
            _ => Err(ProtocolViolation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(target: Option<TargetSize>) -> Request {
        Request {
            payload_len: 100,
            kind: SniffedFormat::Svg,
            target,
            hook: 0,
            max_width: 4096,
            max_height: 4096,
            max_pixels: 16_000_000,
            max_out_pixels: 1 << 20,
            max_decode_bytes: 1 << 27,
        }
    }

    #[test]
    fn request_roundtrip_and_rejects_garbage() {
        for t in [None, Some(TargetSize::new(64, 32, Fit::Cover)), Some(TargetSize::new(5, 7, Fit::Fill)), Some(TargetSize::cover_crop(640, 480))] {
            let r = req(t);
            assert_eq!(Request::decode(&r.encode(), MAX_INPUT_BYTES), Some(r));
        }
        let mut b = req(None).encode();
        b[0..4].copy_from_slice(&(MAX_INPUT_BYTES + 1).to_le_bytes());
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "girdi sınırı");
        let wp = WorkerProfile::Wallpaper.caps().max_input_bytes;
        assert!(Request::decode(&b, wp).is_some(), "duvar kağıdı profili daha büyük girdi kabul eder");
        b[0..4].copy_from_slice(&(wp + 1).to_le_bytes());
        assert_eq!(Request::decode(&b, wp), None, "profil sınırı");
        let mut b = req(None).encode();
        b[4] = 200;
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "tür");
        let mut b = req(None).encode();
        b[5] = 9;
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "fit");
        let mut b = req(None).encode();
        b[7] = 2;
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "tanımsız istek bayrağı");
        let mut b = req(None).encode();
        b[7] = REQ_CROP;
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "hedefsiz kırpma");
        let mut b = req(Some(TargetSize::new(4, 4, Fit::Contain))).encode();
        b[7] = REQ_CROP;
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "Contain'de kırpma");
        let mut b = req(Some(TargetSize::new(4, 4, Fit::Contain))).encode();
        b[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(Request::decode(&b, MAX_INPUT_BYTES), None, "sıfır hedef");
        // `crop` yalnızca Cover'da tele çıkar.
        let contain_crop = TargetSize { crop: true, ..TargetSize::new(8, 8, Fit::Contain) };
        assert_eq!(req(Some(contain_crop)).encode()[7], 0);
    }

    #[test]
    fn response_validation_treats_worker_as_hostile() {
        let r = req(Some(TargetSize::new(64, 64, Fit::Contain)));
        assert_eq!(Response::ok(64, 32).validate(&r), Ok(Some(64 * 32 * 4)));
        assert_eq!(Response::ok(65, 32).validate(&r), Err(ProtocolViolation), "w > tw");
        assert_eq!(Response::ok(0, 32).validate(&r), Err(ProtocolViolation), "boş");
        assert_eq!(Response::ok(u32::MAX, u32::MAX).validate(&req(None)), Err(ProtocolViolation), "4 GiB beyan");
        assert_eq!(Response::ok(2048, 1024).validate(&req(None)), Err(ProtocolViolation), "çıktı bütçesi");
        let cover = req(Some(TargetSize::new(64, 64, Fit::Cover)));
        assert_eq!(Response::ok(128, 64).validate(&cover), Ok(Some(128 * 64 * 4)));
        assert_eq!(Response::ok(128, 65).validate(&cover), Err(ProtocolViolation));
        // Kırpılmış Cover: taşan eksen ihlal.
        let crop = req(Some(TargetSize::cover_crop(64, 64)));
        assert_eq!(Response::ok(64, 64).validate(&crop), Ok(Some(64 * 64 * 4)));
        assert_eq!(Response::ok(40, 64).validate(&crop), Ok(Some(40 * 64 * 4)), "küçük kaynak");
        assert_eq!(Response::ok(128, 64).validate(&crop), Err(ProtocolViolation), "taşan genişlik");
        assert_eq!(Response::ok(64, 65).validate(&crop), Err(ProtocolViolation), "taşan yükseklik");
        // Bayraklar.
        assert_eq!(Response::ok(8, 8).with_flags(RESP_ANIMATED_SOURCE).validate(&r), Ok(Some(256)));
        assert_eq!(Response::ok(8, 8).with_flags(2).validate(&r), Err(ProtocolViolation), "tanımsız bayrak");
        assert_eq!(Response::error(ErrorCode::Svg, 0, 0).with_flags(RESP_ANIMATED_SOURCE).validate(&r), Err(ProtocolViolation), "hatada bayrak");
        assert_eq!(Response::error(ErrorCode::Svg, 0, 0).validate(&r), Ok(None));
        assert_eq!(Response { status: STATUS_ERROR, error: 99, ..Response::ok(0, 0) }.validate(&r), Err(ProtocolViolation));
        assert_eq!(Response { status: 7, ..Response::ok(1, 1) }.validate(&r), Err(ProtocolViolation));
        let h = Response::ok(3, 4).with_flags(RESP_ANIMATED_SOURCE);
        assert_eq!(Response::decode(&h.encode()), h);
        assert!(Response::decode(&h.encode()).animated_source());
        assert_eq!(h.animation(), None, "kare sayısı bilinmiyor");
    }

    /// Tel v4: animasyon bilgisi yalnızca `ANIMATED_SOURCE` ile ve tutarlıysa kabul edilir.
    #[test]
    fn animation_fields_are_validated() {
        use nuxut_image::AnimationInfo;
        use std::time::Duration;
        let r = req(Some(TargetSize::new(64, 64, Fit::Contain)));
        let info = AnimationInfo { frames: 12, duration: Duration::from_millis(960) };
        let a = Response::ok(8, 8).with_animation(Some(info));
        assert_eq!(a.validate(&r), Ok(Some(256)));
        let back = Response::decode(&a.encode());
        assert_eq!(back, a);
        assert_eq!(back.animation(), Some(info));
        // Doyurma: u32'yi aşan değerler panik vermez.
        let huge = Response::ok(8, 8).with_animation(Some(AnimationInfo { frames: u64::MAX, duration: Duration::MAX }));
        assert_eq!((huge.frames, huge.duration_ms), (u32::MAX, u32::MAX));
        // Bayraksız alan, tek kare, kare sayısız süre, hata yanıtında alan: ihlal.
        assert_eq!(Response { frames: 12, ..Response::ok(8, 8) }.validate(&r), Err(ProtocolViolation));
        assert_eq!(Response { duration_ms: 5, ..Response::ok(8, 8) }.validate(&r), Err(ProtocolViolation));
        assert_eq!(Response { frames: 1, ..Response::ok(8, 8).with_flags(RESP_ANIMATED_SOURCE) }.validate(&r), Err(ProtocolViolation));
        assert_eq!(Response { duration_ms: 5, ..Response::ok(8, 8).with_flags(RESP_ANIMATED_SOURCE) }.validate(&r), Err(ProtocolViolation));
        assert_eq!(Response { frames: 3, ..Response::error(ErrorCode::Svg, 0, 0) }.validate(&r), Err(ProtocolViolation));
    }

    #[test]
    fn handshake_roundtrip_and_is_bounded() {
        let hs = Handshake { flags: FLAG_SECCOMP, landlock: LANDLOCK_FULL, inherited_fds: 3, profile: WorkerProfile::Wallpaper.code() };
        let b = hs.encode();
        assert!(b.len() <= HANDSHAKE_HEADER_LEN + MAX_VERSION_LEN);
        let head: [u8; HANDSHAKE_HEADER_LEN] = b[..HANDSHAKE_HEADER_LEN].try_into().unwrap();
        let (got, n) = Handshake::decode_header(&head).unwrap();
        assert_eq!((got, n), (hs, b.len() - HANDSHAKE_HEADER_LEN));
        let mut bad = head;
        bad[0] ^= 1;
        assert!(Handshake::decode_header(&bad).is_none(), "imza");
        let mut old = head;
        old[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert!(Handshake::decode_header(&old).is_none(), "eski sürüm");
    }

    /// Bulgu 2a: kilitsiz ya da kancalı işçi üretime giremez; profil uyuşmalı.
    #[test]
    fn parent_rejects_unconfined_or_test_workers() {
        let icon = WorkerProfile::Icon;
        let ok = Handshake { flags: FLAG_SECCOMP, landlock: LANDLOCK_PARTIAL, inherited_fds: 0, profile: icon.code() };
        assert_eq!(ok.accept(icon, false, false), Ok(()));
        assert_eq!(ok.accept(WorkerProfile::Wallpaper, false, false), Err(HandshakeReject::Profile), "profil uyuşmazlığı");
        assert_eq!(Handshake { profile: 0, ..ok }.accept(icon, false, false), Err(HandshakeReject::Profile));
        let no_sc = Handshake { flags: 0, ..ok };
        assert_eq!(no_sc.accept(icon, false, false), Err(HandshakeReject::NoSeccomp));
        assert_eq!(no_sc.accept(icon, false, true), Err(HandshakeReject::NoSeccomp), "kancasız ebeveyn izni yok sayar");
        assert_eq!(no_sc.accept(icon, true, true), Ok(()), "yalnızca kancalı test ebeveyni");
        assert_eq!(Handshake { landlock: 0, ..ok }.accept(icon, true, true), Err(HandshakeReject::NoLandlock));
        let hooked = Handshake { flags: FLAG_SECCOMP | FLAG_TEST_HOOKS, ..ok };
        assert_eq!(hooked.accept(icon, false, false), Err(HandshakeReject::TestHooks), "üretim ebeveyni kancalı işçiyi reddeder");
        assert_eq!(hooked.accept(icon, true, false), Ok(()));
    }
}
