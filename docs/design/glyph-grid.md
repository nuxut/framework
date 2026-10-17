# Glif ızgarası (`GlyphGrid`)

2026-10-13 · talep: kabuk (başlatıcının emoji seçici kipi) · uygulama.
Kod: `crates/nuxut-ui/src/glyph_grid.rs`; galeri: `NUXUT_GALLERY_PAGE=10`.

## Amaç
Emoji, sembol ve karakter seçimi için genel ızgara. Başlatıcıya özgü değil (karakter haritası,
metin alanı emoji paneli, ikon seçici de kullanır); bu yüzden `nuxut-ui`'de. Veri (CLDR adları,
kategoriler, arama, son kullanılanlar) çağıranın işidir; bileşen yalnız gösterir ve gezdirir.

## Veri modeli ve olaylar
- `GlyphItem { glyph, label }`: `glyph` çizilen metin (tek kod noktası ya da ZWJ/ten rengi dizisi),
  `label` erişilebilir ad (çizilmez; `label(i)` ile okunur, ileride a11y ağacına verilecek).
- `GlyphSection { title, items }`: isteğe bağlı kategori. Boş başlık = başlık satırı yok; boş bölüm
  yer kaplamaz. İndeksler bölümlerden bağımsız düzdür (`0..len`), `section_of(i)` / `section_start(s)`.
- `GlyphGridEvent::Selected(i)` (klavye gezinmesi; önizleme için) ve `Activated(i)` (Enter, tıklama).
  ResultList ile aynı sözleşme: tıklama seçimi olaysız taşır ve `Activated` verir; hover seçimi değiştirmez.

## Yerleşim
- Kare hücre: `cell_size = space(11)` (44 px; dokunma hedefi). `GlyphColumns::Auto` genişliğe sığan
  sütun (en az 1), artan genişlik iki yana eşit bölünür (ızgara ortalı). `GlyphColumns::Fixed(n)`:
  hücre kenarı `floor(genişlik / n)`.
- Her bölüm yeni satırda başlar; başlık satırı `space(8)`, `Label` rolü, `on_surface_variant`.
- Glif: arayüz fontu (`Body` ailesi, emoji/sembol fontlarına fontconfig ile düşer) `Display`
  boyunda, tek satır, hücrede ortalı. Renkli emoji olağan metin yolunun RGBA atlasıyla çizilir;
  tek renk sembol `on_surface`.

## Gezinme (karar: WAI-ARIA grid kalıbı)
- ←/→ doğrusal: satır ve bölüm sınırını geçer; uçlarda yerinde (olay yok). Görsel yöndedir:
  sağdan solda (`set_rtl(true)`) ← sonraki, → önceki öğe.
- ↑/↓ aynı sütun: kısa (son ya da bölüm sonu) satırda son hücreye kırpılır; sütun "yapışkan"
  hatırlanır (kısa satırdan geri çıkınca eski sütuna döner). Yatay hareket/tıklama hatırlananı siler.
- PageUp/PageDown: sayfa sayfa — görünüm yüksekliği kadar (piksel; başlık satırları dahil, en az
  bir satır) aşağı/yukarıdaki hücre satırı (sütun yapışkan); görünüm de aynı mesafe kayar: seçim
  ekrandaki yerinde kalır, içerik bir sayfa akar. İlk sürüm `floor(yükseklik / hücre)` satır
  sayıyordu ve görünümü kaydırmıyordu; başlıklı bölümlerde sayfa görünümü aşıyordu (düzeltme
  2026-10-16).
- Home/End satır başı/sonu; Ctrl+Home/End ilk/son öğe. Enter `Activated`. Space ve harfler
  tüketilmez (çağıranın arama alanına gider).
- Seçim değişince satırı görünür kılınır; bölümün ilk satırıysa başlığıyla birlikte.
  `reveal_section(s)` (kategori sekmesi) başlığı tepeye kaydırır ve seçimi bölümün ilk öğesine taşır.
- `set_selected(i)` yerleşimden önce çağrılırsa ilk `set_rect`'te animasyonsuz görünür kılınır.

## Görünüm ve erişilebilirlik
- Seçili hücre `tonal_container` kabı; hover/basılı `state_layer_under(on_surface)`; klavye odağında
  `FOCUS_RING` kalınlığında `focus_ring` halkası (çağıran `FocusScope::ring_visible_for` ile verir).
- Yüksek karşıtlık: seçim yalnız tonla anlatılmaz — seçili hücrede odak yokken de `HAIRLINE`
  kalınlığında halka rengi kenar. `contrast_tests::glyph_grid_cells`: glif ≥ 4.5 (seçili/seçisiz ×
  dinlenme/hover/basılı), halka kap ve yüzeylere ≥ 3, başlık ≥ 4.5.
- Hareket: seçim vurgusu `spatial_fast` yayla kayar; azaltılmış harekette anında
  (`reduced_motion_tests::glyph_grid_selection_and_reveal_are_instant`). Oturunca kare istemez.

## Performans
- Sanallaştırma: satır tablosu (y başlangıçları) + `ScrollView::visible_range` ikili araması; yalnız
  görünen satırların glifleri şekillendirilir ve çizilir (test: 4000 öğede kare başına ≤ 60 hücre).
- Glif layout önbelleği: öğe indisine göre, sığa `max(GLYPH_CACHE_CELLS = 512, 2 × görünen)`;
  taşınca en eski yarısı tek seçimle atılır (ekleme başına tarama yok). Tema değişince boşalır;
  genişlik değişince yalnız başlıklar yeniden şekillenir.
- Sınırlar: `MAX_GLYPH_ITEMS = 65 536`, glif `MAX_GLYPH_TEXT = 64` bayt, ad/başlık `MAX_GLYPH_LABEL = 256`.
  `Debug` yalnız uzunluk/sayı yazar (son kullanılanlar kullanıcı verisi sayılır).

## Açık kalanlar
- Fontta bulunmayan glif için tofu ayıklama (`TextSystem::glyphs_in_family`) çağıranın işi.
- a11y ağacı (AT-SPI) henüz yok; adlar tutuluyor.
- Ten rengi/varyant alt menüsü (uzun basış) çağıranın `Popover`'ıyla yapılır; bileşende yok.

## Sağdan sola (2026-10-16; görev)
- `set_rtl(bool)` / `is_rtl()`: çağıran `nuxut_i18n::current().direction() == Direction::Rtl`
  verir (dil değişince yeniden). `nuxut-ui` i18n'e bağımlı değil; yön dışarıdan gelir.
- Sütun 0 sağdadır; kısa satır sağa yaslanır; başlıklar sağa yaslı. İşaretçi aynalı sütunu bulur.
- ←/→ görsel yöndedir (← sonraki); ↑↓, Home/End (satırın mantıksal başı/sonu), Ctrl+Home/End
  ve indisler değişmez. Yön değişince seçim vurgusu yeni yerine atlar (canlanmaz).
