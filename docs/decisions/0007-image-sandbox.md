# ADR-0007 — `nuxut-image-sandbox`: güvenilmeyen görseller ayrı, kısıtlı süreçte

Tarih: 2026-10-05 · Gözden geçirme: 2026-10-11 · Durum: **kabul edildi ve
uygulandı (framework)** — K1–K6 ÖNERİLEN seçeneklerle onaylandı
(2026-10-12). Shell tarafı (K6 ikinci adım) bekliyor. Uygulamadaki sapmalar §Uygulama notları'nda.

## Bağlam
Shell, başka süreçlerin verdiği görselleri çözüyor:
- tepsi: `IconName` + uygulamanın `IconThemePath` dizinindeki SVG/PNG, dbusmenu `icon-data`,
- bildirim: `image-path` / `app_icon` dosyası, `image-data`,
- MPRIS: `mpris:artUrl` kapağı.

Bunların hepsi saldırgan kontrollü bayt olabilir. Bugünkü durum (main, `nuxut-image`):
- **Güven kapısı var** (ARCHITECTURE "Güven kapısı"): `Trust { System, Untrusted }`, güven
  belirtmeyen her istek `Untrusted`. `Untrusted` istekte SVG/svgz süreç içinde **hiç
  ayrıştırılmaz** (`ImageError::Unsupported("SVG")` → yedek ikon); biçim yalnızca içerik
  baytlarından tespit edilir. İkon dosyasının güveni isteğinkini aşamaz.
- `Untrusted` raster (PNG/JPEG/WebP/GIF/APNG/QOI/JXL) süreç içinde, `Limits` kapısıyla
  (başlıktan boyut, çözücü bellek sınırı, piksel/kare sınırları, cihaz pikselinde iş ve bellek
  bütçesi) çözülür. AVIF (`dav1d`, C) varsayılan derlemede yok.
- Release profili `panic = "abort"` (framework ve shell).

Kalan riskler süreç içinde kapatılamaz:
- **Çökme shell'i düşürür.** `panic = "abort"` altında çözücüdeki tek bir panik (dizin
  taşması, `unwrap`, aritmetik taşma denetimi) ya da yığın taşması bütün masaüstü kabuğunu
  kapatır; `catch_unwind` abort altında işe yaramaz. `image`/`zune-jpeg`/`jxl-oxide` geniş,
  bizim denetimimizde olmayan kod yüzeyleri.
- **Tahmin ≠ ölçüm.** Denetim üst tahmindir: kaçan bir yol CPU'yu ya da belleği sınırsız
  tüketebilir; shell'in kare döngüsünden ayrı thread'de olsa da bellek shell'indir.
- **Bellek hatası** (C/`unsafe` içeren bağımlılıkta) shell'in adres alanında olur. Shell ise
  parola girişi, pano ve oturum kilidini taşıyor.
- **Bugünkü bedel:** uygulamaların kendi SVG ikonları (Electron/Flatpak tepsi ikonları,
  `IconThemePath`, `file://` bildirim ikonları) yedek ikonla görünüyor.

## Kararlar (onay 2026-10-12: hepsi ÖNERİLEN seçenekle)
- **K1 — Kapsam.** (a) Yalnızca SVG + JXL (+ AVIF açılırsa) yalıtımda, PNG/JPEG/WebP/GIF süreç
  içi (ilk taslak); **(b) ÖNERİLEN: her `Untrusted` çözüm yalıtımda.** Gerekçe: `panic = "abort"`
  ile süreç içi raster çözücünün paniği de shell'i kapatır; kural tekleşir ("güvenilmeyen bayt
  shell'in adres alanına girmez"). Maliyet küçük: ikon 64² = 16 KiB boru kopyası, bildirim
  görseli ≤ 1024² = 4 MiB; sıcak işçiyle istek başına < 1 ms + çözüm.
- **K2 — İşçi yokken (`Unavailable`).** (a) Kapalı-güvenli: tüm `Untrusted` görseller yedek
  ikon; **(b) ÖNERİLEN: SVG/JXL/AVIF asla süreç içine düşmez; yalnızca PNG/JPEG/WebP/GIF/QOI,
  `Unavailable` (işçi ikilisi yok / seccomp kurulamıyor) durumunda bugünkü süreç içi yola
  düşer**, bir kez uyarı loglanır. `Timeout`/`Killed`/`Protocol`/`RateLimited` ASLA süreç içine
  düşmez (zehirli girdi). Gerekçe: kurulum hatası tepsi ikonlarının tümünü silmesin; bugünkü
  güvenlik düzeyinin altına inilmez.
- **K3 — Dağıtım.** **ÖNERİLEN: ayrı ikili `nuxut-image-worker`** (aşağıda; `/proc/self/exe`
  reddedildi). ~~Tüketici workspace'inde 3 satırlık `[[bin]]`~~ — **2026-10-12 değişti:** işçi
  framework'ün ayrı paketi `nuxut-image-worker`; tüketici `[[bin]]` tanımlamaz, ayrı cargo
  çağrısıyla derler (`cargo build --release --manifest-path <framework>/Cargo.toml -p
  nuxut-image-worker`), böylece işçinin JPEG XL/seccompiler/landlock özellikleri tüketicinin
  ana ikilisine birleşmez (§Dağıtım). Kurulum yeri (onayda
  netleşti): kullanıcı kurulumunda (`install.sh`) `~/.local/lib/nuxut/nuxut-image-worker`,
  dağıtım paketinde `/usr/lib/nuxut/nuxut-image-worker`, geliştirmede ikilinin yanı. Arama
  sırası: `NUXUT_IMAGE_WORKER` → çalışan exe'ye göreli (aynı dizin, `../lib/nuxut/`) →
  `~/.local/lib/nuxut/` → `/usr/lib/nuxut/`.
- **K4 — Yeni bağımlılıklar.** `seccompiler` (rust-vmm) + `landlock`; `rustix` zaten ağaçta.
  **ÖNERİLEN: kabul** (sistem paketi yok, `unsafe` listesi değişmez).
- **K5 — Animasyon.** **ÖNERİLEN: güvenilmeyen kaynakta yalnızca ilk kare** (tepsi/bildirim
  ikonunda animasyon yok; animasyonlu bildirim görseli statik görünür).
- **K6 — Sıra.** **ÖNERİLEN:** framework işi (crate + işçi + düşmanca testler) → shell
  (`[[bin]]`, `Loader::with_sandbox`, `install.sh`) → bugünkü "Untrusted SVG reddi" yalnızca
  işçi kullanılabilirken kalkar (K2-b).

## Karar
Güvenilmeyen kaynaktan (`Trust::Untrusted`) gelen her görsel (K1-b) yalıtılmış bir yardımcı
süreçte çözülür. `Trust::System` (bizim seçtiğimiz tema ikonu, gömülü varlık) süreç içinde kalır.
~~Kullanıcının seçtiği duvar kağıdı da `System` sayılır ve süreç içinde kalır.~~ **2026-10-13
değişti:** duvar kağıdı artık işçide (aşağıda "Takip: duvar kağıdı").

### API (`nuxut-image`, değişiklik eki)
```rust
// Var: pub enum Trust { System, Untrusted }  — DecodeOptions/Limits içinde, varsayılan Untrusted
pub enum Isolation { Process, Unavailable }  // bu platformda/oturumda ne yapılabildiği (Linux dışı: Unavailable)
impl Loader { pub fn with_sandbox(self, sandbox: Arc<dyn IsolatedDecoder>) -> Self;  // Sandbox: IsolatedDecoder
               pub fn load_keyed(&mut self, key: u64, LoadRequest, done) -> LoadHandle; }  // aynı anahtarda eski iş iptal
// Loader sandbox'sız kurulursa bugünkü davranış (Untrusted SVG reddi + süreç içi raster) sürer.
// nuxut-image-sandbox
pub struct Sandbox { /* havuz, hız sınırı, negatif önbellek */ }
impl Sandbox {
    pub fn new(cfg: SandboxConfig) -> Self;              // süreç başlatmaz (tembel)
    pub fn isolation(&self) -> Isolation;
    pub fn decode(&self, key: u64, bytes: Arc<[u8]>, opts: &DecodeOptions) -> Result<Image, ImageError>;
}
pub fn worker_main() -> !;                               // yardımcı ikilinin tek satırı
```
`ImageError::Sandbox(SandboxFailure)` eklenir: `Timeout`, `Killed`, `Protocol`,
`Unavailable`, `RateLimited`. Hiçbir varyant bayt ya da yol taşımaz. Çağıran, hatada
**yedek ikonu** gösterir; tepside ve bildirimde bu `nuxut-ui` `Icon`'un semantik
`application`/`image-missing` adıdır (ADR-0006 ikon setleri). Süreç içine dönüş yalnızca K2-b
kapsamında.

### Dağıtım: ayrı ikili (`nuxut-image-worker`), `/proc/self/exe` yeniden çalıştırma değil
Yeniden çalıştırma (`/proc/self/exe --nuxut-image-worker`) sürüm uyumunu bedavaya getirir. Yine
de reddedildi:
- Uygulamanın `main`'i kancadan önce iş yapabilir: tek-örnek IPC, config okuma, log dosyası.
  Kanca unutulursa ya da geç kalırsa işçi ikinci bir shell başlatır.
- İşçi, shell'in bütün dinamik kütüphanelerini (libwayland, libxkbcommon…) ve büyük ikilisini
  eşler. seccomp listesi yükleyicinin ve bu kütüphanelerin başlatıcılarının çağrılarına göre
  genişlemek zorunda kalır.
- argv/ortamla işaretlenen ikinci kip, kullanıcının çalışan örneğiyle karışmaya açıktır.

Seçilen (ilk hâli: tüketici workspace'inde 3 satırlık `[[bin]]`; **2026-10-12'de değişti**):
framework'ün ayrı paketi `crates/nuxut-image-worker` (yalnızca ikili:
`fn main() { nuxut_image_sandbox::worker_main() }`; bağımlılık `nuxut-image-sandbox`
`worker` özelliği + `nuxut-image` `jxl`). Neden: tüketici workspace'inde tanımlanan `[[bin]]`
Cargo özellik birleştirmesiyle işçinin `jxl`'ini (ve seccompiler/landlock'u) tüketicinin ana
ikilisine ve kilit sürecine taşıyordu. Şimdi:
- `nuxut-image-sandbox` ana süreç tarafı (`Sandbox`, `protocol`, `locate`) jxl'siz; işçi
  modülü ve `worker_main` `worker` özelliğinin arkasında (seccompiler/landlock/libc yalnızca
  onunla). Doğrulama: `cargo tree -p nuxut-image-sandbox -e normal | grep jxl` boş;
  `-p nuxut-image-worker` ağacında jxl-oxide, seccompiler, landlock var.
- Tüketici kurulumu: `cargo build --release --manifest-path <framework>/Cargo.toml -p
  nuxut-image-worker` → `<framework>/target/release/nuxut-image-worker`'ı aşağıdaki konumlardan
  birine kopyalar. Ayrı cargo çağrısı olduğu için özellik birleşmez.
- Framework testleri: kancalı test işçisi `nuxut-image-sandbox` içinde
  `[[bin]] nuxut-image-worker-test` (`required-features = ["sandbox-test-hooks"]`,
  `CARGO_BIN_EXE_…`; farklı ad: `locate()` bulmaz, kancasız ebeveyn el sıkışmada reddeder);
  dağıtılan işçi `nuxut-image-worker` paketinin testinde kancasız ebeveynle sınanır (JXL dahil).
- ~~`System` güvenli JXL (kullanıcının duvar kağıdı) süreç içinde çözülür~~ (2026-10-13: duvar
  kağıdı işçide; `nuxut-image` varsayılanı jxl'siz). Süreç içi `System` JXL tüketicinin
  `nuxut-image` `jxl` özelliğini ister; yoksa `ImageError::Decode("JPEG XL desteği derlenmedi")`.
  `Untrusted` JXL süreç içinde zaten hiç çözülmez (yalnızca işçide).

libwayland/EGL linklenmez. Konum sırası (ilk düzenli + çalıştırılabilir dosya):
1. `NUXUT_IMAGE_WORKER` (yalnızca test/geliştirme; verilmişse yalnızca o denenir, yanlış yol
   sessizce başka ikiliye düşmez),
2. çalışan ikiliye göreli: `dirname(exe)/nuxut-image-worker`, sonra
   `dirname(exe)/../lib/nuxut/nuxut-image-worker`,
3. `~/.local/lib/nuxut/nuxut-image-worker` (kullanıcı kurulumu, `install.sh`),
4. `/usr/lib/nuxut/nuxut-image-worker` (dağıtım paketi).

Bulunamazsa `Unavailable`. Sürüm
uyumsuzluğunu el sıkışma yakalar: işçinin ilk yanıtı `magic + protokol sürümü +
crate sürümü`; uyuşmazsa `Unavailable`.

### İşçinin kendini kısıtlaması (ilk iş, girdi okumadan önce)
Ebeveynde `pre_exec` (fork sonrası `unsafe`) yok. İşçi başlar başlamaz şu sırayla kendini
kilitler. Herhangi biri başarısız olursa çıkış kodu 70 ile ölür:
1. ~~`close_range(3, ~0)`~~ — uygulanmadı (bkz. §Uygulama notları): miras fd'ler seccomp ile
   etkisizdir. stderr `/dev/null`'dır ve yazılamaz (panik kancası sessiz).
2. `setrlimit`:
   - `AS` 768 MiB. Gerekçe: güvenilmeyen hedef ≤ 1024² olduğundan tuval 4 MiB; katman bütçesi
     512 MiB ve ikili + yığın için pay.
   - `CPU` 4 sn yumuşak / 5 sn sert. Toplam CPU'dur, işçi geri dönüşümünü de sınırlar.
   - `FSIZE` 0, `NOFILE` 3, `NPROC` 0, `CORE` 0, `MEMLOCK` 0.
3. `prctl(PR_SET_NO_NEW_PRIVS, 1)` ve `prctl(PR_SET_DUMPABLE, 0)`.
4. Landlock (çekirdek ≥ 5.13; bu makinede açık): erişim kuralı olmayan kural kümesi, yani hiçbir
   dosya sistemi erişimi yok. Çekirdek desteklemiyorsa atlanır; seccomp zaten `open*`'u kapatır.
5. seccomp-bpf, varsayılan eylem `KILL_PROCESS`. İzin listesi:
   - `read`: yalnızca fd 0. `write`: yalnızca fd 1.
   - `mmap`/`mprotect`: `PROT_EXEC` olmadan (maskeli argüman denetimi). `munmap`, `mremap`,
     `brk`, `madvise`.
   - `futex`, `sigaltstack`, `rt_sigreturn`, `rt_sigprocmask`, `exit`, `exit_group`,
     `clock_gettime`, `getrandom`.

   `open*`, `socket`, `clone*`, `execve*`, `ptrace` ve `ioctl` yok. Filtre `seccompiler` ile
   derlenir: saf Rust, unsafe'siz API.

Bu sırayla bizim kodumuzda `unsafe` gerekmez: rustix, seccompiler ve landlock güvenli API
sunar. Böylece proje kurallarındaki `unsafe` listesi değişmez.

### Protokol (stdin/stdout, çerçeveli, ikili)
- İstek (uygulanan, LE, 40 bayt başlık): `u32 len | u8 kind (SniffedFormat kodu) | u8 fit
  (0 = hedefsiz) | u8 hook (dağıtılan işçide 0) | u8 0 | u32 tw | u32 th | u32 max_w | u32 max_h |
  u64 max_pixels | u64 max_out_pixels | bytes`. Girdi en fazla 8 MiB'dir (`MAX_SVG_BYTES`);
  geçersiz başlık → işçi 65 ile çıkar.
- Yanıt (12 bayt başlık): `u8 status (0 ok, 1 hata) | u8 hata sınıfı | u16 0 | u32 w | u32 h`,
  ok ise ardından `w·h·4` bayt premultiplied RGBA. Hata sınıfı (Empty, UnknownFormat, TooLarge,
  Svg, …) metin taşımaz; ebeveyn sabit iletili `ImageError`'a eşler.

Ebeveyn, işçiyi **ele geçirilmiş** sayar:
- `w ≤ tw`, `h ≤ th` (Cover'da kutuyu kaplama gereği bir eksen yeterli), `w ≤ max_w`,
  `h ≤ max_h`, `w·h ≤ min(max_pixels, max_out_pixels)` ve uzunluk tam `w·h·4` olmalı; aksi
  `Protocol` olur ve işçi öldürülür.
- Tampon yalnızca bu denetimden sonra ayrılır: en büyüğü `max_out_pixels·4` (varsayılan 64 MiB).
- Fazla bayt da kısa okuma da `Protocol`'dür.

Animasyon yalıtıma girmez: güvenilmeyen kaynakta yalnızca ilk kare çözülür (bildirim/tepsi
ikonunda animasyon yok).

### Havuz, soğuk başlangıç, eşzamanlılık
- Ölçüm: süreç başlatma bu VM'de (6 çekirdek, yük ~33) ~6.6 ms duvar / ~0.9 ms CPU
  (`/usr/bin/true` ×200). Rust işçisi ve seccomp/landlock kurulumu için hedef ≤ 10 ms.
  **Gerçek ölçüm** (`examples/sandbox_bench.rs`, dev profili + bağımlılıklar opt 2, yük ~9):
  soğuk (süreç + kilit + el sıkışma + 64² SVG) medyan 1.6 ms, en kötü 2.8 ms; sıcak istek
  138 µs (aynı çözüm süreç içinde 98 µs).
- ~~**Sıcak işçi, sıralı yeniden kullanım.**~~ **Güvenlik incelemesiyle değişti (bulgu 3): işçi
  tek kullanımlıktır.** Her istek girdi görmemiş taze bir işçiye gider, yanıttan sonra işçi
  öldürülür (ele geçirilmiş işçi başka kaynağın yanıtını taklit edemez). En fazla 2 yaşayan
  işçi (meşgul + önceden ısıtılmış taze + öldürülmeyi bekleyen). Öldürme ve yerine taze işçi
  başlatma bir temizlikçi thread'de, çağıranın yolu dışında; 30 sn kullanılmayan taze işçi
  kapatılır, temizlikçi iş kalmayınca çıkar: boşta iş, süreç ve thread yok. 64 istek / 3 sn
  meşguliyet sınırları (`max_requests_per_worker`, `max_busy_per_worker`) kalktı. Ölçüm (yük
  ~12): soğuk 1.7 ms medyan; aralıklı istek (taze işçi hazır) 0.55 ms medyan; sıkı döngü
  1.45 ms; süreç içi aynı çözüm 0.13 ms.
- **Zaman aşımı**: istek başına duvar saati 1.5 sn (`SandboxConfig::timeout`). Uygulamada
  okuyucu thread yok: ebeveyn borular bloklamasız, her okuma/yazma tek son tarihe karşı `poll`
  ile bekler; süre dolunca `Child::kill` + `wait` çağrılır, zombi kalmaz.
- **Hız sınırı**: belirteç kovası, 30 iş/sn, patlama 60. `key` (çağıranın kaynak kimliği; tepside
  öğe, bildirimde uygulama) başına en fazla 1 bekleyen iş vardır. Yenisi gelince eskisi
  iptal edilir: `NewIcon` fırtınası tek işe iner. Kova boşsa `RateLimited` döner ve çağıran
  yedek ikonu gösterir.
- **Negatif önbellek**: zaman aşımına uğrayan ya da işçiyi öldüren girdinin özeti (`blake3`,
  ilk 16 bayt) 256 girdilik LRU'ya girer. Aynı bayt yeniden gelirse işçi başlatılmadan
  `Killed` döner: kötü niyetli bir ikon her yenilemede süreç doğuramaz. `key` başına art arda
  3 başarısızlıkta o anahtar 60 sn susturulur.

### Linux dışı (ADR-0003)
`nuxut-image-sandbox` her platformda derlenir. Süreç yalıtımı `#[cfg(target_os = "linux")]`
arkasındadır. **Güvenlik incelemesiyle değişti (bulgu 1a):** diğer platformlarda `isolation()`
→ `Unavailable` döner (eski `InProcess` kipi ve `Isolation::InProcess` kaldırıldı): SVG/JXL/AVIF
yedek ikon, yalnızca kapılardan geçmiş temel raster süreç içinde, ilk kare (K2-b). macOS
`sandbox_init` / Windows AppContainer ihtiyaç doğunca eklenir. Linux'ta seccomp ya da landlock
kurulamazsa (eski çekirdek, kısıtlı kapsayıcı) işçi çıkış kodu 70 ile ölür ve `isolation()` →
`Unavailable` döner.

### Log
İşçi hiçbir şey yazmaz (stderr `/dev/null`). Ebeveyn yalnızca hata türünü, süreyi, bayt
uzunluğunu ve çıkış kodunu/sinyali loglar. Yol, URL, uygulama adı ve `key` loglanmaz
(değişmez 9).

## Düşmanca test planı
Test kancaları yalnızca `cfg(test)` / `sandbox-test-hooks` özelliğinde derlenir; işçiye
istek türü olarak verilir.
1. **Kaynak**:
   - sonsuz döngü → zaman aşımı, işçi ölü (`ps`), havuz toparlanır;
   - 2 GiB ayırma → `AS` ile ENOMEM/abort, ebeveyn `Killed` alır;
   - `RLIMIT_CPU` aşımı → SIGXCPU.
2. **seccomp**: işçi içinden `open("/etc/passwd")`, `socket`, `fork`, `execve`, `PROT_EXEC`
   mmap, fd 3'e `write` ve dosya yazımı denenir. Her biri SIGSYS ile ölür; ebeveyn `Killed`
   alır; dosya oluşmaz.
3. **Protokol sahteciliği**: `w·h` uyuşmazlığı, `w > tw`, 4 GiB beyan, eksik ya da fazla
   bayt, yanlış magic/sürüm. Hepsi `Protocol` döner, işçi öldürülür, ebeveynin belleği sınırlı
   kalır.
4. **Taşma**: stdout'a sonsuz yazım (okuyucu üst sınırda keser) ve stderr taşması (uygulamada
   stderr'e yazmak seccomp'ta yasak → SIGSYS).
5. **Sızıntı**: 1000 iş sonra ebeveynde `/proc/self/fd` sayısı ve çocuk süreç sayısı sabit,
   zombi yok.
6. **Fırtına**: aynı `key` ile 500 istek → en fazla 1 çözüm; 100 farklı `key` → en fazla 2
   eşzamanlı işçi, kova boşalınca `RateLimited`.
7. **Negatif önbellek**: aynı zehirli bayt ikinci kez işçi başlatmaz.
8. **Gerçek girdiler**: svg.rs düşmanca test külliyatı işçi üzerinden; kabul edilenler
   süreç içi sonuçla piksel piksel aynı.
9. **Kullanılamazlık**: `NUXUT_IMAGE_WORKER` var olmayan yol → `Unavailable`, yedek ikon,
   süreç içine düşme yok.
10. **Linux dışı**: cross-check derlemesi; işçisiz SVG asla süreç içinde değil (birim testi).

Uygulama durumu (`crates/nuxut-image-sandbox/tests/it/adversarial.rs`, gerçek işçi ikilisiyle;
kancalar `sandbox-test-hooks`, kendine dev-bağımlılıkla yalnızca testte açık): 1–9 sınanıyor;
ek olarak canlı işçinin `/proc/<pid>/status` (`Seccomp: 2`, `NoNewPrivs: 1`), `/proc/<pid>/limits`
ve boşta `S` (uyuyor) durumu doğrulanıyor, landlock tek başına (seccomp kapalı kipte) dosya
okuma/yaratma, TCP bağlantısı, thread ve süreç başlatmayı reddediyor. `PROT_EXEC` mmap ve fd 3'e
yazım çalışma zamanında sınanmıyor (denemek `unsafe` ister); filtrenin derlendiği ve tehlikeli
çağrıların listede olmadığı birim testinde. `Loader` fırtınası (aynı anahtarla 500 istek → ≤ 2
çözüm, yalnızca sonuncusu sonuç verir) `nuxut-image` birim testinde sahte çözücüyle. 10:
`tools/cross-check.sh nuxut-image-sandbox` Windows/macOS TAMAM + işçisiz yol birim testi.

## Uygulama notları (sapmalar)
- **`close_range` yok.** rustix 1'de yok, fd kapatmak `unsafe` ister (bu crate'te yasak).
  Miras fd'ler seccomp ile etkisizdir: `read` yalnızca fd 0, `write` yalnızca fd 1, `mmap`
  yalnızca anonim (fd argümanı -1) ve `PROT_EXEC`'siz; fd alan başka çağrı (`close`, `dup*`,
  `fcntl`, `ioctl`, `sendmsg`, `pread`…) listede yok.
- **`PR_SET_PDEATHSIG` kullanılmaz:** işçiyi başlatan *thread* bitince tetiklenir; havuzdaki
  işçi başka thread'den kullanılırken ölürdü. Ebeveyn ölünce boştaki işçi stdin EOF ile çıkar,
  çözüm ortasındaki `RLIMIT_CPU` ile biter.
- **Landlock kapsamı genişletildi:** dosya sistemi + TCP bağla/dinle + soyut UNIX soketi ve
  sinyal kapsamı (ABI v6'ya kadar en iyi çaba). `NOFILE` 3, landlock kural kümesi fd'si
  kapandıktan sonra kurulur. İzin listesine `sched_yield` ve `restart_syscall` eklendi.
- **El sıkışma kilitten sonra** yazılır: gelmesi "işçi kilitlendi" demektir. Çıkış 70
  (kilit kurulamadı) ya da imza/sürüm uyuşmazlığı → `Unavailable`, 60 sn sonra yeniden denenir.
- **Geri dönüşüm** "her hatadan sonra" çözme hatalarını da kapsar (işçinin bildirdiği bozuk
  görsel dahil). Anahtar sayacı da çözme hatalarını sayar; negatif önbellek yalnızca
  `Timeout`/`Killed`/`Protocol`'ü. CPU geri dönüşümü istek duvar saati toplamıyla ölçülür
  (tek thread'li işçide CPU'nun üst sınırı; `/proc` okunmaz).
- **İmzası tanınmayan bayt** (`SniffedFormat::Other`) işçi başlatılmadan `UnknownFormat` olur.
- **Boşta kapatıcı** ayrı bir thread'dir ama yalnızca boşta işçi varken yaşar (son işçiyle çıkar):
  boştayken süreç de thread de yok.
- **`nuxut` cephesi** sandbox'ı yeniden dışa aktarmaz; isteyen tüketici doğrudan bağımlanır
  (işçi ikilisi zaten crate adıyla çağrılır).

## Güvenlik incelemesi düzeltmeleri (2026-10-12)
Dört bulgu teyit edildi ve düzeltildi; her biri için düşmanca test var (gerçek işçiyle olanlar
`tests/it/adversarial.rs`).
1. **Fail-open.**
   a. Linux dışında `InProcess` kipi SVG/JXL/AVIF'yi süreç içinde çözüyordu → kaldırıldı; Linux
      dışı `Unavailable` (K2-b). Genel API: `Isolation::InProcess` yok.
   b. `Unavailable` denetimi kapılardan önceydi; negatif önbellekteki (işçiyi öldürmüş) ya da
      susturulmuş anahtarın PNG'si süreç içine düşebiliyordu → kapılar (susturma, negatif
      önbellek, belirteç kovası) her yolda, yedekten önce. Test: işaretlenmiş `Unavailable`'da
      zehirli PNG `Killed`, susturulmuş anahtar `RateLimited`, temiz PNG yedekle çözülür.
   c. Landlock "en iyi çaba"ydı → hiç uygulanamazsa (`NotEnforced`) işçi 70 ile çıkar; kısmi kabul.
2. **Yalıtım atlatma.**
   a. Ebeveyn yalnızca imza/sürüme bakıyordu → el sıkışma sürüm 2: bayraklar (seccomp, test
      kancaları) + landlock düzeyi; ebeveyn seccomp'u ve landlock ≥ 1'i zorunlu tutar, kancalı
      işçiyi kendisi kancalı derlenmemişse reddeder. Test: gerçek seccomp'suz işçi ve iki sahte
      işçi (seccomp'suz, landlock'suz) `Unavailable`; aynı sahte işçi kilit beyanıyla kabul edilir
      (pozitif kontrol); kabul kuralı birim testi (kancalı işçi + kancasız ebeveyn → red).
   b. `#[cfg(all(feature = "sandbox-test-hooks", not(debug_assertions)))] compile_error!`:
      sürüm derlemesinde kanca olamaz (framework'te `cargo test --release` yok; doğrulandı:
      `cargo check --release --features sandbox-test-hooks` derlenmez).
   c. `decode_isolated` süreç içinden çağrılabiliyordu → süreç geneli işçi kipi
      (`nuxut_image::enter_worker_mode()`, `worker_main` kilitten SONRA çağırır); kip yoksa
      `Unsupported`. Test: `nuxut-image` testlerinde kip kapalı, SVG ve PNG reddedilir.
   d. Miras fd'ler: `close_range` yok, `unsafe` kuralı aynı — **bilinen**: kapatılmaz, seccomp
      ile etkisizdir. Ek olarak işçi kilitten önce miras fd SAYISINI el sıkışmada bildirir,
      ebeveyn > 0 ise debug log'a yazar (yol/içerik yok), `SandboxStats::inherited_fds`. Test:
      CLOEXEC'siz açılan fd sayıyı artırır.
3. **Kaynaklar arası taklit.** Havuzdaki işçi birden çok kaynağa hizmet ediyordu → işçi tek
   kullanımlık (yukarıda §Havuz). Test: ardışık ve paralel isteklerde hiçbir PID iki kez
   görülmez; her istekten sonra yaşayan çocuklar yalnızca girdi görmemiş taze işçilerdir.
4. **Loader sandbox atlatma.**
   a. Sandbox'sız `Untrusted` süreç içi yol JXL/AVIF'yi çözüyordu → güven kapısı yalnızca
      `in_process_fallback()` biçimlerini (PNG/JPEG/WebP/GIF/QOI) geçirir; JXL/AVIF/SVG
      `Unsupported`, imzasız `UnknownFormat`.
   b. K5 süreç içinde de: `Untrusted` → `animate` yok sayılır, yalnızca ilk kare.
   c. Kullanıcı dizinindeki hyprcursor PNG kareleri süreç içindeydi → sandbox varsa raster de
      işçiye gider (tek kare); yoksa süreç içinde PNG, ilk kare.
   d. Anahtarsız (`NO_KEY`) istek susturulmuyordu → Loader kaynaktan anahtar türetir
      (`derive_key`: yol, ikon adı, imleç teması+rolü), bayt kaynağı ve doğrudan anahtarsız
      çağrılar `ANON_KEY` kovasına düşer; kova da susturulur.

## Takip notları (2026-10-12)
- **Boştaki işçiler:** boşta en çok `SandboxConfig::prewarm` (varsayılan **1**) girdi görmemiş
  taze işçi bekler ve `idle_timeout` (30 sn) sonra kapanır; **girdi görmüş işçi asla yaşamaz**.
  Settings'in bir çözümden sonra 7 sn boyunca gördüğü 2 süreç önceden ısıtılmış taze
  işçilerdi (ön ısıtma o zaman `max_workers` = 2'ye kadardı). Doğrulama
  (`idle_fresh_workers_are_bounded_unused_and_reaped`): eşzamanlı 9 istekten sonra boşta
  kalanların PID'leri hizmet edenlerden ayrık, sayısı ≤ `prewarm`, `idle_timeout` sonra çocuk
  süreç 0. Karar `prewarm = 1`: tepsi/bildirim çözümleri çoğunlukla sıralı; ikinci eşzamanlı
  istek soğuk başlar (~1.7 ms), buna karşılık boştaki her işçi ~7 MB RSS tutar (debug ölçümü).
  `prewarm: 0` ön ısıtmayı kapatır.
- **`<image href>` hiçbir şey açmaz (kesin):** usvg seçenekleri tek fonksiyonda
  (`svg::usvg_options`): `resources_dir: None`, dize çözücüsü her şemada açmadan `None`
  (usvg kütüphanesindeki tek dosya okuması varsayılan dize çözücüsündedir ve kurulmaz), `text`
  derlenmez (işçi ağacında fontdb yok). `data:` PNG/JPEG/GIF/WebP ≤ 4 MiB, başlıkta ≤ 4096
  kenar ve ≤ 16 Mpx ise sınırlarla çözülür; sınır dışı raster ve `data:image/svg+xml` (iç içe
  SVG) reddedilir. Testler: çözücü birim testi (`file:`, var olan dosyanın yolu, `../x.png`,
  `x.png`, `/dev/zero`, `http(s)`, `ftp`, `smb`, `//host`, `javascript:`, `jar:` …); işçi
  kipinde süreç içi çıktı `<image>`'sız SVG ile bayt bayt aynı (data: PNG pozitif kontrol:
  çizilir); gerçek işçi aynı külliyatı ölmeden yanıtlar (`failures == 0`; açma girişimi
  seccomp ile `Killed` olurdu).
- **İptal ≠ kaynak anahtarı:** `Loader::load_with_keys(LoadKeys { cancel, source })`.
- **İşçi ayrı paket** `nuxut-image-worker` (§Dağıtım).

## Bağımlılıklar (DEPENDENCIES.md'ye uygulamayla)
| Crate | Lisans | Neden | Alternatif |
|---|---|---|---|
| rustix 1 (`process`, `thread`, `fs`, `event`, `time`) | Apache-2.0/MIT | setrlimit, prctl, bloklamasız boru + `poll` güvenli API (close_range rustix 1'de yok); ağaçta zaten var (calloop/tempfile) | nix (ağaçta, ama `unsafe` gerektiren yollar çok); libc (her çağrı `unsafe`) |
| seccompiler 0.5 | Apache-2.0/BSD-3 | seccomp-bpf'i saf Rust derler ve kurar (rust-vmm, Firecracker) | libseccomp (C kütüphanesi, Arch `libseccomp`); elle BPF (hata riski) |
| landlock 0.4 | MIT/Apache-2.0 | dosya sistemi erişimini çekirdekte sıfırlamak; eski çekirdekte sessizce etkisiz | yalnızca seccomp (yeterli; landlock ikinci katman) |
| blake3 | CC0/Apache-2.0 | negatif önbellek özeti; ADR-0004 ile zaten geliyor | std `DefaultHasher` (çarpışmaya dayanıksız) |
| libc 0.2 | MIT/Apache-2.0 | yalnızca seccomp kuralları için sistem çağrısı sabitleri; ağaçta zaten var | sabitleri elle yazmak |

Sistem paketi gerekmez.

## Sonuçlar
- Kötü niyetli bir ikon en kötü ihtimalle yedek ikonla sonuçlanır. Shell çökmez ya da donmaz,
  dosya okunamaz, ağa çıkılamaz.
- Maliyet:
  - ilk güvenilmeyen SVG'de ~10 ms süreç başlatma, sonrasında yalnızca boru kopyası (64²
    ikon için 16 KiB);
  - boştayken süreç yok.
- Shell'de değişiklik: tepsi, bildirim ve MPRIS yükleyicileri zaten `Untrusted` (varsayılan);
  `Loader::with_sandbox` ile kurulur, `[[bin]] nuxut-image-worker` eklenir, `install.sh` ikiliyi
  kurar (K3, K6; kabuğa iletilecek). Bugün yedek ikonla görünen uygulama SVG ikonları
  görünür hâle gelir.

## Takip: duvar kağıdı (2026-10-13, uygulandı: framework)
**`System` duvar kağıdı artık işçide.** karar: duvar kağıdı kullanıcı dosyasıdır,
internetten inmiş olabilir; tüm biçimler (JXL dahil) güvenden bağımsız yalıtılmış işçide çözülür;
shell/lock/settings ana ağaçlarına jxl-oxide girmez. "`System` (kullanıcının duvar kağıdı) süreç
içinde" kuralı bununla kalktı; `Trust::System` yalnızca bizim seçtiğimiz tema ikonları ve gömülü
varlıklar içindir. Ayrıntı: ARCHITECTURE "Duvar kağıdı yalıtımı"; tasarım ve gerekçeler
`docs/design/wallpaper-sandbox.md`.
- **Profil** (işçi süreci başına, el sıkışmada beyan): `WorkerProfile::Wallpaper` — 256 MiB akışlı
  girdi, 16K kenar, 1.5 GiB çözücü bütçesi, AS 2.5 GiB, CPU 10/12 sn, 20 sn duvar saati, zaman aşımı
  negatif önbelleğe girmez, nice 10, `oom_score_adj` 1000. İkon profili gevşemedi. Protokol sürüm 3.
- **S1:** işçi yokken K2-b aynen: temel raster süreç içinde (`Untrusted` kapısı, ilk kare,
  aynı kapılar), süreç başına tek uyarı; JXL/AVIF/SVG düz zemin rengi.
- **S2:** animasyonlu duvar kağıdı → ilk kare + `animated_source()` (K5 ile tutarlı);
  `Wallpaper`'ın süreç içi animasyon oynatımı kalktı. Animasyon `nuxut-video` içe aktarımıyla.
- **S3:** `Wallpaper::new(fit, motion, decoder: Arc<WallpaperDecoder>, notify)` kırıcı,
  zorunlu parametre (builder unutulup sessizce süreç içi çözüme düşülemez).
- **Genel API:** `IsolatedDecoder::decode_file(key, File, opts, &CancelFlag)` + `open_user_file` +
  `file_key(domain, path)`: lock avatarı ve `nuxut-video` `Importer` durağan görseli de bunu
  kullanacak (açık iş).

## Takip: içe aktarma (2026-10-13, uygulandı: framework)
**`nuxut-video` `Importer`ın görsel yolu işçide.** Duvar kağıdı takibinin "aynı sınıf" bulgusu
(`import.rs` durağan görseli `Trust::System` ile süreç içinde çözüyor, `is_animated`/
`animation_info` kabı süreç içinde yürüyordu) kapandı. Ayrıntı: `docs/design/video-import-sandbox.md`,
ARCHITECTURE "Video ve duvar kağıdı içe aktarma".
- **Tek ayrıştırma işçide:** ana süreç yalnızca 4 KiB imzaya bakar; durağan görselin çözümü,
  animasyon tespiti ve kare sayısı/süre tek `decode_file` çağrısında (duvar kağıdı profili).
- **Protokol sürüm 4:** yanıt başlığı 20 bayt; animasyonlu kaynakta işçi kare sayısını ve tur
  süresini kap yapısından sayıp gönderir (`DecodedFile::animation`). Ebeveyn tutarlılığı denetler
  (yalnızca `ANIMATED_SOURCE` ile; kare 0/süre 0 ya da kare ≥ 2; hata yanıtında 0). Değer yalnızca
  ön denetimdir; kesin sınır mpv'nin `frames=`/`end=` sayacı.
- **Çözücü zorunlu** (S3 ile aynı ilke): `Importer::new(decoder)` / `with_dir(dir, decoder)` kırıcı;
  süreç içi yol yalnızca adıyla uyaran `Importer::in_process_unsandboxed` (örnek/test).
- **Hata yedeğe düşmez:** işçi hatası `ImportError::Image` olarak döner; işçi yokken S1 (temel raster
  K2-b, SVG/JXL/AVIF asla) `Sandbox`'ın kendisinde uygulanır, `Importer` ayrı yedek yazmaz.
- Kapsam dışı (değişmedi): animasyon/video baytlarını mpv süreç içinde çözer (ADR-0002 riski).
