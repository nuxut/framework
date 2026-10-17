# Yol çubuğu (`PathBar`)

2026-10-15 · talep: shell `filechooser` (Nuxut dosya seçicisi, portal FileChooser) · uygulama.
Kod: `crates/nuxut-ui/src/path_bar.rs`; galeri: `NUXUT_GALLERY_PAGE=11` (`STATE=edit`).

## Veri modeli ve olaylar
- `PathSegment { label, icon }`: görünen ad (ör. "Ev", sürücü etiketi) ve isteğe bağlı ikon
  (`IconName::Home`, `Usb`). Gerçek dizin yolu ve ad eşlemesi çağıranındır; bileşen yalnız indis döner.
- `PathBarEvent::Navigate(i)`: tıklanan / Enter-Space'lenen parça. `…` düğmesi gizlenen en yakın
  üst klasörü (`first_visible() - 1`) verir — menü açmak yerine tek adım: dar pencerede yukarı
  çıkmak en sık iş, geri kalan hedefler zaten Yerler kenar çubuğunda.
- `PathBarEvent::Edit(TextInputEvent)`: düzenleme kipindeki girişin olayları.

## Katlama
- Son parça (geçerli klasör) her zaman görünür, hafif tonal hap (`surface_bright`) + `on_surface`;
  diğerleri saydam, `on_surface_variant`, hover/basılıda state layer.
- Sığmayınca baştan katlanır: sondan geriye sığan en çok parça, önünde `…` + ayraç. Son parça tek
  başına sığmazsa etiketi `…` ile kırpılır.
- Ayraç `ChevronRight` (`IconSmall`), iki yanında `space(1)`. Yükseklik `space(10)`.
- Ölçüm metne bağlı olduğundan katlama `layout(text, theme)`'te (draw da çağırır), yalnız
  parçalar/dikdörtgen/tema değişince. `MAX_PATH_SEGMENTS = 256` (daha derin yolda baştakiler
  atılır, indisler yine verilen listenindir), `MAX_PATH_SEGMENT = 255` bayt.

## Düzenleme kipi (karar)
Ctrl+L kısayolunu bileşen değil uygulama yakalar (`begin_edit(theme, tam_yol, now)`): kısayol
eşlemesi pencerenin işi ve odak çubukta olmasa da çalışmalı. Esc kipten bileşenin kendisi çıkar
(`Edit(Cancel)` yine iletilir); Enter'da (`Edit(Submit)`) uygulama `edit_text()`'i doğrular, gidebildiyse
`end_edit()`. Pano/IME tesisatı `text_input_mut()`. Girişin `Debug`'ı redakte.

## Klavye
Sol/Sağ odak (halka yalnız klavyeyle), Home/End uçlar, Enter/Space etkinleştir. Başlangıç odağı
geçerli klasör.

## Açık kalanlar
- a11y (AT-SPI): erişilebilirlik çalışmasının `access()` düzeni main'e inince eklenecek (rol: araç çubuğu,
  parçalar düğme; ad `segment_label`, konum `segment_rect`).
- RTL düzen yok.
