# Dosya ızgarası (`FileGrid`) ve çoklu seçim (`Selection`)

2026-10-16 · talep: shell `filechooser` (Nuxut dosya seçicisi) · uygulama.
Kod: `crates/nuxut-ui/src/{file_grid,selection}.rs`; galeri: `NUXUT_GALLERY_PAGE=11`.

## Veri ve olaylar
- `FileItem { label, icon }`: görünen ad + tür ikonu (`Icon::image` tema ikonu dokusu ya da
  `IconName`). Küçük resim ayrı: `set_thumbnail(i, Option<Texture>)`; doku GPU'ya yüklenene dek
  ikon görünür, yüklendiği kare hasarlı sayılır. `set_icon(i, ..)` sonradan gelen tema ikonu için.
- `FileGridEvent::SelectionChanged` (küme `selection()`, imleç `cursor()`) ve `Activated(i)` (çift
  tık / Enter; çoklu seçimde uygulama `selection()`'a bakar).
- `visible_range(now)`: görünen öğeler; uygulama kareden/kaydırmadan sonra eksik küçük resimleri
  bu aralık için ister (önden yükleme payını kendisi ekler).

## Yerleşim
- Hücre en dar `space(28)`; sütun sayısı genişliğe sığan kadar, artan genişlik sütunlara eşit
  dağılır (sağda boşluk kalmaz). Kutu `space(16)` (küçük resim `Contain`, `radius_small`), tür
  ikonu `space(12)` (`IconView::with_size`), altında `Body` rolünde en çok 2 satır ortalı etiket
  (`…` ile kırpılır). Satır yüksekliği eşit → sanallaştırma indis aritmetiğiyle
  (`visible_range_uniform`); yalnız görünen hücreler şekillenir/çizilir, etiket+ikon LRU
  (`FILE_CACHE_CELLS = 256`, en az 2× görünen).
- Seçili hücre `tonal_container`; hover/basılı state layer; imleçte klavye odağında halka; yüksek
  karşıtlıkta seçili hücreler hep `HAIRLINE` çerçeveli (seçim yalnız tonla anlatılmaz). Renkler
  `GlyphGrid` ile ortak `CellColors`.

## Seçim kuralları (karar: GTK/Nautilus, Gezgin, Finder ortak davranışı)
- Tık: yalnız o; Ctrl+tık ekle/çıkar (çapa oraya); Shift+tık çapadan aralık (öncekinin yerine);
  Ctrl+Shift+tık aralığı ekler. Basışta seçilir. Boş alana değiştiricisiz tık seçimi boşaltır.
- Oklar imleci taşır ve yalnız onu seçer; Shift+ok aralık; Ctrl+ok yalnız imleç (Ctrl+Space
  imleçteki öğeyi ekler/çıkarır); Ctrl+A hepsi. PgUp/PgDn görünen satır kadar; Home/End
  ilk/son öğe; aşağı ok kısa son satırda son öğeye iner.
- Çift tık penceresi 400 ms ve `space(1)` mesafe (metin girişiyle ortak `DOUBLE_CLICK`).
- İşaretçi olayında değiştirici yok: uygulama `App::modifiers_changed`'den gelen değeri
  `pointer(theme, ev, modifiers, now)`'a verir.
- `Selection` ayrı tip (bit kümesi, sayım O(1)): `List::with_selection` (liste görünümü) de aynı
  kuralları kullanır; olaylar `pointer_select`/`key_select` → `ListSelectEvent` (ayrı tür:
  `ListEvent`'e varyant eklemek shell/settings'in kapsamlı eşlemelerini kırardı).

## GlyphGrid ile ortaklık (karar)
Ortak: `CellColors`, `ScrollView`, LRU yaklaşımı. Ayrı: GlyphGrid bölüm başlıklı değişken satır
tablosu ve yapışkan sütunla gezinir, tek seçimlidir; FileGrid eşit satırlı ve çoklu seçimlidir.
Ortak bir satır tablosu soyutlaması iki tarafı da karmaşıklaştırırdı.

## Erişilebilirlik
`access(tree, key, focused, now, item_key)`: `ListBox` (çoklu kipte `multiselectable`),
görünen hücreler (+ imleç) `ListBoxOption` (ad, `selected`, konum); `access_action`: `Click` seçer,
`Focus`/`ScrollIntoView` imleci taşır. `item_key` kalıcı kimlik (dosya adı karması) olmalı.

## Açık kalanlar
- Lastik bant (sürükleyerek) seçim, sürükle-bırak, yazarak arama (type-ahead) yok.
- Yakınlaştırma (hücre boyu) yok; RTL düzen yok.
