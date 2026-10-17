# Pencere ve çıkış yakalama (`nuxut-wayland` `capture.rs`, `toplevels.rs`)

Durum: **uygulandı (framework, 2026-10-14).** Talep: kabuk (ekran paylaşım
seçicisinde pencere önizlemesi, `../shell/docs/design/screenshare-picker.md` "Framework
talepleri" 2; ekran görüntüsü `../shell/docs/design/screenshot.md` "İleride: framework'te
ext-image-copy-capture"), onaylı. Bağlayıcı API ve sözleşme `docs/ARCHITECTURE.md`
"Pencere listesi ve yakalama" bölümünde; bu belge gerekçeyi ve doğrulamayı tutar.

## Amaç ve sınırlar

- **Tek kare, talep üzerine.** Uygulama bir kaynak (çıkış ya da pencere) için bir kare ister,
  runtime yakalar, küçültür ve tek olayla verir. Sürekli akış, video, ekran kaydı **yok**
  (o iş xdph/PipeWire'ın). Boşta sıfır iş: hiçbir oturum açık kalmaz, global'ler tembel bağlanır.
- **Önizleme boyutu.** Sonuç uygulamanın verdiği `max_size` kutusuna en-boy korunarak sığdırılmış,
  doğrudan `Painter::upload_texture`'a verilebilen premultiplied RGBA8 (`ImageData`). Tam boyut da
  istenebilir (`max_size` = kaynak), üst sınır `MAX_TEXTURE_SIDE` (8192).
- **Pencere kimliği.** Standart `ext-foreign-toplevel-list-v1` tutamacı (`ToplevelId`); Hyprland'de
  ayrıca Hyprland pencere adresi (`hyprland-toplevel-mapping-v1`) — seçici xdph'ten adres alıyor.

## Protokoller ve tercih sırası

| Amaç | Birinci | Yedek | Neden |
|---|---|---|---|
| Çıkış karesi | `ext-image-copy-capture-v1` + `ext_output_image_capture_source_manager_v1` | `zwlr_screencopy_manager_v1` (sürüm 1–3) | ext standart (staging), Hyprland 0.56 ve sway 1.12 sunar; wlr eski compositor'lar için |
| Pencere karesi | `ext-image-copy-capture-v1` + `ext_foreign_toplevel_image_capture_source_manager_v1` | yok | bkz. "Reddedilen: hyprland-toplevel-export" |
| Pencere listesi/kimliği | `ext-foreign-toplevel-list-v1` | yok | standart; Hyprland ve sway sunar |
| Hyprland adresi ↔ tutamaç | `hyprland_toplevel_mapping_manager_v1` (gömülü XML) | yok (adres `None`) | Hyprland'e özgü; tek istek/yanıt |

Bağlamalar: ext protokolleri `wayland-protocols` 0.32 `staging` özelliğinde (çalışma alanında zaten
açık), wlr-screencopy `wayland-protocols-wlr` 0.3'te (zaten bağımlılık). Yeni crate yok.

**Doğrulama (registry dökümü, `examples/capture.rs --globals`):**

- Hyprland 0.56.2 (kurulu paket; **kullanıcının oturumuna bağlanılmadı**): paketteki
  `/usr/include/hyprland/protocols/` başlıkları `ext-image-copy-capture-v1`,
  `ext-image-capture-source-v1` (çıkış ve toplevel kaynak yöneticisi, `ImageCaptureSource.hpp`),
  `ext-foreign-toplevel-list-v1`, `hyprland-toplevel-mapping-v1`, `hyprland-toplevel-export-v1`,
  `wlr-screencopy-unstable-v1` içerir. Canlı registry dökümü YAPILMADI (görev kuralı).
- İzole headless sway 1.12: aşağıda "Doğrulama" (hepsi var, hyprland protokolleri yok).

### Reddedilen: hyprland-toplevel-export-v1

Hyprland'in eski pencere yakalama protokolü (`capture_toplevel(frame, overlay_cursor, uint
handle)`, adresin alt 32 biti). Hyprland 0.56 aynı işi standart ext yoluyla da yapıyor; ext yolu
sway'de aynı kodla test edilebiliyor, Hyprland'e özgü kod yalnızca adres eşlemesine iniyor.
XML ağ olmadan yeniden kurulmak zorunda kalınırdı (adres eşlemesinde yapıldığı gibi). Hyprland
< 0.53'e (ext yakalama yok) destek gerekirse ikinci yedek olarak eklenir; `CaptureSupport.toplevel`
o compositor'da `false` döner.

### Gömülü XML: hyprland-toplevel-mapping-v1

Makinede hyprland-protocols XML'i yok, ağ yok. `crates/nuxut-wayland/protocols/
hyprland-toplevel-mapping-v1.xml` iki kaynaktan **birebir** yeniden kuruldu: Hyprland 0.56.2
paketinin hyprwayland-scanner başlığı (`/usr/include/hyprland/protocols/
hyprland-toplevel-mapping-v1.hpp`: arayüz/istek/olay adları ve sırası, BSD-3 telif bloğu) ve
`/usr/bin/Hyprland`'in `.data.rel.ro`'sundaki `wl_interface`/`wl_message` tabloları (RELR
yer değiştirmesiz işaretçiler → `.rodata` dizgileri): yönetici sürüm 1, 3 istek
(`get_window_for_toplevel` "no", `get_window_for_toplevel_wlr` "no", `destroy` ""), 0 olay;
tutamaç sürüm 1, 1 istek (`destroy`), 2 olay (`window_address` "uu", `failed` ""). İmza ya da
sıra hatası Hyprland'de protokol hatası (bağlantı, yani kabuk ölür) demek olduğu için bu ikinci
kaynak şarttı. Açıklama metinleri özet; dosyadaki not bunu söyler. Lisans BSD-3 (telif dosyada),
`docs/DEPENDENCIES.md` `wayland-scanner` satırı.

## Kimlik eşleme

- `ToplevelId`: runtime'ın verdiği opak kimlik (ext tutamacı başına). Yalnızca
  `cx.toplevels_start()` sonrası vardır; liste durunca ya da pencere kapanınca geçersiz.
- `ToplevelInfo.hyprland_address: Option<u64>`: Hyprland'de mapping yöneticisi varsa her yeni
  tutamaç için tek `get_window_for_toplevel`; `window_address(hi, lo)` → `(hi << 32) | lo`.
  `Added` olayı **adres yanıtından sonra** gelir (yanıt bir gidiş-dönüş; `failed` → `None`), böylece
  tüketici adresi ilk olayda görür. `cx.toplevel_by_hyprland_address(addr)` ters arama.
- xdph'in `XDPH_WINDOW_SHARING_LIST`'i hem alt 32 bit kimliği hem tam adresi (u64) veriyor; seçici
  tam adresle arar.

## Tampon ve bellek

- `wl_shm` + memfd (`MFD_CLOEXEC`, dosya sisteminde iz yok), `PROT_READ` eşleme. Dmabuf yok (GPU
  okuması gerekirdi; önizleme için gereksiz).
- Biçim (yalnızca 8 bit): `Argb8888` > `Xrgb8888` > `Abgr8888` > `Xbgr8888`. Başka biçim (10 bit HDR
  vb.) → `NoUsableFormat`. Çıkış kaynağında alfa her zaman 255 (opak); pencerede alfa korunur
  (compositor premultiplied verir).
- **Kaynakta küçültme yok** (ext/wlr protokollerinde ölçek isteği yok): tam tampon yakalanır,
  sonra kutu filtresiyle (alan ortalaması, premultiplied'de doğru) tek geçişte küçültülür +
  biçim çevrilir. Dönüşüm (`transform` olayı / wlr `y_invert`) küçük sonuç üstünde uygulanır.
- Çevirme/küçültme **arka plan thread'inde** (`nuxut-capture`, iş başına bir thread; sonuç calloop
  kanalıyla döner): 4K çıkış ~33 MB okuma, olay döngüsünü kare süresi kadar tıkamasın.
- **Tavanlar:** tampon başına `MAX_BUFFER_BYTES` = 7680×4320×4 (8K UHD; aşan `TooLarge`, hiçbir
  tampon açılmaz); aynı anda en çok `MAX_ACTIVE` = 2 iş (oturum + çevirme; en kötü ~265 MB, tipik
  2×8 MB); fazlası FIFO kuyrukta (`MAX_QUEUED` = 256, aşan `Busy`). Aynı kaynağın ikinci isteği
  birincisi bitene dek bekler (bkz. Doğrulama). İptal edilen işin yuvası çevirme thread'i bitince
  boşalır (bellek tavanı korunur). Sonuç `max_size` ≤ 8192.
- Süre sınırı: oturum başladıktan sonra `timeout` (varsayılan 3 sn) → `Timeout`. Gizli çalışma
  alanındaki pencere için compositor kare üretmeyebilir.

## Güvenlik ve mahremiyet (değişmez 9)

1. **Yakalanan içerik kullanıcı içeriğidir.** Piksel, pencere başlığı ve uygulama kimliği hiçbir
   log seviyesinde yazılmaz; loglar yalnızca istek kimliği, kaynak türü, boyut, biçim kodu ve hata
   türü. `CapturedImage`, `ToplevelInfo`, `ImageData` Debug'ları redakte (boyut/uzunluk) — testli.
2. **Diske yazılmaz.** memfd; framework hiçbir yerde dosya açmaz. Doğrulama örneği PNG yazmaz,
   pikselleri bellekte denetler.
3. **Yalnızca istenen yüzey.** Her istek kendi kaynağını/oturumunu açar ve bitince yok eder;
   kaynak başka isteğe paylaşılmaz. Pencere listesi tembel ve açık uçlu değil:
   `toplevels_start` çağrılmadıkça compositor uygulamaya pencere bildirmez; `toplevels_stop`
   bırakır.
4. **Kilit (2026-10-17).** Yakalama kapısı iki kaynağa bakar:
   - **Kendi kilidi:** bu süreç kilit istediği andan (`Pending` dahil; `lock_state() != Unlocked`)
     kilit bitene dek.
   - **Dış kilit:** başka bir kilitleyici (swaylock, hyprlock, `loginctl lock-session`) kilitliyken
     runtime bunu Wayland'den öğrenemez; uygulama logind oturumunun `LockedHint`'ini izler ve
     `cx.set_session_locked_hint(bool)` ile bildirir (runtime D-Bus'a bağlanmaz). Varsayılan
     `false`: bildirmeyen uygulamada yalnız kendi kilidi geçerlidir. `LockedHint` kilitleyicinin
     kendi bildirimidir, kilidin kanıtı değildir; yalnız ek reddetme sebebi olarak kullanılır,
     yakalamaya izin vermek için kullanılmaz.

   Kapı kapalıyken: yeni istek `Locked`; kapı kapandığı anda yürüyen işler `Locked` ile biter
   (Wayland nesneleri bırakılır), kuyruktakiler yuvaya geçtikçe `Locked` alır, çevirme thread'inde
   olan kare teslimde `Locked`'a çevrilir (kilitten önce alınmış kare de kilitliyken verilmez).
   Kilit açıldıktan sonra yeni istek normal işler; eski sonuçlar geri gelmez. Sınır: dış kilitte
   kilitleyicinin başlaması ile `LockedHint` sinyalinin uygulamaya ulaşması arasında biten bir
   yakalama teslim edilmiş olabilir (o anda ne çizildiği compositor politikası; Hyprland kilit
   yüzeyini çizer). Kapı bu pencereyi kapatmaz, kilit boyunca yeni içerik vermez.
5. **Gizli pencereler.** Hyprland `noscreenshare` pencere kuralı ve gizli çalışma alanındaki
   pencerelerin ne verdiği compositor politikası; runtime ek bir şey gizlemez, kare gelmezse
   `Timeout`. **Hyprland'de canlı doğrulanmadı.**
6. İmleç varsayılan olarak çizilmez (`paint_cursor: false`).

## Hata modeli

Her `cx.capture` için `App::captured` **tam bir kez** gelir (çağrının içinden değil; aynı olay
turunun sonunda ya da sonra), `capture_cancel` edilmediyse. `CaptureError`: `Unsupported`,
`UnknownSource`, `SourceGone`, `Locked`, `TooLarge`, `NoUsableFormat`, `Busy`, `Timeout`,
`Failed`, `Buffer`. ext `failed(buffer_constraints)` bir kez yeni kısıtlarla yeniden denenir.

## API taslağı (uygulanan)

```rust
pub struct ToplevelId(/* opak */);
pub struct ToplevelInfo { pub id, pub app_id: String, pub title: String,
                          pub hyprland_address: Option<u64> }       // Debug redakte
pub enum ToplevelEvent { Added(ToplevelId), Changed(ToplevelId), Removed(ToplevelId) }
pub enum CaptureSource { Output(OutputId), Toplevel(ToplevelId) }
pub struct CaptureOptions { pub max_size: (u32, u32), pub paint_cursor: bool,
                            pub timeout: Duration }                 // Default: 8192², false, 3 sn
pub struct CaptureId(/* opak */);
pub struct CapturedImage { pub image: ImageData, pub source_size: (u32, u32) } // Debug redakte
pub struct CaptureSupport { pub output: Option<CaptureBackend>, pub toplevel: bool }
pub enum CaptureBackend { ExtImageCopy, WlrScreencopy }

cx.toplevels_supported() / toplevels_start() -> Result<(), ToplevelListError> / toplevels_stop()
cx.toplevels() / toplevel(id) / toplevel_by_hyprland_address(addr)
cx.capture_support() / capture(source, options) -> CaptureId / capture_cancel(id) / set_session_locked_hint(bool)
App::toplevel(cx, ToplevelEvent)   App::captured(cx, CaptureId, Result<CapturedImage, CaptureError>)
```

## Testler

- Birim (saf): sığdırma, kutu filtresi (düz renk, 2×2 → 1 ortalama, opak çıkış alfası), biçim
  seçimi ve bayt düzeni, dönüşümler (90/180/270/çevrik, y-ters), adres birleştirme, metin
  kırpma, kabul/kuyruk sınırları, Debug redaksiyonu, tavan hesabı.
- Protokol: gömülü XML `wayland-scanner` ile derlenir (imzalar Hyprland ikilisiyle karşılaştırıldı,
  yukarıda).
- Canlı (yalnızca izole sway): `examples/capture.rs` kendi penceresini (bilinen düz renkler) ve
  çıkışı yakalar, pikselleri bellekte doğrular, PNG yazmaz. ext ve `NUXUT_CAPTURE=wlr` yolları.

## Doğrulama

2026-10-14. Hepsi izole headless sway 1.12'de (`tools/test-compositor.sh`,
`NUXUT_TEST_NAME=test-capture`, `flock` altında); ekran görüntüsü alınmadı, PNG yazılmadı.

- **Registry** (`examples/capture.rs --globals`): `ext_image_copy_capture_manager_v1` v1,
  `ext_output_image_capture_source_manager_v1` v1,
  `ext_foreign_toplevel_image_capture_source_manager_v1` v1, `ext_foreign_toplevel_list_v1` v1,
  `zwlr_screencopy_manager_v1` v3, `wl_shm` v2; `hyprland_toplevel_export_manager_v1` ve
  `hyprland_toplevel_mapping_manager_v1` yok (beklenen).
- **Uçtan uca** (`examples/capture.rs`): kendi penceresi (sol yarı `primary`, sağ yarı `error`
  tema rolü) tam boyut + 64×64 önizleme, çıkış 256×256 önizleme, bir iptal. Dört çeyreğin rengi
  ±3 toleransla, boyut ve en-boy denetlendi. Beş koşu, her biri 16/16:
  ext; `NUXUT_CAPTURE=wlr`; çıkış `transform 90` ile ext ve wlr (çıkış önizlemesi 144×256, dik);
  çıkış ölçeği 1,5 (sway pencereyi mantıksal 1280×720 verdi, çıkışı fiziksel 1920×1080).
- **Bulunan ve düzeltilen:** (1) aynı pencereye eşzamanlı ikinci oturum sway'de kare vermedi
  (durağan içerik, zaman aşımı) → kabul defteri aynı kaynağın isteklerini sıraya koyar. (2) wlr
  yedeği döndürülmüş çıkışta ham tamponu veriyordu (wlr'de dönüşüm olayı yok) → çıkışın
  `wl_output.transform`'u uygulanır.
- **Birim:** 18 test (biçim seçimi/bayt düzeni, kutu filtresi, satır adımı ve geçersiz tampon,
  dönüşümler ve tersleri, y-ters, dik sığdırma, kabul/kuyruk/kaynak sırası, tampon tavanı, gerçek
  memfd eşlemesinden çevirme, adres birleştirme, metin kırpma, Debug redaksiyonu).
- **Hyprland canlı doğrulanmadı** (mahremiyet kuralı: kullanıcının oturumunda hiçbir pencere ya da
  çıkış yakalanmadı, registry'sine bağlanılmadı; örnek `HYPRLAND_INSTANCE_SIGNATURE` varken
  çalışmayı reddeder). Hyprland'e özgü yol (adres eşlemesi) yalnızca birim düzeyinde; XML imzaları
  Hyprland 0.56.2 ikilisine karşı doğrulandı. Hyprland'de ilk canlı denemede bakılacaklar: ext
  pencere kaynağının gizli çalışma alanındaki pencerede kare verip vermediği (vermezse `Timeout`),
  pencere tamponunun fiziksel mi mantıksal mı geldiği, adres eşlemesinin `hyprctl clients`
  adresiyle aynı olduğu.

## Açık kalanlar

- Dmabuf yolu yok (GPU okuması gerekirdi); yalnızca 8 bit shm biçimleri — 10 bit HDR çıkışta
  `NoUsableFormat`.
- `Changed` olayı yalnızca başlık/uygulama kimliği için; adres sonradan değişmez (Hyprland'de
  pencere adresi pencere ömrü boyunca sabit).
- Ekran görüntüsü (shell `screenshot.md`) tam çözünürlük için `CaptureOptions::default()` ile
  kullanılabilir; çok çıkışlı birleştirme ve bölge kırpma tüketicide.
