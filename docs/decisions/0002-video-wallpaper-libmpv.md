# ADR-0002 — Video duvar kağıdı: libmpv render API'si

Tarih: 2026-10-04 · Durum: kabul (bağlama değişikliği aşağıda; uygulama `nuxut-video`)

## Bağlam
Kullanıcı duvar kağıdı olarak görsel, GIF ve **video** istiyor; videoda ses aç/kapat olmalı.
Görsel/GIF yolu saf Rust (`nuxut-image`: `image`, `fast_image_resize`, `resvg`). Video için
aynı ilke ("önce saf Rust") değerlendirildi:

| Seçenek | Çözme | Donanım hızlandırma | Kapsam | Değerlendirme |
|---|---|---|---|---|
| Saf Rust (`rav1d`, `openh264`-rs portları, `symphonia` ses) | yalnızca AV1 (rav1d), H.264 C'ye bağlı | yok | kap (mp4/mkv/webm) ayrıştırma + A/V senkronu + ses çıkışı bizim | Üretimde yetersiz: H.264/HEVC/VP9 yok, 4K'da CPU ile çözme bir çekirdeği doldurur |
| GStreamer (`gstreamer-rs`) | tüm kodekler | VA-API | büyük eklenti ağacı, GL paylaşımı karmaşık | Ağır; çalışma zamanı eklenti eksikliği tanısı zor |
| FFmpeg (`ffmpeg-next`) | tüm kodekler | VA-API (elle) | demux/decode var; ses çıkışı, senkron, döngü, hwdec→GL köprüsü bizim | Çok iş, ince hata yüzeyi |
| **libmpv** (`libmpv2` crate) | mpv'nin tümü | `hwdec=auto-safe` (VA-API/VDPAU/NVDEC) | demux, decode, A/V senkron, ses (PipeWire/Pulse), döngü, render API ile **bizim GL context'imize** çizim | **Seçilen** |

mpvpaper, Hidamari, Caelestia/Quickshell video duvar kağıtları da libmpv kullanır.

## Karar
1. Video duvar kağıdı libmpv'nin **render API**'siyle (`mpv_render_context`, `MPV_RENDER_API_TYPE_OPENGL`)
   yapılır; Rust bağlaması `libmpv2` (bindgen tabanlı `libmpv2-sys`). mpv kendi penceresini
   açmaz (`vo=libmpv`); kareleri bizim EGL context'imizde bir FBO'ya çizer.
2. Kod `nuxut-image` içinde `video` cargo özelliği arkasındadır (varsayılan kapalı); özellik
   kapalıyken libmpv'ye link yoktur. Çalışma zamanında libmpv yoksa (`dlopen` başarısız ya da
   özellik kapalı) duvar kağıdı ilk kareye/statik görsele düşer ve uyarı loglanır — shell çökmez.
3. Kare akışı runtime'ın "boşta sıfır iş" değişmezine uyar: mpv'nin `update_callback`'i
   (herhangi bir thread'den) `MessageSender` ile döngüyü uyandırır; uygulama yalnızca o zaman
   `request_frame` çağırır. Yüzey görünmezken (ör. tam ekran pencere) `pause=yes`.
4. Ses: varsayılan **kapalı** (`mute=yes`, `audio=no` değil — açılabilsin diye); `Video::set_muted`
   ve `set_volume` mpv özelliklerine yazar.
5. Seçenekler: `hwdec=auto-safe`, `loop-file=inf`, `keep-open=yes`, `video-sync=display-resample`
   yok (compositor zamanlaması bizde), `panscan=1.0` (cover) / `0.0` (contain).

## API taslağı (`nuxut_image::video`, `video` özelliği)

İskelet (`crates/nuxut-image/src/video.rs`) bugün derlenir ve her çağrıda
`VideoError::Unavailable` döner; aşağıdaki tam API libmpv bağlanınca doldurulur.
```rust
pub struct Video;                                   // mpv handle + render context
impl Video {
    /// GL context güncelken (draw içinde ilk kare) yaratılır; get_proc_address EGL'den.
    pub fn new(get_proc_address: impl Fn(&str) -> *const c_void, wake: impl Fn() + Send + 'static)
        -> Result<Self, VideoError>;
    pub fn load(&mut self, path: &Path) -> Result<(), VideoError>;
    pub fn set_paused(&mut self, paused: bool);
    pub fn set_muted(&mut self, muted: bool);
    pub fn set_volume(&mut self, percent: f64);
    pub fn set_fit(&mut self, fit: Fit);
    /// Yeni kare varsa `fbo`'ya (fiziksel `w`×`h`) çizer; yoksa false.
    pub fn render(&mut self, fbo: u32, w: i32, h: i32) -> bool;
}
```
Render tarafı: `Video` bir FBO+dokuya çizer; doku `nuxut-render`'in dış doku desteğiyle
(`Texture::from_gl` — gerektiğinde ayrıca tasarlanacak) `Painter::image`
ile çizilir; böylece yuvarlatılmış köşe, opaklık ve çapraz geçiş görselle aynı yoldan gelir.

## Sonuçlar
- Kullanıcı `mpv` paketini kurmalı (`pacman -S mpv`; `libmpv.so` ve `pkg-config mpv` gelir).
  Şu an sistemde yok; derleme `video` özelliği kapalıyken etkilenmez.
- `unsafe` gerekir (mpv render API ham pointer'lar). ARCHITECTURE'a göre `unsafe` yalnızca
  `nuxut-render`/`nuxut-wayland`'da serbest: `libmpv2` güvenli sarmalayıcıyı sağlar; bizim
  tarafta kalan tek `unsafe` (`get_proc_address` köprüsü) uygulanırken onayla
  ya `nuxut-render`'e taşınır ya da `nuxut-image`'e açık istisna olarak yazılır.
- Bellek: mpv + hwdec ile 4K video ~60–120 MB ek RSS (mpvpaper ölçümleri); "monitör başına
  onlarca MB" hedefini aşar — video duvar kağıdı açık bir kullanıcı tercihidir, varsayılan değil.

## Güncelleme (2026-10-04): bağlama
`libmpv2` crate'i LGPL-2.1 (deny.toml izin vermiyor; statik derlenen Rust kodu). Bunun yerine
`nuxut-video` kendi küçük bağlamasını yazar ve `libmpv.so.2`'yi çalışma zamanında
`libloading` ile yükler: derleme mpv gerektirmez, mpv'siz kurulum statik duvar kağıdına
düşer. Video ayrı crate'tir (`nuxut-image` `video` iskeleti kaldırılır). Dönüştürme de
libmpv kodlama kipiyle yapılır (ADR-0004).
