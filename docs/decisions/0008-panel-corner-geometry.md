# ADR-0008 — Panel köşe geometrisi: tek tanım, özel dal yok

Tarih: 2026-10-05 · Durum: kabul edildi (kullanıcı kararları 2026-10-05; uygulama milestone sonrası)

## Bağlam
Çerçeve ve paneller tek bir blob alanında çiziliyor (`nuxut-render/src/blob.rs`,
`shaders/blob.frag`). Bugünkü biçim: her panel yuvarlatılmış kutu SDF'i, şekiller ikişer ikişer
dairesel smooth-min (`smin_circular`, yarıçap `theme.blob.smoothing`) ile birleşir. Bu,
yalnızca iki yüzün gradyanları **dik** olduğunda kesin bir çember yayı verir. Diğer durumlar
(çerçeveye sıfır kalınlıkla yapışık tohum, panel köşesinin duvara değmesi, karşılıklı paralel
yüzler) için üç özel düzeltme birikti: kenar yakınlığı (`EDGE_BOOST`, `prox`), duvar çukuru
(`frame_sink`, `pen`) ve ince şekil solması (`extra.w`). Her biri bir yerleşime göre ayarlı.
Panel sağ kenarda tam yükseklikte, alta yapışık ya da köşede olduğunda biçim tutarsızlaşıyor.

## Kural (kullanıcı onaylı)
Köşe çerçeveye ya da panele birleşiyorsa **içbükey**, boşluğa açılıyorsa **dışbükey**. Her
kenar, köşe ve çoklu panel aynı kuralla çizilir. Özel dal yoktur, yarıçaplar token'dan gelir.

## Karar — tanım
`S` dolu küme olsun: çerçeve duvarları ile tüm panel dikdörtgenlerinin birleşimi (keskin
köşeli, deform uygulanmış). `F = ekran \ S` serbest alandır. Çizilen serbest alan şudur:

    H = closing(opening(F, R), r)        dolu = ekran \ H

- `R` birleşim yarıçapıdır (`theme.blob.smoothing`). `opening(F, R)` F'nin dışbükey köşelerini
  yuvarlar. Bunlar dolu tarafta duvar–panel ve panel–panel birleşimleridir, yani içbükey
  filetolar. Çerçevenin iç köşeleri de böyle oluşur, bu yüzden çerçeve iç yarıçapı ayrı bir
  değer değil, `R`'dir (karar 2: tek token, ayrı ayar yok).
- `r` panel köşe yarıçapıdır (`theme.shape` token'ı). `closing(·, r)` F'nin içbükey köşelerini
  yuvarlar. Bunlar panelin boşluğa açılan köşeleridir, yani dışbükey köşeler.
- Sıra önemli: önce birleşimler kapanır, sonra açık köşeler yuvarlanır. Duvara yapışık bir
  panelin üst köşeleri böylece duvara gömülür: çentik oluşmaz, ayrı bir dal da gerekmez.
- Panel `2r`'den küçükken etkin yarıçap `min(r, yarı boyut)` olur. Bu, `closing` tanımının
  doğal sonucudur, sonradan eklenmiş bir kural değildir.

Altın örnekler bu tanımdan türetilir:
| Yerleşim | Beklenen |
|---|---|
| Üst bar'dan inen panel (bugünkü) | iki üst birleşim içbükey `R`, iki alt köşe dışbükey `r`: bugünkü görünüm korunur |
| Sağda tam yükseklik panel | serbest alanın dört köşesi de içbükey `R` (ikisi panel–duvar birleşimi, ikisi çerçeve köşesi), hiç dışbükey köşe yok |
| Alta yapışık panel | üst panelin aynası (8 simetrinin biri) |
| Köşe paneli (üst-sağ) | iki birleşim içbükey `R`, boşluğa bakan tek köşe dışbükey `r` |
| Duvardan `d` uzakta panel, `d` taranır | bkz. süreklilik |

## Süreklilik ile kesinlik çatışıyor (karar gerekiyor)
Kesin morfoloji iki noktada sürekli değil:
1. **Boğaz.** Panel ile duvar (ya da iki panel) arasındaki aralık `d < 2R` iken `opening` aralığı
   tamamen kapatır. `d = 2R`'de ise `2R` genişliğinde bir şerit bir karede açılır. Panel
   büyürken bu bir zıplama olarak görünür.
2. Bugünkü dairesel smin bunun tersini yapıyor: boğazda süreklidir (köprü sıfır genişlikten
   büyür), ama paralel ya da dik olmayan yüzlerde kesin değildir. Duvara sıfır kalınlıkla
   yapışık bir tohum `≈0.29R` yüksekliğinde ani bir tümsek çıkarır. Solma düzeltmesi
   (`extra.w`) bu yüzden var.

**Karar 1 (kabul edildi):** tanım her yerde kesin kalsın, yalnızca boğazda düzenlensin. Karşılıklı paralel iki
yüz arasında `d < 2R` iken birleşim ikili seçim olmasın. Köprü, boğaz genişliği
`w(d) = 2R·s(1 − d/2R)` ile sıfırdan sürekli büyüsün (`s` smoothstep). `d ≥ 2R`'de tanım
birebir kesindir, `d → 0`'da tam birleşimdir. Diğer tüm yerleşimlerde kesin morfoloji zaten
süreklidir. Örneğin büyüyen bir tohumun yan filetosu kalınlıkla sürekli büyür, çünkü
yuvarlanan top tohumun köşe noktasına yaslanır.

## Uygulama (onaydan sonra)
- **CPU'da kesin sınır, GPU'da kesin SDF.** Şekil yalnızca hareket karelerinde değiştiği için
  `H`'nin sınırı CPU'da bir kez kurulur. Bu, eksen hizalı dikdörtgen birleşiminin
  `R`/`r` ofset analitiğidir: doğru parçaları ve çember yayları, tipik olarak 200'den az
  ilkel. Shader parça ve yaylara işaretli uzaklığı hesaplar (işaret sargı sayısından), AA
  gradyanla normalize edilir. Hesap fiziksel pikselde yapılır, kesirli ölçek kendiliğinden
  doğru çıkar. Boşta GPU işi yoktur, karede yalnızca hasar dikdörtgeni çizilir.
- **Deform** (yay esnemesi) eksen hizalı ölçek olarak kalır (kenara yapışık panellerde
  köşegen matris, bugünkü sözleşme). Böylece birleşim dikdörtgen kalır ve tanım değişmez.
- **Gölge, blur maskesi ve girdi bölgesi** aynı `H`'den türer: gölge `field(p − offset)`, blur
  maskesi aynı kapsama, giriş/opak bölge aynı yolun dikdörtgen yaklaşımı. İkinci bir şekil
  tanımı olmaz.
- Silinecekler: `EDGE_BOOST`/`prox`, `frame_sink`/`pen`, `extra.w` solması ve
  `BlobShape.radii`'nin panel başına elle verilmesi (yarıçap token'dan gelir; panel yalnızca
  dikdörtgen + deform verir).
- API etkisi: `BlobShape::new(rect, radii)` → `BlobShape::new(rect)`. `radii` kalkar,
  `BlobField.frame.inner_radius` `R`'ye bağlanır. Sheet/tepsi/bildirim yaprakları
  sadeleşir. Ayrı ADR eki ve geçiş planıyla gelir.

## Testler
- **8 simetri özellik testi (D4).** Ekran dikdörtgenine uygulanan her yansıma/dönme `g` için
  (dönmelerde ekran boyutu da çevrilir) `H(g·yerleşim) = g·H(yerleşim)` olmalı. Rastgele
  yerleşimlerle sınanır, CPU sınır kurucusu üzerinde ve tolerans içinde.
- **Morfoloji özellikleri:** tekdüzelik (`S ⊂ S'` ⇒ `H' ⊂ H`), idempotentlik, yerellik
  (değişiklik yalnızca `R + r` komşuluğunu etkiler; hasar dikdörtgeni buna dayanır).
- **Süreklilik:** panel kenarı ve `d` ince adımlarla taranır. Ardışık karelerin Hausdorff
  uzaklığı, hareket miktarı + ε ile sınırlı olmalı. Tohumdan büyüme ve `d` taraması dahil.
- **Altın örnekler:** tablodaki yerleşimler başsız EGL'de piksel karşılaştırmasıyla (1.0 ve
  1.5 ölçek). GPU kapsaması CPU referansıyla ±1/255 içinde olmalı. "Mevcut üst panel korunur"
  bugünkü çıktıyla karşılaştırılır.

## Kararlar (kullanıcı, 2026-10-05)
1. **Boğaz:** yalnız karşılıklı paralel yüzler arasındaki boğazda köprü genişliği sıfırdan
   smoothstep ile büyür; başka her yerde tanım kesin. Süreklilik testi `d` taramasını kapsar
   (ardışık adımlar arasında Hausdorff farkı sınırlı).
2. **Çerçeve iç köşe yarıçapı = `R`:** tek token (`theme.blob.smoothing`), ayrı ayar yok.

## Uygulama planı
- Sahibi: framework (renderer). Milestone sırasında yalnız hazırlık.
- Adımlar: (1) CPU sınır kurucusu + saf Rust SDF referansı ve özellik testleri (D4, morfoloji,
  süreklilik) — shader'a dokunmadan; (2) shader'ın parça/yay SDF'ine geçişi, GPU↔CPU altın
  karşılaştırma; (3) özel düzeltmelerin silinmesi; (4) API değişikliği.
- Shell geçişi: `BlobShape::new(rect, radii)` bir sürüm boyunca kalır (`#[deprecated]` değil —
  shell `clippy -D warnings` kırılmasın; radii belgeli olarak yok sayılır), yeni
  `BlobShape::new_rect(rect)` eklenir; kabuk önceden bilgilendirilir,
  shell geçişi kendi temposunda iner, sonra eski imza kaldırılır.

## Uygulama notları (2026-10-05, uygulandı)
Yukarıdaki plandan sapmalar ve uygulama sırasında ortaya çıkan kararlar:
- **Morfoloji genel:** yalnız eksen hizalı dikdörtgenler değil, dışbükey dörtgenler (deform'lu
  paneller) ve yaylar. Aşındırma/genişletme işaretli uzaklığın `∓ρ` düzey kümesidir: tek yanlı
  ofset adayları + kırık köşelerde köşe çemberi, kesişimlerde bölme, orta noktası tam `ρ`
  uzaklıkta kalan parçalar (`nuxut-render/src/blob/morph.rs`).
- **Köprü yerine birlikte incelen cep:** Karar 1'in "köprü sıfırdan smoothstep ile büyür"
  kuralı, boğazdaki serbest şeridin tüm örtüşme boyunca birlikte incelmesi olarak uygulandı.
  Şeridin yarı kalınlığı `h = d/2 · (1 − smoothstep(1 − d/2R))`. İlk denemede köprü ağızlardan
  büyüyordu; görüntülerde panel köşelerinin altında "bacak" gibi tümsekler ve sivri uçlu
  mercek şeritleri çıktı, bu yüzden bırakıldı. Sıra şöyle:
  `O = opening(F, R)` (boğazı doldurur; ağızlarda birleşmiş hâlin filetosu),
  `C = closing(O, r)` (cep YOK: kıl kadar ince bir yarık bile `r` topunun geçişini keserdi ve
  panel değdiği anda köşe sıçrardı), `C ∪ cepler` (açık uçlarda şerit ağızdan dışarı uzar,
  duvar tarafındaki ince bant filetoya tümseksiz bağlanır), sonra `closing(·, min(r, 4h))`.
  Bu son kapanış `d = 2R`'de tam `r` yuvarlamasını geri getirir (açık kanalla sürekli),
  `d → 0`'da etkisi `h` ile birlikte kaybolur. Testler: boğaz taramasında hiçbir köşe kırık
  değil (sivri uç ya da tümsek kırık köşe üretir) ve kapsama sürekli.
- **R çağırana açık ve canlandırılabilir:** `smoothing` alanı `R`'dir. Shell tam ekrana
  geçerken onu yayla 0'a sürer. Tanım `R(t)` ile süreklidir (R taraması testi). Önbellek
  anahtarı `R`'yi içerir. Varsayılan `R = 20`, yani bugünkü çerçeve iç köşesi (karar).
- **Dar sırt düzenlemesi iki adımda da var:** açmada dar boğaz incelen cep olur, kapamada
  ince panel yarı kalınlık yarıçaplı kapsül olur. Bu, yukarıdaki "Panel `2r`'den küçükken etkin
  yarıçap" maddesinin genel hali.
- **Süreklilik ölçüsü piksel kapsamasıdır, Hausdorff değil:** sıfır genişliğe inen kıl kadar
  ince bir geçidin kaybolması işaretli uzaklıkta sıçrar, kapsamada süreklidir (görünen budur).
  Testler: boğaz taraması, tohumdan büyüme, yan yana paneller (0.25 px adımda hiçbir piksel
  tümüyle dönmez). Köprünün `r` topunu sığdırdığı an karekök biçimli diktir; adım küçüldükçe
  değişimin sıfıra indiği ayrıca sınanır.
- **Tekdüzelik yalnızca kesin morfolojide:** düzenleme bilerek tekdüze değil (yeni panelin
  açtığı dar cebin ucu yarıçapı `h < R` olan yuvarlak uç, `R` filetosunun doldurduğu köşeye daha çok girer).
  Kesin açma/kapama tekdüzeliği test ediliyor.
- **`BlobShape.radii` kalıyor, anlamı değişti:** şeklin kendi yuvarlaması (yüzen OSD hapı gibi
  şekiller için). Panel `new_rect` ile sıfır verir. `new(rect)` yerine `new_rect(rect)` eklendi;
  `new(rect, radii)` aynen geçerli. `BlobField.corner_radius` (`r`) yeni alan;
  `FrameShape.inner_radius` yok sayılır.
- **Gölge** aynı alandan türer. **Blur maskesi ve girdi bölgesi:** framework'te blob'a bağlı
  bir blur yok. Girdi bölgesi `BlobField::input_region()` ile `H`'den türer (2026-10-10, t052):
  piksel merkezi dolu alandaysa piksel bölgede; satır taramasıyla ayrık, tam piksel
  dikdörtgenler (bkz. ARCHITECTURE "Girdi bölgesi").
- **Maliyet:** şekil değişen karede 3 panel/1080p ≈ 110 µs, 16 şekil ≈ 1 ms; değişmeyen karede
  sıfır (sahne anahtarlı önbellek). GPU↔CPU kapsaması 9 sahne × 1.0/1.5 ölçekte ±3/255 içinde.
- **Çerçeveye yapışık kenar deformla kopmaz (t074, 2026-10-10):** shell'in kenar çekmecesi iç
  alanın tam boyundadır (bir yanı bar'a, öbür yanı çerçeveye birleşik). Alan koruyan esneme
  büyüme ekseninde uzarken dik eksende büzer; çapa kenarın ortası olduğundan birleşik iki kenar
  hareket sırasında çerçeveden kopuyor (≈ 40 px koyu kanal), yay otururken kapanan aralık son
  karelerde kıl kadar ince bir yarık olarak görünüyordu ("animasyon bitince bir anlık glitch").
  Kural: deformsuz hâlde çerçevenin iç kenarına yapışık (ya da onu aşan) şekil kenarı deformdan
  sonra da o kenarda kalır; esneme yalnızca serbest kenarları oynatır
  (`BlobField::joined_corners`, içerik için `BlobField::visual_rect`). Testler: yapışık kenar
  her esnemede kenarda, 1.0–1.3 esneme taramasında birleşimde boş nokta yok.
- **Cep yalnızca `d > R`'de (2026-10-10):** cebin yarı kalınlığı
  `h = d/2 · smoothstep((d − R)/R)` oldu (eskisi `d/2 · (1 − smoothstep(1 − d/2R))`).
  `d = 2R`'de yine kanalın tamamı (açık kanalla sürekli), ama `d ≤ R`'de sıfır: birbirine
  `R`'den yakın paneller ve kenarlar tek parça görünür, eski formülün `d ≈ R/2`'de bıraktığı
  piksel altı yarık kalmaz. Testler: `R`'ye kadar her aralıkta yarıkta boş nokta yok, ağız
  silueti tekdüze (tümsek yok); GPU↔CPU kapsama testi artık hiç ±3 dışı piksele izin vermiyor.
  Eski "dik geçiş adımla sıfıra iner" testi kalktı: `r` topunu sığdıran dik geçiş `d < R`
  aralığına düşüyordu, orada artık cep yok.
