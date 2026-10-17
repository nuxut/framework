# Uygulama ikonları (`app_id` → tema ikonu)

Durum: **uygulandı (framework, 2026-10-14).** İsteyen: kabuk (ekran
paylaşım seçicisi; Nerd glifi olmayan uygulamalar genel pencere ikonu alıyordu). Bağlayıcı API
ve sınırlar `docs/ARCHITECTURE.md` "Görseller (nuxut-image)" → "Uygulama ikonları"; bu belge
gerekçeleri ve kararları tutar. Kararlar framework
incelemesine açık.

## Sorun

Shell, pencere listesinde (`sharepicker_view`, görev çubuğu) uygulama ikonunu Hyprland `class`'ından
elle yazılmış bir Nerd glifi tablosuyla (`app_glyph`) seçiyor; tabloda olmayan her uygulama
`IconName::Window` alıyor. Gerçek ikon `.desktop` dosyasında ve kullanıcının ikon temasında.
Bu genel bir yetenek (shell, Ayarlar, ileride görev değiştirici) → framework'e ait.

## Mevcut parçalar (yeniden kullanıldı)

- `nuxut_image::IconTheme`: Icon Theme Spec araması (tema → `Inherits` → `hicolor` → pixmaps,
  boyut/ölçek, PNG/SVG, dizin listesi önbelleği) ve **güven kapısı** (`load_as`: dosya fd'den
  denetlenir; ev dizini / Flatpak / mutlak yol → `Untrusted`).
- `LoadRequest::run_with(sandbox, key)`: `Untrusted` dosyayı yalıtılmış işçiye verir (ADR-0007).
- `icon::detect_theme_name` (GTK4/3 → qt6ct/qt5ct → kdeglobals → gtkrc-2.0).
- `icon::desktop_icon_name`: yalnızca tam ad/küçük harf, her çağrıda dosya okur — bildirimlerin
  `desktop-entry` ipucu için yeterli, pencere `app_id`'si için değil (StartupWMClass, ters-DNS yok).

Shell'de genel bir kopya yok: başlatıcı kendi `.desktop` kataloğunu (shell'e özgü: arama, eylemler,
kullanım) tutuyor ve `IconTheme` + `Loader` ile ad çözüyor; tepsi `IconThemePath` (uygulamanın
verdiği dizin, güvenilmez) için kendi dar okuyucusunu kullanıyor. İkisi de shell'e özgü kaldı.

## Kararlar

1. **Yeni bağımlılık yok.** `freedesktop-icons` (ve `xdg`) değerlendirildi, reddedildi:
   eşzamanlı ve önbelleksiz/sınırsız önbellekli arama, güven modeli yok (dosya yolu döner; fd'den
   güven denetimi, Flatpak/ev dizini düşürmesi bizde zaten var), `.desktop` eşlemesi yok. Mevcut
   `IconTheme` spec'i zaten uyguluyor; eksik olan yalnızca `.desktop` dizini ve arka plan/önbellek
   katmanıydı.
2. **Konum `nuxut-image`** (`app_icon` modülü), doku önbelleği `render` özelliğinde
   (`AppIconTextures`, `Wallpaper` emsali). `nuxut-ui` görsel çözmez (ADR-0006: dokuyu uygulama
   verir); tüketici `Icon::image(texture)` ile mevcut `IconView`/`ListItem` yolunu kullanır.
   Çözücü (`AppIcons`) uygulama başına paylaşılır, dokular yüzey/renderer başına.
3. **Tek arka plan thread'i, tembel.** İlk `get`'te başlar (runtime'ın sinyal maskesini miras
   alır), `nice` 19, sınırlı kuyruk. `.desktop` dizini ilk işte taranır; UI thread'i hiçbir IO
   yapmaz (tema adı algılama dahil). Bildirim `notify` kapanışıyla (`Wallpaper` kalıbı); kapanış
   yalnızca `Send` ister (`MessageSender` doğrudan yakalanır).
4. **Eşleme sırası** (Icon=`'u olan ilk kural kazanır): tam ad → küçük harf → `StartupWMClass` →
   ters-DNS sonu (iki yön). Aynı kimlikte `$XDG_DATA_HOME` sistemi gölgeler (spec), `Hidden=true`
   kimliği siler. Belirlenimcilik için dizin girdileri adla sıralanır (aynı sınıf/son bileşen
   birden çok girdide → alfabetik ilk).
5. **Yalnızca tam adla tema araması.** `lookup_with_fallbacks`'in `a-b-c → a-b → a` kısaltması
   sembolik ikonlar için doğru, uygulama ikonunda yanlış ikon üretir (`google-chrome` → `google`).
   Adaylar: `Icon=`, Steam (`steam_app_<n>` → `steam_icon_<n>`; Steam'in oluşturduğu girdiler bu
   adı kullanır), `app_id`, küçük harfi, ters-DNS sonu.
6. **Güven:** istek `Trust::System`, yalnızca düşer (mevcut `IconTheme` kuralı). Kullanıcı
   dizinindeki/Flatpak SVG'si yalıtılmış işçide; işçi verilmezse SVG reddedilir ve sonraki aday
   denenir (ör. aynı uygulamanın `hicolor` PNG'si).
7. **Önbellek:** LRU, 256 girdi + 8 MiB piksel bütçesi (64 px RGBA ≈ 16 KiB → bütçe sığar);
   negatif sonuçlar saklanır (görev çubuğunda her karede yeniden aranmaz). Bekleyen girdi
   atılmaz (yinelenen iş olmasın).
8. **Geçersizleşme:** nesil sayacı. `set_theme_name` (framework.toml `[desktop] icon_theme`
   değişince) ve `refresh` (algılanan GTK teması değişti, ikon paketi kuruldu) her şeyi düşürür.
   `applications` üst dizinlerinin mtime'ı her işte denetlenir (birkaç `stat`, iş yokken sıfır);
   değişince yeniden taranır ve negatif sonuçlar silinir — yeni kurulan uygulama elle yenileme
   olmadan görünür. inotify izleyicisi eklenmedi: boşta iş/FD yok, maliyet yalnızca iş anında.
9. **Dış girdi sınırları:** `app_id` compositor'dan gelir (herhangi bir uygulama seçebilir):
   ≤ 255 bayt, `/`/denetim karakteri/baştaki `.` reddedilir; tema adı tek dosya adı bileşeni
   (`IconTheme` kuralı). `.desktop` taraması dosya/derinlik/dizin girdisi/boyut sınırlı,
   `O_NONBLOCK` + fd'den normal dosya (FIFO bloklamaz), bağlı dizin izlenmez.

## Bilinen sınırlar

- Alt dizindeki (`applications/kde4/…`) değişiklik üst dizin mtime'ını değiştirmez; `refresh` gerekir.
- Kullanıcının GTK `settings.ini`'si izlenmez; temayı algılatan uygulama değişiklikte `refresh`
  çağırır (shell'in `look` servisi zaten izliyor). Shell `framework.toml`'daki birleşik temayı
  `set_theme_name` ile verir.
- `StartupWMClass`'ı olmayan ve `app_id`'si `.desktop` adına benzemeyen uygulamalar (bazı Electron/
  Java uygulamaları) `app_id` ile aynı adlı tema ikonu yoksa `Missing` döner (genel ikon).
- XPM ikonlar çözülmez (`IconTheme` uzantıları PNG/SVG).
- Tema dizininde olup tema dışına bağlanan SVG (ör. `hicolor/…/com.visualstudio.code.oss.svg →
  /usr/lib/code/…`) fd'nin gerçek yolu arama dizininde olmadığı için `Untrusted`'tır: işçisiz
  tüketicide çözülmez (sonraki aday denenir). Shell işçiyle verir (galeride işçiyle doğrulandı).

## Tüketici (shell) için

```rust
// Açılışta (uygulama başına bir kez):
let sender = cx.sender();
let app_icons = AppIcons::new(
    AppIconOptions { sandbox: image_sandbox::decoder(&self.image_sandbox), theme: theme.desktop.icon_theme.clone(), dirs: None },
    move || { let _ = sender.send(ShellMessage::AppIcon); },
);
// Tema değişince: app_icons.set_theme_name(theme.desktop.icon_theme.as_deref());
// ShellMessage::AppIcon → ilgili yüzeye request_frame.
// draw içinde (yüzey başına AppIconTextures):
let icon = match textures.get(painter, &app_icons, &w.class, 24, scale.ceil() as u32) {
    Some(t) => Icon::image(t),
    None => app_glyph(&w.class).map_or(Icon::named(IconName::Window), |g| Icon::glyph(&g.to_string())),
};
```

Galeri: sayfa 8 (`NUXUT_GALLERY_PAGE=8`), "Uygulama ikonları (app_id)" kartı.
