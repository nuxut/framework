# Nuxut renk temaları

Karar: 2026-10-16 — **kullanıcı isteği**; tasarım ve tema seçimi framework tarafında.
İstek: "Nuxut arka planlarını renklere göre ayarlanabilen birkaç renk paleti… normal mavimiz
bir renk teması olabilir, birkaç başka renk teması olabilir"; kullanıcı ArtStation'daki bir
görselin (artwork `1L4RQo`) nötr paletini çok beğendi, "birebir olmasına gerek yok".

Görsel kaynağa erişilemedi (ArtStation 403). "Nötr" tema kullanıcının tarifinden ve bugün
seçtiği Material You **Neutral** varyantının hissinden yola çıkar: sakin, sıcak-nötr, düşük
doygunluk, ama gri değil — karakterli bir taş/kum tonu. Kullanıcı görseli paylaşırsa yalnızca
`NEUTRAL` tanımının beş (hue, kroma) çifti ayarlanır; başka hiçbir şey değişmez.

## Kavram

**Nuxut renk teması**, adlandırılmış ve elle ayarlanmış bir **kaynak palettir**: beş tonal
palet (Material 3 Dynamic Color'ın yapı taşları) —

| Palet | Ne boyar |
|---|---|
| birincil (primary) | vurgu: etkin workspace, seçim, odak halkası, dolu düğme |
| ikincil (secondary) | tonlu düğme, filtre çipi, ikincil vurgu (dış uygulamalar) |
| üçüncül (tertiary) | karşıt vurgu (dış uygulamalar, terminal renkleri) |
| nötr (neutral) | yüzeyler: çerçeve, bar, panel, kart, ana metin |
| nötr-varyant (neutral variant) | ikincil metin, ayırıcı/kenar |

Her palet bir HCT ton çizgisidir: **hue + kroma** sabit, ton 0…100. Hangi rolün hangi tonu
aldığını tema değil, Material 3 **tonal-spot** ton kuralları belirler (koyu: yüzey T6, kap T12,
parlak kap T17, metin T90, vurgu T80…; açık: tersi). Sonuç:

- Tema yalnızca **renk ailesini ve doygunluğu** seçer; ton mimarisi her temada aynıdır. Metin/zemin
  karşıtlığı tonlarla gelir, temadan bağımsızdır (aşağıda test).
- Koyu ve açık kip aynı tanımdan çıkar; tema başına iki elle palet bakımı yoktur.
- Dış uygulamalar (GTK/Qt/KDE/Hyprland/terminal) aynı paletlerden 57 rolün tamamını alır
  (nuxut-palette `Palette::from_color_theme`).

Tanım (`nuxut-theme::color_theme`): `seed` (anahtar renk; göreli hue'ların kökü, `success`/
`warning` ona uyumlandırılır) + beş `Tonal { hue (göreli/mutlak), chroma }`. Yalnızca Nuxut
mavisi koyu kipte tarihî elle ayarlanmış 11 token'ı (`Palette::DARK`) bayt bayt korur.

## Temalar

Görünen adlar uygulamaların Fluent kataloğunda (`color-theme-<ad>`; TR aşağıda, EN taslak
hâlinde). Kimlikler kalıcıdır (`framework.toml`'a yazılır).

| Kimlik | Ad (TR) | Tohum | Birincil | İkincil | Üçüncül | Nötr / varyant | Karakter |
|---|---|---|---|---|---|---|---|
| `blue` (**varsayılan**) | Nuxut mavisi | `#9ec5ff` | tohum, 36 | tohum, 16 | +60°, 24 | tohum, 6 / 8 | Serin, derin gece mavisi; bugünkü Nuxut. |
| `neutral` | Nötr | `#c2a68a` | tohum, 22 | tohum, 9 | 112° (zeytin), 16 | 70° (sıcak), 4 / 7 | Taş ve kum: sıcak gri zemin, kil-kum vurgu, zeytin dokunuşu. Renk yalnızca imada. |
| `sage` | Adaçayı | `#9dbb98` | tohum, 26 | tohum, 11 | +75° (sisli mavi), 18 | tohum, 4 / 7 | Griye çalan yumuşak yeşil; sakin, doğal. |
| `rose` | Gül kurusu | `#cf9a9f` | tohum, 30 | tohum, 13 | +55° (altın), 20 | tohum, 4 / 7 | Tozlu pembe vurgu, sıcak gri zemin; zarif, yumuşak. |
| `amber` | Kehribar | `#e2a85c` | tohum, 40 | −5°, 15 | −40° (kiremit), 26 | −5°, 5 / 8 | Bal sarısı vurgu, kahveye çalan zemin; sıcak, akşam ışığı. |
| `graphite` | Grafit | `#a0a6b0` | tohum, 6 | tohum, 3 | tohum, 8 | tohum, 1,5 / 2,5 | Neredeyse renksiz, serin gri; vurgu yalnızca ton farkıyla. Dikkat dağıtmayan. |

Seçim gerekçesi: altı tema renk çemberinde birbirinden uzak (mavi, kum, yeşil, pembe, kehribar,
renksiz; `themes_are_distinct` testi birincil renkte hue > 20° ya da kroma > 12 fark ister) ve
hepsi düşük-orta doygunlukta — DESIGN "sakin, derin, yumuşak" dili. Mor tema bilerek yok (DESIGN
yasakları: mor gradyanlar; aurora'nın mor ışığı mavi temada kalır). Doygun "neon" tema yok.

## Kaynak: iki kip

`framework.toml [colors]`:

```toml
source = "static"     # "static": Nuxut renk teması | "wallpaper": duvar kağıdından
theme  = "blue"       # blue | neutral | sage | rose | amber | graphite
```

1. **"Nuxut teması seç, duvar kağıdı ona uysun"** (`source = "static"`): tema temel paleti verir;
   yerleşik Nuxut duvar kağıtları temanın renklerine yeniden boyanır (aşağıda). `Theme::color_theme
   = Some(id)`.
2. **"Renkleri duvar kağıdından al"** (`source = "wallpaper"`, bugünkü yol, değişmedi):
   `palette.toml` (Ayarlar üretir; şema varyantı — Nötr dahil — orada seçilir). `Theme::color_theme
   = None`; duvar kağıdı olduğu gibi çizilir. `theme` alanı yalnızca palet dosyası eksik/bozukken
   altta kalan temel paleti seçer.

Açıkça yazılmış renkler (`[colors] surface = …`) iki kipte de en üstte kalır (katman sırası
değişmedi: tema paleti ← palette.toml (yalnız wallpaper) ← açık renkler → yüksek karşıtlık).
Bilinmeyen `theme`: uyarı + `blue` (dosyaya yazılmaz). Tema değişimi de diğer renk değişimleri
gibi `motion.effects` yayıyla akar (AnimatedTheme).

## Duvar kağıtları renge uyar

Yerleşik duvar kağıtları (shell `crates/nuxut-wallpapers`, 9 tasarım × koyu/açık) **vektördür**:
yalnızca `path`/`use`/doğrusal gradyan; renkler `fill="#rrggbb"` ve `stop-color="#rrggbb"`
öznitelikleri. Hepsi Nuxut mavisiyle tasarlanmış. En kaliteli yol bu yüzden **render'dan önce
SVG'nin renklerini değiştirmek**: piksel tonlaması/LUT gerekmez, gradyanlar ve kenar yumuşatma
yeni renklerle yeniden hesaplanır, maliyet SVG başına birkaç düzine renk dönüşümü (µs).

- `nuxut_theme::recolor_svg(svg, map)`: yalnızca renk özniteliklerinin (`fill`, `stroke`,
  `stop-color`, `flood-color`, `lighting-color`, `color`) tırnaklı `#hex` değerlerini eşler;
  `href="#k"`, `url(#g)` dokunulmaz (test).
- `ColorTheme::recolor(&BLUE, c)`: Nuxut mavisiyle çizilmiş rengi temaya taşır. **Ton (algısal
  parlaklık) aynen korunur** — görselin ışık düzeni ve üstündeki arayüzle karşıtlık değişmez. Hue
  ve kroma, rengin kromasına göre iki eşleme arasında yumuşakça (smoothstep) karışır: kroması
  kaynak nötr-varyantınki (8) kadar olan zemin renkleri **nötr** paletin farkıyla döner ve nötr
  kroma oranıyla ölçeklenir; kaynak birincilinki (36) ve üstü vurgular **birincil** paletin
  farkıyla döner (vurgular arası hue aralığı korunur — auroranın üç ışığı yeni temada da üç ayrı
  ışık), birincil kroma oranıyla ölçeklenir. Grafit'te vurgu kroması 6/36'ya iner: duvar kağıdı
  neredeyse tek renkli olur.
- Zemin sakinleştirme (`wallpaper_chroma`, tema başına, ekran görüntüleriyle göz kararı): mavinin
  lacivert koyu zeminleri sıcak hue'larda aynı kromada çamurlu kahveye döner; zemin tarafının
  (vurgu ağırlığı düşük renkler) kroması ek olarak çarpılır — Nötr 0,55, Gül kurusu 0,7, Kehribar
  0,85, Adaçayı 0,9; vurgular (kuşlar, iz, aurora ışıkları) etkilenmez.
- `blue` seçiliyken eşleme kimliktir (SVG baytları değişmez, önbellek anahtarı aynı).
- Kullanıcının kendi görselleri yeniden boyanmaz (fotoğraf tonlaması kalitesizdir; renkler
  "duvar kağıdından al" kipinde zaten görselden gelir).

Shell tarafı (`look` modülü; framework API'si hazır): `nuxut-wallpapers` render
girdisi `recolor_svg(builtin.svg(v), |c| theme.recolor(&BLUE, c))`, `image_key`'e tema kimliği
girer, `theme_changed` → `Theme::color_theme` değiştiyse yeniden render (kip değişimiyle aynı
yol, çapraz geçiş). Kilit ekranı aynısını yapar.

## Kontrast

Framework contrast yardımcılarıyla (`nuxut_theme::contrast`) her tema × kip (koyu/açık) ×
karşıtlık (standart/yüksek) için testle (`every_theme_meets_contrast_in_every_mode`):

- metin ≥ 4,5:1 — `on_surface`, `on_surface_variant` üç yüzeye (`surface`, `surface_container`,
  `surface_bright`) karşı; `on_primary`/`primary`, `on_error`/`error`;
- grafik ≥ 3:1 — `primary`, `error` üç yüzeye karşı (odak halkası, etkin gösterge);
- yüzey kipi doğru (`ThemeMode::of_surface`) ve yüzey tonları sıralı (tonal ayrım);
- yüksek karşıtlıkta ek olarak `primary`, `error` ≥ 4,5 ve `outline` ≥ 3.

Dış uygulamalara giden 57 rolün `on_X`/`X` çiftleri nuxut-palette testinde aynı eşiklerle
denetlenir.

**Yüksek karşıtlık kipiyle ilişki:** tema ile karşıtlık birbirine diktir. `contrast = "high"`
seçili temanın paletini `Palette::high_contrast`'tan geçirir (hue/kroma korunur, tonlar uca
itilir; docs/design/accessibility.md §1); tema seçimi yüksek karşıtlığı kapatmaz, yüksek
karşıtlık temayı griye çevirmez. Grafit'te vurgu kroması düşük olduğu için odak ve seçim tonla
ayrışır — standart kipte de ≥ 3:1 garanti.

## Uygulama payları

- **framework** (bu iş): `ColorThemeId`, `ColorTheme` (`palette`, `dynamic_scheme`, `recolor`),
  `recolor_svg`, `[colors] theme`, `Theme::color_theme`; nuxut-palette
  `Palette::from_color_theme` (dışa aktarım için iki kipin tam rol kümesi).
- **shell / look**: statik kaynakta dış uygulama paleti `Palette::from_color_theme(theme)`'dan
  (bugün `Seed::new` temanın `primary`'sini tohum alıyordu; açık kipte tohum kayıyordu).
  Yerleşik duvar kağıdı yeniden boyama (yukarıda).
- **settings**: Görünüm → "Renkler" bölümünde kip seçimi (Nuxut teması / duvar kağıdından) ve
  tema seçici: önizleme kartları (temanın kendi `surface`/`surface_container`/`primary`/ikincil
  metin tonlarıyla mini arayüz + yerleşik duvar kağıdı önizlemesi), Türkçe adlar; yazım
  `[colors] theme`, `source = "static"`.

## Bilinen sınırlar

- Görselin kendisi görülemedi; "Nötr" tarif üzerinden. Kullanıcı görseli verirse ayarlanır.
- `theme` ile `source = "wallpaper"` birlikte: tema yalnızca palet dosyası yokken görünür.
- Kullanıcı görselleri yeniden boyanmaz (bilinçli).
