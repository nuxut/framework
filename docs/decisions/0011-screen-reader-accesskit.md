# ADR-0011 — Ekran okuyucu: AccessKit üzerinden AT-SPI

Tarih: 2026-10-15 · Durum: kabul edildi (t127; yol onayı 2026-10-13, uygulama
framework). Ön araştırma ve ölçümler: `docs/design/accessibility.md` §4.

## Bağlam

MVP hedefi: bar, başlatıcı ve Ayarlar Orca ile kullanılabilsin. Linux'ta bunun yolu AT-SPI2:
uygulama erişilebilirlik ağacını a11y bus'ında D-Bus nesneleri olarak yayımlar, değişiklikleri
olay olarak gönderir. Framework'te merkezi bir bileşen ağacı yok; uygulama bileşen yapılarını
kendisi tutar ve `set_rect` → `pointer`/`key` → `draw` çağırır, klavye sırası `FocusScope` ile.

## Seçenekler

| | AccessKit (`accesskit` + `accesskit_unix`) | Doğrudan AT-SPI (`atspi` crate'i ya da elle `zbus::interface`) |
|---|---|---|
| Bizim yazacağımız | Yalnız köprü: bileşen → düğüm, runtime bağlantısı (~1,5 bin satır) | AT-SPI sunucu tarafının tamamı: `Accessible`/`Component`/`Action`/`Text`/`Value`/`Selection`, `Cache`, olay sıralaması, etkin torun, metin ofsetleri — AccessKit'te ~17 bin satır (`accesskit_consumer` + `_atspi_common` + `_unix`) |
| Bakım | AccessKit kuruluşu; egui, Slint, Bevy, xilem kullanıyor. Ayda bir 0.x sürüm (kırıcı olabilir) | Bütün protokol uyumu bizde; Orca'nın beklentileri değiştikçe biz izleriz |
| Boşta | 0 CPU (ölçüldü: a11y kapalı/açık, istemcisiz), +~3 MB RSS, +3 thread | Aynı sınıf: bir zbus bağlantısı + yürütücü thread'i kaçınılmaz |
| Ağırlık | +~1,7 MB ikili (strip); shell'de zbus 5 zaten var, ek: accesskit*, atspi*, zbus_xml, zbus-lockstep, phf, uuid | Daha az crate, çok daha fazla kendi kodumuz |
| Fark/olay üretimi | Uygulama tam ağacı **iter**; `accesskit_consumer` önceki ağaçla karşılaştırıp yalnız gerçek değişikliğe olay üretir — bizim "her karede yeniden kur" modelimize birebir | Fark hesabını da biz yazarız |

## Karar

1. **AccessKit.** `accesskit 0.25` + `accesskit_unix 0.24` (varsayılan `async-io` yürütücüsü;
   tokio yok). Doğrudan AT-SPI'nin tek kazancı birkaç crate; bedeli protokolün tamamını
   yeniden yazıp bakmak.
2. **Katmanlar.**
   - `nuxut-ui::access` (koşulsuz): `accesskit` saf veri crate'idir (tek zorunlu bağımlılık
     `uuid`, platform kodu yok; ADR-0003 cross-platform kuralına uyar). `AccessTree` kurucu,
     kararlı düğüm kimliği, `AccessRequest`, MVP bileşenlerinin `access` yöntemleri. API her
     zaman var: uygulama kodunda `cfg` gerekmez.
   - `nuxut-wayland` `a11y` özelliği (cephe `nuxut/a11y`, varsayılan **kapalı**):
     `accesskit_unix` adaptörleri, etkinleşme/eylem için calloop kanalı, `App::accessibility`
     çağrısı. Özellik kapalı derlemede adaptör ve D-Bus bağlantısı hiç yoktur; `App`
     yöntemleri hiç çağrılmaz.
3. **Tembel başlatma, boşta sıfır iş.**
   - at-spi2-core kurulu değilse (`org.a11y.Bus` D-Bus hizmet dosyası `XDG_DATA_DIRS`'te yok)
     ya da `NUXUT_A11Y=0` ise adaptör kurulmaz: thread, bağlantı, bellek yok.
   - Kuruluysa adaptör ilk yüzeyle kurulur; arka plan thread'i yalnız `org.a11y.Status.IsEnabled`
     değişimini bekler (0 CPU). **Ağaç hiç kurulmaz** ta ki bir yardımcı teknoloji (Orca)
     `IsEnabled`'ı açana dek. Kalan bedel: +~3 MB RSS, +3 thread (karar 4,
     2026-10-13: özellikle derlenmişse adaptör açık).
   - Etkinken ağaç **çizimden sonra** kurulur (bileşen dikdörtgenleri güncel): yüzey çizildiyse,
     etkinleşmede ve `cx.a11y_invalidate(surface)` ile. Animasyon sırasında en çok
     `A11Y_REBUILD_INTERVAL` (100 ms) aralıkla; yüzey son karesini çizince (artık kare istemiyorsa)
     her zaman — son durum kaçmaz. Ek kare ya da zamanlayıcı yok.
4. **Ağaç modeli.** Yüzey başına bir AccessKit ağacı (= bir AT-SPI `Frame`/`Window`), tek süreç
   = tek AT-SPI uygulaması (adı ikili adı). Kök düğüm `Role::Window`, adı uygulamanın verdiği
   katalog dizgisi. Her güncellemede yüzeyin tüm görünür düğümleri gönderilir (fark AccessKit'te).
   Düğüm kimliği = (ebeveyn kimliği, uygulama anahtarı) karması; indis kullanılmaz (liste
   yeniden sıralanınca Orca karışmasın) — liste öğeleri uygulamanın kendi kimliğinden (masaüstü
   kimliği, bildirim id'si, SSID) anahtarlanır. Yinelenen kimlik `debug_assert`, release'te
   düğüm atlanır (AT-SPI nesne yolu çakışması accesskit_unix'te `panic` → abort olurdu).
5. **Eşleme.** Rol, ad (`label`), açıklama (`description`), durum (`disabled`, `toggled`,
   `selected`, `expanded`), değer (sayısal: `numeric_value/min/max/step`; metin: `value` +
   tek `TextRun`), sınır (son `set_rect`, yüzeye göre mantıksal), odak (`TreeUpdate.focus` /
   `active_descendant`), eylemler. Ayrıntılı tablo: `docs/ARCHITECTURE.md` → "Erişilebilirlik".
6. **Eylemler.** AT'nin isteği AccessKit thread'inden calloop kanalıyla ana thread'e taşınır →
   `App::accessibility_action(cx, surface, AccessRequest)`. Yalnız şu eylemler iletilir:
   `Click`, `Focus`, `Increment`, `Decrement`, sayısal `SetValue`, `Expand`, `Collapse`,
   `ScrollIntoView`. **Metin `SetValue`/`ReplaceSelectedText` hiçbir zaman uygulamaya
   ulaşmaz** (aynı-uid bir süreç a11y bus'ından metin alanına — ileride kilit/polkit parola
   alanına — yazamasın).
7. **i18n.** Framework bileşenleri katalog bilmez (nuxut-ui `nuxut-i18n`'e bağımlı değil):
   görünen metni olan bileşen adını o metinden alır; yalnız simgeli düğme, anahtar, kaydırıcı,
   ikon gibi adı görünmeyen düğümlerin adını uygulama `t!()` ile verir. `locale_changed`
   yüzeyleri yeniden çizdiğinden ağaç da yenilenir; ad değişimi olay olarak gider.
8. **Gizlilik.** Ağaç içeriği (ad, değer, açıklama) hiçbir log seviyesinde yazılmaz
   (değişmez 9); `AccessTree`/`AccessRequest` `Debug`'ı yalnız sayı/rol verir. Parola alanı rolü
   `PasswordInput`, değeri yalnız maske (`•` × uzunluk; karar 6, 2026-10-13). Ekranda
   görünmeyen içerik ağaca konmaz. Kilit yüzeylerine adaptör yalnız uygulamanın açık izniyle
   kurulur (`Context::set_a11y_lock_surfaces`; tehdit modeli madde 8, 2026-10-17).

## Tehdit modeli

Eklendi 2026-10-16 (c523922 incelemesinin iki bulgusu — yetki atlatma ve
bilgi sızıntısı, `nuxut-wayland/src/state.rs`; düzeltildi).

**Saldırgan:** oturumda aynı kullanıcıyla çalışan herhangi bir yerel süreç.

1. **Ağacı kim okur, kim eylem çağırır?** a11y bus'ında erişim denetimi yok. `org.a11y.Status.IsEnabled`
   (ve `ScreenReaderEnabled`) oturum bus'ında aynı-uid her sürecin yazabildiği bir özellik; bir
   süreç ayrıca `org.a11y.Bus.GetAddress` ile a11y bus'ına doğrudan bağlanıp registry'yi dolaşabilir.
   Yani "yalnız AT etkinken" kapısı **güvenlik sınırı değildir**, yalnız maliyet kapısıdır (boşta
   sıfır iş). Bu GTK/Qt'nin de kabul ettiği Linux modelidir: aynı uid zaten `/proc`, (yama dışı)
   `ptrace`, Wayland'de ekran yakalama ve sanal klavye ile eşdeğer erişime sahiptir. "Orca
   gerçekten çalışıyor mu" denetimi (süreç adı) aldatılabilir, eklenmedi. Gerçek önlemler aşağıda.
2. **Kilitliyken askı (bulgu 1 ve 2, kapandı).** Önce `State::a11y_event` AT eylemini
   `input_allowed` denetimsiz uygulamaya veriyordu ve ağaç kilitliyken de yayımlanıyordu; kilit
   ayrı süreçte (nuxut-lock) olduğunda kabuğun `cx.lock`'u `None` olduğundan `input_allowed` tek
   başına yetmez. Şimdi: `Context::set_a11y_suspended(bool)`. Bir yüzeyin ağacı/eylemi ancak
   `!askıda && input_allowed(yüzey)` iken açıktır; değilse:
   - AT eylemi atılır (loglanır: yüzey kimliği + eylem türü),
   - ağaç **içeriksiz** gösterilir (yalnız adsız kök; `App::accessibility` hiç çağrılmaz),
   - pencere odağı AT'ye bildirilmez (askı kalkınca gerçek odak geri bildirilir).
   Kabuk, oturum kilidinde (logind `Lock`/`LockedHint`) askıya alır, kilit açılınca sürdürür.
   Kilit yüzeyleri yalnız açık izinle (madde 8). Düşmanca uçtan uca test: askıdayken ağaç içeriksiz,
   eski düğüm yollarıyla `DoAction` uygulamaya ulaşmaz, sürdürünce ağaç geri gelir
   (`tests/a11y_atspi.rs`).
3. **Hassas içerik.** Ekran okuyucu görüneni okumalı, fazlasını değil:
   - Ad/açıklama/değer en çok `MAX_ACCESS_TEXT` (512 bayt) — dış içerik ağaçtan topluca dökülemez.
   - **Pano geçmişi:** öğe adı tam metin değil `nuxut_access::preview` (ilk satır, 40 grafem, "…").
   - **Bildirim gövdesi:** kartın açıklamasında, sınırlı; kilitliyken askı; bildirim gizliliği
     ayarı içeriği gizliyorsa ağaca da konmaz (shell).
   - **Wi-Fi/VPN ve diğer parola alanları:** `Role::PasswordInput`, yalnız maske; AT'den metin
     yazılamaz (karar 6).
   - **Polkit penceresi (nuxut-polkit, ayrı süreç):** MVP dışında (kilit ekranı madde 8); `App::accessibility`
     uygulamadığından ağacı boş çerçevedir ve eylem alacak düğümü yoktur. Benimsenince: yalnız
     ileti, parola alanı (maske), Doğrula/İptal; AT metin yazamadığından Doğrula parolasız sonuç
     veremez — ama boş parolayla PAM denemesi (faillock sayacı) tetiklenmesin diye Doğrula boş
     parolada devre dışı olmalı ve bu testle doğrulanmalı (polkit benimsemesinin kabul ölçütü).
4. **Parola maskesi uzunluğu.** GTK3 (ATK `GtkEntry`), GTK4 (`GtkText`/`GtkPasswordEntry`) ve Qt
   (`QLineEdit` `Password` kipi, `displayText`) erişilebilir metin olarak gizli karakterin aynı
   uzunlukta dizisini verir; ekranda da aynı noktalar görünür. Sabit maske ya da "parola, boş
   değil" durumu, kör kullanıcının kaç karakter yazıp sildiğini duymasını engeller (Orca her
   tuşta "nokta" der). Kazanç yalnız "a11y bus'ındaki aynı-uid süreç uzunluğu öğrenemez" olurdu;
   aynı süreç ekranı zaten yakalayabilir. **Karar:** uzunluk maskesi kalır — kilit ekranı hariç, madde 8 (karar 6,
   2026-10-13); askıda/kilitliyken hiç metin yok. Değiştirmek gerekirse tek nokta
   `nuxut_access::AccessNode::text_content(.., mask = true)`.
5. **Loglar:** a11y modülleri yalnız yüzey kimliği, eylem türü, düğüm/yinelenen sayısı ve askı
   durumunu yazar; ad, değer, açıklama, metin hiçbir seviyede yazılmaz (değişmez 9).
6. **Metin yazma yok:** `SetValue` (dizge), `ReplaceSelectedText` ve tanınmayan eylemler
   `AccessRequest::from_accesskit`'te düşer (karar 6).
7. **AT eylemi erişim vermez (2026-10-17, kabuk HIGH bulgusu).** a11y bus'ı Flatpak
   uygulamalarına da varsayılan olarak izinli: AT eylemi kullanıcı niyeti sayılmaz. Uygulamalar
   (framework değil — anlam uygulamada) **erişim ya da kalıcı değişiklik veren** eylemi
   (dosya seçici onayı, uygulama başlatma, kimlik doğrulama, kilidi açma, pano öğesi seçme)
   AT'den tetiklememeli; AT yalnız okur, odak taşır ve gezinir. Onay yalnız gerçek Wayland
   girdisiyle. Böyle düğümde `Click` eylemi hiç sunulmaz (`node_mut().remove_action`).
   Shell uygulaması: `docs/design/screen-reader.md` "Güvenlik" (shell 31d57fd); düşmanca istemci
   `examples/at_probe`.
8. **Kilit ekranı (2026-10-17).** Kilit yüzeyleri (ext-session-lock) varsayılan olarak ağaç
   yayımlamaz; kilit ekranı uygulaması `lock_session`'dan önce
   `Context::set_a11y_lock_surfaces(true)` ile açar ve şunları üstlenir:
   - ağaçta yalnız saat, tarih, kullanıcı adı, parola alanı ve durum metni; kilit altındaki
     masaüstü (pencere başlıkları, bildirimler, medya, takvim) ağaçta yok — kabuk kilitliyken
     kendi ağacını askıya alır (`set_a11y_suspended`);
   - **parolanın ne değeri ne uzunluğu (bilgi sızıntısı bulgusu, 2026-10-17):** düğüm yalnız
     ad + "boş/dolu" açıklaması taşır; değer, maske, metin parçası ve eylem yok. Madde 4'teki
     uzunluk maskesi kilit ekranında **geçerli değil**. `TextInput::access` kullanılmaz: metin
     parçası (`TextRun`) maskeyi taşır ve accesskit_atspi_common metin aralığı olan giriş
     düğümüne `EditableText` arayüzü açar (yazma isteği `from_accesskit`'te düşse de arayüz
     görünür). Sonuç: `Text`/`EditableText`/`Action` arayüzü ve text-changed olayı yok; düğüm
     tuş başına değişmez, yalnız boş↔dolu geçişinde (olay sayısından da uzunluk çıkmaz). Shell
     `lock/src/view.rs` `password_access`;
   - durum metni saldırgana yarayan ayrıntı taşımaz: PAM modülü mesajı (faillock), kalan deneme
     sayısı ve saniye saniye geri sayım yerine sabit genel metin (shell `access_status`);
   - `App::accessibility_action` en çok `Focus` karşılar (shell kilit ekranı hiçbirini: odak
     zaten gerçek klavye odağıyla parola alanında); kilidi açma, parola ve diğer her şey yalnız
     gerçek girdiyle (madde 7); açma düğmesi varsa `Click` eylemi yok; reddedilen AT eylemi
     loglanmaz ve kilit süreci `nuxut_wayland::a11y` ile accesskit modüllerini her `RUST_LOG`
     değerinde kapatır;
   - boş parolada açma devre dışı: gerçek Enter ya da tıklama PAM denemesi başlatmaz (faillock
     sayacı);
   - askı (`set_a11y_suspended`) kilit sürecini kapsamaz: kilitliyken a11y bus'ındaki aynı-uid
     süreç kilit ekranında görüneni (saat, ad, durum, parola boş/dolu) okuyabilir — fazlasını
     değil.
   Kabul ölçütü: düşmanca istemci kilidi açamaz, parola yazamaz, değerini ya da uzunluğunu
   öğrenemez; Orca odağı okur.
   **Karşılandı (2026-10-17, shell kilit ekranı):** birim `password_leaks_neither_value_nor_length`
   (sahte "abc123": değer, maske, uzunluk yok; 1 ve 6 karakterde ağaç aynı) ve
   `status_hides_attacker_details`. Yalıtılmış a11y ortamında düşmanca istemci parola
   düğümünde yalnız `Accessible | Component` buldu, eylem uygulayamadı, doğrulama başlatamadı;
   Orca "Kilit ekranı" → "Şifre parola metni" → "Boş" → bir kez "Dolu" → "Doğrulanıyor…" →
   "Yanlış şifre" okudu; text-changed olayı yok; "abc123", maske ve uzunluk ne Orca çıktısında
   ne debug günlükte. Bedel: Orca tuş başına geri bildirim vermez. Ayrıntı: shell
   `docs/design/screen-reader.md` "Kilit ekranı".

## Bilinen sınırlar

- **Konum (Wayland).** İstemci pencere konumunu bilemez. Layer yüzeylerinde runtime çıkış
  konumu + anchor + margin + boyuttan kök sınırını kendisi hesaplar (bar/panel/başlatıcı için
  doğru ekran koordinatı). Pencerelerde (Ayarlar) kök (0,0) kabul edilir: `GetExtents(SCREEN)`
  yanlış konum verir; Orca'nın fare incelemesi ve büyüteç odak izleme etkilenir, konuşma ve
  odak okuma etkilenmez.
- **Orca tuş komutları (Hyprland).** libatspi 2.62 Wayland'de tuşları
  `org.freedesktop.a11y.KeyboardMonitor` ile ister; Hyprland bunu sağlamaz. Orca odak/ad/değer
  değişimlerini ve canlı bölgeleri okur; Orca değiştiricisiyle verilen komutlar (düz inceleme,
  "neredeyim") büyük olasılıkla çalışmaz (karar 3, 2026-10-13: belgelenir).
- **Odak almayan yüzeyler.** Bar, OSD, bildirim açılır kartı klavye odağı almaz; Orca
  kendiliğinden okumaz. Bildirim/OSD için canlı bölge (`Live::Polite`), bar için shell'in
  klavye odağı veren bir kısayolu gerekir (shell kararı).
- **Dil.** accesskit_unix `Accessible.Locale`'i boş döndürür; düğüm başına dil taşınmaz. Orca
  sesi kendi/sistem ayarından seçer.
- **Abort profili.** accesskit_unix arka plan thread'i yalnız programcı hatasında
  (`InterfaceExists`/`MissingParameter`, ör. yinelenen nesne yolu) `panic!` eder; profillerimiz
  `panic = "abort"`. Kimlik tekilliği kurucuda garanti edilir (karar 4).
- **accesskit_atspi_common 0.21 etkinleşme sırası (2026-10-16, canlı kabuk testinde bulundu).** Bekleyen
  adaptör ilk güncellemede uygulama listesine sona eklenir, liste kimliğe göre ikili aranır: adaptörler
  kimlik sırasıyla etkinleşmezse panik (abort). Runtime her `Activate`'da adaptörü HEMEN içeriksiz ağaçla
  etkinleştirir (olaylar adaptör sırasıyla gelir); gerçek ağaç çizimden sonra. Çok yüzeyli e2e testi kapsar.
  Üst akışa bildirilecek.
- **0.x kırılmaları.** AccessKit tipleri yalnız `nuxut-ui::access` ve runtime'ın `a11y`
  modülünde; yükseltme tek yerden, sürüm sabit (`=`değil, `0.25`/`0.24` küçük sürüm kilidi).

## Sonuçlar

- Shell ve Ayarlar `nuxut` bağımlılığında `features = ["a11y"]` açar, `App::accessibility` ile
  yüzeylerinin ağacını kurar, `App::accessibility_action`'da isteği ilgili bileşen çağrısına
  çevirir.
- Test: D-Bus'sız birim testleri (`accesskit_consumer::Tree` üzerinde rol/ad/durum/odak) +
  yalıtılmış `dbus-run-session` içinde at-spi-bus-launcher + at-spi2-registryd ile uçtan uca
  test (AT-SPI'den ağacı okuyup karşılaştırır; araçlar yoksa açık mesajla atlanır).
