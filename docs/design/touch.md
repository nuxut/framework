# Dokunmatik ekran (wl_touch)

Durum: v1, 2026-10-16 (gerekçe: ekran klavyesi t125 ve tabletler).

## Amaç

Ekran klavyesi ve tabletler için parmakla kullanım. Mevcut bileşenlerin (Button, Switch, Slider,
List, Tile, Chip…) **ek kod gerektirmeden** çalışması; listelerde parmakla kaydırma ve atalet;
çoklu dokunma isteyen yüzeyin (ekran klavyesi) ham temaslara erişimi.

## Karar: birincil temas işaretçi olarak öykünülür

İki seçenek vardı: ayrı `TouchEvent` modeli (her bileşen dokunmayı ayrıca öğrenir) ya da tek
temasın işaretçi gibi davranması. İkincisi seçildi; ayrım yine de görünür:

- `PointerEvent` yeni alan taşır: `source: PointerSource { Mouse, Touch }`. `PointerEvent::new`
  `Mouse` verir (tüketicilerdeki yapı oluşturmaları değişmez), `PointerEvent::touch`
  `Touch` verir, `with_kind` türev olayda kaynağı korur.
- Runtime (`nuxut-wayland/src/touch.rs`, Wayland bilmeyen durum makinesi, birim testli):
  - Sahiplenilmeyen ilk temas **birincil**: `Down` → `Enter, Motion, Press(Left)`;
    `Motion` → `Motion`; `Up` → `Release(Left), Leave`; compositor `cancel` → yalnız `Leave`
    (bırakma yok, eylem üretilmez).
  - Birincil sürerken gelen diğer temaslar öykünülmez (bileşen modeli tek işaretçili). Birincil
    kalkınca, hâlâ basılı olan ikinci parmak dizinin ortasında birincil olmaz.
  - Birincil temas `TOUCH_TAP_DELAY` (100 ms) boyunca `TOUCH_SLOP` (8 px) içinde kalırsa runtime
    aynı konumda tek bir `Motion` üretir (bekleme tiki; aşağıda kaydırma).
  - Aynı anda en çok 16 temas izlenir (bozuk compositor'a karşı sınır).
  - Dokunma `down` serial'ı son girdi sayılır: `begin_move`/`begin_resize`, etkinleştirme
    belirteci ve pano dokunmayla da çalışır.
  - Kilit varken kilit dışı yüzeye dokunma girmez (işaretçi kuralıyla aynı).
- Ham olaylar: `App::touch(cx, surface, TouchEvent) -> bool` (varsayılan `false`) her temas için
  `Down/Motion/Up/Cancel` alır. `Down`'da `true` dönmek o temasın dizisini **sahiplenir**: öykünme
  yapılmaz. Ekran klavyesi böyle çoklu dokunur (iki başparmakla hızlı yazışta ikinci parmak
  kaybolmaz).
- `wl_touch.frame`: sctk olayları frame'e kadar biriktirir, sonra sırayla verir; runtime frame
  başına ayrıca birleştirme yapmaz (her temas frame başına en çok bir hareket bildirir).
  `shape`/`orientation` yok sayılır.

## Kaydırma

`ScrollView` dokunma kaynağını ayırır (`pointer` içinde):

1. Görünüm içindeki basma **çocuklardan bekletilir** (iOS `delaysContentTouches`, Android
   `TAP_TIMEOUT` gibi). Çocuklar dokunmada basmadan önce hover görmez.
2. Parmak eşiği **dikey** aşarsa (ve içerik kaydırılabilirse) kaydırma başlar: içerik parmağı
   doğrudan izler, eşik payı düşülür (sıçrama yok), çocuk basmayı hiç görmez — ne iptal ne
   yanlış tıklama gerekir. Parmak basılıyken sessizlik bırakma sayılmaz (`held`).
3. Eşik **yatay** aşılırsa (yatay kaydırıcı, kartı kaydırıp kapatma) ya da 100 ms dolarsa (bekleme
   tiki) bekletilen basma çocuğa `Motion, Press` olarak iletilir; dizinin kalanı doğrudan geçer.
4. Parmak bekleme dolmadan kalkarsa (hızlı dokunuş) `Motion, Press, Release` hemen iletilir:
   tıklamada gecikme yok.
5. Bırakma: son 80 ms'de hareket yoksa hız sıfır; değilse var olan üstel fırlatma.
6. Fırlatma sürerken dokunuş içeriği **yakalar**: durur, dokunuş eylem üretmez.
7. **Kenar:** dokunmatik yüzeydeki lastik direnç dokunmatik ekranda da var (taşma arttıkça azalan
   kazanç, `RUBBER_LIMIT` = görünüm yüksekliğinin %18'i), bırakınca sınıra yaylanır. Gerekçe:
   parmak içeriği doğrudan tutuyor; sınırda düz durmak "takıldı" gibi okunur. Tekerlekte sekme
   yok kararı (73a6cd9) değişmedi: tekerlek doğrudan işlem değil. Hareket kapalıyken fırlatma yok,
   taşma anında oturur (dokunmatik yüzeyle aynı).

Kapsayıcı sözleşmesi: `scroll.pointer(..)` sonra çocuklara `event` yerine
`for ev in scroll.forward(event) { .. }` (en çok üç olay; fare olayı aynen, çubuk sürüklenirken
hiçbiri). Framework'te `List`, `GlyphGrid`, `ResultList`, `WeekGrid` buna geçti. `forward`
kullanmayan kapsayıcı derlenir ve fareyle aynen çalışır; ama dokunmada çocuklar ham diziyi görür:
içerik kayarken altındaki öğe basılı kalır ve parmak aynı öğede kalkarsa tıklanır. Kaydırma
görünümü kullanan tüketiciler (shell bildirim paneli, settings sayfası) geçmeli.

## Hover, imleç, ipuçları

- Dokunmada hover yalnız parmak basılıyken vardır (öykünme `Enter`'ı basmayla, `Leave`'i
  kalkışla verir). Fare aynı yüzeydeyse kalkıştan sonra hover bir sonraki fare hareketiyle döner.
- İmleç şekli dokunmada değişmez: öykünme imleç izleyicisine girmez (`cursor.tracker`), bileşenlerin
  `cursor` istekleri yalnız gerçek işaretçinin yüzeyine uygulanır. Compositor imleci dokunmada
  zaten gizler.
- İpuçları: parmak basılı tutulursa (uzun basma) ipucu her zamanki gecikmeyle çıkar, kalkışta
  `Leave` ile kapanır. Ayrı uzun basma jesti v1'de yok.
- `Slider`: dokunmada bırakmasız `Leave` (compositor iptali) sürüklemeyi bitirir; farede Leave
  örtük tutuş yüzünden gelmediğinden davranış değişmedi.

## Ekran klavyesi

Tuş basılı kaldıkça basılı kalır (tekrar istemcide, `zwp_virtual_keyboard` tuşu basılı tutar);
parmağı kaydırınca tuş değişir. Shell `App::touch` ile OSK yüzeyindeki temasları sahiplenir ve
temas başına tuş izler (shell `docs/design/osk.md`).

## Bilinen sınırlar (v1, 2026-10-16; istekle yazıldı)

- **İç içe kaydırma:** dış `ScrollView` içindeki ikinci bir kaydırılan alan (settings yazı tipi
  seçicisi: sayfa içinde `FontPicker` listesi) dokunmada önce dışa aittir. Hızlı dikey sürükleme
  dış sayfayı kaydırır; iç liste ancak parmak `TOUCH_TAP_DELAY` basılı tutulup basma içe
  iletildikten sonra kaydırılır. Tık sızmaz. v2: iç kaydırılabilir alan basmayı kendisi sahiplenir
  (dış görünüm, iç alan o yönde kaydırılabiliyorsa beklemez).
- **Menüler ve açılır listeler:** kendi içinde tekerlekle kayan bileşenler (`Menu`, `Select`
  açılır listesi ve benzerleri) `ScrollView` kullanmaz; dokunmada parmakla kaydırılamaz
  (dokunuşla seçim çalışır, tık sızmaz). v2: bunları `ScrollView`'a taşımak.

## Test ve bilinen sınır

- Birim: `nuxut-wayland/src/touch.rs` (öykünme dizileri: dokunuş, iptal, ikinci parmak,
  sahiplenme, bekleme tiki, yok edilen yüzey, temas sınırı), `nuxut-ui/src/scroll.rs` (dokunuş,
  bekleme, yatay/dikey ayrımı, kaydırma ve fırlatma, duraklayıp kalkma, lastik, yakalama, iptal,
  kısa içerik), `list.rs` (dokunuş satırı etkinleştirir, sürükleme etkinleştirmez), `slider.rs`.
- **Canlı sınır (2026-10-16):** izole headless sway'de dokunmatik koltuk yok ve Wayland'de sanal
  dokunma protokolü yok (`virtual-pointer` yalnız işaretçi). `ydotool` uinput ile **gerçek**
  oturuma girdi verir (kullanıcının ekranı) — kullanılmaz. Bu yüzden wl_touch yolu canlı olarak
  sürülmedi; öykünmenin ürettiği işaretçi dizileri birim testlerde kayıtlı olay dizileriyle,
  sctk `TouchHandler` bağlantısı derlemeyle doğrulandı. Gerçek dokunmatik donanımda ilk deneme
  açık kalan iştir.
- **Sentetik dokunma (2026-10-17):** canlı testte dokunma yolu uçtan uca `test-touch` özelliğiyle
  sürülür (`nuxut-wayland/src/test_touch.rs`, yalnız debug derlemesi — sürümde derleme hatası).
  Uygulama `NUXUT_TEST_TOUCH=<fifo>` ile başlar; test betiği FIFO'ya satır yazar: `down ID NS X Y`
  (NS = layer namespace'i, koordinat yüzey içi), `motion ID X Y`, `up ID`, `cancel`. Olaylar
  `wl_touch` işleyicisiyle aynı yöntemlerden geçer (`State::touch_down/up/motion/cancel`:
  sahiplenme, öykünme, bekleme tiki, kilit kuralı); yalnız compositor tarafı (seri numarası,
  yüzey seçimi) sentetiktir. Kullanıcının oturumuna girdi gitmez.
