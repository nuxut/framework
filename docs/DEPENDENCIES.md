# Bağımlılıklar

Sahibi: framework. İlke: kod Rust'tır ama her şey sıfırdan yazılmaz;
doğru, bakımlı ve **hıza/belleğe/estetiğe zarar vermeyen** crate'ler ve sistem kütüphaneleri
(Rust binding'iyle) kullanılır. Her yeni crate ya da sistem paketi buraya **neden, alternatif,
lisans, maliyet** ile yazılır; review'da bu dosya kontrol edilir.

Denetim: `cargo deny check` (yapılandırma `deny.toml`): RustSec bildirimleri, lisans izin
listesi (Nuxut MIT; izinli lisanslar MIT ile birleşebilenler, MPL-2.0 dosya düzeyinde copyleft olarak kabul; GPL ailesi yok — caelestia/noctalia GPL, kod kopyalanmaz), yinelenen sürümler (uyarı), yalnızca
crates.io kaynağı. Son çalıştırma: hata yok; yinelenenler aşağıda.

Ölçek (2026-10-04): tüm feature'larla 236 crate (geçişli). `frame` demosu release ikilisi
~5.1 MB (thin LTO, debuginfo yok).

Bellek (release, llvmpipe, boşta): ui galerisi (1880×1040 tek yüzey, tüm bileşenler) RSS 120 MB =
anon 43.6 MB + dosya 69 MB (çoğu Mesa/llvmpipe ve paylaşılan kütüphaneler) + shm 7.6 MB;
frame demosu RSS 111–113 MB (anon ~40 MB). Boşta CPU 0. Gerçek GPU sürücüsüyle anon ve
dosya payının belirgin düşmesi beklenir (llvmpipe tamponları ve LLVM).

## Çalışma zamanı sistem paketleri (Arch)

| Paket | Ne için | Kim yükler | Zorunlu mu |
|---|---|---|---|
| `wayland` | `libwayland-client`, `libwayland-egl` | nuxut-wayland (wayland-backend `client_system`, wayland-egl) | evet |
| `libglvnd` + `mesa` | `libEGL.so.1`, GLES 3 sürücüsü | nuxut-wayland (khronos-egl, çalışma zamanında `dlopen`) | evet |
| `libxkbcommon` | klavye düzeni / keysym | smithay-client-toolkit `xkbcommon` | evet |
| `fontconfig` + fontlar | sistem fontları (fontdb dizinleri tarar; fontconfig kütüphanesi linklenmez) | nuxut-render (cosmic-text) | font gerekir |
| `ttf-material-symbols-variable-git` (AUR) | `material` ikon seti (`typography.icon`, ADR-0006) | nuxut-ui ikonları | önerilen (yoksa kurulu Nerd Font'a düşülür) |
| `inter-font` | `ui`/`display` rolü varsayılanı (Inter 4 değişken, `opsz`, `tnum`) | nuxut-ui tipografi | önerilen (yoksa sans-serif) |
| `adobe-source-serif-fonts` | `document` rolü varsayılanı (Source Serif 4) | nuxut-ui tipografi | önerilen (yoksa serif) |
| `ttf-jetbrains-mono-nerd` | `mono` rolü ve `nerd` ikon seti | nuxut-ui tipografi/ikonları | önerilen (yoksa monospace; `nerd` seti çizilmez) |
| `ca-certificates` | TLS kök sertifikaları (rustls-platform-verifier) | nuxut-net | ağ isteyen uygulamalar için evet (Arch temel sistemde) |
| `mpv` | `libmpv.so.2` — video duvar kağıdı oynatma ve içe aktarma dönüştürmesi (kodlama kipi; libx264/libx265/libvpx/svt-av1 Arch `ffmpeg` üzerinden) | nuxut-video, çalışma zamanında `dlopen` (derlemede gerekmez) | yalnızca video/animasyon duvar kağıdı; yoksa statik görsel. Lisans: Arch mpv/ffmpeg GPL özellikli; dinamik yükleme, kaynak MIT |
| `xdg-desktop-portal` + `xdg-desktop-portal-gtk` (ya da `-gnome`/`-kde`) | dosya seçici diyaloğu (FileChooser portalı; `xdg-desktop-portal-hyprland` FileChooser sunmaz) | nuxut-portal (D-Bus, oturum bus'ı) | yalnız `portal` özelliğini kullanan uygulama (Ayarlar); yoksa `PortalError::Unavailable` |

Not: VM'de 3D hızlandırma yoksa Mesa `llvmpipe`'a düşer; çalışır ama animasyon CPU'su artar.

## Rust crate'leri (doğrudan)

### Çekirdek / yapılandırma

| Crate | Sürüm | Lisans | Kullanan | Neden | Alternatif / not |
|---|---|---|---|---|---|
| serde | 1 | MIT/Apache-2.0 | core (opsiyonel), config, theme, palette | config şemaları | — (fiilî standart) |
| toml | 1 | MIT/Apache-2.0 | config, theme, palette | `framework.toml`, `palette.toml` | — |
| serde_ignored | 0.1 | MIT/Apache-2.0 | config | bilinmeyen anahtarları uyarı olarak raporlamak | elle serde visitor (gereksiz kod) |
| thiserror | 2 | MIT/Apache-2.0 | hepsi | hata tipleri (çalışma zamanı maliyeti yok) | — |
| tracing | 0.1 | MIT | hepsi | yapılandırılmış log | `log` (tracing span'leri runtime'da işe yarıyor) |
| dirs | 7 | MIT/Apache-2.0 | config, theme, render, image, palette | XDG dizinleri | `$XDG_*` elle okumak (kenar durumları) |
| inotify | 0.11 | ISC | config, i18n | config/palet dosyası ve kullanıcı çeviri dizini izleme (boşta sıfır CPU) | `notify` (çok platformlu, daha ağır; Linux'ta inotify'ı sarar) |

### Hareket / tema
Dış bağımlılık yok (`nuxut-motion` saf Rust, analitik yay + bezier).

### Render

| Crate | Sürüm | Lisans | Neden | Alternatif / not |
|---|---|---|---|---|
| glow | 0.18 | MIT/Apache-2.0/Zlib | GLES 3.0 çağrıları (ince, sıfır maliyetli sarmalayıcı) | `gl` crate'i (aynı); wgpu (ADR-0001: bellek/başlangıç maliyeti, ileride ikinci backend) |
| cosmic-text | 0.19 | MIT/Apache-2.0 | metin şekillendirme (harfrust), sarma, bidi, font fallback | kendi şekillendiricimiz (yıllar sürer); glyphon (wgpu'ya bağlı) |
| swash | 0.2 | MIT/Apache-2.0 | glyph rasterleştirme — değişken eksenler (wght + FILL + opsz) için doğrudan; cosmic-text zaten aynı sürümü çeker, kopya yok | cosmic-text'in SwashCache'i (yalnızca wght uygular) |
| sys-locale | 0.3 | MIT/Apache-2.0 | font önbelleğinin anahtarı için cosmic-text'in kendi okuduğu yerel ayar (cosmic-text dışa açmıyor); ağaçtaki aynı sürüm, kopya yok | `LANG` elle okumak (Windows/macOS'ta yanlış) |
| fontconfig-parser | 0.5 | MIT | genel aileleri (sans-serif/serif/monospace) kullanıcının `~/.config/fontconfig` tercihine ve kurulu bir aileye bağlamak (cosmic-text sabit "Open Sans" kullanıyor; kurulu değilse kalın parça başka aileye düşüyordu). fontdb zaten aynı sürümü çeker | libfontconfig (C) linklemek; `fc-match` süreci çalıştırmak |

### Wayland runtime (bilgi: `nuxut-wayland`)

| Crate | Sürüm | Lisans | Neden | Not |
|---|---|---|---|---|
| smithay-client-toolkit | 0.21 (`calloop`, `xkbcommon`) | MIT | layer-shell, xdg, seat, output, klavye eşlemi | elle wayland-client (çok kod) |
| xkbcommon | 0.8 (özelliksiz) | MIT | compositor keymap göndermezse yedek `us` keymap'i derlemek (`keyfocus.rs`); tuşların değiştiricisiz keysym'i ve adı (`keysym.rs`, `KeyEvent::xkb`); sctk zaten aynı sürümü çeker, yeni sistem paketi yok (`libxkbcommon`, `xkeyboard-config`) | gömülü keymap metni (bakımı zor, düzen verisi eskir) |
| rustix | 1 (`std`, `fs`, `mm`, `event`) | MIT/Apache-2.0 | yedek keymap'i mühürlü `memfd`'de vermek (dosya sisteminde iz yok, compositor/sctk değiştiremez); `mm`: yazılım sunumunun (llvmpipe) `wl_shm` tamponlarını memfd üzerinde `mmap` etmek; `event`: pano izleme (data-control) okumasında boruyu `poll` ile son tarihli okumak (çağıranın thread'inde, ana döngü bloklanmaz) — özellikler, yeni crate değil; wayland-backend zaten aynı sürümü çeker, yeni sistem paketi yok | `$XDG_RUNTIME_DIR`'de geçici dosya (iz/yarış riski); `libc` ile elle `unsafe` |
| calloop | 0.14 (`signals`) | MIT | olay döngüsü; SIGTERM/INT/HUP signalfd ile (`nix`'i çeker) | tokio (gereksiz, ağır) |
| calloop-wayland-source | 0.4 | MIT | Wayland kuyruğunu calloop'a bağlama | — |
| wayland-client / wayland-backend | 0.31 / 0.3 (`client_system`) | MIT | protokol; EGL için libwayland'e bağlı backend | saf Rust backend EGL ile çalışmaz |
| wayland-scanner | 0.31 | MIT | gömülü Hyprland protokol XML'lerinden (`crates/nuxut-wayland/protocols/`) istemci bağlaması üretmek: `hyprland_focus_grab_v1` (odak kapma, t078), `hyprland_ctm_control_manager_v1` (renk dönüşümü / gece ışığı, t113), `hyprland_toplevel_mapping_manager_v1` (pencere ↔ Hyprland adresi, yakalama 2026-10-14 — makinede XML yok, ağ yok: Hyprland 0.56.2 paketinin hyprwayland-scanner başlığından ve ikilideki `wl_message` tablolarından birebir yeniden kuruldu, bkz. `docs/design/capture.md`). wayland-protocols zaten aynı sürümü çeker; yeni indirme yok, yalnızca derleme zamanı makrosu | `hyprland-protocols` Arch paketinin XML'ini `build.rs` ile okumak (derlemede sistem paketi gerekir, sürüm kayması); elle yazılmış bağlama (imza hatası riski). XML kaynağı hyprland-protocols (BSD-3-Clause, telif bloğu dosyada korunur). Çalışma zamanı paketi yok: protokolü compositor (Hyprland) sunar, yoksa `focus_grab` `None` |
| wayland-egl | 0.32 | MIT | `wl_egl_window` | — |
| wayland-protocols | 0.32 (`client`, `staging`; IME ve xdg-foreign için `unstable`) | MIT | fractional-scale, viewporter, cursor-shape, text-input-v3; `staging`: ext-data-control-v1 (pano izleme, t060); `staging` ayrıca: ext-foreign-toplevel-list-v1, ext-image-capture-source-v1, ext-image-copy-capture-v1 (pencere listesi ve tek kare yakalama); `unstable`: xdg-foreign-unstable-v2 (pencere dışa aktarma, portal diyaloğunun ebeveyni; nuxut-wayland açıkça ister) | — |
| wayland-protocols-wlr | 0.3 (`client`) | MIT | `zwlr_data_control_manager_v1` (sürüm ≤ 2; 2 birincil seçim): ext-data-control-v1 sunmayan compositor'da pano izleme yedeği (t060, `data_control.rs`); `zwlr_screencopy_manager_v1` (sürüm ≤ 3): ext-image-copy-capture sunmayan compositor'da çıkış yakalama yedeği (`capture.rs`); `zwlr_foreign_toplevel_handle_v1` yalnızca gömülü eşleme XML'inin arayüz bağımlılığı. Ağaçta zaten var (smithay-client-toolkit çekiyor); yeni indirme/derleme yok. Çalışma zamanı paketi yok: protokolü compositor sunar, ikisi de yoksa `data_control_protocol() == None` | XML'i gömüp `wayland-scanner` ile üretmek (aynı bağlama, kopya bakım); `smithay_client_toolkit::reexports::protocols_wlr` (sctk sürümüne örtük bağlılık) |
| khronos-egl | 6 (`dynamic`) | MIT/Apache-2.0 | EGL; `libEGL` çalışma zamanında yüklenir (derlemede libglvnd gerekmez) | glutin (pencere soyutlaması gereksiz) |
| libloading | 0.8 | ISC | khronos-egl `dynamic` | — |

### Locale (nuxut-locale)

| Crate | Sürüm | Lisans | Kullanan | Neden | Alternatif / not |
|---|---|---|---|---|---|
| chrono | 0.4.45 (`std`, `unstable-locales`; nuxut-locale'de ek `clock`) | MIT/Apache-2.0 | nuxut-locale | tarih/saat biçimleme; shell ile aynı sürüm (özellikler birleşir). `clock`: `Locale::format_system_time` yerel saat dilimi (`chrono::Local`: `TZ` ve `/etc/localtime`, kendi tzfile ayrıştırıcısı; unsafe/alt süreç yok) | time (locale yok); elle ad tabloları (yalnızca tr/en vardı); `localtime_r` (unsafe, `forbid(unsafe_code)`); jiff (ikinci tarih kütüphanesi) |
| iana-time-zone | 0.1 (chrono `clock` geçişli) | MIT/Apache-2.0 | chrono | Linux'ta yalnızca `/etc/localtime` bağını okur, bağımlılıksız; windows-*/android ve diğer platform crate'leri Cargo.lock'ta görünür ama yalnızca o hedeflerde derlenir | — |
| pure-rust-locales | 0.8.2 (chrono'nunkiyle aynı; doğrudan bağımlılık) | MIT/Apache-2.0 | nuxut-locale | glibc LC_TIME, LC_NUMERIC (ondalık/gruplama ayırıcısı) ve LC_ADDRESS (dil/ülke adı: `language_name`, `display_name`) verisi gömülü: kurulu locale gerekmez, her platformda aynı; chrono zaten çekiyor, ek derleme yükü yok | libc `strftime`/`nl_langinfo` (unsafe, süreç geneli `setlocale`, sistemde locale üretilmiş olmalı); icu4x (çok büyük veri) |

Not: yalnızca kullanılan locale'in tablolarına dokunulur (rodata sayfaları tembel yüklenir);
yeni sistem paketi yok.

### Uluslararasılaştırma (nuxut-i18n, ADR-0010)

| Crate | Sürüm | Lisans | Kullanan | Neden | Alternatif / not |
|---|---|---|---|---|---|
| fluent-bundle | 0.16 (`concurrent` paket yolu) | MIT/Apache-2.0 | nuxut-i18n | Project Fluent referans uygulaması: çoğul/seçici, mesaj/terim referansı, hataya dayanıklı ayrıştırma (bozuk girdi atlanır), döngü ve placeable patlama sınırı (100), `set_formatter` ile `NUMBER` biçimi `nuxut-locale` ayırıcılarına bağlanır | gettext/.po (değişken adı yok, C çoğul ifadeleri); kendi biçimimiz (çoğul kurallarını yeniden yazmak) |
| fluent-syntax | 0.12 | MIT/Apache-2.0 | nuxut-i18n (`check`, kullanıcı dosyası mesaj sayımı) | AST: çevirmen yorumu, değişken kümesi, referans grafiği denetimi; fluent-bundle zaten çeker | — |
| fluent-langneg | 0.13 (fluent-bundle 0.16 ile aynı; 0.14 alınmaz, ikili sürüm olurdu) | MIT/Apache-2.0 | nuxut-i18n | istenen ↔ mevcut dil eşlemesi (`tr-TR` → `tr`, `de-AT` → `de`) | elle önek eşleme (betik/bölge kuralları eksik kalır) |
| unic-langid | 0.9 | MIT/Apache-2.0 | nuxut-i18n | BCP 47 dil kimliği, `character_direction` (RTL) | icu_locale_core (ağaçta var ama fluent unic-langid ister; iki tip dönüştürülürdü) |
| intl-memoizer | 0.5 | MIT/Apache-2.0 | nuxut-i18n | `set_formatter` imzasındaki eşzamanlı memoizer tipi; fluent-bundle zaten çeker | — |
| unicode-segmentation | 1.12 | MIT/Apache-2.0 | nuxut-i18n, nuxut-ui | `[max: N]` grafem sayımı, 4 KiB çıktı kırpmasının grafem sınırı, sözde dil uzatması; takvimin dar gün adı (ilk grafem); ağaçta zaten var (cosmic-text) | `chars().count()` (birleşik aksanı iki sayar) |

Dolaylı yeni crate'ler: `intl_pluralrules` (gömülü CLDR çoğul tablosu, küçük), `type-map`,
`unic-langid-impl`; `self_cell`, `smallvec`, `rustc-hash`, `tinystr` ağaçta zaten vardı. Hepsi
saf Rust, `unsafe` kullanıcı kodumuzda yok (`forbid(unsafe_code)`). **Çalışma zamanı paketi: yok**
(Arch'ta sistem paketi gerekmez; katalog metni ikiliye gömülü).

`nuxut-i18n-macros` (proc-macro, `embed!`): dış bağımlılık yok (yalnızca `proc_macro`);
`syn`/`quote` alınmadı — girdi iki dizge sabiti, çıktı tek ifade.

Gömülü metin bekçisinin Rust tarayıcısı (`check/scan.rs`) elle yazıldı: `syn` (tam AST, `full`
özelliği) derleme süresini ve test ikilisini büyütürdü; bekçinin ihtiyacı yalnızca yorum/dizge/
parantez/öznitelik ayrımı. Süreç geneli katalog için `arc-swap` alınmadı: okuma iş parçacığı
önbelleği + nesil sayacıyla zaten kilitsiz (`global.rs`).

### Bileşenler (nuxut-ui)

| Crate | Sürüm | Lisans | Kullanan | Neden | Alternatif / not |
|---|---|---|---|---|---|
| zeroize | 1 | MIT/Apache-2.0 | nuxut-ui (`SecretBuffer`, `TextInput` parola kipi), nuxut-render (`TextSystem::layout_secret`, `TextLayout::wipe`: gösterilen parolanın layout kopyaları), nuxut-core (`SecretText`: `KeyEvent.text`, IME metni; `XkbKey`/`NamedKeysym` Drop), nuxut-wayland (keymap'siz `RawKey` kuyruğu; data-control okuma tamponları `Zeroizing`, seçime konan paylaşılan baytlar son sahipken sıfırlanır) | sırların (kilit ekranı parolası) bellekten derleyicinin optimize edemeyeceği biçimde silinmesi; saf Rust, bağımlılıksız | elle `write_volatile` döngüsü (unsafe; nuxut-ui `forbid(unsafe_code)`) |
| memmap2 | 0.9 | MIT/Apache-2.0 | nuxut-ui (`SecretBuffer::locked`, `TextInput` parola kipi) | parola tamponu kendi sayfa hizalı anonim eşlemesinde (`map_anon`), `mlock` (`lock`/`unlock`) ve Linux'ta `MADV_DONTDUMP` (`advise(Advice::DontDump)`) — hepsi **güvenli** API, nuxut-ui `forbid(unsafe_code)` kalır. Kendi eşlemesi: başka veriyle sayfa paylaşmaz, `munlock` başka tamponu çözmez. Ağaçta zaten var (fontdb üzerinden), yeni derleme yok; saf Rust + libc. Windows/macOS'ta derlenir (kilit unix, dontdump Linux). Çalışma zamanı paketi: yok | `region` (`lock` güvenli ama tampona erişim ham işaretçi → `unsafe`; madvise yok), `memsec`/`rustix::mm` (`mlock`/`madvise`/`mmap` `unsafe fn`), `secrets` (libsodium tarzı korumalı sayfalar; C kütüphanesi, `unsafe` yüzey), `nuxut-render`'a `unsafe` modül (GL crate'ine ilgisiz sorumluluk, ARCHITECTURE istisnasını genişletir) |
| tracing (artık koşulsuz) | 0.1 | MIT | nuxut-ui | kilit/eşleme hatası uyarısı (süreç başına bir kez); önceden yalnızca `brand` özelliğiyle geliyordu, ağaçta zaten var | `eprintln!` (tüketicinin log yapılandırmasını atlar) |
| rustix (`process`, yalnızca Linux dev-dep) | 1 | Apache-2.0/MIT | nuxut-ui testleri | `secret.rs` alt süreç testinde gerçek `setrlimit(RLIMIT_MEMLOCK, 0)` (güvenli API); ağaçta zaten var | `prlimit` aracıyla dış süreç (util-linux bağımlılığı) |
| nuxut-image (iç crate, `default-features = false`) | — | — | nuxut-ui `brand` özelliği (varsayılan açık): `BrandMark` | gömülü marka SVG'sini sertleştirilmiş SVG yoluyla (`render_svg`, resvg) alfa maskesine rasterleştirmek; yeni dış crate yok (resvg/image zaten ağaçta). Çalışma zamanı paketi yok | resvg'yi doğrudan çağırmak (aynı ağaç, ikinci SVG yolu); maskeyi elle yay/çizgi tarayıcıyla çizmek (yeniden icat). Yalın tüketici `default-features = false` ile kapatır |

### Medya (sahibi: `nuxut-image`)

| Crate | Sürüm | Lisans | Kullanan | Neden | Alternatif / maliyet |
|---|---|---|---|---|---|
| image | 0.25 (`png`, `jpeg`, `webp`, `gif`; varsayılanlar ve `rayon` kapalı) | MIT/Apache-2.0 | nuxut-image | PNG/JPEG (zune-jpeg)/WebP/GIF/APNG çözme, `Limits` ile boyut/tahsis sınırı, EXIF yönü | ayrı ayrı `png`+`zune-jpeg`+`image-webp`+`gif` (aynı ağaç, daha çok yapıştırma kodu). Saf Rust; C yok |
| png | 0.18 (image'ın kullandığı sürüm; ağaçta zaten var) | MIT/Apache-2.0 | nuxut-image | Paletli PNG'nin indekslerini satır satır okuyup RGB(A)'ya açmadan en dar biçimde (gri paletli L8, saydam gri LA8…) küçültmek (`indexed.rs`; image çözücüsü paleti hep RGB8/RGBA8'e açar). Yeni crate yok, çalışma zamanı paketi yok (2026-10-16) | image'ın çıktısını sonradan daraltmak (tam boyutlu RGB tamponu yine oluşur) |
| fast_image_resize | 6 | MIT/Apache-2.0 | nuxut-image | SIMD (AVX2/SSE4.1/NEON) Lanczos3 küçültme; premultiplied veride doğrudan, opak JPEG'de RGB üzerinde (tepe bellek düşük) | `image::imageops::resize` ~5–10× yavaş |
| resvg | 0.48 (`raster-images` açık: yalnızca boyutu başlıktan denetlenen gömülü data: PNG/JPEG/GIF/WebP; metin, sistem fontu, dış dosya yok) | Apache-2.0/MIT | nuxut-image | SVG ikonlar; tiny-skia premultiplied RGBA verir (renderer biçimi). `raster-images`, image ile aynı gif/image-webp/zune-jpeg sürümlerini kullanır; yeni crate yok | librsvg (C, cairo/glib ağacı) |
| roxmltree | 0.21 | MIT/Apache-2.0 | nuxut-image | SVG ayrıştırma öncesi yapı denetimi: bayt/düğüm/derinlik/`<use>` açılımı sınırları, `<!ENTITY>` reddi (güvenilmeyen girdi). usvg'nin kullandığıyla aynı sürüm, yineleme yok | doğrudan usvg'ye vermek (sınırlar ayrıştırmadan sonra; bellek patlaması riski) |
| jxl-oxide | 0.12 (opsiyonel `jxl`, varsayılan KAPALI — yalnızca `nuxut-image-worker` açar; `image` entegrasyonu) | MIT/Apache-2.0 | nuxut-image | JPEG XL, saf Rust | libjxl (C++) |
| image `avif-native` | opsiyonel `avif` (kapalı) | — | nuxut-image | AVIF çözme | sistem `dav1d` (C) gerektirir; saf Rust AVIF çözücü yok. Açılırsa Arch: `dav1d` |
| rustix | 1 (`std`, `fs`, `process`; yalnızca unix) | MIT/Apache-2.0 | nuxut-image | görsel/ikon dosyasını `O_NONBLOCK` açmak: kullanıcının yazabildiği tema dizinine bırakılmış FIFO/aygıt ikon thread'ini bloklayamaz (sabit mimariye göre değişir; `unsafe`'siz). `process`: düşük öncelikli çözme thread'i (`setpriority`, yalnızca Linux'ta çağrılır; yeni crate yok). nuxut-wayland ile aynı sürüm, kopya yok | libc (her çağrı `unsafe`, sabitler elle); `metadata` önce denetlemek (yarış) |
| miniz_oxide | 0.8 (varsayılan `with-alloc`) | MIT/Zlib/Apache-2.0 | nuxut-image | hyprcursor `.hlc` (ZIP) girdilerinin deflate açılması, açılmış boyut sınırıyla (`decompress_to_vec_with_limit`: bomba sınırda durur). `png` üzerinden zaten ağaçta, aynı sürüm; yeni crate yok, saf Rust. ZIP kap ayrıştırması kendi yazımımız (`cursor::zip`, ~150 satır, tüm sınırlar açık) | `zip` crate (geniş yüzey: yazma, şifreleme, zip64, çok sayıda yöntem ve bağımlılık; yalnızca okuma ve iki yöntem gerekiyor); flate2 (miniz_oxide 0.9 + zlib-rs, ağaçta, ama sınırlı açma için ek sarmalama) |
| crc32fast | 1 | MIT/Apache-2.0 | nuxut-image | açılmış ZIP girdisinin CRC-32 doğrulaması (bozuk/kesik arşiv sessizce yanlış piksel vermesin). `png`/`flate2` üzerinden zaten ağaçta; SIMD | elle CRC tablosu (yavaş, gereksiz kod) |
| material-colors | 0.4.2 (`std`) | MIT/Apache-2.0 | nuxut-palette, nuxut-theme | material-color-utilities'in saf Rust portu: Celebi niceleme + puanlama, HCT, 9 şema varyantı, kontrast. nuxut-theme: yalnız HCT ton itme (yüksek karşıtlık kipi, aynı sürüm; yeni crate yok) | 0.5 rustc 1.97 istiyor (workspace 1.95); matugen ikilisi (dış süreç, şablon motoru bizimkinden geniş ama komut çalıştırır) |

Şablon motoru kendi yazımımız (`{{colors.<rol>.<kip>.<biçim>}}`, yalnızca değiştirme):
minijinja gibi tam motor döngü/koşul/ekleme getirir; kullanıcı şablonlarının yüzeyini dar
tutmak için bilerek seçilmedi.

| blake3 | 1 (`std`, `pure`) | CC0-1.0 OR Apache-2.0 | nuxut-video | İçerik hash'i (önbellek adı `<blake3[..16]>-<W>x<H>`): aynı dosya farklı adla eklense de tek girdi. `pure`: C/asm yok, Rust SIMD; çapraz derlenir | std hasher sürümler arası kararsız (kalıcı ad için uygun değil); sha2 ~5× yavaş |
| image `qoi` | 0.25 özelliği (qoi 0.4) | MIT/Apache-2.0 | nuxut-image (çözme), nuxut-video (kodlama) | Durağan duvar kağıdı önbelleği (ADR-0004: kayıpsız WebP'ten 2.5–5× hızlı çözme, ~%15 büyük; tepe bellek yalnızca çıktı) | kayıpsız WebP, zstd ham (C) |
| rustix | 1 (`std`, `fs`; yalnızca Linux) | Apache-2.0/MIT | nuxut-video | güvenilmeyen medyayı `O_NOFOLLOW`/`O_NONBLOCK` ile güvenli açmak (sabitler mimariye göre değişir; `unsafe`'siz API). Ağaçta zaten var (calloop, tempfile) | libc (her çağrı `unsafe`, sabitler elle); nix (ağaçta, daha geniş yüzey) |
| libloading | 0.8 (yalnızca Linux) | ISC | nuxut-video | `libmpv.so.2` ve `libEGL.so.1` (eglGetProcAddress) çalışma zamanı yüklemesi; kendi bağlamamız (libmpv2 crate'i LGPL-2.1, deny) | derlemede `-lmpv` bağlamak (mpv'siz kurulum derlenemezdi) |

Video (ADR-0002/0004): ölçümler — 960×540 yazılım çözme llvmpipe'ta ~%43 CPU, 79 MB anonim;
1080p30 yazılım çözme ~%85–97 CPU (bir çekirdek), ~150–230 MB anonim (VM, VA-API yok, yük ~30).
Animasyon akışı (AnimationStream) 102 kareli GIF'te 97.7 MB → ~3 MB; JPEG'i RGB'de küçültme 1080p
hedefte tepe RSS 133 → 89 MB.

### Görsel yalıtımı (nuxut-image-sandbox, ADR-0007)

Yeni sistem paketi yok. `unsafe` listesi değişmez: üç crate de güvenli API sunar; bizim kodumuzda
`unsafe` yok. Çalışma zamanında kurulması gereken tek şey işçi ikilisidir
(`nuxut-image-worker`, ARCHITECTURE "Görsel yalıtımı").

Yerleşim (2026-10-12): `seccompiler`, `landlock`, `libc` yalnızca `nuxut-image-sandbox` `worker`
özelliğiyle gelir ve onu yalnızca ayrı `nuxut-image-worker` paketi açar (JPEG XL de: `nuxut-image`
`jxl`). Tüketici işçiyi ayrı cargo çağrısıyla derlediği için bunlar tüketicinin ana ikilisine
girmez; ana süreç tarafı yalnızca `rustix` ve `blake3` çeker.

| Crate | Sürüm | Lisans | Neden | Alternatif / maliyet |
|---|---|---|---|---|
| seccompiler | 0.5 (yalnızca Linux) | Apache-2.0 OR BSD-3-Clause | işçinin seccomp-bpf izin listesini saf Rust'ta derleyip kurar (rust-vmm, Firecracker'ın kullandığı); argüman koşulları (`read` yalnızca fd 0, `mmap` `PROT_EXEC`'siz ve anonim). Tek bağımlılığı `libc` | libseccomp (C kütüphanesi, Arch `libseccomp`, binding `unsafe`); elle BPF (hata riski yüksek) |
| landlock | 0.4 (yalnızca Linux) | MIT OR Apache-2.0 | işçinin dosya sistemi, TCP ve soyut soket/sinyal erişimini çekirdekte sıfırlar (seccomp'a ek ikinci katman; eski çekirdekte en iyi çaba ile etkisiz). Bağımlılıkları `enumflags2`, `libc`, `thiserror` (ağaçta) | yalnızca seccomp (yeterli; landlock derinlemesine savunma) |
| libc | 0.2 (yalnızca Linux) | MIT OR Apache-2.0 | yalnızca sistem çağrısı/bayrak sabitleri (`SYS_*`, `PROT_EXEC`) seccomp kuralları için; fonksiyon çağrılmaz. Ağaçta zaten var (seccompiler, rustix, landlock) | sabitleri elle yazmak (mimariye göre değişir) |
| rustix | 1 (`std`, `fs`, `process`, `thread`, `event`, `time`; yalnızca Linux) | Apache-2.0/MIT | işçide `setrlimit`, `PR_SET_NO_NEW_PRIVS`, `PR_SET_DUMPABLE`; ebeveynde bloklamasız boru + `poll` ile son tarihli okuma/yazma (istek başına ek thread yok). Ağaçta zaten var | nix (ağaçta, daha geniş yüzey); libc (`unsafe`) |
| blake3 | 1 (`std`, `pure`; nuxut-video ile aynı) | CC0-1.0 OR Apache-2.0 | negatif önbellek: işçiyi öldüren girdinin özeti (ilk 16 bayt) | std `DefaultHasher` (çarpışma üretilebilir: kötü niyetli ikon başka ikonu "zehirli" gösterebilir) |

Not: rustix 1'de `close_range` yok ve fd kapatmak `unsafe` ister. İşçi miras fd'leri kapatmaz,
seccomp onları etkisiz kılar (ADR-0007 §Kısıtlama).

### Ağ (nuxut-net)

| Crate | Sürüm | Lisans | Neden | Alternatif / maliyet |
|---|---|---|---|---|
| ureq | 3.4 (`gzip`; `tls` özelliğiyle `rustls` + `platform-verifier`) | MIT/Apache-2.0 | senkron HTTP/1.1, tokio yok; özel çözümleyici (DNS sabitleme) ve bağlayıcı kancası, havuz/vekil kapatılabilir. calendar ile aynı sürüm | reqwest (tokio + hyper ağacı, boşta iş parçacıkları); elle HTTP (TLS/chunked/gzip hatası riski) |
| rustls (`ring`; ureq üzerinden + `tls` ile doğrudan, aynı sürüm/özellikler) | 0.23 (≥ 0.23.46: RUSTSEC-2026-0346..0349) | Apache-2.0/ISC/MIT | TLS; OpenSSL yok (deny.toml yasaklar). Doğrudan bağımlılık yeni crate getirmez; iki amaç: ureq'in `Io(InvalidData)` içinde sardığı sertifika/el sıkışma hatalarını `HttpError::Tls` olarak sınıflamak (geçici ağ hatası sanılıp yeniden denenmesin) ve yalnız `test-support`'taki sahte TLS sunucusu (`FakeServer::start_tls`; gömülü yalnız-test CA/sertifika DER fixture'ları, çalışma zamanında üretim yok) | native-tls (OpenSSL, C); sahte sunucu için rcgen (çalışma zamanında sertifika üretimi, ağır ağaç) |
| rustls-platform-verifier (ureq üzerinden) | 0.7 | MIT/Apache-2.0 | kök sertifikalar işletim sisteminden (Linux: `/etc/ssl` — Arch `ca-certificates`; Windows/macOS kendi depoları) | webpki-roots (gömülü, güncellenmez) |
| url | 2.5 | MIT/Apache-2.0 | URL ayrıştırma, köken, göreli yönlendirme çözümü | — |
| base64 | 0.22 | MIT/Apache-2.0 | Basic kimlik başlığı | elle (gereksiz) |
| zeroize | 1 | MIT/Apache-2.0 | `Secret`/`Auth` bellekte sıfırlanır | — |

Lisans: `webpki-root-certs` (Mozilla kök verisi) CDLA-Permissive-2.0 — deny.toml izin listesinde.
Çapraz denetim: `ring` C kodu derler; Linux'tan Windows/macOS C araç zinciri olmadığından
`tools/cross-check.sh` nuxut-net'i `--no-default-features` (TLS'siz) denetler. rustls/ring her iki
platformu resmen destekler.

### Portal (nuxut-portal, isteğe bağlı; 2026-10-13)

Framework'ün geri kalanı D-Bus'a bağlanmaz: zbus yalnız bu crate'te; cephede kapalı varsayılan
`portal` özelliği. zbus/async-io/futures-lite yalnız Linux hedefinde; Linux dışında crate derlenir ve her istek `PortalError::Unavailable` ile biter (ADR-0003 kural 2; 2026-10-13).

| Crate | Sürüm | Lisans | Neden | Alternatif / maliyet |
|---|---|---|---|---|
| zbus | 5.16 (varsayılan: `async-io` + `blocking-api`; tokio yok) | MIT | xdg-desktop-portal FileChooser çağrısı, `Request.Response` sinyali, `Request.Close`. Shell (tepsi) ve takvim aynı zbus'u kullanır: shell'in ağacında ek derleme yok | **ashpd 0.13** (`async-io`, `file_chooser`): ölçüm (2026-10-13, aynı çağrı, release + thin LTO + strip) zbus 1,65 MB / ashpd 2,15 MB (+0,5 MB); bağımlılık kümesi 66 / 72 crate (ashpd ek: ashpd, futures-util/-macro/-task, getrandom, memchr). ashpd yalnız async API verir (yine `block_on` gerekir), bize gereken tek arayüz için katman ekler; `Response` gönderen doğrulaması gibi sıkılaştırmalar yine bizde kalırdı. Elle D-Bus (libdbus, C) |
| async-io | 2 | MIT/Apache-2.0 | istek thread'inde `block_on`, zaman aşımı `Timer` | zbus'un çektiği sürüm; yeni crate değil |
| futures-lite | 2 | MIT/Apache-2.0 | `Response` ile iptal yarışı (`future::or`), `StreamExt` | zbus'un çektiği; yeni crate değil |
| async-channel | 2 | MIT/Apache-2.0 | istek başına iptal kanalı | zbus'un çektiği; yeni crate değil |

Ölçüm notu: karşılaştırma `cargo tree -e normal` ve aynı `OpenFile` çağrısını yapan iki küçük
ikiliyle yapıldı. **Çalışma zamanı paketleri (Arch):** `xdg-desktop-portal` + bir FileChooser arka
ucu — `xdg-desktop-portal-gtk` (Hyprland'de önerilen; `xdg-desktop-portal-hyprland` FileChooser
sunmaz) ya da `xdg-desktop-portal-gnome`/`-kde`. Yoksa istekler `PortalError::Unavailable` ile
biter (uygulama yolu elle yazdırma gibi bir yedek sunmalı).

### Erişilebilirlik (nuxut-access + nuxut-wayland `a11y`; ADR-0011, 2026-10-15, t127)

`nuxut-access` (yeni crate; ui ve runtime ortak ağaç tipi) koşulsuz, yalnız `accesskit` (saf veri)
ve ağaçta zaten olan `unicode-segmentation`. AT-SPI adaptörü (`accesskit_unix` → zbus) yalnız
`nuxut-wayland/a11y` (cephe `nuxut/a11y`, kapalı varsayılan) ile; özellik kapalıyken zbus yok.

| Crate | Sürüm | Lisans | Neden | Alternatif / maliyet |
|---|---|---|---|---|
| accesskit | 0.25 | MIT/Apache-2.0 | Erişilebilirlik ağacı modeli (`Node`, `Role`, `TreeUpdate`); tek zorunlu bağımlılık `uuid`, platform kodu yok (ADR-0003) | Kendi düğüm modelimiz + AT-SPI sunucusu (~17 bin satır; ADR-0011) |
| accesskit_unix | 0.24 (`async-io`, tokio yok) | MIT/Apache-2.0 | AT-SPI2 sunucu tarafı (arayüzler, olaylar, önbellek, registry kaydı), `IsEnabled` izleme, tembel ağaç. Çeker: accesskit_atspi_common, accesskit_consumer, atspi(-common/-proxies) 0.29, zbus 5 (shell'de zaten var), zbus_xml, zbus-lockstep, phf. Ölçüm: +~1,7 MB ikili, boşta 0 CPU, +~3 MB RSS, +3 thread (accessibility.md §4.2) | `atspi` crate'i + kendi sunucumuz; elle `zbus::interface` |
| accesskit_consumer | 0.39 | MIT/Apache-2.0 | yalnız dev: nuxut-ui `access_tests` ağaç geçerliliği (accesskit_unix'in zaten çektiği) | — |
| atspi | 0.29 (`proxies`) | Apache-2.0/MIT | yalnız dev (Linux): `tests/a11y_atspi.rs` ve `examples/atspi_dump.rs` AT gibi ağacı okur; accesskit_unix'in zaten çektiği sürüm | elle zbus proxy'leri |

**Çalışma zamanı paketleri (Arch):** `at-spi2-core` (a11y bus'ı + registry; yoksa adaptör hiç kurulmaz,
maliyet sıfır), ekran okuyucu için `orca` (isteğe bağlı; install.sh `--with-a11y`).

### Dış yapılandırma uygulama (nuxut-apply, shell ADR-0001)

| Crate | Sürüm | Lisans | Neden | Alternatif / maliyet |
|---|---|---|---|---|
| toml_edit | 0.25 (`parse`, `display`) | MIT/Apache-2.0 | kullanıcının/göçün TOML dosyasında **tek anahtarı** değiştirmek: yorumlar, boşluk, sıra ve değer yazımı (tırnak türü, onaltılık sayı) bayt bayt korunur; geri yükleme eski ifadeyi aynen geri koyar. `toml` 1 ile aynı `toml_parser`/`toml_writer`'ı paylaşır (yeni alt ağaç yok); settings zaten aynı crate'i kullanıyor | `toml` + serde ile yeniden serileştirme (yorumları ve biçimi siler); elle TOML düzenleyici (spesifikasyon kenar durumları) |
| serde, toml, thiserror, tracing | (çekirdekle aynı) | — | snapshot manifest'i (`manifest.toml`), hata tipleri, log (değer/yol yazılmaz) | — |
| blake3 | 1 (`std`, `pure`; nuxut-video ile aynı) | CC0-1.0 OR Apache-2.0 | yazım ön koşulundaki içerik özeti (`Expect.hash`) ve snapshot değer özeti (yabancı değer tespiti, blob bütünlüğü); ağaçta zaten var | std `DefaultHasher` (64 bit, sürümler arası kararsız — diskteki özet için uygun değil); sha2 (~5× yavaş, yeni crate) |
| rustix | 1 (`std`, `fs`, `process`; yalnızca unix) | Apache-2.0/MIT | `geteuid`: hedef dosya başka kullanıcınınsa üstüne yazılmaz (`ExternallyManaged`); `unsafe`'siz. Ağaçta zaten var; `process` özelliği yeni crate getirmez | libc (`unsafe`); dosya sahibini kendi oluşturduğumuz geçici dosyayla karşılaştırmak (dolambaçlı) |
| nix | 0.31 (`user`; yalnızca unix) | MIT | test örneği kapısı için **gerçek** ev dizini (`User::from_uid(geteuid)`, `getpwuid_r`; NSS/LDAP dahil) — `$HOME` taklit edilebilir. Ağaçta zaten var (nuxut-render, calloop); `user` özelliği yeni crate getirmez; `unsafe`'siz | libc `getpwuid_r` (`unsafe`, crate'te yasak); `/etc/passwd` ayrıştırmak (NSS kullanıcılarını kaçırır); `$HOME`'a güvenmek (kapı aşılır) |

INI düzenleyici kendi kodumuz (~200 satır): `rust-ini`/`configparser` yorumları, sırayı ve
`kdeglobals` `[$e]` eklerini korumuyor. Dosya kilidi std `File::lock`/`try_lock` (Rust 1.89+),
crate yok.

### Yalnızca geliştirme

| Crate | Neden |
|---|---|
| tracing-subscriber 0.3 (`fmt`, `env-filter`, `ansi`) | örneklerin loglaması |
| khronos-egl (dev; render, image) | başsız (surfaceless EGL) GL testleri (image: duvar kağıdı bulanıklığı) |
| nix 0.31 (`fs`; dev, render, yalnızca Linux) | `font_cache_bench --evict` (posix_fadvise ile sayfa önbelleğinden atma); calloop zaten çeker |
| flate2 1 (dev, net) | gzip bombası testi (ureq zaten çeker) |
| image 0.25 (`png`; dev, render) | altın görüntü testi (`tests/it/blob_golden.rs`): PNG okuma/yazma; ağaçta zaten var (nuxut-image), yalnız png özelliği. Alternatif: `png` crate'i doğrudan (RGBA dönüşümünü elle yazmak) |
| nuxut-theme (dev, render), nuxut-motion (dev, wayland) | ADR-0008 sahneleri `Theme::default()` token'larıyla; `blob_scenes` örneğinde açılış yayı |
| image 0.25 (dev, image-sandbox) | düşmanca testlerde PNG fikstürü kodlamak; ağaçta zaten var |
| image 0.25 (dev, ui) | `brand_icons` örneği: hicolor PNG'lerini kodlamak; ağaçta zaten var |
| nuxut-image-sandbox `sandbox-test-hooks` (dev, kendine bağımlılık) | testlerde işçiye düşmanca kancalar (sonsuz döngü, dosya/ağ/süreç denemesi, sahte yanıt); dağıtılan işçide derlenmez |
| tempfile 3 (dev; image, image-sandbox, palette, video, apply, i18n) | sahte ikon teması, önbellek ve şablon dizinleri; nuxut-apply ve nuxut-i18n (çeviri dizini izleme) testlerinde geçici XDG kökleri |

## Platform (ADR-0003)

Framework cross-platform'dur; `nuxut-wayland` dışındaki crate'ler Windows ve macOS'ta derlenir
(`tools/cross-check.sh`, son çalıştırma: 9 crate × {x86_64-pc-windows-msvc, aarch64-apple-darwin}
hepsi TAMAM). "Linux" satırları `#[cfg(target_os = "linux")]` arkasındadır ve diğer platformlarda
yedekleri vardır.

| Bağımlılık | Platform | Not / diğer platformlardaki yedek |
|---|---|---|
| serde, toml, serde_ignored, thiserror, tracing, dirs | hepsi | dirs platformun dizinlerini verir |
| inotify (nuxut-config, nuxut-i18n) | Linux | `ConfigWatcher::new` → `Unsupported`; config açılışta okunur. `nuxut_i18n::watch::UserDirWatcher` yalnızca Linux (Debounce/ChangeTracker her platformda) |
| glow, cosmic-text, swash | hepsi | macOS'ta GL yerine ileride wgpu/Metal yolu (ADR-0003) |
| fontconfig-parser (nuxut-render) | Linux | genel aileler platform varsayılanlarından (Segoe UI / SF Pro …) |
| image, fast_image_resize, resvg, roxmltree, jxl-oxide, material-colors, chrono, pure-rust-locales | hepsi | saf Rust |
| zeroize (nuxut-core, nuxut-render, nuxut-ui, nuxut-wayland, nuxut-net) | hepsi | saf Rust |
| memmap2 (nuxut-ui `SecretBuffer::locked`) | hepsi derlenir | `mlock` unix, `MADV_DONTDUMP` Linux; Windows'ta eşleme kilitsiz (alan aynı çalışır) |
| ureq, rustls (ring), rustls-platform-verifier, url (nuxut-net) | hepsi | ring C/asm içerir; çapraz denetim TLS'siz (`--no-default-features`) |
| image `avif` (dav1d-sys, C) | Linux (opsiyonel, kapalı) | çapraz derlenmez; varsayılan özelliklerde yok |
| ikon teması araması (nuxut-image, freedesktop) | Linux mantığı, her yerde derlenir | diğer platformlarda tema bulunamaz → yedek ikon (cfg ile açıkça işaretlenecek) |
| uygulama ikonları (nuxut-image `app_icon`: `.desktop` dizini + tema; yeni crate yok — `freedesktop-icons`/`xdg` reddedildi, gerekçe `docs/design/app-icons.md`) | Linux mantığı, her yerde derlenir | diğer platformlarda `.desktop`/tema bulunamaz → `AppIcon::Missing` (yedek ikon) |
| libmpv, libloading, rustix (nuxut-video) | Linux | diğer platformlarda `ffi` yerine uninhabited taslak: `available() == false`, kurucular `Unavailable("bu platformda video duvar kağıdı yok")`; durağan görsel içe aktarma (QOI) her yerde çalışır |
| blake3 (`pure`), image `qoi` | hepsi | saf Rust |
| seccompiler, landlock, libc (nuxut-image-sandbox `worker`, nuxut-image-worker), rustix (nuxut-image-sandbox) | Linux | diğer platformlarda işçi yok: `Sandbox::isolation()` → `Unavailable` (kapılardan geçmiş temel raster süreç içinde ilk kare; SVG/JXL/AVIF yedek ikon); `worker_main` hemen 70 ile çıkar |
| nuxut-apply (toml_edit, rustix `process`) | hepsi derlenir; tam güvence unix | izin/sahip/inode denetimleri `sys` modülünde `cfg(unix)`; diğer platformlarda en iyi çaba (izin korunmaz, sahip denetimi `readonly`). dconf/systemctl alt süreçleri yoksa `Unavailable` |
| smithay-client-toolkit, wayland-*, calloop, khronos-egl (nuxut-wayland) | yalnızca Linux | platform katmanı (winit) ilk cross-platform uygulamayla gelir |

## Sürüm sabitleri (geçici; takip edilir)

### `blocking = "=1.6.2"` (2026-10-16, kabuk performans turu; takipli)

zbus'ın `unblock` havuzu için yalnız bir sürüm sabiti; kodda kullanılmaz. Kök `[workspace.dependencies]`
içinde tanımlı. zbus'ı çeken crate'ler alır: `nuxut-portal` (Linux) ve `nuxut-wayland`'in `a11y`
özelliği. Bu crate'leri kullanan uygulamaların (Ayarlar: `portal`) kilidi de kendiliğinden 1.6.2'ye iner.

- **Kök neden:** smol-rs/blocking PR #75 (commit `6f41360`, 1.7.0) havuz thread'inin çıkış koşulunu
  `thread_count > thread_limit && timed_out && queue.is_empty()` yaptı. Varsayılan sınır 500 olduğu
  için koşul pratikte hiç tutmuyor: boştaki thread süreç ömrü boyunca 500 ms'de bir uyanıyor.
  Upstream issue #81 "Erroneous condition when stopping threads" açık; son sürüm 1.7.0
  (2026-10-16).
- **Ayarla önlenmez:** tek ayar thread üst sınırı (`BLOCKING_MAX_THREADS`,
  `set_max_blocking_threads`). Thread yalnız sınır aşılınca çıkar; sınır 1 olsa da son thread kalır.
- **Ölçüm:** shell'de boşta 4,1 → 0,1 uyanma/sn (shell `docs/research/perf-2026-10.md`).
- **Kaldırma koşulu:** #81'i düzelten `blocking` sürümü çıkınca kök satır ve iki crate'teki satırlar
  silinir. Ardından shell `tools/perf` ile boşta ölçüm tekrarlanır. Shell ve takvimin kendi
  sabitleri de aynı anda kaldırılır.

## Yinelenen sürümler (cargo-deny uyarıları)

| Crate | Sürümler | Kaynak | Durum |
|---|---|---|---|
| skrifa / read-fonts / font-types | 0.40 + 0.44 / 0.37 + 0.41 / 0.11 + 0.12 | cosmic-text 0.19: harfrust eski skrifa'yı, swash yenisini çeker | upstream; cosmic-text güncellemesinde kontrol |
| hashbrown | 0.13 + 0.17 | image `avif-native` → mp4parse → fallible_collections | `avif` özelliği varsayılan kapalı; Cargo.lock kilitlese de varsayılan derlemede linklenmez |
| miniz_oxide | 0.8 + 0.9 | png doğrudan + flate2 üzerinden | upstream png |
| roxmltree, syn | 2 sürüm | resvg ağacı / proc-macro'lar | derleme süresi; ikiliye etkisi yok (syn) |

## Açık güvenlik bildirimleri (cargo-deny)

| Bildirim | Crate | Kaynak | Durum |
|---|---|---|---|
| RUSTSEC-2026-0192 (bakımsız; düzeltilmiş sürüm yok) | ttf-parser 0.25 | nuxut-render → cosmic-text 0.19 → fontdb 0.23 | `unmaintained = "all"`; deny.toml `ignore` listesinde kimliği ve gerekçesiyle tek istisna (2026-10-16; önceki "workspace" politikası dolaylı bakımsız crate'leri toptan gizliyordu). Güvenlik açığı değil. **Takip:** fontdb 0.24 ttf-parser'ı bırakıyor; cosmic-text fontdb 0.24'e geçen sürüm çıkarınca yükselt (2026-10-15). Shell ve settings aynı ağacı framework'ten alır |

## Review kontrol listesi (yeni bağımlılık)

1. Gerçekten gerekli mi; std ya da mevcut bir crate yetmiyor mu?
2. Bakımlı mı (son sürüm, açık sorunlar), lisansı `deny.toml` izin listesinde mi?
3. Varsayılan feature'lar kapalı, yalnızca gerekenler açık mı?
4. Boşta CPU/bellek ve başlangıç süresine etkisi ölçüldü mü (özellikle arka plan thread'i,
   global önbellek, büyük tablo)?
5. Sistem kütüphanesi istiyorsa Arch paketi yukarıdaki tabloya eklendi mi; yoksa
   özelliğin zarifçe kapanması (ör. mpv yoksa statik duvar kağıdı) sağlandı mı?
6. `cargo deny check` hatasız.
