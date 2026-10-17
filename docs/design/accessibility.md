# Erişilebilirlik (t124)

Durum: taslak, 2026-10-13 (pano t124). Sahibi: framework.
Kapsam: framework tarafı — yüksek karşıtlık tema kipi, azaltılmış hareket, metin ölçeğinin
GTK/Qt'ye aktarım sözleşmesi, ekran okuyucu (AT-SPI) ön araştırması. Shell tarafı (look yazımı,
ayar arayüzü) kabuğun ve Ayarlar'ındır.

## 1. Yüksek karşıtlık kipi

Durum: uygulandı, 2026-10-13 (bileşen karışımları). Kod: `nuxut-theme`
(`contrast` modülü, `Palette::high_contrast`, `Theme::contrast`), `nuxut-palette`
(`Scheme::high_contrast`, `Scheme/Palette::generate_with`, CLI `--high-contrast`), `nuxut-ui`
(`style` karışım yardımcıları, `Interaction::draw_state_layer`).

**Yapılandırma.** `framework.toml`:

```toml
[colors]
contrast = "standard"   # varsayılan | "high"
```

Anahtar yoksa `standard` (geriye uyumlu; standart kipin çıktısı bayt bayt aynı — mevcut
anlık görüntü/golden testleri değişmeden geçer). Geçersiz değer: uyarı + `standard`. Ham şema
`ColorsConfig::contrast: Choice<ContrastMode>`, çözülmüş değer `Theme::contrast: ContrastMode`.
Canlı değişim ayrı yol istemez: runtime framework.toml'u izler, `Theme` değişince
`App::theme_changed` (renkler `AnimatedTheme` ile akar). İzole sway'de doğrulandı: çalışan
galeride `standard → high` yazımı, açılıştan `high` ile alınan görüntüyle aynı kareyi verdi.

**Eşikler (WCAG 2.x AA).** Metin: 1.4.3 ≥ 4.5:1; metin dışı UI (ikon, kenar, odak halkası,
bileşen sınırı): 1.4.11 ≥ 3:1. Yüksek kipte birincil/ikincil metin ve `on_*` rolleri için hedef
7:1'dir (AAA 1.4.6); uç tonda bile ulaşılamazsa 4.5 yine garanti edilir.

| Çift | Tema (11 rol) | Şema (57 rol) |
|---|---|---|
| `on_surface`, `on_surface_variant` (+ `on_background`) / tüm yüzeyler | ≥ 4.5 (hedef 7) | ≥ 4.5 (hedef 7; `surface_variant`'a yalnız 4.5) |
| `primary`, `error` (+ `secondary`, `tertiary`, `success`, `warning`) / tüm yüzeyler | ≥ 4.5 | ≥ 4.5 |
| `on_X` / `X`, `on_X_container` / `X_container` | ≥ 4.5 (hedef 7) | ≥ 4.5 (hedef 7) |
| `inverse_on_surface`, `inverse_primary` / `inverse_surface` | — | ≥ 4.5 |
| `on_X_fixed(_variant)` / `X_fixed`, `X_fixed_dim` | — | ≥ 4.5 (hedef 7) |
| `outline` (tema = Material `outline_variant`) / yüzeyler | ≥ 3 | `outline` ≥ 4.5, `outline_variant` ≥ 3 |

"Tüm yüzeyler": tema için `surface`, `surface_container`, `surface_bright`; şema için
`SURFACE_ROLES` (`background`, `surface`, `surface_dim`, `surface_bright`,
`surface_container_lowest…highest`, `surface_variant`). `primary` metin/bağlantı ve odak
halkası (2 px) olarak kullanıldığından metin eşiğine tabidir.

**Yöntem — estetik önce.** Renk yeniden seçilmez. Her ön plan rengi HCT uzayında yalnız
**tonuyla** (L\*) zeminlerinden uzağa itilir (`nuxut_theme::contrast::ensure_contrast`): fg
zeminlerin ortalamasından açıksa yukarı, değilse aşağı; ikili aramayla eşiği geçen **en küçük**
ton değişimi seçilir, ölçüm 8 bit nicelenmiş renkle yapılır. Hue ve kroma korunur (gamut dışına
taşarsa MCU çözücüsü kromayı azaltır), alfa korunur. Yüzeyler, kap dolguları, gölge ve perde
değişmez — dinamik rengin kimliği (duvar kağıdından gelen ton ailesi) kalır. Çiftte fg uç tonda
bile 4.5'e ulaşamazsa (orta tonlu zemin) zemin de fg'den uzağa itilir (`ensure_pair`). Eşiği
zaten geçen rol bayt bayt aynı kalır: dönüşüm idempotenttir (yüksek karşıtlıklı palet tekrar
geçirilse değişmez).

Değerlendirilen ve reddedilen: MCU `contrast = 1.0` (Material "high contrast" şeması). Eşikleri
fazlasıyla geçer ama `primary`'yi neredeyse beyaza/siyaha, `on_surface_variant`'ı `on_surface`'e
çeker (ör. `#c0602a` koyu: `primary #ffece5`, iki metin rolü de `#ffffff`) — renkli kimlik ve
metin hiyerarşisi kaybolur. Bizim yöntemde aynı kaynakta yalnız `on_surface_variant`
(`#d7c2b9 → #dbc6bc`, hedef 7) ve `outline` (`#52443d → #a08f86`) değişir. Kullanıcının MCU kontrast
düzeyi (`palette.toml meta.contrast`, -1…1) ayrı bir eksendir; yüksek kip onun üstüne uygulanır.

**Kenarlar.** Yüksek kipte `shape.outlines = "none"` → `subtle` (bileşen sınırı her zaman
çizilir, WCAG 1.4.11; DESIGN "Kenarlar ve ayrım"), `full` kenarı tam opak `on_surface_variant`
(standartta %70). Açıkça `subtle`/`full` seçimi korunur.

**Bileşen karışımları.** Karar (2026-10-13; uygulayan): bileşenlerin token'ları
karıştırarak ürettiği renkler de yüksek kipte eşiktedir. Tek merkez `nuxut_ui::style` yardımcıları;
bileşen sihirli sayı yazmaz. Standart kipte her yardımcı eski ifadeyi aynen döndürür (çıktı
değişmez; `standard_mode_is_unchanged`). Yüksek kipte "zemin" gerçek birleşimdir: alfalı dolgular
(kap, state layer, tonal ton) üç tema yüzeyinin üstüne sırayla bindirilir ve ölçüm ekrandaki gibi
8 bit nicelenmiş renkle yapılır. Yöntem yine **en az değişim**: eşiği geçen karışım aynen kalır.

| Karışım | Yardımcı | Yüksek kipte |
|---|---|---|
| Soluk anlamlı metin: ay dışı takvim günü, geçmiş etkinlik, kullanılamayan cihaz | `faded_text` | ≥ 4.5 (önce opaklık artar; yetmezse renk siyah/beyaza en az karışır) |
| Soluk işaret: etkinlik noktası/şeridi, kaydırma göstergesi | `faded_mark` | ≥ 3 |
| Hover/basılı state layer, "satırı parlat", metin seçimi, alan hover tonu | `state_layer(_under)`, `Interaction::draw_state_layer(_under)` | Katman opaklığı üstündeki içerik ≥ 4.5 kalacak kadarla sınırlanır |
| Tonal kap (seçili satır, gezinme göstergesi, tonal düğme, bağlı cihaz), etkinlik zemini | `tinted` (`tonal_container`, `week::event_fill`) | Karışım oranı üstteki metin ≥ 4.5 kalacak kadarla sınırlanır |
| `primary` içerik tonal/renkli zeminde (seçili ikon, bugün) | `accent_on` | ≥ 4.5 |
| Değer göstergesi izi (kaydırıcı pasif kısmı, ilerleme, gösterge, döner, OSD) | `track` | HCT tonu yüzeylere ≥ 3 (hue korunur) |
| Özel gösterge rengi (`Tint::Color`) | `Tint::resolve` | Yüzeylere ≥ 3 |
| Odak halkası | `focus_ring`, `Pressable::draw_focus_on` | Yüzeyler + bitişik bileşen dolgusuna ≥ 3 |
| Ayraç / kenar | `divider` (= `outline()`), `hairline` (döşeme "›" ayracı) | ≥ 3 |
| Rastgele renk üstünde içerik (renk örneği, avatar) | `content_on` | ≥ 4.5 |
| Devre dışı içerik (%38) | `disabled_content` | ≥ 3 (aşağıdaki karar) |

Değer göstergelerinde etkin kısım (`primary`) yüzeye ≥ 4.5, iz ≥ 3; iki koşul ve "dolgu ize ≥ 3"
düz renklerle çoğu palette aynı anda sağlanamaz (iz yüzeyden uzaklaştıkça dolguya yaklaşır), bu
yüzden değer konumu dolgu/iz kontrastıyla değil şekille işaretlenir: kaydırıcıda topuz (dolguya
≥ 4.5), `Progress`/`Gauge`'da boşluk. Anahtarın kapalı rayı kenarla (`outline` ≥ 3), açık rayı
`primary` ile (≥ 4.5) ayrışır; topuz raya ≥ 3 (hover dahil).

*Devre dışı kararı.* WCAG 1.4.3/1.4.11 devre dışı bileşeni muaf tutar; Nuxut yüksek kipte yine de
okunurluğu garanti eder: devre dışı içerik gerçek zeminine (devre dışı kap dahil) **≥ 3:1** (metin
dışı eşiği). 4.5 seçilmedi: etkin metin ≥ 4.5 (hedef 7) olduğundan devre dışı hâl ayırt edilebilir
kalmalı; 3:1 hem okunur hem belirgin biçimde soluktur. Sessiz OSD aynı kurala uyar.

*Hover bedeli.* Katmanın sınırlanması yalnız içerik zaten eşiğe yakınsa devreye girer. Ölçüm
(`report_how_often_mixes_are_limited`, 673 yüksek karşıtlık teması): `primary` içerikli basılı
katman 207 temada sınırlandı, ortalama %87'si korundu (en kötü %2 — `primary`'si tam 4.5'te
duran palet); basılı geri bildirimi düğmede ayrıca sıkışma şekliyle verilir. Tonal karışım hiçbir
temada sınırlanmadı; soluk metin her temada opaklaştı (%38 → eşik).

*Shell / uygulamalar.* `Interaction::draw_layer_with` (ham) geriye uyum için sınırsız kalır;
kendi bileşenini çizen tüketiciler `draw_state_layer(_under)` ve yukarıdaki yardımcıları kullanır.

**Kapsam ve sınırlar.**
- Açıkça yazılmış `[colors]` renkleri de eşiğe itilir (kip bir erişilebilirlik garantisidir).
  Kullanıcı orta tonlu bir yüzey yazarsa (ör. `#808080`) yüzeylere karşı garanti en iyi
  çabadır (uç ton döner); çiftler yine ≥ 4.5'tir.
- Garanti bileşen karışımlarını da kapsar (yukarıda "Bileşen karışımları"). Kapsam dışı (bilinçli):
  dekoratif yer tutucular (yüklenmemiş ikon `icon_placeholder`, tema önizlemesi iskeleti),
  sparkline alan dolgusu (veri çizgidir), hafta görünümünde "bugün" sütun tonu, `ConfirmButton`
  geri sayım izi (bilgi süpürmededir, iz zemindir), geçiş animasyonlarının ara kareleri
  (`with_alpha` solmaları, renk geçişleri), hafta görünümündeki etkinlik şeridi (takvim rengi;
  tanıtıcıdır, bilginin tek taşıyıcısı değil — gündem şeridi ve ay noktaları ise ≥ 3).
  Rastgele renkli bir örneğe bitişik odak halkası (renk seçici örneği) ve belirli kip `Spinner`'ın
  dolu/boş sınırı (boşluksuz) en iyi çabadır; değer göstergeleri için `Gauge`/`Progress` boşluklu
  çizilir.
- Terminal ANSI renkleri (`term1…14`) eşiğe tabi değildir; 0/7/8/15 yüksek karşıtlıklı
  yüzey/metin rollerinden gelir.

**Testler.** `nuxut-palette/tests/palette.rs`:
`high_contrast_meets_wcag_aa_everywhere` — 97 kaynak (uçlar + LCG ile 80 rastgele) × 9 varyant ×
2 kip × MCU kontrast {-1, 0, 1} = 5238 şema, yukarıdaki her çift ve idempotentlik;
`high_contrast_keeps_dynamic_identity` — yüzeyler aynı, vurguların hue kayması < 8° (renkli
olanlarda), `generate_with(Standard) == generate`; `theme_layer_high_contrast_on_generated_and_static_palettes`
— 11 rollük tema paleti (statik koyu/açık + her kaynak × varyant × kip). `nuxut-theme`:
`high_contrast_mode_from_config` (şema, uyarı, kenar, açık renk, `same_visuals`), `contrast::tests`.
`nuxut-ui/src/contrast_tests.rs` (bileşen karışımları): 673 yüksek karşıtlık teması (statik koyu/açık,
36 kaynak × 9 varyant × 2 kip, palet garantisi tutan rastgele "kötü" paletler) × bileşenlerin renk
seçen saf fonksiyonları (`variant_colors`, `ChipColors`, `ToggleColors`, `SliderColors`, `DayColors`,
`tile_colors`, `agenda::palette`, `week::event_fill`) ve satır içi karışımlarla aynı argümanlı
yardımcılar; her ön plan çizildiği birleşik zemine (hover/basılı 0, %8, %12 dahil) 8 bit
ölçülür; standart kipte yardımcıların eski ifadeyle aynılığı.

**Shell / ayarlar nasıl açar.**
- Ayarlar: `framework.toml` `[colors] contrast = "high"` yazar (`"standard"` ya da anahtarı
  silmek kapatır). Varsayılan belge serileştirmesi (`FrameworkConfig::default()`) anahtarı
  `contrast = "standard"` olarak taşır. Başka bir şey gerekmez; çalışan her Nuxut süreci temayı
  canlı yeniler.
- Shell (GTK/Qt/terminal dışa aktarımı): `Theme::contrast`'ı okuyup
  `nuxut_palette::Palette::generate_with(source, variant, Mode::Dark, contrast, theme.contrast)`
  (ya da `Scheme::generate_with`) çağırır; 11 tema token'ı zaten yüksek karşıtlıklıdır ve
  üstüne yazılır. `palette.toml` standart yazılmaya devam edebilir: tema dönüşümü kendisi
  uygular (idempotent olduğundan yüksek karşıtlıklı dosya da sorun değildir).
- CLI: `nuxut-palette generate … --high-contrast`.

Shell durumu (doğrulama 2026-10-13, shell kodu okunarak): dışa aktarılan palet
`src/services/look/palette.rs` `Palette::generate_with(…, contrast)` ile (shell f33aba8) — VAR.
AÇIK: portal `org.freedesktop.appearance` `contrast` (`u`, 1 = yüksek) hâlâ sabit `0`
(`portal/src/appearance.rs:Appearance::from_theme`); `theme.contrast == High` → `1` olmalı
(shell'in işi; framework üzerinden kabuğa).


## 2. Azaltılmış hareket

Karar (2026-10-13, t124).

**Tanım.** Ayarlar → Hareket "Kapalı" = `[motion] preset = "instant"`; ana anahtar
`[motion] enabled = false` aynı sonucu verir (ön ayardan, `style`'dan ve rol ezmelerinden
bağımsız). Tek API: `Theme::reduced_motion() -> bool` (`!theme.motion.enabled`). `true` iken
tema her hareket rolünü (`spatial`, `spatial_fast`, `effects`) `Motion::INSTANT`, blob esnemesini
0 (`deform_spring` anında) verir.

**Sözleşme (bileşenler).** Hareket kapalıyken her durum değişikliğinin İLK karesi hedef hâldir ve
bileşen ek kare istemez (boşta sıfır CPU). Zamanlama token'ları (`theme.timing`: ipucu beklemesi,
onay penceresi, "satırı parlat" bekleyişi) hareket değildir, sürer; beklemeler kare değil
`Ui::wake_at` ile yapılır.

**Kanıt.** `crates/nuxut-ui/src/reduced_motion_tests.rs`: değişiklikten hemen sonraki kare
`wants_frame = false` ve çizim kaydı on saniye sonraki kareyle aynı. Kapsanan: Button (state layer
+ sıkışma), IconButton, Toggle, Slider, SegmentedButton, Chip, Badge, Avatar, NavigationRail,
TextInput imleci, Progress, Gauge, Spinner (belirsiz + belirli), Menu açılış/alt sayfa
geçişi/kapanış (blob morf + esneme), Select, Popover (açılış + boyut morfu), Tooltip, Dialog,
Snackbar, OSD, PageStack push/pop, Disclosure, NotificationStack, ScrollView (tekerlek, dokunmatik
bırakma, geri sekme, ensure_visible). ConfirmButton halkası zaten saniyelik adımla `wake_at`
kullanıyordu (`confirm::reduced_motion_steps_without_frames`). Karşı kanıtlar: hareket açıkken
spinner döner, fırlatma süzülür.

**Canlı kanıt — durağan Spinner** (2026-10-13; görev). İzole
headless sway (llvmpipe, shm sunum), galeri `NUXUT_GALLERY_PAGE=2 NUXUT_GALLERY_ANIM=spinner`
(eğri önizlemeleri kapalı; sayfada yalnız belirsiz Spinner hareketli), geçici `XDG_CONFIG_HOME`'da
`framework.toml`. Açılıştan 7 sn sonra iki tam çıkış görüntüsü (5 sn arayla) ve aradaki süreç
CPU'su (`/proc/<pid>/stat` utime+stime):

| Durum | CPU (5 sn) | İki görüntü | Spinner bölgesi (140×60) |
|---|---|---|---|
| `[motion] enabled = false` açılıştan | 0 tık | bayt bayt aynı | 0 piksel fark; tonal iz + üç eşit `primary` parça |
| `enabled = true` (karşı kanıt) | 185 tık (1,85 sn) | farklı | 205 piksel fark (döner) |
| `true` ile açılıp çalışırken `false` yazıldı (`set_theme` canlı) | 0 tık | bayt bayt aynı; açılıştan `false` görüntüsüyle de aynı | 0 |

Sonuç: açık kapandı; kod değişikliği gerekmedi (birim kanıtı zaten
`reduced_motion_tests::progress_gauge_and_spinner`, `spinner_spins_when_motion_is_on`).

**Bulunan ve düzeltilen kesmeyen yerler.**

1. **Spinner (belirsiz)** dönüş periyodunu `motion.spatial`'dan alıyordu; Kapalı'da rol anında
   olduğundan 0.4 s yedeğine düşüp sonsuza dek dönüyor ve her kare istiyordu. Artık durağan
   alternatif çizer: tonal iz + eşit aralıklı üç `primary` parça (dönel simetrik — tek bir yay
   "%25 ilerleme" gibi okunurdu). Kare istemez; `set_theme` ile canlı geçer. Belirli kip zaten
   `spatial_fast` ile anında.
2. **ScrollView fırlatması** (dokunmatik yüzey ataleti) `FLING_TAU / speed` ile tema hareketinden
   bağımsız sürüyordu. Kapalı'da fırlatma yok: bırakınca olduğu yerde durur; kenar taşması anında
   sınıra oturur. Parmağı izlemek (lastik direnç dahil) doğrudan işlemdir, hareket sayılmaz.
3. **Blob kaplar (Menu, Popover, Select, OSD — `Grow`)** kapanışta oturmuş sıfır kalınlıklı tohumu
   bir kare daha çiziyordu; ardından kare istenmediğinden kenar yumuşatması + gölgenin soluk izi
   ekranda kalabiliyordu (Kapalı'da `close`'tan sonraki ilk kare tam bu kareydi; hareket açıkken de
   son karede aynı iz). `Grow::shape` oturmuş kapanışta `None` verir; "kapanış bitti" bir sonraki
   `settle`'da bir kez bildirilir; Menu gizli dalda da `settle_phase` çağırır.

**Bilinçli istisnalar.** CurveEditor önizlemesi (Ayarlar'da düzenlenen eğrinin kendisi; kullanıcı
açıkça ister, görünürken döngüsel) ve ThemePreview animasyonlu imleç önizlemesi (yalnız işaretçi
üstündeyken; gösterilen şey içeriktir — imleç temasının kendisi). Ayarlar uygulaması Kapalı'da
eğri düzenleyicisini gizleyebilir (karar).

**Kural (yeni kod).** Hareket yalnızca tema rolleriyle kurulur (`Animated::new(v, theme.motion.*)`,
`theme.blob.*`). Sabit `Spring::new`/süre ya da `now − başlangıç` ile sürekli döngü yazan bileşen
`theme.reduced_motion()`a bakar ve `reduced_motion_tests.rs`'e bir durum ekler.

**Portal / GTK tutarlılığı.** Kaynak: framework.toml → `Theme::reduced_motion()`. Framework
portal OKUMAZ: Nuxut oturumunda portal arka ucu bizim (shell `xdg-desktop-portal-nuxut`) ve
değeri zaten framework.toml'dan türetir — okumak döngü olurdu. Yabancı masaüstünde (GNOME)
sistem tercihini izlemek v1 dışı. Dışa ilan (shell'in işi, sözleşme):

| Hedef | Değer | Durum |
|---|---|---|
| portal `org.freedesktop.appearance` `reduced-motion` (`u`) | `reduced_motion() as u32` | VAR (shell `portal/src/appearance.rs:Appearance::from_theme`, `u32::from(!theme.motion.enabled)`; aynı ifade) |
| dconf `org.gnome.desktop.interface enable-animations` (`b`) | `!reduced_motion()` | VAR (shell f33aba8: `src/services/look/gtk.rs:dconf` ← `plan.rs:build` `animations = !input.reduced_motion`, `LookInput::reduced_motion = theme.reduced_motion()` `look/mod.rs`) |
| `settings.ini [Settings] gtk-enable-animations` (GTK 3/4) | `!reduced_motion()` → `0`/`1` | VAR (shell f33aba8: `src/services/look/gtk.rs:settings_ini`, `plan.rs:build` hem `gtk-3.0` hem `gtk-4.0/settings.ini`'ye; kullanıcının önceki değeri ezilir, snapshot'la geri alınır) |

Shell'de üçü (portal hariç) ayrı `look.motion` ayarıdır (ayrı snapshot; tek başına geri
alınabilir; shell ADR-0004 §18). Doğrulama (2026-10-13): shell kodu okundu; testler
`look::plan::tests::reduced_motion_writes_both_keys`, `look::tests::reduced_motion_applies_and_restores`,
`look::gtk::tests` (`enable-animations` `false`, `gtk-enable-animations` `0`/`1`).

İkisi ayrı anlamdadır: `reduced-motion` bir TERCİH (GTK 4.24 `gtk-interface-reduced-motion` ve CSS
`@media (prefers-reduced-motion)`; ikilide `org.freedesktop.appearance` ad alanı mevcut, makinede
gtk4 4.24.1 — kod yolu GTK kaynağından ayrıca doğrulanmadı), `enable-animations` ise GTK'nın
kendi geçişlerini tamamen keser. Nuxut'un "Kapalı"sı "her şey anında" olduğundan ikisi birlikte
ilan edilir; "Sakin/Canlı/Çevik"te `reduced-motion = 0`, `enable-animations = true`. Qt'de
standart bir anahtar yok (KDE `kdeglobals [KDE] AnimationDurationFactor = 0` Plasma stiline
özgü; v1 dışı, bilinen sınır).


## 3. Metin ölçeği ve GTK/Qt aktarımı

Karar (2026-10-13, t124). Framework yalnız hesap ve sözleşme
verir; yazımı shell look yapar (`nuxut_palette::look` ile aynı desen: saf, deterministik).

**Girdi.** Framework metin ölçeği `[typography] size` (mantıksal px, 4..=128) ve rol çarpanları
(`[typography.<rol>] scale`). Ölçek `f = size / 13` (`TEXT_SIZE_REFERENCE`, varsayılan boy);
`Typography::text_scale()`. Ayarlar'daki "Metin boyutu" bu oranı gösterir.

**API** (`nuxut_palette::text_scale`, saf):

- `TextScale::of(&Typography)` / `from_size(px)`; `factor()` = `round(size / 13, 0.01)`, sonra
  `[0.5, 3.0]` (gsettings aralığı); sonlu olmayan girdi 1.0.
- `gtk_text_scaling_factor() -> f64` = `f`; `gtk_xft_dpi() -> i32` = `round(96 · 1024 · f)`
  (`f = 1` → 98304).
- `qt_font_dpi(QtFonts) -> Option<u32>` = `round(96 · f)`; yalnız `QtFonts::NotWritten` ve `f ≠ 1`.
- `gtk_font_pt(role_scale)` = `round½(13 · role_scale · 0.75)` (REFERANS boy);
  `qt_font_pt(size, role_scale)` = `round½(size · role_scale · 0.75)` (ölçek katılmış). `round½` =
  0.5 pt adımı; px 4..200'e kırpılır (shell `look::fonts::px_to_pt` ile aynı).
- Toplu: `gtk(&Typography) -> GtkText { text_scaling_factor, xft_dpi, ui_pt, document_pt, mono_pt }`,
  `qt(&Typography, QtFonts) -> QtText { ui_pt, mono_pt, font_dpi }`.

**Model — çift ölçeklemeyi önleme kuralı.** Ölçek her hedefe TAM BİR kez girer:

| Hedef | Ölçeğin girdiği yer | Yazı tipi boyu | Canlı mı |
|---|---|---|---|
| GTK 3/4 (dconf) | `org.gnome.desktop.interface text-scaling-factor = f` | `font-name`/`document-font-name`/`monospace-font-name` REFERANS boyda (`gtk_font_pt`) | evet (GSettings/portal) |
| GTK (settings.ini) | `gtk-xft-dpi = round(96·1024·f)` | `gtk-font-name` REFERANS boyda | yeniden başlatma |
| Qt, kdeglobals yolu (`kde`) | QFont pt boyu (`qt_font_pt`) | ölçek katılmış | evet (KDE platform teması) |
| Qt, qt6ct yolu | `QT_FONT_DPI = round(96·f)` (oturum ortamı; `f = 1` → yazılmaz/kaldırılır) | yazılmaz (ADR-0004 §17-4) | yeniden başlatma |

Gerekçe: GTK `text-scaling-factor`'ı Xft DPI'a çevirir ve sistem fontunu kullanmayan metni de
(Firefox, Chromium/Electron, sabit pt'li CSS) aynı oranda büyütür — GNOME "Büyük metin" modeli;
bu yüzden GTK adına ölçek KATILMAZ, yoksa iki kez çarpılır. GSettings okunabilen GTK'da
`settings.ini gtk-xft-dpi` yerine `text-scaling-factor`'dan türetilen değer geçer (toplanmaz);
ikisi aynı `f`'ten yazıldığı için tutarlıdır. Qt'de canlı bir ölçek anahtarı yok; kdeglobals
yolunda ölçek pt'ye katılır ve `QT_FONT_DPI` AYARLANMAZ. `QT_SCALE_FACTOR` hiç kullanılmaz (metni
değil tüm arayüzü ölçekler).

**Yuvarlama.** `f` 0.01 adım (dconf'a kayan nokta gürültüsü yazılmaz; etkin boy hatası < yarım pt);
pt 0.5 adım (GTK/KDE seçicilerinin adımı). Örnek `size = 16`: `f = 1.23`, `gtk-xft-dpi = 120914`,
GTK `font-name` 10 pt (× 1.23 = 12.3 pt etkin), Qt `font` 12 pt, qt6ct `QT_FONT_DPI = 118`.
Varsayılan `size = 13`: `f = 1`, GTK/Qt 10 pt, `QT_FONT_DPI` yok.

**Sınırlar.** `f` 3.0'ı aşarsa (size > 39 px) GTK'da kırpılır; Qt pt'si kırpılmaz. Xwayland
X11 istemcileri (`Xft.dpi`, Xresources) v1 dışı. Rol çarpanları ölçeğe girmez, yalnız rolün pt
boyuna.

**Shell'e geçiş — tamamlandı** (shell f33aba8; doğrulama 2026-10-13,
shell kodu okunarak). GTK adı REFERANS boyda: `src/services/look/fonts.rs:pango`
→ `text_scale::gtk_font_pt(role.scale)` (eski ölçekli ad göçü testli). Ölçek tek giriş:
`fonts.rs:fonts` → `text_scale::gtk(t)` → dconf `text-scaling-factor` (`gtk.rs:dconf`, `{:.2}`) +
`settings.ini gtk-xft-dpi` (`gtk.rs:settings_ini`). Qt kdeglobals yolu `fonts.rs:qfont` →
`text_scale::qt_font_pt` (değişmedi). qt6ct yolu `fonts.rs:qt_font_dpi` → `TextScale::qt_font_dpi
(QtFonts::NotWritten)` → `env.rs:variables` (environment.d + oturum ortamı); `None` iken
`plan.rs:build` `LiveAction::SessionUnset(["QT_FONT_DPI"])`. `QT_SCALE_FACTOR` yazılmaz; kullanıcının
çakışan `QT_FONT_DPI`'ı `env.rs:scan_session` ile bildirilir. Testler: `look::plan::tests`
(`text-scaling-factor 1.23`, `gtk-xft-dpi`, `QT_FONT_DPI=118` yaz/kaldır), `look::env::tests`,
`look::fonts::tests`.


## 4. Ekran okuyucu (AT-SPI) — ön araştırma

Durum: ön araştırma 2026-10-13; **uygulandı 2026-10-15 (t127, ADR-0011)** — framework MVP: `nuxut-access`, `nuxut-wayland` `a11y`, bileşen eşlemesi, uçtan uca AT-SPI testi. Aşağıdaki plan tarihsel; güncel sözleşme ARCHITECTURE "Erişilebilirlik".
Ön araştırma ölçümleri atılabilir bir deneme projesiyle (`akprobe`) yapıldı.

### 4.1 Bağlam: Linux'ta ekran okuyucu nasıl çalışır

- Uygulama erişilebilirlik ağacını **AT-SPI2** üzerinden yayınlar: oturum bus'ında
  `org.a11y.Bus` (at-spi-bus-launcher) ayrı bir **a11y bus'ı** verir; uygulama oraya bağlanır,
  `org.a11y.atspi.Registry`'ye (at-spi2-registryd) kaydolur, her düğümü bir D-Bus nesnesi
  (`Accessible`, `Component`, `Action`, `Text`, `Value`, `Selection`…) olarak sunar ve
  değişiklikleri sinyal (`object:state-changed:focused`, `object:text-changed`,
  `object:announcement`, `window:activate`…) olarak yayar.
- Ekran okuyucu (Orca) bu olayları dinler; odak değişince adı/rolü/durumu okur.
- Etkinleştirme: `org.a11y.Status.IsEnabled` (oturum bus'ı) `true` olunca uygulamalar ağaç
  yayınlamaya başlar; kapalıyken ağaç üretmek gerekmez.
- Bu makinede: at-spi2-core 2.62.0, Orca 51.0 kurulu (Arch).

### 4.2 Seçenekler

| | **A. AccessKit** (`accesskit` + `accesskit_unix`) | **B. `atspi` crate'i + zbus** | **C. Kendi zbus uygulamamız** |
|---|---|---|---|
| Ne yapar | Platformdan bağımsız ağaç modeli (`TreeUpdate`, `Node`, `Role`, `Action`); `accesskit_consumer` ağacı tutar ve farkı hesaplar; `accesskit_atspi_common` + `accesskit_unix` AT-SPI sunucu tarafını (arayüzler, olaylar, önbellek, kayıt) uygular | Odilia projesinin saf Rust AT-SPI **protokol tipleri ve proxy'leri** (istemci tarafı ağırlıklı: ekran okuyucu yazmak için). Sunucu arayüzlerini yine biz yazarız | AT-SPI arayüzlerini doğrudan `zbus::interface` ile yazmak |
| Sürüm (2026-10) | accesskit 0.25.1, accesskit_unix 0.24.0, accesskit_atspi_common 0.21.0, accesskit_consumer 0.39.1 | atspi 0.30.0 (accesskit_unix 0.24 kendi içinde atspi 0.29 kullanır) | — |
| Bakım | Aktif; AccessKit kuruluşu, egui/Slint/Bevy/xilem kullanıyor. Sürümler ~ayda bir (unix 0.22.0 2026-06-12 → 0.22.1 07-14 → 0.23.0 08-29 → 0.24.0 09-25); 0.x, her küçük sürüm kırıcı olabilir | Aktif, daha yavaş (0.29 2025-09 → 0.30 2026-05) | Biz |
| Kod hacmi (bizim yazmamız gereken) | Yalnız köprü: bileşen → `Node` eşlemesi, runtime bağlantısı | AT-SPI sunucu tarafının tamamı (≈ accesskit_atspi_common + unix + consumer'ın işi; aşağıda ~17 bin satır) | B ile aynı, artı protokol tipleri |
| Lisans | MIT/Apache-2.0 | MIT/Apache-2.0 | — |
| Yürütücü | Kendi arka plan thread'i + `async-executor`/`async-io` (zbus 5 varsayılanı; tokio özelliği kapalı kalır) | zbus 5 | zbus 5 |

Referans hacim (kaynak satırı, `src/` toplamı): accesskit 4 564, accesskit_consumer 9 871,
accesskit_atspi_common 4 417, accesskit_unix 2 760. B ya da C, AT-SPI'nin olay sıralaması,
önbellek arayüzü (`Cache`), metin arayüzü (karakter ofsetleri, kelime/satır sınırı), etkin
torun (active-descendant) ve Orca'nın beklediği çok sayıda ayrıntı için bu ~17 bin satırlık
işin büyük kısmını yeniden yazmak demektir — yararı yok.

#### Ölçüm (deneme projesi, release + strip, rustc 1.95)

Pencere + düğme + etiketten oluşan 3 düğümlü ağaç; yalıtılmış `dbus-run-session`
(`GSETTINGS_BACKEND=memory`, geçici `XDG_CONFIG_HOME`/`DCONF_PROFILE`; kullanıcının dconf'una ve
a11y bus'ına dokunulmadı), registryd elle başlatıldı (yalıtılmış bus'ta systemd etkinleştirmesi yok).

| Durum | 10 sn CPU tıkı | RSS | Thread |
|---|---|---|---|
| Adaptörsüz taban | 0 | 2,6 MB | 1 |
| Adaptör, a11y kapalı (`IsEnabled=false`) | 0 | 5,6 MB | 4 |
| Adaptör, a11y açık, istemci yok | 0 | 5,9 MB | 4 |

- **Boşta sıfır CPU** üç durumda da (değişmez 1 korunuyor). Bellek: +~3 MB RSS (zbus bağlantısı,
  yürütücü, async-io reaktörü), +3 thread.
- İkili: yalnız zbus bağlayan karşılaştırma ikilisi 2,4 MB, accesskit_unix'li 4,2 MB → **+~1,7 MB**
  (strip'li). Shell zaten zbus 5 + async-io/async-executor bağlıyor; shell için ek derleme
  yalnız accesskit*, atspi*, zbus-lockstep(+macros), zbus_xml, phf (proc-macro), uuid,
  static_assertions. Framework'te zbus normal bağımlılık değil: yeni ağır bağımlılık olur → özellik
  bayrağı arkasında (bkz. 4.4).
- AT-SPI'den doğrulandı: registry kökünün çocuğu uygulamamız, uygulama adı ikili adı
  (`akprobe`), kökün çocuğu pencere düğümü; `request_initial_tree` yalnızca `IsEnabled` olunca çağrıldı.

### 4.3 Önerilen yol: AccessKit (A), framework'te

AccessKit'i `nuxut-wayland` (runtime: yüzey başına adaptör, odak, etkinleşme) + `nuxut-ui`
(bileşen → düğüm) üzerinden, bir cargo özelliği (`a11y`) arkasında kullanmak. Gerekçe: sunucu
tarafını doğru yapmak işin zor kısmı ve AccessKit bunu egui/Slint gibi immediate-ish araç
takımları için tam bizim modelimize uygun biçimde (uygulama ağacı **itiyor**, kütüphane
farkı ve olayları üretiyor) çözüyor; boşta maliyeti sıfır CPU, ölçülü bellek.

#### Ağaç nasıl üretilir (nuxut-ui modeli üzerinden)

Framework'te merkezi bir bileşen ağacı yok: uygulama bileşen yapılarını (`Button`, `Toggle`,
`TextInput`, `List`…) kendisi tutar, `set_rect` → `pointer`/`key` → `draw` çağırır; klavye
sırası `FocusScope` (indis tabanlı) ile. Öneri buna dokunmadan paralel bir "erişilebilirlik
geçişi":

- **API taslağı.** `nuxut-ui`'de her etkileşimli bileşene
  `fn access(&self, node: &mut AccessNode)` (rol, ad, durum, değer, sınırlar, eylemler doldurur;
  sınır = son `set_rect`). Uygulamanın yeni `App::accessibility(&mut self, cx, surface, tree:
  &mut AccessTree)` yöntemi `draw` gibi çağrılır ama **yalnızca a11y etkin ve yüzey kirliyse**
  (`cx.a11y_invalidate(surface)`; durum değişince bileşen `Response.repaint` ile zaten haber
  veriyor — kirli işareti aynı yerden). Kapalıyken hiçbir şey kurulmaz.
- **Düğüm kimliği kararlılığı.** AccessKit `NodeId(u64)`. İndis kullanılmaz (liste
  yeniden sıralanınca Orca odak/olay karışır). Kimlik = kararlı anahtar yolunun karması:
  `tree.child(key)` (`"bar"/"clock"`, `"launcher"/"result"/<desktop-id>`). Liste öğeleri
  uygulamanın zaten tuttuğu kimlikten (masaüstü kimliği, bildirim id'si, ağ SSID'si) türetilir.
  Kök pencere düğümü yüzey başına sabit. Hata ayıklamada çakışma `debug_assert`.
- **Tam ağaç, fark kütüphanede.** Her güncellemede yüzeyin tüm düğümleri gönderilir;
  `accesskit_consumer` önceki ağaçla karşılaştırıp yalnız gerçek değişiklik için AT-SPI olayı
  üretir. Bar/başlatıcı ölçeğinde (onlarca–birkaç yüz düğüm) ucuz; büyük listelerde
  (bildirim geçmişi, uzun ayar sayfası) yalnız kaydırma görünümündeki + odaktaki öğeler (Qt/GTK
  da sanal listede böyle yapar).
- **Rol / ad / durum eşlemesi** (ilk küme): Button → `Button` (simge düğmesi için **zorunlu**
  erişilebilir ad), Toggle → `Switch` + `toggled`, Checkbox → `CheckBox`, Slider → `Slider` +
  `numeric_value/min/max/step`, TextInput → `TextInput` (parola: `PasswordInput`), Select →
  `ComboBox` + `expanded`, Menu → `Menu`/`MenuItem`, List/ResultItem → `ListBox`/`ListBoxOption`
  + `selected`, Segmented → `RadioGroup`/`RadioButton`, Disclosure → `expanded`, Label → `Label`,
  Bar → `Toolbar`, Notification → `Alert`/`Status`, ProgressBar → `ProgressIndicator`.
  Devre dışı → `disabled`; açıklama (ipucu metni) → `description`; kısayol → `keyboard_shortcut`.
- **Odak.** `FocusScope::focused()` → `TreeUpdate.focus`. Yüzey odağı runtime'ın
  `keyboard_focus(surface, bool)` olayından `Adapter::update_window_focus_state` ile; Orca yalnız
  etkin pencerenin odak olaylarını okur.
- **Eylemler.** `ActionHandler` AccessKit thread'inden çağrılır → runtime bunu calloop kanalıyla
  ana thread'e taşır → `App::accessibility_action(cx, surface, node_id, action)`; uygulama
  karşılık gelen bileşen çağrısını yapar (Click → düğmenin etkinleşmesi, Focus →
  `FocusScope::focus`, SetValue/Increment → slider). Bileşenler için ortak yardımcı
  (`Button::access_action(&ActionRequest) -> Response<…>`) bunu bir satıra indirir.
- **Etkinleşme akışı.** `ActivationHandler::request_initial_tree` da başka thread'den gelir:
  `None` döner (Pending), runtime ana thread'e mesaj atar, uygulama ağacı kurar,
  `update_if_active` ile verir. Devre dışı kalınca (`deactivate_accessibility`) üretim durur.
- **Canlı bölge / bildirim.** `Live::Polite` düğümün adı değişince AccessKit `object:announcement`
  yayar (Orca 51 okur). OSD (ses/parlaklık), snackbar, bildirim açılır kartı, saat-dışı durum
  değişimleri (pil azaldı) polite; kilit ekranı hata iletisi ("parola yanlış") assertive.
- **i18n.** Ad/açıklama her zaman `t!()` ile (görünen metinle aynı anahtar ya da `a11y-*`
  anahtarı; simge düğmeleri için ayrı anahtar). `locale_changed` → tüm yüzeyler kirli → ağaç
  yeniden; ad değişimleri olay olarak gider. AccessKit `Accessible.Locale` özelliğine boş dizgi
  döndürüyor (accesskit_unix 0.24 kaynağı): Orca ses dilini sistem/Orca ayarından alır,
  düğüm başına dil (`set_language`) AT-SPI'ye taşınmıyor — bilinen sınır.

#### Yüzeyler: layer-shell, pencere, kilit

- **Yüzey başına bir adaptör** (`accesskit_unix::Adapter`): her layer yüzeyi AT-SPI'de
  uygulamanın altında bir `Frame` olur (bar her çıkışta bir tane, panel, başlatıcı, OSD,
  bildirim kartları). Tek süreç → tek AT-SPI uygulaması (ad: ikili adı, ör. `nuxut-shell`;
  toolkit adı `Tree.toolkit_name = "nuxut"`).
- **Odak almayan yüzeyler** (bar: `KeyboardMode::None`, OSD, bildirim): hiçbir zaman etkin
  pencere olmaz; Orca kendiliğinden okumaz. Erişim yolu: (1) shell bir "bara odaklan" kısayolu
  (bar'ı geçici `OnDemand`/`Exclusive` yapar, FocusScope ile gezilir — klavye erişilebilirliği
  için zaten gerekli), (2) OSD/bildirim için canlı bölge. Orca'nın nesne gezintisi (flat review)
  odaksız pencerelere ulaşabilir ama Hyprland'de Orca tuşları sorunlu (aşağıda).
- **xdg_toplevel** (Ayarlar uygulaması): olağan durum; pencere odağı `keyboard_focus`'tan.
- **Kilit ekranı** (ayrı süreç, ext-session-lock): kendi adaptörü; odak açılışta parola alanında,
  hata iletisi assertive canlı bölge.

#### Wayland'de konum/koordinat

- AT-SPI `Component.GetExtents(SCREEN)` mutlak ekran koordinatı ister; Wayland'de istemci
  pencere konumunu bilemez (AccessKit `set_root_window_bounds` belgesi: "yalnız X11'de anlamlı").
  AccessKit `WINDOW` koordinatını doğru, `SCREEN`'i kök sınırları + yerel konum olarak verir.
- **Layer yüzeylerinde biz biliyoruz:** çıkışın mantıksal konumu (`OutputInfo.position`) + anchor
  + margin + yüzey boyutu → runtime `set_root_window_bounds`'u kendisi hesaplayabilir (bar, panel,
  başlatıcı için doğru ekran koordinatı). Pencerelerde bilinmez (Hyprland IPC `clients` konumu
  verir ama bu shell'e/Hyprland'e özgü; v1'de pencerede kök = (0,0)).
- Etkisi: Orca'nın fare incelemesi (mouse review) ve büyütecin odak izlemesi koordinata dayanır;
  konuşma/odak okuma dayanmaz. MVP odak + konuşma odaklı olduğundan engel değil.

#### Hyprland'de Orca: bilinen sınırlar (doğrulanması gerekiyor)

- **Orca tuşları.** Orca kendi komutları (Orca değiştiricisi + tuş: flat review, "neredeyim",
  ayarlar) için tüm tuş olaylarını görmek ister. libatspi 2.62 bunu Wayland'de
  `org.freedesktop.a11y.Manager` / `org.freedesktop.a11y.KeyboardMonitor` arayüzüyle yapıyor
  (makinedeki libatspi'de dizgiler mevcut; GNOME'da Mutter sağlıyor). **Hyprland bu arayüzü
  sağlamıyor**; libatspi "legacy" aygıta düşer, o da Wayland-yerli istemcilerde araç takımının
  tuşları AT-SPI'ye iletmesine dayanır — AccessKit (ve GTK4) iletmez. Sonuç: Orca odak
  değişimlerini ve canlı bölgeleri **okur**, ama Orca'ya özgü komutlar Hyprland'de büyük
  olasılıkla çalışmaz. Bu shell'e değil masaüstüne ait bir boşluk; çözüm adayları açık sorularda.
- **Ekran okuyucuyu açma/kapama** (`IsEnabled`, Orca başlatma) ayarlar + kısayolla shell'in işi;
  Orca'nın kendisi oturumda başlatılmalı (systemd kullanıcı birimi ya da exec-once).
- `IsEnabled` gerçek oturumda at-spi-bus-launcher'ın gsettings anahtarına
  (`org.gnome.desktop.interface toolkit-accessibility`) bağlı; test her zaman yalıtılmış bus +
  `GSETTINGS_BACKEND=memory` ile (kullanıcının dconf'una yazılmaz).

#### Güvenlik ve hassas içerik

- **a11y bus'ının erişim denetimi yok:** aynı kullanıcının her süreci `IsEnabled`'ı açıp ağacı
  okuyabilir ve eylem çağırabilir. Yani ağaçta görünen her şey (bildirim metni, pano geçmişi
  önizlemesi, Wi-Fi adları) aynı-uid süreçlere açıktır — ekranda zaten görünenden fazlası
  ağaca **konmaz** (gizlenen/katlanmış içerik, görünmeyen sayfa, `redact` edilen alanlar yok).
- **Parola alanı:** rol `PasswordInput` (AT-SPI `PasswordText`); değer/metin olarak **yalnız
  maske** (`•` × uzunluk — görsel maskeyle aynı bilgi, uzunluk dışında hiçbir şey) ya da hiç
  metin; gerçek metin `SecretBuffer`'dan hiçbir zaman çıkmaz (`with_secret` a11y yolunda
  kullanılmaz). Yazma olayları maskeli metinle. Orca bunu "yıldız/nokta" diye okur (GTK davranışı).
- **Eylem kısıtı:** parola alanında AT'den gelen `SetValue`/`ReplaceSelectedText` reddedilir
  (aynı-uid bir sürecin kilit ekranına D-Bus'tan parola denemesi yazması engellensin; PAM
  gecikmesi zaten var ama yol açılmasın). Kilit ekranında yalnız `Focus` ve `Click` (kullanıcı
  değiştir/güç gibi zaten yetkisiz yapılabilen) kabul edilir.
- **Kilit ekranı:** kilitliyken yalnız kilit sürecinin ağacı anlamlıdır; shell'in diğer yüzeyleri
  (bildirim içeriği!) kilitliyken ağaçtan çekilir ya da `hidden` olur (bildirim gizliliği
  ayarıyla aynı kural). Loglar: değişmez 9 — ağaç içeriği (ad/değer) hiçbir log seviyesinde
  yazılmaz; `AccessNode` `Debug`'ı `redact` kalıbını izler.
- **Çökme riski:** accesskit_unix arka plan thread'i `InterfaceExists`/`MissingParameter`
  zbus hatalarında `panic!` ediyor ve bizim profillerimiz `panic = "abort"` → tüm shell düşer.
  Bu hatalar programcı hatası (aynı nesneyi iki kez kaydetme) olsa da kabul kriteri: kilit
  sürecinde a11y açılmadan önce bu yolun testi; gerekirse kilit sürecinde özellik kapalı
  derlenir ya da üst akışa düzeltme gönderilir.

### 4.4 Aşamalı plan ve iş tahmini

Tahminler tek işçi, odaklı gün; inceleme/iniş hariç.

| Faz | İş | Sahibi | Tahmin |
|---|---|---|---|
| 0 | `nuxut-wayland` `a11y` özelliği (varsayılan kapalı; shell açar): yüzey başına adaptör yaşam döngüsü, etkinleşme/eylem için calloop kanalı, `keyboard_focus` → pencere odağı, layer yüzeylerinde kök sınırları, `App::accessibility`/`accessibility_action`, `cx.a11y_invalidate`, `cx.a11y_active()`. DEPENDENCIES + ARCHITECTURE sözleşmesi | framework | 3–4 g |
| 1 | `nuxut-ui`: `AccessNode`/`AccessTree` (kararlı kimlik), MVP bileşenlerinin `access` + eylem yardımcıları: Button/IconButton, Toggle, Slider, TextInput (+parola), Select, Menu, List/ResultItem, Segmented, Label, FocusScope bağlantısı. Birim test: `accesskit_consumer::Tree` üzerinde rol/ad/durum/odak doğrulaması (D-Bus'sız) + simge düğmesinin adsız kalamayacağı testi | framework | 5–7 g |
| 2 | Uçtan uca test altyapısı: `dbus-run-session` + bus launcher + registryd + `atspi` crate'i (yalnız dev-dependency) ile Rust entegrasyon testi — ağacı AT-SPI'den okuyup karşılaştırır, olay sırasını doğrular. Orca + speech-dispatcher (dummy çıktı) ile elle duman testi betiği, yalıtılmış sway'de | framework | 2–3 g |
| 3 (MVP) | Shell: bar (çalışma alanları, saat, tepsi, durum simgeleri → `Toolbar` + düğmeler; "bara odaklan" kısayolu), başlatıcı (arama alanı + sonuç listesi, seçili öğe odak/etkin torun, sonuç sayısı polite duyuru) | shell | 4–5 g |
| 3 (MVP) | Ayarlar formu (sayfa gezinmesi, form satırları: etiket ↔ denetim ilişkisi `labelled_by`) | settings | 3–4 g |
| 4 | Panel/popout'lar, bildirimler + OSD canlı bölgeleri, kilit ekranı (güvenlik kuralları, ayrı review), polkit diyaloğu | shell/framework | 5–7 g |
| 5 | Konum: pencerelerde Hyprland IPC'den kök sınırı (isteğe bağlı), büyüteç odak izleme; Orca tuşları sorunu (açık soru 3) | sonra | ? |

MVP toplamı (faz 0–3): **~17–23 iş günü** (≈ 3,5–4,5 hafta tek işçi; framework ve shell
fazları kısmen paralel).

### 4.5 Riskler

1. **Hyprland'de Orca tuş komutları** (KeyboardMonitor yok): kullanıcı deneyimini en çok
   sınırlayan konu, bizim kodumuzla tam çözülemez. Odak + konuşma çalışır; Orca komutları
   çalışmayabilir. Gerçek Orca ile doğrulanmalı (faz 2).
2. **AccessKit 0.x kırıcı sürümleri** (~ayda bir): tek köprü modülünde (`nuxut-ui::access` +
   runtime) tutularak yükseltme maliyeti sınırlanır; sürüm sabitlenir, toplu yükseltilir.
3. **Abort profilinde arka plan panic'i** (4.3 Güvenlik).
4. **Bellek/ikili:** +~3 MB RSS, +3 thread, +~1,7 MB ikili (shell'de zbus zaten var). Özellik
   kapalı derlenen uygulamalar (duvar kağıdı aracı vb.) etkilenmez. a11y kapalıyken de adaptör
   oturum bus'ına bağlanıp `IsEnabled`'ı dinler (thread + bağlantı açık, CPU 0); istenirse
   adaptörü yalnızca `IsEnabled` olunca kuran ince bir bekçi yazılabilir (açık soru 4).
5. **Kararsız kimlik / eksik ad** — test ve lint ile: simge düğmesi adsız olamaz, liste öğesi
   indisle kimlik alamaz.
6. **Kapsam kayması:** her bileşene a11y eklemek tüm bileşen API'sine dokunur; MVP bileşen
   listesiyle sınırlı tutulmalı.

### 4.6 Açık sorular (karar)

1. **Yol onayı:** AccessKit (A) framework'te, `a11y` cargo özelliği arkasında — kabul mü?
   (B/C önerilmiyor: ~17 bin satır sunucu tarafını yeniden yazmak.)
2. **Öncelik/zamanlama:** MVP (bar + başlatıcı + ayarlar formu, ~3,5–4,5 hafta) hangi
   kilometre taşında? Kilit ekranı MVP'ye girsin mi (güvenlik review'u ister)?
3. **Orca tuşları (Hyprland):** (a) sınırı belgeleyip bırakmak, (b) shell'in
   `org.freedesktop.a11y.KeyboardMonitor`'ün `SetKeyGrabs` kısmını Hyprland bind'leriyle taklit
   etmesini araştırmak (tüm tuşları izleme — `WatchKeyboard` — compositor olmadan mümkün değil),
   (c) Hyprland üst akışına/eklentisine arayüz önermek. Öneri: (a) şimdi, (b) faz 5'te araştırma.
4. **a11y kapalıyken maliyet:** adaptör hep kurulu (+3 MB, 0 CPU) mı, yoksa `IsEnabled` bekçisi
   ile tembel mi? Öneri: önce hep kurulu (basit), ölçüm sorun gösterirse bekçi.
5. **Ekran okuyucuyu açma yeri:** Ayarlar → Erişilebilirlik anahtarı + kısayol (Super+Alt+S,
   GNOME ile aynı) Orca'yı başlatsın mı; Orca bağımlılığı paket "önerilen"i mi?
6. **Parola alanı metni:** maske (`•` × uzunluk, GTK ile aynı) mı, hiç metin vermemek mi
   (uzunluk da sızmasın, Orca yalnız "parola alanı" der)? Öneri: maske — kullanıcı yazdığını
   duyabilmeli; uzunluk zaten ekranda.

**Kararlar (2026-10-13):**

1. ONAY: AccessKit 0.25 + accesskit_unix 0.24, framework'te isteğe bağlı `a11y` özelliği.
2. MVP = bar + başlatıcı + Ayarlar. Kilit ekranı MVP'de YOK; ayrı güvenlik incelemesiyle sonra.
   Zamanlama: şimdi başlanmaz. Önce yüksek karşıtlık takibi (bileşenlerin kendi karıştırdığı
   renkler palet garantisine girer) ve küçük işler; AT-SPI MVP tam masaüstü kilometre taşının
   (m6) son işi.
3. Orca kısayollarının Hyprland'de çalışmaması bilinen sınır olarak belgelenir (seçenek a).
4. Adaptör `a11y` özelliğiyle derlenmişse hep açık (ölçüm 0 CPU). Özellik kapalı derlemede
   adaptör hiç yoktur.
5. Ayarlar'da ekran okuyucu anahtarı + Super+Alt+S. Orca install.sh'te isteğe bağlı
   (`--with-a11y`).
6. Parola alanı uzunluk maskesi verir (`•` × uzunluk). EVET.

### 4.7 Kaynaklar

- AccessKit: <https://github.com/AccessKit/accesskit> — crates.io: accesskit 0.25.1
  (2026-09-25), accesskit_unix 0.24.0 (2026-09-25; varsayılan özellik `async-io`, alternatif
  `tokio`), accesskit_atspi_common 0.21.0, accesskit_consumer 0.39.1. İncelenen kaynak:
  `accesskit_unix-0.24.0/src/{adapter,context,executor}.rs` (tek arka plan thread'i,
  `IsEnabled` dinleme, `set_root_window_bounds` Wayland notu, kurtarılamaz hatada `panic!`),
  `accesskit_atspi_common-0.21.0/src/{node,adapter}.rs` (rol eşlemesi `PasswordInput →
  PasswordText`, `Window → Frame`, canlı bölge → `Announcement`), `atspi/interfaces/accessible.rs`
  (`Locale` boş).
- atspi (Odilia): <https://github.com/odilia-app/atspi> — atspi 0.30.0 (2026-05-06).
- AT-SPI2 belirtimi ve libatspi: <https://gitlab.gnome.org/GNOME/at-spi2-core> (sistemde 2.62.0;
  `org.freedesktop.a11y.Manager`/`KeyboardMonitor` desteği libatspi'de mevcut).
- Orca: <https://gitlab.gnome.org/GNOME/orca> (sistemde 51.0).
- zbus 5: <https://github.com/dbus2/zbus> (shell'de zaten 5.16+, varsayılan async-io).
- Deneme ölçümü: 3 düğümlü ağaç, yalıtılmış `dbus-run-session`, 2026-10-13 (4.2'deki tablo).
