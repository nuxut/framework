# Nuxut Tasarım Sistemi

Tüm Nuxut uygulamaları aynı görsel dili konuşur. Değerlerin tek kaynağı
`~/.config/nuxut/framework.toml` dosyasıdır; dosya yoksa aşağıdaki varsayılanlar geçerlidir.
Bileşenler **asla sabit renk/yarıçap/süre içermez**, her zaman `Theme` token'ı okur.

## His

Sakin, derin, yumuşak. Shell içeriğin önüne geçmez. Hareket organik: şekiller birbirinden
**büyüyerek** çıkar ve yerine yumuşakça oturur. Keskin, mekanik, doğrusal hareket yok.
Varsayılan **sakin**dir (`motion.preset = "calm"`): taşma algı eşiğinin altında (≈ %0.5),
esneme belli belirsiz. Belirgin taşmalı, esneyen "puding" hissi isteğe bağlıdır
(`preset = "lively"`).

## framework.toml şeması ve varsayılanlar

```toml
[colors]
source             = "static"    # "static" (Nuxut renk teması) | "wallpaper" (duvar kağıdından üretilmiş palet)
theme              = "blue"      # Nuxut renk teması: "blue" (Nuxut mavisi) | "neutral" (Nötr) | "sage" (Adaçayı) |
                                 # "rose" (Gül kurusu) | "amber" (Kehribar) | "graphite" (Grafit).
                                 # source = "static" iken temel paleti verir; yerleşik duvar kağıtları
                                 # ona yeniden boyanır (docs/design/color-themes.md). Geçersiz: uyarı + blue.
mode               = "auto"      # "auto" | "dark" | "light": statik varsayılanın ve üretilmiş paletin kipi
                                 # (açık statik palet: koyu primary'den tonal-spot açık şema).
                                 # auto: duvar kağıdı paletinin kayıtlı kipi (yalnızca source =
                                 # "wallpaper") → yüzey parlaklığı (açıkça yazılmış surface dahil)
                                 # → koyu. Geçersiz değer: uyarı + auto.
contrast           = "standard"  # "standard" | "high": yüksek karşıtlık kipi (docs/design/accessibility.md §1):
                                 # metin ≥ 4.5:1 (hedef 7:1), metin dışı ≥ 3:1; tonlar uca itilir,
                                 # hue/kroma korunur; kenarlar en az "subtle". Geçersiz: uyarı + standard.
# Aşağıdaki renkler yazılırsa her kaynağın üstüne yazar; yazılmayanlar varsayılandan
# (source = "wallpaper" ise üretilmiş paletten) gelir.
surface            = "#12141a"   # çerçeve, bar, panel zemini
surface_container  = "#1a1d25"   # panel içindeki kartlar
surface_bright     = "#252935"   # hover/basılı katman tabanı
on_surface         = "#e4e6ee"   # ana metin
on_surface_variant = "#9298a6"   # ikincil metin
primary            = "#9ec5ff"   # vurgu (aktif workspace, seçim)
on_primary         = "#0a1a33"
error              = "#ffb4ab"   # hata (başarısız eylem, doğrulama)
on_error           = "#690005"
outline            = "#2c303b"
shadow             = "#000000"

[shape]
radius_small  = 8.0
radius_medium = 14.0
radius_large  = 22.0
outlines      = "none"   # "none" | "subtle" | "full" — kenar çizgisi kipi (bkz. "Kenarlar ve ayrım")

[shadow]
enabled  = true
blur     = 16.0      # mantıksal piksel
offset_y = 2.0
opacity  = 0.42

[motion]
preset  = "calm"     # "calm" | "lively" | "snappy" | "instant" — aşağıdaki VARSAYILANLARI belirler
enabled = true       # false: her şey anında (ön ayardan bağımsız ana anahtar)
speed   = 1.0        # global çarpan (2.0 = iki kat hızlı; yay response'u ve eğri süresi bölünür)
# style = "curve"    # rollerin stili; yazılmazsa ön ayarınki ("spring")

# Rol tabloları yalnızca ön ayarı EZMEK için yazılır; yazılan her anahtar yalnızca kendisini
# ezer, yazılmayanlar ön ayardan gelir. Örnek (calm değerleri):
# [motion.spatial]     # konum/boyut
# style       = "curve"          # rol düzeyinde stil
# response    = 0.38             # spring: periyot (sn)
# damping     = 0.87             # spring: sönüm oranı (<1 taşar)
# easing      = "emphasized"     # curve: eğri
# duration_ms = 500              # curve: süre
# [motion.spatial_fast]          # 0.26 / 0.92 · "emphasized-decelerate" / 350
# [motion.effects]               # opaklık/renk — taşmaz: 0.22 / 1.0 · "ease" / 200

[blob]
smoothing = 20.0     # R: şekiller birleşirken içbükey fileto ve çerçeve iç köşesi (ADR-0008)
# deform  = 0.2      # hızla gelen esneme/basılma şiddeti (0 = kapalı); yazılmazsa ön ayarınki

[desktop]            # diğer uygulamaların görünümü (shell ADR-0004; dış hedefleri yalnızca shell yazar)
unified      = true  # birleşik görünüm (varsayılan açık); false iken hiçbir dış hedef yazılmaz
icon_theme   = ""    # ikon teması dizin adı; "" = önerilen (≤ 128 bayt, harf/rakam ve " -_.+")
cursor_theme = ""    # imleç teması dizin adı; "" = önerilen
cursor_size  = 24    # 16..=96
qt_backend   = "auto" # "auto" | "kde" | "qt6ct"

[locale]             # ADR-0010; Ayarlar "Dil ve biçimler" yazar. Görsel token değil: değişince
                     # yalnızca App::locale_changed (yeni katalog), theme_changed gelmez.
language = "auto"    # "auto" (LANGUAGE > LC_ALL > LC_MESSAGES > LANG; C/POSIX → en) | "tr" | "en" | "pt-BR"…
formats  = "auto"    # tarih/saat + sayı biçimi: "auto" (LC_TIME/LC_NUMERIC…) | "tr_TR" | "en_US.UTF-8"…
                     # Dilden bağımsız (İngilizce arayüz + Türkçe tarih geçerli). Geçersiz değer: uyarı + auto.
hour_cycle = "auto"  # saat:dakika: "auto" (formats locale'inin verisi: tr → 24 s, en_US → 12 s) | "h23" | "h12".
                     # format_hm / time_locale() buna uyar; uygulamanın açık biçimi (shell clock.format)
                     # her zaman kazanır. Geçersiz değer: uyarı + auto (yalnızca bu anahtar). Değişince
                     # App::locale_changed. Karar: 2026-10-13 (Ayarlar t119).

# [appearance]       # varsayılanlar dosyaya yazılmaz; yalnızca kapatılınca görünür
# mascot = true      # maskot Fiyu (boş durum, hata, karşılama…); false: hiçbir yerde çizilmez, yer kaplamaz.
                     # Canlı değişir (theme_changed). Karar: kullanıcı, 2026-10-13 (docs/design/mascot.md).
```

### Hareket ön ayarları

Tek kaynak `nuxut_theme::MotionPreset::values()`. Yay değerleri `response / damping`.

| Ön ayar   | spatial     | spatial_fast | effects    | blob.deform | Not |
|-----------|-------------|--------------|------------|-------------|-----|
| `calm` (varsayılan) | 0.38 / 0.87 | 0.26 / 0.92 | 0.22 / 1.0 | 0.2 | taşma ≈ %0.5, sakin |
| `lively`  | 0.42 / 0.70 | 0.28 / 0.82  | 0.22 / 1.0 | 0.6 | puding: belirgin taşma ve esneme |
| `snappy`  | 0.30 / 1.0  | 0.22 / 1.0   | 0.22 / 1.0 | 0.0 | kritik sönümlü, kısa |
| `instant` | —           | —            | —          | 0.0 | `enabled = false` ile eşdeğer |

Eğri stili (`style = "curve"`) seçilirse süre/eğri her ön ayarda aynıdır (500 / 350 / 200 ms).
Ezme kuralı anahtar başınadır: `[motion.spatial] damping` yazmak yalnızca o değeri ezer.
"Özelleştirilmiş" = rol tablolarında ya da `motion.style`'da en az bir anahtar var
(`MotionConfig::is_customized`); `enabled`/`speed` ön ayar değildir. Ön ayara dönmek bu
anahtarları ve `blob.deform`'u siler. Bilinmeyen ön ayar adı uyarıyla `calm` olur.

```toml
[typography]           # ADR-0006: rol başına aile + ağırlık + ölçek
size     = 13.0        # taban (mantıksal px); tüm tip ölçeği buradan
icon_set = "material"  # "material" (Material Symbols, FILL animasyonu) | "nerd" (Nerd Font nf-md-*)

[typography.ui]        # arayüz metni
family = "Inter"
weight = 500
scale  = 1.0

[typography.display]   # büyük metin: saat, başlık (Inter opsz → Display kesimi)
family = "Inter"
weight = 600
scale  = 1.0

[typography.document]  # uzun okuma: takvim/etkinlik açıklaması (bildirim gövdesi DEĞİL — o ui ailesinde)
family = "Source Serif 4"
weight = 400
scale  = 1.0

[typography.mono]      # kod, sayısal tablo
family = "JetBrainsMono Nerd Font"
weight = 400
scale  = 1.0

[typography.icon]      # family boş = ikon setinin fontu (material → Material Symbols Rounded, nerd → mono)
family = ""
weight = 400
scale  = 1.0
```

Tipografi kuralları:
- Kurulu olmayan aile sessizce genel aileye düşer (`ui`/`display` → sans-serif, `document` →
  serif, `mono` → monospace). İkon fontu yoksa `material` → kurulu Nerd Font → hiç çizme.
- Eski anahtarlar uyarıyla eşlenir: `family`/`weight` → `ui`, düz dize `mono`/`icon` → rolün ailesi.
- Bildirim gövdesi `ui` ailesinde (`TextRole::Body`) çizilir; `document` (serif) yalnız uzun okuma
  içindir (takvim/etkinlik açıklaması). Bildirim kısa ve arayüzün sesidir; serif gövde kartta
  yabancı duruyordu. Karar: 2026-10-14.
- Değişen sayılar (saat, yüzde, sayaç) tabular rakamla çizilir (`tnum`), genişlik titremez.
- İkonlar semantik adla istenir (`wifi`, `volume_high`, `battery_50`…); glif ikon setinden gelir.
  `nerd` setinde dolgu, çizgi/dolu glif çifti arasında opaklık geçişidir.

### Renk katmanları

`renk teması ([colors] theme; varsayılan Nuxut mavisi) ← palette.toml (yalnızca source = "wallpaper") ← framework.toml'daki açık renkler`.
Üretilmiş palet `$XDG_STATE_HOME/nuxut/palette.toml` (genelde `~/.local/state/nuxut/`)
dosyasındadır; palet üreticisi yalnızca bu dosyayı (atomik) yazar, **framework.toml asla
otomatik yazılmaz**. Biçimi:

```toml
[meta]                       # bilgi; tema yalnızca mode'u okur ([colors] mode = "auto" iken)
source   = "#4a6fa5"         # tohum renk
scheme   = "tonal-spot"
mode     = "dark"            # üretimdeki kip; auto'nun ilk kaynağı
contrast = 0.0
image    = "/home/…/wallpaper.png"

[colors]                     # framework.toml [colors] ile aynı 11 rol, meta.mode kipinde
primary = "#a6c8ff"

[dark]                       # aynı 11 rol koyu kipte; [light] açık kipte. Tema çözülmüş
primary = "#a6c8ff"          # kipin bölümünü okur (eski dosyada yoksa [colors] + uyarı)

[material]                   # seçili kipin tüm Material 3 rolleri (uygulamalar için, ≤ 128)
primary_container = "#284777"
```

Hata davranışı (ARCHITECTURE değişmez 7): palet dosyası **bozuksa son geçerli palet
korunur** (uyarı loglanır); dosya **silinirse** palet katmanı kalkar ve renkler varsayılana
(+ framework.toml'daki açık renklere) döner. framework.toml bozuksa son geçerli tema korunur.

Kurallar:
- Geçersiz değer (negatif yarıçap, `damping <= 0`, NaN…) yüklemede sınırlanır ve uyarı verilir.
- `motion.enabled = false` → tüm hareketler anında (`Motion::INSTANT`).
- **Hareket stili.** `spring` kesilebilir ve hızı korur (varsayılan, önerilen). `curve` sabit
  süreli eğridir; kesilince o anki değerden yeni hedefe yeni bir geçiş başlar ve başlangıç hızı
  korunur (sıçrama yok). `easing` değerleri: `linear`, `ease`, `ease-in`, `ease-out`,
  `ease-in-out`, `emphasized`, `emphasized-decelerate`, `emphasized-accelerate` ya da
  `cubic-bezier(x1, y1, x2, y2)` (x ∈ [0, 1], y ∈ [-1, 2]). Geçersiz eğri uyarıyla rolün
  varsayılanına döner; `duration_ms` [0, 5000]. Blob esnemesi (deform) her zaman yaydır.
- Tema değişimi animasyonludur: renkler `motion.effects` yayıyla yeni değerlere akar.

## Zamanlama

Hareket olmayan bekleme süreleri (`theme.timing`; şimdilik framework.toml'da değil, tek kaynak
`nuxut-theme` `Timing::DEFAULT`). `motion.enabled`/`speed` bunları etkilemez.

| Token | Değer | Anlam |
|---|---|---|
| `hover_delay` | 500 ms | Fare üzerine gelince ipucunun belirmesine kadar bekleme |
| `hover_warm` | 800 ms | İpucu kapandıktan sonra yeni öğede beklemeden gösterme penceresi |
| `confirm_window` | 4 s | Yerinde onayda (`ConfirmButton`) ikinci basış için bekleme; dolunca vazgeçilir |
| `confirm_guard` | 400 ms | Onay kurulduktan hemen sonra ikinci basışın yok sayıldığı süre (yanlışlıkla çift tık) |
| `flash_hold` | 900 ms | "Satırı parlat" vurgusunun (arama sonucu) tam opaklıkta kaldığı süre; sonra `motion.effects` ile söner. Hareket kapalıyken de görünür |

## Bileşen kuralları

- **State layer:** hover = `on_surface` %8, basılı = %12, `surface` üzerine karıştırılır;
  `motion.effects` ile geçer.
- **Odak:** klavye odağı `primary` 2px halka; fare tıklamasında görünmez.
- **Satırı parlat:** arama sonucuna gidilen satır `primary` %16 (`FLASH_TONE`) zeminle `flash_hold`
  boyunca vurgulanır, sonra `motion.effects` ile söner (`Disclosure::flash`/`flash_content`).
- **Satır yüksekliği:** standart satır (ayar satırı, açılır satır başlığı) en az `style::row_height` =
  `space(16)` (64 px); bir sayfadaki satırlar bu ritmi paylaşır. Gezinme listesi kendi yoğunluğunu kullanır.
- **Açılır satır (Disclosure):** satır kalıbı (iç boşluk yatay `space(4)`, dikey `space(3)`, en az
  `row_height`); özet `Caption` `on_surface_variant`, sığmazsa başlığın altına; rozet `space(2)` `primary`
  nokta; chevron `IconSmall` boyunda, açılma yayıyla 180° döner; içerik `spatial_fast`.
- **Yarıçap ölçeği:** küçük öğe (chip, workspace) `radius_small`, kart `radius_medium`,
  panel `radius_large`. İç içe yarıçap = dış yarıçap − iç boşluk.
- **Boşluk ızgarası:** 4'ün katları.
- **Metin:** ana `on_surface`, ikincil `on_surface_variant`; ikincil metin hiçbir zaman
  renkli zemin üzerinde gri olmaz.
- **İpucu:** tonal (`surface_bright` dolgu, `on_surface` metin, çizgi yok; gölge temadan),
  `radius_small`, Caption metni. `hover_delay` sonra belirir; görünürken ya da `hover_warm`
  içinde öğeden öğeye beklemesiz geçer. Kenardaki bar'dan ekranın içine açılır, sığmazsa
  karşı tarafa döner. En çok ~40 karakter (`space(60)`) genişlikte sarılır, 4 satırdan sonra
  "…". Hareket sakin: bağlantı kenarından 0.92 → 1 büyüme + solma, kesilebilir.
- **Yasaklar:** zıplayan (bounce/elastic) bezier eğrileri, doğrusal mekânsal hareket, saf
  `#000`/`#fff` yüzeyler, iç içe gölgeli kartlar, mor gradyanlar.

## Marka işareti

Nuxut'un işareti **Kara Sağan**: yatışta, sağ üste tırmanan bir kara sağan (slogan: "The penguin
can't fly. This one can."). Geometri `nuxut-ui/assets/brand/` altındaki SVG'lerdedir ve elle
değiştirilmez (yalnızca daire yayları ve doğrular; gerekçe ve testler ekibin logo turunda).
Uygulama içinde işaret her zaman `BrandMark` ile ve **tema rolleriyle** çizilir: tek renk
(`on_surface`; vurgu `primary`), iki ton (ön kanat `primary`, gövde `primary`'den türetilen koyu
ton) ya da karo (uygulama ikonu görünümü). Sabit renkli tek kopya dışa aktarılan uygulama
ikonudur (`brand_icons` örneği → hicolor). 16 px'te çatal kuyruk incelir; işaret o boyda siluet
olarak okunur — kuyruğun seçilmesi gereken yerde en az 24 px kullanın.

## Maskot

Nuxut'un maskotu **Fiyu**: "Geniş" oranlı, orak kanatlı küçük bir kuş; yedi ruh hâliyle (`Mascot`,
`Mood`). Yalnızca boş durum, sonuç yok, hata, Hoş geldin ilk/son adımı, Hakkında ve "güncelleme
bitti" yüzeylerinde, 48–160 px; **asla** bar, kilit ekranı, parola diyalogları ya da uygulama ikonu
olarak. Renkler tema rollerinden türetilir (ön kanat ve gövde zemine ≥ 3:1). Kullanıcı
`[appearance] mascot = false` ile her yerde gizler. Ayrıntı: `docs/design/mascot.md`.

## Kenarlar ve ayrım

Varsayılan görsel dil **tonaldir**: öğeler birbirinden 1px kenar çizgisiyle değil, **yüzey
tonu, boşluk ve hafif yükseltiyle** ayrılır (`[shape] outlines = "none"`). Kenar çizgileri
görsel gürültüdür ve yoğun arayüzlerde ızgara hissi verir. Tonal ayrım daha sakin ve
Nuxut'un "yumuşak, derin" hissine uygun.

- Bileşen kenarı **yalnızca** `theme.outline()` ile çizilir: `none` → hiç; `subtle` → `outline`
  rengi (ince, koyu ton); `full` → `on_surface_variant` %70 — yüzeylere karşı ≥ 3:1 (yüksek
  kontrast / erişilebilirlik tercihi; testle korunur). Sabit kenar yok.
- `outlines = "none"` dönüşümleri:
  - Outlined düğme → tonal (`surface_bright` dolgu).
  - Metin kutusu / açılır seçim → dolgulu alan (`surface_bright`: kaplar `surface_container` olduğundan
    alan onlardan ayrışsın; hover'da state layer ile bir ton açık); odakta kenar
    yerine 2px `primary` alt vurgu + imleç.
  - Toggle kapalı → dolgulu ray (`surface_bright`) + `on_surface_variant` topuz, çerçeve yok.
  - Liste öğesi / kart → yalnızca ton ve boşluk.
  - Takvim: bugün → dolgulu daire, seçili gün → `primary` dolgu.
  - Renk seçici seçili örnek → arada `surface` boşluğu olan halka ("gap ring") + hafif büyüme.
  - Ayırıcı → önce boşlukla çöz; gerekiyorsa `on_surface` %6–8 opaklıkta.
- **Erişilebilirlik (WCAG 1.4.11, bileşen sınırı ≥ 3:1):** tonal dolgunun zeminle kontrastı
  tek başına yetmiyorsa sınırı etiket/placeholder/ikon taşır; metin kutusu ve toggle gibi
  "nereye tıklanır" belirsizleşen yerlerde dolgu tonu zeminden yeterince ayrılır.
- **Yüksek karşıtlık kipi** (`[colors] contrast = "high"`, docs/design/accessibility.md §1) bu
  kuralın ötesine geçer, WCAG AA'yı palet düzeyinde garanti eder: metin ve `on_*` çiftleri
  ≥ 4.5:1 (hedef 7:1), `primary`/`error` yüzeylere ≥ 4.5:1, `outline` ≥ 3:1; `outlines = "none"`
  `subtle`'a yükselir, `full` tam opaktır. Yüzeyler ve hue/kroma korunur — renkli kimlik kalır.
  Bileşenin kendi karıştırdığı renk (alfa, iz/ray tonu) bu garantinin dışındadır; gereken
  yerde `theme.contrast` okunur.
- **Klavye odak halkası** (`primary` 2px) her kipte kalır; fareyle tıklamada görünmez.

## İmleç

İmleç her zaman işaret ettiği şeyin ne yapacağını söyler. Karar: **tıklanabilir her öğe el
(`Pointer`) gösterir.** GNOME/KDE masaüstü düğmelerde oku korur; ama Nuxut'un yüzeyleri
(bar, panel, bildirim, ayarlar) web/dokunmatik diline daha yakın, state layer hover'ı zaten
gösteriyor ve kullanıcı açıkça el istedi. Kural shell'de ve tüm uygulamalarda aynıdır.

| Durum | Şekil |
|---|---|
| Metin girişi, seçilebilir metin | `Text` |
| Düğme, ikon düğmesi, bağlantı, tıklanabilir liste öğesi, toggle, menü öğesi, sekme/bölümlü düğme, açılır seçim, takvim günü, renk örneği, workspace hapı, tepsi ikonu, bildirim kartı gövdesi | `Pointer` |
| Slider rayı | `Pointer`; tutamaç üstünde `Grab`, sürüklerken `Grabbing` |
| Sürüklenebilir tutamaç (ReorderList, eğri düzenleyici noktası) | `Grab` / sürüklerken `Grabbing` |
| Kaydırarak kapatma (bildirim) sürüklenirken | `Grabbing` |
| Renk seçici SV alanı | `Crosshair`; ton şeridi `Pointer` |
| Devre dışı öğe; menü/seçim listesinde bölüm başlığı ve ayraç | `Default` (ok); `NotAllowed` yalnızca reddedilen açık bir eylemde (ör. bırakılamaz hedef) |
| Pencere kenarı / köşesi | `Resize{N,S,E,W,NE,NW,SE,SW}` |
| Başlık çubuğu, boş zemin, çerçeve, panel arka planı, kaydırma çubuğu | `Default` |
| Arka planda iş sürüyor, arayüz kullanılabilir | `Progress` (`Wait` kullanılmaz) |

Uygulama: her bileşen `fn cursor(&self, p: Point) -> Option<CursorShape>` verir (`None` = görüş
yok); kök, en üstteki isabetten başlayarak `CursorShape::pick([...])` ile seçer ve
`cx.set_cursor(surface, shape)` der. Runtime aynı şekli tekrar göndermez, işaretçi yüzeye
girince son şekli yeniden uygular.
