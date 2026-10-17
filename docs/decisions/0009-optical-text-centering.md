# ADR-0009 — Metin ve ikonun optik ortalanması

Tarih: 2026-10-11 · Durum: kabul edildi (t091, kullanıcı hatası)

## Bağlam
Kullanıcının ana PC'sinde (ultrawide, ölçek ~2.5) bar'daki workspace ikonları (Nerd Font
glifleri) hücrenin sağ üstüne, saat metni hücrenin üstüne kayık görünüyordu. Çağıranlar
(shell, bileşenler) layout kutusunu hücrede ortalıyor (`rect.center() - size / 2`); hata
kutunun içindeydi:

1. **Yatay (ikon):** layout genişliği glifin *ilerlemesi*dir. Eş aralıklı olmayan Nerd Font
   ailelerinde (ör. temanın varsayılan `mono`'su `JetBrainsMono Nerd Font`) ikon glifleri
   hücre genişliğini aşar ve sağa taşar: 15 px ikon 1.0'da +1.7, 2.5'te +4.7 fiziksel px sağda.
2. **Dikey (hepsi):** cosmic-text taban çizgisini fontun ascent/descent'ini satırda ortalayarak
   koyar; bu ikisi fonttan fonta farklı dengelenir, büyük harf/rakam bandını ortalamaz.
3. **Piksel yerleşimi:** cosmic-text `LayoutGlyph::physical` taban çizgisini fiziksel piksele
   *keser* (`truncf`): metin her zaman 0–1 px yukarı kayar. Üstüne TrueType ipuçlaması büyük
   harf yüksekliğini ızgaraya çekerek ±1 px daha oynatır. Ölçülen önce: dikeyde −1.75 px'e kadar
   (JetBrainsMono 13 px, 2.5), hep yukarı yönlü.
4. **Optik boyut:** değişken fontların `opsz` ekseni *fiziksel* piksel boyutuyla seçiliyordu;
   2.5 ölçekte 13 px metin 32 px'lik başlık kesimiyle çiziliyordu (Inter 4, Google Sans Flex,
   Material Symbols). Glif biçimi (ve mürekkep kutusu) ölçeğe göre değişiyordu.

## Karar
`TextStyle::optical: OpticalCenter` (varsayılan `Auto`). Layout kutusunun boyutu hiçbir kipte
değişmez; yalnızca glifler kutu içinde kaydırılır. Böylece mevcut "kutuyu ortala" çağrıları
kendiliğinden düzelir, API kırılmaz.

- **Metin → `CapHeight`:** ilk satırın büyük harf üstünden son satırın taban çizgisine uzanan
  bant kutunun dikey ortasına oturur. Büyük harf yüksekliği birincil fontun OS/2
  `sCapHeight`'ı; yoksa `H` glifinin anahattı; o da yoksa kaydırma yok.
  - *Neden mürekkep değil:* mürekkep içeriğe bağlıdır (`gy` inişli, `14:05` değil); saat her
    dakika zıplar, yan yana etiketlerin taban çizgileri ayrışır. Büyük harf bandı içerikten
    bağımsızdır; rakamlar (lining figures) büyük harf yüksekliğindedir, yani saat/yüzde/sayı
    tam optik merkezde durur. Yalnız küçük harfli metin bir tık aşağıda kalır — tipografik
    gelenek (Material, Apple) budur.
  - Çok satırlı metinde tüm blok tek miktarla kayar; satır aralığı değişmez.
- **İkon → `Ink`:** gliflerin ipucusuz anahat sınırlarının birleşimi kutunun ortasına, iki
  eksende. Tek glif ikonda tasarım kutusu (hücre, ilerleme, ascent) fonta göre keyfidir; tek
  güvenilir ölçü çizimin kendisidir.
- **`Auto`:** tek glife şekillenen ve (Özel Kullanım Alanı kod noktası — Nerd Font — ya da
  birden çok karakterin tek glif olduğu ligatür — Material Symbols adı, emoji dizisi) olan
  metin `Ink`, diğer her şey `CapHeight`. Kural koda/şekillendirmeye bakar; font adına değil.
- `Metrics`: eski davranış (gerekirse açıkça seçilir).

Piksel yerleşimi (`text::place_glyph`, Painter'ın tek yolu): ölçek **1.0'da** anahat ipuçlanır
(TrueType hinting) ve taban çizgisi en yakın tam piksele **yuvarlanır** (dikey alt piksel yok;
önceden aşağı kesiliyordu); diğer tüm ölçeklerde anahat **ipucusuz** rasterleştirilir ve iki
eksende çeyrek piksel alt konumla yerleştirilir (önceden yatayda vardı, dikeyde kesiliyordu).
Yatay alt konum her ölçekte var. swash'ın alt piksel kaydırması y-yukarıdır; dikey bölme eksiyle
verilir. `opsz` mantıksal boyuttan gelir (önbellek anahtarı bayraklarının üst 8 biti; `FILL` 16–23).
Bozuk ölçek (NaN/sonsuz/≤0) 1'e düşer; fiziksel glif boyutu 0.25–2048 px'e sınırlanır.

## Ölçüm (birim testi, rasterleştirilmiş mürekkep — `nuxut-render/src/optical_tests.rs`)
Hücre merkezi ile mürekkep merkezi farkı, fiziksel px, ölçekler 1.0/1.25/1.5/1.75/2.0/2.5, üç
farklı piksel evresinde hücre: tüm kurulu sans/mono fontlarda saat rakamları ve Nerd/Material
ikonlarında **≤ 0.5 px**; 1.0 ölçekte dikeyde **≤ 0.75 px** (tam piksel taban çizgisi + ipuçlama;
bu makinede en kötü: metin 0.70, ikon 0.68). Önce: ikon yatayda +4.7, metin dikeyde −1.75 px'e kadar.

## Netlik (1.0 ölçek kararı)
İlk sürüm 1.0'da da ipucusuz + dikey alt pikseldi. İzole sway'de 11–14 px metin (sans ve mono)
main ile karşılaştırıldı: yatay gövdeler (5, E, saat iki noktası) iki piksele yayılıp griye
dönüyordu; ölçü: |Laplace| ortalaması sans −%3.5, mono −%8.5; tam dolu piksel sans −%6, mono −%15.
1.0'da ipuçlama + tam piksel taban çizgisiyle çıktı main ile piksel piksel aynı (aynı konumda).
2.0 ve kesirli ölçeklerde gözle fark yok (yoğunluk yeterli); 2.0'da ipuçlama dikeyde 1 fiziksel
px sapma getirdiği için orada ipucusuz kalır.

## Sonuçlar
- Ölçek 1.0'da metin ipuçlanır ve taban çizgisi tam piksele oturur: düşük yoğunlukta netlik
  korunur; bedeli dikey optik ortalamada en çok 0.75 px (diğer ölçeklerde 0.5).
- Dikey alt konum, glif önbelleğinde glif başına en çok 4 dikey varyant demektir (yatayla 16);
  atlas zaten bölmeli, maliyet küçük.
- Layout başına bir font metriği okuması (`CapHeight`) ve tek glifli ikonlarda bir anahat
  ölçümü; layout önbellekli olduğundan karede maliyet yok.
- Çağıranın ek ofset hack'i gerekmez; olanlar kaldırılmalı.
