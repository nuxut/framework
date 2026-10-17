# İçe aktarılan görsel yalıtılmış işçide (ADR-0007 takip)

Durum: **uygulandı (framework, 2026-10-13).** Yaklaşım (kapanış WIP'i: tel v4,
animasyon bilgisi işçide), uygulama ve testler framework. Bağlayıcı API ve
sözleşme `docs/ARCHITECTURE.md` "Video ve duvar kağıdı içe aktarma" (§İçe aktarma, Yalıtım) ve
"Görsel yalıtımı" (Protokol sürüm 4) bölümlerindedir; bu belge gerekçeyi tutar.

## Sorun
`docs/design/wallpaper-sandbox.md` §9 "kapsam dışı, aynı sınıf" bulgusu: `nuxut-video`
`Importer` kullanıcının seçtiği görseli
- durağan ise `Trust::System` ile **süreç içinde** çözüyordu (SVG dahil; `import.rs` eski :357),
- `still_frame`'de mpv'nin güvenilmeyen videodan yazdığı PNG'yi süreç içinde çözüyordu (eski :666),
- animasyonlu ise `is_animated`, `ImageReader::into_dimensions` ve `animation_info` ile kap
  yapısını süreç içinde yürüyordu.
Duvar kağıdı internetten inmiş olabilir; ADR-0007 takibinden sonra bunların hepsi işçiye aittir.

## Karar
1. **Tek ayrıştırma, işçide.** Ana süreç yalnızca ilk 4 KiB'ın imzasına bakar (`is_image`:
   görsel mi, video mu). Görsel ise tek `IsolatedDecoder::decode_file` çağrısı (duvar kağıdı
   profili, `Limits::WALLPAPER`, kırpmasız Cover hedef, ilk kare, anahtar
   `file_key("video-import", yol)`) şunları döndürür: küçültülmüş ilk kare, `animated_source`,
   animasyonsa kare sayısı ve tur süresi. Durağan görselin QOI'si bu kareden yazılır.
2. **Tel v4.** Animasyon bilgisini ana süreçte saymak, güvenilmeyen kabı yine ana süreçte yürümek
   demekti; bu yüzden işçi sayar ve yanıt başlığına koyar (`u32 kare`, `u32 süre_ms`, doyurmalı).
   Ebeveyn tutarlılığı denetler: alanlar yalnızca `ANIMATED_SOURCE` ile; kare 0 (yapı tanınmadı,
   süre de 0) ya da ≥ 2; hata yanıtında sıfır. Aksi `Protocol`.
3. **Ele geçirilmiş işçi yalan söyleyebilir** (küçük sayı). Bu yalnızca ön denetimdir: aşan
   animasyon mpv'ye hiç gitmesin. Kesin sınır mpv'nin `frames=`/`end=` sayacıdır (bütçe 3. kat,
   değişmedi). Kare sayısı bilinmiyorsa (kare 0) akış sırasında kabın tahmini de denetlenir.
4. **Animasyonun tuval boyutu** artık ana süreçte `image` ile okunmaz; kodlayacak olan mpv'nin
   yoklamasından alınır (`Limits::DEFAULT` kenar/piksel kapısı aynı). mpv yoksa ya da kaynağı
   açamıyorsa kaynak görsel olarak kalır (öncekiyle aynı geri dönüş; `Wallpaper` ilk kareyi
   işçide açar).
5. **Çözücü zorunlu** (S3 ile aynı ilke, sessizce süreç içine düşülmesin): `Importer::new(decoder)`,
   `Importer::with_dir(dir, decoder)`; `Default` kalktı. Süreç içi yol yalnızca adıyla uyaran
   `Importer::in_process_unsandboxed(dir)` (örnek/test, `Trust::System`).
6. **Hata yedeğe düşmez.** İşçi hatası (`Unavailable`, `Killed`, `Timeout`, `Protocol`) olduğu
   gibi `ImportError::Image` döner; önbelleğe bir şey yazılmaz. İşçi yokken S1 (temel raster K2-b,
   `Untrusted`; SVG/JXL/AVIF asla) `Sandbox`'ın içindedir — `Importer` ayrı yedek yazmaz.
7. **İptal.** `ImportJob` artık `CancelFlag` taşır; süren görsel çözümünde işçi ≤ 50 ms'de
   öldürülür (`Cancelled`). `Importer::run(.., &CancelFlag, ..)` kırıcı (önceden `&AtomicBool`).

## Kırıcı API
- `Importer::new(Arc<dyn IsolatedDecoder>)`, `Importer::with_dir(PathBuf, Arc<dyn IsolatedDecoder>)`;
  `impl Default for Importer` kalktı.
- `Importer::run(&ImportRequest, &CancelFlag, &dyn Fn(f32))`.
- `nuxut_image::DecodedFile` yeni alan `animation: Option<AnimationInfo>` (literal kurulumlar
  `DecodedFile::from_source` ya da alanı eklemeli).
- Tel sürümü 4: eski işçi ikilisi el sıkışmada reddedilir (`Unavailable`); işçi ve kütüphane
  birlikte kurulur (`install.sh`).
Tüketici: shell/settings bugün `Importer` kullanmıyor; değişiklik gerekmez.

## Testler
- `nuxut-video` `tests/it/isolation.rs`: sahte çözücüyle durağan görsel tek `decode_file` çağrısı
  (sınırlar, hedef, anahtar) ve QOI çözücünün karesinden — kaynak süreç içinde çözülemeyen bir
  dosya; animasyon bütçesi işçinin sayısıyla, mpv'ye gitmeden; dönüştürülemeyen animasyon kaynak
  kalır; çözücü hatası/iptal yedeğe düşmez, önbellek boş; işçisiz gerçek `Sandbox` ile PNG K2-b'de
  içe aktarılır, SVG `Unavailable`, yedek kapalıyken PNG de `Unavailable`.
- `nuxut-image-sandbox`: protokol birim testi (v4 alanları, doyurma, tutarsızlıklar), gerçek
  işçiyle animasyonlu GIF'in kare/süre bilgisi (ve K2-b yedeğinde aynı), kancalı sahte yanıt
  (`SpoofAnimation`: `ANIMATED_SOURCE` + 1 kare → `Protocol`).
