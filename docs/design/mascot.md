# Maskot: Fiyu

Bileşen: `nuxut_ui::Mascot` (`crates/nuxut-ui/src/mascot.rs`, `brand` özelliği). Varlıklar:
`crates/nuxut-ui/assets/mascot/` (kaynak `.team/mascot/genis/`, geometri aynen kopyalandı).

## Kararlar

| Karar | Kim, ne zaman |
|---|---|
| Ad **Fiyu**; oran "Geniş" (gövde elipsi 76×86, gözler r22, göz bebekleri r12.5; `oranlar/5-genis`) | kullanıcı, 2026-10-13 |
| Genel gizleme ayarı: `framework.toml [appearance] mascot = true` (varsayılan açık) | kullanıcı, 2026-10-13 |
| Varsayılan döküm (`FrameworkConfig::default()` serileştirmesi, Ayarlar varsayılan tablosu) diğer tablolar gibi `[appearance] mascot = true` taşır; kullanıcının mevcut dosyasına kendiliğinden yazılmaz (bölümsüz dosya aynen yüklenir, Ayarlar yalnız değişen anahtarı yazar) | istek, 2026-10-14 |
| Bileşen framework'te (`nuxut-ui`), tüketici shell/Ayarlar/Hoş geldin | 2026-10-13 |

## Ruh hâlleri ve yerleşim kuralı

| `Mood` | Dosya | Nerede |
|---|---|---|
| `Empty` | `empty` | Boş durumlar: bildirim yok, panel boş, pano boş. |
| `NoResults` | `no-results` | Başlatıcıda / aramada sonuç yok. |
| `Error` | `error` | Bağlantı yok, servis ulaşılamadı. |
| `Celebrate` | `celebrate` | Hoş geldin son adımı ("Hazırsın"), güncelleme bitti. |
| `Hello` | `hello` | Hakkında sayfası, Hoş geldin ilk adımı. |
| `Rest` | `rest` | Uzun boşta kalma, gece (isteğe bağlı boş durum çeşidi). |
| `Thinking` | `thinking` | Yükleniyor / bekleme (isteğe bağlı; canlanmaz). |

**Yalnızca** şu yüzeylerde: boş durum, sonuç yok, hata, Hoş geldin ilk/son adım, Hakkında,
güncelleme bitti (iç bildirim kartı; dış uygulama bildirimlerinde asla — "Bildirim yerleşimi").

**ASLA**: bar, kilit ekranı, parola/kimlik doğrulama diyalogları, uygulama ikonu (uygulama ikonu
marka işaretidir, `BrandMark`). Gerekçe: maskot sakin bir eşlikçidir; güvenlik ve sürekli görünen
yüzeylerde dikkat dağıtır ve ciddiyeti azaltır, ikon rolünde markayla karışır.

## Boyut

Mantıksal kenar **48–160 px** (`MASCOT_MIN_SIZE`..=`MASCOT_MAX_SIZE`; dışı sıkıştırılır,
varsayılan 96). 48 px'te göz ve gaga nokta olarak okunur, konfeti/yağmur toza döner (rahatsız
etmez); daha küçük boyda yüz okunmaz — orada maskot kullanılmaz. Önerilen: boş durum ve sonuç yok
96, hata 96, Hoş geldin/Hakkında 128–160. Raster fiziksel boyuttadır (kenar × ölçek, piksele
hizalı); ölçek değişince yeniden üretilir.

## Boş durum yerleşimi

Bileşen: `nuxut_ui::EmptyState` (`crates/nuxut-ui/src/empty_state.rs`). Talep: kabuk,
2026-10-13 (shell'deki geçici `widgets/empty_state.rs` geometrisi framework'e taşındı; tüketiciler
kendi yerleşimini yazmaz). Uygulama.

- Alanda yatay ve dikey ortalı tek sütun: maskot (ya da ikon), `space(4)`, başlık (`Title`,
  ortalı, en çok 2 satır), `space(1)`, gövde (`Body`, `on_surface_variant`, ortalı, en çok 4
  satır), `space(4)`, isteğe bağlı eylem düğmesi (`Tonal`). Kenar boşluğu `space(4)`; metin
  genişliği en çok `space(90)` (okunur satır boyu).
- Maskot kenarı 96 (`EMPTY_MASCOT_SIZE`; `with_mascot_size` ile Hoş geldin/Hakkında 128–160).
  Alan darsa küçülür, `MASCOT_MIN_SIZE` (48) da sığmazsa düşer: yalnız metin, kendi başına ortalı,
  aralık da kalkar. Gerekçe: 48'in altında yüz okunmaz (yukarı "Boyut").
- `[appearance] mascot = false`: maskot yer kaplamaz, aynı düşme kuralı; varsayılan olarak ikona
  geçilmez (kullanıcı görsel istemedi, boşluk da kalmaz). **Yedek ikon** (`with_fallback_icon(theme,
  Icon)` / `set_fallback_icon(theme, Option<Icon>)`) verildiyse maskotun yerine o ikon çizilir:
  "gizliyken eski görünüm" — ör. bildirim merkezinin sessiz kip ikonu, maskot gelmeden önce orada
  duran görsel. Yerleşim ikon kipiyle birebir aynı (`IconLarge`, `on_surface_variant`, ölçeklenmez,
  sığmazsa düşer); maskot görünürken yedek ikon çizilmez; maskotsuz/ikonlu kipte etkisizdir; tema
  değişiminde canlı geçer. Karar: 2026-10-14 (talep; önceki "ikona hiç
  geçilmez" kuralı varsayılan olarak kalır). Uygulama.
- İkon kipi (`with_icon`): maskotun uygun olmadığı yüzeyler için; ikon ölçeklenmez, sığmazsa düşer.
- Sütun alandan uzunsa üstü alanın iç üstüne yaslanır (başlık hiçbir zaman yukarıdan kesilmez).
- Ruh hâli eşlemesi yukarıdaki tablodur (boş → `Empty`, sonuç yok → `NoResults`, hata → `Error`).
- Yerleşim çizimde yapılır: tema, ölçek ve gizleme değişimi ek çağrı istemez. Canlanma yok.

## Bildirim yerleşimi

Bileşen: `NotificationCard::with_mascot(theme, Mood)` / `set_mascot(theme, Option<Mood>)`
(`crates/nuxut-ui/src/notification.rs`). Talep: 2026-10-14 (shell "Nuxut güncellendi"
bildiriminde maskotu görsel yuvasına saydam doku + `surface_bright` karo üstüne koyuyordu; kaydırma
solması ve dolgu varsayımı kopyalanıyordu). Uygulama.

- **Yalnız Nuxut'un kendi iç bildirimleri**: güncelleme bitti (`Celebrate`) ve benzeri sistemin
  kendi olayları. **Dış uygulama bildirimlerinde (D-Bus `org.freedesktop.Notifications` ile
  gelen) maskot YOK** — kart başka bir uygulamanın sesidir; maskot orada Nuxut'u o uygulamanın
  ağzına koyar ve görsel yuvasıyla (uygulamanın resmi) karışır. Shell iç/dış ayrımını kaynakta
  yapar; framework bunu bilmez.
- Maskot metin sütununun **solunda**, başlık satırının altında, kart dolgusuna yaslı; **karo
  yok** (maskot kart zemininde durur; renkleri `surface_container`'a karşı ≥ 3:1 garantili).
  Metin maskotun `space(3)` sağında başlar; kart en az maskot boyu kadar uzar.
- Kenar **64** (`NOTIFICATION_MASCOT_SIZE`): istenen 64–96 aralığının alt ucu. Gerekçe: 360–420 px
  kartta metin sütunu ≥ 240 px kalır; başlık + iki satır gövdeyle aşağı yukarı aynı boy, kısa
  bildirimi gereksiz uzatmaz; `MASCOT_MIN_SIZE` (48) üstünde yüz okunur. Sabit; kart daralırsa
  maskot küçülmez (bildirim genişliği sabittir).
- Görsel yuvası (sağda `space(14)` kare, nötr zemin) değişmez; maskotla birlikte bulunabilir.
- `[appearance] mascot = false`: yuva hiç yok — metin sola yaslanır, yükseklik yalnız metne göre;
  ikona ya da boşluğa geçilmez. `set_theme` ile canlı.
- Kartın kaydırma/kapanma solması ve yığının giriş alfası maskota da uygulanır; maskot kaydırmayı
  izler. Canlanma yok, boşta kare yok (doku inene kadar hariç).

## Renk rolleri

SVG'lerde renk yoktur; sınıflar `mascot_colors(&Palette)` ile tema rollerinden türetilir
(sabit hex yok):

| Sınıf | Koyu zemin | Açık zemin |
|---|---|---|
| `m-body` (gövde, kuyruk, yağmur) | `brand_shade(primary)` | `primary`, ön kanada ≥ 2,2:1 olacak kadar koyulaştırılmış |
| `m-body m-wing-back` (arka kanat) | gövdeyle aynı | gövdeyle aynı |
| `m-wing-front` (ön kanat, göz akı, süsler) | `primary` (gövdeye ≥ 1,6:1; yetmezse önce ön kanat açılır, sonra gövde koyulaşır) | `primary`'nin HCT tonu, zemine ≈ 3,3:1 veren en açık ton |
| `m-detail` (göz bebeği, kapalı göz yayı) | `surface`/`on_surface`'tan koyusu | aynı kural |
| `m-beak` (gaga) | `surface` (= göz bebeği) | `surface`, gövdeye ≥ 3:1 (açık gaga) |

Ardından garanti: ön kanat ve gövde `surface` ve `surface_container`'a karşı **≥ 3:1** (WCAG
1.4.11 metin dışı; `ensure_contrast`, hue/kroma korunur), göz bebeği göz akına karşı ≥ 3:1, ön
kanat/gövde ayrımı koyuda ≥ 1,6:1, açıkta ≥ 2,2:1. Testli: statik koyu/açık × standart/yüksek
karşıtlık ve 12 kaynak renk × tüm varyantlar × iki kip × standart/yüksek üretilmiş palet.
Yüksek karşıtlık için ayrı kural yoktur: framework politikası metin dışı öğede de 3:1'dir ve
palet zaten `Palette::high_contrast`'tan geçer (accessibility.md §1).

Gerekçe:
- Koyu kipte ikili ton marka işaretiyle (`BrandMark::duotone`) aynıdır: aynı aile.
- README'nin bilinen zayıflığı: önizlemedeki açık ön kanat (#8FB6F6 / #F5F7FC zemin) ≈ 1,9:1
  kalıyordu. `primary_container` rolü paletimizde yok; ön kanat `primary`'den HCT tonuyla
  zeminde okunan en açık tona (≈ 3,3:1) açılır. Orta tonda kalmak zorunda olduğundan gövdeyle
  ayrımı gövdenin koyulaşması sağlar: ilk denemede (ön kanat/gövde 1,8:1) canlı ekran
  görüntüsünde göz akı gövdeye karışıyordu; 2,2:1'de göz okunuyor (2026-10-13).
- Göz bebeği her kipte koyudur (`on_surface` koyu kipte açık olduğundan göz akı üstünde
  kaybolurdu); koyu kipte `surface` ile aynı ton, mono sürümdeki "delik" yüzle tutarlıdır.
- Gaga ayrı sınıftır (`m-beak`, 2026-10-16, görev): önceden göz bebeğiyle
  aynı sınıftaydı ve açık kipte koyu gövde üstünde ≈ 2:1 kalıyordu. Açık kipte koyu gaga 3:1'e
  matematiksel olarak ulaşamaz: ön kanat zemine 3:1 için orta tonda (göreli parlaklık ≈ 0,25),
  gövde ondan 2,2:1 koyu (≤ 0,09) → siyah bile gövdeye < 3:1. Gaga bu yüzden monodaki "delik"
  gibi `surface` rengindedir: gövde zemine zaten ≥ 3:1 olduğundan gaga/gövde de ≥ 3:1
  (`ensure_contrast(surface, [gövde], 3)` güvencesi; WCAG 1.4.11). Koyu kipte `surface` göz
  bebeğiyle aynıdır, görünüm değişmez. Testli: statik ve üretilmiş tüm paletlerde gaga/gövde ≥ 3:1.

Mono kip (`Mascot::mono(role)`): `-mono.svg` alfa maskesi, rengi rol (`ImageStyle::tint`).
Tema değişiminde yeniden raster yok. Sakin/ikincil yüzeyler için.

## Gizleme

`[appearance] mascot = false` iken `Mascot::measure()` `Size::ZERO` döner ve `draw` hiçbir şey
çizmez (önceki bölge hasarlanır). Tüketici ek iş yapmaz; yerleşimi `measure()` ile kuruyorsa
maskotun yeri kendiliğinden kapanır. Değişim `theme_changed` → `set_theme` ile canlıdır
(`Theme::appearance` görsel token sayılır). Ayarlar bu anahtarı yazar (Görünüm).

## Performans

Canlanma yok; boşta kare istenmez. Süreç genelinde (mood, biçim, fiziksel kenar, renkler)
anahtarlı LRU (24); bileşen son dokusunu tutar, aynı anahtarda yeniden raster/yükleme yoktur
(testli). Mono maske renkten bağımsız anahtarlanır (tema değişimi mono'yu yeniden rasterlemez).
Rasterleştirme önbellek kilidi dışında yapılır (bir maskot diğerlerini bekletmez). Palet aynı
kaldıkça renkler yeniden hesaplanmaz. Bozuk varlık ya da raster hatası: uyarı + çizim yok, panik
yok.

## Doğrulama

- Birim testleri (`nuxut-ui` `mascot::tests`): her mood'un iki biçimi 48/96/160'ta ayrıştırılıp
  rasterleşir; kaynaklarda renk yok, yalnız bilinen sınıflar; renk/karşıtlık (yukarıda);
  gizleme → `Size::ZERO` + çizim yok + geri açılma; önbellek isabeti (yerel önbellekle raster
  sayısı, bileşen yükleme sayısı, LRU sınırı); boyut sıkıştırma ve piksel hizası.
- Varlıklar `.team/mascot/genis/` ile bayt bayt aynıdır (`cmp`, 14 dosya; 2026-10-13).
- Galeri: `NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=mascot` — üst yarı etkin tema (başlıkta kip,
  karşıtlık ve gizleme durumu), alt yarı `Palette::light()`; 7 mood × 160/96/48 iki ton + 48 mono.
  İzole headless sway'de koyu, açık, açık+yüksek karşıtlık ve `mascot = false` ile incelendi.
