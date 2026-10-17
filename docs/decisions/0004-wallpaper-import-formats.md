# ADR-0004 — Duvar kağıdı içe aktarma: önbellek biçimleri ve dönüştürme yolu

Tarih: 2026-10-04 · Durum: taslak (inceleme'ı)

## Bağlam
Kullanıcı duvar kağıdı eklediğinde kaynak, oynatması en ucuz biçime bir kez dönüştürülüp
`$XDG_CACHE_HOME/nuxut/wallpapers/` altında saklanmalı: shell her açılışta 5600×3200 JPEG
çözmesin, GIF karelerini CPU'da çözüp yüklemesin, 4K AV1 videoyu yazılımla çözmesin.
Kaynak dosyaya dokunulmaz. Makine: VM, virtio-gpu (llvmpipe), VA-API sürücüsü yok.

## Durağan görsel: QOI
5600×3200 JPEG, hedefe cover küçültülmüş; release; makine yükü ~30 (6 çekirdek, eşzamanlı
derlemeler) altında ölçüldü — mutlak sayılar gürültülü, oranlar anlamlı:

| Biçim | 1920×1097 boyut | çözme | 3840×2194 boyut | çözme | Not |
|---|---|---|---|---|---|
| kaynak JPEG çöz + Lanczos | — | ~340–850 ms | — | ~340 ms | her açılışta bu ödenirdi |
| **QOI** | 3.3 MiB | 13 ms | 11.3 MiB | 69 ms | saf Rust, tek geçiş, ara tampon yok |
| kayıpsız WebP | 2.8 MiB | 60 ms | 9.3 MiB | 163 ms | en küçük, 2.5–5× yavaş |
| PNG (hızlı) | 3.8 MiB | 51 ms | 12.8 MiB | 429 ms | |
| zstd-3 ham RGBA (C) | 4.9 MiB | 11 ms | 17.6 MiB | 78 ms | C kütüphanesi; saf Rust (ruzstd) 25× yavaş |
| lz4 ham RGBA | 6.7 MiB | 20 ms | 24.7 MiB | 47 ms | disk 2× |
| ham RGBA | 8 MiB | okuma | 32 MiB | okuma | disk 3× |

Karar: **QOI** (`image` crate'inin `qoi` özelliği; çözücü zaten `nuxut-image` yolunda,
yeni çözme kodu yok). Kayıpsız; WebP'den %15–20 büyük ama 2.5–5× hızlı; zstd'ye denk hız, C
bağımlılığı yok. Tepe bellek = yalnızca çıktı tamponu.

## Animasyon (GIF/WebP/APNG) → video
CPU'da GIF çözümü kare başına LZW + tuval birleştirme + RGBA doku yüklemesi demektir
(102 kareli 634×396 GIF: ~%10 CPU llvmpipe'ta, bellekte tutulursa 98 MB). Video çözücüleri
(özellikle donanım) çok daha ucuz ve kareleri zaten GPU'da üretir. Dönüşüm: hedefe sığacak
şekilde küçült (büyütme yok), `yuv420p`, H.264 (libx264 `crf=18 preset=slow`) `.mkv`, sessiz.
Şeffaflık siyaha düzlenir (duvar kağıdı opak).

## Video
Yeniden kodlama yalnızca gerektiğinde: çözünürlük hedeften büyükse ya da codec VA-API ile
çözülemiyor ve yazılımda pahalıysa (AV1/VP9/HEVC). Hedef codec: VA-API yoklamasında
desteklenenler arasında H.264 > HEVC > VP9 > AV1 (çözme maliyeti sırası); VA-API yoksa
H.264 (yazılımda en ucuz). Ses akışı korunur (`oac=copy` olmazsa libopus ile yeniden kodlanır).

## Dönüştürme motoru: libmpv kodlama kipi
Seçenekler: `ffmpeg-next` (WTFPL, derlemede FFmpeg başlıkları + bindgen gerekir), `rsmpeg`
(FFmpeg 8'e sabit; sistemde 9 var), harici `ffmpeg` süreci (yasak: harici betik/araç),
**libmpv kodlama kipi** (`o=`, `ovc=`, `ovcopts=`, `vf=scale`). Oynatma için libmpv zaten
`dlopen` ediliyor; aynı kütüphane kodlamayı da yapar, ek bağımlılık ve derleme zamanı başlık
gerektirmez. Ölçüm (aynı GIF → H.264): ffmpeg CLI 1.2 s CPU, mpv kodlama 2.2 s CPU — tek
seferlik arka plan işi için kabul edilebilir.

## VA-API yeteneği
`vainfo` kurulu değil ve zaten dış araç. Yoklama libmpv ile: her aday codec için 16 karelik
`lavfi testsrc` klibi yazılımla kodlanır, `hwdec=vaapi vo=null` ile açılır, `hwdec-current`
okunur. Sonuç (sürücü + mpv sürümüyle anahtarlanmış) `hwcaps.toml`'da önbelleklenir.

## Önbellek
Ad `<blake3(içerik)[..16]>-<W>x<H>.<ext>`: aynı dosyanın farklı adla eklenmesi ya da
monitör değişimi doğru girdiyi bulur. Yazım geçici dosya + `rename`; iptalde geçici dosya
silinir. Boyut sınırı (varsayılan 2 GiB) aşılırsa en eski erişilenler silinir.

## Sonuçlar
- Sistem paketleri: `mpv` (libmpv; isteğe bağlı), libx264 mpv'nin FFmpeg'inde.
- Lisans: libmpv ve FFmpeg sistemden dinamik yüklenir; Arch derlemeleri GPL özellikli
  olabilir — kaynak kodumuz MIT kalır, dağıtılan ikili GPL kütüphaneyi içermez.
