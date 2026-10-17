# ADR-0010 — Uluslararasılaştırma (i18n): Fluent katalogları

Tarih: 2026-10-11 · Durum: kabul edildi (t097; onay 2026-10-11)

## Bağlam
Framework, shell, Ayarlar ve takvimdeki kullanıcıya görünen metinler kodun içinde Türkçe dizge
olarak duruyor. Ürün çok dilli olacak; çevirmen kodu okumadan çalışabilmeli,
eksik/bozuk çeviri uygulamayı kırmamalı, dil değişimi yeniden başlatma istememeli. Tarih/saat ve
sayı biçimleme zaten `nuxut-locale`'de (glibc verisi, gömülü; ARCHITECTURE "Locale").

Kullanıcı ve kararlar (girdi): Fluent; repo başına `locales/<dil>.ftl`, dil başına tek
dosya; her anahtarda çevirmen yorumu (nerede, anlam, azami uzunluk, değişkenler); sistem locale'i +
Ayarlar'dan geçersiz kılma; tr → en geri düşüş; gömülü + kullanıcı dizini; RTL'yi API engellemesin;
eksik anahtar/değişken tutarlılığı Rust testiyle; gömülü kullanıcı metni bekçi testi; yanlış
dilde kalmış çeviri iniş öncesi ayrıca denetlenir.

## Karar

### 1. Crate ve bağımlılıklar
Yeni `nuxut-i18n` (`#![forbid(unsafe_code)]`, UI'dan bağımsız; servisler de kullanır), `nuxut`
üzerinden `nuxut::i18n` olarak yeniden dışa açılır.

| crate | gerekçe | alternatif |
|---|---|---|
| fluent-bundle 0.16 (`concurrent` paket yolu) | Project Fluent referans uygulaması: çoğul/seçici, mesaj referansı, terimler, hataya dayanıklı ayrıştırma (bozuk girdi atlanır, kalan dosya kullanılır), döngü/patlama sınırı | gettext (.po; çoğul kuralları C, bağlam zayıf, çevirmen yorumu var ama değişken adı yok); kendi biçimimiz (çoğul kuralları yeniden yazmak) |
| fluent-syntax 0.12 | yorum ve değişken denetimi için AST (test aracı) | — |
| unic-langid 0.9 | dil etiketi + yazı yönü (`character_direction`) | — |
| fluent-langneg 0.13 | istenen diller ↔ mevcut diller eşleştirmesi (`tr-TR` → `tr`) | elle önek eşleştirme |

Hepsi saf Rust, MIT/Apache-2.0; sistem paketi yok. `icu4x` alınmaz: tarih/sayı `nuxut-locale`'de
kalır, çoğul kuralları fluent'in `intl_pluralrules`'ü (küçük gömülü CLDR tablosu) ile gelir.
DEPENDENCIES.md'ye uygulama turunda yazılır.

### 2. Dosyalar ve anahtarlar
- Her repo kökünde `locales/<dil>.ftl` (BCP 47 dil kodu, bölgesiz: `tr.ftl`, `en.ftl`, `de.ftl`;
  bölge ancak gerçekten ayrışırsa: `pt-BR.ftl`). Dil başına tek dosya; içinde `##` bölüm yorumları
  (ekran/bileşen başına).
- Kaynak dil **tr** (ürünün yazıldığı dil) ve **en** zorunlu-tam: ikisinde de her anahtar var (test).
  Diğer diller kısmi olabilir.
- Anahtar ad alanı repo önekiyle çakışmaz: `ui-…` (framework bileşenleri), `shell-…`, `settings-…`,
  `calendar-…` (repo adı; kısaltma yok, karar 2026-10-11). Biçim `<önek>-<alan>-<ad>` kebab-case (`shell-bar-battery-tooltip`). Erişilebilirlik /
  ipucu gibi ikincil metinler mesaj özniteliği: `.tooltip`, `.a11y`.
- **Çevirmen yorumu her mesajda zorunlu** (mesajın hemen üstündeki `#` yorumu), sabit biçim:
  ```ftl
  ## Bar — pil göstergesi

  # Nerede: bar, pil simgesinin üzerine gelince ipucu. Anlam: kalan şarj ve tahmini süre.
  # Değişkenler: $percent (0–100 tamsayı), $time (biçimlenmiş süre, ör. "2 sa 10 dk").
  # [max: 40]
  shell-bar-battery-tooltip = %{ $percent } · { $time } kaldı
  ```
  `[max: N]` grafem cinsinden azami uzunluk (yer darsa zorunlu; yoksa yazılmaz). Yorumda her
  değişken `$ad` olarak geçer.

### 3. Çalışma zamanı
```rust
pub struct Catalog;            // Arc'la paylaşılır; Send + Sync (fluent concurrent bundle)
impl Catalog {
    pub fn load(sources: &[Source], prefs: &LanguagePrefs) -> Self;   // asla başarısız olmaz
    pub fn text(&self, id: &str) -> Cow<'_, str>;                    // değişkensiz
    pub fn format(&self, id: &str, args: &Args) -> Cow<'_, str>;     // $değişkenli
    pub fn attr(&self, id: &str, attr: &str, args: Option<&Args>) -> Option<Cow<'_, str>>;
    pub fn languages(&self) -> &[LanguageIdentifier];                // çözülmüş zincir
    pub fn direction(&self) -> Direction;                            // Ltr | Rtl (zincirin ilki)
}
pub struct Source { pub domain: &'static str, pub embedded: &'static [(&'static str, &'static str)] }
nuxut_i18n::embed!("settings")   // → Source; locales/*.ftl include_str! ile (derleme zamanı liste)
nuxut_i18n::set_current(Arc<Catalog>); current() -> Arc<Catalog>;    // süreç geneli, ArcSwap benzeri
t!("shell-bar-clock")  ·  t!("shell-bar-battery-tooltip", percent = 80, time = s)
```
- **Dil zinciri:** istenen diller (aşağıda) `fluent-langneg` ile mevcut dosyalara eşlenir, sonuna
  `en` ve en sona `tr` eklenir (tekrarsız). Mesaj zincirde ilk bulunduğu dilden gelir: Almanca
  kullanıcıda eksik anahtar İngilizce, Türkçe kullanıcıda (tr tam olduğundan) hiçbir zaman
  İngilizceye düşmez; tr eksikse (yalnızca geliştirme sırasında, test yakalar) en. Hiçbir dilde
  yoksa anahtarın kendisi döner + bir kez uyarı (`tracing::warn`, anahtar başına tekil).
- **Dil seçimi:** `framework.toml [locale] language = "auto" | "<etiket>"` (Ayarlar yazar; şema
  `nuxut-config`'te). `auto` → `LANGUAGE` (iki nokta listesi) > `LC_ALL` > `LC_MESSAGES` > `LANG`;
  `C`/`POSIX`/boş → `en`. Mesaj dili ile tarih/sayı biçimi ayrıdır: biçim `nuxut-locale`'in
  `LC_TIME`/`LC_NUMERIC` seçimiyle kalır (İngilizce arayüz + Türkçe tarih geçerli bir seçim).
  İsteğe bağlı `[locale] formats = "auto" | "<locale>"` aynı dosyada (Ayarlar'da "Biçimler").
  `[locale] hour_cycle = "auto" | "h23" | "h12"` saat:dakikanın 12/24 saatini geçersiz kılar
  (`auto` = biçim locale'i; `nuxut_i18n::time_locale()`); uygulamanın açık biçimi (shell
  `clock.format`) kazanır (2026-10-13).
- **Kullanıcı dizini:** `$XDG_DATA_HOME/nuxut/locales/<domain>/<dil>.ftl` gömülünün **üstüne**
  yazar (topluluk çevirisi, düzeltme; anahtar başına geçersiz kılma). Güvenilmeyen girdi: düz dosya,
  symlink izlenmez (`O_NOFOLLOW|O_NONBLOCK`), dosya ≤ 1 MiB, ≤ 10 000 mesaj; ayrıştırma hatalı
  girdiler atlanır + tek uyarı (içerik loga yazılmaz). Biçimlenmiş çıktı ≤ 4 KiB (`clip_text`).
  Çeviri düz metindir: hiçbir bileşen çevrilmiş metni biçimlendirme/bağlantı olarak yorumlamaz.
- **Değişkenler:** sayılar `NUMBER` yerine çağıranın `nuxut-locale` ile biçimlediği metin olarak ya
  da çoğul seçimi için ham sayı olarak geçer (`$count` → `[one]`/`*[other]`). Fluent'in `NUMBER`
  işlevi `nuxut-locale` ayırıcılarıyla yeniden tanımlanır (glibc verisi; tek doğruluk kaynağı).
  Tarih/saat her zaman önceden biçimlenmiş metin.
- **Yön ve yalıtım:** zincirin ilk dili RTL ise (`ar`, `he`, `fa`, `ur`…) `Direction::Rtl` ve
  Fluent'in FSI/PDI yalıtımı açık; LTR dillerde yalıtım kapalı (görünmez karakter yok, ölçüm
  değişmez). Bugün bileşenler aynalanmaz; ama **yeni API Left/Right değil Start/End ile** yazılır
  (MenuAlign zaten öyle), metin hizası `Direction`'dan okunabilir — RTL sonradan eklenince API
  kırılmaz. Yazı yönü metin şekillendirmede cosmic-text bidi'sine bırakılır.
- **Canlı dil değişimi:** framework.toml izleyicisi `[locale]` değişince yeni `Catalog` kurar,
  `set_current` eder ve uygulamaya `Event::LocaleChanged` gönderir (tema değişimiyle aynı yol:
  bileşenler metinlerini yeniden ister, yerleşim yenilenir, tek kare). Boşta iş yok; katalog
  yalnızca değişimde kurulur.
- **Bellek/başlangıç:** yalnızca zincirdeki diller ayrıştırılır (genelde 2: istenen + en, tr ise 2);
  gömülü metin rodata'da, ayrıştırılmış kaynak dil başına tahmini 100–300 KiB. Başlangıç maliyeti
  birkaç ms (ölçülecek; bütçe 5 ms / 2000 mesaj).
- **Sözde dil:** `NUXUT_LANG=qps` (yalnızca test/geliştirme) her mesajı aksanlı ve %35 uzatılmış
  üretir (`[Ŝéţţîñĝš·····]`): çevrilmemiş gömülü metni ve taşmaları ekranda yakalar (izole sway).

### 4. Doğrulama kapıları
- **Katalog testi (`nuxut_i18n::check`, her repo bir `#[test]` ile çağırır):** her dosya hatasız
  ayrışır; anahtar tekrarı yok; tr ve en anahtar kümeleri eşit; her dilde mesaj başına değişken
  kümesi tr ile aynı (eksik/fazla `$x` hata); her mesajın yorumu var ve yorum her değişkeni anar;
  `[max: N]` varsa tr/en/diğer tüm dillerde değer ≤ N grafem; mesaj/terim referansları çözülür;
  önek repo ad alanına uyar; kısmi diller için kapsam yüzdesi yazdırılır (hata değil).
- **Gömülü metin bekçisi (`nuxut_i18n::check::source_guard`):** `src/**/*.rs`'deki dizge
  sabitlerini (düz, ham, bayt, C) tarar. Hata: (a) Türkçeye özgü harf (çğıöşüÇĞİÖŞÜ) taşıyan
  sabit; (b) `DEFAULT_UI_CALLS`'a (`Label::new`, `Button::new`, `Toast::new`, `Tooltip::new`…,
  `with_/set_` + `label|text|title|subtitle|description|summary|placeholder|tooltip|hint`;
  repo ekleri `add_ui_calls`) argüman olarak verilen, yer tutucu dışında harf içeren sabit.
  Loglar ve CLI tanı çıktısı çevrilmez (geliştiriciye). Muafiyet dardan genişe:
  - *Yapısal (izin satırı gerekmez):* yorumlar; tüm öznitelikler (`#[error("..")]` dahil);
    `#[cfg(test)]`/`#[test]` öğeleri (`not`/`any` içermeyen `cfg`), `#[cfg(test)] mod x;` ile
    bağlanan dosyalar ve alt modülleri, `tests/` `examples/` `benches/` `fixtures/` dizinleri;
    `impl Debug` gövdeleri; hata/uyarı türünün (adı `..Error`/`..Warning`, ya da dosyada
    `impl Error for Tür` / `#[derive(.., Error)]`) `Display` gövdesi ve kurucuları
    (`XError::A(..)`/`{..}`, `io::Error::other(..)`, kendi `impl`'inde `Self::A(..)`) — başka
    türün `Display`'i denetlenir (`to_string()` ile arayüze gidebilir); log/tanı makroları
    (`tracing`/`log` düzeyleri ve span'ler, `print*!`/`eprint*!`, `panic!`, `unreachable!`,
    `todo!`, `assert*!`, `include_*!`, `env!`, `compile_error!`), `.expect(..)`, `t!(..)`;
    `add_diagnostic_calls` ile verilen repo geneli tanı çağrıları. CLI ikilileri (`src/main.rs`,
    `src/bin/**`) ve proc-macro crate'lerinde Türkçe harfli sabit muaf ama not olarak raporlanır.
  - *Satır içi işaret* `// i18n: tanı <gerekçe>` (düz `//`; belge yorumu değil): satır sonundaysa
    o satırda başlayan, tek başına satırdaysa sonraki deyimin Türkçe harfli sabitleri. Gerekçesiz,
    `tanı` dışı ve hiçbir bulguyu bastırmayan (gereksiz) işaret hata.
  - *`i18n-allow.txt`* (köke göreli düz yol; `..`, mutlak yol, `.` parçası, glob yok; her satır
    gerekçeli): `dosya:çağrı()  gerekçe` (o dosyada `ad()`/`Tür::ad()`/`alıcı.ad()` argümanları
    tanı; satır kaymasından etkilenmez — tercih edilen), `dosya:satır  gerekçe`; `dosya:*` ve
    `dizin/**` desteklenir ama her kullanımda uyarı notu üretir (yeni UI metni de kaçar).
    Öncelik `dosya:satır` > `dosya:*` > en dar `dizin/**`. Gerekçesiz, bozuk, yinelenen ya da
    kullanılmayan satır hata (liste yalnızca küçülür); geçiş istisnası: taranan dosyayı kapsayan
    ama yapısal muafiyetle gereksiz kalan `dosya:*`/`dizin/**` yalnızca not.
  UI kurucusunun argümanı hata kurucusu, tanı çağrısı, işaret ya da `dosya:çağrı()` içinde de
  bulgudur; orada yalnızca harfsiz ve makine biçimli dizge (yol/URL, küçük harfli `snake_case`
  ya da noktalı kimlik, `-symbolic` ikon adı) muaftır; kesin muaf bölgeler (test, log/tanı
  makrosu, `t!`, öznitelik) onu da kapsar. Bilinen sınır: Türkçe harf taşımayan ASCII Türkçe
  (`"bilinmeyen hedef"`) ve değişkene atanıp UI'a giden sabit yakalanmaz; taşımada elle taranır.
  Ayrıntı: ARCHITECTURE i18n kural 8 ve `check/guard.rs` modül belgesi. (Güncelleme:
  2026-10-13.)
- **Hata metinleri (karar 2026-10-11):** kütüphane hata/uyarı türlerinin `Display`/`Debug`
  metni tanı metnidir, loglar gibi çevrilmez (bekçi bunları yapısal olarak tanır — hata türünün
  Display/kurucuları, Debug, log makroları; ARCHITECTURE i18n kural 8 — `dosya:*` gerekmez). Kullanıcıya gösterilen hata metnini uygulama, hata **türünden** kendi anahtarıyla
  (`ui-…`/`shell-…`/`settings-…`) üretir; arayüzde `e.to_string()` gösterilmez. Config uyarıları
  arayüzde gösterilecekse framework uyarıyı türlü (anahtar + değişkenler) verir, metni uygulama çevirir.
- **Dil kapısı:** bir dalda `locales/*.ftl` değişince her değerin dili denetlenir: `en.ftl`'de
  Türkçe kalmış ya da `tr.ftl`'de İngilizce kalmış çeviri DUR. `cargo test`'te değil, iniş
  kapısında.

### 5. Geçiş planı
1. framework çalışma zamanı (yalnızca kod): `nuxut-i18n` + `check` + `source_guard` +
   `embed!`/`t!` + `[locale]` şeması + `LocaleChanged`. Metin içermez; testler için küçük fikstür
   `.ftl` dosyaları.
2. Metin taşıma — framework'ün kendi `ui-…` metinleri dahil, sonra settings, shell, calendar:
   çıkarma + anahtarlama + `locales/tr.ftl` yorumları + `en.ftl` çevirisi (sayfa/modül başına
   küçük parçalar, her parça gözden geçirilir); kapı = katalog testi + bekçi + dil kapısı +
   derleme/test/clippy. Bekçi testi ilk turda `i18n-allow.txt` geçici
   listesiyle girer, liste her parçada küçülür, boşalınca kalıcı.
3. Ayarlar'a "Dil ve biçimler" satırları.

## Sonuçlar
- Çevirmen yalnızca `.ftl` görür; yorum + `[max]` + değişken listesi bağlamı taşır.
- Eksik çeviri asla boş metin ya da panik değil: zincirde bir sonraki dil, en kötü anahtar.
- Bağımlılık: 4 saf Rust crate (fluent-bundle zinciri ~10 küçük crate), sistem paketi yok.
- Kararlar (2026-10-11): zincir `istenen → en → tr` (tr kaynak ve tam); `[max]` grafemle
  ölçülür (CJK gelince hücre genişliğine geçilebilir); shell `msg` CLI yanıtları ve loglar çevrilmez.
