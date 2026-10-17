# Duvar kağıdı yalıtılmış işçide (ADR-0007 takip)

Durum: **onaylandı ve uygulandı (framework, Aşama 2, 2026-10-13).** Bağlayıcı API ve sınırlar
`docs/ARCHITECTURE.md` "Duvar kağıdı yalıtımı" bölümündedir; bu belge tasarım gerekçesini ve
kararların kaydını tutar. Kararlar: S1 ve S2 ÖNERİLEN seçenekler, S3 kabuk
(kırıcı zorunlu parametre), palet kırıcı imzası Ayarlar. Sayılar
`nuxut-image-worker` `examples/wallpaper_bench.rs` ile ölçüldü (§1 sonu).

**Uygulamada taslaktan sapmalar:**
- `decode_file_info` yok: tek yöntem `IsolatedDecoder::decode_file(key, File, opts, &CancelFlag) ->
  DecodedFile { image, animated_source }`.
- `WallpaperDecoderOptions.key` yok: `WallpaperDecoder` anahtarı istek başına kendisi türetir
  (`file_key("wallpaper", yol)`); tüketici başına anahtar kalıbı §7.
- Kırpma `TargetSize { crop }` / `TargetSize::cover_crop` ile taşınır (`DecodeOptions` ve `Fit`
  değişmedi: tüketicilerin literal kurulumları kırılmasın). Tel üzerinde istek bayrağı `CROP`.
- İstek başlığı 48 bayt: çözücü bütçesi (`max_decode_bytes`) de tele çıktı; `Limits`e
  `max_decode_bytes` alanı eklendi (`..Limits::ICON` kalıbı kırılmaz).
- JXL tahmini ölçümle 20 → **28 B/px**; duvar kağıdı bütçesinde JXL tavanı ≈ 57 Mpx (taslakta 80).
- RSS dönüşü ölçüldü: bırakılan 4K kare işletim sistemine dönüyor (+1 MiB kalıntı), özel ayırma
  gerekmedi.
- `Wallpaper`'ın süreç içi animasyon oynatımı tümden kalktı (S2-a); `WallpaperStatus.next_frame_in`
  her zaman `None`.

## Karar
Duvar kağıdı kullanıcı dosyasıdır, internetten inmiş olabilir. Bu yüzden **tüm biçimler (JXL
dahil) güven ne olursa olsun yalıtılmış işçide çözülür**. Shell, lock ve settings ana
ağaçlarına `jxl-oxide` girmez. ADR-0007'deki "`System` (kullanıcının seçtiği duvar kağıdı) süreç
içinde kalır" cümlesi bununla değişir. `Trust::System` artık yalnızca bizim seçtiğimiz sistem
teması ikonları ve gömülü varlıklar içindir.

## Bugün (main b2099c9)
- `Wallpaper::set_source` her çıkış için kendi `Loader`'ında (çıkış başına 1 kalıcı thread)
  `Trust::System` + `Limits::DEFAULT` (256 MiB, 16K kenar, 80 Mpx) ile süreç içinde çözer.
  Animasyon (GIF/WebP/APNG) süreç içinde oynar. Çok monitörde aynı dosya her çıkış için
  yeniden çözülür.
- Settings: `source_color_from_file` (süreç içi, System, 128² Contain). Küçük resimler
  `Wallpaper` ile çözülür: kahraman/önizleme 1280×800, çıkış satırı 192×120.
- İşçi tek profillidir: AS 768 MiB, CPU 4/5 sn, girdi ≤ 8 MiB (`MAX_INPUT_BYTES`), duvar saati
  1.5 sn. Duvar kağıdına yetmez.

## 1. Profil: `WorkerProfile`
İşçinin rlimit'leri girdi okunmadan önce kurulur. Bu yüzden profil **istek başına değil, işçi
süreci başına** belirlenir: ebeveyn işçiyi `--profile=wallpaper` argümanıyla başlatır, işçi
kilitlenirken profili uygular ve el sıkışmada bildirir. Ebeveyn beyanı kendi profiliyle
karşılaştırır, uyuşmazsa `Unavailable` döner. Her profil ayrı bir `Sandbox` örneğidir (ayrı
havuz, kova, susturma). Böylece tepsi ikonu fırtınası duvar kağıdını hız sınırına sokamaz.
İkon işçisinin dar sınırları (768 MiB / 4 sn) da gevşemez: ele geçirilmiş bir tepsi ikonu daha
fazla kaynak elde edemez.

```rust
// nuxut-image-sandbox
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkerProfile { #[default] Icon, Wallpaper }
pub struct SandboxConfig { /* mevcut alanlar */ pub profile: WorkerProfile, pub negative_on_timeout: bool }
impl SandboxConfig {
    pub fn wallpaper() -> Self;   // aşağıdaki tablo
}
```

| Sınır | Icon (bugün) | **Wallpaper** | Gerekçe |
|---|---|---|---|
| Girdi baytı | 8 MiB | **256 MiB** | Bugünkü süreç içi `Limits::DEFAULT` ile aynı, gerileme yok. Akışla gönderilir (§3); ebeveyn tamamını belleğe almaz |
| Kenar | req (4096) | **16 384** | 16K panorama/tek parça çoklu monitör |
| Kaynak piksel (başlık kapısı) | req | **268 Mpx (16384²)** | İstenen örnek 16K×16K |
| Çözücü bellek bütçesi (`max_alloc` + biçim tahmini) | max_pixels·8 | **1.5 GiB** | 8 bit JPEG 16K² ≈ 805 MB geçer. 16 bit RGBA PNG 16K² ≈ 2 GiB `TooLarge` olur. JXL **ölçüm ≈ 27.7 B/px → 28** → ~57 Mpx (8K UHD geçer, 8K×8K `TooLarge`) |
| `RLIMIT_AS` | 768 MiB | **2.5 GiB** | girdi 256 MiB + bütçe 1.5 GiB + ikili/ayırıcı payı |
| `RLIMIT_CPU` | 4 / 5 sn | **10 / 12 sn** | Gerçek iş sınırı budur. Tek thread JXL 8K UHD **4.6 sn** (ölçüm), 16K² JPEG 2.5 sn; seccomp `clone`'u kapattığı için işçi tek thread'lidir (jxl-oxide rayon'suz) |
| Duvar saati (`timeout`) | 1.5 sn | **20 sn** | Yüklü VM'de (yük 9–33) aç kalan işçi sahte zaman aşımı vermesin; CPU'yu rlimit sınırlar |
| Zaman aşımı → negatif önbellek | evet | **hayır** (`negative_on_timeout: false`) | Yük altında meşru dosya kalıcı karalisteye düşmesin. `Killed` (SIGXCPU, AS, seccomp) ve `Protocol` yine girer |
| Çıktı piksel (`max_out_pixels`) | 4096² | **8192² (64 Mpx)** | 8K çıkış; asıl sınır hedef kutudur (§2) |
| `max_workers` / `prewarm` | 2 / 1 | **1 / 0** | Değişim seyrek, soğuk başlangıç ~2 ms ve çözümün yanında önemsiz. Boşta süreç yok |
| Kova / susturma | 30/sn, 60; 3 hata → 60 sn | **4/sn, 8; 3 hata → 30 sn** | Çok çıkış tek çözüme iner (§4). Slayt gösterisi betiği döngüye girerse kesilir |
| Öncelik | — | **nice 10, `oom_score_adj` 1000** | İşçi kilitten önce kendine uygular (rustix güvenli API; `/proc/self/oom_score_adj` landlock'tan önce yazılır). Bellek sıkışınca çekirdek önce işçiyi öldürür, shell'i değil. nice 19 yerine 10: yük altında açlık sahte zaman aşımı üretmesin |

**Ölçüm** (release işçi, VM yük 15–18, uçtan uca `WallpaperDecoder` → kare; işçi VmHWM): 4K JPEG →
4K 0.54 sn / 63 MiB; 8K JPEG → 4K 0.47 sn / 181 MiB; 8K JPEG → 8K 0.57–0.84 sn / 235 MiB; 16K² JPEG →
4K 2.5 sn / 939 MiB; 8K PNG → 4K 0.60 sn / 239 MiB; 8K JXL → 4K 4.6 sn / 879 MiB. Ebeveyn: kare
tutulurken +çıktı kadar, bırakınca +1 MiB. 200 MiB akış: ebeveyn +20 KiB. İptal → işçi ölü 51 ms.

Bellek tavanının tamamı geçicidir. İşçi tek kullanımlıktır ve yanıttan hemen sonra
öldürülür. Aynı anda en fazla 1 duvar kağıdı işçisi yaşar.

Tel biçimi **sürüm 3** olur:
- el sıkışmaya `u8 profile` eklenir (başlık 12 bayt),
- istekte ayrılmış `b[7]` bayrak baytı olur: bit0 `CROP`,
- yanıtta ayrılmış `u16` bayrak alanı olur: bit0 `ANIMATED_SOURCE` (kaynak animasyonluydu,
  ilk kare verildi),
- `payload_len` üst sınırı işçinin profilinden gelir.

`Response::validate`: `CROP` varsa iki eksen de hedefin içinde olmalıdır.

## 2. İşçide ölçekleme
- **Hedef = çıkışın fiziksel boyutu** (`mantıksal × ölçek`). Ana sürece yalnızca hedef
  boyuttaki premultiplied RGBA gelir.
- **Cover → `CROP`.** Görsel kutuyu kaplayacak ölçeğe getirilir ve ortadan kırpılır, kırpma
  küçültmeden ÖNCE kaynak dikdörtgende yapılır (`fast_image_resize` `ResizeOptions::crop`, tek
  geçiş). Böylece çıktı **tam olarak ≤ hedef** olur. Bugünkü Cover bir ekseni taşırır:
  3840×2160 çıkışta 16384×2160 panorama 141 MB'lık çıktı verirdi. Kırpma `ImageFit::Cover`'ın
  çizimdeki ortalamasıyla piksel aynıdır.
- **Contain / Fill:** ≤ hedef, kırpma yok. **Pay:** "biraz pay" istendi ama kırpma ve tam
  boyutla gerekmiyor. Ölçek değişiminde istek zaten yenilenir (anahtar hedef boyutu içerir).
- **Filtre:** küçültmede Lanczos3 (`fast_image_resize`, SIMD), bugünküyle aynı. Asla
  büyütülmez; küçük görseli GPU doğrusal örnekleyiciyle büyütür. Dar biçim (RGB/gri) önce kendi
  kanalında küçültülür: tam boyutlu RGBA kopyası oluşmaz (mevcut `from_dynamic`).
- **Animasyon:** işçi yalnızca ilk kareyi verir (K5) ve `ANIMATED_SOURCE` bayrağını koyar.
  **S2 kararı: (a)** ilk kare + `animated_source()`.

## 3. Dosya akışı: ebeveyn dosyayı belleğe almaz
256 MiB'lık bir dosya shell'in adres alanında `Arc<[u8]>` olarak tutulmamalıdır.

```rust
// nuxut-image (isolation.rs) — varsayılan gövde: max_input_bytes'a kadar okur, decode() çağırır
pub trait IsolatedDecoder {
    /* mevcut */
    fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag)
        -> Result<Image, ImageError>;
    fn decode_file_info(..) -> Result<(Image, DecodeInfo /* animated_source */), ImageError>; // aynı, bayraklı
}
pub struct CancelFlag(Arc<AtomicBool>);           // set → işçi öldürülür, Err(Cancelled)
pub fn open_user_file(&Path) -> Result<std::fs::File, ImageError>;  // bugünkü open_regular: O_NONBLOCK, fstat normal dosya
```

`Sandbox::decode_file` şu sırayla çalışır:
1. `fstat` ile `len` alınır ve `len ≤ max_input_bytes` denetlenir.
2. Koklama için ilk 64 bayt `read_at` ile okunur.
3. Kapılardan geçilir.
4. Boruya 256 KiB'lık parçalar `read_at` → `write_all` (son tarih + iptal) ile yazılır.

İptal 50 ms'lik `poll` dilimleriyle denetlenir. İptal **başarısızlık sayılmaz**, negatif
önbelleğe girmez.

**Negatif önbellek anahtarı (dosya):** `blake3(dev ‖ ino ‖ len ‖ mtime_ns ‖ ctime_ns)`. Önceden
bilinir, içeriği iki kez okumak gerekmez. Dosya değişirse anahtar da değişir. Bayt yolu
bugünkü gibi içerik özetini kullanır.

## 4. `WallpaperDecoder`: paylaşılan çözüm + önbellek
`nuxut-image` içinde, `render`'sız modül (`wallpaper_decode.rs`). Süreç başına tek örnek
vardır: shell bütün çıkışlarına bir tane verir, lock kendi sürecinde kendisininkini kurar,
settings kendisininkini kurar.

```rust
pub struct WallpaperDecoder;                       // Send + Sync; Arc ile paylaşılır
pub struct WallpaperDecoderOptions {
    pub cache_bytes: u64,      // tutulan çözülmüş karelerin üst sınırı; varsayılan 96 MiB
    pub linger: Duration,      // son kullanımdan sonra tutma; varsayılan 10 sn (lock: 0)
    pub key: u64,              // yalıtılmış çözücüye giden kaynak anahtarı; shell KeyDomain'den "wallpaper"
}
impl WallpaperDecoder {
    pub fn new(decoder: Arc<dyn IsolatedDecoder>, opts: WallpaperDecoderOptions) -> Arc<Self>;
    /// Yalnızca örnekler/testler: süreç içi, `System`, animasyonlu. Adı açıkça söylüyor.
    pub fn in_process_unsandboxed(opts: WallpaperDecoderOptions) -> Arc<Self>;
    pub fn request(self: &Arc<Self>, req: WallpaperRequest,
                   done: impl FnOnce(Result<WallpaperFrame, ImageError>) + Send + 'static) -> WallpaperTicket;
    pub fn trim(&self);                            // bekleyen dışındaki tüm kareleri bırak (bellek baskısı / kilit)
    pub fn stats(&self) -> WallpaperDecodeStats;   // decodes, hits, joined, derived, cancelled, resident_bytes, entries
}
pub struct WallpaperRequest { pub path: PathBuf, pub target_px: (u32, u32), pub fit: Fit }
#[derive(Clone)] pub struct WallpaperFrame { pub width: u32, pub height: u32,
    pub pixels: Arc<[u8]> /* premultiplied RGBA8, ImageData'ya kopyasız */, pub animated_source: bool }
pub struct WallpaperTicket;                        // drop = bu bekleyeni iptal et
```

**Anahtar:** `(dev, ino, len, mtime_ns, yol) + target_px + fit`. Dosya bir kez açılır, anahtar
aynı fd'nin `fstat`'ından alınır. İsabet olmazsa **aynı fd** işçiye akıtılır (TOCTOU yok).
Yol, adı değiştirilen dosyanın aynı içeriği eşleşmesin diye anahtarda kalır. Anahtar
loglanmaz.

**Akış:**
1. **İsabet:** kare hazırsa `done` hemen (çağıranın thread'inde değil, çözüm thread'inde)
   çağrılır.
2. **Uçuşta:** aynı anahtar çözülüyorsa yeni bekleyen eklenir. Çok monitör ve aynı
   çözünürlükte tek çözüm yapılır: `joined` sayacı.
3. **Türetme:** aynı dosya + fit için hedeften büyük bir kare elde varsa ve türetilebiliyorsa
   (Contain/Fill her zaman; Cover-kırpma yalnızca en-boy oranı ±1 px aynıysa), küçük kare
   **süreç içinde güvenilir RGBA'dan** Lanczos3 ile türetilir. Yeniden ayrıştırma olmaz.
   Karışık çözünürlüklü monitörler (4K + 1080p) de tek çözüme iner: `derived` sayacı.
4. **Kuyruk:** yoksa iş kuyruğa girer. Tek çözüm thread'i vardır (`nice` 19). Thread ilk
   istekte açılır, kuyruk boşalınca çıkar: boşta thread yok. Bugün her `Wallpaper`'ın kendi
   kalıcı `Loader` thread'i vardır, o kalkar.
5. **İptal:** `WallpaperTicket` düşünce bekleyen çıkarılır. Hiç bekleyen kalmazsa kuyruktaki
   iş silinir, süren iş `CancelFlag` ile durdurulur (≤ 50 ms'de işçi öldürülür). Duvar kağıdı
   hızlı değişirse çıkış başına en fazla 1 süren + 1 kuyrukta iş olur, ara seçimler hiç
   çözülmez.

**Bellek tavanı ve bırakma:**
- 4K kare 33 MB, 5K kare 59 MB, 1080p 8 MB.
- `cache_bytes` 96 MiB: iki farklı 4K kare, ya da 4K + 5K.
- Kare, bekleyenlere `Arc` ile verilir. `Wallpaper` onu GPU'ya yükleyince bırakır
  (`Layer.pending.take()`; bugün de öyle).
- Önbellek kareyi yalnızca `linger` (10 sn) boyunca tutar: geç gelen çıkış (hotplug, ölçek
  configure'u) yeniden çözmesin diye. Sonra bırakılır: **boşta CPU belleğinde çözülmüş kare
  yok, işçi süreci yok, thread yok.**
- Bütçeyi aşan kare teslim edilir ama tutulmaz (LRU). Lock `linger: 0` kullanır.
- **RSS'e dönüş (Aşama 2'de ölçülecek):** 4K tamponu (33 177 600 B) glibc'nin dinamik mmap
  eşiğinin (≤ 32 MiB) hemen altında kalabilir. Serbest bırakılınca yığında kalıyorsa kare
  tamponları 32 MiB'ı aşacak biçimde (ya da `Vec` yerine tek parça `Box<[u8]>`) ayrılır. Ölçüm:
  bırakmadan sonra `/proc/self/status` VmRSS.

## 5. `Wallpaper` değişikliği
```rust
impl Wallpaper {
    pub fn new(fit: ImageFit, motion: impl Into<Motion>, decoder: Arc<WallpaperDecoder>,
               notify: impl Fn() + Send + Sync + 'static) -> Self;   // KIRICI: decoder zorunlu
    pub fn set_source(&mut self, path, target_px);                    // değişmez
    pub fn animated_source(&self) -> bool;                            // gösterilen kaynak animasyonluydu (ilk kare)
}
```
İstenen `with_sandbox(Arc<dyn IsolatedDecoder>)` yerine **zorunlu parametre** öneriyorum.
Gerekçeler:
- Builder unutulursa sessizce süreç içi çözüm olurdu. Kararın ihlali derleme hatası olmalı.
- Önbelleğin çıkışlar arasında paylaşılması için ortak nesne gerekiyor; `Sandbox`'ın kendisi
  yetmez.

Üç tüketici (shell, lock, settings) zaten değişecek. Örnekler/testler
`in_process_unsandboxed` kullanır. Bulanıklık ve zemin (`set_blur`, `set_backdrop`
`Blur`/Contain letterbox) değişmez: aynı dokudan GPU'da üretilir, ek çözüm yoktur.

## 6. Hata ve yedek akışı
- `Err` → bugünkü davranış: önceki katman ekranda kalır, hiç katman yoksa zeminin düz rengi
  (`WallpaperBackdrop::color()`; shell'de opak taban) görünür. `last_error()` dolar.
- Log yalnızca hata türüdür (yol yok).
- Hata eşlemesi (shell'in kullanıcıya göstereceği):

| Hata | Anlamı |
|---|---|
| `TooLarge` / `FileTooLarge` | "Görsel çok büyük" |
| `UnknownFormat` / `Decode` | "Görsel açılamadı" |
| `Sandbox(Timeout \| Killed)` | "Görsel çözülemedi" |
| `Sandbox(RateLimited)` | sessiz yeniden deneme yok; sonraki değişimde |
| `Sandbox(Unavailable)` | → S1 |
| `Cancelled` | hiçbir şey |

- **S1 — işçi yokken (`Unavailable`). KARAR: ÖNERİLEN seçenek uygulandı;** uyarı
  süreç başına tek, kapılar (boyut/piksel/bütçe) aynı. Önceki metin: ADR K2-b ile aynı. PNG/JPEG/WebP/GIF/QOI süreç
  içinde, `Untrusted` kapısıyla, ilk kare, düşük öncelikli thread'de çözülür ve bir kez uyarı
  loglanır. JXL/AVIF/SVG zemin rengine düşer. Bu davranış `SandboxConfig::in_process_fallback:
  bool` alanıyla (ikonda zaten `true`) kapatılabilir. Alternatif (`false`): kurulum hatasında
  duvar kağıdı tümüyle düz renk olur. Kararın lafzı ("Trust ne olursa olsun işçide") ikincisine
  daha yakın; seçim açık.

## 7. Palette ve özellik düzeni
```toml
# nuxut-palette
[features]
default = ["image", "sandbox"]
image   = ["dep:nuxut-image"]                          # jxl YOK: source_color(&Image) + örnekleme sabitleri
sandbox = ["image", "dep:nuxut-image-sandbox"]         # CLI: işçiyle çözer (jxl'siz ana taraf)
[[bin]] required-features = ["sandbox"]
```
```rust
pub const SAMPLE_TARGET: TargetSize;      // 128×128 Contain (bugünkü SAMPLE_SIDE)
pub fn sample_options() -> DecodeOptions;  // Limits = duvar kağıdı profili, animate: false
pub fn source_color(&Image) -> Color;      // var; artık büyük görseli kendisi 128'e indirir (güvenilir RGBA, süreç içi)
pub fn source_color_from_file(path: &Path, decoder: &dyn IsolatedDecoder, key: u64) -> Result<Color, PaletteError>;  // KIRICI imza
pub fn source_color_from_frame(&WallpaperFrame) -> Color;   // settings önbellekteki kareden
```
- Süreç içi `source_color_from_file(path)` kalkar. Tek çağıranı settings'tir
  (`services/palette.rs`).
- **Tüketici başına anahtar:** `key` = `nuxut_image::file_key("wallpaper", path)`
  (= `derive_key([b"wallpaper", yol baytları])`; mevcut `derive_key` mekanizması). Örnek:
  `source_color_from_file(path, &*sandbox, file_key("wallpaper", path))`. Yol anahtarda: bir bozuk
  dosya diğerlerini susturmaz. Settings tek `Sandbox::new(SandboxConfig::wallpaper())`'ı hem
  `WallpaperDecoder::new`'e hem palete verir; tercihen renk `source_color_from_frame` ile önbellekteki
  128² Contain kareden alınır.
- CLI (`nuxut-palette <görsel>`) `Sandbox::new(SandboxConfig::wallpaper())` kurar. İşçi
  bulunamazsa S1 kuralı uygulanır.
- `nuxut-image` varsayılanı `["render", "jxl"]` → **`["render"]`**: güvenli seçim varsayılan
  olur. `jxl`'i yalnızca işçi paketi açar (ve açıkça isteyen süreç içi tüketici).
- `nuxut-video`'nun `jxl` varsayılanı da kapanır (§9).
- Doğrulama (Aşama 2): aşağıdakilerin hepsi boş olmalı.
  - `cargo tree -p nuxut-palette -e normal | grep jxl`
  - `--no-default-features --features image` ile aynı komut
  - settings ve shell ağaçlarında aynı komut (`--manifest-path`)

**Settings küçük resim hedefleri:**
- Kahraman ve önizleme: **kartın fiziksel boyutu** (`mantıksal × ölçek`), üst sınır
  **1280×800** (bugünkü sabit), Cover (kırpma).
- Çıkış satırı: **192×120 × ölçek** (1.5 ölçekte 288×180), Cover.
- Renk: ayrı istek, **128×128 Contain**. Cover kırpması kenar renklerini atardı, matugen tam
  görsele bakar.
- Hepsi settings'in tek `WallpaperDecoder`'ı üzerinden gider (`cache_bytes` 16 MiB yeter:
  kahraman 4 MB). Aynı dosyanın Contain 128'i elde Contain kare varsa türetilir.

## 8. Testler (Aşama 2)
- **Düşmanca:** 16K×16K başlıklı 1 KiB PNG (başlık kapısı, işçi başlamaz ya da `TooLarge`).
  Bütçe aşan 16 bit PNG → `TooLarge`. Sonsuz döngü kancası + duvar kağıdı profili → `Timeout`
  negatif önbelleğe GİRMEZ. CPU kancası → SIGXCPU → `Killed` GİRER. 2.5 GiB'tan büyük ayırma →
  `Killed`. Sahte yanıt `CROP`'ta taşan eksen → `Protocol`. Yanlış profil beyan eden işçi →
  `Unavailable`. 256 MiB + 1 dosya → `FileTooLarge` (işçi başlamaz).
- **Akış:** 200 MiB dosyada ebeveyn VmRSS artışı < 8 MiB. Akış ortasında iptal → işçi ≤ 100
  ms'de ölü (`ps`), `Cancelled`, negatif önbellek boş.
- **Önbellek:** 3 çıkış × aynı hedef → `decodes == 1`, `joined == 2`. 4K + 1080p Contain →
  `decodes == 1`, `derived == 1`. `linger` sonra `resident_bytes == 0`. Dosya `mtime`
  değişince yeniden çözülür. Hızlı 10 değişim → en fazla 2 çözüm, yalnızca sonuncusu teslim
  edilir.
- **Kırpma:** Cover-kırpma çıktısı, süreç içi Cover + ortalanmış kırpmayla piksel aynı
  (Lanczos sınır payı ±1).
- **Ağaç:** palette/shell/settings/lock `cargo tree | grep jxl` boş; işçi ağacında var.
- **Ölçüm:** 4K/8K/16K JPEG, 8K PNG, 8K JXL için işçi süresi ve tepe RSS (`/proc/<pid>/status`
  VmHWM). §1 tablosu ölçümle düzeltilir.

## 9. Açık sorular ve kapsam dışı bulgular
- **S1** (§6): işçi yokken temel raster süreç içine düşsün mü? (ÖNERİLEN: evet, K2-b ile aynı.)
- **S2 — animasyonlu duvar kağıdı.** İşçi yalnızca ilk kareyi verir. Seçenekler:
  - (a) Durağan ilk kare göster ve `animated_source()` ile settings'te "animasyon için içe
    aktar" ipucu ver. ÖNERİLEN, şimdilik.
  - (b) Animasyonlu kaynağı `nuxut-video` `Importer` → mkv → `VideoPlayer` yoluna yönlendir
    (ADR-0004 zaten bunu öngörüyor).
  - (c) İşçiden kare akışı. Reddedildi: tek kullanımlık işçi + uzun ömürlü akış çelişir.
- **S3 — `Wallpaper::new` kırıcı imza** (§5): **KARAR: zorunlu parametre**;
  shell/lock/settings aynı turda uyarlar.
- **Kapsam dışı, aynı sınıf (kullanıcı dosyası süreç içinde çözülüyor):**
  - lock `load_avatar` (`~/.face`, `Untrusted` → süreç içi temel raster, K2-b yolu) lock'un
    sandbox'ına alınabilir.
  - `nuxut-video` `Importer` durağan görseli `Trust::System` ile süreç içinde çözüyor
    (`import.rs:357`, `:666`) ve `is_animated`/`animation_info` kap yapısını süreç içinde
    yürüyor. Shell bugün `Importer` kullanmıyor; kullanılırsa aynı işçiye bağlanmalı. **Kapandı (2026-10-13):** `docs/design/video-import-sandbox.md` (tel v4).
  - mpv video ayrıştırması süreç içinde: ADR-0002 riski, ayrı konu.
