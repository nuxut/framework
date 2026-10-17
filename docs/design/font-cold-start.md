# Soğuk font açılışı (2026-10-16/17)

İstek (2026-10-16): dosya seçici ve paylaşım seçicisinin ilk açılışında `TextSystem` beklemesi
5,4 sn, ilk kare 1,16 sn ölçülmüştü (shell `docs/design/file-chooser.md`, debug derleme, yük
altında VM, Nuxut font önbelleği boş). Hedef: soğuk font önbelleğiyle ilk kare < 1 sn.

## Ölçüm: zaman nereye gidiyor

`cargo run --release -p nuxut-render --example font_cache_bench [-- --evict]` (890 yüz, 981 dosya,
624 MB; `--evict` font dosyalarının sayfa önbelleğini `posix_fadvise(DONTNEED)` ile boşaltır).
Bağımlılıklar dev profilinde de `opt-level = 2` olduğu için debug derleme aynı sırayı verir.

1. **Tarama G/Ç'ye bağlı, ayrıştırmaya değil.** Sayfa önbelleği sıcakken fontdb'nin tam taraması
   ~25 ms; soğuk diskte 2,2 sn. fontdb her dosyayı tek iş parçacığında sırayla açar: yüzlerce
   dosya × birkaç rastgele okuma, kuyruk derinliği 1.
2. **Önbellek geçerliyken bile ilk şekillendirme ~0,8 sn** (soğuk disk). Neden: cosmic-text
   `get_font_matches` her yeni öznitelik kümesinde (aile/ağırlık/stil) veritabanındaki ağırlığı
   istenenden farklı HER yüzün dosyasını açıp `wght` eksenine bakar (`FontMatchKey::new`). Ailesi
   alakasız yüzler dahil ~600 dosya, yine sıralı.

## Tasarım

- **Paralel tam tarama** (`nuxut-render/src/font_scan.rs`, Linux). Dosya listesi fontdb'nin
  dolaşma sırası ve sembolik bağ/tekrar kuralıyla aynen çıkarılır; dosyalar en çok 8 iş
  parçacığında fontdb'nin kendi ayrıştırıcısıyla (`Database::load_font_file`) ayrıştırılır, yüzler
  liste sırasıyla eklenir. Sonuç `FontSystem::new()`in veritabanıyla yüz yüze, sırasıyla aynıdır
  (`parallel_scan_matches_fontdb`). fontconfig bir kez okunur (kayıtlar + dizinler + takma adlar).
- **Artımlı geçersizleştirme** (`font_cache::Loaded::Stale`). Önbellek eskiyse atılmaz: kaydı
  değişmemiş dizindeki dosyaların yüzleri yeniden kullanılır, yalnızca değişen dizinlerdeki
  dosyalar ayrıştırılır (`stale_cache_reparses_only_changed_dirs`). Bir font paketi güncellemesi
  ya da kullanıcının tek font eklemesi sonrası ilk açılan süreç tüm diski taramaz. Bu, "kabuk
  açıkken alt süreçlerde tarama yok" isteğini kabuğa izleyici eklemeden karşılar: kabuk ya da ilk
  açılan süreç önbelleği yazar, ötekiler hazır dosyayı okur; değişiklik olduysa yalnızca değişen
  dizin taranır.
- **Ön okuma** (`font_scan::prefetch`). `TextSystem::new` sonunda kısa ömürlü bir iş parçacığı
  (içinde 4 işçi) her font dosyasının ilk 128 KiB'ını `posix_fadvise(WILLNEED)` ile çekirdekten
  ister: okumalar zaman uyumsuz ve paralel kuyruğa girer, ilk şekillendirme (ve uygulamada aynı
  anda süren EGL/configure) beklerken diskten gelir. Sayfalar zaten bellekteyse maliyet dosya
  başına üç sistem çağrısıdır; iş bitince iş parçacıkları çıkar (boşta iş yok).

Reddedilenler:
- **libfontconfig ile `FcFontList`** (fontconfig'in kendi mmap önbelleği): hızlı, ama ağırlık
  (fontconfig ölçeği), aile listesi (tipografik + eski aile), `monospaced` (glif genişliklerinden)
  fontdb'den farklı türetilir; önbellek yolu ile tarama yolu farklı eşleşme verirdi. C kütüphanesi
  de linklenmeyecekti (ADR-0003, DEPENDENCIES).
- **Tembel aile yükleme / veritabanını sonradan değiştirmek:** cosmic-text `FontSystem` veritabanını
  sahiplenir; sonradan yüz eklemek eşleşme/şekil önbelleklerini geçersizleştirir, ilk kare ile
  sonrakiler farklı fontla çizilebilirdi.
- **cosmic-text çatallamak** (`wght` aralığını yüz başına önbelleğe almak): kök neden orada ama
  bakım yükü; ön okuma aynı kazancın çoğunu çatalsız verir. cosmic-text'e yukarı akış önerisi
  olarak not edildi.

- **Önbellek tazelik denetimi kayıt kayıt** (`font_cache::classify`, 2026-10-17 ölçümünde
  bulundu). Aynı yol birden çok kayıtta olabiliyor (fontconfig adayı aynı zamanda birleştirilen
  dosya); taze kayıtlar kümeye alınıp sayıyla karşılaştırıldığında her açılış "eski" sayılıyor,
  önbellek eski yoldan kurulup yeniden yazılıyordu. Önbellekli `TextSystem::new` (debug) 25–40 ms
  → 1,7 ms. Test `duplicate_records_stay_fresh`.

Reddedilen ek deneme (2026-10-17): font iş parçacığını EGL'den önce başlatmak. Aynı tur
dönüşümlü 8'er ölçümde soğuk ilk kare medyanı 568 ms (EGL'den sonra: 514 ms), önbellekli 229 ms
(255 ms) — fark gürültü içinde, bir önbellekli turda 800 ms kuyruk; önceki karar (EGL'den sonra)
korundu.

## Sonuç (release, aynı makine, yük ortalaması ~24)

| Durum | Önce | Sonra |
|---|---|---|
| Önbellek yok, soğuk disk: `TextSystem::new` | 2186 ms | ~480–640 ms |
| Önbellek yok, soğuk disk: ilk şekillendirme | — | 10–17 ms |
| Önbellek geçerli, soğuk disk: `TextSystem::new` + ilk şekillendirme | 1 + 770–800 ms | ~5–13 + 90–115 ms |
| Önbellek yok, sıcak disk: `TextSystem::new` | ~25–40 ms | ~30–60 ms |
| Önbellek geçerli, sıcak disk | ~1–7 + 7–9 ms | aynı (ön okumalı/okumasız A/B farkı gürültü içinde) |

**Bilinen sınırlar:** dentry/inode önbelleği de soğuksa (açılıştan hemen sonra) ilk ölçümde
önceki yol 5,3 sn gördü; kök yetkisi olmadan boşaltılamadığı için yeni yol bu durumda ayrıca
ölçülemedi (dosya açmaları da paralel olduğu için aynı oranda kazanç beklenir). Yerinde (yeniden
adlandırmadan) değiştirilen font dosyası, tam önbellekte olduğu gibi fark edilmez.

## Ölçüm 2026-10-17 (önce ↔ sonra, aynı makine, aynı turda dönüşümlü)

Font dosyalarının sayfa önbelleği her ölçümden önce `fadvise -a dontneed` ile boşaltıldı (soğuk
disk; diğer süreçlerin eşlediği sayfalar kalır). "Önbelleksiz" = boş `XDG_CACHE_HOME` (Nuxut font
önbelleği yok), "önbellekli" = hemen önceki açılışın yazdığı önbellek.

**İlk kare, uçtan uca** (`nuxut-ui` `gallery` örneği, debug, izole başsız sway + llvmpipe,
`NUXUT_PROFILE_STARTUP=1`, süreç başından ilk kareye; 8 tur, medyan [en az – en çok]):

| Durum | Önce | Sonra |
|---|---|---|
| Önbelleksiz, soğuk disk | 985 ms [833 – 4864] | **514 ms** [467 – 4459] |
| Önbellekli, soğuk disk | 729 ms [212 – 1249] | **255 ms** [201 – 387] |

Hedef (soğuk önbellekle ilk kare < 1 sn) medyanda karşılandı; iki taraftaki 4,5–4,9 sn'lik tek
turlar aynı anda başka derlemelerin disk yükü (değişiklikten önce de görüldü). Makine daha yüklüyken
yapılan önceki turda (6 tur) önbelleksiz medyan önce ~3,5 sn, sonra ~0,85 sn idi.

**`TextSystem::new` + ilk şekillendirme** (`font_cache_bench --evict`, release, 5 tur):

| Durum | Önce | Sonra |
|---|---|---|
| Önbelleksiz, soğuk disk: `TextSystem::new` | 536 ms (500–805) | **156 ms** (149–179) |
| Önbellekli, soğuk disk: ilk şekillendirme | 364 ms (348–435) | **33 ms** (31–44) |
