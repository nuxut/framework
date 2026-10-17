# nuxut-framework — Mimari

Nuxut uygulamalarının (Linux/Hyprland'e özgü `nuxut-shell` ve Linux/Windows/macOS'ta çalışacak
uygulamalar; ADR-0003) üzerine kurulduğu cross-platform
Rust UI çerçevesi. Kendi renderer'ımız var (noctalia v5 felsefesi), ama tekerleği yeniden icat
etmeyiz: Wayland için `smithay-client-toolkit`, GPU için `glow` (OpenGL ES 3.0), metin için
`cosmic-text`, event loop için `calloop`.

## Crate'ler ve bağımlılık yönü

```
nuxut-core      geometri + renk                         (bağımlılık yok, unsafe yok)
nuxut-motion    analitik yay, bezier, deform            → core
nuxut-config    TOML yükleme, doğrulama, inotify izleme  (unsafe yok)
nuxut-theme     framework.toml şeması → Theme token'ları → core, config, motion
nuxut-render    GL renderer, Painter, metin, blob alanı  → core, glow, cosmic-text
nuxut-wayland   runtime: layer-shell, EGL, input, döngü  → core, config, theme, render, i18n, sctk
nuxut-locale    locale (zaman: LC_ALL > LC_TIME > LANG;     → chrono (unstable-locales, clock), pure-rust-locales
                sayı: LC_ALL > LC_NUMERIC > LANG),
                yerelleştirilmiş tarih/saat ve sayı biçimleme, dil/ülke adları,
                gömülü locale listesi
nuxut-i18n      Fluent katalogları (ADR-0010): dil zinciri, → config, locale; fluent-bundle, unic-langid
                t!/embed!, katalog denetimi + gömülü        (+ nuxut-i18n-macros: embed! proc-macro)
                metin bekçisi
nuxut-ui        bileşenler (Label, Interaction, …)       → core, motion, theme, render, locale;
                                                           `brand` özelliğiyle (varsayılan) image
nuxut-image     görsel çözme, ikon teması, animasyon,    → core; `render` özelliğiyle render, motion
                Loader, Wallpaper
nuxut-image-sandbox  güvenilmeyen görselin yalıtılmış    → image (jxl YOK); Linux: rustix; `worker`
                çözümü, ana süreç tarafı (ADR-0007)      özelliğiyle seccompiler, landlock (cephe
                                                          yeniden dışa aktarmaz)
nuxut-image-worker  işçi ikilisi (yalnızca `[[bin]]`)   → image-sandbox[worker], image[jxl]; ayrı
                                                          cargo çağrısıyla derlenir
nuxut-palette   duvar kağıdından Material You paleti,    → core, image, theme
                şablon dışa aktarımı, `nuxut-palette` CLI
nuxut-video     libmpv oynatma (dlopen) + duvar kağıdı   → core, render, image, motion
                içe aktarma/dönüştürme
nuxut-net       SSRF-dayanıklı senkron HTTP istemcisi     (framework crate'ine bağımlılık yok; ureq/rustls)
nuxut-apply     dış config yazımı, parça düzenleyicileri,  → config (yalnızca MAX_CONFIG_BYTES); toml_edit
                snapshot/geri yükleme (shell ADR-0001)
nuxut-access    erişilebilirlik ağacı kurucusu (ADR-0011):   → core; accesskit (saf veri)
                AccessTree, kararlı kimlik, AT eylem süzgeci
nuxut-portal    xdg-desktop-portal istemcisi (FileChooser), (framework crate'ine bağımlılık yok; zbus)
                isteğe bağlı; Linux dışında Unavailable
nuxut           cephe: hepsini yeniden dışa aktarır     → hepsi (nuxut-net, nuxut-apply hariç: isteyen
                                                          tüketici doğrudan bağımlanır; nuxut-portal
                                                          yalnız `portal` özelliğiyle)
```

Kurallar:
- Ok yönü tersine çevrilemez. `render` ve `ui` Wayland, EGL ya da Linux API'si bilmez; GL'e özgü
  tip `Painter` API'sine sızmaz (ikinci render backend'i — wgpu/Metal — eklenebilsin).
- **Platform kapsamı (ADR-0003):** framework cross-platform'dur; shell yalnızca Linux/Hyprland.
  `nuxut-wayland` dışındaki her crate Windows ve macOS hedeflerinde derlenebilir kalır
  (`tools/cross-check.sh`). Linux'a özgü kod (fontconfig, inotify, XDG yolları, libmpv/VA-API,
  ikon teması…) `#[cfg(target_os = "linux")]` arkasında, platform bağımsız arayüz ve diğer
  platformlarda çalışan bir yedekle (en azından "kullanılamıyor") durur. Windows/macOS platform
  katmanı (winit; macOS için wgpu/Metal) ilk cross-platform uygulamayla gelir.
- **Crate'ler bileşen başına değil, bağımlılık sınırına göre bölünür.** Yeni bileşen
  `nuxut-ui` içine girer. Ağır dış bağımlılık getiren bir bileşen (markdown, video, takvim…)
  `nuxut-ui` içinde cargo feature arkasına girer; yalnızca bağımlılık ağaçları gerçekten
  ayrışırsa ayrı crate olur. Kullanılmayan kod zaten linklenmez; crate patlaması bakım yüküdür.
- `unsafe` yalnızca `nuxut-render` (GL çağrıları), `nuxut-wayland` (EGL/ham pointer) ve
  `nuxut-video::ffi` (libmpv `dlopen`, ADR-0004) içinde, her blok `// SAFETY:` yorumuyla. Diğer crate'lerde `#![forbid(unsafe_code)]`.

## Çalışma zamanı sözleşmesi (v0)

Bu bölüm `nuxut-wayland` ile `nuxut-render` ve uygulamalar arasındaki sözleşmedir. Değişiklik
gerekiyorsa önce bu belge güncellenir.

### Uygulama trait'i

```rust
pub trait App: Sized + 'static {
    /// Arka plan thread'lerinden gelen mesajlar (IPC olayları vb.).
    type Message: Send + 'static;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo);
    fn output_changed(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {}
    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId);

    /// Bir yüzey ilk kez veya yeniden boyutlandı (configure) ya da ölçeği değişti.
    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {}
    /// Yüzeyin ilk tamponu sunuldu (ilk tamponlu commit'in frame callback'i `done`); yüzey
    /// başına bir kez. Bkz. Runtime davranışı → İlk sunum.
    fn surface_presented(&mut self, cx: &mut Context<Self>, surface: SurfaceId) {}
    /// Çıkışı compositor'a bırakılmış layer yüzeyinin çıkışı öğrenildi (`wl_surface.enter`:
    /// `Some`) ya da yüzey çıkışından ayrıldı (`None`). Bkz. `OutputTarget`.
    fn surface_output_changed(&mut self, cx: &mut Context<Self>, surface: SurfaceId, output: Option<OutputId>) {}

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent);
    /// Ham dokunma (tüm temaslar). Down'da true = temas sahiplenildi, işaretçi öykünmesi yok.
    /// Varsayılan false: sahiplenilmeyen ilk temas PointerSource::Touch işaretçisi olur (docs/design/touch.md).
    fn touch(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: TouchEvent) -> bool { false }
    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {}
    /// Yüzey klavye odağını aldı (`true`) ya da kaybetti (`false`). Panel dışına (bir
    /// pencereye) tıklanınca odak kaybıyla kapanan paneller bunu kullanır.
    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {}
    /// Değiştiriciler/kilit durumları (Caps/Num Lock) değişti; klavye odağındaki yüzey için.
    fn modifiers_changed(&mut self, cx: &mut Context<Self>, surface: SurfaceId, modifiers: Modifiers) {}

    fn message(&mut self, cx: &mut Context<Self>, message: Self::Message) {}
    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {}

    /// framework.toml değişti; `cx.theme()` yeni tema. Yalnızca `[locale]` değiştiyse gelmez.
    fn theme_changed(&mut self, cx: &mut Context<Self>) {}
    /// framework.toml `[locale]` değişti (ADR-0010 `LocaleChanged`): süreç geneli katalog
    /// (`nuxut::i18n::current()`) zaten yenilendi; metinler yeniden istenir, yerleşim yenilenir.
    fn locale_changed(&mut self, cx: &mut Context<Self>) {}
    /// `cx.watch_config(name)` ile izlenen dosya değişti.
    fn config_changed(&mut self, cx: &mut Context<Self>, name: &str) {}

    /// Kare çizimi. `now` karedeki tüm animasyonlar için ortak saat.
    /// Animasyon sürüyorsa uygulama `cx.request_frame(surface)` çağırır; çağırmazsa
    /// yüzey bir sonraki olaya kadar uyur (boşta sıfır CPU/GPU).
    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter);

    /// SIGTERM/SIGINT/SIGHUP geldi. Varsayılan `cx.quit()`; uygulama geciktirmek isterse
    /// (kaydedilmemiş durum vb.) kendisi sonra `quit` der. Çıkışta `run` normal döner ve
    /// `App` düşürülür (Drop çalışır: arka plan thread'leri temiz kapanır).
    fn terminate(&mut self, cx: &mut Context<Self>) { cx.quit() }
}

pub fn run<A: App>(app: A) -> Result<(), RunError>;
```

### Context

```rust
impl<A: App> Context<A> {
    pub fn create_layer_surface(&mut self, spec: LayerSurfaceSpec) -> SurfaceId;
    pub fn destroy_surface(&mut self, id: SurfaceId);
    pub fn set_input_region(&mut self, id: SurfaceId, rects: &[Rect]); // boş = tıklama geçer
    pub fn set_exclusive_zone(&mut self, id: SurfaceId, zone: i32);
    // anchor + margin (+ boyut; None = korunur) tek commit, bir sonraki karede; kare beklenirken
    // çağrılar birleşir (son değer kazanır), değişmediyse istek yok. Ayrıntı: "Runtime davranışı".
    pub fn set_layer_placement(&mut self, id: SurfaceId, anchor: Anchor, margin: Insets, size: Option<(u32, u32)>);
    pub fn set_keyboard_mode(&mut self, id: SurfaceId, mode: KeyboardMode);
    pub fn request_frame(&mut self, id: SurfaceId);       // frame callback ile hizalı
    pub fn surface_size(&self, id: SurfaceId) -> Option<Size>; // mantıksal; configure öncesi None
    pub fn surface_scale(&self, id: SurfaceId) -> Option<f32>;
    pub fn surface_output(&self, id: SurfaceId) -> Option<OutputId>; // Compositor hedefli layer: enter öncesi None
    pub fn theme(&self) -> &Theme;
    pub fn text(&mut self) -> &mut TextSystem;              // çizim dışı; draw içinde panic → painter.text_system()
    pub fn sender(&self) -> MessageSender<A::Message>;      // Clone + Send
    pub fn set_timer(&mut self, after: Duration) -> TimerId; // tek atımlık
    pub fn cancel_timer(&mut self, id: TimerId);
    pub fn watch_config(&mut self, name: &str);              // <config_dir>/<name>.toml
    pub fn set_cursor(&mut self, id: SurfaceId, shape: CursorShape); // yüzey başına saklanır
    pub fn quit(&mut self);
}

pub struct LayerSurfaceSpec {
    pub namespace: String,
    pub output: OutputTarget,       // Output(OutputId) | Compositor
    pub layer: Layer,               // Background | Bottom | Top | Overlay
    pub anchor: Anchor,             // bitflags: TOP | BOTTOM | LEFT | RIGHT
    pub size: (u32, u32),           // 0 = anchor yönünde gerilir
    pub exclusive_zone: i32,
    pub margin: Insets,
    pub keyboard: KeyboardMode,     // None | OnDemand | Exclusive
}
impl LayerSurfaceSpec {
    pub fn new(namespace, output: impl Into<OutputTarget>) -> Self; // Top, anchor yok, boyut 0; OutputId verilebilir
    pub fn on_focused_output(namespace) -> Self;                    // = new(namespace, OutputTarget::Compositor)
}
pub enum OutputTarget { Output(OutputId), Compositor }  // From<OutputId>
// Çıkışı compositor'a bırakmak (`Compositor`; polkit diyaloğu, karşılama, OSD):
// - `get_layer_surface`'e çıkış null gider; compositor kendi politikasıyla seçer (Hyprland, sway:
//   odaklı/etkin çıkış). Bilinmeyen `Output(id)` (yarışta kaldırılmış) da böyle davranır (uyarı).
// - `surface_output(id)` ilk `wl_surface.enter`e dek `None`, sonra seçilen çıkış;
//   `App::surface_output_changed(surface, Some(o))` bildirir (`leave` → `None`). Açık çıkışlı
//   layer, pencere ve kilit yüzeylerinde gelmez.
// - Ölçek: ilk configure'dan önceki tahmin bilinen çıkışların en büyük ölçeği (tek çıkışta tam;
//   değilse ilk kare keskin çizilip compositor küçültür, bulanık 1x kare yok). Hyprland ve sway
//   kesirli/tercih edilen ölçeği yüzey yaratılınca, ilk configure'dan önce gönderir (sway'de
//   doğrulandı: ilk kare doğru ölçekte, yeniden çizim yok). Geç gelirse `surface_resized` + yeni kare.
// - Çıkış kaldırılınca compositor yüzeyi kapatır (`closed`): yüzey sessizce kaldırılır, uygulama
//   `output_removed` alır — açık çıkışlı yüzeylerle aynı. Uygulama `surface_scale(id).is_none()`
//   ile yüzeyin gittiğini anlar, gerekirse `on_focused_output` ile yeniden açar
//   (örnek: `nuxut-wayland/examples/focused_output.rs`).
// Anchor: kendi bayrak tipi (NONE, TOP, BOTTOM, LEFT, RIGHT, ALL; `|`, contains)

// Clone + Send + 'static (M: Send + 'static). calloop channel: send döngüyü her zaman uyandırır.
// Arka plan callback'inden: `move |e| { let _ = sender.send(Msg::Hypr(e)); }`
pub struct MessageSender<M>;
impl<M> MessageSender<M> { pub fn send(&self, m: M) -> Result<(), Closed>; }
pub struct Closed;            // runtime kapandı; mesaj düşürüldü

// `name` bağlayıcı adıdır ("Virtual-1", "DP-1"); Hyprland monitörünü eşlemenin yolu budur.
pub struct OutputInfo { pub id: OutputId, pub name: String, pub description: String,
                        pub position: Point, pub size: Size /* mantıksal */,
                        pub scale: f32 /* tamsayı çıkış ölçeği */, pub refresh: f32 /* Hz */,
                        pub mode_size: (u32, u32), pub physical_size: (u32, u32) }
// Fiziksel boyut (2026-10-14, istek): `mode_size` geçerli kipin çözünürlüğüdür
// (`wl_output.mode` current, fiziksel piksel, panelin kendi yönünde — ayarlarda gösterilen
// "2560×1440"); xdg-output'tan gelmez. `physical_size` aynı kip compositor uzayının yönünde
// (90/270 derece dönüşte eksenler yer değiştirir; `size` ile aynı yön): `physical_size / size`
// etkin (kesirli olabilen) ölçektir. `refresh` geçerli kipin yenileme hızıdır. Bilinmeyen
// kip: `(0, 0)` ve `refresh` 0. Kip/dönüş/ölçek değişimi `output_changed` ile gelir. Test
// compositor'ında doğrulandı: 1920×1080 kip, `scale 1.5` → size 1280×720, physical 1920×1080,
// `scale` (tamsayı) 2; `transform 90` → size 720×1280, physical 1080×1920, mode 1920×1080.
// xdg-output yoksa mantıksal boyut yedeği artık yönlendirilmiş kipten hesaplanır (önce dönüş
// hesaba katılmıyordu).

// Kimlikler opaktır (gerçek değeri runtime verir). Birim testte kurmak için `test-util` özelliği
// (nuxut-wayland; cephe `nuxut/test-util` iletir) YALNIZCA dev-dependency'de açılır:
//   [dev-dependencies] nuxut = { path = "…", features = ["test-util"] }
// Üretim derlemesinde bu kurucular yoktur.
impl OutputId  { pub fn for_test(raw: u32) -> Self; }    // cfg(any(test, feature = "test-util"))
impl SurfaceId { pub fn for_test(raw: u64) -> Self; }

pub enum RunError { Connect(String), MissingProtocol(&'static str), Egl(String),
                    EventLoop(String), Disconnected(String) }
```

### Girdi tipleri

`nuxut_core::input` modülündedir; platform crate'leri yeniden dışa aktarır, `nuxut-ui` Wayland
bilmeden tüketir. Konumlar yüzeye göre mantıksal. Yardımcılar: `PointerEvent::{new, translated}`,
`KeyEvent::{new, is_down}`, `XkbKey::hyprland`, `Button::from_code` (Linux), `Key::from_keysym` (XKB), `input::{printable, printable_secret}`.
Klavye/IME metni `SecretText` taşır (bkz. "Gizli metin girişi").

Dokunmatik ekran (`wl_touch`, `docs/design/touch.md`): sahiplenilmeyen ilk temas işaretçi olarak
öykünülür — `Down` → `Enter, Motion, Press(Left)`, `Motion` → `Motion`, `Up` → `Release(Left),
Leave`, compositor iptali → yalnız `Leave`; basılı ve kımıldamayan birincil temas için
`TOUCH_TAP_DELAY` sonra aynı konumda tek `Motion` (bekleme tiki). Diğer temaslar yalnız
`App::touch`'a gelir; en çok 16 temas. Dokunmada imleç şekli değişmez, hover yalnız parmak
basılıyken. Dokunma serial'ı son girdi sayılır (move/resize, etkinleştirme, pano).
Canlı test için `test-touch` özelliği (`nuxut/test-touch`, yalnız debug; sürümde derleme hatası):
`NUXUT_TEST_TOUCH=<fifo>` satırlarından (`down ID NS X Y`, `motion ID X Y`, `up ID`, `cancel`)
sentetik temas, `wl_touch` ile aynı yoldan (`docs/design/touch.md` "Sentetik dokunma").

```rust
pub struct PointerEvent { pub position: Point, pub kind: PointerKind, pub source: PointerSource }
// new → Mouse, touch → Touch, with_kind(pos, kind) kaynağı korur, is_touch()
pub enum PointerSource { Mouse /*varsayılan; fare, dokunmatik yüzey*/, Touch /*dokunmatik ekranın birincil teması, öykünme*/ }
pub struct TouchEvent { pub id: TouchId, pub position: Point, pub phase: TouchPhase }   // App::touch
pub struct TouchId(pub i32);  pub enum TouchPhase { Down, Motion, Up, Cancel }
pub const TOUCH_SLOP: f32 = 8.0;  pub const TOUCH_TAP_DELAY: Duration = 100 ms;
pub enum PointerKind { Enter, Leave, Motion, Press(Button), Release(Button),
                       Scroll { delta: Point /* piksel */, steps: Point /* çentik, +y aşağı */ },
                       ScrollEnd /* dokunmatik yüzey axis_stop: fırlatma başlar */ }
pub enum Button { Left, Right, Middle, Other(u32) }
pub struct KeyEvent { pub key: Key, pub state: KeyState, pub text: Option<SecretText>, pub modifiers: Modifiers,
                      pub xkb: Option<XkbKey> /* gerçek klavyede Some; KeyEvent::new → None */ }
pub struct XkbKey { pub keycode: u32 /* evdev+8 */, pub sym: NamedKeysym /* üretilen */,
                    pub base: NamedKeysym /* etkin düzen, seviye 0 */,
                    pub first_layout_base: NamedKeysym /* düzen 0, seviye 0 */ }
pub struct NamedKeysym { pub raw: u32, pub name: String /* xkb_keysym_get_name */ }   // Drop'ta sıfırlanır (XkbKey Drop'suz: tüketiciler yapı güncellemesiyle taşır; keycode yığıtta u32)
// Düşerken sıfırlanan metin: Deref<Target = str>, AsRef/Borrow<str>, From<String> (taşıma)/&str/&String/char,
// PartialEq<str/&str/String> (iki yönlü), Debug = "<n karakter gizli>", Display yok; clear(), truncate_zeroized(max).
pub struct SecretText(/* String */);
pub enum KeyState { Pressed, Repeated, Released }        // tekrar runtime'da (calloop timer)
pub struct Modifiers { pub ctrl: bool, pub shift: bool, pub alt: bool, pub logo: bool,
                       pub caps_lock: bool, pub num_lock: bool /* kilit DURUMU; kısayol eşlerken yok say */ }
pub enum Key { Escape, Enter, Tab, Backspace, Delete, Left, Right, Up, Down, Home, End,
               PageUp, PageDown, Space, Char(char), Other(u32 /* keysym */) }
```

`text` yalnızca yazdırılabilir metindir (kontrol karakterleri elenir); bırakmada `None`.
Keysym çözümü sctk'nin xkbcommon klavyesiyle yapılır (`libxkbcommon` çalışma zamanı bağımlılığı).
`xkb` kısayol yakalama içindir (Ayarlar → Kısayollar): runtime compositor keymap'ini keymap
olayında ikinci kez derler (`keysym.rs`) ve aynı tuşun değiştiricisiz keysym'lerini Hyprland'in
çözümüyle birebir verir. Hyprland 0.56 bind'i **seviye 0** keysym'le eşler (değiştiriciler ayrı
maske): varsayılan ilk düzen (`first_layout_base`), `input:resolve_binds_by_sym = true` iken etkin
düzen (`base`); `XkbKey::hyprland(by_sym)` doğru olanı seçer. Türkçe Q'da Shift+1 → `SHIFT, 1`,
Shift+ı → `SHIFT, idotless`. `code:<n>` bind'i `keycode` kullanır.

### Runtime davranışı

- **Layer yerleşimi (`set_layer_placement`; 2026-10-16, osk isteği — yüzen ekran klavyesi).** İstek
  yüzeyin bekleyen yerleşimini (anchor, kablodaki `i32` margin, istenen boyut) değiştirir ve kare
  ister; çizim yolu `apply_scale`'in yanında `set_anchor`/`set_margin`/`set_size`'ı gönderir, swap'ın
  (ya da hasarsız karede tamponsuz) commit'i taşır: konum ve içerik aynı commit'te. Kare beklenirken
  (`callback_pending`) gelen çağrılar birleşir, son değer kazanır; gönderilenle aynı yerleşim (aynı
  tamsayı margin) istek de kare de üretmez, gönderilene dönmek bekleyeni iptal eder. İlk tampondan
  önce istek doğrudan uygulanır, ilk karenin commit'ine biner. Boyut değişirse compositor configure
  gönderir → `surface_resized`. Pencere/kilit yüzeyinde etkisiz (debug log). Birim testi
  `placement_coalesces_to_the_latest_and_skips_unchanged`.
  **Sürükleme sınırı (bulgu):** basılıyken (örtük kapma) işaretçinin yüzey içi konumunun tabanı
  compositor'a bağlı. sway 1.12 basma anındaki yüzey konumunu tutar: ekran konumu = *basmadaki*
  margin + yerel (yüzey taşınsa da). Tabanı "son gönderilen margin" saymak sway'de her adımda
  hareketi ikiye katladı (yüzey ekrandan çıktı). Hyprland'in tabanı canlı doğrulanmadı; ekran
  klavyesi Hyprland'de doğrulamalı. Pencere taşımada (`begin_move`) sorun yok: onu compositor yapar.
  Kanıt: `examples/layer_drag.rs` izole sway'de 15 adımlık sürükleme → margin (400, 250), yüzey
  ekran görüntüsünde tam orada; 15 çağrı, 32 kare (`NUXUT_DRAG_BASE=current` diğer tabanı dener).
- **Kare akışı.** Yüzey başına `wants_frame` + `callback_pending`. Her olay turunun sonunda
  `configured && wants_frame && !callback_pending` yüzeyler çizilir (aynı turda hepsi aynı
  `now`): `make_current` → `wl_surface.frame` → `Renderer::render` → `eglSwapBuffers`
  (swap interval 0). Callback gelince yeniden istenmişse çizilir; istenmemişse yüzey uyur.
  Configure ve ölçek değişimi kendiliğinden bir kare ister; boyutu değişmeyen layer configure'u
  (tamponu olan yüzeyde) yalnızca tamponsuz commit alır, yeniden çizilmez (compositor'lar bir katman
  durum değiştirince tüm katmanları aynı boyutla yeniden configure eder; ölçüm: shell
  docs/research/perf-2026-10.md, 2026-10-16). `render_frame` sonrası
  `Renderer::has_pending_uploads()` doğruysa (kare yükleme bütçesi doldu) az önce çizilen yüzeye
  bir kare daha istenir; büyük dokular uygulama bir şey yapmadan birkaç karede tamamlanır, bütçe
  dolmadıkça ek kare yok (`examples/uploads.rs`: 3 × 100 MiB → 3 kare, sonra boşta 0 CPU).
- **İlk sunum.** Yüzey başına `Presentation` (Unpresented → Committed → Presented). Gerçek
  tampon gönderen ilk swap (tamponsuz boş-hasar commit'i sayılmaz) `Committed` yapar; ilk
  tampondan önce hiç commit yapılmadığı için sonraki ilk frame callback `done`'u o commit'e
  aittir → `Presented` ve `App::surface_presented(id)` bir kez. Commit'e değil sunuma bağlıdır:
  kilit ekranı (ext-session-lock) "gerçekten gösterildi" durumunu (LockedHint/ready) buna
  bağlar. Layer, pencere, kilit yüzeyinde aynı; yok edilen yüzeyin callback'i haritada
  olmadığından olay üretmez; yeniden yaratılan yüzey yeni girdidir (yeniden bir kez). Ek kare,
  timer ya da uyanma yok (callback zaten karenin kendi callback'i). Compositor görünmeyen
  yüzeyde `done`'u geciktirebilir; uygulama gerekiyorsa kendi zaman aşımını koyar.
- **Açılış.** `TextSystem::new` (font taraması) EGL init'ten hemen sonra `nuxut-fonts`
  thread'inde başlar (configure beklemesi ve `Renderer::new` ile çakışır; EGL ile aynı anda
  başlatmak yük altında uzun kuyruklar verdi); ilk draw ya da ilk `cx.text()` bekler.
  Uygulama yüzeylerini yaratıp configure beklerken istekler flush edilir ve `Renderer::new`
  yüzeysiz context'te kurulur (shader derleme configure gidiş-dönüşüyle çakışır).
  `NUXUT_PROFILE_STARTUP=1` aşama sürelerini ilk karede `info` olarak loglar.
- **Hasar.** Yüzey başına `DamageHistory`; `make_current`'tan sonra `EGL_EXT_buffer_age` varsa
  tamponun yaşı okunur (yoksa 0 = tüm kare) ve `Renderer::render_frame` çağrılır. Dönen
  `Repaint` bu karenin hasarıdır: `Partial(region)` → `eglSwapBuffersWithDamage{KHR,EXT}` her
  ayrık parça bir dikdörtgen (Mesa her birini ayrı `wl_surface.damage_buffer` yapar; eklenti
  yoksa normal swap), `Full` → normal swap, boş `Partial` (hiçbir şey değişmedi) → tampon
  gönderilmez, yalnızca `wl_surface.commit` (frame callback ve bekleyen durum yine iletilir).
  Eklentiler EGL display'in extension dizgesinden tespit edilir. `NUXUT_DEBUG_SURFACES` açıkken
  her kare `Full` gönderilir. Geçmiş **swap başına** tutulur: boş hasarlı (swap'sız) kare
  geçmişe yazılmaz (yazılsaydı yaş hesabı kayar, eski tampondaki bölge yeniden çizilmeden
  kalırdı); yeni boyutta uygulamanın bildirdiği hasar yok sayılır, tüm kare. Kare zamanlaması:
  `RUST_LOG=nuxut::frame=trace` (yaş, bölge, kayıt/GL/sunum µs, yol).
- **Yazılım sunumu (`shm` modülü).** GL_RENDERER bir yazılım rasterleştiricisiyse (llvmpipe,
  softpipe, swrast; `NUXUT_SOFTWARE_PRESENT=1` zorlar, `0` kapatır) yüzeyler EGL pencere
  yüzeyi kullanmaz: context yüzeysiz bağlanır, her yüzey kendi RGBA8 FBO'suna çizer (FBO hep
  son kareyi taşır → çizim yaşı 1, yeni/yeniden boyutlu FBO 0), değişen bölge `glReadPixels`
  ile kendi `wl_shm` tamponlarımıza (ARGB8888, memfd + mmap, en çok 3, `wl_buffer.release`
  ile meşgul takibi) parça parça kopyalanır: kopyalanan bölge tamponun yaşına göre birikmiş hasar (ikinci
  `DamageHistory`), compositor'a parça başına `wl_surface.damage_buffer` ile yalnızca bu
  karenin hasarı.
  Tüm tamponlar meşgulse tamponsuz commit + bir kare daha; sonraki sunum tüm kareyi kopyalar.
  Neden: Mesa'nın Wayland swrast yolunda swap maliyeti hasardan bağımsız, yüzey alanıyla
  ölçekleniyor (tüm tampon kopyası; ölçüm): 1920×1080 yüzeyde 74×30 saat tıkı ~4 ms CPU
  idi, bu yolla ~0,8 ms (`damage_bench`).
  Karar ilk çizimde bir kez verilir; yüzeysiz context ya da `wl_shm` yoksa EGL yolu. GPU
  sürücülerinde değişiklik yok.
- **`LP_NUM_THREADS` ayarlanmaz (karar, 2026-10-16).** llvmpipe varsayılanda
  çekirdek sayısı kadar rasterleştirici iş parçacığı açar (6 çekirdekli VM'de süreç 26 iş
  parçacığı; `LP_NUM_THREADS=1/2/4` → 4/6/10). `damage_bench` (release, izole sway,
  1920×1080, 16 ms tik × 300, yük ~25, iki tekrar): saat tıkı (shell'in olağan karesi)
  varsayılan 1,07–1,43 ms CPU/kare, 1/2/4 iş parçacığı 0,87–1,10 ms — kare başına ≤ 0,4 ms;
  tüm yüzey karesi varsayılan ~125 ms CPU / ~38 ms duvar, `1` ~112 ms CPU ama ~130 ms duvar,
  `2` ~95 ms CPU / ~50 ms duvar. Az iş parçacığı CPU'yu biraz düşürür ama büyük karelerde
  (panel açılışı, tam yeniden çizim) gecikmeyi 1,5–3× artırır: akıcılık önce gelir. Boşta
  kare yok, uyuyan iş parçacığı CPU harcamaz. Ayrıca Mesa değeri `eglInitialize`'da ortamdan
  okur: süreç içinden ayarlamak çok iş parçacıklı süreçte `setenv` (Rust 2024'te `unsafe`,
  C `getenv` ile yarış) ister, süreç dışından vermek başlatılan uygulamalara sızar. Yalnızca
  yazılım rasterleştiricide (VM/yedek) geçerli; isteyen kullanıcı oturum ortamında verebilir.
- **Ölçek.** `wp_fractional_scale_v1` + `wp_viewporter` varsa buffer scale 1, tampon
  `round(mantıksal × ölçek)`, viewport hedefi mantıksal boyut. Yoksa tamsayı
  `wl_surface.preferred_buffer_scale` / çıkış ölçeği ve `set_buffer_scale`.
- **Input bölgesi, exclusive zone, klavye modu** hemen commit edilir (kare beklemez).
  `set_input_region` dikdörtgenleri **mantıksal**, yüzeye göredir; runtime her birini dışa
  doğru tam sayıya yuvarlar (`Rect::round_out`, compositor bölgeleri mantıksal tamsayı
  koordinatta alır). Boş dilim = tüm tıklamalar geçer; hiç çağrılmazsa tüm yüzey tıklama alır.
- **Sıfır boyut.** `size` ekseninde 0, o eksende karşılıklı iki anchor varsa "geril" demektir.
  Karşılıklı anchor yokken 0 protokol hatasıdır (tüm bağlantı düşer); runtime yüzeyi
  reddetmez, o ekseni 1'e yükseltip uyarı loglar.
- **Config.** `framework.toml`'u runtime kendisi yükler; bozuk dosyada önceki tema korunur,
  değiştiyse `App::theme_changed` (`[locale]` değiştiyse `nuxut_i18n::configure` + `App::locale_changed`;
  kullanıcı çeviri dizini değişince de `reload` + `App::locale_changed`, bkz. i18n kural 6;
  yalnızca dil değiştiyse `theme_changed` gelmez). `watch_config(name)` ile izlenen diğer dosyalar için
  (`config_changed("shell")`) runtime **yalnızca haber verir**: yükleme, doğrulama ve bozuk
  dosyada son geçerli config'i koruma çağıranın işidir.
- **Yapı değişikliği `draw` içinde** (yüzey yarat/yok et, input bölgesi): debug'da
  `debug_assert`, release'te loglanıp yok sayılır.
- **Config izleme.** Tek inotify (`nuxut_config::ConfigWatcher`) calloop'a bağlı; `framework`
  her zaman izlenir → `Theme::load` → değiştiyse `App::theme_changed`. Bozuk dosyada önceki tema
  korunur ve uyarı loglanır. `watch_config(name)` adı listeye ekler (izleyici yeniden kurulur).
  Üretilmiş palet (`nuxut_theme::palette_path()`, `$XDG_STATE_HOME/nuxut/palette.toml`) ayrı bir
  inotify ile izlenir (dizin yoksa oluşturulur; üretici atomik rename ile yazar → `MOVED_TO`);
  değişince aynı yol: `Theme::load` (iki katman) → değiştiyse `App::theme_changed`.
- **Exclusion yüzeyi örneği** (pencereleri ekran kenarından itmek için görünmez şerit; her
  kenar için bir tane):

  ```rust
  // Üst kenar, kalınlık t: üste ve iki yana yapışık, genişlik gerilir.
  LayerSurfaceSpec { anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                     size: (0, t), exclusive_zone: t as i32, ..LayerSurfaceSpec::new("exclusion", out) }
  // Alt: BOTTOM | LEFT | RIGHT, (0, t).  Sol: LEFT | TOP | BOTTOM, (t, 0).  Sağ: RIGHT | TOP | BOTTOM, (t, 0).
  // Görünmez olduğundan: set_input_region(id, &[]) ve draw boş.
  ```

  `t == 0` olan kenar için yüzey yaratılmaz (ya da `exclusive_zone = 0`); runtime 0 kalınlığı
  yukarıdaki kuralla 1'e yükseltir, bu da 1 px'lik görünmez bir şerit ve 1 px itme demektir.
- **Klavye odağı.** sctk `enter` → `keyboard_focus(id, true)`, `leave` → `false`. Leave'de
  basılı tuşun tekrarı durur (sctk tekrar durumunu temizler). Yok edilmiş yüzey için çağrı
  yapılmaz; odaktaki yüzey yok edilirse odak sessizce düşer, `keyboard` olayları da gelmez.
  Hyprland `OnDemand` yüzeye eşlenince odağı hemen verebilir; uygulama ilk `true`'yu buna göre
  yorumlamalı.
- **Keymap'siz odak.** sctk, keymap gelmeden gelen `enter`'ı ve tuşları yutar. Runtime aynı
  koltuktan ikinci, gözlemci bir `wl_keyboard` açar (`keyfocus.rs`): keymap gelince yutulan
  odak `keyboard_focus(true)` ile yeniden uygulanır; keymap hiç gelmeden tuş basılırsa sctk'ye
  yerleşik yedek keymap (`us`, xkbcommon) verilir, bir kez `warn` loglanır (yalnızca düzen adı)
  ve yutulan tuşlar sırayla yeniden oynatılır. Kilit ekranı böylece en azından ASCII parolayla
  açılabilir.
- **Sinyaller.** SIGTERM/SIGINT/SIGHUP calloop `Signals` (signalfd) ile `App::terminate`'e
  gelir; varsayılan `quit` → `run` normal döner, yüzeyler/EGL kapanır, sonra `App` düşer (Drop).
  Sinyal maskesi `run`'ın **ilk adımında**, Wayland bağlantısı ve EGL'den (Mesa'nın kendi
  thread'lerinden) önce, olay döngüsü thread'inde kurulur; sonradan yaratılan thread'ler maskeyi
  miras alır. `run`'dan **önce** spawn edilmiş thread'ler maskesizdir: sinyal onlara düşerse
  süreç varsayılan eylemle (temizlik yapmadan) ölür. Arka plan thread'lerini olay
  işleyicilerinde (ör. ilk `output_added`) başlatın ya da kendiniz maskeleyin.
- **İmleç.** `cx.set_cursor(surface, CursorShape)` (kurallar DESIGN.md "İmleç") yüzey başına son
  şekli saklar; varsayılan `Default`. İşaretçi o yüzeydeyse hemen uygulanır, aynı şekil tekrar
  istenirse protokol çağrısı yapılmaz; başka yüzeydeyse yalnızca saklanır. İşaretçi yüzeye her
  girdiğinde (compositor imleci sıfırladığı için) son şekil enter serial'ıyla yeniden uygulanır.
  Yol: `wp_cursor_shape_v1` varsa `set_shape` (CSS adları birebir); yoksa sctk `ThemedPointer` ile
  xcursor teması (`XCURSOR_THEME`/`XCURSOR_SIZE`, yüzeyin tamsayı ölçeği; ad `cursor-icon`
  CSS adı + yedek adlar = `CursorShape::xcursor_name()` ile aynı). Layer yüzeyleri ve pencereler
  aynı yoldan.
- **Hata.** Bağlantı kopması/protokol hatası `RunError::Disconnected`; EGL yüzeyleri
  `wl_surface`'lerden, EGL display'i bağlantıdan önce yok edilir.
- **Hata ayıklama.** `NUXUT_DEBUG_SURFACES=1` her yüzeyin sınırına eflatun çerçeve çizer.

### Tek örnek + komut IPC (Faz 2)

Uygulama başına bir Unix socket: `$XDG_RUNTIME_DIR/nuxut/<app>.sock`. Kilit:
`$XDG_RUNTIME_DIR/nuxut/<app>.lock` (`File::try_lock`, birincil örnek ömrü boyunca tutar).
Komutlar **asla** shell'e verilmez (değişmez 8); runtime satırı argümanlara böler, uygulama
argüman vektörünü kendisi yorumlar.

```rust
pub mod ipc {
    /// `app`: `[a-z0-9_-]{1,32}`.
    pub fn try_single_instance(app: &str) -> Result<Instance, IpcError>;
    pub enum Instance {
        Primary(Listener),   // kilit bizde; socket bağlandı
        Secondary(Client),   // başka bir örnek çalışıyor
    }
    pub struct Listener;     // run_with'e verilir; Drop'ta socket'i (hâlâ bizimse) siler
    impl Listener {
        /// Sunucu yanıt penceresi: Reply::send bağlantı kabulünden bu süre içinde yazmalı,
        /// sonrası düşer (istemci boş yanıt alır). MIN_REPLY..=MAX_REPLY'ye kırpılır;
        /// varsayılan TIMEOUT. İstek satırını okuma yine en çok min(pencere, TIMEOUT).
        /// Uzun komutlar (ör. shell ekran doğrulaması ~2 sn+) için; istemciler
        /// Client::with_timeout ile en az pencere kadar beklemeli.
        pub fn with_reply_timeout(self, timeout: Duration) -> Listener;
        pub fn reply_timeout(&self) -> Duration;
    }
    pub struct Client;       // Secondary'den: birincil henüz bind etmediyse ≤ 500 ms yeniden dener
    pub const TIMEOUT: Duration;            // 2 sn: varsayılan yanıt penceresi + istemci varsayılanı
    pub const MIN_CLIENT_TIMEOUT: Duration; // 10 ms
    pub const MAX_CLIENT_TIMEOUT: Duration; // 30 sn
    pub const MIN_REPLY_TIMEOUT: Duration;  // = MIN_CLIENT_TIMEOUT
    pub const MAX_REPLY_TIMEOUT: Duration;  // = MAX_CLIENT_TIMEOUT
    impl Client {
        pub fn new(app: &str) -> Result<Client, IpcError>;  // yeniden deneme yok (CLI)
        /// İstek başına (bağlan + yaz + oku) süre; MIN..=MAX aralığına kırpılır. Sunucunun
        /// yanıt penceresini (Listener::with_reply_timeout) uzatmaz.
        pub fn with_timeout(self, timeout: Duration) -> Client;
        pub fn timeout(&self) -> Duration;
        pub fn send(&self, line: &str) -> Result<String, IpcError>;
        pub fn send_args(&self, args: &[impl AsRef<str>]) -> Result<String, IpcError>;
    }
    /// CLI'lar için (`nuxut-shell msg panel toggle right`): bağlan, gönder, yanıtı oku.
    pub fn send_args(app: &str, args: &[impl AsRef<str>]) -> Result<String, IpcError>;
    /// Ham satır (argümanlar zaten `\t` ile ayrılmış).
    pub fn send(app: &str, line: &str) -> Result<String, IpcError>;

    /// Bir komutun yanıt kanalı. `Send`: başka thread'den de yanıtlanabilir. Hemen ya da
    /// zaman aşımına kadar yanıtlanır; yanıtsız düşerse boş yanıt gider.
    pub struct Reply;
    impl Reply { pub fn send(self, text: &str); }

    pub enum IpcError { InvalidName, InvalidArgument, NoRuntimeDir, Insecure(String),
                        NotRunning, Timeout, TooLong, Io(std::io::Error) }
}

pub struct RunOptions { pub ipc: Option<ipc::Listener> }
pub fn run_with<A: App>(app: A, options: RunOptions) -> Result<(), RunError>;  // run(app) = varsayılan

// App:
/// Başka bir süreçten gelen komut. Boş satır → `args = []`.
fn command(&mut self, cx: &mut Context<Self>, args: Vec<String>, reply: ipc::Reply) {
    reply.send("error: komut desteklenmiyor");
}
```

Protokol: bağlantı başına **tek istek**. İstemci tek satır yazar: argümanlar `\t` (0x09) ile
ayrılır, satır `\n` ile biter, en çok 4 KiB, UTF-8. Argümanda `\t`, `\n` ya da NUL varsa istemci
`IpcError::InvalidArgument` döner (gönderilmez). Sunucu yanıt metnini yazıp bağlantıyı kapatır
(yanıt serbest metin, en çok 64 KiB; EOF yanıtın sonudur). Hata kuralı uygulamanındır (öneri:
`error: …` ile başlayan yanıt).

Güvenlik ve sınırlar:
- Dizin `0700`, socket ve kilit `0600`. Dizin symlink değil, sahibi bu süreç (`/proc/self`'in
  uid'i) ve grup/diğer izni yoksa kullanılır; aksi `IpcError::Insecure` (düzeltmeye kalkışılmaz).
- Kilit bizdeyse eski socket dosyası (yalnızca bizim uid'imize ait bir **socket** ise) silinip
  yeniden bağlanır. Kilit başkasındaysa `Secondary`.
- Sunucu: calloop'a bağlı, engellemesiz. Eşzamanlı en çok 16 bağlantı (bekleyen `Reply`'ler
  dahil; fazlası hemen kapatılır); bağlantı başına 2 sn zaman aşımı (satırı okuma + yanıt). 4 KiB'ı
  aşan ya da UTF-8 olmayan satır → `error: …` yanıtı, `App::command` çağrılmaz. Zaman aşımından
  sonra gelen `Reply::send` sessizce düşer.
- İstemci: bağlan + yaz + oku toplam 2 sn zaman aşımı.

### Pencereler — xdg_toplevel (Faz 2)

Normal uygulama pencereleri (Ayarlar vb.) layer yüzeyleriyle aynı `App` yolunu kullanır:
`surface_resized`, `draw`, `pointer`, `keyboard`, `keyboard_focus` aynı `SurfaceId` ile gelir.

```rust
pub struct WindowSpec {
    pub title: String,
    pub app_id: String,                 // Hyprland pencere kuralları / .desktop eşlemesi
    pub size: (u32, u32),               // mantıksal başlangıç boyutu (compositor değiştirebilir)
    pub min_size: Option<(u32, u32)>,
    pub max_size: Option<(u32, u32)>,
}
impl Default for WindowSpec { /* boş başlık/app_id, 800x600, min/max None */ }
impl WindowSpec { pub fn new(title, app_id) -> Self; }   // ..Default
pub enum ResizeEdge { Top, Bottom, Left, Right, TopLeft, TopRight, BottomLeft, BottomRight }
pub struct WindowState { pub maximized: bool, pub fullscreen: bool, pub activated: bool,
                         pub tiled: bool /* herhangi bir kenarı döşenmiş */ }

impl<A: App> Context<A> {
    pub fn create_window(&mut self, spec: WindowSpec) -> SurfaceId;
    pub fn set_window_title(&mut self, id: SurfaceId, title: &str);
    /// Çalışan pencerede app-id (hemen geçerli). Layer/kilit yüzeyinde etkisiz (debug log).
    pub fn set_app_id(&mut self, id: SurfaceId, app_id: &str);
    /// Mantıksal en küçük/en büyük boyut; `None` ya da 0 eksen = sınır yok. Sabit boyut:
    /// min == max. Çift tamponlu: çizilmiş pencerede hemen commit, yoksa ilk karede.
    /// Layer/kilit yüzeyinde etkisiz (debug log).
    pub fn set_min_size(&mut self, id: SurfaceId, size: Option<(u32, u32)>);
    pub fn set_max_size(&mut self, id: SurfaceId, size: Option<(u32, u32)>);
    /// Yaşayan xdg_toplevel sayısı (son pencere kararı için).
    pub fn window_count(&self) -> usize;
    /// Son işaretçi basma serial'ı ile etkileşimli taşıma/boyutlandırma başlatır
    /// (başlık çubuğunda `Press` işleyicisinden çağrılır; serial yoksa yok sayılır).
    pub fn begin_move(&mut self, id: SurfaceId);
    pub fn begin_resize(&mut self, id: SurfaceId, edge: ResizeEdge);
    pub fn set_maximized(&mut self, id: SurfaceId, maximized: bool);
    pub fn set_minimized(&mut self, id: SurfaceId);
    pub fn window_state(&self, id: SurfaceId) -> Option<WindowState>;
}

// App:
/// Compositor pencereyi kapatmak istedi (ör. killactive). Varsayılan: yüzeyi yok et.
fn close_requested(&mut self, cx: &mut Context<Self>, surface: SurfaceId) { cx.destroy_surface(surface) }
/// Pencere durumu değişti (maximize/activated/tiled…). Boyut ayrıca `surface_resized` ile gelir.
fn window_state_changed(&mut self, cx: &mut Context<Self>, surface: SurfaceId, state: WindowState) {}
```

Davranış:
- `zxdg_decoration_manager_v1` varsa server-side dekorasyon istenir; yoksa yok sayılır. Runtime
  CSD çizmez: başlık çubuğu ve kenarlar uygulamanındır (`begin_move`/`begin_resize`).
- Configure boyutu 0 ise (compositor serbest bırakır) son boyut, ilk configure'da
  `spec.size` kullanılır. Kesirli ölçek, hasar, kare akışı layer yüzeyleriyle aynıdır.
- `set_exclusive_zone`/`set_keyboard_mode` pencerelerde etkisizdir (debug log).
- Hyprland notu: yüzen pencerelerde de `tiled` ve (ilk boyutlandırmadan sonra) `maximized`
  bildirir (istemci gölge/CSD çizmesin diye); uygulama kenar boyutlandırmasını bunlara göre
  kapatmamalı, yalnızca `fullscreen`'e bakmalı. Yüzen pencerenin ilk boyutunu compositor seçer
  (`min_size` olabilir). `begin_move`/`begin_resize` canlıda doğrulandı (`examples/window.rs`).
- Pencere kapatılınca (`destroy_surface`) `xdg_toplevel` → `xdg_surface` → `wl_surface` sırasıyla
  yok edilir; son pencere kapandığında çıkmak uygulamanın kararıdır (`cx.quit()`).
- Boyut ipuçları doğrulanır: protokol bir eksende `0 < max < min`'i ve i32'ye sığmayan değeri
  `invalid_size` hatasıyla cezalandırır (tüm bağlantı düşer). Değerler `i32::MAX`'e kırpılır;
  çakışmada son çağrılan kazanır (`set_max_size` küçükse `min` ona iner, `set_min_size`
  büyükse `max` ona çıkar; `WindowSpec`'te `max` kazanır), uyarı loglanır.

**Çoklu pencere (tek süreç).** Bir uygulama istediği kadar `create_window` çağırabilir (ör.
Ayarlar + ayrı "Hoş geldin" penceresi); her pencere bağımsız bir yüzeydir:
- Boyut/configure, kesirli ölçek (`wp_fractional_scale` yüzey başına), hasar geçmişi, kare akışı
  (frame callback), EGL yüzeyi (swap interval yüzey başına 0) ya da shm tamponları ayrıdır.
- İşaretçi olayları üzerindeki yüzeyin, tuşlar klavye odağındaki yüzeyin `SurfaceId`'siyle gelir.
  Odak geçişinde önce eskiye `keyboard_focus(eski, false)`, sonra yeniye `(yeni, true)`; IME ve
  imleç şekli de yüzey başınadır. `last_press` (taşıma/boyutlandırma serial'ı) koltuk başınadır.
- `close_requested` (varsayılan: `destroy_surface`) ve `destroy_surface` yalnızca o pencereyi
  kapatır; diğer pencereler ve süreç yaşar. Odaktaki pencere yok edilirse ona
  `keyboard_focus(_, false)` GELMEZ (yüzey artık yok); compositor odağı başka pencereye verince
  o pencere `true` alır. `keyboard_focus(id, false)` öncesinde `true` gelmemiş olabilir
  (keymap'siz enter'ı sctk yutar, leave yine bildirilir); uygulama bunu idempotent işler.
- Kapatma olayı (`close_requested(id)`) her zaman compositor'ın kapatmak istediği
  `xdg_toplevel`'ın `SurfaceId`'sini taşır; kapatılan pencerenin EGL yüzeyi/shm tamponları ve
  FBO'ları tur sonunda bırakılır (`destroy_surface` → mezarlık → `reap_surfaces`), diğer
  pencerelerin çizimi kesilmez. Kimlikler yeniden kullanılmaz: yeniden açılan pencere yeni
  `SurfaceId` alır.
- **Son pencere kapandığında runtime çıkmaz** (layer yüzeyi olmayan uygulama görünmez hâlde
  çalışmayı sürdürür). Pencere uygulaması bunu kendisi seçer:
  ```rust
  fn close_requested(&mut self, cx: &mut Context<Self>, id: SurfaceId) {
      cx.destroy_surface(id);
      if cx.window_count() == 0 { cx.quit(); }
  }
  ```
- Sabit boyutlu ikinci pencere: `WindowSpec { size: (720, 640), min_size: Some((720, 640)),
  max_size: Some((720, 640)), ..WindowSpec::new(başlık, "nuxut-welcome") }`. Sway min == max
  pencereyi kendiliğinden yüzdürür; Hyprland'de `windowrule = float, class:^(nuxut-welcome)$`.
- Canlıda doğrulandı (`examples/two_windows.rs`, headless sway, ölçek 1,5, shm ve EGL sunum
  yolu): odak geçişi/tuş yönlendirmesi, compositor `kill` ve uygulama içi kapatmada yalnızca o
  pencerenin kapanması, yeniden açma, `set_app_id`, son pencerede kendiliğinden çıkış.
  `swaymsg -t get_tree`: `nuxut-two-windows` döşeli (1920×1080; ölçek 1,5'te 1280×720),
  `nuxut-welcome` yüzen 720×640 (her iki ölçekte). İki pencere açıkken boşta 3 sn'de 0 CPU tiki
  (ölçek 1 ve 1,5). `DEMO_CYCLE_MS=150 DEMO_CYCLES=60` ile 30 kapatma sonrasında canlı yüzey 1,
  EGL yüzeyi 1 (shm yolunda 0), doku 0 — sızıntı yok.

### Metin girişi / IME (Faz 2)

`zwp_text_input_v3` ile giriş yöntemi (fcitx5 vb.) desteği. IME yoksa ya da compositor
protokolü sunmuyorsa her şey sessizce no-op'tur; tuşlar normal `KeyEvent.text` ile gelir.

```rust
pub struct TextInputState {
    pub cursor_rect: Rect,                   // mantıksal, yüzeye göre (aday penceresi konumu)
    pub surrounding: Option<SurroundingText>,
    pub purpose: TextPurpose,                // Normal | Password | Email | Url | Number | Phone | Terminal
    pub multiline: bool,
}
pub struct SurroundingText { pub text: String /* ≤ 4000 bayt */, pub cursor: usize, pub anchor: usize } // bayt
pub enum ImeEvent {
    Preedit { text: SecretText, cursor: Option<(usize, usize)> },   // bayt aralığı; boş text = preedit bitti
    Commit(SecretText),                                            // gizli alanda preedit hiç gelmez
    DeleteSurrounding { before: usize, after: usize },          // imlecin önü/arkası, bayt
}

impl<A: App> Context<A> {
    /// `Some`: bu yüzey odaktayken metin girişi etkin, durum IME'ye gönderilir (değişince yeniden
    /// çağrılır). `None`: devre dışı.
    pub fn set_text_input(&mut self, id: SurfaceId, state: Option<TextInputState>);
    /// Etkin xkb düzeninin adı ("Turkish", "English (US)"…); klavye yoksa `None`.
    pub fn keyboard_layout(&self) -> Option<&str>;
}

// App:
fn ime(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: ImeEvent) {}
/// Etkin klavye düzeni değişti (`cx.keyboard_layout()`).
fn keyboard_layout_changed(&mut self, cx: &mut Context<Self>) {}
```

Davranış: text-input'un kendi `enter`/`leave`'inde, durumu `Some` olan yüzey için
`enable` + durum + `commit` gönderilir. Bir `done` olayındaki değişiklikler protokol sırasıyla
verilir: önce `DeleteSurrounding`, sonra `Commit`, sonra `Preedit`. Metin uzunlukları sınırlıdır
(surrounding 4000 bayt, preedit/commit 4 KiB; fazlası kesilir, karakter sınırına göre).

### Oturum kilidi — ext-session-lock-v1 (güvenlik kritik)

Kilit ekranı için. Protokol yoksa `Unsupported`; ekran kilitlenmez (çağıran başka bir
yola düşmemeli, hatayı göstermeli).

```rust
pub enum LockError { Unsupported, AlreadyActive, NotLocked, InDraw /* draw içinde çağrıldı */ }
pub enum LockState { Unlocked, Pending /* istendi, `locked` bekleniyor */, Locked }
pub enum LockEnd {
    Refused,          // `locked` hiç gelmedi (reddedildi / başka kilitleyici)
    CompositorEnded,  // `Locked` iken compositor `finished` gönderdi
    Unlocked,         // `unlock_session` sonrası
}

impl<A: App> Context<A> {
    /// Kilit ister. Başarı = istek gönderildi; kilit ANCAK `App::session_locked` ile kurulur.
    pub fn lock_session(&mut self) -> Result<(), LockError>;
    /// Yalnızca `Locked` durumunda: `unlock_and_destroy` gönderilir, `wl_display.sync` ile
    /// compositor'ın işlediği BEKLENİR (ardından `quit` güvenli), sonra kilit yüzeyleri
    /// kaldırılır. Aksi `Err(NotLocked)` (loglanır). Runtime kendiliğinden ASLA çağırmaz.
    pub fn unlock_session(&mut self) -> Result<(), LockError>;
    pub fn lock_state(&self) -> LockState;
    /// Bu yüzey bir kilit yüzeyi mi (çizim/girdi yönlendirmesi için).
    pub fn is_lock_surface(&self, id: SurfaceId) -> bool;
}

// App:
/// Kilit yüzeyi yaratıldı (kilit isteğinde her çıkış için; Pending/Locked iken eklenen çıkış
/// için de). Aynı draw/pointer/keyboard/keyboard_focus/set_text_input yolu.
fn lock_surface_added(&mut self, cx: &mut Context<Self>, surface: SurfaceId, output: OutputId) {}
/// Compositor `locked` gönderdi: oturum kilitli, uygulama ancak şimdi "kilitlendi"ye geçer.
fn session_locked(&mut self, cx: &mut Context<Self>) {}
/// Kilit kurulmadı ya da sona erdi; kilit yüzeyleri kaldırılmıştır. `end` runtime'ın kesin
/// bilgisidir (`CompositorEnded`'da `nuxut-lock` parola alanını sıfırlar, UI'ı kapatır).
fn lock_finished(&mut self, cx: &mut Context<Self>, end: LockEnd) {}
```

Kurallar:
1. **`locked` gelmeden kilit başarılı sayılmaz.** `lock_session` yalnızca isteği gönderir;
   `lock_state()` `Pending` olur. Uygulama "kilitli" UI/durumuna yalnızca `session_locked` ile
   geçer.
2. **İlk kare hemen, opak.** Compositor kilit yüzeyleri ilk tamponu alana dek ekranı (siyah)
   gösterir; runtime her kilit yüzeyinin ilk configure'ında kare ister. Kilit yüzeyinin tamponu
   **opak olmalı** (alfa 1): yarı saydam bir kare kilitli oturumun altını gösterebilir; uygulama
   tüm yüzeyi opak zeminle boyar. Savunma derinliği: runtime kilit yüzeyine tam yüzey opak bölge (`set_opaque_region`) verir ve her karede, uygulama çizdikten sonra tamponun alfasını 1 yapar (renkler önceden çarpılmış: hiç boyanmamış yer opak siyah, yarı saydam boya siyah üzerine bindirilmiş görünür); uygulama yanlışlıkla saydam boyasa da alttaki oturum görünmez. Protokol gereği kilit yüzeyine configure'dan önce ve hiçbir
   zaman boş tamponla commit gönderilmez: runtime çift tamponlu durumu (input bölgesi vb.)
   ilk tampon gönderilene dek bekletir.
3. **Çökme / çıkış güvenli tarafta kalır.** Süreç çökerse ya da `quit`/SIGTERM ile kilitliyken
   çıkarsa compositor oturumu kilitli tutar (kilit nesnesi `unlock_and_destroy` olmadan yok
   edilirse oturum kilitli kalır). Runtime kapanışta kilidi açmaz; `Locked` iken düzenli
   çıkışta kilit nesnesine hiçbir istek göndermez (`destroy` `locked`'tan sonra protokol
   hatasıdır), bağlantının kapanması yeter. Panik/çökmede gönderilen `destroy` protokol hatasıyla
   bağlantıyı düşürür; sonuç yine kilitli oturumdur. Açmanın tek yolu `unlock_session`.
4. **Girdi.** Kilitliyken compositor girdiyi yalnızca kilit yüzeylerine verir; runtime ek olarak
   (`Pending` dahil) klavye, değiştirici, IME ve işaretçi olaylarını (işaretçi `Leave` hariç) ve
   `keyboard_focus(true)`'yu kilit yüzeyi dışındaki bir yüzeye vermez (savunma derinliği). `keyboard_focus` ve `set_text_input` (Password amacı dahil) kilit yüzeylerinde
   diğer yüzeylerle aynı çalışır. Layer yüzeyleri ve pencereler var olmaya devam eder ama
   görünmez/girdisizdir.
5. **Çıkışlar.** Pending/Locked iken eklenen çıkışa runtime yeni kilit yüzeyi yaratır
   (`lock_surface_added`); çıkış kalkınca yüzeyi yok eder (`output_removed` her zamanki gibi).
6. **Tek kilit ve bitiş.** `Pending`/`Locked` iken ikinci `lock_session` → `AlreadyActive`. `lock_session` ve `unlock_session` `draw` içinden çağrılamaz (yüzey haritası çizim sırasında değişmez): debug derlemede assert, sürümde `Err(InDraw)`.
   Compositor `finished` gönderirse: `locked` ALINDIYSA `unlock_and_destroy`, alınmadıysa
   `destroy` gönderilir (protokol); durum `Unlocked`, yüzeyler kaldırılır,
   `lock_finished(Refused | CompositorEnded)`. `finished`, `Locked`'tan sonra da gelebilir
   (compositor kilidi kendisi bitirir).
7. **Sıra garantisi.** `unlock_session` döndüğünde `unlock_and_destroy` compositor'a ulaşmış ve
   işlenmiştir (`wl_display.sync` beklenir); hemen ardından `cx.quit()` güvenlidir. Kabul edilen sınır: bu bekleme zaman aşımsızdır; compositor donarsa `unlock_session` (ve olay döngüsü) onunla birlikte bloklanır (donmuş compositor zaten kilidi açamaz; güvenli taraf).
8. **Loglar** (değişmez 9): yalnızca durum geçişleri ve kilit yüzeyi/çıkış sayısı.
9. **Test.** Yalnızca `tools/test-compositor.sh` (sway, ext-session-lock destekli) içinde; kullanıcının
   oturumu ASLA kilitlenmez.

### Etkinleştirme belirteci — xdg-activation-v1

Başlatıcı bir uygulamayı başlatırken compositor'dan belirteç alır ve onu
`XDG_ACTIVATION_TOKEN` ile çocuğa verir; böylece yeni pencere odak çalma önleyicisine
takılmadan öne gelir.

```rust
// nuxut_wayland::{…} ve kök crate'ten: nuxut::{ActivationRequestId, ActivationToken}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActivationRequestId(/* runtime'a özel */);

/// Compositor'ın verdiği belirteç. Clone yok (tek kullanımlık; kopyası sızmaz), Debug
/// redakte ("ActivationToken(<gizli>)"), hiçbir log seviyesinde yazılmaz.
pub struct ActivationToken(/* String */);
impl ActivationToken {
    pub fn as_str(&self) -> &str;
    pub fn into_string(self) -> String;
}

impl<A: App> Context<A> {
    /// Belirteç ister. Runtime `surface`'ı, son girdinin (tuş basma ya da işaretçi basma)
    /// serial'ini ve koltuğunu kendisi koyar; `app_id` verilirse iletilir.
    pub fn request_activation_token(
        &mut self,
        surface: SurfaceId,
        app_id: Option<&str>,
    ) -> ActivationRequestId;
}

// App:
/// İsteğin sonucu; her istek için TAM BİR KEZ çağrılır.
fn activation_token(
    &mut self,
    cx: &mut Context<Self>,
    id: ActivationRequestId,
    token: Option<ActivationToken>,
) {}
```

Kurallar:
1. **Hemen `None`.** Compositor protokolü sunmuyorsa, `surface` yoksa ya da oturum kilidi
   `Pending`/`Locked` ise istek gönderilmez; `activation_token(id, None)` çağıran işleyici
   bittikten sonra, aynı olay turunun sonunda gelir (çağrının içinden değil).
2. **Serial.** Son girdi yoksa istek yine gönderilir (serial'siz); compositor geçersiz/zayıf
   belirteç verebilir, bu istemciden anlaşılamaz. Uygulama isteği girdi olayının hemen
   ardından yapmalı.
3. **Yaşam döngüsü.** `done` gelince belirteç nesnesi yok edilir. İstek sürerken yüzey yok
   edilirse istek yine tamamlanır (compositor yanıtı ne ise).
4. **Loglar** (değişmez 9): belirteç ve `app_id` yazılmaz; yalnızca istek kimliği ve sonuç
   türü (var/yok).

### Pencere dışa aktarma — xdg-foreign-unstable-v2

Başka bir sürecin diyaloğu (xdg-desktop-portal dosya seçicisi) pencerenin üstüne ebeveynli/kipli
açılsın diye `xdg_toplevel` tanıtıcısı. Portal `parent_window` olarak `wayland:<tanıtıcı>` bekler
(`nuxut_portal::ParentWindow::wayland`). (2026-10-13)

```rust
// nuxut_wayland::{…} ve kök crate'ten: nuxut::{ExportRequestId, ToplevelHandle}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExportRequestId(/* runtime'a özel */);

/// Debug redakte ("ToplevelHandle(<gizli>)"); hiçbir log seviyesinde yazılmaz.
#[derive(Clone, PartialEq, Eq)]
pub struct ToplevelHandle(/* String */);
impl ToplevelHandle {
    pub fn as_str(&self) -> &str;
    pub fn window_identifier(&self) -> String;   // "wayland:<tanıtıcı>"
}

impl<A: App> Context<A> {
    pub fn export_toplevel(&mut self, surface: SurfaceId) -> ExportRequestId;
    pub fn unexport_toplevel(&mut self, surface: SurfaceId);
}

// App: her istek için TAM BİR KEZ.
fn toplevel_exported(&mut self, cx: &mut Context<Self>, id: ExportRequestId,
                     surface: SurfaceId, handle: Option<ToplevelHandle>) {}
```

Kurallar:
1. **Yalnız pencere.** `create_window` yüzeyi dışa aktarılır; layer-shell/kilit yüzeyi, bilinmeyen
   yüzey ya da compositor `zxdg_exporter_v2` sunmuyorsa `None` — işleyici bittikten sonra, aynı
   olay turunun sonunda (çağrının içinden değil). Portal o zaman ebeveynsiz (`""`) çağrılır.
2. **Yüzey başına bir dışa aktarma.** Tekrar çağrı aynı tanıtıcıyı döner (gelmediyse bekleyenlere
   eklenir). Tanıtıcı dışa aktarma nesnesi yaşadıkça geçerlidir: `destroy_surface` ya da
   `unexport_toplevel` bırakır; tanıtıcı gelmeden bırakılırsa bekleyen istekler `None` alır.
3. **Loglar** (değişmez 9): tanıtıcı yazılmaz (tanıtıcıyı bilen istemci kendi yüzeyini bu pencereye
   bağlayabilir); yalnızca istek kimliği ve var/yok.
4. Sürüm 1; bağlama `wayland-protocols` `unstable` özelliği. Hyprland ve sway sunar.

**İçe aktarma (yabancı ebeveyn).** Portal arka ucu olan uygulama (Nuxut dosya seçicisi) aldığı
`parent_window = "wayland:<h>"`'den öneki atıp kendi penceresini o pencerenin çocuğu yapar
(`zxdg_importer_v2.import_toplevel` + `zxdg_imported_v2.set_parent_of`; `xdg_toplevel.set_parent`
anlamı — Hyprland ebeveynin üstünde ortalar). (2026-10-15; filechooser talebi)

```rust
impl<A: App> Context<A> {
    /// false: protokol yok, yüzey pencere değil ya da tanıtıcı boş (pencere ebeveynsiz kalır).
    pub fn set_window_parent(&mut self, surface: SurfaceId, handle: &str) -> bool;
    pub fn clear_window_parent(&mut self, surface: SurfaceId);
    pub fn has_window_parent(&self, surface: SurfaceId) -> bool;
}
```

5. **Yüzey başına bir yabancı ebeveyn.** Yeni çağrı öncekini bırakır; `destroy_surface` da bırakır.
6. **`destroyed`** (ebeveyn kapandı, tanıtıcı geçersiz): ilişki düşer, `has_window_parent` `false`
   olur, uygulamaya olay gelmez (pencere ebeveynsiz yaşamaya devam eder). Geçersiz tanıtıcı
   ancak bu olayla anlaşılır; `true` dönüşü "istek gönderildi" demektir.
7. Tanıtıcı hiçbir log seviyesinde yazılmaz (değişmez 9). Örnek: `nuxut-wayland --example
   foreign_parent` (sway'de doğrulandı: ebeveyn kapanınca `destroyed` gelir).

### Pencere listesi ve yakalama — ext-foreign-toplevel-list-v1 / ext-image-copy-capture-v1

Ekran paylaşım seçicisinde pencere önizlemesi, ileride ekran görüntüsü (grim'in yerine) için
**tek kare, talep üzerine** çıkış/pencere yakalama ve bunun için pencere kimliği. Gerekçe, protokol
doğrulaması ve tavanlar `docs/design/capture.md` (2026-10-14; talep kabuk,
onaylı). Kod `toplevels.rs`, `capture.rs`, `capture_pixels.rs`.

```rust
// nuxut_wayland::{…} ve kök crate'ten nuxut::{…}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToplevelId(/* runtime'a özel */);
#[derive(Clone, PartialEq, Eq)]                 // Debug redakte: yalnız uzunluklar
pub struct ToplevelInfo { pub id: ToplevelId, pub app_id: String, pub title: String,
                          pub hyprland_address: Option<u64> }
pub enum ToplevelEvent { Added(ToplevelId), Changed(ToplevelId), Removed(ToplevelId) }
pub enum ToplevelListError { Unsupported }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CaptureId(/* runtime'a özel */);
pub enum CaptureSource { Output(OutputId), Toplevel(ToplevelId) }
pub struct CaptureOptions { pub max_size: (u32, u32), pub paint_cursor: bool, pub timeout: Duration }
impl CaptureOptions { pub fn preview(w: u32, h: u32) -> Self; }  // Default: 8192², imleçsiz, 3 sn
#[derive(Clone, PartialEq, Eq)]                 // Debug redakte: "CapturedImage(WxH ← WxH)"
pub struct CapturedImage { pub image: ImageData /* premultiplied RGBA8, dik */,
                           pub source_size: (u32, u32) }
pub enum CaptureError { Unsupported, UnknownSource, SourceGone, Locked,
                        TooLarge { width: u32, height: u32 }, NoUsableFormat, Busy, Timeout,
                        Failed, Buffer }
pub struct CaptureSupport { pub output: Option<CaptureBackend>, pub toplevel: bool }
pub enum CaptureBackend { ExtImageCopy, WlrScreencopy }
pub const MAX_CAPTURE_BUFFER_BYTES: usize;      // 7680×4320×4

impl<A: App> Context<A> {
    pub fn toplevels_supported(&self) -> bool;
    pub fn toplevels_start(&mut self) -> Result<(), ToplevelListError>;  // tembel; no-op tekrar
    pub fn toplevels_stop(&mut self);                                     // duyurulanlara Removed
    pub fn toplevels(&self) -> impl Iterator<Item = &ToplevelInfo>;
    pub fn toplevel(&self, id: ToplevelId) -> Option<&ToplevelInfo>;
    pub fn toplevel_by_hyprland_address(&self, address: u64) -> Option<ToplevelId>;

    pub fn capture_support(&self) -> CaptureSupport;                     // bağlanmaz
    pub fn capture(&mut self, source: CaptureSource, options: CaptureOptions) -> CaptureId;
    pub fn capture_cancel(&mut self, id: CaptureId);                      // sonucu gelmez
    pub fn set_session_locked_hint(&mut self, locked: bool);              // dış kilit (logind LockedHint)
}
// App:
fn toplevel(&mut self, cx: &mut Context<Self>, event: ToplevelEvent) {}
fn captured(&mut self, cx: &mut Context<Self>, id: CaptureId,
            result: Result<CapturedImage, CaptureError>) {}
```

Sözleşme:

1. **Pencere listesi tembel.** `toplevels_start` çağrılmadıkça `ext_foreign_toplevel_list_v1`'e
   bağlanılmaz, compositor pencere bildirmez. `Added` ilk `done`'dan sonra (Hyprland'de eşleme
   yöneticisi varsa adres yanıtından da sonra) gelir; başlık/uygulama kimliği değişince `Changed`,
   pencere kapanınca ya da liste durunca `Removed`. Metin 4096 bayta, liste 1024 pencereye
   sınırlı. `hyprland_address`: `hyprland_toplevel_mapping_manager_v1` (gömülü XML, Hyprland'e
   özgü) ile `(hi << 32) | lo`; protokol yoksa ya da `failed` → `None`.
2. **Yakalama tek kare.** Her `capture` kendi kaynağını ve oturumunu açar, kare gelince/hata olunca
   hepsini yok eder; sürekli akış yok, boşta iş yok. Yöneticiler ilk yakalamada bağlanır.
   Tercih: ext-image-copy-capture (çıkış ve pencere); çıkış için yedek wlr-screencopy (sürüm ≤ 3;
   çıkışın `wl_output.transform`'u uygulanır). `NUXUT_CAPTURE=wlr` test düğmesi wlr'yi seçtirir.
   Pencere yakalama yalnızca ext ile (hyprland-toplevel-export kullanılmaz; gerekçe tasarım
   belgesinde).
3. **Sonuç tam bir kez**, `App::captured` ile, çağrının içinden değil (aynı olay turunun sonunda
   ya da sonra); `capture_cancel` edilen için hiç. Sonuç dik (compositor `transform`'u ve wlr
   `y_invert` geri alınır), `max_size` kutusuna en-boy korunarak sığdırılmış (büyütülmez,
   kenar ≤ 8192), premultiplied RGBA8 — `Painter::upload_texture`'a doğrudan verilir. Çıkışta alfa
   255; pencerede compositor'ın alfası. Pencere tamponunun ölçeği compositor'a bağlıdır (sway 1.12
   kesirli ölçekte mantıksal boyut verir).
4. **Kaynakta küçültme yok:** tam tampon (`wl_shm`, memfd, salt okunur eşleme; biçim
   ARGB/XRGB/ABGR/XBGR 8888, başkası `NoUsableFormat`) alınır, `nuxut-capture` thread'inde kutu
   filtresiyle küçültülür; sonuç calloop kanalıyla döner. Olay döngüsü piksel işlemez.
5. **Tavanlar:** tampon ≤ `MAX_CAPTURE_BUFFER_BYTES` (aşan `TooLarge`, tampon açılmaz); aynı anda
   en çok 2 iş (oturum + çevirme), **aynı kaynağa ikinci istek birincisi bitene dek bekler**
   (sway'de durağan pencereye eşzamanlı ikinci oturum kare vermedi); bekleyen ≤ 256 (aşan `Busy`);
   oturum başladıktan sonra `timeout` (varsayılan 3 sn) → `Timeout`. ext `failed(buffer_constraints)`
   bir kez yeni kısıtlarla yeniden denenir. Kaynak giderse (`stopped`) `SourceGone`.
6. **Mahremiyet (değişmez 9):** piksel, başlık, uygulama kimliği hiçbir log seviyesinde yazılmaz
   (loglar: istek kimliği, kaynak türü, boyut, biçim kodu, hata türü); Debug'lar redakte (testli);
   framework yakalananı diske yazmaz; yalnızca istenen kaynak yakalanır. Bu süreç oturum kilidi
   isterken/tutarken (`lock_state() != Unlocked`) ya da uygulama dış kilidi bildirdiyse
   (`set_session_locked_hint`, logind `LockedHint`) yeni yakalama `Locked`, yürüyenler `Locked`
   ile biter, çevrilen kare verilmez (capture.md §4). `noscreenshare` gibi gizleme
   kuralları ve gizli çalışma alanındaki pencerenin ne verdiği compositor politikasıdır.
7. **Destek yok:** `capture_support()` `output: None` / `toplevel: false`; `capture` →
   `Err(Unsupported)` (olay olarak). Global geri çekilirse bağlı yönetici bırakılır.

Doğrulama: izole sway 1.12'de `examples/capture.rs` (kendi penceresi + çıkış, piksel denetimi
bellekte; ext ve wlr; normal, 90° döndürülmüş ve 1,5 ölçekli çıkış) — ayrıntı tasarım belgesinde.
**Hyprland canlı doğrulanmadı** (kullanıcının oturumunda yakalama yapılmaz); Hyprland'e özgü adres
eşlemesi yalnızca birim düzeyinde ve XML imzaları Hyprland ikilisine karşı doğrulandı.

### Odak kapma — hyprland_focus_grab_v1 (Hyprland'e özgü)

Sorun: bir uygulamanın `xdg_popup` grab'ı (ör. Discord bağlam menüsü) açıkken shell paneli
açılınca Hyprland odağı panele vermez, menü panelin üstünde açık kalır. Çözüm caelestia,
noctalia ve end4 ile aynı: panel açılırken bir odak kapma kurulur. Grab başlayınca compositor
önceki grab'ı (popup menüsünü) bitirir; işaretçi/dokunma ve klavye odağı beyaz listedeki
yüzeylerle sınırlanır; liste dışına tıklama grab'ı bitirir ve `cleared` gelir.

Bağlama: protokol XML'i `crates/nuxut-wayland/protocols/hyprland-focus-grab-v1.xml`'de gömülü
(hyprland-protocols, BSD-3; telif bloğu dosyada). İstemci kodu derlemede `wayland-scanner`
(`generate_interfaces!`/`generate_client_code!`) ile üretilir — wayland-protocols crate'iyle
aynı yol; derlemede ya da çalışma zamanında sistem paketi gerekmez. Yönetici global'i
açılışta varsa bağlanır; sonradan ilan edilirse (`RegistryHandler`) bağlanır, geri çekilirse
bırakılır (canlı grab nesneleri geçerli kalır, yeni `focus_grab` `None`). Compositor yeniden
başlarsa bağlantı kopar, `run` `Disconnected` ile döner (tüm Wayland durumu gibi).

```rust
// nuxut_wayland::{…} ve kök crate'ten: nuxut::FocusGrabId
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FocusGrabId(/* runtime'a özel */);

impl<A: App> Context<A> {
    /// Compositor protokolü sunuyor mu (Hyprland: evet; sway: hayır).
    pub fn focus_grab_supported(&self) -> bool;
    /// Grab kurar; ilk yüzey birincildir (klavye odağı ona). Protokol yoksa `None` ve hiçbir
    /// şey gönderilmez. Bilinmeyen yüzeyler atlanır.
    pub fn focus_grab(&mut self, surfaces: &[SurfaceId]) -> Option<FocusGrabId>;
    /// Beyaz listeyi değiştirir (fark + tek commit). Boş liste grab'ı bitirir, kimlik kalır;
    /// `cleared` sonrası yeniden kurmak için de bu çağrılır.
    pub fn focus_grab_set_surfaces(&mut self, id: FocusGrabId, surfaces: &[SurfaceId]);
    /// Bırakır (nesne yok edilir; `cleared` gelmez).
    pub fn focus_grab_release(&mut self, id: FocusGrabId);
    /// Compositor'da etkin mi (en az bir yüzey commit'lendi, `cleared` gelmedi).
    pub fn focus_grab_active(&self, id: FocusGrabId) -> bool;
}

// App:
/// Compositor grab'ı bitirdi (liste dışına tıklama ya da başka bir grab). Grab etkisiz,
/// listesi boş, kimlik geçerli. Uygulamanın kendi bıraktığı/boşalttığı grab için gelmez.
fn focus_grab_cleared(&mut self, cx: &mut Context<Self>, id: FocusGrabId) {}
```

Kurallar (runtime uygular; uygulama yalnızca yüzey kümesini verir):
1. **Commit sırası.** `add_surface`/`remove_surface` `commit`'e dek etkisizdir. Runtime her
   değişikliği tek commit'le gönderir; ilk kurulumda iki commit: önce yalnızca birincil yüzey
   (Hyprland grab başlarken listenin ilk yüzeyine klavye odağı verir; başka yüzey — ör. bar —
   seçilemez), sonra diğerleri.
2. **Yalnızca map edilmiş yüzeyler.** Henüz tampon commit'lememiş yüzey listeye ilk tamponu
   gönderildikten sonra eklenir (grab o zaman başlar; `focus_grab_active` o ana dek `false`).
   Paneli açtığınız işleyicide `create_layer_surface` + `focus_grab` aynı anda çağrılabilir.
3. **Boşalma.** Liste boşalınca (uygulama boşalttı ya da yüzeyler yok edildi) boş commit
   yapılmaz, grab nesnesi yok edilir: Hyprland boş commit'te de `cleared` gönderir, bu yolla
   kendi kapattığımız grab için sahte `cleared` gelmez. Yeniden dolunca yeni nesne (kuşak)
   yaratılır; eski nesnenin geç olayı yok sayılır.
4. **Yüzey yok edilmesi.** `destroy_surface`, compositor'ın kapattığı layer yüzeyi ve kilit
   yüzeyleri: yüzey `wl_surface` hâlâ canlıyken listeden çıkarılır (`remove_surface` + commit,
   ya da son yüzeyse nesne yok edilir). Kimlik geçerli kalır.
5. **`cleared`.** Grab etkisizleşir, beyaz liste boşalır, nesne yok edilir;
   `focus_grab_cleared` olay turunun sonunda gelir. Grab kendiliğinden yeniden kurulmaz
   (dışarı tıklama geri alınmaz).
6. **Loglar**: yalnızca kimlik, yüzey sayısı, adım sayısı.

Shell kullanımı (panel/çekmece):
- Panel açılırken: `grab = cx.focus_grab(&[panel_surface, /* bar yüzeyleri, açık popout'lar */])`.
  Birincil = panelin yüzeyi. Bar listede olursa bar'a tıklama grab'ı bitirmez (bar düğmesi
  paneli kendi kapatır/değiştirir); listede yoksa bar'a tıklama da `cleared` üretir.
  Shell'in tek ana yüzeyi (çerçeve + paneller aynı layer yüzeyinde) varsa liste o yüzeydir;
  girdi bölgesi dışı (geçirgen) alana tıklama alttaki pencereye gider ve Hyprland bunu liste
  dışı sayar → `cleared`.
- `focus_grab_cleared(id)` → paneli kapat (animasyonlu); grab kimliğini düşür ya da
  `focus_grab_release` (kapalı grab için istek göndermez, defteri temizler).
- Panel kapanırken (Escape, düğme, komut): önce `focus_grab_release(id)`; bu yolda `cleared`
  gelmez, kapanış çift tetiklenmez.
- Sabitlenmiş (pin) panel grab tutmaz (dışarı tıklama onu kapatmamalı); geçici panel açıkken
  sabit panel yüzeyi aynı yüzeyse listede kalır.
- `None` (sway vb.): mevcut davranış (klavye odağı kaybıyla kapanma, `keyboard_focus`) sürer.
- Klavye etkileşimi: grab, compositor'ın klavye odağını birincil yüzeye verir — layer
  yüzeyinin `KeyboardMode`'u `None` olsa bile (Hyprland kaynağına göre grab yeniden odaklarken
  interactivity'ye bakmıyor; canlı doğrulanmadı — tuş girdisi ölçülmedi).
  Tuş isteyen panel için `OnDemand` yeterli; noctalia `Exclusive` ile açıp ~100 ms sonra
  `OnDemand`'a geçiyor ve geçişte grab'ı yeniden kuruyor (follow-mouse odağı yarışı) —
  shell `Exclusive` kullanıyorsa aynı sıra: `set_keyboard_mode(OnDemand)` SONRA yeni grab
  (`focus_grab_release` + `focus_grab`). Grab sürerken odak kaybı (`keyboard_focus(false)`)
  ile kapatma mantığı yinelenen kapanış üretmemeli: grab etkinken kapanışı `cleared` sürsün.
- Sınır: XWayland uygulamalarının menüleri (override-redirect X pencereleri) Wayland
  `xdg_popup` grab'ı değildir; Hyprland onları grab başlarken kapatmayabilir.

### Renk dönüşümü — hyprland_ctm_control_manager_v1 (Hyprland'e özgü)

Gece ışığı (shell t113, `../shell/docs/design/night-light.md`) için çıkış başına 3×3 renk
dönüşüm matrisi (CTM; KMS'te uygulanır, gamma tablosuyla çakışmaz). Kabuk matrisi hesaplar,
runtime protokol nesnesinin yaşamını, doğrulamayı ve `wl_fixed` dönüşümünü yürütür. XML
`crates/nuxut-wayland/protocols/hyprland-ctm-control-v1.xml` (hyprland-protocols 6c520135,
sürüm 2), kod `color_transform.rs`. Ek thread yok; boşta hiçbir iş yok (yalnızca çağrıda istek).

```rust
/// Satır-öncelikli 3×3: [r', g', b'] = M · [r, g, b]. Bileşenler sonlu ve ≥ 0.
pub struct ColorMatrix(pub [f32; 9]);
impl ColorMatrix {
    pub const IDENTITY: Self;
    pub const fn diagonal(r: f32, g: f32, b: f32) -> Self;
    pub fn is_identity(&self) -> bool; // 1/256 adımına yuvarlanınca
}
pub enum ColorTransformState { Unsupported, Idle, Active, Blocked }
pub enum ColorTransformError {
    Unsupported, Blocked, UnknownOutput(OutputId),
    InvalidMatrix { output: OutputId, index: usize },
}
impl<A: App> Context<A> {
    /// Global ilan edildi mi (bağlanmaz, CTM'e dokunmaz).
    pub fn color_transform_supported(&self) -> bool;
    pub fn color_transform_state(&self) -> ColorTransformState;
    /// Tek atomik commit. Hata varsa hiçbir istek gitmez.
    pub fn set_color_transforms(&mut self, transforms: &[(OutputId, ColorMatrix)])
        -> Result<(), ColorTransformError>;
    /// Yöneticiyi bırak → Hyprland tüm çıkışları birime döndürür; Blocked'ı sıfırlar.
    pub fn color_transform_release(&mut self);
}
// App:
fn color_transform_blocked(&mut self, cx: &mut Context<Self>) {}
```

Sözleşme:

- **Tembel bağlanma.** Açılışta global yalnızca kaydedilir. Hyprland'de sahiplik bağlanmayla
  alınır (ilk bağlanan yönetici sahip, sonrakiler `blocked`) — bu yüzden yönetici ilk **birim
  olmayan** `set_color_transforms`'ta bağlanır. Gece ışığı kapalı kullanıcıda CTM'e dokunulmaz,
  kullanıcının hyprsunset'i engellenmez. Yönetici yokken yalnızca birim matris istemek no-op.
- **Kapatma = `color_transform_release`**, birim matris göndermek değil: birim gönderen
  yönetici sahipliği tutmaya devam eder (başka araç engellenir). Bırakınca Hyprland sıfırlar.
- **Tekrar yok:** çıkış başına son gönderilen (sabit noktaya yuvarlanmış) değer tutulur;
  değişmeyen çıkış yeniden gönderilmez, hiçbiri değişmediyse commit yok (Hyprland her commit'te
  geçiş animasyonunu yeniden başlatır). Listede olmayan çıkış compositor'daki değerini korur.
- **Yeni çıkış:** runtime kendiliğinden matris vermez; uygulama `output_added`'da yeniden
  `set` çağırır. Kaldırılan çıkışın kaydı silinir.
- **Doğrulama:** negatif, NaN/∞ ya da `wl_fixed` aralığı (> 8 388 607) dışı bileşen
  `InvalidMatrix` — Hyprland'de bu `invalid_matrix` **protokol hatasıdır ve bağlantıyı (tüm
  kabuğu) öldürür**, bu yüzden gönderilmeden reddedilir. `-0.0` geçerli. Değerler 1/256'ya
  yuvarlanır (`wl_fixed` 24.8).
- **`blocked`** (sürüm 2): başka istemci önceden bağlıysa gelir. Runtime engellenmiş yöneticiyi
  yok eder (protokol önerisi; engellenmiş yöneticinin yıkımı CTM'e dokunmaz), durum `Blocked`,
  `App::color_transform_blocked` çağrılır; `set` `Err(Blocked)` döner, kendiliğinden yeniden
  denenmez (sahip istemci çıkınca haber gelmez). Uygulama `color_transform_release` ile sıfırlayıp
  sonra (ör. kullanıcı tekrar açınca) yeniden deneyebilir. Kuşak numarası eski yöneticinin geç
  olayını eler.
- **Destek yok** (sway vb.): `color_transform_supported() == false`, durum `Unsupported`,
  `set` → `Err(Unsupported)`; hiçbir istek gönderilmez. Global geri çekilirse yönetici yok
  edilir, durum `Unsupported`.
- Kapanışta yönetici açıkça yok edilir.

Hyprland 0.56.2 doğrulaması (2026-10-13; kaynak
`https://raw.githubusercontent.com/hyprwm/Hyprland/v0.56.2/src/protocols/CTMControl.cpp`):

- **Kendiliğinden sıfırlanma — evet.** Yönetici kaynağının yıkıcısı engellenmemişse tüm
  monitörlere birim CTM verir (`~CHyprlandCTMControlResource`, satır 74–81). Yıkıcı hem
  `destroy` isteğinde (`setDestroy`, satır 17) hem kaynak istemciyle birlikte yok olunca
  (`setOnDestroy`, satır 18 — istemci kapanır ya da çöker) `destroyResource` (108–110) ile
  çalışır. Kabuk çökse de ekran turuncu kalmaz. Ek güvence gerekmez.
- **`blocked`:** `bindManager` (91–106): `m_manager` (zayıf işaretçi) canlıysa yeni kaynak
  `block()` alır, değilse sahip olur. `block()` (67–72) yalnızca kaynak sürümü ≥ 2 ise olayı
  yollar. Engellenmişken `set_ctm_for_output`/`commit` sessizce düşer (22–23, 51–52). Olay
  yalnızca bağlanma anında gelir; sahip sonradan çıksa da engellenmiş kaynak engelli kalır.
  Hyprland global'i sürüm 2 ilan eder (`src/managers/ProtocolManager.cpp` satır 210). Olay
  hyprland-protocols'a 2025-02-01'de eklendi (commit 6c520135, "add
  hyprland_ctm_control_manager_v1.blocked (#14)").
- **Commit semantiği:** `commit` (50–64) tüm monitörleri gezer; bu yöneticiyle hiç ayarlanmamış
  monitör birim alır, ayarlanan monitör (ada göre, `m_ctms`) son değerini alır.
- **Değer sınırları:** her bileşen `isfinite && >= 0` olmalı, aksi `invalid_matrix` hatası
  (38–43). Üst sınır yok (XML: `[0, ∞)`); pratik sınır `wl_fixed`.
- **`render:ctm_animation`:** 0 kapalı, 1 açık, 2 otomatik (varsayılan; NVIDIA < 575 sürücüde
  kapalı, aksi açık) (`isCTMAnimationEnabled` 112–122; `src/config/values/ConfigValues.cpp`
  565). Açıkken her `setCTM` önceki hedeften yeni hedefe doğrusal geçiş yapar
  (`__internal_fadeCTM`: hız 5 = 500 ms, `linear`; `AnimationTree.cpp` 63) — sıfırlama da
  aynı geçişle. Kabuğun kendi yumuşak geçişi ince adımlarla `set` çağırırsa her adım bu 500 ms
  geçişi yeniden başlatır (zararsız; yalnızca gecikmeli takip).
- Canlı kontrol (salt-okur, kullanıcının oturumu): `cargo run -p nuxut-wayland --example
  color_transform` → `supported=true state=Idle`, birim `set` bağlanmadı. İzole sway:
  `supported=false state=Unsupported`, `set` → `Err(Unsupported)`. CTM gerçek ekranda
  ayarlanmadı (`--apply` yalnızca elle).

Sınırlar: sürüm 1 compositor'da `blocked` gelmez, başka sahip varken istekler sessizce düşer
(durum `Active` görünür). `wlr-gamma-control` (sway) desteklenmez.

### Boşta bildirimi — ext-idle-notify-v1

Shell'in boşta servisi (karartma, ekranı kapatma, kilit, askıya alma; shell t061, varsayılan
KAPALI) için. Sayaç compositor'dadır; runtime yoklama/zamanlayıcı kullanmaz (boşta sıfır CPU,
yalnızca protokol olayları). Global `ext_idle_notifier_v1` açılışta ya da sonradan ilan
edildiğinde 1..=2 sürüm aralığında bağlanır; geri çekilirse nesneler yok edilir.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdleWatchId(/* gizli */);

pub enum IdleSupport {
    Unsupported,                              // global yok
    Supported { ignore_inhibitors: bool },    // true: sürüm ≥ 2
}
#[derive(Default)]
pub enum IdleInhibitors {
    #[default] Respect, // get_idle_notification — inhibitor etkinken boşta sayılmaz
    Ignore,             // get_input_idle_notification (v2) — salt girdi yokluğu
}
pub enum IdleWatchError { Unsupported, InputIdleUnsupported } // Display + Error

impl<A: App> Context<A> {
    /// O anki destek durumu (global sonradan gelebilir/gidebilir).
    pub fn idle_support(&self) -> IdleSupport;
    /// Respect türünde izleme, varsayılan koltuk.
    pub fn idle_watch(&mut self, timeout: Duration) -> Result<IdleWatchId, IdleWatchError>;
    pub fn idle_watch_with(&mut self, timeout: Duration, inhibitors: IdleInhibitors)
        -> Result<IdleWatchId, IdleWatchError>;
    /// Ayar değişince yeniden kayıt: aynı kimlik, yeni süre, sayaç baştan.
    /// Bilinmeyen/bitmiş kimlikte false.
    pub fn idle_set_timeout(&mut self, id: IdleWatchId, timeout: Duration) -> bool;
    /// Bildirim nesnesini yok eder; bilinmeyen/bitmiş kimlik yok sayılır (idempotent).
    pub fn idle_unwatch(&mut self, id: IdleWatchId);
}

// App:
/// true: süre doldu (idled), false: kullanıcı döndü (resumed).
fn idle(&mut self, cx: &mut Context<Self>, id: IdleWatchId, idle: bool) {}
```

Sözleşme:
- Destek yoksa durum açık: `idle_support() == Unsupported`, `idle_watch` →
  `Err(Unsupported)`. Shell özelliği "bu compositor desteklemiyor" olarak gösterir; sessiz
  geri düşüş (zamanlayıcıyla taklit) YOK.
- İki tür (ayrım belgeli, seçim uygulamanın):
  - `Respect` (varsayılan): Wayland idle inhibitor'ları (`zwp_idle_inhibit_manager_v1`, ör.
    tam ekran video, sunum) etkinken compositor `idled` göndermez. Karartma, ekran kapatma,
    kilit, askıya alma için DOĞRU tür; shell kendi inhibitor mantığını ayrıca yönetmez.
  - `Ignore`: yalnızca protokol sürüm 2 (`get_input_idle_notification`); inhibitor'lar yok
    sayılır, salt girdi yokluğu ölçülür ("uzakta" durumu, istatistik). Ekran/kilit için
    kullanılmaz (video izlerken ekran kararır). Sürüm 1'de `Err(InputIdleUnsupported)`;
    global sonradan sürüm 1 olarak yeniden ilan edilirse var olan `Ignore` izleme `Respect`
    olarak kurulur (uyarı loglanır).
- Süre kabloda `u32` milisaniye: `[1 ms, u32::MAX ms]` aralığına kırpılır (0 ve milisaniye
  altı → 1 ms, yani "hemen boşta").
- Her izleme ayrı bir bildirim nesnesi; birden çok eşzamanlı izleme serbest (karart/ekran
  kapat/kilit/askıya al ayrı süreler). Kimlikler tekrar kullanılmaz.
- Yeniden kayıt (`idle_set_timeout`): protokolde süre değiştirme isteği olmadığından nesne yok
  edilip yeni süreyle yaratılır, sayaç baştan başlar; izleme o an boştaysa `idle(.., false)`
  verilir (kullanıcı hâlâ boştaysa yeni süre sonunda yine `true`). Shell ayar değişiminde
  kimlikleri tutar, yalnızca değişen süreyi yeniden kaydeder; kapatılan adım `idle_unwatch`.
- Koltuk: açılıştaki ilk `wl_seat` (varsayılan). Koltuk yokken kurulan izleme `Ok` döner ve
  bekler; koltuk gelince nesne yaratılır. Varsayılan koltuk kaybolursa nesneler yok edilir,
  kalan bir koltuk varsa onunla, yoksa yeni koltuk ilan edilince yeniden kurulur (sayaç
  baştan). Notifier global'i geri çekilip yeniden ilan edilirse de aynı.
- Kayıp (koltuk/notifier) anında boşta bildirilmiş izlemeye `idle(.., false)` verilir:
  uygulama "boşta" durumunda takılı kalmaz.
- `idle` yalnızca durum değişiminde gelir (ilk olay her zaman `true`; aynı değer iki kez
  gelmez), olay turunun sonunda (`after_dispatch`) teslim edilir. `idle_unwatch` sonrası o
  kimlik için olay gelmez — aynı turda kuyruğa girmiş olay da teslimde düşülür; eski nesnenin
  geç olayı (kuşak) yok sayılır.
- Kanıt: `examples/idle_watch.rs` (headless sway 1.12, notifier v2: 2 sn Respect + 2 sn
  Ignore → `idle=true`; `idle_set_timeout(1 sn)` → `false`, 1 sn sonra `true`; `wtype` ile
  girdi → `false`; unwatch sonrası olay yok).

### Sanal klavye — zwp-virtual-keyboard-unstable-v1 (güvenlik hassas)

Ekran klavyesi (shell t125) odaktaki pencereye tuş gönderir. Sanal klavye **her** pencereye
girdi verebildiği için varsayılan bir yol yoktur: runtime kendiliğinden kurmaz, tuş üretmez,
tekrar etmez; yalnızca bu yöntemleri açıkça çağıran uygulama kullanır (2026-10-15,
istek: ekran klavyesi).

```rust
impl<A: App> Context<A> {
    pub fn virtual_keyboard_supported(&self) -> bool;     // global var ve compositor reddetmedi
    pub fn keyboard_keymap(&self) -> Option<&str>;        // compositor'ın şu anki XKB_V1 keymap'i
    pub fn virtual_keyboard_set_keymap(&mut self, keymap: &str) -> Result<(), VirtualKeyboardError>;
    pub fn virtual_keyboard_key(&mut self, evdev_code: u32, pressed: bool) -> Result<(), VirtualKeyboardError>;
    pub fn virtual_keyboard_modifiers(&mut self, depressed: u32, latched: u32, locked: u32, group: u32)
        -> Result<(), VirtualKeyboardError>;
    pub fn virtual_keyboard_release(&mut self);           // basılıları bırak, mods 0, nesne + bağlantı kapanır
}
pub enum VirtualKeyboardError { Unsupported, Unauthorized, InvalidKeymap, NoKeymap, NoSeat,
    InvalidKey, TooManyKeys, Disconnected }
pub const KEY_MAX: u32 = 0x2ff; pub const MAX_PRESSED: usize = 16; pub const MAX_KEYMAP_LEN: usize = 1 << 20;
```

- **Ayrı Wayland bağlantısı.** Protokolün bütün hataları ölümcüldür (yetkisiz istemciye
  yöneticinin `unauthorized`'ı, keymap'siz tuşa `no_keymap`; wlroots derlenemeyen keymap'e
  `no_memory`) ve bağlantıyı koparır. Bu yüzden sanal klavye ilk `set_keymap`'te açılan ikinci
  bağlantıda yaşar; ana bağlantıda yönetici global'i yalnızca kaydedilir, bağlanmaz. Hata yalnız
  ikinci bağlantıyı öldürür, kabuk ayakta kalır. Kurulmamışken ve `release` sonrası kaynak yok
  (ek soket, nesne, olay kaynağı yok).
- `set_keymap` eşzamanlıdır (bağlantı + kayıt turu + nesne + keymap + gidiş-dönüş; yerel
  sokette ms altı) — compositor'ın reddi aynı çağrıda `Err(Unauthorized)` olarak döner.
  `Unauthorized` ve diğer protokol hataları kalıcıdır: `virtual_keyboard_supported()` oturum
  boyunca `false`, uyarı bir kez. G/Ç kopması (`Disconnected`) sonraki `set_keymap`'te
  yeniden denenir. Compositor reddi istemciyi düşürmek yerine aygıtı sessizce bağlamazsa
  (ör. Hyprland izin sistemi `deny`) bu istemciden görülemez.
- Keymap: sondaki NUL'lar atılır; boş, `MAX_KEYMAP_LEN`'den büyük, iç NUL'lu ya da
  xkbcommon'ın derleyemediği metin gönderilmez (`InvalidKeymap`). Mühürlü memfd'ye NUL'la
  yazılır (dosya sisteminde iz yok, compositor yazamaz). Aynı metin tekrar gönderilmez.
  Değişen keymap'ten önce basılı tuşlar bırakılır ve değiştiriciler sıfırlanır. Ekran klavyesi
  `keyboard_keymap()`'i verir ve `App::keyboard_layout_changed`'da yeniden çağırır.
- Tuşlar evdev kodudur (`KEY_A` = 30); `time` monoton ms. Basılı tuşlar izlenir: basılıya
  basma / basılı olmayanı bırakma gönderilmez, en çok `MAX_PRESSED` birlikte. Tekrar yok:
  basılı tutulan tuşu hedef istemci `repeat_info` ile kendisi tekrarlar; kabuğun tekrarı
  basma/bırakma çiftleridir. Değiştiriciler tuşlardan türetilmez (wlroots): kabuk kendi
  keymap'ine göre `virtual_keyboard_modifiers` gönderir. Keymap'siz `key` → `NoKeymap`
  (protokol hatası değil).
- Takılı tuş: `release`, `Context` düşüşü (`Drop`) ve koltuk/global kaybı basılıları bırakır.
  Bağlantı ölmüşse ya da süreç çökerse compositor istemciyle birlikte aygıtı yok eder ve
  basılıları bırakır (wlroots `wlr_keyboard_finish`).
- Koltuk (ikinci bağlantıda ilk `wl_seat`) giderse nesne yok edilir, kayıt unutulur; koltuk
  gelince nesne yeniden yaratılır ve son keymap yeniden gönderilir. Arada `key` → `NoSeat`.
- Loglarda tuş kodu yok (yazılan metni ele verir); `Debug` yalnız sayı gösterir (testli).
- Kanıt: `examples/virtual_keyboard.rs` (headless sway, `kitty --class nuxut-test` içinde
  `read`: compositor keymap'iyle (Turkish) "Nuxut vkbd" + Enter — Shift + değiştirici maskesiyle
  büyük harf — okundu; aynı keymap no-op, `release` iki kez, sonrasında `NoKeymap`). Koltuk
  yeniden ilanı canlı denenmedi (sway koltuğu kaldıramıyor); birim testler: memfd, doğrulama,
  basılı tuş takibi, bırakma, `unauthorized` ayrımı.

### Pano — wl_data_device + primary selection

Platformdan bağımsız tipler `nuxut_core::clipboard`'ta (nuxut-wayland yeniden ihraç eder):
`Selection::{Clipboard, Primary}`, `ClipboardText` (Clone yok, Debug yalnız uzunluk),
`PasteError::{Unsupported, Empty, NotText, Locked, TooLarge, Timeout, InvalidUtf8, Io}`,
`CopyError::{Unsupported, Locked, NoInput}`, `preferred_mime`, `decode`, sınırlar
(`MAX_PASTE_BYTES` 4 MiB, `PASTE_TIMEOUT` 2 sn, `COPY_WRITE_TIMEOUT` 5 sn). Başka bir platform
katmanı aynı tipleri ve kuralları kullanır.

```rust
impl<A: App> Context<A> {
    /// Seçimi sahiplenir; metin bellekte tutulur, istendikçe boruya yazılır.
    pub fn set_clipboard(&mut self, selection: Selection, text: &str) -> Result<(), CopyError>;
    /// Sonuç `App::paste` ile, her istek için TAM BİR KEZ, çağrının içinden değil.
    pub fn request_paste(&mut self, selection: Selection) -> PasteRequestId;
}

// App:
fn paste(
    &mut self,
    cx: &mut Context<Self>,
    id: PasteRequestId,
    result: Result<ClipboardText, PasteError>,
) {}
```

Kurallar:
1. **Yalnız metin.** Kopyalarken `text/plain;charset=utf-8` ve `text/plain` sunulur; başka
   türdeki istek boruyu kapatır. Yapıştırırken önce `charset=utf-8`, sonra `text/plain`
   (büyük/küçük harf ve boşluk toleranslı); ikisi de yoksa `NotText`. İçerik UTF-8 olarak
   doğrulanır (kayıplı dönüşüm yok → `InvalidUtf8`).
2. **Olay döngüsü bloklanmaz.** Borular O_NONBLOCK, calloop kaynağıyla hazır oldukça
   okunur/yazılır. Okuma `MAX_PASTE_BYTES`'ı aşarsa `TooLarge`, `PASTE_TIMEOUT`'ta bitmezse
   `Timeout` (kaynak hiç yazmasa da UI donmaz). Başka istemciye yazma `COPY_WRITE_TIMEOUT`'ta
   kesilir; aynı anda en çok 8 yazma, fazlası hemen kapatılır.
3. **Serial.** `set_clipboard` son girdinin (tuş ya da işaretçi basma) serial'ini kullanır;
   hiç girdi yoksa `NoInput`. Compositor eski serial'i sessizce yok sayabilir: kopyalama bir
   girdi işleyicisinden yapılmalı.
4. **Kendi seçimimiz** boru kullanmadan bellekten yapıştırılır. Başka istemci seçimi alınca
   (`cancelled`) metin bırakılır.
5. **Kilit** (`Pending`/`Locked`): `set_clipboard` → `Locked`, `request_paste` →
   `paste(id, Err(Locked))`; kilit istendiği anda süren okumalar `Locked` ile biter (içerik
   uygulamaya varmaz). Kilitliyken kendi seçimimiz başka istemciye yazılmaz (boru hemen
   kapatılır; savunma derinliği — kilitte odak zaten bizdedir). Seçim kaynağı korunur; kilit
   açılınca yeniden sunulur.
6. **Koltuk.** Aygıtlar ilk klavyeli koltuğa bağlanır; klavye giderse aygıtlar ve sahip
   olunan seçimler bırakılır. Protokol ya da koltuk yoksa `Unsupported`. Compositor seçimi
   yalnızca klavye odağı almış istemciye bildirir: hiç odak almamış uygulamada `Empty`.
7. **Sürükle-bırak yok**; DnD teklifleri kabul edilmez.
8. **Loglar** (değişmez 9): içerik yazılmaz; yalnızca seçim türü, bayt sayısı, hata türü ve
   istek kimliği.

`TextInput` panoya dokunmaz (`Copy`/`Cut`/`PasteRequested` olayları); uygulama bunları
`set_clipboard`/`request_paste`'e bağlar ve `App::paste`'te `TextInput::paste` çağırır
(örnek: `nuxut-ui/examples/gallery.rs`; demo: `nuxut-wayland/examples/clipboard.rs`).

`set_clipboard` kaynağı (Clipboard ve Primary) metin türlerine ek olarak sürecin **işaret
MIME'ını** (`owner_mime()`, aşağıda) da sunar; o tür istenirse boru boş kapatılır.

### Pano izleme — ext-data-control-v1 / zwlr-data-control-v1 (t060)

Pano geçmişi gibi tüketiciler için odaksız seçim izleme ve koyma (`data_control.rs`). Talep ve
gizlilik kuralları: shell `docs/design/clipboard.md`. Protokol: `ext_data_control_manager_v1`
(sürüm 1), yoksa `zwlr_data_control_manager_v1` (ilan edilen sürüm, en çok 2; sürüm 1'de
birincil seçim yok); ikisi de yoksa `data_control_protocol() == None`.
`NUXUT_DATA_CONTROL=wlr` (test düğmesi) ikisi de varken wlr'yi seçtirir. Pano
(`Selection::Clipboard`) her zaman izlenir; birincil seçim (`Selection::Primary`) olayları
yalnızca `data_control_watch_primary(true)` ile gelir (varsayılan kapalı: seç-bırak çok sık
değişir, shell geçmişi istemez).

```rust
impl<A: App> Context<A> {
    pub fn data_control_protocol(&self) -> Option<DataControlProtocol>; // Ext | Wlr; bağlanmadan
    pub fn data_control_primary_supported(&self) -> bool;              // ext ya da wlr ≥ 2
    /// Yönetici + varsayılan koltukta aygıt; compositor güncel seçimi hemen bildirir.
    pub fn data_control_start(&mut self) -> Result<DataControlHandle, DataControlError>; // Unsupported | NoSeat
    pub fn data_control_watch_primary(&mut self, on: bool); // açılınca güncel birincil durum olay olur
    pub fn data_control_stop(&mut self);              // aygıt + teklifler bırakılır; kendi seçimimiz kalır
    pub fn data_control_handle(&self) -> Option<DataControlHandle>;
    /// Odak/serial istemez. Kaynak başka istemci seçimi alana dek sunulur.
    pub fn data_control_set_selection(&mut self, selection: Selection,
        data: Vec<(String, Arc<[u8]>)>) -> Result<(), SetSelectionError>;
        // NotStarted | Unsupported (birincil, wlr v1) | Invalid | Locked
    /// Kısa ömürlü thread'de okur; sonuç App::data_control_read ile, TAM BİR KEZ.
    pub fn data_control_read(&mut self, id: DataOfferId, mime: &str, max_bytes: usize,
        timeout: Duration) -> DataReadId;
}

/// Clone + Send + Sync. Bloklayan çağrılar ana thread'den `MainThread` döner.
impl DataControlHandle {
    pub fn read(&self, id: DataOfferId, mime: &str, max_bytes: usize, timeout: Duration)
        -> Result<Zeroizing<Vec<u8>>, DataReadError>;
    pub fn set_selection(&self, selection: Selection, data: Vec<(String, Arc<[u8]>)>)
        -> Result<(), SetSelectionError>;
}

pub enum DataControlEvent {
    SelectionOffered { selection: Selection, id: DataOfferId, mimes: Vec<String>,
                       own: bool, sensitive: bool, truncated: bool },
    SelectionCleared { selection: Selection },
    Stopped, // aygıt finished / koltuk ya da global gitti; yeniden start denenebilir
}
pub enum DataReadError { Gone, NotOffered, Sensitive, Locked, TooLarge, Timeout, Io, MainThread, Closed }
pub fn is_sensitive_mime(mime: &str) -> bool; // PASSWORD_HINT_MIME ya da adında secret/password

// App:
fn data_control(&mut self, cx: &mut Context<Self>, event: DataControlEvent) {}
fn data_control_read(&mut self, cx: &mut Context<Self>, id: DataReadId,
    result: Result<Zeroizing<Vec<u8>>, DataReadError>) {}
```

Kurallar:
1. **Tembel.** Globaller açılışta yalnızca kaydedilir; `data_control_start` çağrılmadan
   yönetici bağlanmaz, aygıt yaratılmaz ve compositor bize seçim bildirmez. Varsayılan koltuk
   ilk `wl_seat`; giderse izleme durur (`Stopped`).
2. **MIME sınırı.** Teklif başına ≤ `MAX_MIMES` (64) tür, her ad 1..=`MAX_MIME_LEN` (256)
   bayt, tekrarsız; fazlası düşer ve `truncated = true` (liste eksik — temkinli tüketici,
   ör. shell gizlilik filtresi, teklifi atlar).
3. **`sensitive`.** Teklif `x-kde-passwordManagerHint` (`PASSWORD_HINT_MIME`; KeePassXC,
   KDE) ya da adında `secret`/`password` geçen bir tür sunuyorsa (ASCII büyük/küçük harf
   duyarsız; sınırı aşıp listeden düşen türler dahil) `sensitive = true` ve framework bu
   teklifin **hiçbir** türünü okumaz (`DataReadError::Sensitive`; ipucu türünün değeri de
   okunmaz). Karar yalnızca tür adlarından verilir. Shell'in ek kuralları (kilit, `exclude_apps`)
   üstte uygulanır. Bilinen sınır: işaret koymayan uygulamanın parolası ayırt edilemez.
4. **`own`.** Bu sürecin her pano kaynağı (`set_clipboard` Clipboard/Primary ve
   `data_control_set_selection`) süreç başına rastgele `application/x-nuxut-owner-<16 hex>`
   türünü de sunar. Teklifte görülürse `own = true` ve tür listeden atılır (64 sınırı
   dolduktan sonra gelse de tanınır). Zamanlamaya ya da compositor olay sırasına bağlı değildir.
   Olay yine gelir (seçimin değiştiğini bilmek gerekir); tüketici `own` teklifi geçmişe yeniden
   almaz — kendi koyduğu seçim döngüye girmez. Bilinen sınır: işareti gören başka bir istemci
   onu kopyalayıp teklifini "bizim" gösterebilir; etkisi yalnızca o kopyanın geçmişe
   girmemesidir (veri sızmaz).
5. **Okuma yalnızca istenince.** Framework hiçbir teklifi kendiliğinden okumaz. Okuma
   yalnızca güncel teklif (pano ya da birincil) ve onun sunduğu bir tür için yapılır: eski
   kimlik → `Gone`, sunulmayan tür (işaret dahil) → `NotOffered`. `SelectionOffered` olayı
   kuyrukta beklerken yerine yenisi geldiyse (ya da birincil izleme kapatıldıysa) uygulamaya
   verilmez.
6. **Ana döngü bloklanmaz; boşta iş yok.** `DataControlHandle::read` çağıranın thread'inde
   boru açar, `receive`'i bir calloop kanalıyla ana döngüye verir (ana döngü yalnızca isteği
   gönderip kendi yazma ucunu kapatır) ve boruyu `poll` ile son tarihe dek okur: `max_bytes`
   aşılırsa `TooLarge` (okuma durur, boru kapanır), süre dolarsa `Timeout`. Kalıcı thread yok;
   `data_control_read` okuma başına kısa ömürlü bir thread açar (en çok `timeout` yaşar). Boşta
   yalnızca uyuyan kanal (canlı ölçüm: izlerken 3 sn boşta 0 CPU tiki).
7. **Seçim koyma.** Liste 1..=64 öğe, ad 1..=256 bayt, kontrol karakteri ve tekrar yok,
   işaret öneki ayrılmış (`Invalid`). Baytlar paylaşılan `Arc<[u8]>`; her istek bloklamadan,
   `COPY_WRITE_TIMEOUT` (5 sn) ile ve pano yazmalarıyla ortak en çok 8 eşzamanlı yazmayla
   sunulur. Aynı seçimdeki yeni kaynak öncekini yok eder; başka istemci seçimi alınca
   (`cancelled`) kaynak bırakılır. `data_control_stop` kendi seçimimizi bırakmaz; `Stopped`
   bırakır.
8. **Gizlilik (sıfırlama).** Okuma tamponu `Zeroizing`; büyürken içerik yeni `Zeroizing`
   tampona taşınır ve eskisi sıfırlanır (`Vec` yeniden ayırması serbest bellekte kopya
   bırakmaz); parça tamponu da sıfırlanır; `TooLarge`/`Timeout`/hata yolunda yarım tampon
   sıfırlanarak düşer. Seçime konan baytlar, kaynak yerine geçince/iptal edilince ya da süren
   yazma bitince, framework son sahipse (`Arc::get_mut`) sıfırlanır; uygulama hâlâ tutuyorsa
   (ör. geçmiş öğesi) dokunulmaz — o kendi kopyasından sorumludur. Çekirdek boru tamponu ve
   karşı uygulamanın kopyaları kapsam dışıdır.
9. **Kilit** (`Pending`/`Locked`): okuma `Locked`, seçim koyma `Locked`; kendi data-control
   seçimimiz başka istemciye yazılmaz (boru hemen kapanır). Olaylar (yalnızca tür listesi)
   gelmeye devam eder. Bilinen sınır: kilit, `receive` gönderildikten sonra gelirse süren okuma
   tamamlanır (kilit kararı shell'de de verilir: okuma sürerken kilitlenirse saklanmaz).
10. **Loglar** (değişmez 9): içerik yazılmaz; yalnızca kimlik, tür sayısı, bayt sayısı, hata
    türü. Okuma sonucunun kuyruktaki `Debug`'ı yalnızca uzunluk yazar.

Durum makinesi (`OfferBook`: seçim başına güncel teklif, birincil izleme anahtarı, teslim
kapısı, okuma kabulü) protokol nesnesinden bağımsız ve birim testlidir.

Demo ve canlı doğrulama: `nuxut-wayland/examples/data_control.rs` (YALNIZCA test
compositor'ında; gerçek oturumda kullanıcının panosunu okur/değiştirir). İzole sway 1.12'de
ext (sürüm 1) ve wlr (sürüm 2) yollarıyla doğrulandı (2026-10-13): `wl-copy`
teklifi, olaylı ve thread'den okuma aynı özet, `max_bytes = 1` → `TooLarge`, ana thread →
`MainThread`, thread'den `set_selection` → `wl-paste` aynı bayt, sonraki teklif `own = true`,
eski kimlik → `Gone`; birincil seçim için aynısı (`wl-copy --primary` / `wl-paste --primary`);
`wl-copy --type x-kde-passwordManagerHint` → `sensitive = true`, okuma `Sensitive`;
`wl-copy --clear` → `SelectionCleared`; `wl-paste --list-types` işaret türünü gösterir.

### Renderer / Painter

```rust
pub struct Renderer;  // tek GL context, tüm yüzeyler paylaşır (shader, atlas)
impl Renderer {
    pub fn new(gl: glow::Context) -> Result<Self, RenderError>;
    /// Hedef framebuffer'ı temizler (şeffaf), çizdirir, gönderir.
    pub fn render(&mut self, text: &mut TextSystem, size_px: (u32, u32), scale: f32,
                  f: impl FnOnce(&mut Painter));
}

impl Painter<'_> {   // tüm koordinatlar mantıksal; ölçek içeride uygulanır
    pub fn rect(&mut self, rect: Rect, style: &RectStyle);        // dolgu + opsiyonel kenarlık
    pub fn gradient_rect(&mut self, rect: Rect, radii: Corners, colors: [Color; 4]); // tl,tr,br,bl bilineer
    pub fn line(&mut self, p0: Point, p1: Point, width: f32, color: Color);   // yuvarlak uçlu kapsül SDF
    pub fn polyline(&mut self, points: &[Point], width: f32, color: Color);  // yuvarlak birleşim; yarı saydamda ek yerleri iki kez karışır
    pub fn arc(&mut self, center: Point, radius: f32, start: f32, sweep: f32, width: f32, color: Color); // tek dörtgen yay SDF; radyan, 0 = +x, + saat yönü
    pub fn area(&mut self, points: &[Point], baseline: f32, color: Color, fade: f32); // kırık çizgi altı alan (sparkline dolgusu): parça başına
        // bir sütun örneği (stroke boru hattı, tür 2); eğik üst kenar ve taban 1 fiziksel px AA, sütunlar [x0, x1) sert sınırla
        // ek yersiz ve çift karışımsız birleşir (yarı saydam güvenli, kesirli ölçekte de). fade: tabandaki opaklık çarpanı
        // (1 düz, 0 tabana sönen; geçiş alanın tepesinden tek parça). x artmayan ya da sonlu olmayan uçlu parça atlanır (boşluk).
    pub fn shadow(&mut self, rect: Rect, radii: Corners, style: &ShadowStyle);
    pub fn text(&mut self, layout: &TextLayout, origin: Point, color: Color);
    pub fn blob(&mut self, field: &BlobField);                     // çerçeve + paneller
    pub fn push_clip(&mut self, rect: Rect);
    pub fn pop_clip(&mut self);
    // Kaplama katmanı: begin..end arası çizimler kare sonunda en üstte, temel katmanın kırpma
    // yığınından bağımsız (kendi yığını boş başlar). Kayıt düzeyinde: iki komut listesi yer
    // değiştirir, renderer kare sonunda Clip(None) + kaplamayı ekler (ek GL geçişi yok). İç içe
    // begin false döner; kapatılmayan kaplama Painter düşerken kapatılır (warn).
    pub fn begin_overlay(&mut self) -> bool;  pub fn end_overlay(&mut self);  pub fn in_overlay(&self) -> bool;
    pub fn scale(&self) -> f32;
    pub fn size(&self) -> Size;                                    // mantıksal hedef boyutu
    /// `draw` içinde `cx.text()` kullanılamaz (TextSystem render'a ödünç verilmiştir);
    /// önbelleği boş bir layout'u burada üretmek için.
    pub fn text_system(&mut self) -> &mut TextSystem;
}
```

`Recorder` (GL'siz kayıt, bileşen testleri için): `Recorder::new(); record(text, size_px, scale, |painter| ..);
draws() -> Vec<RecordedDraw { kind: DrawKind::{Quad, Blob, Glyphs, Image, Gradient, Stroke, Area}, rect /*fiziksel; çizgi/yay/alanda
sınır kutusu (AA payı dahil), metinde boş*/, clip: Option<PxRect> }>;
quads(); is_empty()` — çizim sırası ve etkin kırpma, kare sonu birleştirmesi (kaplama) dahil.

**Blob alanı** — caelestia'nın "puding" efektinin çekirdeği; şekil ADR-0008'in tek tanımı:

```rust
pub struct BlobField {
    pub frame: Option<FrameShape>,   // outer içinde, inner dışında kalan yer dolu
    pub shapes: Vec<BlobShape>,      // ≤ 16; çerçeveyle ve birbirleriyle birleşir
    pub smoothing: f32,              // R: birleşim (içbükey) filetosu ve çerçeve iç köşesi
    pub corner_radius: f32,          // r: panelin boşluğa açılan (dışbükey) köşesi
    pub fill: Color,
    pub shadow: Option<ShadowStyle>,
}
pub struct FrameShape { pub outer: Rect, pub inner: Rect, pub inner_radius: f32 /* yok sayılır: = R */ }
pub struct BlobShape { pub rect: Rect, pub radii: Corners /* şeklin kendi yuvarlaması */,
                       pub deform: [f32; 4] /* 2x2 satır öncelikli, kimlik = [1,0,0,1] */,
                       pub deform_anchor: Point /* rect içinde normalize sabit nokta; merkez (0.5,0.5) */ }
// Panel: BlobShape::new_rect(rect) (köşeleri alan biçimlendirir). Kendi yuvarlaması olan şekil
// (ör. yüzen OSD hapı): BlobShape::new(rect, radii); boşluğa açılan köşede r ile büyüğü görünür.
// .with_deform(m, anchor): kenara yapışık panelde çapa o kenarın ortası olmalı (ör. üst bar için
// (0.5, 0.0)). BlobField::visual_rect(&shape): esneme dahil görünen dikdörtgen — panel içeriği
// buna yerleşir ve kırpılır, böylece iç boşluk her karede sabit kalır. (BlobShape::visual_rect()
// çerçeve bilgisi olmadan aynısıdır; yapışık kenar kuralını bilmez.)
// BlobField::joined_corners(&shape): alanın çizdiği dört köşe (deform + yapışık kenar kuralı).
impl BlobField {
    /// Dolu alanın girdi bölgesi: ayrık, tam piksele hizalı dikdörtgenler (mantıksal piksel,
    /// alanın koordinatları; çerçeveliyse frame.outer ile kırpılı). Gölge dahil değil.
    pub fn input_region(&self) -> Vec<Rect>;
}
```

**Girdi bölgesi (`input_region`)** — ADR-0008'in "girdi bölgesi aynı `H`'den türer" maddesi.
Çizimle aynı sahneden (aynı paneller: deform, yapışık kenar kuralı, 1/256 px ızgarası; 1.0
ölçekte) `H` kurulur. Her mantıksal piksel satırının merkezinde (`y + ½`) `H`'nin ilkelleriyle
kesişimler, shader ve `Region::inside` ile aynı yarı açık kuralla bulunur. Çift-tek ile serbest
aralıklar çıkar, `clip` içindeki tümleyen o satırın dolu parçalarıdır. Aynı `[x0, x1)` aralığı
alt satırda sürerse dikdörtgen aşağı uzar. Böylece düz bölgeler büyük dikdörtgen olur, yalnızca
eğri sınırın geçtiği satırlar 1 px yüksekliğinde şerittir.
- **Piksel kuralı:** piksel merkezi dolu alandaysa piksel bölgededir. Bu, 1.0 ölçekte GPU
  kapsaması ≥ 0.5 ile aynıdır; bölge sınırı çizilen kenardan en çok ±0.5 px (yarım AA pikseli)
  sapar. Birleşim filetoları (R) bölgededir, boşluğa açılan `r` köşelerinin dışı ve boğaz
  cepleri değildir. Kesirli ölçekte geometri aynıdır, karar mantıksal piksel merkezinde verilir.
- **Maliyet:** 3 panel/1080p (bar'dan inen panel, sağ tam boy çekmece, sol alt köşe paneli,
  R = 20, r = 12): ≈ 160 µs/çağrı (release; `free_area` ≈ 110 µs + satır taraması ≈ 40 µs),
  86 dikdörtgen. Yalnız çerçeve en çok 4 + 4·R dikdörtgendir. Önbelleği yoktur, çağıran
  yalnızca şekil değişince çağırır.
- **Testler:** her pikselde CPU referansıyla birebir (`H`'nin dışı ⇔ bölgede; altın sahneler,
  esneyen/büzülen yan çekmece, çerçevesiz hap, 1080p), ayrıklık ve tam piksel, GPU kapsaması
  (alfa > ½ ⇔ bölgede; ½'ye ±4/255 yakın AA pikselleri hariç).

**Shell için kullanım:** katmanın `wl_surface.set_input_region`'ı dikdörtgenden kurulan bölge
yerine `field.input_region()` ile kurulur (her dikdörtgen `wl_region.add`). Bölgeyi blob dışı
etkileşimli yüzeyler (ör. bildirim popup'ı, alan dışında çizilen bileşen) için ayrıca birleştirmek
çağıranın işidir. Yeniden hesaplama: alanın şekli değişince, yani şekil listesi, `rect`,
`deform`, `smoothing`/`corner_radius` ya da çerçeve değişince. Animasyon sürerken her karede
gerekmez: hareket bitince (yay oturunca) son şekille bir kez, hareket başlarken de hedefin
kapsayıcısı (ör. eski bölge ∪ hedef `visual_rect`) ile hesaplanabilir. Alan eşitliği
(`BlobField: PartialEq`) önbellek anahtarı olarak yeter.

**Yapışık kenar deformla kopmaz:** deformsuz hâlde çerçevenin iç kenarına yapışık (ya da onu
aşan) şekil kenarı deformdan sonra da o kenarda kalır; esneme yalnızca serbest kenarları
oynatır. Kenar boyunca uzanan çekmece alan koruyan büzülmede bar'dan/çerçeveden kopmaz
(`prepare`, `damage_rect` ve `visual_rect` aynı köşeleri kullanır).

Şekil kuralı (ADR-0008): serbest alan `F` = çerçevenin içi eksi şekiller;
çizilen serbest alan `H = closing(opening(F, R), r)`. Panel çerçeveye ya da başka bir panele
birleşiyorsa köşe `R` yarıçaplı içbükey filetodur, boşluğa açılıyorsa `r` yarıçaplı dışbükey
yaydır; çerçevenin iç köşeleri `R`'dir. Özel dal yok: üst, alt, yan, köşe panelleri ve çoklu
panel aynı tanımdan çıkar. Süreklilik düzenlemesi (hareket sırasında sıçrama yok):
- **Dar boğaz** (karşılıklı paralel iki kenar, aralık `d < 2R`): kesin açma onu bir karede
  kapatırdı. Bunun yerine boğaz, tüm örtüşme boyunca birlikte incelen bir cep olarak kalır.
  Cebin yarı kalınlığı `h = d/2 · smoothstep((d − R)/R)`: `d = 2R`'de kanalın tamamı,
  `d ≤ R`'de sıfır (birbirine `R`'den yakın kenarlar tek parça; kıl kadar ince yarık yok). Ağızlarda cep dışarı açılır. Ağızdaki fileto kesin açmanın kendi
  filetosudur (panel değmiş gibi), tümsek ya da sivri uç oluşmaz. Cep `r` kapanışından
  sonra eklenir, böylece kıl kadar ince bir yarık köşe yuvarlamasını kesmez. Birleşimleri
  `min(r, 4h)` ile kapanır.
- **İnce şekil** (kalınlığı `2r`'den az): silinmez, ucu yarı kalınlık yarıçaplı kapsül olur.
- Sınırın hareketi piksel kapsamasında süreklidir. Köprünün `r` topunu sığdırdığı anda
  karekök biçimli, dik ama sürekli bir geçiş vardır.
- Bilinen sınır: düzenleme tekdüze değildir. Yeni bir panelin açtığı dar cebin ucu `d/2`
  yarıçaplıdır ve `R` filetosunun doldurduğu köşeye ondan daha çok girer.

Uygulama: sınır CPU'da (f64) doğru parçası ve yay olarak kesin hesaplanır. Morfoloji, işaretli
uzaklığın düzey kümesi olarak ofset ve kırpmayla yapılır. Girdiler 1/256 fiziksel piksel
ızgarasına oturtulur. Renderer sonucu sahne anahtarıyla önbelleğe alır: değişmeyen karede CPU işi
yoktur, doku yalnızca değişince yüklenir. Tipik maliyet şekil değişen karede 3 panel/1080p için
≈ 110 µs, 16 şekil için ≈ 1 ms. Shader ilkellerden (RGBA32F doku; uçlar paylaşılan köşede
yapıştırılmış) kesin işaretli uzaklığı ve yatay ışın çift-tek kuralıyla içeriyi bulur. Kapsama
gradyanla normalize edilir (1 fiziksel piksel AA, kesirli ölçekte de). Gölge aynı alanın ofsetli,
bulanık örneğidir. **Tam ekran tek quad çizilmez**: yalnızca çerçeve şeritleri ve her şeklin
`2(R + r)` + gölge payı kadar genişletilmiş sınır kutusu rasterize edilir. `damage_rect` aynı
payı kullanır. Bu dikdörtgenlerin serbest alan sınır kutusundan `SOLID_MARGIN` (2 fiziksel px)
uzak parçaları alan gölgelendiricisi yerine düz dolguyla (`blob_solid.frag`, aynı `u_fill`)
çizilir: orada kapsama kesin 1 ve gölge terimi yok; sonuç kanal başına en çok 1/255 farkla aynı
(`solid_split_matches_full_sdf`). Bar ve çerçeve şeritleri piksel başına tüm ilkelleri dolaşmaz
(llvmpipe'ta 1920 px bar şeridi hasarı ~5 ms → ~0,3 ms CPU).

#### Görseller

```rust
pub struct ImageData { pub width: u32, pub height: u32, pub pixels: Arc<[u8]> } // premultiplied RGBA8, sıkı satırlar
pub struct TextureOptions { pub mipmaps: bool }            // LINEAR_MIPMAP_LINEAR; yoksa LINEAR
#[derive(Clone)] pub struct Texture;                        // Arc tutamağı
impl Texture { pub fn size(&self) -> (u32, u32); pub fn is_ready(&self) -> bool; }
pub enum ImageFit { Cover, Contain, Fill }
impl ImageFit {
    /// `Painter::image` ile aynı yerleşim: çizilen dikdörtgen + doku koordinatları [u0, v0, u1, v1]
    /// (Contain'de letterbox'lı küçük dikdörtgen, Cover'da kırpılmış uv). Yalnızca en-boy oranı kullanılır.
    pub fn placement(self, rect: Rect, texture: (u32, u32)) -> (Rect, [f32; 4]);
}
pub struct ImageStyle { pub fit: ImageFit, pub radii: Corners, pub opacity: f32,
                        pub tint: Option<Color> }           // tint: çıktı = renk × doku alfası
pub const MAX_TEXTURE_SIDE: u32 = 8192;                    // ve GL_MAX_TEXTURE_SIZE (new'de bir kez sorgulanır)
pub const UPLOAD_BUDGET_PER_FRAME: usize = 64 << 20;

impl Painter<'_> {
    pub fn upload_texture(&mut self, image: ImageData, opts: TextureOptions) -> Texture;
    pub fn update_texture(&mut self, texture: &Texture, image: ImageData); // aynı boyut: TexSubImage
    pub fn image(&mut self, rect: Rect, texture: &Texture, style: &ImageStyle);
    /// Bulanık kopya (dual Kawase, aynı boyut, kenar koşulu ayna); kaynak hazır olduğu karede GPU'da üretilir.
    /// Önbellekle: kaynak/yarıçap değişmedikçe tekrar çağırma. `radius` mantıksal piksel.
    pub fn blur_texture(&mut self, source: &Texture, radius: f32) -> Texture;
}
impl Renderer {
    pub fn texture_stats(&self) -> (usize /* doku */, usize /* ≈ bayt */);
    pub fn has_pending_uploads(&self) -> bool;   // true → runtime bir kare daha ister
}
```

- Painter GL çağırmaz: yükleme kaydedilir, `render_frame` çizimden önce (erken dönüşte de)
  gönderir. Kare başına `UPLOAD_BUDGET_PER_FRAME` bayt (en az bir yükleme); aşan sonraki
  karelere kalır, `is_ready()` o zamana dek `false` ve doku çizilmez.
- Tutamak `Arc`'tır; son klon düşünce id serbest bırakma kuyruğuna gider ve bir sonraki
  `render_frame` başında GL dokusu silinir (hangi thread'de düşerse düşsün). Bir karede çizilen
  doku o kare boyunca canlı tutulur. Renderer düşerken kalan dokular silinir; renderer'dan uzun
  yaşayan tutamak sessizce düşer.
- Çizim: doku başına ayrı draw (program `image`), `fit`'e göre uv kırpma (Cover) ya da ortalanmış
  hedef (Contain), köşe SDF'iyle 1 fiziksel piksel AA kırpma, premultiplied × `opacity`.
  `painter.image` hasar bildirmez; değişen görseli uygulama bildirir.
- Geçersiz veri (uzunluk ≠ w·h·4) ya da sınırı aşan boyut yüklenmez, uyarı loglanır.

### Hasar takibi

```rust
impl Painter<'_> {
    pub fn damage(&mut self, rect: Rect);   // bu karede değişen bölge (mantıksal); Region'a eklenir
    pub fn damage_all(&mut self);
}
pub struct FrameTarget { pub size_px: (u32, u32), pub scale: f32, pub buffer_age: u32 }
impl Renderer {
    /// Yüzey başına DamageHistory; dönen değer bu karenin hasarı (swap_with_damage için).
    pub fn render_frame(&mut self, text: &mut TextSystem, history: &mut DamageHistory,
                        target: FrameTarget, f: impl FnOnce(&mut Painter)) -> Repaint;
}
pub enum Repaint { Full, Partial(Region) }   // NOTHING = boş Partial; rect(r); region(size)
pub struct Region;   // ≤ MAX_DAMAGE_RECTS (8) ayrık PxRect; Copy; add, union, clipped, rects, bounds, area
                     // PxRect fiziksel, sol-üst; to_egl(h) sol-alt
impl BlobField { pub fn damage_rect(&self, shape: &BlobShape) -> Rect; }
```

Uygulama karede hiç `damage` çağırmazsa tüm yüzey çizilir (eski davranış). Çağırırsa yine
her şeyi kaydeder; renderer birikmiş hasara (buffer age: son *n* karenin bölgelerinin
birleşimi) scissor uygular. Hareket eden şeklin **eski ve yeni** `damage_rect`'i birlikte
bildirilir; taşan efektlerin (gölge, blob filetosu, bulanıklık) payı her `damage` çağrısında
o dikdörtgene eklenir, parçalar bu payla tutulur.

**Çoklu bölge.** `damage` çağrıları tek kutuda birleşmez: `Region` ayrık dikdörtgenler tutar
(bar'ın ortasındaki saat ve sağ ucundaki sistem izleyici aynı karede değişince ~900 px'lik kutu
yerine iki küçük parça). Kurallar: örtüşen ya da kenar paylaşan (yalnızca köşe değmesi
sayılmaz) dikdörtgenler sınırlayıcı kutularında birleşir, kutunun eklediği boş alan ≤ 32×32 px²
ise de birleşir (ayrı geçişin sabit maliyeti); birleşen kutu yeni komşularla örtüşürse onlar
da toplanır, parçalar böylece hep ayrıktır (hiçbir piksel iki kez harmanlanmaz). En çok
`MAX_DAMAGE_RECTS` (8) parça; aşılınca birleşince alanı en az büyüyen çift birleşir. Bölge
eklenenlerin tamamını kapsar. Renderer kaydı bir kez hazırlar (örnek yüklemeleri, uniform'lar,
blob geometrisi ve düz dolgu bölmesi) ve komut listesini her parça için o parçaya scissor'la
yeniden gönderir (uygulamanın clip'iyle kesişim). Kısmi karede her komutun korumacı kapsamı
(örnek dikdörtgenleri + gölgelendiricideki genişletme + 2 px) hesaplanır; parçayla
kesişmeyen komut o geçişte gönderilmez — geçiş başına maliyet yalnızca o parçaya düşen
çizimler (llvmpipe'ta iki parçalı karede GL gönderimi ~870 → ~270 µs). Ölçüm (`damage_bench
DEMO_DAMAGE=two`, 1920×1080, ortada saat + sağda sistem izleyici): tek kutuya göre süreç
CPU'su kare başına shm 1,60 → 1,07 ms, EGL 4,9 → 3,8 ms. Buffer age birikimi parça kümelerinin
birleşimidir. Runtime: `EGL_EXT_buffer_age` ile yaşı okur (yoksa 0 = tam kare) ve varsa
`eglSwapBuffersWithDamage` ile bu karenin parçalarını (n dikdörtgen) compositor'a bildirir;
yazılım sunumunda her parça ayrı okunup kopyalanır ve ayrı `damage_buffer` olur.

### Stil tipleri

```rust
pub struct RectStyle { pub fill: Color, pub radii: Corners, pub border: Option<Border> }
pub struct Border { pub width: f32, pub color: Color }          // içe çizilir
pub struct ShadowStyle { pub color: Color, pub blur: f32, pub offset: Point, pub spread: f32 }
pub enum RenderError { Gl(String), Compile{..}, Link{..}, Unsupported(String) }
```

Renkler API'de düz (straight) alpha'dır; premultiply renderer içinde yapılır.
`Renderer::render` bağlı framebuffer'a çizer, `swap_buffers` çağıranın (runtime) işidir.
`nuxut_render::glow` yeniden dışa aktarılır (runtime aynı sürümü kullansın diye).

### Metin

`TextSystem` = `cosmic-text` `FontSystem` + `SwashCache`; GL glyph atlas'ı `Renderer`
içindedir (GL context'ine bağlı). `TextLayout`
`text.layout(&str, &TextStyle, max_width: Option<f32>) -> TextLayout` ile üretilir,
`size()`/`baseline()` verir, ölçekten bağımsızdır ve çağıran tarafından önbelleklenir (her
karede yeniden şekillendirme yapılmaz).

**Optik ortalama** (ADR-0009): `TextStyle::optical` (`Auto` varsayılan) glifleri layout kutusu
içinde kaydırır, kutu boyutu değişmez: metin büyük harf bandıyla (`CapHeight`), tek glif ikon
(PUA / ligatür) mürekkep kutusuyla (`Ink`) ortalanır. Kutuyu hücrede ortalamak yeterlidir; ek
ofset yazılmaz. Ölçek 1.0'da glifler ipuçlanır ve taban çizgisi tam piksele yuvarlanır (netlik);
diğer ölçeklerde ipucusuz, iki eksende çeyrek piksel alt konumla yerleşir
(`text::place_glyph`); `opsz` mantıksal boyuttan gelir.

**Font önbelleği** (`font_cache.rs`): `TextSystem::new` tam taramanın sonucunu (yüz meta verisi +
fontdb genel aileleri, `resolve_generic_families` ÖNCESİ) `dirs::cache_dir()/nuxut/fonts-v1.bin`
dosyasına yazar; sonraki açılışlar font dosyalarını ayrıştırmadan `push_face_info` ile aynı
veritabanını kurar (ölçüm: sıcak ~40 ms → ~0.7 ms, soğuk disk ~1.3 s → ~0.7 ms; 890 yüz,
114 KB). Geçersizleştirme: fontconfig config dosyaları/dizinleri ve tüm font dizinleri + alt
dizinlerinin mtime'ı; sürüm etiketi `fontdb-0.23+cosmic-text-0.19` + biçim sürümü. Bozuk/kesik/
uyumsuz dosya sessizce tam tarama (sınırlar: 8 MiB, sağlama toplamı). Yazım atomik, 0600.
**Bilinen sınır:** yeniden adlandırılmadan yerinde değiştirilen font dosyası fark edilmez.
Kaçış: `NUXUT_NO_FONT_CACHE=1` (okumaz, tam tarar, yeniden yazar) ya da dosyayı silmek.
Linux'ta (`font_scan.rs`, `docs/design/font-cold-start.md`): tam tarama fontdb ile aynı sıra ve
sonuçla paralel ayrıştırır; eski önbellekte kaydı değişmemiş dizinlerin yüzleri yeniden kullanılır,
yalnızca değişen dizinler taranır; `TextSystem::new` sonunda kısa ömürlü bir iş parçacığı font
dosyalarının başını `fadvise(WILLNEED)` ile ister (cosmic-text'in her yeni öznitelikte tüm yüzlere
bakması soğuk diskte ilk şekillendirmeyi bekletmesin).

```rust
pub struct TextStyle { pub family: String, pub size: f32, pub weight: u16,
                       pub line_height: f32, pub align: TextAlign, pub max_lines: Option<usize>,
                       pub fill: f32 /* değişken font FILL ekseni 0..=1; with_fill */,
                       pub tabular_numbers: bool /* OpenType tnum; with_tabular_numbers() */,
                       pub fallback: GenericFamily /* aile kurulu değilse; with_fallback */ }
pub enum GenericFamily { SansSerif, Serif, Monospace }
pub struct FamilyInfo { pub name: String, pub weights: Vec<u16> /* dik yüzler, artan */,
                       pub weight_range: Option<(u16, u16)> /* wght ekseni */, pub italic: bool,
                       pub monospace: bool, pub latin: bool /* cmap'te 'a' var */ }
impl TextSystem {
    pub fn has_family(&mut self, name: &str) -> bool;                       // genel aile adları her zaman true; önbellekli
    pub fn glyphs_in_family(&mut self, family: &str, text: &str) -> Option<usize>;  // yedeğe düşmeden glif sayısı (ikon testleri)
    pub fn families(&mut self) -> &[FamilyInfo];            // kurulu aileler, ada göre sıralı, önbellekli; ilk çağrı aile başına bir yüzün cmap/fvar'ını okur (~0.2 sn / 272 aile, debug)
    pub fn is_variable(&mut self, family: &str) -> bool;     // tek yüz + wght ekseni (yalnız o yüzü yükler)
    pub fn effective_family(&mut self, name: &str, fallback: GenericFamily) -> String;  // gerçekte kullanılan aile
}
// Rasterleştirme wght + FILL + opsz (= piksel boyutu) eksenlerini uygular (Material Symbols, Inter 4).
// Aile kurulu değilse `fallback` genel ailesi kullanılır (fontconfig tercihi); uyarı yok, debug log.
```

İmleç ve vuruş testi (TextInput için; bayt konumları mutlak, küme sınırına göre):

```rust
impl TextLayout {
    pub fn line_count(&self) -> usize;
    pub fn caret_rect(&self, byte: usize) -> Rect;   // genişlik 0, satırın üstü/yüksekliği
    pub fn cursor_x(&self, byte: usize) -> f32;      // tek satır kısayolu
    pub fn hit(&self, p: Point) -> usize;            // en yakın imleç baytı
    pub fn hit_byte(&self, x: f32) -> usize;         // tek satır kısayolu
}
```

Zengin metin (bildirim gövdesi, bağlantılar):

```rust
pub struct SpanStyle { pub bold: bool, pub italic: bool, pub underline: bool, pub strikethrough: bool,
                       pub color: Option<Color>, pub link: Option<u32> }
impl TextSystem { pub fn layout_rich(&mut self, spans: &[(&str, SpanStyle)], style: &TextStyle,
                                     max_width: Option<f32>) -> TextLayout; }
impl TextLayout { pub fn link_at(&self, p: Point) -> Option<u32>; }
```
Parça rengi çizim renginin opaklığıyla söner; altı/üstü çizgileri `Painter::text` çizer.

Gizli metin (gösterilen parola; `TextInput` göster/gizle — bkz. "TextInput parola kipi"):

```rust
impl TextSystem { pub fn layout_secret(&mut self, text: &str, style: &TextStyle) -> TextLayout; } // tek parça, sarmasız; ara kopyalar sıfırlanır
impl TextLayout { pub fn wipe(&mut self); } // glif kimlikleri, bayt aralıkları, konumlar zeroize; sonra boş
```

### Tema

```rust
// nuxut-theme
pub struct Theme { pub mode: ThemeMode /* çözülmüş */, pub colors: Palette, pub color_theme: Option<ColorThemeId> /* source = static iken seçili tema, wallpaper iken None */,
                   pub contrast: ContrastMode, pub shape: Shape, pub shadow: Shadow,
                   pub motion: Motion, pub blob: Blob, pub typography: Typography, pub timing: Timing,
                   pub desktop: Desktop, pub appearance: Appearance, pub locale: LocaleConfig }
// [appearance] (kullanıcı kararı, 2026-10-13): arayüz kişiliği. Görsel token sayılır (same_visuals'a dahil →
// değişince theme_changed). Ham şema AppearanceConfig { mascot: bool } varsayılan true; varsayılan dökümde diğer
// tablolar gibi `[appearance] mascot = true` yer alır (istek, 2026-10-14; önceden skip_serializing_if);
// kullanıcı dosyasına kendiliğinden yazılmaz, bölümsüz eski dosya aynen yüklenir; yanlış tür = bozuk dosya.
pub struct Appearance { pub mascot: bool /* false: nuxut_ui::Mascot çizmez, ölçüsü 0 */ }
// [desktop] (shell ADR-0004 §17-1): diğer uygulamaların görünümü; doğrulanmış (geçersiz ad → None + uyarı,
// boy 16..=96 sınırlanır, bilinmeyen qt_backend → Auto + uyarı; uyarıya kullanıcı değeri girmez).
pub struct Desktop { pub unified: bool /* varsayılan true */, pub icon_theme: Option<String> /* None = önerilen */,
                     pub cursor_theme: Option<String>, pub cursor_size: u32, pub qt_backend: QtBackend }
pub enum QtBackend { Auto, Kde, Qt6ct }       // "auto" | "kde" | "qt6ct" ("qtct" eski ad)
pub struct DesktopConfig { pub unified: bool, pub icon_theme: String, pub cursor_theme: String,
                           pub cursor_size: u32, pub qt_backend: Choice<QtBackend> }   // ham şema
pub fn valid_theme_name(&str) -> bool;        // ≤ 128 bayt, `.` ile başlamaz, harf/rakam + " -_.+"
// Dosyayı bozmayan seçenek alanı: bilinmeyen ad/yanlış tür → varsayılan + invalid (doğrulayıcı uyarır).
pub struct Choice<T> { pub value: T, pub invalid: bool }   // serde: yalnızca ad
// [colors] mode ham şeması: "auto" (varsayılan; anahtar yoksa, geçersizse uyarı + auto) | "dark" | "light".
pub enum ColorMode { Auto, Dark, Light }      // fixed() -> Option<ThemeMode> (Auto → None); From<ThemeMode>
// Çözülmüş kip; Theme.mode. nuxut_palette::Mode ile iki yönlü From. Auto'yu sessizce bir kipe
// eşleyen From<ColorMode> bilerek yok: önce ColorsConfig::resolve_mode.
pub enum ThemeMode { Dark, Light }            // name(), from_name(), of_surface(Color) (WCAG Y > ~0.18 → Light)
// [colors] contrast (docs/design/accessibility.md §1): "standard" (varsayılan; anahtar yoksa, geçersizse
// uyarı + standard) | "high". High: Theme.colors = resolve_palette(..).high_contrast() (açık renkler dahil),
// shape.outlines None → Subtle, outline() Full tam opak. Canlı değişim theme_changed ile (same_visuals karşılaştırır).
pub enum ContrastMode { Standard, High }      // name(), is_high(); ChoiceValue
pub mod contrast {                            // WCAG yardımcıları (tema + palet aynı yöntem)
    pub const TEXT_MIN: f32 = 4.5; pub const NON_TEXT_MIN: f32 = 3.0; pub const TEXT_TARGET: f32 = 7.0;
    pub fn relative_luminance(Color) -> f32; pub fn contrast_ratio(Color, Color) -> f32; // kökten de yeniden ihraç
    pub fn worst_ratio(fg: Color, bgs: &[Color]) -> f32;
    // fg'nin HCT tonunu zeminlerden uzağa en az gereken kadar iter (hue/kroma/alfa korunur); geçen fg aynen döner.
    pub fn ensure_contrast(fg: Color, bgs: &[Color], target: f32) -> Color;
    pub fn ensure_pair(fg: Color, bg: Color, target: f32, min: f32) -> (Color, Color); // fg yetmezse bg de itilir
}
// Zamanlama (framework.toml'da değil; DESIGN "Zamanlama"). Hareket değildir: motion.enabled/speed etkilemez.
pub struct Timing { pub hover_delay: Duration /*500 ms*/, pub hover_warm: Duration /*800 ms*/,
                    pub confirm_window: Duration /*4 s*/, pub confirm_guard: Duration /*400 ms*/,
                    pub flash_hold: Duration /*900 ms; "satırı parlat"*/ }  // Timing::DEFAULT
impl Theme {
    pub fn load() -> Result<Loaded<Theme>, ConfigError>;   // <config_dir>/framework.toml
    pub fn load_from(path: &Path) -> Result<Loaded<Theme>, ConfigError>;
    pub fn from_config(&FrameworkConfig) -> (Theme, Vec<String>);  // doğrula + sınırla
    pub fn parse(&str) -> Result<(Theme, Vec<String>), String>;
    pub fn broken_config() -> Theme;   // default + desktop.unified = false: dosya VAR ama okunamadı/ayrıştırılamadı
    pub fn reduced_motion(&self) -> bool;  // !motion.enabled: Hareket "Kapalı" (preset instant) ya da enabled = false (t124)
}
pub const CONFIG_NAME: &str = "framework";
// Birleşik görünüm kuralı: dosya ya da anahtar yok → unified true (varsayılan); dosya bozuk
// (sözdizimi, yanlış tür — ör. `unified = "no"` —, çok büyük, G/Ç) → load* Err → çağıran
// Theme::broken_config() (kapalı: dış hedef yazılmaz). Runtime açılışta böyle düşer; çalışırken
// gelen hata son geçerli temayı korur. Kendi yükleyen tüketici (shell/settings) aynı kuralı uygular.

// Tipografi (ADR-0006): rol başına aile + ağırlık + ölçek.
pub struct Typography { pub size: f32, pub icon_set: IconSet,
                        pub ui: FontRole, pub display: FontRole, pub document: FontRole,
                        pub mono: FontRole, pub icon: FontRole }
pub struct FontRole { pub family: String /* icon'da boş = setin fontu */, pub weight: u16, pub scale: f32 }
pub enum IconSet { Material, Nerd }
impl Typography { pub fn icon_family(&self) -> &str;  // icon.family ya da setin fontu (nerd → mono.family)
                  pub fn text_scale(&self) -> f32; }  // size / TEXT_SIZE_REFERENCE (13 px = 1.0); dış aktarım nuxut_palette::text_scale
pub const TEXT_SIZE_REFERENCE: f32 = 13.0;            // [typography] size varsayılanı
```
Eski şema (`family`, `weight`, düz dize `mono`/`icon`) uyarıyla yeni rollere eşlenir.

```rust
// Hareket ön ayarları (DESIGN.md "Hareket ön ayarları"; varsayılan Calm).
pub enum MotionPreset { Calm, Lively, Snappy, Instant }
impl MotionPreset { pub const ALL: [Self; 4]; pub fn name(self) -> &'static str;
                    pub fn from_name(&str) -> Option<Self>; pub fn values(self) -> PresetValues; }
pub struct PresetValues { pub enabled: bool, pub style: MotionStyle, pub spatial: RoleValues,
                          pub spatial_fast: RoleValues, pub effects: RoleValues, pub deform: f32 }
pub struct RoleValues { pub response: f32, pub damping: f32, pub easing: &'static str, pub duration_ms: f32 }
// Ham config: ön ayar + yalnızca AÇIK anahtarlar (None = ön ayardan).
pub struct MotionConfig { pub preset: MotionPreset, pub enabled: bool, pub speed: f32,
                          pub style: Option<MotionStyle>, pub spatial: RoleConfig,
                          pub spatial_fast: RoleConfig, pub effects: RoleConfig, pub unknown_preset: bool }
pub struct RoleConfig { pub style: Option<MotionStyle>, pub response: Option<f32>, pub damping: Option<f32>,
                        pub easing: Option<String>, pub duration_ms: Option<f32> }   // is_empty()
impl MotionConfig { pub fn for_preset(MotionPreset) -> Self; pub fn is_customized(&self) -> bool; }
pub struct BlobConfig { pub smoothing: f32, pub deform: Option<f32> /* None = ön ayarınki */ }
```
Ezme anahtar başınadır; `enabled = false` ya da `Instant` → tüm roller `Motion::INSTANT`, deform 0.

**Azaltılmış hareket sözleşmesi (t124, docs/design/accessibility.md §2):** `Theme::reduced_motion()`
tek kaynaktır. `true` iken her bileşen durum değişikliğinin ilk karesinde hedef hâli çizer ve ek
kare istemez; sonsuz göstergeler durağan alternatif çizer (Spinner), kaydırmada atalet yoktur,
kapanan blob kabın oturmuş tohumu çizilmez. Kanıt: `nuxut-ui/src/reduced_motion_tests.rs`. Yeni
bileşen hareketini yalnızca tema rolleriyle (`theme.motion.*`, `theme.blob.*`) kurar; sabit
`Spring::new`/süre ya da `now − başlangıç` döngüsü yazarsa `reduced_motion()`a bakar ve o dosyaya bir
test ekler. Bilinçli istisnalar: CurveEditor önizlemesi (düzenlenen eğrinin kendisi, açıkça istenir)
ve ThemePreview animasyonlu imleç önizlemesi (yalnız işaretçi üstündeyken; içerik önizlemesi).
Framework portal okumaz; Nuxut oturumunda portal arka ucu (shell) değeri bundan türetir.
Serileştirme yalnızca açık anahtarları yazar (ön ayar değerleri dosyaya kopyalanmaz).

`theme.motion.{spatial, spatial_fast, effects}` birer `nuxut_motion::Motion`'dır
(`Spring(Spring) | Curve(Curve { easing: CubicBezier, duration })`), `enabled`/`speed`/stil
uygulanmış hâlde. `Animated::new(v, impl Into<Motion>)` ve `set_motion`/`set_spring(impl
Into<Motion>, now)` ikisini de kabul eder; uygulama stili bilmez. Hareket, sapma ve hız eşiğin
altına inince hedefe yapıştırılır (`is_settled`): 0..1 ölçekli değerlerde (`f32`, `Color`) 0,002,
konum/boyut türlerinde (`Point`, `Rect`, `Insets`, `Corners`; mantıksal px) 0,05; hız eşiği 10 katı
(birim/sn). Görünmez kuyruk için kare çizilmez (`spatial` yayında ~700 px panel geçişi ~0,89 → ~0,7 sn;
2026-10-16). Bozuk dosyada `load` hata döner;
runtime son geçerli temayı korur.

**Palet katmanları** (DESIGN.md "Renk katmanları"):

```rust
pub struct Palette { surface, surface_container, …, shadow: Color }   // çözülmüş
pub struct PalettePatch { pub surface: Option<Color>, … }             // kısmi
pub struct ColorsConfig { pub source: ColorSource, pub mode: Choice<ColorMode>, pub contrast: Choice<ContrastMode>, pub theme: Choice<ColorThemeId>, pub surface: Option<Color>, … }
// Kip çözümü (auto): generated (paletin kayıtlı kipi; yalnızca source = wallpaper) → surface
// parlaklığı (açıkça yazılmış surface; yoksa statik varsayılanın yüzeyi) → dark. dark/light olduğu gibi.
impl ColorsConfig { pub fn resolve_mode(&self, generated: Option<ThemeMode>) -> ThemeMode; }
impl Palette { pub const DARK: Self; pub fn light() -> Self; pub fn for_mode(ThemeMode) -> Self; // Nuxut mavisi statik paletleri
               pub fn high_contrast(&self) -> Self;  // yüzeyler aynı; metin ≥ 4.5 (hedef 7), primary/error ≥ 4.5, outline ≥ 3; idempotent
               pub fn success(&self) -> Color; }  // türetilmiş olumlu renk (token değil, config anahtarı yok): SUCCESS_SEED primary'ye
// harmonize (MCU; nuxut-palette success ailesiyle aynı yöntem), ton 80 koyu / 40 açık, sonra üç yüzeye ≥ 4.5'e itilir —
// standart ve yüksek karşıtlıkta metin olarak da geçer. Saf hesap; sık çizen bileşen paletle önbelleğe alır (Notice alır).
pub const SUCCESS_SEED: Color;  // tek kaynak; nuxut_palette::SUCCESS_SEED bunu yeniden verir (2026-10-13)
pub enum ColorRole { Surface, SurfaceContainer, SurfaceBright, OnSurface, OnSurfaceVariant, Primary, OnPrimary, Error, OnError, Outline, Shadow }
impl ColorRole { pub const ALL: [Self; 11]; pub fn name(self) -> &'static str /*[colors] anahtarı: "surface_container"*/;
                 pub fn from_name(&str) -> Option<Self> /*config/IPC; boşluk kırpılır*/ }
impl Palette { pub fn get(&self, ColorRole) -> Color; }  // bileşene renk değil token verilir; renk her karede temadan (tema geçişini izler)
pub fn resolve_palette(&ColorsConfig, ThemeMode, Option<&PalettePatch>) -> Palette;  // contrast = high → .high_contrast()
pub enum ColorSource { Static, Wallpaper }
// Nuxut renk temaları ([colors] theme, docs/design/color-themes.md; kullanıcı isteği 2026-10-16). Adlar kalıcı;
// görünen ad uygulama kataloğunda (color-theme-<ad>). Bilinmeyen: uyarı + Blue. Temel katman = seçili temanın paleti.
pub enum ColorThemeId { Blue /* varsayılan */, Neutral, Sage, Rose, Amber, Graphite }  // ALL, name(), from_name(), theme(); ChoiceValue
pub struct ColorTheme { pub id, pub seed: Color, pub primary: Tonal, pub secondary, pub tertiary, pub neutral, pub neutral_variant, .. }
pub struct Tonal { pub hue: Hue { relative: bool, degrees: f64 }, pub chroma: f64 }   // HCT ton çizgisi
pub static BLUE: ColorTheme; /* NEUTRAL, SAGE, ROSE, AMBER, GRAPHITE */
impl ColorTheme {
    pub fn palette(&self, ThemeMode) -> Palette;          // 11 token (tonal-spot ton kuralları); Blue koyu = Palette::DARK, açık = light()
    pub fn dynamic_scheme(&self, dark: bool, contrast: f64) -> material_colors::DynamicScheme;
    pub fn recolor(&self, reference: &ColorTheme, Color) -> Color; } // reference ile çizilmiş rengi taşır: ton aynen, hue/kroma
    // kromaya göre nötr↔birincil palet farkı (smoothstep); self == reference → kimlik; alfa korunur
pub fn recolor_svg(svg: &[u8], map: impl Fn(Color) -> Color) -> Vec<u8>;  // yalnızca fill/stroke/stop-color/flood-color/
    // lighting-color/color özniteliklerinin tırnaklı #hex değerleri; href/url(#…) dokunulmaz (yerleşik duvar kağıtları)
pub struct GeneratedPalette { pub meta: PaletteMeta /* source, scheme, mode, contrast, image */,
                              pub colors: PalettePatch, pub dark: Option<PalettePatch>, pub light: Option<PalettePatch>,
                              pub material: BTreeMap<String, Color> /* ≤ 128 */ }
impl GeneratedPalette { pub fn colors_for(&self, ThemeMode) -> (&PalettePatch, bool /* başka kipte üretilmiş */);
                        pub fn recorded_mode(&self) -> Option<ThemeMode>; // meta.mode, yoksa [colors] yüzeyi
                        pub fn load(&Path) -> …; pub fn save(&self, &Path) -> io::Result<()> /* atomik */ }
pub fn palette_path() -> Option<PathBuf>;           // $XDG_STATE_HOME/nuxut/palette.toml
impl Theme {
    pub fn load() -> …;                             // framework.toml + palette.toml katmanı
    pub fn load_layers(config: &Path, palette: Option<&Path>) -> …;
    // Önerilen: palet dosyasıyla; auto'da kip paletin kayıtlı kipinden, sonra o kipin bölümü.
    pub fn from_generated(&FrameworkConfig, Option<&GeneratedPalette>) -> (Theme, Vec<String>);
    // Yalnızca yama elde varken: auto'da paletin kipi yamanın surface parlaklığından.
    pub fn from_layers(&FrameworkConfig, Option<&PalettePatch>) -> (Theme, Vec<String>);
}
```

- Yazan: yalnızca palet üreticisi (nuxut-media), `GeneratedPalette::save` ile.
- Uzun ömürlü süreçler `ThemeLoader::new()` + `loader.load()` kullanır: bozuk `palette.toml`
  → **son geçerli palet korunur** (uyarı); dosya silinirse palet katmanı kalkar. Durumsuz
  `Theme::load` bozuk paleti yalnızca atlar.
- Okuyan: `Theme::load`. Runtime `palette.toml`'un dizinini de izler; değişince `Theme::load`
  → değiştiyse `App::theme_changed` (framework.toml değişimiyle aynı yol).
- **Tema geçişi:** `nuxut_theme::AnimatedTheme` — `set(theme, now)` ile yeni tema; çizimde
  `colors(now)`; `is_animating(now)` iken kare iste. Renkler `motion.effects` ile akar, renk
  dışı token'lar hemen değişir; geçiş ortasında yeni tema gelirse o anki renklerden devam eder.

### Görseller (nuxut-image)

Tüm pikseller **premultiplied RGBA8**. Dış girdi (dosya, D-Bus pixmap) `Limits` ile sınırlı:
boyut/piksel/dosya baytı önce başlıktan denetlenir (dekompresyon bombası tahsis yapılmadan
reddedilir), animasyon kare sayısı sınırlı.

```rust
pub struct Image;            // w, h, premultiplied RGBA8 — from_straight / from_premultiplied
pub struct Limits { max_file_bytes, max_width, max_height, max_pixels,
                    max_decode_bytes /* çözücü bellek bütçesi: başlıktan tahmin (tampon + 8 bit dönüşüm + çıktı;
                                        JXL_BYTES_PER_PIXEL = 28 B/px, ölçüm) aşarsa TooLarge; image::Limits::max_alloc */,
                    max_frames, max_animation_bytes, max_animation_pixels /* aşılırsa kareler akıtılır */,
                    trust: Trust }   // DEFAULT (640 MB bütçe), ICON (128 MB), WALLPAPER (256 MiB dosya, 16K kenar,
                                     // 16K² kaynak, 1.5 GiB bütçe); hepsi Untrusted; .with_trust(Trust)
                                     // check_dimensions(w, h) / check_budget((w, h), tahmin)
// Tek sınır kapısı: her biçim (PNG/JPEG/WebP/GIF/APNG/JXL/AVIF/SVG), animasyon karesi, akış ve
// ham piksel (Raw/Argb32) isteğin `Limits`'inden geçer: başlıktan boyut → çözücü bellek
// sınırı → çözüm sonrası boyut doğrulaması. Kare süresi `MAX_FRAME_DELAY` (60 sn) ile sınırlı.
pub enum Fit { Cover, Contain, Fill }
pub struct TargetSize { width, height, fit, crop: bool } // fiziksel px; çözme yalnızca KÜÇÜLTÜR. new(w, h, fit): crop=false;
// cover_crop(w, h): Cover + taşan eksen ORTADAN kırpılır, kırpma küçültmeden önce kaynak dikdörtgende
// (fast_image_resize crop, tek geçiş): çıktı her iki eksende ≤ kutu, çizimdeki ImageFit::Cover ile aynı bölge.
impl Image { pub fn fit(self, TargetSize) -> Result<Image>; pub fn resized(&self, TargetSize) -> Result<Option<Image>>; }
pub struct DecodeOptions { limits, target: Option<TargetSize>, animate: bool }  // trust() / with_trust(Trust)
/// Kaynağın güveni (biçime göre değil). Güven belirtmeyen her çağrı `Untrusted` (varsayılan).
/// Güven `Limits` içinde taşınır: `..Limits::ICON` ile kurulan istek güvenli varsayılanı alır.
pub enum Trust {
    System,     // sistem ikon temasından bizim seçtiğimiz adla bulunan dosya, gömülü varlık. Kullanıcının duvar
                // kağıdı System DEĞİL: yalıtılmış işçide (aşağıda "Duvar kağıdı yalıtımı")
    Untrusted,  // başka süreçten gelen ad/yol/bayt: tepsi IconName/IconThemePath, dbusmenu icon-name/icon-data,
                // bildirim image-path/app_icon/image-data, MPRIS artUrl
}
// Güven kapısı: `Untrusted` istekte SVG süreç içinde HİÇ ayrıştırılmaz; `Loader::with_sandbox`
// kuruluysa güvenilmeyen kodlanmış görsel yalıtılmış işçide çözülür (aşağıda "Görsel yalıtımı",
// ADR-0007). Biçim yalnızca içerik baytlarından tespit edilir; uzantıya bakılmaz
// (".png" adlı SVG, ".svg" adlı PNG). Baştaki BOM/boşluktan sonra `<` ile başlayan (SVG/XML,
// uzun yorum/DOCTYPE dahil) ya da gzip (svgz) içerik → `ImageError::Unsupported("SVG")`;
// JXL/AVIF → `Unsupported("JXL"/"AVIF")`; imzası tanınmayan içerik → `UnknownFormat`. Süreç
// içinde `Untrusted` yolda YALNIZCA temel raster (PNG/APNG/JPEG/WebP/GIF/QOI;
// `SniffedFormat::in_process_fallback`) çözülür, aynı `Limits` kapısından, ve YALNIZCA ilk kare
// (`animate` yok sayılır; K5). Animasyon yalnızca `System` güvende.
// İkon (`Source::Icon`, `IconTheme::load_as`): dosyanın güveni isteğinkini ASLA aşmaz —
// `Untrusted` istekten çözülen dosya her zaman `Untrusted`. `System` istekte dosya ancak
// (1) ad geçerli bir tema adıysa (tek bileşen: `/`, `..`, NUL/denetim karakteri, baştaki `.`
// ve mutlak yol reddedilir; yol/`file://` ad → `Untrusted`), (2) ev dizini dışındaki bir
// arama dizinindeyse ve (3) dosya ile `/`'a kadar her üst dizin root'un ve grup/diğerleri
// yazamıyorsa ve (4) Flatpak/Snap dışa aktarımı değilse (`/var/lib/flatpak`, `/var/lib/snapd`,
// `/snap`, yolda `flatpak` bileşeni) `System` kalır. Uygulamanın verdiği `IconThemePath`
// (`with_dirs`'e konsa bile), `~/.icons`, `~/.local/share/icons`, `/tmp` güveni yükseltemez.
// TOCTOU yok: dosya bir kez açılır, güven o fd'den (`fstat`, `/proc/self/fd` gerçek yolu)
// denetlenir ve aynı fd'den çözülür. Her dosya (`decode_file`, ikon) `O_NONBLOCK` açılır ve
// fd'den normal dosya olduğu doğrulanır: tema dizinine bırakılan FIFO/aygıt thread'i
// bloklamaz (`ImageError::Io`). Unix dışında hiçbir tema dosyası `System` sayılmaz.
pub enum Decoded { Still(Image), Animated(Animation /* kareler + gecikme */),
                   Streamed(AnimationStream /* arka plan thread'i, 2 karelik kuyruk */) }
pub fn decode_file(&Path, &DecodeOptions) -> Result<Decoded, ImageError>;  // PNG/JPEG/WebP/GIF/APNG/JXL/SVG
pub fn decode_bytes(&[u8], &DecodeOptions) -> Result<Decoded, ImageError>;
pub fn is_animated(&[u8], &Limits) -> Result<bool, ImageError>; // piksel çözmeden (GIF blok yürüyüşü, APNG acTL, WebP VP8X); aynı kapı: dosya boyutu + başlıktaki tuval
pub fn animation_info(&[u8]) -> Option<AnimationInfo { frames: u64, duration }>; // animasyonlu GIF/APNG/WebP: kare sayısı + bir tur süresi, yalnızca kap yapısından (GIF blok + GCE, APNG fcTL, WebP ANMF; piksel çözülmez, ayırma yok); gecikme kuralı oynatıcınınki (≤ 10 ms → 100 ms) ama üst sınırsız (içe aktarma bütçesi)
pub fn is_image(&[u8]) -> bool;                                 // yalnızca biçim imzası
pub fn render_svg(&[u8], TargetSize) -> Result<Image, ImageError>;   // yalnızca System girdi; kırpılmış Cover (cover_crop): tuval = kutu, taşan eksen ortadan kırpılır

pub struct IconTheme;        // Send + Sync, önbellekli (dizin listeleri + sonuçlar)
impl IconTheme {
    pub fn system() -> Self;                       // tema adı: GTK4/3 settings.ini → qt6ct/qt5ct → kdeglobals → gtkrc-2.0
    pub fn lookup(&self, name, size, scale) -> Option<PathBuf>;   // spec: tam eşleşme → en yakın → Inherits → hicolor → pixmaps
    pub fn lookup_with_fallbacks(&self, name, size, scale) -> Option<PathBuf>;  // a-b-c → a-b → a
    #[deprecated] pub fn load(&self, name, size, scale) -> Result<Image, ImageError>;  // = load_as(.., Untrusted)
    pub fn load_as(&self, name, size, scale, Trust) -> Result<Image, ImageError>;
}
pub fn icon::desktop_icon_name(app_id) -> Option<String>;          // .desktop Icon=
pub fn icon::sni_pixmap_to_image(w, h, argb_network_order) -> Result<Image, ImageError>;
pub fn icon::best_sni_pixmap(sizes, px) -> Option<usize>;
// Kurulu temalar (Ayarlar seçicileri; shell ADR-0004): default_base_dirs önceliği, aynı adda ilk
// dizinin index.theme'i kazanır; bir düzey (özyineleme yok → symlink döngüsü zararsız), ≤ 512 tema,
// dizin başına ≤ 4096 girdi, index.theme ≤ 64 KiB normal dosya (O_NONBLOCK), `default` ve gizli adlar dışarıda.
pub fn icon::installed_icon_themes() -> Vec<InstalledIconTheme>;   // index.theme'de Directories olanlar
pub fn icon::installed_cursor_themes() -> Vec<InstalledCursorTheme>; // cursors/ ya da manifest.hl|toml
pub fn icon::icon_themes_in(&[PathBuf]) / cursor_themes_in(&[PathBuf]);  // kök dizinler parametreli
pub struct InstalledIconTheme { id, name, inherits: Vec<String>, hidden: bool, has_cursors: bool, dir }
pub struct InstalledCursorTheme { id, name, inherits: Vec<String>, xcursor: bool, hyprcursor: bool,
                             hidden: bool /* index.theme Hidden=true; ikon kaydıyla aynı kural */, dir }
// `name`: index.theme Name → (yoksa) hyprcursor manifest `name` → id.
```

**Uygulama ikonları** (`nuxut_image::app_icon`; 2026-10-14, istek — ekran paylaşım
seçicisi, görev çubuğu; tasarım `docs/design/app-icons.md`). `app_id` → `.desktop` → tema ikonu,
UI thread'ini bloklamadan:

```rust
pub struct AppIcons;          // Clone (Arc), Send + Sync; uygulama başına bir tane
impl AppIcons {
    pub fn new(AppIconOptions, notify: impl FnMut() + Send + 'static) -> Self;  // IO yok; thread ilk get'te
    pub fn get(&self, app_id, size /* mantıksal */, scale /* tamsayı, kesirli → ceil */) -> AppIcon;
    pub fn set_theme_name(&self, Option<&str>) -> bool;   // framework.toml [desktop] icon_theme; None = algıla
    pub fn refresh(&self);                                 // tema yeniden algılanır, .desktop yeniden taranır
    pub fn generation(&self) -> u64; pub fn cache_stats(&self) -> (usize, usize);
}
pub enum AppIcon { Pending, Ready(Arc<Image>), Missing }
pub struct AppIconOptions { sandbox: Option<Arc<dyn IsolatedDecoder>>, theme: Option<String>,
                            dirs: Option<AppIconDirs> /* None = from_env, ilk istekte */ }
pub struct AppIconDirs { applications, icons, pixmaps: Vec<PathBuf> }  // from_env() / from_vars(home, data_home, data_dirs)
pub struct DesktopIndex;      // scan(&[PathBuf]) (bloklar), find(app_id) -> Option<(id, icon, DesktopMatch)>, icon_for, is_stale
pub fn app_icon::candidate_names(&DesktopIndex, app_id) -> Vec<String>;
#[cfg(feature = "render")]
pub struct AppIconTextures;   // yüzey başına: get(&mut Painter, &AppIcons, app_id, size, scale) -> Option<Texture>
```

- **Sözleşme:** `get` yalnızca önbelleğe bakar; yoksa işi tek, düşük öncelikli (`nice` 19) arka plan
  thread'ine verir (kuyruk `APP_ICON_QUEUE` = 64; doluysa `Pending` döner, girdi yazılmaz, sonraki
  `get` yeniden dener) ve `Pending` döner. Her sonuçta (bulunamayan dahil) `notify` thread'den
  çağrılır; uygulama `sender.send(Msg::AppIcon)` ile döngüyü uyandırıp yeniden çizer. Geçersiz
  `app_id` (boş, `/`, denetim karakteri, `.` ile başlayan, > 255 bayt) hemen `Missing`. Boşta iş yok.
- **Eşleme** (`DesktopIndex::find`, `Icon=`'u olan ilk kural): `<app_id>.desktop` → küçük harfli
  kimlik → `StartupWMClass` (harf duyarsız) → ters-DNS sonu (`org.gnome.Nautilus` ↔ `nautilus`).
  Kimlik `applications/` altındaki göreli yolun `/` → `-` hâli; aynı kimlikte önce gelen dizin
  kazanır, `Hidden=true` ya da `Type` ≠ `Application` kimliği gölgeler. Dizinler: `$XDG_DATA_HOME`,
  `$XDG_DATA_DIRS`, Flatpak dışa aktarımları (`<data_home>/flatpak/exports/share`,
  `/var/lib/flatpak/exports/share`; listede yoksa eklenir).
- **Aday adlar** (`candidate_names`): `Icon=` → Steam (`steam_app_<n>` → `steam_icon_<n>`) →
  `app_id` → küçük harfi → ters-DNS sonu. Her ad temada **yalnızca tam adla** aranır (kısaltmalı
  yedek uygulama ikonunda yanlış ikon verir); tema zinciri `IconTheme`'inki (seçili tema →
  `Inherits` → `Adwaita` (tema kurulu değilse) → `hicolor` → pixmaps). `Icon=` mutlak yol olabilir.
- **Güven:** istek `Trust::System`; güven yalnızca düşer (`IconTheme::load_as` kuralı: ev dizini,
  Flatpak/Snap dışa aktarımı, mutlak yol → `Untrusted`). `Untrusted` dosya `sandbox` verildiyse
  yalıtılmış işçide çözülür (ADR-0007), verilmediyse SVG reddedilir ve sonraki aday denenir.
  Hedef `size × scale` kare (Contain, yalnızca küçültür), `Limits::ICON`.
- **Sınırlar:** `.desktop` ≤ `MAX_DESKTOP_FILES` (4096) dosya, dosya ≤ 64 KiB (`O_NONBLOCK`, fd'den
  normal dosya), alt dizin derinliği < 3, sembolik bağlı dizin izlenmez, dizin başına ≤ 8192 girdi.
  Önbellek LRU: ≤ `MAX_CACHED_ICONS` (256) girdi ve ≤ `MAX_CACHED_BYTES` (8 MiB) piksel; bekleyen
  girdi atılmaz; negatif sonuçlar da saklanır. `AppIconTextures` yüzey başına ≤ 128 doku (LRU).
- **Geçersizleşme:** `set_theme_name` (değişirse) ve `refresh` nesli artırır, önbelleği boşaltır;
  eski nesilden gelen sonuç atılır, `AppIconTextures` nesil değişince dokularını bırakır. Bir
  `applications` dizininin değişiklik zamanı değişince (uygulama kuruldu/kaldırıldı; yalnızca üst
  dizin) sonraki işte dizin yeniden taranır ve negatif sonuçlar silinir. GTK ikon teması dosyası
  izlenmez: algılanan temayı kullanan uygulama değişiklikte `refresh` çağırır.
- **Loglar:** `app_id`, ikon adı ve yol yazılmaz; `Debug` yalnızca sayılar.
- Eski `icon::desktop_icon_name` (tek dosya, yalnızca tam/küçük harf) korunur; yeni kod
  `AppIcons`/`DesktopIndex` kullanır.

**İmleç temaları** (`nuxut_image::cursor`; Ayarlar önizlemesi — imleci compositor çizer, burada
yalnızca görüntüsü okunur). Her imleç dosyası, sistem dizininden gelse de güvenilmez ayrıştırılır.

```rust
pub struct CursorTheme;      // Send + Sync; zincir ve hyprcursor dizinleri ilk kullanımda bir kez çözülür
impl CursorTheme {
    pub fn new(id) -> Self;                          // arama: XCURSOR_PATH ya da ikon dizinleri + /usr/share/pixmaps
    pub fn with_dirs(id, Vec<PathBuf>) -> Self;
    pub fn chain(&self) -> &[String];                // tema + Inherits (derinlik öncelikli, ≤ 8 derin, ≤ 32 tema, döngüsüz;
                                                     // Inherits virgül/noktalı virgül, tırnaklı ad kabul)
    pub fn has_xcursor(&self) -> bool; pub fn has_hyprcursor(&self) -> bool;
    pub fn find(&self, CursorRole) -> Option<PathBuf>;                 // XCursor: zincirin her temasında tüm eşanlamlılar
    pub fn sizes(&self) -> CursorSizes;                                // Default rolünden: Xcursor TOC / hyprcursor meta (görsel çözülmez)
    pub fn load(&self, CursorRole, size_px: u32, animate: bool) -> Result<Vec<CursorImage>, ImageError>;
}
pub struct CursorSizes { fixed: Vec<u32> /* artan */, scalable: bool /* hyprcursor SVG */ }
pub struct CursorImage { image: Image, nominal: u32, hotspot: (u32, u32), delay: Duration }
pub enum CursorRole { Default, Pointer, Text, Wait, Progress, Help, Crosshair, Move, Grab, Grabbing,
                      NotAllowed, ResizeEw, ResizeNs, ResizeNwse, ResizeNesw }
// CursorRole::ALL, ::PREVIEW (9: default, pointer, text, wait, crosshair, move, not-allowed, ew, nwse);
// names(): CSS/cursor-shape adı → X11 → Qt/KDE → karma adlar; From<CursorShape>.
pub fn cursor::parse_xcursor(&mut (Read+Seek), len, size, animate) -> Result<Vec<CursorImage>, ImageError>;
pub fn cursor::xcursor_sizes(&mut (Read+Seek), len) -> Result<Vec<u32>, ImageError>;
```

- Sıra: rol için zincirin XCursor dosyası; hiçbir temada yoksa zincirin hyprcursor şekilleri
  (iki biçimi de olan temada XCursor tercih edilir: raster, ucuz, güven sorunu yok).
- **Xcursor sınırları:** dosya ≤ 64 MiB (tamamı okunmaz: başlık + TOC, sonra yalnızca seçilen
  boyutun kareleri `seek` ile), TOC ≤ 4096 girdi, kenar ≤ 512, ≤ 128 kare, karelerin toplamı
  ≤ 8 M piksel; taşmasız konum aritmetiği, her parça dosya sınırına karşı; sıcak nokta görsel
  içinde; renk kanalı alfayı aşamaz. Boyut seçimi libXcursor gibi (en yakın, eşitlikte TOC'ta
  önce gelen). Animasyonda sonraki bozuk/farklı boyutlu kare animasyonu keser; ilk kare bozuksa hata.
- **Hyprcursor** (`cursor::hyprcursor`): `manifest.hl|toml` (≤ 64 KiB; `cursors_directory` tek
  bileşen olmalı) + `<dizin>/<şekil>.hlc` ZIP. Şekil önce `<ad>.hlc` ile, bulunamazsa
  `define_override` haritasıyla (dizinde ≤ 1024 arşiv, tema başına bir kez) bulunur. Meta
  hyprlang ve düz toml (`;` ayrılmış liste) okunur; ≤ 256 `define_size`, ≤ 64 eşanlamlı.
  ZIP okuyucu kendi yazımımız (`cursor::zip`, sınırların tek yerde ve açık olması için): arşiv
  ≤ 16 MiB belleğe, ≤ 512 girdi, düz adlar (`/`, `\`, `..` olan girdi atlanır), yalnızca
  stored/deflate, ZIP64/çok disk/şifreli reddedilir; açılmış girdi ≤ 4 MiB (açma bu sınırda
  durur — bildirilen boyuta güvenilmez), sonra boyut + CRC-32 doğrulanır.
- **Hyprcursor güveni (ADR-0007):** raster (yalnızca PNG imzalı veri; başka raster biçimi yok)
  her zaman tercih edilir. Yalnızca SVG'si olan şekil, `.hlc` dosyası ikon temalarıyla aynı
  `system_origin` kuralını geçerse (ev dizini dışı arama dizini, kök sahipli, grup/diğerleri
  yazamaz, Flatpak/Snap değil; güven açılmış fd'den) `Trust::System` ile süreç içinde çizilir;
  kullanıcı dizinindeki SVG şekil `Untrusted` kalır: `Loader::with_sandbox` kuruluysa
  (`LoadRequest::cursor`) SVG baytları yalıtılmış işçide tek kare çözülür; yalıtımsız
  (`CursorTheme::load`, sandbox'sız `Loader`) ya da işçi `Unavailable` ise asla süreç içinde
  değil — `Unsupported("SVG")` / `Sandbox(Unavailable)` (önizlemede "yok"). Xcursor ve PNG kareler
  süreç içinde kalır.

**Tema önizlemeleri** (`nuxut_image::preview`; Ayarlar tema kartları):

```rust
pub enum IconRole { Folder, Home, Downloads, FileManager, Terminal, Settings, WebBrowser,
                    TextEditor, ImageFile, TextFile, Music, Trash }
// IconRole::ALL, ::PREVIEW (8: folder, file manager, terminal, settings, browser, editor, image, trash);
// names(): freedesktop adı → yaygın uygulama adları (ör. Terminal: utilities-terminal … kitty, foot).
pub fn preview::cursor_request(Arc<CursorTheme>, CursorRole, px: u32, animate: bool) -> LoadRequest; // hedefe sığdırır
pub fn preview::icon_request(Arc<IconTheme>, IconRole, size: u32, scale: u32) -> LoadRequest;        // Trust::System istenir
impl PreviewKey { pub fn load_key(&self) -> u64; }   // load_keyed anahtarı: FNV-1a(tür, tema, rol), px hariç, ≠ NO_KEY
pub fn preview::cancel_theme(&Loader, theme: &str);  // temanın tüm imleç+ikon rol anahtarlarını iptal
pub fn preview::outcome(Result<Decoded, ImageError>) -> Option<PreviewEntry>;
// Some(Some(img)) / Some(None) = sakla ("yok": temada yok, çözülemedi, işçi yok/öldü);
// None = geçici (Cancelled, Sandbox(RateLimited)) — saklanmaz, sonraki istekte yeniden denenir.
// LoadRequest::cursor(..) / LoadRequest::icon_any(theme, names, size, scale); Source::Cursor / Source::IconAny.
// IconTheme::load_first_as(names, size, scale, Trust): önce tam adlar sırayla, sonra kısaltmalı.
pub struct PreviewImage { frames: Arc<[Frame]> }    // Send + Sync; from_decoded(Decoded) (akışta ilk kare)
pub enum PreviewKey { Cursor { theme, role: CursorRole, px }, Icon { theme, role: IconRole, px } }
pub struct PreviewCache;     // Send + Sync, LRU, bayt bütçeli (DEFAULT_PREVIEW_BUDGET = 8 MiB)
impl PreviewCache {
    pub fn new(budget_bytes) -> Self;
    pub fn get(&self, &PreviewKey) -> Option<Option<PreviewImage>>;   // Some(None) = bilinen "yok"
    pub fn insert(&self, PreviewKey, Option<PreviewImage>);           // bütçeden büyük tek görsel saklanmaz
    pub fn forget_theme(&self, id); pub fn clear(&self);
}
```

Akış (tembel, boşta iş yok): şerit (`nuxut_ui::CursorPreview`/`IconThemePreview`) ilk
çizildiğinde görünen yuvaların aralığını verir → her yuva için önce `PreviewCache::get`, yoksa
düşük öncelikli `Loader`'a `cursor_request`/`icon_request` → sonuç `MessageSender` ile olay
döngüsüne → `PreviewCache::insert` + `Painter::upload_texture` → `set_slot`. Örnek:
`nuxut-ui/examples/gallery/theme_page.rs` (`NUXUT_GALLERY_PAGE=8`).

Güven ve yalıtım (ADR-0007): ikon istekleri `Trust::System` ister; kullanıcı dizinindeki
(`~/.icons`, `~/.local/share/icons`; genel olarak kök sahipli sistem dizini dışındaki) dosya
`Untrusted`'a düşer. Böyle bir SVG ikon (Papirus vb. kullanıcı kurulumu) ya da kullanıcı
dizinindeki hyprcursor şekli (SVG ya da PNG kareleri), `Loader::with_sandbox` ile kurulmuş
yükleyicide yalıtılmış işçide **tek kare** çözülür. Sandbox'sız yükleyicide ve işçi `Unavailable`
iken SVG asla süreç içinde değil — yuva "yok" (yer tutucu) gösterir; PNG/temel raster K2-b ile
süreç içinde yalnızca ilk kare. Uygulama kurulumu:

```rust
let loader = Loader::with_options(LoaderOptions { threads: 2, low_priority: true })
    .with_sandbox(Arc::new(nuxut_image_sandbox::Sandbox::new(SandboxConfig::default())));
// iş: loader.load_keyed(key.load_key(), icon_request(..) / cursor_request(..), |r| send(outcome(r)));
//     Err(Cancelled) sonucu gönderilmez (aynı yuvanın yeni isteği gelir).
// tema değişince: preview::cancel_theme(&loader, eski_tema) (+ gerekiyorsa cache.forget_theme).
```

Uygulama işçiyi framework'ten ayrı derleyip kurmalı (`-p nuxut-image-worker`; Görsel yalıtımı).
Aynı `Loader`'ı paylaşan iki önizleme tüketicisi aynı yuvayı isterse aynı iptal anahtarını üretir
ve birbirini iptal eder (`PreviewKey::load_key` belgesi; gerekirse `load_with_keys`). İşçi
tembel başlar, 30 sn boşta kalınca kapanır: boşta süreç/thread/CPU yok. Galeri işçiyi
`nuxut_image_sandbox::locate::locate()` ile arar; bulamazsa sandbox'ı hiç kurmaz (sessiz yer
tutucu). Ölçüm (headless sway, sahte kullanıcı dizini SVG ikon + hyprcursor teması): 8 ikon +
3 imleç işçide çözüldü (2 işçi), boşta 5 sn'de galeri 0 CPU tiki, 35 sn sonra işçi süreci 0.

```rust
// Bildirim `image-data` (rowstride, has_alpha, 8 bit, 3/4 kanal) ve tepsi ARGB32; doğrulanır, en fazla
// `max_side` kenara küçültülür (premultiplied). Asenkron: `LoadRequest::raw`, `Source::Argb32`.
pub fn downscale_rgba(pixels: &[u8], width, height, rowstride, has_alpha: bool, max_side) -> Result<Image, ImageError>;
pub fn downscale_argb32(argb: &[u8], width: i32, height: i32, max_side) -> Result<Image, ImageError>;
impl Image { pub fn into_image_data(self) -> nuxut_render::ImageData; }   // `render` özelliği

pub struct Player;           // kare zamanlaması: set_visible/set_paused, frame(now), next_change(now)
pub struct Loader;           // arka plan thread'leri İLK load'da açılır (sinyal maskesini miras alsın)
// `LoadRequest::with_trust(Trust)`: isteğin güveni (`options.limits.trust`, varsayılan Untrusted);
// `Source::Icon` → `IconTheme::load_as`.
impl Loader { pub fn load(&mut self, LoadRequest, done: impl FnOnce(Result<Decoded, ImageError>) + Send) -> LoadHandle; }
// ADR-0007: `Loader::with_sandbox(Arc<dyn IsolatedDecoder>)` (tipik: `Arc<nuxut_image_sandbox::Sandbox>`).
// Kuruluysa `Untrusted` `Path`/`Bytes`, etkin güveni `Untrusted` kalan `Icon`/`IconAny` dosyası
// ve kullanıcı dizinindeki hyprcursor arşivinin şekli (`Cursor`: yalnızca-SVG şekil ve PNG
// kareleri) okunup baytları yalıtılmış çözücüye verilir; sonuç tek karedir (K5). `System`,
// Xcursor ve ham pikseller (`Raw`/`Argb32`) süreç içinde kalır. Kurulmazsa süreç içi yol:
// Untrusted SVG/JXL/AVIF → Unsupported, temel raster ilk kare (hyprcursor: PNG ilk kare).
// `load_keyed(key: u64, ..)` = `load_with_keys(LoadKeys::same(key), ..)`.
// `load_with_keys(LoadKeys { cancel, source }, ..)`: aynı `cancel` ile bekleyen eski iş iptal
// edilir (`Err(Cancelled)`; sürüyorsa sonucu atılır) — `NewIcon` fırtınası tek çözüme iner;
// `source` yalıtılmış çözücüye gider (susturma, negatif kova, hız sınırı). Ör. bildirim:
// cancel = uygulama + yuva + bildirim kimliği (art arda bildirimler birbirini iptal etmez),
// source = uygulama (uygulama başına susturma korunur). `NO_KEY` (0): iptal birleştirmesi yok;
// source `NO_KEY` ise çözücüye kaynaktan türetilmiş anahtar gider (`derive_key`: yol / ikon adı /
// imleç teması+rolü; bayt kaynağı `ANON_KEY` kovası) — anahtarsız güvenilmeyen iş de susturulur.
// Anahtarlar log'a yazılmaz. `cancel_key(&self, key) -> bool`: İPTAL anahtarı `key` olan
// bekleyen/süren işi iptal eder (kaynak anahtarına bakılmaz).
pub struct LoadKeys { pub cancel: u64, pub source: u64 }   // LoadKeys::same(k)
pub trait IsolatedDecoder: Send + Sync + Debug { fn isolation(&self) -> Isolation;
    fn decode(&self, key: u64, bytes: Arc<[u8]>, &DecodeOptions) -> Result<Image, ImageError>;
    fn max_input_bytes(&self) -> u64 { u64::MAX } }   // Loader bundan fazlasını belleğe okumaz
pub enum Isolation { Process, Unavailable }   // Linux dışı da Unavailable (InProcess kaldırıldı)
pub const NO_KEY: u64 = 0; pub const ANON_KEY: u64 = u64::MAX; pub fn derive_key(&[&[u8]]) -> u64;
// `decode_isolated` (doc(hidden)) yalnızca işçi kipinde çalışır: `enter_worker_mode()` işçi
// kilitlendikten SONRA çağrılır; kip açık değilse `Unsupported` (süreç içinde kazayla
// güvenilmeyen bayt ayrıştırılamaz). `in_worker_mode()` sorgular.
pub enum SandboxFailure { Timeout, Killed, Protocol, Unavailable, RateLimited }  // ImageError::Sandbox(_); bayt/yol taşımaz
pub fn sniff_format(&[u8]) -> SniffedFormat;  // içerikten; SniffedFormat::in_process_fallback() = PNG/JPEG/WebP/GIF/QOI
// `Loader::with_options(LoaderOptions { threads, low_priority })`: `low_priority` → çözme thread'leri
// Linux'ta thread başına nice 19 (`setpriority(PRIO_PROCESS, 0)` yalnızca o thread; yetki gerekmez);
// diğer platformlarda yok sayılır. UI ile çekirdek paylaşan çözme (bildirim görselleri) arayüzü aç bırakmaz.
// Çözme bellek tepesi: küçültülecek gri/gri+alfa/RGB (8/16 bit, float) görsel önce kendi kanal
// sayısıyla küçültülür, sonra RGBA'ya genişletilir (tam boyutlu RGBA kopyası yok; sonuç piksel aynı).
```

### Görsel yalıtımı (nuxut-image-sandbox, ADR-0007)

Kural: **güvenilmeyen bayt shell'in adres alanında ayrıştırılmaz.** Her `Trust::Untrusted`
kodlanmış görsel (SVG, JXL, AVIF, PNG/JPEG/WebP/GIF/QOI) Linux'ta ayrı, kısıtlı bir yardımcı
süreçte çözülür. Kullanıcının duvar kağıdı ve diğer kullanıcı dosyaları da (güven ne olursa olsun,
JXL dahil) işçide, ayrı **duvar kağıdı profiliyle** çözülür ("Duvar kağıdı yalıtımı", aşağıda);
süreç içinde yalnızca `System` (bizim seçtiğimiz tema ikonu, gömülü varlık) kalır. Tek istisna
K2-b: işçi kullanılamıyorken (ve Linux dışında) yalnızca temel raster, ilk kare, aynı kapılarla.

```rust
pub struct SandboxConfig { worker: Option<PathBuf> /* None → arama sırası */,
    profile: WorkerProfile /* Icon (varsayılan) | Wallpaper; işçi süreci başına */, timeout /* 1.5 sn */,
    negative_on_timeout /* true; duvar kağıdında false */, in_process_fallback /* true: K2-b */,
    spawn_timeout /* 3 sn */, max_workers /* 2: meşgul + taze */, prewarm /* 1 boşta taze */, idle_timeout /* 30 sn */,
    rate_per_sec /* 30 */, burst /* 60 */, negative_cache /* 256 */, key_failure_threshold /* 3 */,
    key_mute /* 60 sn */, max_input_bytes /* 8 MiB, üst sınır */, max_output_pixels /* 4096² */,
    unavailable_retry /* 60 sn */ }
impl SandboxConfig { pub fn wallpaper() -> Self; }  // aşağıdaki "Duvar kağıdı yalıtımı" tablosu
pub enum WorkerProfile { Icon, Wallpaper }          // code(), arg() ("--profile=…"), caps() -> ProfileCaps
pub struct ProfileCaps { max_input_bytes, limits: Limits, max_out_pixels, address_space,
    cpu_soft_secs, cpu_hard_secs, nice, oom_score_adj }  // clamp(&Limits): isteği tavana indirir, asla yükseltmez
impl Sandbox {
    pub fn new(SandboxConfig) -> Self;          // süreç başlatmaz; işçi yolu bir kez çözülür
    pub fn isolation(&self) -> Isolation;       // Process | Unavailable (Linux dışı her zaman Unavailable)
    pub fn decode(&self, key, Arc<[u8]>, &DecodeOptions) -> Result<Image, ImageError>;  // tek kare
    pub fn decode_file(&self, key, File, &DecodeOptions, &CancelFlag) -> Result<DecodedFile, ImageError>;
                                                // dosya akışı; IsolatedDecoder::decode_file (aşağıda)
    pub fn profile(&self) -> WorkerProfile;
    pub fn stats(&self) -> SandboxStats;        // spawned, live, idle, decoded, failures, rate_limited,
                                                // negative_hits, in_process_fallbacks, cancelled, inherited_fds
}
impl IsolatedDecoder for Sandbox;               // → Loader::with_sandbox(Arc::new(sandbox))
impl Drop for Sandbox;                          // işçiler öldürülür, temizlikçi beklenir: sonra süreç yok
pub fn worker_main() -> !;                      // `worker` özelliği; nuxut-image-worker paketinin tek satırı
pub const EXIT_LOCKDOWN: i32 = 70; pub const EXIT_PROTOCOL: i32 = 65;   // işçi çıkış kodları
```

- **İşçi ikilisi** `nuxut-image-worker` **ayrı framework paketidir** (`crates/nuxut-image-worker`:
  `nuxut-image-sandbox` `worker` özelliği + `nuxut-image` `jxl`). Tüketiciler kendi `[[bin]]`'ini
  TANIMLAMAZ; işçiyi framework'ten AYRI bir cargo çağrısıyla derler:
  `cargo build --release --manifest-path <framework>/Cargo.toml -p nuxut-image-worker` (çıktı
  `<framework>/target/release/nuxut-image-worker`). Ayrı çağrı olduğu için işçinin özellikleri
  (JPEG XL, seccompiler, landlock) tüketicinin ana ikilisine birleşmez: `nuxut-image-sandbox` ana
  süreç tarafı (`Sandbox`, `protocol`, `locate`) jxl'siz, seccompiler/landlock'suzdur
  (`cargo tree -p nuxut-image-sandbox -e normal` içinde yoklar). Framework testleri kancalı test
  işçisini kullanır: `nuxut-image-sandbox` içinde `[[bin]] nuxut-image-worker-test`
  (`required-features = ["sandbox-test-hooks"]`, `CARGO_BIN_EXE_…`; ad farklı, `locate()` onu
  bulmaz); `nuxut-image-worker` paketinin kendi testi dağıtılan işçiyi kancasız ebeveynle sınar
  (JXL dahil). libwayland/EGL linklenmez.
- **JPEG XL yalnızca işçide.** `nuxut-image` varsayılan özellikleri `["render"]` (jxl yok),
  `nuxut-video` `[]`, `nuxut-palette` `["image", "sandbox"]` (jxl yok); jxl-oxide yalnızca
  `nuxut-image-worker` ağacında. Duvar kağıdı dahil kullanıcı görselleri işçide çözüldüğü için
  shell/lock/settings ağaçlarına JXL girmez. Süreç içi JXL yalnızca `jxl`'i açıkça açan tüketicide
  (`System` güven); kapalıysa `ImageError::Decode("JPEG XL desteği derlenmedi …")`.
  `tools/cross-check.sh` bekçisi: palet (üç özellik kümesi), nuxut-image, nuxut-video ve
  nuxut-image-sandbox ağaçlarında jxl yok; işçi ağacında jxl-oxide var.
- **Arama sırası** (ilk düzenli + çalıştırılabilir dosya): `NUXUT_IMAGE_WORKER` (verilmişse
  yalnızca o; geliştirme/test) → çalışan ikiliye göreli: aynı dizin, sonra `../lib/nuxut/` →
  `~/.local/lib/nuxut/` (kullanıcı kurulumu) → `/usr/lib/nuxut/` (dağıtım paketi). Bulunamazsa
  `Unavailable`.
- **İşçinin kilidi** (girdi okumadan önce; başarısızsa çıkış 70): miras fd SAYISI okunur →
  (duvar kağıdı profilinde önce `nice` 10 + `oom_score_adj` 1000, en iyi çaba) → `setrlimit` AS
  768 MiB, CPU 4/5 sn (ikon; duvar kağıdı 2.5 GiB, 10/12 sn), FSIZE/NPROC/CORE/MEMLOCK 0 → `NO_NEW_PRIVS`, `DUMPABLE 0` →
  Landlock (dosya sistemi, TCP, soyut soket/sinyal; kuralsız; kısmi uygulama kabul, **hiç
  uygulanamazsa çıkış 70**) → NOFILE 3 → seccomp `KILL_PROCESS` izin listesi: `read` yalnız fd 0,
  `write` yalnız fd 1, `mmap` `PROT_EXEC`'siz ve anonim, `mprotect` `PROT_EXEC`'siz,
  `munmap`/`mremap`/`brk`/`madvise`/`futex`/`sched_yield`/sinyal dönüşü/`clock_gettime`/
  `getrandom`/`exit*`. `open*`, `socket`, `clone*`, `execve*`, `ioctl`, `close` yok — miras fd'ler
  kullanılamaz (kapatılamazlar: bilinen, ADR-0007). Ardından `nuxut_image::enter_worker_mode()`
  ve el sıkışma.
- **Protokol** (stdin/stdout, LE, **sürüm 4**): el sıkışma (12 bayt başlık) `NXIW | u16 sürüm |
  u8 bayraklar (seccomp, test kancaları) | u8 landlock düzeyi (1 kısmi, 2 tam) | u16 miras fd
  sayısı | u8 profil (1 ikon, 2 duvar kağıdı) | u8 n | crate sürümü`. Ebeveyn **seccomp bitini,
  landlock ≥ 1'i ve kendi profilini zorunlu tutar**, kancalı (test) işçiyi kendisi kancalı
  derlenmemişse reddeder → `Unavailable`; miras fd > 0 ise debug log (yalnızca sayı). İşçi
  profili `--profile=icon|wallpaper` argümanından alır (rlimit'ler girdi okunmadan kurulur).
  İstek 48 bayt başlık (uzunluk ≤ profilin `max_input_bytes`'ı, biçim, fit, kanca, **bayrak
  baytı: bit0 `CROP`** (yalnızca Cover), hedef, max_w/h, max_pixels, max_out_pixels,
  **max_decode_bytes**; işçi bunları da profil tavanına kırpar) + bayt; yanıt **20 bayt** (durum,
  hata sınıfı, **u16 bayrak: bit0 `ANIMATED_SOURCE`**, w, h, **u32 kare sayısı, u32 tur süresi
  ms** — v4) + tam `w·h·4` premultiplied. Kare sayısı ve süre animasyonlu kaynakta **işçide** kap
  yapısından sayılır (`animation_info`, piksel çözülmez; `u32`'ye doyurulur) ve
  `DecodedFile::animation`'a çıkar: ana süreç güvenilmeyen kabı yürümez (`nuxut-video` içe
  aktarma bütçesi bunu kullanır). Hata sınıfı metin taşımaz. Ebeveyn işçiyi ele geçirilmiş sayar:
  boyut isteğin sınırları ve hedef kutuya (Cover: bir eksen; **kırpılmış Cover: iki eksen**) göre
  denetlenmeden tampon ayrılmaz; tanımsız bayrak, hata yanıtında bayrak ya da animasyon alanı,
  `ANIMATED_SOURCE`'suz animasyon alanı, 1 kare ya da karesiz süre (geçerli: kare 0 = bilinmiyor
  ve süre 0, ya da kare ≥ 2), eksik/fazla bayt, tanımsız durum → `Protocol`. Animasyon bilgisi
  yalnızca ön denetimdir (ele geçirilmiş işçi küçük sayı söyleyebilir); kesin sınır kodlayıcının
  kendi sayacıdır.
- **Tek kullanımlık işçi:** her istek girdi görmemiş taze bir işçiye gider, yanıttan sonra işçi
  öldürülür; ele geçirilmiş işçi sonraki isteğin (başka uygulamanın ikonu) yanıtını taklit
  edemez. **Girdi görmüş işçi asla yaşamaz.** Gecikme önceden ısıtmayla saklanır: kullanılmış
  işçiyi öldürmek ve yerine taze işçi başlatmak (el sıkışma kullanılırken okunur) bir
  **temizlikçi** thread'de, çağıranın yolu dışında yapılır. Yaşayan işçi (meşgul + taze +
  öldürülmeyi bekleyen) ≤ `max_workers` (2). **Boşta en çok `prewarm` (varsayılan 1) taze işçi
  bekler ve `idle_timeout` (30 sn) sonra kapanır**; temizlikçi iş ve boşta işçi kalmayınca çıkar.
  Bir çözümden sonra uygulamanın altında görülen 1 işçi bu girdi görmemiş taze işçidir (PID'i
  hizmet edenlerden farklıdır; `idle_fresh_workers_are_bounded_unused_and_reaped` testi).
  `prewarm` 1 kararı: tepsi/bildirim çözümleri çoğunlukla sıralı gelir; ikinci eşzamanlı istek
  soğuk başlar (~2 ms), buna karşılık her boştaki işçi ~7 MB RSS tutar (debug ölçümü) — önceki
  davranışta (`prewarm` = `max_workers`) eşzamanlı iki çözümden sonra 30 sn boyunca 2 işçi
  kalıyordu. `prewarm: 0` ön ısıtmayı kapatır. **30 sn sonra: süreç yok, thread yok, CPU 0.**
  Ebeveyn borular bloklamasız + `poll` son tarihi; zaman aşımı/çökme → `Child::kill` + `wait`
  (zombi yok).
  Zombi incelemesi (2026-10-17, `hostile_svg_corpus` testinde ara sıra görülen zombi): bütün
  yollar (`kill`, `kill_classify`, `Worker::drop`, temizlikçi) `process::reap` ile bekler; işçi
  isteği yanıtladıktan sonra kendiliğinden çıkmaz, temizlikçi öldürene dek canlıdır. Görülen
  zombi, temizlikçi thread'in `kill` ile `wait` arasındaki anıdır (çocuk ölünce çekirdek onu
  `Z` yapar, yük altında bekleyen thread birkaç ms geç uyanır): her çocuğun geçtiği durum,
  sızıntı değil. Test bu yüzden zombinin kısa sürede oturmasını bekler ve Sandbox kapanınca
  (temizlikçi join edilir) beklemesiz kesin sıfır çocuk/zombi ister.
- **`<image href>` / dış kaynak:** işçide de süreç içinde de tek nokta `svg::usvg_options()`:
  `resources_dir: None`, `href` dize çözücüsü her şemada (`file:`, mutlak/göreli yol, `http(s):`,
  `ftp:`, `//host`, …) hiçbir şey açmadan `None` (usvg'nin dosya okuyan varsayılan çözücüsü hiç
  kurulmaz), `text`/font derlenmez. `data:` yalnızca PNG/JPEG/GIF/WebP, ≤ 4 MiB ve başlıkta
  ≤ 4096 kenar / 16 Mpx ise **çözülür** (aynı tuval/iş bütçesinden geçer); sınır dışı raster
  ve gömülü SVG (`image/svg+xml`, iç içe ayrıştırma) **reddedilir** (öğe hiç çizilmez). Testler:
  çözücü birim testi; işçi kipinde çıktı `<image>`'sız SVG ile bayt bayt aynı; gerçek işçi her
  şemayı ölmeden yanıtlar (açma girişimi seccomp ile `Killed` olurdu).
- **Kapılar (her yolda, süreç içi yedekten ÖNCE):** susturulmuş anahtar → `RateLimited`;
  `Timeout`/`Killed`/`Protocol` vermiş girdinin blake3 özeti (baytta içerik; dosyada
  `dev ‖ ino ‖ len ‖ mtime_ns ‖ ctime_ns`, içerik iki kez okunmaz, dosya değişince anahtar değişir)
  256'lık LRU'da → işçi başlatmadan `Killed` (`negative_on_timeout: false` profilde zaman aşımı
  girmez; iptal hiç girmez ve susturma sayacını etkilemez); belirteç kovası (30/sn, patlama 60) → `RateLimited`. Anahtar başına art arda 3 hatada
  60 sn susturma; `NO_KEY` istekler tek `ANON_KEY` kovasında susturulur.
- **Kullanılamıyorsa** (ikili yok, kilit kurulamıyor, el sıkışma reddedildi, Linux dışı; K2-b):
  kapılardan geçmiş PNG/JPEG/WebP/GIF/QOI süreç içi yola düşer (`Untrusted` kapısı, isteğin
  profil tavanına kırpılmış sınırları — boyut/piksel/bütçe — ilk kare; uyarı SÜREÇ başına bir kez;
  `in_process_fallback: false` ile kapatılır);
  SVG/JXL/AVIF → `Sandbox(Unavailable)`. Zehirli (negatif önbellekteki) ya da susturulmuş girdi
  asla süreç içine düşmez.
- **Test kancaları** (`sandbox-test-hooks`) yalnızca testte açılır; sürüm derlemesinde
  `compile_error!`.
- **Hata → yedek ikon:** çağıran `Err` aldığında tepside/bildirimde `nuxut-ui` `Icon`'un semantik
  `application`/`image-missing` adını gösterir.
- **Log:** yalnızca hata türü, süre, bayt uzunluğu, çıkış kodu/sinyali, miras fd sayısı; yol, URL,
  ad, `key` asla.
- **Maliyet** (`examples/sandbox_bench.rs`, dev profili + bağımlılıklar opt 2, VM yük ~12):
  soğuk (süreç + kilit + el sıkışma + 64² SVG) medyan 1.7 ms; aralıklı istek (önceden ısıtılmış
  taze işçi; tepsi/bildirim tipik) medyan 0.55 ms; sıkı döngü (istek başına yeni süreç) ortalama
  1.45 ms; aynı çözüm süreç içinde 0.13 ms.

### Duvar kağıdı yalıtımı (ADR-0007 takip)

Duvar kağıdı kullanıcı dosyasıdır, internetten inmiş olabilir: **tüm biçimler (JXL dahil) güven ne
olursa olsun yalıtılmış işçide, duvar kağıdı profiliyle** çözülür. Aynı yol diğer kullanıcı
dosyaları içindir (`nuxut-video` `Importer`ın görsel yolu da bunu kullanır — "Video duvar kağıdı"
§İçe aktarma; sıradaki iş: lock avatarı `~/.face`); API duvar kağıdına özel değildir.

**Profil** (`WorkerProfile`): rlimit'ler girdi okunmadan kurulduğu için işçi süreci başınadır;
her profil ayrı bir `Sandbox` (havuz, kova, susturma, negatif önbellek ayrı). Tepsi ikonu fırtınası
duvar kağıdını hız sınırına sokamaz; ele geçirilmiş tepsi ikonu geniş sınırları elde edemez.

| Sınır | Icon | **Wallpaper** (`SandboxConfig::wallpaper()`) |
|---|---|---|
| Girdi baytı | 8 MiB (bellekten) | **256 MiB**, akışla (ebeveyn belleğe almaz) |
| Kenar / kaynak piksel | isteğin (≤ 4096, 16 Mpx) | **16 384 / 16384²** (`Limits::WALLPAPER`) |
| Çözücü bütçesi (`max_decode_bytes`) | 128 MB | **1.5 GiB**: 8 bit JPEG 16K² geçer; 16 bit RGBA PNG 16K² `TooLarge`; JXL ≈ 57 Mpx |
| `RLIMIT_AS` | 768 MiB | **2.5 GiB** |
| `RLIMIT_CPU` | 4 / 5 sn | **10 / 12 sn** (işçi tek thread'li; seccomp `clone` yok) |
| Duvar saati | 1.5 sn | **20 sn** |
| Zaman aşımı → negatif önbellek | evet | **hayır**; SIGXCPU/AS/seccomp (`Killed`) ve `Protocol` girer |
| Çıktı piksel | 4096² | **8192²**; kırpılmış Cover'da çıktı ≤ hedef |
| `max_workers` / `prewarm` | 2 / 1 | **1 / 0** (boşta süreç yok) |
| Kova / susturma | 30/sn, 60; 3 hata → 60 sn | **4/sn, 8; 3 hata → 30 sn** |
| Öncelik | — | **nice 10, `oom_score_adj` 1000** (işçi kendine, kilitten önce) |

Ölçüm (`nuxut-image-worker` `examples/wallpaper_bench.rs`, release işçi, VM yük 15–18; uçtan uca
`WallpaperDecoder` isteği → kare, ikinci koşu; tepe RSS işçinin VmHWM'i):

| Kaynak → hedef (Cover kırpma) | Süre | İşçi tepe RSS |
|---|---|---|
| 4K JPEG → 3840×2160 | 0.54 sn | 63 MiB |
| 4K JPEG → 1920×1080 | 0.13–0.59 sn | 50 MiB |
| 8K JPEG → 3840×2160 | 0.47 sn | 181 MiB |
| 8K JPEG → 7680×4320 | 0.57–0.84 sn | 235 MiB |
| 16K² JPEG → 3840×2160 | 2.5 sn | 939 MiB |
| 8K PNG (RGB8) → 3840×2160 | 0.60 sn | 239 MiB |
| 8K JXL (d1) → 3840×2160 | 4.6 sn | 879 MiB (≈ 27.7 B/px → `JXL_BYTES_PER_PIXEL` 28) |

Ebeveyn: kare tutulurken yalnızca çıktı kadar (+33 MiB 4K, +127 MiB 8K), bırakınca +1 MiB —
glibc mmap eşiğine rağmen 4K tampon işletim sistemine dönüyor; özel ayırma gerekmedi. 200 MiB
dosya akışı ebeveyn VmRSS'ini 20 KiB büyüttü (test). İptal → işçi ölü: 51 ms (test, ≤ 100 ms).

```rust
// nuxut-image — genel kullanıcı dosyası API'si (render'sız)
pub fn open_user_file(&Path) -> Result<std::fs::File, ImageError>;   // O_NONBLOCK, fstat normal dosya
pub fn file_key(domain: &str, &Path) -> u64;   // susturma kovası: derive_key([domain, yol]); loglanmaz
#[derive(Clone, Default)] pub struct CancelFlag;  // new(), cancel(), is_cancelled(); kopyalar paylaşır
pub struct DecodedFile { pub image: Image, pub animated_source: bool, pub animation: Option<AnimationInfo> /* v4: işçide sayılır */ }
impl DecodedFile { pub fn from_source(Image, bytes: &[u8], &Limits) -> Self; } // süreç içi çözücüler (yedek, test)
pub trait IsolatedDecoder {
    /* isolation, max_input_bytes, decode — değişmedi */
    fn decode_file(&self, key: u64, file: std::fs::File, opts: &DecodeOptions, cancel: &CancelFlag)
        -> Result<DecodedFile, ImageError>;   // varsayılan gövde: okur + decode; Sandbox akıtır
}
```
`Sandbox::decode_file` sırası: `fstat` → `len ≤ min(max_input_bytes, limits.max_file_bytes)`
(aşarsa `FileTooLarge`, işçi başlamaz) → ilk 4 KiB `read_at` ile biçim koklama → kapılar (negatif
anahtar dosya kimliği) → işçiye 256 KiB parçalarla `read_at` → boru (son tarih + iptal). İptal 50
ms `poll` dilimleriyle denetlenir, işçi hemen öldürülüp beklenir: `ImageError::Cancelled`,
başarısızlık sayılmaz. Dosya akış sırasında kısalırsa `ImageError::Io` (negatif önbelleğe girmez).

**Anahtar kalıbı (tüketici başına):** süreç başına ayrı `Sandbox` olduğu için anahtar yalnızca o
süreçte anlamlıdır; önerilen `nuxut_image::file_key("<amaç>", yol)` — ör. duvar kağıdı
`file_key("wallpaper", path)` (`WallpaperDecoder` bunu kendisi verir), palet
`source_color_from_file(path, &*sandbox, file_key("wallpaper", path))`, lock avatarı
`file_key("avatar", path)`. Yol anahtardadır: bozuk bir dosyanın 3 hatası başka dosyaları
susturmaz; aynı dosyayı döngüyle deneyen betik 30 sn susturulur. (Shell'in `KeyDomain` +
`source_key` SipHash'i de olur; kova ayrı `Sandbox`'ta olduğundan çakışma riski yoktur.)

```rust
// nuxut-image (render'sız) — süreç başına TEK örnek, Arc ile paylaşılır
pub struct WallpaperDecoderOptions { pub cache_bytes: u64 /* 96 MiB */, pub linger: Duration /* 10 sn; lock: ZERO */ }
pub struct WallpaperRequest { pub path: PathBuf, pub target_px: (u32, u32), pub fit: Fit }  // new(path, px, fit); Debug yolu gizler
#[derive(Clone)] pub struct WallpaperFrame { pub width: u32, pub height: u32,
    pub pixels: Arc<[u8]> /* premultiplied RGBA8; doku yüklemesine kopyasız */, pub animated_source: bool }
impl WallpaperFrame { size(), byte_size(), to_image(), fit_image(TargetSize), to_image_data() /* render */, is_valid() }
impl From<Image> for WallpaperFrame;   // pikseller kopyasız Arc'a; animated_source false
pub struct WallpaperDecodeStats { decodes, hits, joined, derived, cancelled, failed,
    resident_bytes, entries, pending, thread_alive }
impl WallpaperDecoder {
    pub fn new(decoder: Arc<dyn IsolatedDecoder> /* Sandbox::new(SandboxConfig::wallpaper()) */,
               opts: WallpaperDecoderOptions) -> Arc<Self>;
    pub fn in_process_unsandboxed(WallpaperDecoderOptions) -> Arc<Self>;  // YALNIZCA örnek/test: System, süreç içi
    pub fn request(self: &Arc<Self>, WallpaperRequest,
                   done: impl FnOnce(Result<WallpaperFrame, ImageError>) + Send + 'static) -> WallpaperTicket;
    pub fn trim(&self);                       // bekleyen dışındaki kareleri bırak (bellek baskısı, kilit)
    pub fn stats(&self) -> WallpaperDecodeStats;
}
pub struct WallpaperTicket;                   // drop = bu bekleyeni iptal et
```
- **Hedef:** çıkışın fiziksel boyutu. `Cover` → `TargetSize::cover_crop` (kaynakta ortadan kırpma,
  çıktı ≤ hedef; 16384×2160 panorama 3840×2160 çıkışta 33 MB, eskiden 141 MB); `Contain`/`Fill`
  ≤ hedef. Lanczos3, asla büyütmez; dar biçim kendi kanalında küçültülür.
- **Birleştirme:** aynı (yol, hedef, fit) kuyrukta/çözülüyorsa bekleyen eklenir (`joined`); tek
  `Arc` tampon bütün çıkışlara gider. **Önbellek** anahtarı `(yol, dev, ino, len, mtime, ctime) +
  hedef + fit`; dosya bir kez açılır, kimlik ve akış aynı fd'den. **Türetme:** aynı dosya + fit için
  daha büyük kare elde varsa küçük kare süreç içinde güvenilir RGBA'dan Lanczos3 ile türetilir
  (`derived`; Contain/Fill her zaman, kırpılmış Cover en-boy ±1 px aynıysa); kuyruktan aynı
  dosyanın en büyük hedefi önce alınır. 4K + 1080p → tek çözüm.
- **Tek geçici thread** (`nuxut-wallpaper`, nice 19): ilk istekte açılır; kuyruk boşalınca süresi
  dolan kareleri bırakır (`linger` son kullanımdan), kare kalmayınca çıkar. Bütçeyi (`cache_bytes`)
  aşan kare teslim edilir ama tutulmaz (LRU). **Boşta: çözülmüş kare yok, thread yok, işçi süreci
  yok, CPU 0** (`decoder_joins_derives_lingers_and_goes_idle` testi).
- **İptal:** bilet düşünce bekleyen çıkar; işin bekleyeni kalmazsa kuyruktan silinir ya da
  `CancelFlag` ile durdurulur. Hızlı 10 değişim → ≤ 2 çözüm, yalnızca sonuncusu teslim edilir.
- **Animasyon (S2-a):** işçi ilk kareyi ve `ANIMATED_SOURCE`'u verir; `Wallpaper::animated_source()`
  ile ayarlar "animasyon için içe aktar" (nuxut-video) ipucu verebilir. Süreç içi oynatım kalktı.
- **İşçi yokken (S1):** PNG/JPEG/WebP/GIF/QOI süreç içinde (`Untrusted` kapısı, ilk kare,
  aynı boyut/piksel/bütçe kapıları, çağıranın düşük öncelikli thread'i), süreç başına tek uyarı;
  JXL/AVIF/SVG → `Sandbox(Unavailable)` → zeminin düz rengi.
- **Hata eşlemesi** (önceki katman ekranda kalır, hiç yoksa zemin rengi; `last_error()`; log
  yalnızca tür): `TooLarge`/`FileTooLarge` → "Görsel çok büyük"; `UnknownFormat`/`Decode`/`Io` →
  "Görsel açılamadı"; `Sandbox(Timeout | Killed | Protocol)` → "Görsel çözülemedi";
  `Sandbox(RateLimited)` → sessiz, sonraki değişimde; `Sandbox(Unavailable)` → "Bu biçim için
  görsel işçisi gerekli"; `Cancelled` → hiçbir şey.
- **Tüketici kurulumu:** shell bütün çıkışlarına tek `WallpaperDecoder` (varsayılan seçenekler);
  lock kendi sürecinde `linger: ZERO`; settings tek örnek (`cache_bytes` 16 MiB): kahraman/önizleme
  kartın fiziksel boyutu (üst sınır 1280×800) Cover, çıkış satırı `192×120 × ölçek` Cover, renk ayrı
  istek `128×128 Contain` → `nuxut_palette::source_color_from_frame` (elde daha büyük Contain kare
  varsa türetilir). İşçi ikilisi her süreçte aynı (`nuxut-image-worker`, arama sırası yukarıda).

`render` özelliğiyle **Wallpaper** (Background katmanı için):

```rust
impl Wallpaper {
    // KIRICI (ADR-0007 takip): decoder zorunlu — süreçte tek WallpaperDecoder, çıkışlar paylaşır
    pub fn new(fit: ImageFit, motion: impl Into<Motion> /* theme.motion.effects */,
               decoder: Arc<WallpaperDecoder>, notify: impl Fn() + Send + Sync + 'static) -> Self;
    pub fn set_source(&mut self, path, target_px: (u32, u32));   // paylaşılan çözücüye istek (öncekini iptal); bitince notify()
    // Bellek içi GÜVENİLİR kaynak (shell: gömülü SVG yerleşiğinin render_svg(svg, cover_crop) çizimi):
    // işçi/dosya/WallpaperDecoder yok; aynı (key, kare boyutu, fit) → no-op; süren set_source iptal;
    // çapraz geçiş ve bulanıklık/zemin dosya yoluyla aynı. Kare olduğu gibi (GPU'da fit); çıkış boyutu
    // değişince ÇAĞIRAN yeni boyutta yeniden çizer. Geçersiz kare → last_error, eski görsel kalır.
    pub fn set_image(&mut self, key: u64, frame: impl Into<WallpaperFrame> /* Image ya da paylaşılan kare */);
    pub fn source(&self) -> Option<&WallpaperSource>;  // WallpaperSource::{File(PathBuf), Image(u64)}; Debug yolu/anahtarı gizler
    pub fn animated_source(&self) -> bool;                        // kaynak animasyonluydu, ilk kare gösteriliyor
    pub fn set_visible(&mut self, bool, now); pub fn visible(&self) -> bool;  // yalnızca kaydedilir (durağan)
    pub fn set_fit / set_radii / set_motion;
    pub fn set_blur(&mut self, radius: Option<f32>);              // mantıksal px; None/≤0/NaN = kapalı
    pub fn blur(&self) -> Option<f32>;
    pub fn set_backdrop(&mut self, WallpaperBackdrop);            // letterbox zemini; varsayılan None
    pub fn backdrop(&self) -> WallpaperBackdrop;
    pub fn texture_bytes(&self) -> u64;                           // GPU dokuları (bulanık kopyalar dahil)
    pub fn draw(&mut self, &mut Painter, rect, now) -> WallpaperStatus { redraw, next_frame_in /* her zaman None */ };
}
pub enum WallpaperBackdrop {
    None,                                   // görselin dışı saydam (yüzeyin kendi arka planı)
    Color(Color),                           // düz renk (tema token'ı)
    Blur { radius: f32, fallback: Color },  // aynı görselin Cover yerleşimli bulanık kopyası; radius ekranda mantıksal px
}
impl WallpaperBackdrop { pub fn color(&self) -> Option<Color>; }  // Color ya da Blur'un yedeği (video yolu için)
```
**Bellek içi kaynak** (`set_image`, talep 2026-10-13, karar): shell
gömülü SVG yerleşiklerini kendisi çizer; önceden PNG'ye yazıp işçiden geri okuyordu (4K'da ~0.5 sn
fazladan tur). Güven: yalnızca çağıranın kendi ürettiği veri (gömülü varlığın çizimi); kullanıcı
dosyası her zaman `set_source` → yalıtılmış işçi. **Boyut kararı:** Wallpaper kareyi yeniden
örneklemez; çağıran çıkışın fiziksel boyutunda yeniden çizer (vektör kaynakta yeniden çizim keskin
ve tek geçiş; rasteri ölçeklemek büyütmede bulanık, küçültmede ikinci kopya olurdu). Yeni kare
gelene dek eski doku yeni dikdörtgene yerleşimle (GPU) çizilir. Anahtar `(key, kare boyutu, fit)`
`set_source`'un `(yol, hedef, fit)` tekilleştirmesiyle aynı kuralda: aynı istek no-op, her
`configure`'da çağrılabilir. Çok çıkış: kare bir kez çizilir, `WallpaperFrame` `Arc`'ıyla klonlanıp
her çıkışa verilir (kopya yok; doku yüklenince katman payını bırakır). `WallpaperDecoder`
önbelleğine girmez (çağıran zaten kareyi tutar ya da yeniden çizer). Boşta iş yok: kare/timer
istenmez, `notify` çağrılmaz (çağıran ana thread'de; bir kare ister). Testler
(`tests/it/wallpaper_blur.rs`, başsız EGL): dosyadan bellek içi kareye çapraz geçiş + durulunca boşta,
aynı anahtar no-op, yeni boyut kabul; süren dosya isteğini geçersiz kılar; geçersiz kare reddi.

Kaynak değişince yeni görsel eski görselin üstünde `motion.effects` ile çapraz geçer. Duvar kağıdı
her zaman durağandır (animasyonlu kaynakta ilk kare, `animated_source()`); kare/timer istenmez.
Kare GPU'ya yüklenince katmandaki paylaşılan pikseller bırakılır (CPU'da tutan yalnızca
`WallpaperDecoder`'ın `linger` önbelleği). Çıkış başına kalıcı `Loader` thread'i yok.
**Bulanıklık** (`set_blur`, kilit ekranı): her katmanın dokusu hazır olunca `Painter::blur_texture`
ile bir kez bulanıklaştırılıp katmanda saklanır; her karede yeniden yapılmaz. Anahtar
(yarıçap, `Painter::scale`, katman içeriği) değişince yeniden üretilir; yeni kopya GPU'da hazır
olana dek eskisi çizilir.
Görünür duvar kağıdında açma/kapama tek karede atlamaz: kopya hazır olunca keskin → bulanık,
kapatınca bulanık → keskin, geçiş hareketiyle (`set_motion`; yay, kesilebilir) çapraz geçer;
kopya kapanış geçişi bitince bırakılır. Bulanıklık açıkken gelen yeni kaynak, bulanık kopyası
hazır olmadan devreye girmez ve doğrudan bulanık görünür — keskin kare hiç görünmez. Çapraz
geçişte iki katman da bulanıktır. Kopyalar `texture_bytes()`'e sayılır (açıkken ×2). Durulunca
boşta CPU/GPU işi yok.
**Zemin** (`set_backdrop`, shell t095): görselin kaplamadığı alan (Contain letterbox'ı, saydam
pikseller) için, her katmanın altına tüm dikdörtgene, katmanın opaklığıyla boyanır; görsel yokken
de düz rengi (`Color` ya da `Blur`'un `fallback`'i) boyanır — yüzey opak kalır. `Blur` kopyası
yalnızca `ImageFit::Contain`'de üretilir: katmanın (Contain'e küçültülmüş, küçük) dokusu mevcut
yoldan, `Painter::blur_texture` ile bir kez bulanıklaştırılır ve `Cover` ile çizilir; yarıçap
dokuda Cover büyütme oranına bölünür (ekranda istenen yarıçap, küçük dokuda ucuz iş). Anahtar
(ekrandaki yarıçap, ölçek, dikdörtgen/doku oranı, durağan görselde içerik) değişince yeniden
üretilir; her karede değil, boşta iş yok. Yeni kaynak, zemin kopyası hazır olmadan devreye girmez. Kopya
`texture_bytes()`'e sayılır; Contain dışına ya da başka zemine geçince bırakılır. Kopya hazır
olana dek, diğer yerleşimlerde ve geçersiz yarıçapta `fallback` boyanır. Zemin değişimi geçişsiz
(bir kare isteyin). Çapraz geçişte yeni katmanın zemini de `t` opaklığıyla gelir (görsel altında
`t·(1−t)` kadar zemin payı; göze çarpmaz). **Video duvar kağıdı** `Wallpaper` değil `VideoPlayer`
ile çizilir; zemini aynı türle `VideoPlayer::set_backdrop` alır (aşağıda): `Color(c)` → `c`,
`Blur { fallback, .. }` → `fallback` (bulanık video zemini yok: kare başına bulanıklık gerekirdi),
`None` → saydam letterbox (yüzeyin kendi arka planı görünür). Kabuk iki türe aynı değeri verir.
Video: aşağıda "Video ve duvar kağıdı içe aktarma" (ADR-0002, ADR-0004).

### Video ve duvar kağıdı içe aktarma (nuxut-video)

Ayrı crate (`nuxut-video` → core, render, image, motion; dış: `libloading`, `blake3`, `rustix`).
libmpv **çalışma zamanında `dlopen`** ile yüklenir (`libmpv.so.2`): derlemede mpv başlığı ya da
kütüphanesi gerekmez, mpv'siz kurulumda her çağrı `VideoError::Unavailable` döner ve duvar
kağıdı görsele düşer. Bağlama kendi yazımımız (~20 C işlevi): `libmpv2` crate'i LGPL-2.1
olduğu için kullanılmaz (deny.toml). **`unsafe` istisnası:** yalnızca `nuxut_video::ffi`
modülünde, her blok `// SAFETY:` ile; geri kalan crate `#![deny(unsafe_code)]`.

**Özellikler:** `jxl` (varsayılan **KAPALI**, ADR-0007 takip) → `nuxut-image/jxl` (jxl-oxide, süreç
içi); yalnızca `Importer::in_process_unsandboxed` (örnek/test) yolunu etkiler. Üretim yolu
(`Importer::new(decoder)`) `.jxl` dahil her kullanıcı görselini yalıtılmış işçide çözer
(`cargo tree -p nuxut-video -e normal | grep jxl` boş). `nuxut-image/render` her zaman açıktır
(Painter dokusu).

**Oynatma** — libmpv render API'si (`vo=libmpv`) bizim GL context'imize çizer:

```rust
pub struct VideoPlayer;
impl VideoPlayer {
    /// `wake`: mpv yeni kare/olay bildirince herhangi bir thread'den çağrılır (MessageSender).
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, VideoError>;
    pub fn load(&mut self, path: &Path) -> Result<(), VideoError>; // open_media kuralları
    pub fn set_muted(&mut self, bool);           // varsayılan sessiz
    pub fn set_volume(&mut self, percent: f64);  // 0…100
    pub fn set_paused(&mut self, bool);
    pub fn set_visible(&mut self, bool);         // görünmez/tam ekran pencere → duraklat
    pub fn set_fit(&mut self, ImageFit);         // Cover: panscan=1, Contain: 0, Fill: keepaspect=no
    pub fn set_backdrop(&mut self, WallpaperBackdrop); // letterbox zemini; varsayılan None (saydam)
    pub fn backdrop(&self) -> WallpaperBackdrop;
    pub fn draw(&mut self, &mut Painter, rect, now) -> VideoStatus { redraw: bool };
    pub fn info(&self) -> VideoInfo { size, duration, has_audio, hwdec: Option<String> };
}
```
mpv seçenekleri: `hwdec=auto-safe`, `loop-file=inf`, `mute=yes`, `keep-open=yes`,
`audio-client-name=nuxut`. Kare akışı: mpv update callback → `wake` → uygulama
`request_frame`; `mpv_render_context_update` yeni kare yoksa çizim yapılmaz (boşta iş yok).

**Letterbox zemini** (`set_backdrop`): mpv'nin kendi kenar temizliğiyle uygulanır —
`background=color`, `background-color=#AARRGGBB` (`backdrop.color()`, yoksa `#00000000`),
`border-background=color` (mpv'nin `blur` seçeneği kare başına bulanıklık olurdu). mpv render
hedefini her çizimde zaten temizler (premultiplied; saydam renk gerçekten saydam kalır), bu yüzden
ne ek çizim ne ek doku var; boşta da iş yok. Videonun altına `Painter` ile zemin boyamak her
karede tam dikdörtgen bir ek dolgu olurdu — seçilmedi. Renk onaltılık verilir: mpv kesirli
`r/g/b/a`'yı 8 bite keserek alır (0x22 → 0x21). Sürüm uyumu: mpv < 0.38'de `background` rengin
kendisidir (`background-color` yok) — `background=color` reddedilince renk `background`'a
yazılır; `border-background` (≥ 0.39) yoksa yok sayılır. Değişim geçişsiz; `force` ile son kare
(duraklatılmışta da) yeni zeminle yeniden çizilir — çağıran bir kare ister. Köşe yarıçapı ve
opaklık zemine de uygulanır (doku `Painter::image` ile çizilir). Varsayılan `None`: önceki
sürümde letterbox mpv'nin siyahıydı.

**Güvenilmeyen medya** — kaynak dosya saldırgan kontrollü olabilir; tek kapı:
- `pub fn open_media(&Path) -> Result<File, SourceError>`: mutlak yerel yol, son bileşen
  sembolik bağ değil (`O_NOFOLLOW`), açtıktan sonra `fstat` ile normal dosya (FIFO/aygıt/dizin
  yok; `O_NONBLOCK`). İçe aktarma (hash, başlık, görsel baytları, mpv) ve oynatıcı yalnızca bunu
  kullanır.
- mpv'ye **yol ya da URL verilmez**: `Mpv::load(MediaSource)` → `fdclose://N` (ya da kodda
  sabit `lavfi` üreteci, yalnızca VA-API yoklaması). fd mpv'ye ancak `loadfile` kabul edilince
  geçer; reddedilirse bizde kapanır. Kabul edilen sınır: kabul edilmiş ama hiç açılmamış bir
  girdinin fd'si (ör. açılmadan `replace` edilen) mpv içinde kalabilir; oynatıcı tek dosya
  yüklediği için pratikte oluşmaz.
- Her `Mpv` örneğine `ffi::HARDENING` önce uygulanır, çağıran ezemez: `config/scripts/ytdl/
  input/osc` kapalı, `terminal=no`, `msg-level=all=no` (iletiler yol/üst veri taşır),
  `demuxer=lavf` (playlist/EDL/cue/sıralı bölüm demuxer'ları denenmez),
  `load-unsafe-playlists=no`, `access-references=no` (HLS/concat/mov iç açılışları),
  `demuxer-lavf-o`/`stream-lavf-o` `protocol_whitelist=[file,fd]`, `sub-auto`/`audio-file-auto`/
  `cover-art-auto`/`autoload-files=no`, `sid=no` (gömülü/varsayılan alt yazı seçilmez),
  `osd-level=0` (OSD iletileri çizilmez). `FileLoaded`'da `playlist-count != 1` → durdur.
- Kodlama kipi (içe aktarma) ve VA-API yoklaması aynı `Mpv::new` yolundan geçer.

**Render eki** — mpv'nin dokuya çizebilmesi için. Çekirdek `Painter` backend bağımsız kalır
(ADR-0003 kural 3); GL kancası `nuxut_render::gl` uzantısındadır:

```rust
// Boş, şeffaf; bu karede ayrılır. Tüketici boyut değişmedikçe AYNI dokuyu tutar (her karede
// çağırmak her karede yeni doku ayırır).
impl Painter<'_> { pub fn render_target(&mut self, width: u32, height: u32) -> Texture; }
pub mod gl {
    pub trait GlPass: Send + Sync {
        /// `render_frame` içinde yüklemelerden sonra, çizimden önce; `fbo` hedefe bağlı.
        fn render(&self, gl: &glow::Context, fbo: u32, size: (u32, u32));
    }
    /// GL kaynağı tutan dış değerler (mpv render context'i) herhangi bir thread'de, context
    /// güncel değilken düşebilir. `defer` onları renderer'a taşır: bir sonraki karenin başında
    /// (context güncel) ya da `Renderer::drop`'ta düşer; renderer yoksa sızdırılır (context'siz
    /// GL çağrısı bazı sürücülerde çöker). Klonlanır, Send.
    pub struct GlGraveyard;
    impl GlGraveyard { pub fn defer(&self, value: Box<dyn Send>); }
    pub trait PainterGlExt {
        fn gl_pass(&mut self, target: &Texture, pass: Arc<dyn GlPass>);
        fn gl_graveyard(&self) -> GlGraveyard;
    }
    impl PainterGlExt for Painter<'_> { … }
}
```
Renderer geçişten sonra framebuffer ve viewport'u geri yükler; program, VAO, ARRAY/ELEMENT ve
PIXEL_UNPACK/PACK buffer, birim 0/1 doku ve sampler, scissor, stencil, depth, cull, polygon
offset, color mask, blend func/equation, UNPACK_ALIGNMENT/ROW_LENGTH ve PACK_ALIGNMENT'i kendi
varsayımlarına sıfırlar (durumu kasıtlı bozan GL testi: PBO, sampler, cull dahil). Hedef doku
normal `Painter::image` ile çizilir (köşe, opaklık, çapraz geçiş görselle aynı yoldan).
`VideoPlayer` düşerken render context'i `GlGraveyard` ile GL thread'inde (context güncel) yok
edilir; ardından çekirdeğin `mpv_terminate_destroy`'u (çözücü thread'lerini bekler) arka plan
thread'inde yapılır — render thread'i kare kaçırmaz (`Mpv::terminate_in_background`; kısa
ömürlü yoklama/içe aktarma örnekleri eşzamanlı kapanır).

**İçe aktarma** — duvar kağıdı eklenince kaynak, en ucuz oynatılan biçime dönüştürülüp
önbelleğe alınır (ADR-0004). Kaynak dosyaya dokunulmaz.

```rust
pub struct ImportRequest { pub source: PathBuf, pub target_px: (u32, u32) /* çıkış fiziksel boyutu */ }
pub enum Imported { Image(PathBuf), Video(PathBuf) }   // önbellek dosyası ya da (gerekmiyorsa) kaynak
pub enum ImportEvent { Progress(f32 /* 0…1 */), Done(Result<Imported, ImportError>) }
pub struct Importer;
impl Importer {
    // Kullanıcı görselini çözen yalıtılmış çözücü ZORUNLU (ADR-0007; `Wallpaper::new` ile aynı
    // ilke): duvar kağıdı profili `Sandbox::new(SandboxConfig::wallpaper())`, süreçte varsa paylaşılır.
    pub fn new(decoder: Arc<dyn IsolatedDecoder>) -> Self;        // $XDG_CACHE_HOME/nuxut/wallpapers
    pub fn with_dir(dir: PathBuf, decoder: Arc<dyn IsolatedDecoder>) -> Self;
    pub fn default_dir() -> PathBuf;
    pub fn in_process_unsandboxed(dir: PathBuf) -> Self;          // YALNIZCA örnek/test: süreç içi, Trust::System
    pub fn lookup(&self, &ImportRequest) -> Option<Imported>;     // önbellekte varsa anında
    pub fn start(&self, ImportRequest, events: impl Fn(ImportEvent) + Send + 'static) -> ImportJob;
    pub fn run(&self, &ImportRequest, cancel: &CancelFlag, progress: &dyn Fn(f32)) -> Result<Imported, ImportError>;
    pub fn capabilities(&self) -> &DecodeCaps;                    // VA-API kod çözme yetenekleri (önbellekli yoklama)
}
impl ImportJob { pub fn cancel(&self); }                          // düşürmek iptal etmez; süren görsel çözümünde işçi ≤ 50 ms'de öldürülür
impl Importer {
    pub fn set_cache_limit(&self, bytes: u64);                   // LRU, varsayılan 2 GiB
    pub fn set_limits(&self, ImportLimits); pub fn limits(&self) -> ImportLimits; // kodlama bütçesi, sonraki içe aktarmalarda
    pub fn still_frame(&self, &ImportRequest, at: f64) -> Result<Imported, ImportError>; // videodan QOI kare
}
impl DecodeCaps { pub fn software_only(&self) -> bool; pub fn target_codec(&self) -> Codec; }
/// Yalnızca kodlanan kaynaklara (animasyon, yeniden kodlanan video); olduğu gibi oynatılan
/// video bağlantı kaydıdır, iş üretmez. Eşit olan geçer.
pub struct ImportLimits { pub max_duration: Duration, pub max_frames: u64, pub encode_timeout: Duration }
impl ImportLimits { pub const DEFAULT: Self; }   // 10 dk, 36 000 kare (10 dk × 60 fps), 2 sa kodlama
pub enum BudgetExceeded { Duration { actual, limit }, Frames { actual, limit } } // Display kullanıcıya gösterilebilir
// ImportError::Budget(BudgetExceeded)  "içe aktarılamadı, kaynak çok uzun: 12 dk (sınır 10 dk)"
// ImportError::TimedOut(Duration)      "dönüştürme 2 sa içinde bitmedi, durduruldu"
```
- **Yalıtım (ADR-0007 takip, `docs/design/video-import-sandbox.md`):** ana süreç kaynaktan
  yalnızca ilk 4 KiB imzaya bakar (görsel mi video mu). Görselin tüm ayrıştırması — durağan
  görselin çözümü, animasyon tespiti, kare sayısı/süre için kap yürüyüşü — tek
  `IsolatedDecoder::decode_file` çağrısında yapılır: dosya `open_media` ile bir kez açılır
  (sembolik bağ yok, normal dosya), fd akıtılır; `Limits::WALLPAPER`, Cover hedef (kırpmasız),
  ilk kare, anahtar `file_key("video-import", yol)`. Yanıtın `animated_source`'u ve `animation`'ı
  (tel v4) yolu seçer; QOI çözücünün karesinden yazılır. `still_frame`'de mpv'nin güvenilmeyen
  medyadan yazdığı PNG de aynı çözücüde açılır. Çözücü hatası (işçi yok/öldü/zaman aşımı) olduğu
  gibi `ImportError::Image` döner, süreç içi çözüme **düşülmez**; işçi yokken `Sandbox` S1
  kuralını uygular (temel raster K2-b ile süreç içinde, `Untrusted`; SVG/JXL/AVIF asla). İptal
  (`ImportJob::cancel`) süren çözümde de etkilidir → `Cancelled`. Animasyon ve video baytlarını
  mpv çözer (süreç içi, ADR-0002 riski, bu işin kapsamı dışında); animasyonun tuval boyutu mpv
  yoklamasından alınır (`Limits::DEFAULT` kenar/piksel kapısı), yoklama başarısızsa kaynak görsel
  olarak kalır.
- Önbellek adı: `<blake3(içerik)[..16]>-<W>x<H>.<ext>`; yazım geçici dosya + `rename`.
- Durağan görsel → hedefe cover küçültülmüş **QOI** (`.qoi`; ADR-0004 ölçümleri).
- Animasyonlu GIF/WebP/APNG → döngüsel H.264 (ya da VA-API'nin çözebildiği en ucuz codec)
  `.mkv`, sessiz; şeffaflık siyaha düzlenir.
- Video → çözünürlük hedeften büyükse ya da codec donanımla çözülemiyor ve yazılımda pahalıysa
  (AV1/VP9/HEVC) hedef boyutta yeniden kodlanır; ses akışı korunur. Gerekmiyorsa kaynak döner.
- Kodlama libmpv'nin kodlama kipiyle (`o=`, `ovc=`, `vf=scale`) ayrı bir mpv örneğinde,
  arka plan thread'inde; ilerleme `percent-pos`'tan, iptal `quit` + geçici dosya silme.
- **Kodlama bütçesi** (`ImportLimits`; dosya/tuval sınırı `nuxut_image::Limits` içindeki bir
  animasyon da binlerce kare ya da saatler sürebilir, kodlaması yalnızca iptalle sınırlı kalmaz):
  1. *Önce ucuz yoklama* — animasyonlu görselde `animation_info` (kesin kare + süre, piksel
     çözülmez; **işçide** sayılır, tel v4 — yapı tanınmazsa akışta kabın tahmini de denetlenir);
     videoda mpv `vo=null` yoklamasının `duration` ve `container-fps`'i; kap kare
     hızı bildirmiyorsa (Matroska) ve video kodlanacaksa ayrı örnekle ≤ 2 sn oynatılıp
     `estimated-vf-fps` (%5 pay: tahmin dalgalanır, sınırdaki kaynak yanlışlıkla reddedilmez).
     Aşan kaynak kodlayıcıya hiç gitmez → `ImportError::Budget`. Bütçe aşan animasyon artık
     görsel olarak oynatılmaya da düşmez (hata çağırana gider).
  2. *Akışta* — kodlama yoklanırken kabın süre/kare tahmini aşılırsa durdurulur (`Budget`).
  3. *mpv'nin kendi sayacı* — her kodlamaya `end=<max_duration>` ve `frames=<max_frames>`
     eklenir: üst veri yalan ya da yoksa (sonsuz akış) da kodlanan iş bütçede kesilir; bu
     durumda çıktı bütçe uzunluğunda kırpılmış bir döngüdür (hata değil; ayırt edilemez).
  4. *Duvar saati* — `encode_timeout` aşılırsa `quit` + mpv örneği **dönmeden önce eşzamanlı**
     yok edilir (`mpv_terminate_destroy`: çekirdek ve kodlayıcı iş parçacıkları biter, çıktı
     kapanır), ardından yarım `.tmp.mkv` silinir → `ImportError::TimedOut`. libmpv süreç içi
     çalışır, alt süreç yoktur; test (`tests/import_timeout.rs`) iş parçacığı sayısının ve
     `/proc/self/task/*/children`'ın önceki hâline döndüğünü ve geçici dosya kalmadığını
     doğrular. Sınır: tek bir karenin çözümü takılırsa `terminate_destroy` onu bekler
     (süreç içi kütüphane öldürülemez; kare boyutu `Limits` ile sınırlı).
     Test kararlılığı (2026-10-17): test eskiden 800 ms'lik son tarihin kodlama sürerken dolacağını
     duvar saatine bırakıyordu; yük altında mpv ilk ilerlemeyi son tarihten önce veremeyince
     "ilerleme > 0,05" iddiası düşüyordu (ürün doğru davranıyordu: zaman aşımı, temizlik tam).
     Test artık ilk gerçek ilerlemede geri çağırmada `timeout` kadar bekler (son tarih o anda
     kesin dolmuştur); kodlama son tarihe hiç yetişmezse süre ikiye katlanıp yeniden denenir,
     her denemede temizlik aynı biçimde doğrulanır.
- VA-API yoklaması: her aday codec için küçük bir test klibi kodlanıp `hwdec=vaapi`, `vo=null`
  ile açılır, `hwdec-current` okunur; sonuç `$XDG_CACHE_HOME/nuxut/hwcaps.toml`'da saklanır.
- **Donanım çözme yoksa** (`capabilities().software_only()`): 1080p30 yazılım çözme bu VM'de
  bir çekirdeğin ~%85–97'si. Önerilen varsayılan: video eklenirken kullanıcıya uyarı ve
  `still_frame` ile durağan kareye geri dönüş seçeneği (Ayarlar/shell karar verir).
- Platform (ADR-0003): libmpv yolu yalnızca Linux; diğer platformlarda `available() == false`,
  kurucular `Unavailable`; durağan görsel içe aktarma (QOI) her yerde çalışır.
- Oynatma notu: `vo=libmpv`'de render context'i yaratılmadan `loadfile` video çıkışını
  kapatır; `VideoPlayer::load` yüklemeyi context ilk GL geçişinde yaratılana dek bekletir.

### Locale (nuxut-locale, `nuxut::locale`)

```rust
pub struct Locale;   // Clone; Default = en_US
impl Locale {
    pub fn from_env() -> Self;                         // LC_ALL > LC_TIME > LANG
    pub fn numeric_from_env() -> Self;                 // LC_ALL > LC_NUMERIC > LANG (sayı kategorisi)
    pub fn from_vars(lc_all, lc_category, lang) -> Self; // saf hâli (testler)
    pub fn parse(&str) -> Option<Self>;                // tr_TR.UTF-8, sr_RS@latin, tr; C/POSIX → None
    pub fn en_us() -> Self;
    pub fn name(&self) -> &str; language(); region(); chrono(&self) -> chrono::Locale;
    pub fn first_weekday(&self) -> Weekday;            // CLDR firstDay bölgeleri
    pub fn uses_12_hour(&self) -> bool;                // geçersiz kılma varsa o, yoksa locale'in %X kalıbından
    pub fn with_12_hour(self, Option<bool>) -> Self;   // [locale] hour_cycle: Some(true)=h12, Some(false)=h23, None=veri
    pub fn twelve_hour_override(&self) -> Option<bool>;
    pub fn day_periods(&self) -> [String; 2];         // %p (AM_STR/PM_STR); boşsa ["AM", "PM"]
    pub fn month_name(&self, month: u32, Length) -> String; weekday_name(Weekday, Length);
    pub fn format_date(&self, NaiveDate, &str) -> Result<String, FormatError>;
    pub fn format_time(&self, NaiveTime, &str) -> Result<String, FormatError>;
    pub fn format_datetime<Tz>(&self, &DateTime<Tz>, &str) -> Result<String, FormatError>;
    pub fn decimal_separator(&self) -> &'static str;   // LC_NUMERIC: tr_TR ",", en_US "."
    pub fn grouping_separator(&self) -> &'static str;  // tr_TR ".", fr_FR U+202F; gruplamasızda ""
    pub fn grouping(&self) -> &'static [i64];          // sağdan grup boyları, son tekrar; -1/boş = yok

    // --- chrono türü istemeyen API (ayarlar "Dil ve biçimler") ---
    pub fn from_language_tag(&str) -> Option<Self>;    // BCP 47: tr, pt-BR, sr-Latn-RS (→ @latin), zh-Hant (→ zh_TW)
    pub fn modifier(&self) -> Option<&str>;            // sr_RS@latin → "latin"
    pub fn language_name(&self) -> String;             // LC_ADDRESS lang_name, ilk harf büyük: "Türkçe", "English", "Français"
    pub fn country_name(&self) -> Option<&'static str>;// LC_ADDRESS country_name: "Türkiye", "United States"
    pub fn display_name(&self) -> String;              // "English (United States)", "Català (Espanya, valencia)"
    pub fn number_symbols(&self) -> NumberSymbols;
    pub fn percent_pattern(&self) -> PercentPattern;                               // tr "%50", en "50%", de "50 %"
    pub fn format_percent(&self, ratio: f64, fraction_digits: u8) -> String;     // 0.5 → tr "%50", en "50%", fr "50 %"
    pub fn format_number(&self, value: f64, fraction_digits: u8) -> String;          // tr "1.234.567,89", en_IN "12,34,567"
    pub fn format_number_with(&self, value: f64, &NumberOptions) -> String;
    pub fn format_ymd(&self, year: i32, month: u32, day: u32, DateStyle) -> Result<String, FormatError>;
    pub fn format_month_year(&self, year: i32, month: u32) -> Result<String, FormatError>; // "Ekim 2026", "2026年10月"
    pub fn compose_month_year(&self, month: &str, year: i32) -> String;           // hazır ay metni + yıl, locale sırasıyla
    pub fn standalone_month_name(&self, month: u32) -> String;                    // alt_mon (ru "Октябрь"); ja "10月"
    pub fn format_hm(&self, hour: u32, minute: u32) -> Result<String, FormatError>; // "13:05", "1:05 PM", fi "13.05"
    pub fn format_system_time(&self, SystemTime, DateTimeStyle) -> Result<String, FormatError>; // yerel saat dilimi
}
pub fn available() -> &'static [&'static str];         // gömülü locale adları: kanonik, sıralı, tekrarsız (POSIX ve @euro hariç)
#[non_exhaustive] pub enum DateStyle { Short /*%x*/, Long /*varsayılan*/, Full /*haftanın günüyle*/ }
#[non_exhaustive] pub enum DateTimeStyle { Date(DateStyle), Time, DateTime(DateStyle) }
pub struct NumberSymbols { pub decimal: &'static str, pub group: &'static str, pub grouping: &'static [i64],
                          pub percent: PercentPattern } // Copy
pub struct PercentPattern { pub prefix: bool, pub gap: &'static str }   // Copy; SUFFIX = "50%"
impl PercentPattern { pub const SUFFIX: Self; pub fn for_locale(language: &str, region: &str) -> Self;
                      pub fn apply(self, number: &str) -> String; }      // hazır sayı metnine: tr "-5" → "-%5"
impl NumberSymbols { pub const PLAIN: Self; pub fn format(&self, f64, &NumberOptions) -> String; }
#[non_exhaustive] pub struct NumberOptions { pub min_fraction_digits: usize, pub max_fraction_digits: Option<usize>,
    pub min_integer_digits: usize, pub grouping: bool, pub percent: bool }   // Default; fixed(digits)
pub const MAX_FRACTION_DIGITS: usize = 20;  MAX_INTEGER_DIGITS: usize = 64;
pub enum Length { Short, Long }
pub enum FormatError { Pattern, Value }
pub use chrono::Weekday;
```

Kurallar:
1. **Seçim POSIX'tir:** ilk boş olmayan değişken karar verir; `LC_ALL=C` iken `LANG`'a
   düşülmez. `C`/`POSIX`, geçersiz karakterli (yalnızca `[A-Za-z0-9_.@-]`, ≤ 64 bayt) ya da
   veride olmayan ad → `en_US`. Değiştirici veride yoksa değiştiricisiz ada düşülür.
2. **Veri gömülü:** glibc locale verisi (chrono `unstable-locales` → `pure-rust-locales`);
   sistemde locale üretilmiş olması gerekmez. `%x`/`%X`/`%c` locale kalıbına açılır.
3. **Panik yok:** kalıp önce ayrıştırılır (`Pattern`), değer kalıbı karşılamazsa (`%Z` saat
   dilimsiz) `Value`; chrono'nun panik eden `to_string` yolu kullanılmaz. Kalıp ≤ 256 bayt
   (kullanıcı config'inden gelebilir).
4. **Kullanılabilir liste gömülü veridir:** `available()` sistemde *kurulu* locale'leri değil,
   biçimleyebildiğimiz gömülü glibc verisini listeler (biçimleme kurulu locale'e bakmaz; alt süreç
   / `locale -a` yok). `@euro` eşleri (tarih/sayı verisi değiştiricisizle aynı) ve `POSIX` hariç;
   yazı/varyant değiştiricileri (`@latin`, `@cyrillic`, `@valencia`, `@devanagari`) dahil. Her ad
   `parse` ile kendisine çözülür. Yalnızca dil verilirse (`en`, `ja`, `eo`): `<dil>_<DİL>` →
   yaygın bölge tablosu (`en` → `en_US`, `zh` → `zh_CN`) → bölgesiz ad → listedeki ilk ad.
5. **Dil adı** glibc `LC_ADDRESS lang_name`'den; menü biçimi için ilk harf büyütülür (Türk dilleri
   `i` → `İ`; ilk sözcüğünde büyük harf olan "isiZulu" ve Gürcüce olduğu gibi).
6. **Tek sayı uygulaması:** `NumberSymbols::format` (Fluent `NUMBER` de bunu çağırır). NaN →
   "NaN", sonsuz → "∞"/"-∞", kesir ≤ 20, dolgu ≤ 64; 1e300 gibi değerler üssüz yazılır (sınırlı
   uzunluk), sıfıra yuvarlanan negatif işaretsiz. Gruplama glibc `grouping` (en_IN `[3, 2]`).
   Yüzde kalıbı glibc'de yok: CLDR `percentFormats`'tan küçük tablo (`PercentPattern::for_locale`;
   tr `%50`, eu `% 50`, de/es/sv/ru… `50 %` U+00A0, fr `50 %` U+202F, de_CH/fr_CH ve tabloda olmayan
   her dil `50%`). Eksi her zaman başta (tr `-%5`). Fluent `NUMBER($n, style: "percent")`,
   `NumberOptions::percent` ve `Locale::format_percent` aynı kalıbı kullanır; elle `"{n}%"` yazılmaz.
   Yüzde × 100 artığı: `max_fraction_digits = None` iken değer 15 anlamlı basamağa yuvarlanır
   (0.29 → `29%`, `28,999999999999996%` değil; 0.1234 → `12.34%`).
7. **Tarih biçimleri:** gün/ay/yıl sırası `%x` (`d_fmt`) kalıbından; ISO `d_fmt` (sv_SE, en_CA,
   hu_HU) dil sırası sayılmaz (İngilizce A-G-Y, diğerleri G-A-Y). Noktalama küçük gerekçeli tablo
   (`date.rs` modül belgesi): Doğu Asya sayısal "2026年10月4日"/"2026년 10월 4일", hu "2026. október
   4.", de-ailesi "4. Oktober 2026", es/pt "4 de octubre de 2026", A-G-Y "October 4, 2026".
   Tarih içinde `%B` (glibc `mon`, ru/pl/cs -in hâli), başlıkta `alt_mon` (yalın). `format_hm`
   saat kalıbından saniye ve saat dilimi atılarak (24 s `t_fmt`, 12 s `t_fmt_ampm`); 12/24 seçimi
   `uses_12_hour` (`with_12_hour` geçersiz kılması; döngüye uymayan veri kalıbı → `%H:%M` /
   `%-I:%M %p`, gün dilimi adı boş locale'de "AM"/"PM"). Açık strftime kalıbı geçersiz kılmaya
   uymaz (çağıranın kalıbı kazanır). Geçersiz
   tarih/saat ve ±9999 yılı dışındaki `SystemTime` → `Value` (panik yok). Bilinen sınır: ek isteyen
   diller (fi "lokakuuta", ru "г.") yalın kalır.

### Uluslararasılaştırma (nuxut-i18n, `nuxut::i18n`; ADR-0010)

```rust
// Kaynak: repo başına locales/<dil>.ftl, derleme zamanında gömülür.
pub struct Source { pub domain: &'static str, pub embedded: &'static [(&'static str, &'static str)] }
impl Source { pub fn languages(&self) -> impl Iterator<Item = &'static str>; }
embed!("settings")                      // → Source; <CARGO_MANIFEST_DIR>/locales/*.ftl (include_str!)
embed!("ui", "../../locales")           // crate köküne göreli dizin

pub struct Catalog;                     // Send + Sync, değişmez; Arc ile paylaşılır
impl Catalog {
    pub fn load(sources: &[Source], prefs: &LanguagePrefs) -> Self;   // asla başarısız olmaz
    pub fn empty() -> Self;                                           // her istek anahtarı döndürür
    pub fn text(&self, id: &str) -> Cow<'_, str>;
    pub fn format(&self, id: &str, args: &Args<'_>) -> Cow<'_, str>;
    pub fn attr(&self, id: &str, attr: &str, args: Option<&Args<'_>>) -> Option<Cow<'_, str>>;
    pub fn has(&self, id: &str) -> bool;
    pub fn languages(&self) -> &[LanguageIdentifier];                // çözülmüş zincir
    pub fn direction(&self) -> Direction;                            // zincirin ilki
    pub fn is_pseudo(&self) -> bool;
    pub fn numeric(&self) -> &nuxut_locale::Locale;   // sayı locale'i ([locale] formats ya da ortam); NUMBER bununla
}
pub enum Direction { Ltr, Rtl }
pub type Args<'a> = fluent_bundle::FluentArgs<'a>;   pub use fluent_bundle::FluentValue;
pub use unic_langid::LanguageIdentifier;
pub const MAX_OUTPUT_BYTES: usize = 4096;  MAX_USER_FILE_BYTES: u64 = 1 MiB;  MAX_USER_MESSAGES = 10_000;

pub struct LanguagePrefs;
impl LanguagePrefs {
    pub fn from_config(&LocaleConfig) -> Self;          // [locale] + süreç ortamı + default_user_dir()
    pub fn resolve(language: &LocaleSetting, formats: &LocaleSetting, env: &LocaleEnv) -> Self; // saf
    pub fn fixed(tags: &[&str]) -> Self;                // testler/araçlar: ortam yok, en_US sayı, kullanıcı dizini yok
    pub fn with_user_dir(Option<PathBuf>) / with_pseudo(bool) / with_numeric(nuxut_locale::Locale) -> Self;
    pub fn requested(&self) -> &[LanguageIdentifier]; is_pseudo(); numeric() -> &Locale; user_dir() -> Option<&Path>;
}
pub struct LocaleEnv { nuxut_lang, language, lc_all, lc_messages, lc_numeric, lang: Option<String> } // from_process()
pub fn posix_to_langid(&str) -> Option<LanguageIdentifier>;   // tr_TR.UTF-8 → tr-TR, sr_RS@latin → sr-Latn-RS
pub fn formats_locale(&LocaleSetting) -> Option<nuxut_locale::Locale>;  // [locale] formats; None = auto (env)
pub fn time_locale_for(&LocaleConfig) -> nuxut_locale::Locale;   // formats (auto → Locale::from_env) + hour_cycle
pub fn twelve_hour_override(HourCycle) -> Option<bool>;          // auto → None, h12 → Some(true), h23 → Some(false)
pub fn default_user_dir() -> Option<PathBuf>;                  // $XDG_DATA_HOME/nuxut/locales

// Dil seçimi listesi (ayarlar): kayıtlı kaynakların gömülü dilleri ∪ kullanıcı dizini (yalnızca
// düz <dil>.ftl dosyaları; symlink/dizin/geçersiz ad atlanır, içerik okunmaz), BCP 47 etiketine
// göre sıralı, tekrarsız; `und` ve `qps` yok.
pub fn available_languages() -> Vec<LanguageIdentifier>;        // registered() + default_user_dir()
pub fn available_languages_in(&[Source], Option<&Path>) -> Vec<LanguageIdentifier>; // saf hâli
pub fn language_locale(&LanguageIdentifier) -> Option<nuxut_locale::Locale>;     // tr → tr_TR, sr-Latn → sr_RS@latin
pub fn language_label(&LanguageIdentifier) -> String;  // "Türkçe", "English"; bölge/yazı varsa "Português (Brasil)"; yoksa etiket

// Süreç geneli (okuma kilitsiz: iş parçacığı önbelleği + nesil sayacı)
pub fn set_current(Arc<Catalog>);  pub fn current() -> Arc<Catalog>;   // kurulmadıysa boş katalog
pub fn time_locale() -> nuxut_locale::Locale;     // etkin saat/tarih locale'i: son init/configure [locale]
                                                  // (time_locale_for) > ortam; locale_changed'de yeniden al
pub fn numeric_locale() -> nuxut_locale::Locale;  // etkin sayı locale'i: kurulu katalogunki > son init/configure
                                                  // [locale] formats > ortam. Uygulamanın kendi format_percent /
                                                  // format_number'ı için (Locale::numeric_from_env formats'ı görmez)
pub fn register(Source);  pub fn registered() -> Vec<Source>;          // aynı domain'in yerine
pub fn set_register_hook(Option<Arc<dyn Fn() + Send + Sync>>);         // yeni domain kaydında (runtime kurar)
pub fn init(&[Source]) -> Arc<Catalog>;     // kaydet + framework.toml [locale] oku + kur + set_current
pub fn configure(&LocaleConfig) -> bool;    // ayar değiştiyse true (+ kaynak varsa yeniden kur)
pub fn reload() -> Arc<Catalog>;            // aynı ayarla yeniden kur (runtime kullanıcı dosyası değişince çağırır)
t!("shell-bar-clock") -> String;  t!("shell-bar-battery-tooltip", percent = 80, time = s.as_str()) -> String

// Kullanıcı çeviri dizini izleme (nuxut_i18n::watch; runtime kendisi kullanır, uygulama çağırmaz)
pub const DEBOUNCE: Duration /* 200 ms */;  pub const MAX_WAIT: Duration /* 1 s */;  pub const MAX_ANCESTORS: usize /* 3 */;
pub struct Debounce;   // saf: new(delay, max_wait) | default(); event(now) -> Instant; deadline(); is_pending(); poll(now) -> DebouncePoll
pub enum DebouncePoll { Idle, Wait(Instant), Fire }
pub struct ChangeTracker;  // new(root, &[domain]) taban özeti; take_change() -> bool (meta → içerik özeti)
pub struct UserDirWatcher; // Linux: new(root, &[domain]) -> io::Result; fd(); process_events() -> bool; take_change(); watched_dirs(); set_domains(&[domain]) -> bool; domains()

// framework.toml şeması (nuxut-config; FrameworkConfig.locale, Theme.locale)
pub struct LocaleConfig { pub language: LocaleSetting, pub formats: LocaleSetting }
pub struct LocaleSetting { pub invalid: bool, .. }  // AUTO, parse(&str), is_auto(), as_str(), value() -> Option<&str>
nuxut_config::locale::load_locale_config() -> LocaleConfig;   // yalnızca [locale]; yok/bozuk → auto

// Denetim (her repo tek #[test])
pub mod check {
    pub fn catalog(dir, prefix: &str) -> Report;
    pub fn catalog_embedded(&Source, dir, prefix) -> Report;   // + gömülü liste/içerik diskle aynı mı
    pub fn catalog_texts(&[(&str, &str)], prefix) -> Report;   // bellekten (testler)
    pub fn source_guard(root) -> Report;                       // root altındaki her src/**/*.rs
    pub struct SourceGuard;  // new(root).add_ui_calls(&[..]).add_diagnostic_calls(&[..]).allow_file(p).run(); scan_text(rel, text)
    pub const DEFAULT_UI_CALLS: &[&str];
    pub struct Report { pub errors: Vec<Finding>, pub notes: Vec<String> }  // is_ok(), assert_ok(), has(&str)
    pub struct Finding { pub file: String, pub line: usize, pub message: String }   // Display: dosya:satır: ileti
}
```

Uygulama girişi:

```rust
fn main() {
    nuxut::i18n::init(&[nuxut::i18n::embed!("shell") /*, framework ui kaynağı (adım 2) */]);
    nuxut::run(App::new());   // [locale] değişince: configure → current() yeni → App::locale_changed
}
// testler/kök: tests/i18n.rs
#[test] fn i18n_catalog() {
    nuxut::i18n::check::catalog_embedded(&nuxut::i18n::embed!("shell"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/locales"), "shell").assert_ok();
}
#[test] fn i18n_source_guard() { nuxut::i18n::check::source_guard(env!("CARGO_MANIFEST_DIR")).assert_ok(); }
```

Kurallar:
1. **Zincir:** istenen diller `fluent-langneg` (Filtering) ile mevcut dillere (gömülü ∪ kullanıcı
   dizini) eşlenir, sonra `en`, en sona `tr` (tekrarsız). Mesaj zincirde ilk bulunduğu dilden;
   çözümleme hatalı dil (eksik değişken, kırık/döngülü referans) atlanır, hiçbiri temiz değilse
   ilk sonuç. Hiçbir dilde yoksa anahtarın kendisi; uyarılar anahtar başına bir kez
   (`tracing::warn!`, yalnızca anahtar — içerik yok).
2. **Dil seçimi:** `NUXUT_LANG` (yalnızca geliştirme; `qps` sözde dil, başka etiket o dil) >
   `[locale] language` etiketi > `auto`: `LANGUAGE` (iki nokta listesi) > `LC_ALL` > `LC_MESSAGES`
   > `LANG`, ilk boş olmayan; `C`/`POSIX`/boş/çözülemeyen → `en`. Geçersiz değer şemada `auto` +
   tema uyarısı (`locale.language geçersiz`). Biçimler (`formats`) dilden bağımsız: sayı ayırıcıları
   `LC_ALL` > `LC_NUMERIC` > `LANG` ya da `formats = "<locale>"`; tarih/saat biçimleyen bileşenler
   `formats_locale` ile aynı ayarı okur; saat:dakika 12/24 saati `[locale] hour_cycle` (`auto` = locale
   verisi, `h23`, `h12`; geçersiz → uyarı + `auto`) geçersiz kılar: `time_locale()` / `time_locale_for`.
   Uygulamanın açık biçimi (shell `clock.format`) her zaman kazanır. Yalnızca `hour_cycle` değişimi de
   `configure` → `true` → `App::locale_changed` (karar: 2026-10-13).
3. **Sayılar:** `{ $n }` ve `NUMBER($n, minimumFractionDigits: 2)` `nuxut-locale` ile (tek uygulama:
   `NumberSymbols::format`, `Locale::format_number` ile aynı çıktı) biçimlenir (fluent `set_formatter`; katalogun simgeleri biçimleme süresince iş parçacığı
   yerelinde). `NUMBER($r, style: "percent")` ECMA-402 gibi varsayılan **0 kesir basamağı**
   (0.29 → tr `%29`); yalnızca `minimumFractionDigits` verilirse en çok = en az, açık
   `maximumFractionDigits` geçerli. Yüzde dışı stilde kesir sınırı yok. Çoğul için ham sayı geçin (`count = 3`); tarih/saat her zaman önceden biçimlenmiş metin.
4. **Yön:** zincirin ilk dili RTL ise `Direction::Rtl` ve FSI/PDI yalıtımı açık; LTR'de kapalı
   (görünmez karakter yok). Yeni API Left/Right değil Start/End kullanır.
5. **Kullanıcı dizini** `$XDG_DATA_HOME/nuxut/locales/<domain>/<dil>.ftl` gömülünün üstüne yazar
   (anahtar başına) ve yeni dil ekleyebilir. Güvenilmeyen girdi: `O_NOFOLLOW|O_NONBLOCK|O_CLOEXEC`,
   `fstat` ile yalnızca düz dosya, ≤ 1 MiB (okuma da sınırlı), UTF-8, ≤ 10 000 mesaj+terim (aşan
   dosya tümüyle yok sayılır); hatalı girdiler atlanır + dosya başına tek uyarı (sayı; içerik/yol
   yok). Biçimlenmiş çıktı ≤ 4 KiB (grafem sınırında kırpılır); fluent placeable sınırı 100.
6. **Canlı değişim:** runtime açılışta `configure(&theme.locale)` (uygulama `init` ile aynı ayarı
   kurduysa iş yok), framework.toml değişince `[locale]` farklıysa `configure` → yeni katalog +
   `set_current` → `App::locale_changed`. Yalnızca dil değiştiyse `theme_changed` gelmez
   (`Theme::same_visuals`). Boşta iş yok.
   **Kullanıcı çeviri dizini izlenir** (`nuxut_i18n::watch`; runtime kayıtlı kaynak varsa açılışta
   kurar): `$XDG_DATA_HOME/nuxut/locales/<domain>/` inotify ile (yoklama yok, boşta sıfır CPU;
   test örneğinde geçici `XDG_DATA_HOME` aynı yoldan). Yalnızca kayıtlı domain dizinleri ve
   `*.ftl` adları ilgilidir; dizin yoksa en yakın var olan üst dizin izlenir (en çok 3 düzey:
   `locales`, `nuxut`, veri dizini) ve dizin oluşunca izleme aşağı iner, silinince yukarı çıkar.
   Symlink izlenmez (`IN_DONT_FOLLOW`, symlink dizine inilmez; veri dizininin kendisi hariç).
   Olaylar `Debounce` ile birleştirilir (son olaydan 200 ms sonra, sürekli akışta en geç 1 s;
   zamanlayıcı yalnızca bekleyen olay varken döngüde). Tetikte `ChangeTracker`: dosya meta
   özeti (ad, boy, mtime, ctime, inode) aynıysa dosya okunmaz; değiştiyse güvenli okuyucuyla
   içerik özeti — içerik aynıysa (aynı metnin yeniden kaydı) yeniden kurma yok. Değiştiyse
   `reload()` + `App::locale_changed` (tek kez). `run`'dan sonra kaydedilen domain de izlenir
   (2026-10-16; görev): runtime açılışta `set_register_hook` ile döngüye bir
   ping bağlar; yeni domain kaydı pingler, runtime izleyiciyi kurar (ilk kayıtsa) ya da
   `UserDirWatcher::set_domains` ile kümeyi günceller (yeni domain'in o anki dosyaları taban;
   eski domain'de bekleyen değişiklik korunur). Katalogu kayıttan sonra uygulama kurar
   (`reload`/`configure`; `register` kurmaz).
7. **`embed!` dil listesi derleme zamanıdır:** yeni `<dil>.ftl` eklemek mevcut dosyaları
   değiştirmediği için yeniden derlemeyi tetiklemez → makroyu çağıran dosyaya dokunun;
   `catalog_embedded` farkı yakalar.
8. **Denetim kapıları** ADR-0010 §4: ayrıştırma, tekrar, tr = en anahtar + öznitelik kümesi,
   değişken kümesi her dilde tr ile aynı, her mesajda çevirmen yorumu ve her `$değişken`i anar,
   `[max: N]` (tr'nin yorumundan; yoksa dilin kendisinden) tüm dillerde sabit metin ≤ N grafem
   (seçicide en uzun dal; değişken/referans 0), referanslar aynı dosyada çözülür ve döngüsüz,
   kimlikler kebab-case + `<önek>-`, terimler `-<önek>-`; kısmi dil kapsamı not.
   **Bekçi:** Türkçe harfli ya da `DEFAULT_UI_CALLS`'a verilen harfli dizge sabiti hata. Yapısal
   muaf (izin satırı istemez): yorum, öznitelikler (`#[error("..")]` dahil), `#[cfg(test)]`/`#[test]`
   öğeleri ve `#[cfg(test)] mod x;` dosyaları, `tests/`/`examples/`/`benches/`, `Debug` gövdeleri,
   hata/uyarı türünün (`..Error`/`..Warning` ya da dosyada `impl Error for`/`#[derive(.., Error)]`)
   `Display` gövdesi ve kurucuları (`XError::A(..)`, `Self::A(..)` kendi impl'inde), log/tanı/
   panic/assert makroları, `.expect(..)`, `t!`, `add_diagnostic_calls`; CLI ikilileri
   (`src/main.rs`, `src/bin/**`) ve proc-macro crate'lerinde Türkçe harfli sabit muaf (not).
   Hata olmayan türün `Display`'i denetlenir (`to_string()` ile arayüze gidebilir). UI kurucusunun
   argümanı tanı bölgesinde de bulgudur; orada yalnızca harfsiz ve makine biçimli dizge (yol/URL,
   küçük harfli snake/noktalı kimlik, `-symbolic`) muaf. `i18n-allow.txt`: `dosya:çağrı()  gerekçe`
   (`err()`, `Tür::ad()`, `alıcı.ad()`, makro `s!()`; satır kaymaz), `dosya:satır  gerekçe`; `dosya:*` eski biçim,
   desteklenir ama uyarı notu üretir (framework kullanmaz); `dizin/**  gerekçe` dizin ve alt
   dizinlerindeki tüm dosyalar için `dosya:*` ile aynı kurallarla (uyarı notu; framework
   kullanmaz). Öncelik `dosya:satır` > `dosya:*` > en dar `dizin/**`. Yollar köke göreli ve düz:
   `..`, mutlak yol, `.`/boş parça, ortada `**` ve başka glob karakteri bozuk satır hatası.
   Gerekçesiz, yinelenen ya da kullanılmayan satır hata; geçiş dönemi istisnası: taranan bir
   dosyayı kapsayan ama yapısal muafiyetle gereksiz kalan `dosya:*`/`dizin/**` satırı yalnızca
   not ("gereksiz izin … satırı silin"), kapsadığı taranan dosya yoksa hata.
   **Satır içi işaret** `// i18n: tanı <gerekçe>` (düz `//` yorumu): satır sonundaysa o satırda
   başlayan, tek başına satırdaysa sonraki deyimin (derinlik 0'daki ilk `;`/`,`, kapsayan
   parantezin kapanışı ya da yeni satırda `else`/`.`/`?` ile sürmeyen `{..}` bloğu) Türkçe harfli
   sabitlerini tanı metni sayar. Gerekçesiz işaret, `tanı` dışındaki `// i18n:` işareti ve hiçbir
   bulguyu bastırmayan ("gereksiz") işaret hata. UI kurucusu argümanını muaf tutmaz.
9. **Sözde dil** (`NUXUT_LANG=qps`): en mesajları aksanlı, `[…]` sarılı, %35 `·` dolgulu; yalnızca
   izole sway testlerinde.

### Ağ (nuxut-net)

Başka bir sürecin, sunucunun ya da kullanıcının verdiği adrese giden her HTTP isteği buradan
geçer (MPRIS kapağı, CalDAV/ICS, OAuth). Senkron API; çağıran kendi iş parçacığında çağırır
(UI thread'inde değil). Kaynak: calendar `nuxut-accounts/src/http.rs`.

```rust
pub struct HttpClient;            // Clone ucuz; kimlik taşımaz
impl HttpClient {
    pub fn new() -> Self; pub fn with_policy(Policy) -> Self; pub fn with_lookup(Policy, Arc<dyn Lookup>) -> Self;
    pub fn send(&self, Request) -> Result<Response, HttpError>;            // ilk adres Trust::User
    pub fn send_as(&self, Request, Trust) -> Result<Response, HttpError>;
    pub fn fetch_untrusted_media(&self, Url) -> Result<Response, HttpError>; // untrusted_media + Trust::Server
    pub fn vet(&self, &Url, Trust) -> Result<Vec<SocketAddr>, HttpError>;
}
pub struct Policy { pub allow_insecure_loopback: bool, pub https_only: bool }   // Default = en sıkı
pub enum Trust { User, Server }   // User: genel+LAN+loopback; Server: yalnızca genel internet
pub struct Request { pub method, pub url, pub body, pub max_body, pub timeout, pub follow_redirects,
                     pub max_redirects, pub https_only, .. }
impl Request { get, post_form, dav, header /* Authorization/Cookie reddedilir */, body, max_body,
               timeout, no_redirects, untrusted_media /* https, 8 MiB, 5 sn, 3 yönlendirme */ }
pub struct Response { pub status, pub headers, pub body, pub url }
pub struct OriginBoundClient;     // kimlik (Auth::{Basic, Bearer(Secret)}) yalnızca bağlı kökene
pub enum HttpError { BadUrl, CredentialsInUrl, HostNotFound, Timeout, Tls, Io(ErrorKind), TooLarge(u64),
                     TooManyRedirects, Downgrade, InsecureScheme, InsecureAuth, OriginMismatch(Url),
                     CrossOriginRedirect(Url), ForbiddenTarget(String), Protocol }
impl HttpError { pub fn target_url(&self) -> Option<&Url>; pub fn forbidden_host(&self) -> Option<&str>; }
pub fn classify(IpAddr) -> IpClass;   // Public | Private | Loopback | Never (gömülü IPv4 dahil)
```

Kurallar:
1. **Çöz → denetle → bağlan.** Her adımda (yönlendirmeler dahil) ad bir kez çözülür, TÜM
   adresler sınıflandırılır; tek yasak adres bütün hedefi reddeder. Bağlantı yalnızca denetlenmiş
   adrese yapılır (ureq çözümleyicisi sabitlenmiş listeyi verir; ikinci DNS sorgusu yok). Havuz
   ve ortam vekili kapalı (ikisi de sabitlemeyi atlatırdı). Başka kökene yönlendirme `Trust::Server`.
2. **Never** (her güvende): link-local/meta veri, belirtilmemiş, çok noktaya yayın, belgeleme,
   ayrılmış, 6to4 aktarıcı, site-local, discard, kıyaslama, Teredo, SRv6. **Private/Loopback**
   yalnızca `Trust::User` (kullanıcının girdiği adres).
3. **Kimlik:** yalnızca `OriginBoundClient`, yalnızca https (geliştirme bayrağıyla loopback
   http), köken dışına yönlendirmede gönderilmez (`CrossOriginRedirect`, karar çağıranın). URL'de
   `user:parola@` (ilk adres ya da yönlendirme) → `CredentialsInUrl`. Gövdeli istek (form,
   OAuth `client_secret`) başka kökene yönlendirilirse (307/308) izlenmez →
   `CrossOriginRedirect`; 303 ve POST'ta 301/302 gövdeyi düşürüp GET olarak izlenir.
4. **Sınırlar:** gövde AÇILMIŞ boyutuyla (gzip bombası), toplam süre yönlendirmeler boyunca,
   yönlendirme sayısı, `https → http` yok. Bilinen sınır: ad çözümü (`getaddrinfo`) süre
   sınırına dahil değildir (sistem çözümleyicisinin kendi zaman aşımı geçerlidir).
   **EINTR ve zaman aşımı** (2026-10-14; görev): okuma/yazmada
   `Interrupted` hata değildir — taşıma (TCP ile TLS arasında, `transport::RetryInterrupted`)
   kalan süreyle yeniden dener, süre biterse `Timeout`. Neden: `SO_RCVTIMEO`'lu soket çağrısı
   sinyal/durdurma/cgroup dondurmada `SA_RESTART`'a rağmen `EINTR` döner, ureq yinelemez.
   `WouldBlock` ve `TimedOut` (soket süresi dolması) → `HttpError::Timeout`.
5. **Güvenilmeyen medya:** `fetch_untrusted_media`. `Content-Type`'a güvenilmez; çağıran
   baytları görsel imzasıyla doğrular ve `nuxut-image`'e güvenilmeyen kaynak olarak verir.
6. **Loglar** (değişmez 9): `HttpError`, `Request`, `Response`, `Auth`, `Secret` Display/Debug'ı
   URL, ana makine, gövde ya da kimlik yazmaz (testli). UI'da hedef göstermek için
   `target_url`/`forbidden_host`.
7. Cephe (`nuxut`) yeniden dışa aktarmaz: ağ isteyen tüketici `nuxut-net`'e doğrudan bağlanır.
   `tls` özelliği varsayılan açık (rustls/ring); kapalıysa https isteği TLS hatası verir.
   Sertifika/el sıkışma hataları `HttpError::Tls`'tir (geçici değildir, yeniden denenmez).
8. **Test desteği** (`test-support` özelliği; üretimde yok): `testing::FakeServer::start`
   (http) ve `start_tls` (https, yalnız `tls` ile; sertifika gömülü yalnız-test CA'sıyla
   imzalı, SAN `localhost`/`nuxut.test`/`127.0.0.1`), `testing::TEST_CA_DER`, `OTHER_CA_DER`
   (yanlış CA), `testing::LoopbackLookup` (her ad → 127.0.0.1),
   `HttpClient::with_test_root_ca(policy, ca)` ve `with_test_tls(policy, ca, lookup)`. SSRF
   politikası adı çözdüğü için sahte sunucuya adla gitmek: `LoopbackLookup` +
   `server.url_at("nuxut.test", path)`; sunucunun verdiği adres (`Trust::Server`, ör.
   `fetch_untrusted_media`) loopback'e yalnız `Policy { allow_insecure_loopback: true, .. }`
   ile gider (örnek: `FakeServer::start_tls` belgesi).

### Portal (nuxut-portal) — dosya seçici

xdg-desktop-portal `org.freedesktop.portal.FileChooser` istemcisi (Ayarlar: VPN içe aktarma,
duvar kağıdı/avatar seçimi; ileride diğer Nuxut uygulamaları). **İsteğe bağlı:**
framework'ün geri kalanı D-Bus'a bağlanmaz; zbus yalnız bu crate'te, cephede kapalı varsayılan
`portal` özelliği (`nuxut = { features = ["portal"] }` → `nuxut::portal`). nuxut-wayland'e bağlı
değildir: sonuç uygulamanın verdiği alıcıya gelir, uygulama `cx.sender()` (calloop kanalı) ile
olay döngüsüne taşır. Seçim gerekçesi (doğrudan zbus, ashpd değil): DEPENDENCIES.md "Portal".
(2026-10-13; görev)

**Platform (ADR-0003 kural 2; 2026-10-13, görev).** API her
platformda aynıdır ve derlenir (`tools/cross-check.sh` listesinde; cephe `--features portal` ile).
zbus/async-io/futures-lite yalnız Linux hedefinde; Linux dışında `request_stub.rs` her isteği
yine istek thread'inden, tam bir kez `PortalError::Unavailable` ile bitirir (nuxut-video
deseni). Uygulama yolu elle yazdırma yedeğini zaten sunar; platforma göre `cfg` yazmaz. Önceki
hâl (Linux dışında boş crate, cephede `target_os` kapısı) tüketiciyi `cfg` yazmaya zorluyordu.
`file_uri_to_path` Unix dışında UTF-8'e kayıplı çevirir (portal yokken kullanılmaz).

```rust
pub struct Portal;                                   // uygulama başına bir tane; Send
impl Portal {
    /// Oturum bus'ı; bağlantı ilk istekte (tembel). `sink` istek thread'lerinden, sırayla çağrılır.
    pub fn new(sink: impl FnMut(PortalResponse) + Send + 'static) -> Self;
    /// Belirli bus adresi (testler: özel dbus-daemon).
    pub fn with_address(address: impl Into<String>, sink: impl FnMut(PortalResponse) + Send + 'static) -> Self;
    pub fn open_file(&self, parent: &ParentWindow, options: OpenFileOptions) -> PortalRequestId;
    pub fn save_file(&self, parent: &ParentWindow, options: SaveFileOptions) -> PortalRequestId;
    pub fn cancel(&self, id: PortalRequestId);      // Request.Close; sonuç Cancelled
    pub fn pending(&self) -> usize;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortalRequestId;  impl PortalRequestId { pub fn get(self) -> u64; }

pub struct ParentWindow;     // Debug yalnız tür
impl ParentWindow { pub fn none() -> Self; pub fn wayland(handle: &str) -> Self;
                    pub fn from_identifier(id: impl Into<String>) -> Self; pub fn as_str(&self) -> &str; }

pub enum FilterRule { Glob(String), Mime(String) }
pub struct Filter { pub name: String, pub rules: Vec<FilterRule> }
impl Filter { pub fn new(name) -> Self; pub fn glob(self, p) -> Self; pub fn mime(self, m) -> Self; }

#[derive(Clone, Default)]    // Debug yalnız bayrak/sayı
pub struct OpenFileOptions { pub title: String, pub accept_label: Option<String>, pub multiple: bool,
    pub directory: bool, pub filters: Vec<Filter>, pub current_filter: Option<usize>,
    pub current_folder: Option<PathBuf>, pub modal: bool }
impl OpenFileOptions { pub fn new(title) -> Self /* modal: true */ }
#[derive(Clone, Default)]
pub struct SaveFileOptions { pub title: String, pub accept_label: Option<String>,
    pub current_name: Option<String>, pub current_folder: Option<PathBuf>, pub filters: Vec<Filter>,
    pub current_filter: Option<usize>, pub modal: bool }
impl SaveFileOptions { pub fn new(title) -> Self /* modal: true */ }

pub struct PortalResponse { pub id: PortalRequestId, pub result: Result<Vec<PathBuf>, PortalError> } // Debug: yol sayısı
#[non_exhaustive]
pub enum PortalError { Cancelled, Unavailable, Denied, InvalidResponse, Dbus(String /* yalnız hata adı */) }
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf>;
```

Kullanım (Ayarlar):

```rust
// output_added / başlangıç:
let sender = cx.sender();
self.portal = Portal::new(move |r| { let _ = sender.send(Message::Picked(r)); });
// pencere configure olunca: cx.export_toplevel(window) → App::toplevel_exported → ParentWindow
let mut o = OpenFileOptions::new(t!("vpn-import-title"));
o.filters = vec![Filter::new(t!("vpn-filter")).glob("*.ovpn").glob("*.conf")];
let id = self.portal.open_file(&parent, o);
// App::message(Message::Picked(r)) → r.id == id ise r.result
```

Kurallar:
1. **Tam bir kez, çağrının içinden değil.** Her istek kısa ömürlü bir thread'de (`nuxut-portal`)
   yürür; sonuç alıcıya tam bir kez gelir. (Thread başlatılamazsa — istisna — `Unavailable`
   çağrının içinden.) `Portal` düşerken süren istekler iptal edilir (diyaloglar kapanır; sonuç
   yine `Cancelled` olarak alıcıya gelir).
2. **Akış.** `Response` aboneliği, `handle_token` (`nuxut_<kimlik>`) ile öngörülen istek yolu
   için çağrıdan ÖNCE kurulur; portal farklı yol dönerse (çok eski portal) o yola yeniden abone
   olunur. Çağrı yanıtı 30 sn'de gelmezse (etkinleştirme dahil) `Unavailable`. `Response` 0 →
   `uris`, 1 → `Cancelled`, 2 → `Denied`.
3. **Güvenlik.** `Response` yalnız çağrıyı yanıtlayan bağlantıdan (portalın benzersiz adı) kabul
   edilir; oturum bus'ındaki başka istemcinin gönderdiği sahte `Response` yok sayılır (testli).
   Yalnız `file://` URI (host boş ya da `localhost`), yüzde-kod çözülür, UTF-8 olmayan bayt
   korunur; başka şema/host, sorgu/parça, bozuk kaçış, NUL → atlanır; hiçbiri kalmazsa
   `InvalidResponse`. Document portal yolları (`/run/user/<uid>/doc/…`) geçerli yoldur.
4. **Hata eşlemesi.** Bus yok / `ServiceUnknown`, `NameHasNoOwner`, `Unknown{Method,Interface,
   Object}`, `NoReply`, `TimedOut`, `Spawn.*` → `Unavailable`; `AccessDenied`,
   `org.freedesktop.portal.Error.NotAllowed` → `Denied`; diğerleri `Dbus(hata adı)` (ileti metni
   kullanıcı içeriği taşıyabilir, alınmaz). `Dbus` sonrası paylaşılan bağlantı bırakılır, sonraki
   istek yeniden bağlanır.
5. **Seçenekler.** `directory` FileChooser sürüm 3+, `current_folder` sürüm 4+; eski portal yok
   sayar. `current_folder` NUL sonlu `ay`. `title`, `accept_label`, filtre adları kullanıcıya
   görünür: çevrilmiş (`t!`) verilir. `current_filter` geçersiz sıra ise gönderilmez.
6. **Ebeveyn.** Pencere: `cx.export_toplevel` → `ParentWindow::wayland(h.as_str())`. Layer-shell
   yüzeyi (shell panelleri) dışa aktarılamaz → `ParentWindow::none()`; diyalog ebeveynsiz açılır.
7. **Loglar** (değişmez 9): seçilen yollar, başlangıç klasörü/adı, ebeveyn tanıtıcısı yazılmaz;
   yalnız istek kimliği, sonuç türü, yol sayısı, hata adı. `Debug` çıktıları redakte (testli).
8. **Sınırlar.** `choices`, `current_file` (SaveFile), `SaveFiles` ve diğer portallar yok
   (gerektiğinde eklenir). Bağlantı süreç başına `Portal` örneği başına bir tane; zbus kendi
   yürütücü thread'ini açar (boşta 0 CPU).
9. **Test.** `tests/fake_portal.rs`: her test kendi özel `dbus-daemon`'unu başlatır (geçici dizin,
   servis etkinleştirme dizini yok, boş ortam) ve sahte `org.freedesktop.portal.Desktop` yazar:
   Response 0/1/2, `cancel` → `Request.Close`, `Portal` düşünce kapanış, portal yok, bus yok,
   sahte gönderen. Kullanıcının oturum bus'ına bağlanılmaz. Canlı deneme: izole sway + yalnız
   portal ve gtk arka ucunu etkinleştirebilen özel bus + geçici HOME/XDG dizinleri
   (`GSETTINGS_BACKEND=memory`, `GDK_DEBUG=no-portals`) ile `examples/file_chooser.rs`
   (`NUXUT_FILE_CHOOSER_AUTO=<sn>`): pencere dışa aktarıldı, GTK diyaloğu açıldı, otomatik iptal
   `Request.Close` ile kapattı.

### Palet (nuxut-palette)

Özellikler (varsayılan `["image", "sandbox"]`; **hiçbirinde JPEG XL yok**, `tools/cross-check.sh`
bekçisi): `image` = `nuxut-image` (render'sız): kaynak renk (`source_color*`, `PaletteError::Image`);
çözümü çağıranın `IsolatedDecoder`'ı yapar. `sandbox` = `image` + `nuxut-image-sandbox`: CLI
(`[[bin]] required-features = ["sandbox"]`) görseli `Sandbox::new(SandboxConfig::wallpaper())`
işçisinde çözer (işçi yoksa S1 kuralı). Yalnızca şema/rol/üretici isteyenler (shell)
`default-features = false`.

```rust
pub const SAMPLE_TARGET: TargetSize;     // 128×128 Contain (Cover kırpması kenar renklerini atardı)
pub fn sample_options() -> DecodeOptions; // Limits::WALLPAPER, SAMPLE_TARGET, ilk kare
// KIRICI: süreç içi `source_color_from_file(&Path)` kalktı.
pub fn source_color_from_file(path: &Path, decoder: &dyn IsolatedDecoder, key: u64) -> Result<Color, PaletteError>;
                                          // open_user_file + decoder.decode_file(key, …, sample_options()); key: file_key("wallpaper", path)
pub fn source_color_from_frame(&WallpaperFrame) -> Color;  // WallpaperDecoder karesinden (SAMPLE_TARGET'a süreç içi küçültme, kopyasız)
pub fn source_color(&Image) -> Color;     // büyük görseli önce SAMPLE_TARGET'a küçültür; Celebi + puanlama
pub enum Variant { TonalSpot, Vibrant, Expressive, Neutral, Fidelity, Content, Monochrome, Rainbow, FruitSalad }
pub enum Mode { Dark, Light }
impl Palette { pub fn generate(source: Color, Variant, Mode, contrast: f64) -> Self; // iki kip de
               pub fn generate_with(source, Variant, Mode, contrast: f64, ContrastMode) -> Self; } // + karşıtlık kipi (alan contrast_mode)
// Nuxut renk teması (docs/design/color-themes.md): iki kipin tam rol kümesi, source = tema tohumu, variant TonalSpot;
// temanın 11 token'ı THEME_ROLES rollerine yazılır (Nuxut ile dış uygulamalar aynı). Statik kaynakta dışa aktarım bundan.
impl Palette { pub fn from_color_theme(&nuxut_theme::ColorTheme, Mode, ContrastMode) -> Self; }
impl Scheme { pub fn from_color_theme(&nuxut_theme::ColorTheme, Mode, contrast: f64) -> Self; }
pub const THEME_ROLES: [(&str, &str); 11];   // Nuxut rolü → Material rolü (surface_bright → surface_container_high, outline → outline_variant)
pub fn export::generated_palette(&Palette, image) -> nuxut_theme::GeneratedPalette;  // palette.toml
pub fn export::export_templates(&Palette, image, out_dir, user_dir) -> Vec<Exported>;
impl Palette { pub fn scheme(&self, Mode) -> &Scheme; }                  // kipin tam rol kümesi
// Tam Material 3 rol kümesi (57 rol, kararlı sıra; yeni rol yalnızca sona):
pub struct Scheme { pub primary: Color, pub on_primary: Color, /* … *_container, secondary*, tertiary*,
    error*, background, surface, surface_dim/bright, surface_container_lowest…highest, surface_variant,
    inverse_*, outline(_variant), shadow, scrim, surface_tint, *_fixed(_dim), on_*_fixed(_variant),
    success, on_success, success_container, on_success_container, warning, … */ }
impl Scheme { fn generate(source, Variant, Mode, contrast) -> Self; fn get(&self, Role) -> Color;
              fn generate_with(source, Variant, Mode, contrast, ContrastMode) -> Self; // Standard ≡ generate
              fn high_contrast(&self) -> Self;  // accessibility.md §1 tablosu; yüzey/kap/gölge aynı, idempotent
              fn generate_with(source, Variant, Mode, contrast, ContrastMode) -> Self; // Standard ≡ generate
              fn high_contrast(&self) -> Self;  // accessibility.md §1 tablosu; yüzey/kap/gölge aynı, idempotent
              fn set(&mut self, Role, Color); fn iter(&self) -> impl Iterator<Item = (Role, Color)>;
              fn iter_snake / iter_camel (&self) -> impl Iterator<Item = (&'static str, Color)>;
              fn by_name(&self, &str) -> Option<Color>; }        // snake/camel/kebab
pub enum Role { Primary, OnPrimary, … }  impl Role { const ALL; fn snake(); fn camel(); fn from_name(&str) }
pub const SURFACE_ROLES: [Role; 10]; pub const ON_PAIRS: [(Role, Role); 12];  // yüksek karşıtlık çiftleri
pub use nuxut_theme::ContrastMode;
pub const SURFACE_ROLES: [Role; 10]; pub const ON_PAIRS: [(Role, Role); 12];  // yüksek karşıtlık çiftleri
pub use nuxut_theme::ContrastMode;
// success/warning: SUCCESS_SEED/WARNING_SEED kaynağa harmonize (hue ≤ 15° döner), MCU özel renk
// tonları (açık 40/100/90/10, koyu 80/20/30/90); kontrasttan etkilenmez.
pub fn look::gtk4_css(dark: &Scheme, light: &Scheme, active: Mode) -> String; // libadwaita CSS değişkenleri, iki kip
pub fn look::gtk3_css(&Scheme, Mode) -> String;                               // adw-gtk3 + klasik @define-color
// Metin ölçeği aktarımı (t124, docs/design/accessibility.md §3): saf hesap; yazımı shell look yapar.
pub struct text_scale::TextScale;  // from_size(px) / of(&Typography); factor() = round(size/13, 0.01) ∈ [0.5, 3.0]
    // gtk_text_scaling_factor() -> f64; gtk_xft_dpi() -> i32 /*round(96·1024·f)*/; qt_font_dpi(QtFonts) -> Option<u32>
pub enum text_scale::QtFonts { Written /*kdeglobals: ölçek pt boyuna katılır*/, NotWritten /*qt6ct: yalnız QT_FONT_DPI*/ }
pub fn text_scale::gtk(&Typography) -> GtkText;           // {text_scaling_factor, xft_dpi, ui_pt, document_pt, mono_pt}; pt REFERANS boyda
pub fn text_scale::qt(&Typography, QtFonts) -> QtText;    // {ui_pt, mono_pt /*ölçek katılmış*/, font_dpi}
pub fn text_scale::{gtk_font_pt(role_scale), qt_font_pt(size_px, role_scale), round_half_pt(pt)} -> f32;  // 0.5 pt adımı
```
CLI: `nuxut-palette generate <görsel>|--color #hex|nuxut:<id> --color #hex [--scheme S] [--mode dark|light] [--contrast C] [--high-contrast]`.
Çıktılar: `$XDG_STATE_HOME/nuxut/palette.toml` (`GeneratedPalette::save`) ve
`$XDG_STATE_HOME/nuxut/exports/{gtk3.css,gtk4.css,qt6ct-colors.conf,kdeglobals-colors,kitty.conf,foot.ini,hyprland.conf}`;
kullanıcı şablonları `~/.config/nuxut/templates/<ad>` → `exports/<ad>` (aynı adlı yerleşiği
geçersiz kılar). `gtk3.css`/`gtk4.css` şablon değil `look::gtk*_css` üreticisidir (libadwaita ≥ 1.6
`@define-color`'ı yok sayar; GTK 4 dosyası CSS değişkenleri + iki kip `@media` bloğu). Şablon motoru yalnızca `{{colors.<rol>.<default|dark|light>.<biçim>}}`,
`{{mode}}`, `{{scheme}}`, `{{source}}`, `{{image}}`, `{{image_path}}`, `{{contrast}}` değiştirir; bilinmeyen
değişken o şablonu reddeder (yarım dosya yazılmaz). Dosya adları düz ad olmalı; yazım atomik.
Kullanıcının uygulama config'lerine dokunulmaz (`@import`/`include`/`source` satırını kullanıcı ekler).

**Yerleşik duvar kağıtları** (sözleşme `wallpaper-builtins-v1`, 2026-10-13): gömülü SVG'ler,
dizin, `nuxut:<id>[/dark|light]` ayrıştırması ve tohumlar **shell deposundaki
`shell/crates/nuxut-wallpapers`** crate'indedir; framework adresi ayrıştırmaz, çözmez, yalnızca
saydam taşır. `export::is_builtin_address(&Path)` (`BUILTIN_SCHEME = "nuxut:"`): adres yol olarak
açılmaz, kanonikleştirilmez; `generated_palette`/`palette_toml`/`export_templates`'in `image`'ı
adresse `[meta] image`'a ve şablonun `{{image}}`'ına olduğu gibi gider. `{{image_path}}` yalnızca
gerçek dosya yoludur (adreste ve görsel yoksa boş): dış araca yol veren şablonlar onu kullanır.
CLI `nuxut-palette generate nuxut:<id>[/dark|light] --color #hex`: adres yalnızca `[meta] image`'a
yazılır, kaynak renk `--color`'dır (tohumu yerleşik crate'i verir); `--color`'sız adres hata.
Gerekçe: framework'te tohum tablosu tutmak shell crate'iyle ikinci bir kaynak olurdu; `--color`
ile palet her zaman açık bir kaynak renkle üretilir, sessiz varsayılan yok.

### Dış yapılandırma uygulama (nuxut-apply, shell ADR-0001)

Nuxut'un kendi dosyaları dışına (GTK/Qt ayarları, `hyprland.conf`, dconf, systemd `--user`)
ve kendi config göçlerine yazan her şey bu crate'ten geçer: tek atomik yazıcı, parça
düzenleyicileri, snapshot/geri yükleme, kilit, ortam kapısı. Ayarlar, shell ve göç yapan her
uygulama doğrudan kullanır (çok süreçli; `flock`). Cephe yeniden dışa aktarmaz.

```rust
// Ortam kapısı (SyncEnv::guard'ın genel hâli)
pub struct Env { pub home, pub real_home, pub config_home, pub state_home: Option<PathBuf>,
                 pub test_instance: bool, pub hyprland: Option<String>, pub disabled: bool }
impl Env {
    pub fn from_process(instance: Option<&str>) -> Self; // instance: çağıranın örnek adı (Ayarlar
                                       // NUXUT_SETTINGS_INSTANCE'ı verir); ortamdaki her boş olmayan
                                       // NUXUT_<X>_INSTANCE de test örneği yapar (fail-closed)
    pub fn for_test(root: &Path) -> Self;   // test örneği, root/{home,config,state}, Hyprland yok;
                                            // TESTLERDE from_process KULLANILMAZ
    pub fn guard(&self, TargetKind) -> Result<(), Skip>;
    pub fn may_control_hyprland(&self) -> bool; pub fn snapshot_dir(&self) -> Option<PathBuf>;
    pub fn display_path(&self, &Path) -> String /* ~/… */ }
pub enum TargetKind { File, Hyprland, Dconf, Unit }
pub enum Skip { TestInstance, NoSession, Disabled }

// Tek yazıcı
pub fn read_file(&Path) -> Result<(String, Expect), ApplyError>;      // ≤ MAX_FILE_BYTES (1 MiB)
pub fn read_bytes(&Path) -> Result<(Vec<u8>, Expect), ApplyError>;
pub fn read_optional(&Path) -> Result<Option<(String, Expect)>, ApplyError>;
pub fn write_file(&Env, &Path, &[u8], Option<&Expect>) -> Result<Written, ApplyError>;
// İzinli yazım: mode None = korunur/yeni 0644; dir_mode = eksik atalar (vars. DIR_MODE 0755);
// PRIVATE = 0600/0700. 0o777 dışı bit → Invalid. Var olan dizinin iznine dokunulmaz.
pub struct WriteOptions { pub mode: Option<u32>, pub dir_mode: u32 }  // Default, PRIVATE
pub fn write_file_with(&Env, &Path, &[u8], Option<&Expect>, &WriteOptions) -> Result<Written, ApplyError>;
// Yalnızca dizin (kapı + test kökü kuralı aynı): var → Unchanged (izin değişmez); dosya/kırık
// symlink → ExternallyManaged.
pub fn ensure_dir(&Env, &Path, mode: u32) -> Result<Written, ApplyError>;
pub fn remove_file(&Env, &Path, Option<&Expect>) -> Result<Written, ApplyError>;
// Yönetilen dosya başlığı: edit::managed::{header(CommentStyle, what), is_managed(&[u8]), HEADER_MARK};
// CommentStyle { Hash "# ", Block "/* … */", Slash "// ", DoubleDash "-- " (Lua) }.
pub enum Expect { Absent, Present { len, mtime_ns, ino, hash } }
pub enum Written { Written, Unchanged }

// Model
pub enum Fragment { TomlKey{file,key: KeyPath}, IniKey{file,section,key}, Line{file,marker,at: Position},
                    Block{file,id}, ManagedFile{file}, Dconf{key}, UnitState{unit},
                    FileForm{file} }   // FileForm: yolun biçimi, yalnızca devralma kaydı (istekte yok)
pub enum FragValue { Absent, Text(String), File { bytes: Vec<u8>, mode: u32 } }
pub struct Change { pub setting: SettingId, pub label: String, pub source: Source,
                    pub writes: Vec<(Fragment, FragValue)>,
                    pub create_missing: Vec<PathBuf>,   // eksikse yaratılabilir Line/Block dosyaları
                    pub take_over: Vec<PathBuf> }       // devralınacak dosyalar (değişmez 14)
impl Change { fn new(SettingId, label, Source) -> Self; fn write(self, Fragment, FragValue) -> Self;
              // write + parça Line/Block ise dosyası yoksa yaratılır (0644, eksik dizinler 0755;
              // ör. gtk.css yoksa yalnızca @import satırı). Diğer parçalarda write ile aynı.
              fn write_creating(self, Fragment, FragValue) -> Self;
              // Başka aracın yönettiği dosyayı devral (Nuxut varsayılanı; değişmez 14).
              fn take_over(self, impl Into<PathBuf>) -> Self; }
pub enum FileForm { Regular, Symlink(String), ReadOnly(u32), HardLink }  // to_text / parse
// Dışarıdan yönetilme nedeni (yalnızca stat; ExternallyManaged kararıyla aynı ölçüt ve sıra).
pub enum ExternalReason { SymlinkBroken, Store, NotOwned, ReadOnly, HardLink, DirNotWritable, Unknown }
impl ExternalReason { fn name(self) -> &'static str /* snake_case */; fn can_take_over(self, &Path) -> bool; }
pub fn external_reason(&Path) -> Option<ExternalReason>;
pub mod edit::gvariant { pub fn equivalent(&str, &str) -> bool }  // dconf değer eşitliği (değişmez 13)
pub enum Source { Settings, Shell, Welcome, Migration, Cli }
pub struct SettingId;   // [a-z0-9_.], 1..=64
pub struct SnapshotId;  // 20261010T120000Z-3fa1
pub enum TxState { Pending, Applied, RolledBack, Interrupted, Restored(SnapshotId) }

// Değer kuralları (Change::validate): 1..=MAX_FRAGMENTS (256) parça ve adres toplamı (yol +
// anahtar/bölüm/işaret/kimlik baytları, Fragment::address_bytes) ≤ MAX_ADDRESS_BYTES (64 KiB) —
// fragments_fit(); birleşik görünüm tek değişiklikte ~110 parça ("ayar başına tek snapshot":
// bölünemez). Text ≤ 64 KiB, Block dışında satır sonu/NUL yok; Line değeri == marker; UnitState
// "enabled"|"disabled"; ManagedFile yalnızca File (≤ 1 MiB, mode ⊆ 0o777, 0 = korunur/0644);
// ManagedFile'ın dosyasında başka parça olamaz. Dosya parçası yolu mutlak UTF-8, ≤ MAX_PATH_BYTES
// (4096), denetim karakteri yok; dconf anahtarı /[A-Za-z0-9/-_.]; birim adı .service|.timer|….
// INI anahtarı `[$` içeremez (kdeglobals bayrak eki düzenleyicinin işi: `key` → `key[$e]`).

// İşlem
impl Applier {
    pub fn open(Env) -> Self;                  // IO yok; depo hatası apply'da SnapshotUnavailable
    pub fn with_source(self, Source) -> Self;  // restore'un kaydı için (varsayılan Settings)
    pub fn with_backend(self, Box<dyn Backend>) -> Self;   // dconf/systemd (varsayılan SystemBackend)
    pub fn env(&self) -> &Env;
    pub fn plan(&self, &Change) -> Result<Plan, ApplyError>;   // yazmadan: parça, current set|absent, foreign
    pub fn apply(&self, Change) -> Outcome;
    // Türetilmiş yeniden üretim: ayarın son işlemi bu kaynağın uygulanmış işlemiyse ve araya
    // kimse girmemişse süre penceresi olmadan ona birleşir (yeni snapshot yok, ilk before korunur);
    // aksi hâlde apply gibi yeni snapshot.
    pub fn apply_derived(&self, Change) -> Outcome;
    pub fn restore(&self, &SnapshotId, RestoreMode) -> RestoreOutcome;
    // restore + gerçekten geri yazılan değerler (yalnızca Restored{Some}/Partial iken dolu;
    // çakışıp atlanan parça yok; ManagedFile içeriği yok).
    pub fn restore_with_values(&self, &SnapshotId, RestoreMode) -> (RestoreOutcome, RestoredValues);
    pub fn list(&self, &Filter) -> Result<Vec<SnapshotSummary>, ApplyError>;   // yeniden eskiye
    pub fn show(&self, &SnapshotId) -> Result<Option<SnapshotDetail>, ApplyError>;
    pub fn latest(&self, &SettingId) -> Result<Option<SnapshotSummary>, ApplyError>;
    pub fn original(&self, &SettingId) -> Result<Option<SnapshotSummary>, ApplyError>;
}
pub enum RestoreMode { Safe, Force }
pub struct RestoredValues { pub setting: Option<SettingId>, pub values: Vec<RestoredValue> }
pub struct RestoredValue { pub fragment: Fragment, pub value: Option<String> }  // None = kaldırıldı
impl RestoredValues { fn get(&self, &Fragment) -> Option<Option<&str>>; }  // None = yazılmadı
// RestoredValue(s): Serialize yok, Debug değer/yol göstermez (IPC JSON'una ve loga girmez).
// Outcome / RestoreOutcome / *Summary / SnapshotDetail / Plan: serde::Serialize (IPC JSON),
// adlar ADR §10.3: applied | noop | partial | skipped | externally_managed | changed | busy |
// too_large | unavailable | snapshot_unavailable | error;
// Outcome::ExternallyManaged { target: "~/…", reason: ExternalReason } (JSON reason snake_case).
// restore: restored | partial | conflict | not_found | busy | skipped | error.
// SnapshotSummary/SnapshotDetail: id, at (ISO 8601 UTC, saniye), created_ms (UNIX epoch ms),
// setting, label, source, state, restored_by?, restores?, pinned, fragments, conflicts.

pub const PER_SETTING_LIMIT: usize = 20; TOTAL_LIMIT = 500; TOTAL_BYTES_LIMIT = 64 MiB;
pub const COALESCE_MS: u64 = 2000; LOCK_WAIT = 5 s; PEEK_EXT_BUDGET = 2 s;

// Dosya dışı hedefler
pub trait Backend: Send + Sync {
    fn read(&self, &Fragment) -> Result<FragValue, ApplyError>;   // Absent = ayarlanmamış / birim yok
    fn write(&self, &Fragment, &FragValue) -> Result<(), ApplyError>;
}
pub struct SystemBackend;   // new(&Env) | with_programs(&Env, dconf, systemctl)
pub struct NoBackend;       // her şey Unavailable
// SystemBackend: `dconf read|write|reset <key> [<gvariant>]`, `systemctl --user
// is-enabled|enable|disable -- <unit>`; sabit argv, kabuk yok, stdin kapalı, stderr atılır,
// zaman aşımı (DCONF_TIMEOUT 2 s; SYSTEMCTL_TIMEOUT 5 s — ADR §5'in 2 s'sinden bilinçli sapma,
// onaylı: `enable`/`disable` örtük daemon-reload yapar, yavaş makinede 2 s'yi
// aşar ve yarıda öldürülen enable belirsiz durum bırakırdı) → çocuk
// öldürülür; çıktı ≤ 64 KiB. Program yok → Unavailable("dconf"|"systemctl"). is-enabled:
// enabled|enabled-runtime → "enabled", disabled → "disabled", not-found → Absent, diğerleri
// (static, masked, alias …) → ExternallyManaged (Nuxut geri yükleyemez, dokunmaz).
// Her çağrıda kendi Env::guard'ı: test örneğinde program başlatılmaz. Crate'in test derlemesinde
// varsayılan program yolları var olmayan bir yerdir (birim testleri gerçek oturuma ulaşamaz).
```

Shell/Ayarlar kullanım kalıbı:

```rust
let app = Applier::open(Env::from_process(instance.as_deref())).with_source(Source::Shell);
// Hyprland hedefi kuran çağıran kapıyı KENDİSİ sorar: Applier parçadan Hyprland türünü
// çıkaramaz (hyprland.conf parçası düz dosyadır, kind_of → File):
if app.env().guard(TargetKind::Hyprland).is_err() { return; }
let change = Change::new(SettingId::new("hypr.source")?, "Hyprland ortak ayarlar", Source::Shell)
    .write(Fragment::Line { file: hyprland_conf, marker: src.clone(), at: Position::End },
           FragValue::Text(src))
    .write(Fragment::ManagedFile { file: nuxut_conf },   // yalnızca yabancı içeriğe ilk dokunuşta
           FragValue::File { bytes, mode: 0 });           // (türetilmiş yeniden üretim: write_file)
match app.apply(change) { Outcome::Applied { id } => …, Outcome::Changed => /* yeniden oku */, … }
// "Önceki hale dön": app.latest(&setting)? → app.restore(&s.id, RestoreMode::Safe)
// "Nuxut öncesine dön": app.original(&setting)? → restore
```

Türetilmiş yeniden üretim (tema değişince `nuxut.conf`/`nuxut.css`) snapshot almaz: hiçbir işlemin
izlemediği, tamamı Nuxut'a ait dosyalar doğrudan `write_file` (aynı güvenceler); bir işlemin
izlediği parça (`kdeglobals` anahtarı, snapshot'lı yönetilen dosya) `apply_derived` ile — dışarıdan
yazılsa kaydın `after`'ı eskir, geri yükleme onu çakışma sayardı. Kendi config'lerinin normal düzenlemesi de yalnızca `write_file`;
göçler `Source::Migration` ile `apply`.

Değişmezler:
1. **Kapı önce.** `Env::guard` `Err` ise hiçbir dosya açılmaz, hiçbir süreç başlatılmaz. Test
   örneği (`NUXUT_*_INSTANCE` ailesi ya da çağıranın verdiği örnek adı) dosyalara yalnızca
   mutlak `XDG_CONFIG_HOME`/`XDG_STATE_HOME` altında yazar; bu köklerin kanonik biçimi
   kullanıcının **gerçek** ev dizinine (`getpwuid(geteuid())`, `$HOME` değil — taklit edilebilir)
   eşit, onun atası (`/`, `/home`) ya da içinde olamaz; gerçek ev bilinmiyorsa hiçbir şey. Hedef
   de kanonik denetlenir (symlink ile dışarı kaçış `Skipped(TestInstance)`). dconf/systemd'ye
   hiç yazmaz; test örneği `Env`'iyle kurulan `Applier` `NoBackend` kullanır (program yolu bile
   yok). Hyprland kapısı (`TargetKind::Hyprland`) çağıranın sorumluluğudur.
2. **Yazım (ADR §4):** symlink hedefine yazılır, link korunur; kırık symlink / nix-guix deposu /
   başka sahip / sahibine salt okunur / sabit bağ / yazılamayan dizin → `ExternallyManaged`.
   Geçici `.<ad>.nuxut-<pid>-<n>.tmp` (`create_new`, `0600`) → izin korunur → `fsync` → `rename`'den
   hemen önce (len, mtime_ns, ino, blake3) yeniden okunur, farklıysa `Changed` → `rename` → dizin
   `fsync`; hata yolunda geçici dosya silinir. Aynı içerik yazılmaz (`Unchanged`). Dosya yoksa
   yalnızca `Some(&Expect::Absent)` ile yaratılır (yeni dosya `0644`, dizinler `0755`).
3. **Parça dışı bayt korunur.** TOML (`toml_edit`), INI (kendi satır düzenleyicimiz; yorum, sıra,
   boş satır, CRLF, `[$e]` ekleri), Line, Block (`# >>> nuxut:<id>` / `# <<< nuxut:<id>`).
   Ekle → sil bayt bayt eski hâl. Eksik INI bölümü sona, önüne (dosya boş değilse) her zaman tam
   bir boş satırla eklenir; Nuxut'un yarattığı bölüm manifestte `section_created` ile işaretlenir
   ve anahtarları silinip gövdesinde yalnızca boş satır kalınca — dosyanın ortasında da — başlığı
   silinir; kullanıcının bölümü (önceden var olan, ya da içine kullanıcı anahtar/yorum eklemiş)
   asla silinmez. `section_created` alanı `false` iken yazılmaz, eksikse `false` (eski kayıtlar
   okunur). (İstisnalar modül belgelerinde: kullanıcının dosya sonundaki boş INI bölümü,
   boş yorumsuz TOML tablosu, satır sonusuz biten TOML'a tablo eklenince sondaki `\n`).
4. **Loglarda değer/yol yok.** `ApplyError`, `Fragment`, `FragValue`, `Change` `Debug`/`Display`'i
   yol ve değer göstermez (testli); snapshot logları yalnızca kimlik, ayar, parça türü, sonuç.
5. **Satır parçası değeri var/yok'tur** (`Text(marker)`); blok değeri satır sonuyla biter
   (`edit::block::normalize`). Eksik `Line`/`Block` dosyası (kullanıcının ana dosyası) yaratılmaz
   (`not_found`); yalnızca açıkça `Change::write_creating` ile (`create_missing`, yalnızca bu
   istekteki satır/blok dosyaları; aksi `Invalid`) yaratılır. Geri yükleme eksik dosyayı yaratmaz.
6. **Snapshot alınamıyorsa dış hedefe yazılmaz** (`SnapshotUnavailable`). Depo
   `$XDG_STATE_HOME/nuxut/snapshots/<id>/{manifest.toml ≤ 260 KiB, N.blob ≤ 1 MiB}` + `journal.log`
   (satır: kimlik, durum, ayar, kaynak, parça sayısı); dizinler `0700`, dosyalar `0600`. Bütün
   değerler blob'da; manifest yalnızca blake3 özeti taşır. Bozuk manifest atlanır (log: kimlik).
7. **İşlem:** kilit `$XDG_STATE_HOME/nuxut/apply.lock` (std `File::lock`, ≤ 5 s → `Busy`) →
   kilit altında görülen `Pending` = çökme → `Interrupted` (otomatik geri alınmaz, §11-4 A) →
   tüm içerikler ve kesin engeller (eksik ana dosya, çözümleme, dışarıdan yönetilme, yazılamayan
   dizin) **manifest yazılmadan önce** denetlenir — başarısızlığı belli işlem iz bırakmaz →
   manifest `Pending` → dosya başına tek yazım → hata: yazılanlar eski baytlarına (yalnızca hâlâ
   bizim yazdığımız gibiyse); dışarıda hiçbir şey değişmediyse snapshot dizini silinir (yalnızca
   günlükte `rolled_back`; tekrarlanan başarısız uygulama depoyu büyütmez), birleştirilen işlem
   önceki hâline döner; geri dönüş başarısızsa `Interrupted` + `Partial` → `after` gerçekte
   yazılandan yeniden okunur → `Applied`. Yazımdan sonra `Applied` kaydı güncellenemezse sonuç
   yine `Applied` (log; diskte `Pending` kalır → sonra `Interrupted`, geri alınabilir). Budama
   işlemden sonra; geri yüklemede eski kayıt `Restored` olarak kaydedildikten sonra. Aynı ayar + kaynak 2 s içinde ve araya dış değişiklik
   girmemişse son işlemle birleşir (ilk `before` korunur; başa dönen birleşik işlem silinir → `Noop`).
8. **Geri yükleme üç yollu:** şu an == `after` → `before` yazılır; şu an == `before` → atlanır;
   başka → çakışma (`Safe`: atlanır, `Partial`/`Conflict`; `Force`: yazılır). Geri yükleme yeni bir
   işlemdir (`restores = eski`), eski kayıt `Restored(yeni)` olur. Nuxut'un yarattığı (işlemin bu dosyadaki tüm
   parçaları `file_existed = false`) ve geri yüklemede boş kalan dosya silinir — `write_creating`
   ile yaratılan `gtk.css`'te yalnızca bizim satırımız kaldıysa dosya gider, kullanıcı kural
   eklediyse yalnızca satır; `ManagedFile`'ın yabancı içeriği geri konur.
   **Yaratıcı bilgisi dosya başınadır (devir).** Bir dosyayı birden çok işlem parça parça yazar
   (ör. `gtk-4.0/settings.ini`'nin dört `look.*` anahtarı; `kdeglobals`, `qt5ct/qt6ct.conf`,
   `~/.icons/default/index.theme`, `kcminputrc`): ilk yazan `file_existed = false`, sonrakiler
   dosyayı var bulur. Sonraki işlem, yazım anında (kilit altında) hâlâ uygulanmış
   (`Applied`/`Interrupted`) bir işlemin bu gerçek dosyadaki Nuxut-yaratımı parçası
   (`file_existed = false` ya da kendisi devralmış) yazdığı değerle dosyada duruyorsa
   (`is_ours`), kaydında `file_inherited = true` taşır. Böylece hangi sırayla (ileri, LIFO, tek
   tek `snapshot restore`) geri alınırsa alınsın dosyayı boşaltan geri yükleme onu siler; ortadaki
   tek geri yükleme boşaltmaz, diğer anahtarlar kalır. Seçim gerekçesi: "depoda o yolun en eski
   kaydı `file_existed = false` mı" kuralı süreklilik bilmez (Nuxut'un dosyası silinip kullanıcı
   kendi dosyasını yaratırsa eski kayıt yine "Nuxut yarattı" derdi, geri alınmış yaratıcıyı da
   sayardı); devir, kararı yazım anındaki canlı kanıta (uygulanmış yaratıcı + değeri dosyada)
   bağlar ve kayıt kendi kendine yeter. Kullanıcının (Nuxut öncesi) dosyası işaret almaz; boş da
   kalsa silinmez. "Boş" parça türüne göre: yalnızca `IniKey` → boş satır/boşluk ve bölüm
   başlıkları; yalnızca `TomlKey` → boş satır ve tablo başlıkları (`[[dizi]]` içerik); diğerleri →
   yalnızca boşluk. Yorum ya da kullanıcının anahtarı varsa dosya kalır. `file_inherited`
   yalnızca `true` iken yazılır, eksikse `false` (eski kayıtlar okunur, eski davranış);
   `file_existed = true` olmayan parçada `file_inherited` → kayıt bozuk. Nuxut'un yarattığı üst
   dizinler (ör. `gtk-4.0/`) silinmez: dizin yaratımı kayıtlı değil, boş dizin zararsız; kullanıcının
   dizinini silme riski alınmaz. Sınır: boş INI bölüm başlığının kaldırılması (`section_created`)
   depo genelinde bakılır, süreklilik denetlemez — kullanıcı Nuxut'un dosyasını silip aynı adlı boş
   bölümle yeniden yaratırsa o boş başlık kalkabilir (dosya kalır).
9. **Saklama (§11-5 A):** ayar başına son 20 + her parçaya ilk dokunan işlem sabitlenir (silinmez);
   toplam 500 işlem ve 64 MiB aşılınca en eski sabitlenmemiş silinir. Budama her `apply` sonunda,
   kilit altında. `.bak` dosyası bırakılmaz (§11-3 A). Manifest sınırı `store::MAX_MANIFEST_BYTES`
   = 4 KiB başlık + 256 parça × 512 B (`ENTRY_OVERHEAD`; ölçülen en kötü 282 B) + 2 × 64 KiB adres
   (`"`/`\` kaçışı) = 260 KiB; en kötü geçerli manifest (ölçülen 203 KiB) sığar, aşan okunmaz
   (testli). Gerçekçi 110 parçalık işlem ≈ 34 KiB. Disk etkisi parça sayısıyla değil kotayla
   sınırlı: manifest + blob'lar 64 MiB / 500 işlem toplamına dahil; `load_all` en çok 500 manifest
   okur (500 × 260 KiB ≈ 127 MiB yalnızca elle şişirilmiş depoda; kendi yazdığımız depo 64 MiB
   budamasıyla bunun altında kalır). Birleştirme birleşime bakar (`fragments_fit`, örtüşen parça
   bir kez sayılır); sığmıyorsa yeni işlem açılır.
10. **Depodan okunan veri güvenilmez.** Manifest: kimlik dizin adıyla aynı, sürüm, etiket, parça
    biçimleri (`Fragment::validate` + yolda `..` yok), değer türü ↔ parça türü, özet 64 hex, blob
    indeksi sınırlı, izin ⊆ `0o777`, yinelenen parça yok, kendini geri yüklemez — uymayan manifest
    bozuk sayılır (atlanır, `NotFound`). Blob: özeti tutmalı ve geri yüklenecek değer istek
    kurallarından (`value_fits`: satır sonu enjeksiyonu yok) geçmeli, yoksa `SnapshotUnavailable`.
    Depo dosyaları `O_NOFOLLOW | O_NONBLOCK` ile açılır ve düz dosya olmalı (symlink/FIFO/aygıt
    reddedilir, açılış takılmaz); snapshot dizini symlink olamaz; kilit ve `journal.log` symlink ise
    izlenmez. Hedef dosyalar da `O_NONBLOCK` ile okunur (FIFO hedef takılmaz). Test örneğinde
    geri yükleme hedefleri de kanonik kök denetiminden geçer.
11. **Çökme:** manifest yazılmadan dış hedefe dokunulmaz; manifest'siz snapshot dizini kilit
    altında silinir. Birleştirme sırasında çökülürse parça eski ya da yeni `after`'da kalır; ikisi
    de "bizim" sayılır (`prior_after`, yalnızca `Pending`'de), `Interrupted` kayıt güvenli kipte
    çakışmasız geri alınır.
12. **Listeleme sınırlı:** `list`/`show`/`latest`/`original` çakışma sayımı için hedefleri okur;
    önbellek çağrı boyunca paylaşılır (aynı dosya/dış hedef bir kez), dış hedef (alt süreç)
    okumaları toplam `PEEK_EXT_BUDGET` (2 s) ile sınırlı, sonrası "bilinmiyor" (çakışma sayılmaz).
    Depo dizini taramasında yalnızca en yeni 4096 kimlik tutulur.
13. **dconf karşılaştırması GVariant anlamıyla** (`edit::gvariant::equivalent`; 2026-10-16,
    shell look-external.md §1): `dconf read` GLib `g_variant_print` biçiminde döndürür (`1.00` →
    `1.0`, `1.10` → `1.1000000000000001`, `"a"` → `'a'`). `plan`/`apply` bu değerleri aynı sayar
    (sayı değeriyle, kesirli/tamsayı ayrı tür; dizge kaçışı çözülmüş içerikle; dizge dışı boşluk
    önemsiz) → yeniden yazım + sinyal yok (`Noop`). Saklanan `after` yine gerçekte okunan metindir.
14. **Devralma (`Change::take_over`, 2026-10-16; tasarım shell
    docs/design/look-external.md §4).** API isteğe bağlıdır (dosya başına); kullanıcı ilkesi
    (2026-10-16): Nuxut çağıranları devralmayı **varsayılan** yapar — onay beklemeden, yedekli
    (snapshot), geri alınabilir. `apply` kilit altında `external_reason(yol)`'a bakar: neden yok →
    etkisiz; `ExternalReason::can_take_over(yol)` → işleme `Fragment::FileForm { file }` girer
    (önce: özgün biçim `symlink:<readlink metni>` | `mode:<sekizli>` | `hardlink` | `foreign:<sekizli>`; sonra:
    `regular`), yol **son bileşeni izlenmeden** aynı dizinde geçici girdi + `rename` ile Nuxut'un
    düz dosyasına çevrilir (symlink → hedef içeriğinin kopyası, salt okunur → izin `| u+w`, sabit
    bağ → bağı koparan kopya; özgün inode `snapshots/<id>/keep-<yol özeti>` bağında tutulur) ve
    parçalar yazılır — tek işlem, tek snapshot. `rename`'den önce yolun `lstat` kimliği ve içerik
    özeti denetlenir (`Changed`). Geri yükleme üç yollu kurala uyar ve özgün biçimi geri koyar:
    symlink aynı metinle yeniden kurulur (hedef dosyaya hiç dokunulmaz), izin geri alınır (içerik
    bayt bayt), sabit bağ tutulan inode'a yeni bağla yeniden kurulur. Devralınabilir: son bileşeni
    symlink olan her yol (hedef kırık, salt okunur ya da başka kullanıcıya ait olsa da; ör.
    `gtk-4.0/gtk.css` → `/usr/share/themes/Dracula/…`, neden `not_owned`), salt okunur ve sabit
    bağlı düz dosya; başka kullanıcıya ait düz dosya da (2026-10-17), okunabiliyorsa ve ata dizin
    yazılabilir, yapışkan değil ya da bizimse (`sys::foreign_replaceable`; kök gerekmez, `rename`
    dizin iznidir) — geri yükleme içeriği ve izni geri koyar, sahip kullanıcı olur (kök olmadan
    sahiplik geri verilemez). Devralınamaz (`ExternallyManaged`): okunamayan ya da başkasının
    yapışkan dizinindeki yabancı dosya, `store`
    (nix/guix; bildirimsel yapılandırma), `dir_not_writable`. Biçim parçası istekte verilemez
    (`Invalid`); `take_over` yolu istekteki bir dosya parçasının yolu olmalı. Bilinen sınırlar:
    tutulan inode budanmış snapshot'la gider (ilk dokunuş sabitlendiği için pratikte budanmaz) ya
    da depo başka dosya sistemindeyse tutulamaz → geri yükleme yalnızca içeriği düz dosya olarak
    geri koyar (uyarı logu); devralınan symlink'in hedefi test örneğinde kök dışındaysa yalnızca
    okunur.
15. **Yaratılan dizinler (2026-10-16).** Dosya yaratan işlem, yolun yukarısında o an var olmayan
    her dizini manifestin `created_dirs` listesine (kanonik, mutlak; en çok 256) yazar; var olan
    bir ata, uygulanmış (`Applied`/`Interrupted`) başka bir işlemin kaydındaysa devralınır (aynı
    Nuxut dizinine birden çok ayar yazar, ör. `kitty/`). İlk kayıtsız var olan dizinde durulur:
    kullanıcının dizini asla kaydedilmez. Geri yükleme, dosyaları geri koyduktan sonra eski
    işlemin dizinlerini en derinden başlayarak **yalnızca boşsa** ve hâlâ (symlink olmayan)
    dizinse siler (`remove_dir`; ortam kapısı uygulanır). Hangi sırayla geri alınırsa alınsın son
    boşaltan siler; içine kullanıcı dosyası konmuş dizin kalır. Depodan okunan liste doğrulanır
    (mutlak, `..`/`.` yok, boy sınırı; bozuksa kayıt yok sayılır). Eski kayıtlarda alan yok.

## nuxut-ui bileşenleri

Wayland bilmez; `Painter`, `TextSystem`, `Theme` ve `nuxut_core::input` kullanır. Bileşenler
sabit renk/yarıçap/süre içermez; hareket `Animated<T>` + tema hareketidir (`theme.motion.*`,
stil — yay ya da eğri — temanın işi). Galeri: `cargo run -p nuxut-ui --example gallery`.

### Ortak kalıp

```rust
pub struct Ui<'p, 'a> { pub painter: &'p mut Painter<'a>, pub theme: &'p Theme, pub now: Instant, .. }
impl Ui<'_, '_> {
    pub fn new(painter, theme, now) -> Self;
    pub fn with_damage_tracking(self, on: bool) -> Self; // kapalı (varsayılan): hasar bildirilmez → tüm yüzey
    pub fn damage(&mut self, r: Rect);                  // takip açıksa painter.damage
    pub fn request_frame(&mut self);  pub fn wants_frame(&self) -> bool;   // karenin sonunda cx.request_frame
    pub fn wake_at(&mut self, at: Instant);  pub fn wake_deadline(&self) -> Option<Instant>;
        // "o ana kadar durağanım" (ipucu gecikmesi): en erken an kalır; uygulama karenin sonunda
        // tek zamanlayıcı kurar (cx.set_timer(at − now), öncekini iptal), zamanlayıcıda cx.request_frame.
        // ZORUNLU (Ui ile çizen her yüzey, her kod yolu): wake_deadline'ı tüketmeyen uygulamada zamanlı
        // bileşenler donar (bir sonraki olaya kadar). Kullananlar: Tooltip (gecikme), Slider (değer
        // balonunun sönme anı), CurveEditor (önizleme döngüsü sonundaki bekleme).
        // Kural: zamanlı bekleme (görünür ama durağan) için sürekli kare istenmez; request_frame yalnızca
        // görüntü gerçekten değişirken.
    pub fn with_alpha<R>(&mut self, a: f32, f: impl FnOnce(&mut Self) -> R) -> R;  pub fn fade(&self, c: Color) -> Color;
    pub fn surface_rect(&self) -> Rect;                 // yüzeyin tamamı: kaplamaların sığdırma alanı
    pub fn overlay<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R;
        // f'in çizimleri kaplama katmanına (Painter::begin_overlay): kare sonunda en üstte, kapsayıcı
        // kırpmasından (ScrollView, PageStack, kart) bağımsız. alpha korunur. Kaplamalar kendi aralarında
        // çizim sırasıyla üst üste (ipucunu en son çiz). Koordinatlar zaten yüzey koordinatı
        // (ScrollView::content_origin ofseti dahil) — çeviri gerekmez.
    pub fn overlay_input(&mut self, r: Rect);  pub fn overlay_regions(&self) -> &OverlayRegions;
    pub fn take_overlay_regions(&mut self) -> OverlayRegions;
        // Kaplamanın girdi bölgesi (açık menü/popover). Uygulama kare sonunda take_overlay_regions ile
        // yüzey başına saklar; işaretçi olayında regions.contains(p) ise olayı ÖNCE kaplamanın sahibine
        // verir (altındaki satır/kaydırma alanı görmez). Açık menünün "dışarı basınca kapan"ı ayrıca
        // is_open() iken tüm olayları sahibine vermeyi gerektirir (değişmedi).
}
#[must_use] pub struct Response<T = ()> { pub repaint: bool, pub event: Option<T> }
pub struct DamageState;   // bileşen içi: mark() durum değişince, report(ui, extent, animating) çizimde (eski ∪ yeni)
pub struct OverlayRegions;  // push(rect), contains(p), is_empty(), rects()
```

Her bileşen: `new(theme, …)` → `preferred_size(…)` → `set_rect(…)` → `pointer(&PointerEvent, now)` /
`key(&KeyEvent, now)` → `Response<_>` → `draw(&mut Ui)` → `set_theme(theme, now)`. Canlanan bileşen
`ui.request_frame()` der; hiçbir şey canlanmıyorsa kare istenmez. Metin ölçen girdiler
(`Menu`, `TextInput`, `ColorPicker`) `&mut TextSystem` alır (olay işleyicisinde `cx.text()`).
`set_theme` ise `TextSystem` istemez (tema değişimi `TextSystem`'e erişimi olmayan yerden gelebilir):
ölçüme bağlı yerleşim kirli işaretlenir ve bir sonraki `set_rect`/`draw`/girdide tembelce yenilenir
(`Select`, `Disclosure`). İstisna: `Menu::set_theme(text, theme, now)` (açık sayfaları hemen yeniden
ölçer; menüyü kendi sahiplenen bileşen `Select` gibi tembel yolu iç yardımcıyla kullanır).

İmleç: etkileşimli her bileşen `cursor(&self, p: Point) -> Option<CursorShape>` verir (`None` = bu
noktada görüş yok; durum duyarlı: sürüklerken `Grabbing`, devre dışı `Default`). Kök, en üstteki
isabetten başlayarak `CursorShape::pick([..])` ile seçer ve her motion'da `cx.set_cursor(surface, shape)`
der. Kaplar (Menu, Popover, Select, List) içerideki boş alan için `Default`, dışarısı için `None`
verir; Popover içeriğin imleci önce sorulur. Bileşen bölgeleri son `set_rect`/çizimden gelir.
`TextInput::caret()` metin imleci/seçim çapasıdır (önceki adı `cursor()`).

Ortak sabitler (`nuxut_ui::style`): `DISABLED_CONTENT = 0.38`, `DISABLED_CONTAINER = 0.12`,
`TONAL_MIX`, `HAIRLINE`, `FOCUS_RING`, `DIVIDER_TONE`; `tonal_container(theme)`, `row_height(theme)` (standart satır en az yüksekliği, `space(16)`), `content_on(theme, bg)`,
`divider(theme)` (ayırıcı/kılavuz: `theme.outline()` ya da `on_surface` %7), `field_fill(theme)` (dolgulu alan), `Hsv`,
`clip_text(s, max)` (dış metin bayt sınırı).

Bileşen karışımları ve yüksek karşıtlık (`nuxut_ui::style`, accessibility.md §1 "Bileşen
karışımları", 2026-10-13,): token'ları karıştıran HER bileşen rengi bu
yardımcılardan geçer; standart kipte eski ifadeyi aynen verirler (çıktı değişmez), yüksek kipte
karışımı eşiği tutturacak en az değişimle ayarlarlar. Zemin = gerçek birleşim: alfalı dolgular üç
tema yüzeyine sırayla bindirilir (`backdrops(theme, fills)`), ölçüm 8 bit nicelenmiş.
`legible(theme, fg, bgs, min)` (önce opaklık, yetmezse siyah/beyaza en az karışım),
`overlay(theme, layer, bgs, contents, min)` (katman opaklığını içerik eşikte kalacak kadar azaltır),
`state_layer(theme, content, fills, opacity)` / `state_layer_under(theme, layer, opacity, fills, contents)`
(hover/basılı; içerik ≥ 4.5), `faded_text` (≥ 4.5: ay dışı gün, geçmiş etkinlik, kullanılamayan
cihaz), `faded_mark` (≥ 3: etkinlik noktası, şerit, kaydırma göstergesi), `disabled_content(theme, color, fills)`
(%38; yüksekte ≥ 3 — WCAG muaf, okunur kalsın kararı), `disabled_container`, `tinted(theme, base, tint, t, contents)`
(tonal kap, etkinlik zemini; üstteki metin ≥ 4.5), `accent_on(theme, bg)` (`primary` palet dışı zeminde ≥ 4.5),
`track(theme)` (değer göstergesi izi; yüksekte HCT tonu yüzeylere ≥ 3), `focus_ring(theme, adjacent)`
(yüzeyler + bitişik dolgulara ≥ 3), `hairline` (bileşen içi ayraç ≥ 3), `content_on` (yüksekte ≥ 4.5),
`surfaces`, `worst_over`. `Interaction::draw_state_layer(ui, rect, radii, content, container)` /
`draw_state_layer_under(…, contents)` framework bileşenlerinin katman çizimidir; `draw_layer` (tema
alır) da sınırlıdır; `draw_layer_with` ham kalır (geriye uyum; shell bileşenleri `draw_state_layer`'a
geçmeli). `Pressable::draw_focus_on(ui, rect, radii, adjacent)`: halka bileşen içine çizildiğinde.
`Tint::Color` (özel gösterge rengi) yüksekte yüzeylere ≥ 3. Kanıt: `nuxut-ui/src/contrast_tests.rs`.
Tipografi (ADR-0006): `TextRole::{Body, Caption, Label,
Title, Display, Document, Mono, Icon, IconSmall, IconLarge}`, `text_style(theme, role)`. Rol → font
rolü: Body/Caption/Label/Title → `ui`, Display → `display`, Document → `document`, Mono → `mono`,
Icon* → `typography.icon_family()`. Boy = `size × 1.125^adım × rol.scale`; her rolün `fallback` genel
ailesi ayarlıdır. Bileşenler fontu yalnızca buradan alır; kodda aile adı yok
(`crates/nuxut/tests/it/no_hardcoded_fonts.rs`: `src`/`examples`'ta `TextStyle::new("…")` ve bilinen
font adları yasak; izin: `nuxut-theme` varsayılanları, `#[cfg(test)]`).

### Bileşenler (imzalar kısaltılmış)

```rust
// İkon (ADR-0006): semantik ad → seçili ikon seti (material: ligatür + FILL ekseni; nerd: nf-md-*
// kod noktası, dolgu = çizgi/dolu glif çifti arası opaklık geçişi) ya da doku (renkli; sembolik =
// içerik rengiyle boyanır). Ham ligatür/kod noktası API'si yok.
pub enum Icon { Named(Option<IconName>), Image { texture: Texture, symbolic: bool }, Glyph(String) }  // Icon::named(n) / Icon::image(t) / Icon::symbolic(t) / Icon::glyph(&str); From<IconName>
// Icon::Glyph (2026-10-13, istek — emoji seçici): ikon kutusunda metin glifi (emoji, harf, sembol); arayüz fontu +
//   sistem yedeği (renkli emoji kendi renkleriyle), OpticalCenter::Ink ile kutu ortasında, kutudan genişse sığacak kadar küçülür;
//   kutu (IconView::size) rolün boyu, değişmez. En fazla MAX_GLYPH_BYTES (64) bayt (Icon::glyph kırpar); boş dize boş çizilir.
//   Her ikon yuvasında çalışır: ListItem::with_leading(theme, Icon::glyph("😀")), ResultItem::glyph("😀") (= .icon(Icon::glyph(..))).
//   Yuva davranışı değişmedi: ListItem ikonsuzsa yuva yok; ResultList ikon sütununu hep ayırır (sütun hizası).
Icon::symbol(&str)    // geçiş kısa yolu: semantik ya da eski Material ligatür adı; bilinmeyen = None, boş çizilir
pub enum IconName { Wifi, WifiOff, Bluetooth, VolumeHigh, VolumeMute, Battery50, Notifications, Close, Cpu, Memory, Gpu /*ekran kartı*/, Disk, … }
impl IconName { pub fn as_str(self) -> &'static str; pub const ALL: &[IconName];      // snake_case: "volume_high"
                pub fn battery(percent: u8) -> Self; pub fn wifi(strength: u8) -> Self; pub fn volume(percent: u8, muted: bool) -> Self; }
impl FromStr for IconName { type Err = UnknownIcon; … }  // config/IPC; eski ligatür adları da (volume_up → VolumeHigh)
pub fn glyph(set: IconSet, name: IconName, filled: bool) -> &'static str;   // nuxut-ui/src/icon_names.rs, tek tablo
// IconName::Emoji ("emoji"; material `mood`, nerd nf-md-emoticon_outline / nf-md-emoticon) — emoji seçici kipi (2026-10-13, istek)
// Ad listesi ve eşleme yalnızca icon_names.rs'tedir; her ad iki sette de vardır (kapsamlı match + test).
// Uygulama logosu glifi (nuxut-ui/src/app_glyph.rs): sınıf (Wayland app_id / Hyprland class) → Nerd Font
// glifi. Tablo VERİDİR: nuxut-ui/src/app_glyphs.txt (gömülü; "<hex> <nf-adı> <sınıf>..." ve
// "prefix <hex> <nf-adı> <önek>"), ilk kullanımda bir kez ayrıştırılır, karma tablo. Eşleme: tam ad →
// noktalı adın parçaları sondan başa (-bin/-git/-wayland/… ekleri atılmış) → önekler; harf duyarsız;
// > MAX_APP_CLASS (256) bayt eşlenmez.
pub fn app_glyph(class: &str) -> Option<char>;                    // = AppGlyphs::builtin().glyph(class)
AppGlyphs::builtin() -> &'static AppGlyphs; AppGlyphs::parse(&str) -> Result<_, AppGlyphParseError { line, reason }>;
    insert(class, char) /*kullanıcı eşlemesi; tam ad önekten önce*/; glyph(class) -> Option<char>; len()
// Nerd Font: TextSystem'in font veritabanı (Linux'ta fontconfig dizinleri/ayarı) üzerinden; NERD_SYMBOLS_FAMILY
// ("Symbols Nerd Font") → preferred (Nerd Font adlıysa, ör. theme.typography.mono.family) → kurulu Nerd
// Font'lardan en uygunu ("… Nerd Font" > "Propo" > "Mono" > "NF"). Yoksa None: glif yerine yedek (numara, baş harf).
pub fn find_nerd_font(text: &mut TextSystem, preferred: &str) -> Option<String>;  pub fn is_nerd_font_family(&str) -> bool;
NerdFont::new(); resolve(text, preferred) -> Option<&str> /*bir kez çözer; preferred değişince yeniden*/; is_resolved(); reset()
IconView::new(theme, Icon) / with_role(theme, Icon, TextRole) / filled(bool); set_filled(bool, now); animate(now) -> bool; is_pending();
    size(); measure(text); draw_centered(painter, center, color)

// Düğmeler — basınca köşeler hap → radius_small yayla sıkışır.
pub enum ButtonVariant { Filled, Tonal, Outlined, Text }
Button::new(theme, variant).with_label(theme, s).with_icon(theme, Icon); set_label(&str); set_variant(v);
    with_icon_role(theme, TextRole::{IconSmall /*vars.*/, Icon, IconLarge}); set_icon_role(theme, role) -> bool /*değişti → yerleşim*/;
    icon_role() -> Option<TextRole>   // büyük ikonda ikon–metin boşluğu space(2) → space(3); yükseklik yine space(10), büyük kareyi çağıran verir
    set_enabled(bool, now); set_focus_visible(bool); set_visual_state(hover, pressed, now); pointer/key -> Response
pub enum IconButtonVariant { Standard, Filled, Tonal }
IconButton::new(theme, Icon, v).toggleable(selected); set_selected(bool, now); set_icon(Icon); pointer/key -> Response<bool>
// Yerinde onay — geri alınamayan eylem için iki adım, soru metni/diyalog yok. Stil, yerleşim, odak halkası, imleç ve
// basılma hissi Button'ın kendisi (içerik takas edilir: konum/hover/odak/etkinlik düğmede kalır).
ConfirmButton::new(theme, idle_variant, Icon, armed_label).with_label(theme, s /*boşta; yoksa yalnız ikon*/)
    .with_armed_icon(theme, Icon /*vars. aynı*/).with_armed_variant(v /*vars. Destructive → error token*/)
    .with_window(Duration /*vars. theme.timing.confirm_window = 4 s*/)
    .with_icon_role(theme, TextRole /*vars. IconSmall; iki durum için; with_armed_icon'dan önce ya da sonra*/);
    // Büyük ikon (Icon/IconLarge): etiketsiz boş hâl IconButton görünümü; geri sayım halkası ikonu sarar (yarıçap ikon/2 +
    // space(1), kalınlık COUNTDOWN_RING × ikon/IconSmall oranı) ve düğme yüksekliğine sığacak şekilde sınırlanır. Oturum
    // kartı gibi büyük kare: set_rect ile daha büyük dikdörtgen ver (ör. space(16)); içerik ortalanır.
    set_label(&str); set_armed_label(&str); set_theme(theme, now); set_enabled(bool, now) /*false → bekleyen düşer*/;
    set_focus_visible(bool); preferred_size(text, theme) /*geçerli durum*/; max_size(text, theme) /*iki durumun büyüğü: sabit yerleşim*/;
    set_rect(r); rect(); cursor(p); is_armed(now); deadline() -> Option<Instant>; remaining(now) -> f32 /*1→0*/;
    cancel(now) -> bool /*programatik vazgeç: kapsayıcı kapandı*/; expire(now) -> bool /*süre dolduysa boşa döner; true → yerleşim yenile*/;
    pointer(ev, now) / key(ev, now) -> Response<ConfirmEvent::{Armed, Confirmed, Cancelled}>; draw(ui)
    // İlk basış (tık, Enter/Space) → Armed: onay içeriği + ikonun çevresinde tepeden saat yönünde geri sayım halkası (içerik
    // rengi; iz COUNTDOWN_TRACK opaklığı, kalınlık COUNTDOWN_RING). Süre içinde ikinci basış → Confirmed. Kurulduktan sonraki
    // theme.timing.confirm_guard (400 ms) içindeki basış yok sayılır (yanlışlıkla çift tık / tuş tekrarı onaylamaz).
    // Vazgeç: süre dolması (expire/draw; girdi sırasında fark edilirse Cancelled), Escape (beklerken tüketilir → Cancelled;
    // beklemiyorken tüketilmez), düğme dışına basış (Cancelled; basışın alttaki öğeye gidip gitmeyeceği çağıranın kararı —
    // beklerken yüzeydeki tüm basışlar bileşene de verilir), cancel(now) (panel kapandı), set_enabled(false).
    // Hareket: halka motion.effects yayıyla belirir/çekilir (kesilebilir). Halka sürerken her kare (hasar = halka kutusu),
    // hareket kapalıyken saniyelik adımlarla azalır (4/4 … 1/4) ve yalnızca ui.wake_at(sonraki adım); süre sonu ui.wake_at;
    // boşta kare/uyanma yok. Çağıran Ui::wake_deadline'ı tüketmek ZORUNDA.
Toggle::new(theme, on); set_on(bool, now) -> bool; pointer/key -> Response<bool /*yeni durum*/>
    // Durumlar rayın hap biçimi içinde: hover/basılı state layer ray dolgusuna karışır (içerik renginden), topuz hover'da
    // fark edilir, basılıyken çok büyür (rayda kalır); klavye odağı: topuz tam boy + ortasında nokta (primary; topuza karşı
    // 3:1 tutmazsa en kontrastlı token). Outline, taşma ve dış hasar yok (= rect).
pub enum Orientation { Horizontal, Vertical }
Slider::new(theme, Orientation, v0..=1).with_steps(n); set_value(v, now) /*sürüklerken yok sayılır*/; pointer/key -> Response<f32>; set_visual_state(hover, basılı, now) /*galeri*/
    // Tonal hap iz (space(5)) + aynı hapta primary dolgu + dolgu ucunda izin İÇİNDE on_primary topuz (OSD ile ortak hap çizimi:
    // iç modül bar::PillBar). Topuz merkezi [r, len−r]'ye eşlenir (uçlarda kavsin içinde; 0'da dolgu = topuz yuvası), tıklama aynı
    // eşlemenin tersi. Hover: topuz büyür + ton; sürükleme/klavye odağı: topuz iz kalınlığında, odakta ortasında primary nokta.
    // Değer balonu sürüklerken ve klavye/tekerlekle değişimde kısa süre (1,2 sn; sürükleme sonrası 0,5 sn) topuzun yanında, rect
    // içinde; görünür beklerken kare istemez, sönme anını ui.wake_at ile bildirir (sönme animasyonu normal kare). Hasar = rect; tıklama
    // alanı space(11). Kontrast: topuz/dolgu ≥ 3:1 (hover/basılı dahil; üretilmiş paletlerde testli); devre dışı bütünüyle soluk
    // (topuz dolgunun bir ton üstü; WCAG 1.4.11 pasif bileşeni muaf tutar).
Separator::horizontal(inset) / vertical(inset); draw(ui, rect)
Badge::new(theme, Option<u32> /*None = nokta*/); set_count(.., now); set_anchor(Point /*ikonun sağ-üstü*/)   // BADGE_MAX = 99
// Metinli rozet (talep, 2026-10-13): "Yeni", "Beta". Zemin tonal_container, metin
// accent_on(zemin) Caption, tam yuvarlak; iç boşluk yatay space(2), dikey toplam space(1); en az kare. Etkileşimsiz,
// canlanmaz (boşta kare yok). Seçili (tonal) satırda zemin surface (aynı zeminde kaybolmasın). Yüksek karşıtlıkta
// metin zemine ≥ 4.5 (contrast_tests, iki zemin).
Pill::new(theme, text /*MAX_PILL_TEXT = 64 bayt*/).with_max_width(Option<f32> /*metin; taşan …*/); set_text(&str) -> bool;
    size(text, theme) -> Size; place_inline(text, theme, x /*sol kenar*/, center_y) -> Rect;
    place_corner(text, theme, Point /*öğenin sağ-üstü; hap space(1) içeri biner*/) -> Rect; set_rect / rect;
    set_on_tonal(bool); set_theme(theme, now); draw(&mut Ui)
pub fn pill_colors(&Theme, on_tonal: bool) -> (Color /*zemin*/, Color /*metin*/)
Avatar::new(theme, name, AvatarSize::{Small /*32*/,Medium /*40*/,Large /*64*/,XLarge /*96 = space(24); profil kartı, 2026-10-13 istek*/}).with_image(Option<Texture>); set_image(..); set_presence(bool, now); set_center(p);
    initials(name)   // görsel daireye kırpılır; doku yüklenene kadar baş harfler. Baş harf: Caption/Label/Title; XLarge = Title × (96/64), daireye oranı Large ile aynı
// Marka işareti (Kara Sağan; `brand` özelliği, varsayılan açık). SVG'ler derlemeye gömülü (nuxut-ui/assets/brand/,
// geometri değiştirilmez): BRAND_SVG (renkli karolu ikon), BRAND_SYMBOLIC_SVG (tek path, currentColor).
// Yalnızca ALFA MASKESİ rasterleşir (nuxut_image::render_svg); renk çizimde ImageStyle::tint → tema/kip değişiminde
// yeniden raster yok. Kenar = round(boyut × ölçek), dikdörtgen fiziksel piksele hizalı (kesirli ölçekte net).
// CPU maskeleri süreç genelinde (parça, fiziksel kenar) anahtarlı LRU'da (BRAND_MASK_CACHE = 16, Arc paylaşımlı;
// MAX_BRAND_MASK_SIDE = 2048), GPU dokuları bileşen başına. Etkileşimsiz, canlanmaz: boşta kare yok.
BrandMark::new()   // Mono(on_surface), DEFAULT_BRAND_SIZE = 48
    .with_role(ColorRole) | .accent() /*Mono(primary)*/ | .duotone() | .tile() /*Tile(surface)*/ | .tile_with(ColorRole)
    .with_size(f32) / .with_style(BrandStyle);  set_style / set_size / set_rect(rect /*ortasına kare*/) / set_theme(theme, now)
    preferred_size() -> Size; device_rect(scale) -> (Rect, u32 /*fiziksel kenar*/); draw(&mut Ui)
    is_pending() -> bool   // son draw'da GPU'ya inmemiş doku vardı (draw zaten request_frame çağırır; kendi kare kararını veren sahipler için)
pub enum BrandStyle { Mono(ColorRole), Duotone, Tile(ColorRole) }
    // Duotone: siluet brand_shade(primary), üstüne ön kanat primary (ortak kenarda dikiş yok).
    // Tile: rol renginde karo (yarıçap = SVG rx/kenar) + SVG'nin 0,86 grup dönüşümüyle iki ton. Renkli SVG'nin üç rengi
    // varsayılan paletin rolleridir (surface, primary, brand_shade(primary) ≈ #5d8fe8) — testli.
pub fn brand_shade(primary: Color) -> Color;          // HSV: ton +3°, doygunluk 1'e doğru %35, değer ×0,91
pub fn brand_mask(MaskPart, side_px: u32) -> Option<ImageData>;   // MaskPart::{Silhouette, FrontWing, TileSilhouette, TileFrontWing}
// Dışa aktarım: `cargo run -p nuxut-ui --example brand_icons -- <hicolor-kökü>` → scalable/apps/nuxut.svg,
// symbolic/apps/nuxut-symbolic.svg, {48,64,128,256}x…/apps/nuxut.png (sabit renkli dış dosya; uygulama içi çizim değil).
// Galeri: NUXUT_GALLERY_PAGE=9 (koyu + açık palet, 16/24/48/128 px).

// Maskot Fiyu (`brand` özelliği; tasarım ve yerleşim kuralı: docs/design/mascot.md). 7 ruh hâli, her biri iki SVG
// (nuxut-ui/assets/mascot/<mood>.svg sınıflı/renksiz, <mood>-mono.svg currentColor; viewBox 256, ortak merkez; gömülü).
// İki ton: SVG metnindeki class="m-body|m-wing-front|m-detail|m-body m-wing-back" → fill="#rrggbb" (paint_svg; bilinmeyen
// sınıf → çizim yok), raster fiziksel boyutta; renkler mascot_colors(palette)'ten (tema değişince yeniden raster).
// Mono: alfa maskesi bir kez, rengi tint (yeniden raster yok). Süreç geneli LRU (MASCOT_CACHE = 24) anahtarı
// RasterKey::{Duotone{mood, side, colors}, Mono{mood, side}}; bileşen son dokusunu tutar → aynı anahtarda yükleme yok.
// Canlanmaz (boşta kare yok). Gizleme: Theme::appearance.mascot (framework.toml [appearance] mascot, varsayılan true)
// false iken measure() = Size::ZERO ve draw hiçbir şey çizmez (eski bölge hasarlanır); değişim theme_changed ile canlı.
pub enum Mood { Empty, NoResults, Error, Celebrate, Hello, Rest, Thinking }   // ALL; name() = varlık adı; svg()/mono_svg()
pub enum MascotStyle { Duotone /*varsayılan*/, Mono(ColorRole) }
Mascot::new(&Theme, Mood, size_px: f32)   // size 48..=160'a sıkıştırılır (MASCOT_MIN/MAX_SIZE), sonlu değilse 96
    .mono(ColorRole) / .with_style(MascotStyle);  set_mood / set_style / set_size / set_rect(rect /*ortasına kare*/) / set_theme(theme, now)
    measure() -> Size /*gizliyse 0*/; is_visible(); device_rect(scale) -> (Rect, u32); draw(&mut Ui); is_pending(); uploads()
    mood() / style() / size()
pub fn mascot_colors(&Palette) -> MascotColors { body, wing_back, wing_front, detail }
    // Koyu zemin: ön kanat primary, gövde brand_shade(primary) (marka ikili tonu), ayrım ≥ 1,6:1. Açık zemin: ön kanat
    // primary'nin zemine ≈ 3,3:1 veren en açık HCT tonu, gövde primary'den ön kanada ≥ 2,2:1 olacak kadar koyu.
    // Detay: surface/on_surface'tan koyusu. Garanti: ön kanat ve gövde surface + surface_container'a ≥ 3:1, detay ön
    // kanada ≥ 3:1 (statik + üretilmiş paletler × standart/yüksek karşıtlık × açık/koyu; testli).
pub fn paint_svg(&str, &MascotColors) -> Option<String>;  pub fn mascot_image(Mood, MascotStyle, side_px, &MascotColors) -> Option<ImageData>;
// Galeri: NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=mascot (7 mood × 160/96/48 iki ton + 48 mono, koyu + açık).

// Boş durum (talep, 2026-10-13; kural: docs/design/mascot.md "Boş durum yerleşimi").
// Alanda yatay + dikey ortalı tek sütun: görsel, space(4), başlık (Title, ortalı, EMPTY_TITLE_LINES = 2), space(1), gövde
// (Body, on_surface_variant, ortalı, EMPTY_BODY_LINES = 4), space(4), eylem düğmesi (Tonal). Metin sarar, taşan …;
// genişlik = alanın içi (kenar space(4)) ile space(90)'ın darı; metinler MAX_EMPTY_TEXT (1024 bayt) ile kırpılır.
// Maskot alan darsa küçülür (tercih EMPTY_MASCOT_SIZE = 96, en az MASCOT_MIN_SIZE), o da sığmazsa düşer (aralığı da);
// [appearance] mascot = false iken yer kaplamaz (varsayılan ikona geçilmez; with_fallback_icon verildiyse maskotun yerine o
// ikon — ikon kipinin kuralları; karar 2026-10-14). İkon ölçeklenmez: sığmazsa düşer. Sütun alandan
// uzunsa üstü alanın iç üstüne yaslanır. Yerleşim çizimde (tema/ölçek/gizleme değişimini kendiliğinden izler);
// hasar = alan, yalnız yerleşim/içerik değişiminde. Canlanmaz, boşta kare yok (maskot dokusu inene kadar hariç).
EmptyState::new(theme, title)
    .with_mascot(theme, Mood) /*brand*/ | .with_icon(theme, Icon /*IconLarge*/)   // görselsiz: ikisi de yok
    .with_mascot_size(f32) .with_body(theme, s) .with_action(theme, label)
    .with_fallback_icon(theme, Icon) / set_fallback_icon(theme, Option<Icon>) / fallback_icon() -> Option<&Icon>  /*yalnız maskot gizliyken çizilir; maskotsuz/ikonlu kipte etkisiz*/
    set_title(&str) / set_body(theme, Option<&str> /*None, "" → kaldır*/) / set_mood(Mood) / mood() -> Option<Mood> /*brand; maskotsuz → None, gizliyken de kurulan*/ / set_icon(Icon) /
    set_action(theme, Option<&str>) / action_mut() -> Option<&mut Button>; set_theme(theme, now)
    set_rect(alan); draw(&mut Ui); layout(text, theme) -> EmptyLayout /*girdi çizimden önce gelirse*/; last_layout()
    measure(text, theme, max_width) -> Size /*dolgu dahil doğal boyut; görsel tercih kenarında*/
    pointer/key -> Response /*event Some(()) = eylem*/; cursor(p); set_focus_visible(bool)
pub struct EmptyLayout { art: Option<Rect> /*None = görsel yok/düştü/gizli*/, title: Point, body: Point, action: Rect, column: Rect }
pub fn empty_layout(inner: Rect, &EmptyMetrics) -> EmptyLayout;  pub fn fit_art(&EmptyMetrics, inner: Size) -> Option<f32>
    // Saf geometri (shell widgets/empty_state.rs empty_column/fit_mascot'un karşılığı; testli).
// Galeri: NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=empty (geniş/kısa/çok kısa/ikonlu + Pill: satır içi, köşe, liste, sonuç).

// Kaydırma — tekerlek: kritik sönümlü, hedefi aşmayan kayma, kenarda düz durur, yön değişimi anında; dokunmatik yüzey: lastik kenar + üstel fırlatma + geri sekme; çubuk sürüklenir.
// Hareket kapalıyken: tekerlek/scroll_to anında, fırlatma yok (bırakınca durur), taşma anında oturur.
ScrollView::new(theme); set_geometry(viewport, content_h, now); offset(now); content_origin(now);
    scroll_to(off, now); ensure_visible(top, bottom, now) / ensure_visible_now /*animasyonsuz; ikisi de yükseklik 0 iken no-op*/; scroll_end(now) /*pointer PointerKind::ScrollEnd'i kendisi bağlar; yoksa 60 ms sessizlik yedeği*/;
    visible_range(&starts, now) -> Range<usize>; pointer(theme, ev, now) -> Response; draw(ui, |ui, origin, span| ..)
    forward(&ev) -> Forward /*pointer sonrası çocuklara iletilecek olaylar (≤ 3); kapsayıcılar event yerine bunları verir — dokunmada bekletilen basma burada üretilir*/; is_panning()
// Dokunmatik ekran: basma 100 ms / 8 px bekletilir; dikey eşik → parmağı izler (lastik kenar, fırlatma), yatay eşik / bekleme / hızlı dokunuş → basma çocuğa; fırlatmada dokunuş yakalar (docs/design/touch.md).
visible_range_uniform(span, item_h, count) -> Range<usize>
pub enum ListTrailing { None, Text(String), Pill(String) /*Pill; sağa yaslı, metin sınırı space(30), başlık hapın soluna sığar (2026-10-13)*/, Toggle(bool), Icon(Icon), Chevron }
ListItem::new(theme, title).with_subtitle(theme, s).with_leading(theme, Icon).with_trailing(theme, ListTrailing); set_leading(theme, Option<Icon>);
    set_value(&str) /*Text ve Pill*/; set_toggle(bool, now); set_selected(bool); pointer/key -> Response<ListEvent::{Clicked, Toggled(bool)}>
    title() -> &str; is_selected(); set_density(ListDensity) /*tek başına satır için; List kendi yoğunluğunu verir*/
List::new(theme).with_items(Vec<ListItem>); set_rect(theme, rect, now); pointer(theme, ev, now)/key(..) -> Response<(usize, ListEvent)>
List::with_selection(multiple) /*seçim kipi (dosya seçici liste görünümü): Selection kuralları — Ctrl/Shift tık, Shift/Ctrl ok, Space, Ctrl+Space, Ctrl+A; satır görünüşü seçimden; a11y ListBox(+multiselectable)/ListBoxOption*/; selection() -> Option<&Selection>; select_only(i);
    pointer_select(theme, ev, modifiers, now) / key_select(theme, ev, now) -> Response<ListSelectEvent::{Changed, Activated(usize) /*çift tık, Enter*/, Toggled(usize, bool)}>   // ayrı olay türü: ListEvent'e varyant tüketicilerin kapsamlı eşlemelerini kırardı (2026-10-16)
    // İndis sözleşmesi: olaylardaki usize ve items()/items_mut() DÜZ öğe indisidir; grup başlıkları öğe değildir.
    with_groups(theme, &[(start, title)]) / set_groups(..) /*grup başlığı = grubun ilk öğesinin indisi; sırasız verilebilir,
        aynı indiste ilk kalır, öğe sayısını aşan çizilmez; items_mut ile ekleyip çıkaran indisleri yeniden verir*/; group_title(start)
    // Başlık: Caption, on_surface_variant, satır dolgusu hizasında; seçilemez, odaklanamaz, tıklanmaz (imleç Default);
    // klavye gezinmesi başlıklara uğramaz, grubun ilk öğesine gelince başlık da görünür kaydırılır.
    // Bant: ilk grup liste başındaysa yalnızca başlık satırı (Regular space(7), Compact space(6)); sonrakilerde üstüne
    // grup arası boşluk (Regular space(4), Compact space(3)).
pub enum ListDensity { Regular /*satır space(14), alt metinli space(18), dolgu space(4)*/, Compact /*space(10) / space(14), dolgu space(3)*/ }
    with_density(d) / set_density(d); density(); ListDensity::padding(theme)
    // İkon şeridi (dar kenar çubuğu): yalnızca öncü ikon satırın ortasında; başlık/alt metin/sağ uç söner (yolun ilk
    // yarısında); grup başlığı ince ayırıcıya (Separator, bant Regular space(4)/Compact space(3); ilk grubun bandı 0) döner.
    // Seçili vurgu ve odak halkası her kipte. Başlık ipucu (Tooltip, sağda): üzerine gelince theme.timing gecikmesiyle,
    // klavye odağında beklemesiz; tıklanan satırda imleç ayrılana kadar yeniden açılmaz; kaydırmada kapanır; tam kipte yok.
    with_rail(bool) /*animasyonsuz*/; set_rail(bool, now) /*spatial yayı, kesilebilir*/; is_rail() /*hedef*/;
    rail_progress(now) -> f32 /*[0,1]*/; is_rail_animating(now); List::rail_width(theme) = space(16);
    animated_width(theme, full_w, now) -> f32 /*geçişte çağıran her kare bu genişlikle set_rect çağırır; liste içini her
        karede yeniden yerleştirir, oturunca kare istemez*/
    focused() -> Option<usize>; blur(now) /*odak listeden çıktı: halka + odak ipucu kalkar*/;
    tooltip_deadline() -> Option<Instant> /*ipucu beklemesi; draw da Ui::wake_at ile bildirir (bekleme sırasında kare yok)*/
    // Odak halkası satırın İÇİNDE çizilir (kaydırma alanının kırpmasına ve komşu satıra taşmaz).
// İpucu — gecikme bileşende (theme.timing), beklerken kare yok; tonal (surface_bright + on_surface, kenar yok);
// sığmazsa karşı tarafa döner + bounds'a kırpılır; 240 px'te (space(60)) sarılır, TOOLTIP_MAX_LINES = 4 satırdan sonra "…";
// spatial_fast ile 0.92 → 1 büyür + effects ile solar (kesilebilir).
Tooltip::new(theme); set_theme(theme, now);
    hover(text, anchor_rect, MenuSide, bounds, now) -> Option<Instant> /*Some(an): bekleme başladı/sürüyor → o ana zamanlayıcı kur;
        None: görünür/ısınmış pencere → hemen gösterildi/yer değiştirdi. Her motion'da çağrılabilir (aynı öğede bekleme yenilenmez)*/;
    unhover(now) /*bekleme iptal; görünürse kapanır, hover_warm penceresi başlar*/; show(..) /*beklemesiz*/; hide(now);
    poll(now) -> bool /*bekleme dolduysa göster; draw da çağırır*/; deadline() -> Option<Instant>; is_visible(); text();
    draw(ui) /*bekliyorsa ui.wake_at(an); HER ZAMAN kaplama katmanında (kırpılı kapsayıcının içinden çağrılabilir);
        bounds boşsa (Rect::ZERO) ui.surface_rect()*/
pub enum ScreenEdge { Top, Right, Bottom, Left }
MenuSide::away_from(ScreenEdge) -> MenuSide /*Top→Below, Bottom→Above, Left→Right, Right→Left*/; MenuSide::opposite(self)

// Menü — bağlantıdan blob olarak büyür; alt menü sayfaya dönüşür (kaskad yok); sınırsız derinlik.
pub enum MenuItemKind { Action, Check(bool), Radio(bool), Separator, Header, Submenu(Vec<MenuItem>) }
// Header (2026-10-13, t116): seçilemeyen bölüm başlığı — Label rolü, on_surface_variant, öğe etiketleriyle aynı sütun,
//   yükseklik space(8); klavye (↑↓/Home/End/Tab), harfle arama ve işaretçi atlar, imleç Default, find() bulmaz.
// İkonlu radyo (icon Some ya da Select'in ikon yuvası kipi): solda ikon, seçiliyse sağda primary onay (Check);
//   ikonsuz radyo eskisi gibi radyo glifi. (Önceden ikonlu radyoda ikon düşüyordu; tepsi menüsündeki ikonlu radyo da böyle görünür.)
pub struct MenuItem { pub id: MenuItemId /*u64*/, pub label, pub icon: Option<Icon>, pub shortcut: Option<String>, pub enabled, pub kind }
MenuItem::action/check/radio/separator/header/submenu(..).with_icon(..).with_shortcut(..).enabled(..)
MenuModel::new(Vec<MenuItem>); find(id); strip_mnemonic(&str)   // MAX_MENU_ITEMS = 512/sayfa, MAX_MENU_LABEL = 256, MAX_MENU_SHORTCUT = 64
pub struct MenuAnchor { pub rect: Rect, pub side: MenuSide /*Below|Above|Left|Right*/, pub align: MenuAlign /*Start|Center|End*/, pub gap: f32 }
MenuAnchor::below(rect) /*gap 0: bar'a yapışık*/; MenuAnchor::at_point(p, gap)
Menu::new(theme); open(text, theme, model, anchor, bounds, now); close(now); set_model(text, theme, model, now) /*yığın kimlikten korunur*/;
    shape(now, theme) -> Option<BlobShape> /*shell kendi BlobField'ına ekler; karede bir kez*/; draw_content(ui); draw(ui) /*kendi alanı*/; draw_overlay(ui) /*kendi alanı, kaplama katmanında + girdi bölgesi*/;
    input_rect() -> Option<Rect>; is_open(); is_visible(); depth();
    pointer(text, theme, ev, now) / key(text, theme, ev, now) -> Response<MenuEvent::{Activated(id), SubmenuOpened(id), Dismissed}>

// Bildirimler — kaydırarak kapatma (mesafe ≥ %35 genişlik ya da hız ≥ space(200)/s), yığın yayla kayar.
pub struct BodySpan { pub text, pub bold, pub italic, pub underline, pub link: Option<String> }
pub struct NotificationContent { pub app_name, pub app_icon: Option<Icon>, pub time, pub title, pub body: Vec<BodySpan>, pub image: Option<Texture>, pub actions: Vec<String> }
    // new() keser: başlık 256, gövde 4096, eylem 64, ad/zaman 128 bayt; en çok 3 eylem
    // Gövde ui ailesinde (TextRole::Body), Document (serif) değil — karar 2026-10-14; kapalı 3, açık 12 satır.
NotificationCard::new(theme, content); measure(text, theme, width) -> f32; set_rect(text, theme, rect); set_expanded(theme, bool);
    set_time(&str); dismiss(now); is_gone(now); image_rect(theme, now); app_icon_rect(theme, now);
    pointer(theme, ev, now) -> Response<NotificationEvent::{Activated, Action(usize), Link(String), Dismissed}>
    // Maskot yuvası (brand; talep, 2026-10-14; kural: docs/design/mascot.md "Bildirim yerleşimi").
    // YALNIZ Nuxut'un iç bildirimleri ("güncelleme bitti" → Mood::Celebrate); dış uygulama bildirimlerinde maskot yok.
    // Metin sütununun solunda, başlık satırının altında, karosuz, NOTIFICATION_MASCOT_SIZE = 64 kenar; metin space(3)
    // sağında başlar, kart en az maskot boyu. Görsel yuvası (sağ, space(14)) değişmez; ikisi birlikte olabilir.
    // Kartın kaydırma/kapanma solması ve dış (yığın) alfası maskota da uygulanır; kaydırmayı izler. Boşta kare yok.
    // [appearance] mascot = false: yuva hiç yok (metin sola yaslı, yükseklik yalnız metne göre); set_theme ile canlı —
    // görünürlük değişince yükseklik değişebilir, çağıran tema değişiminde measure + set_rect yapar.
    .with_mascot(theme, Mood) / set_mascot(theme, Option<Mood>) /*yuva eklenir/kalkar → yeniden ölç*/; mascot() -> Option<Mood>;
    mascot_rect(theme, now) -> Option<Rect> /*kaydırma dahil; yok ya da gizliyse None*/
    // Galeri: NUXUT_GALLERY_PAGE=9 NUXUT_GALLERY_STATE=notif-mascot.
NotificationStack::<K>::new(theme); layout(&[(K, h)], gap, now) -> f32; slot(&K, now) -> Option<StackSlot { y, alpha }>; is_animating(now); bottom_up

// Metin girişi — pano/IME kancaları; imleç yanıp sönmez, yayla kayar.
TextInput::new(theme).with_placeholder(s).with_text(s).password(bool).with_max_bytes(n); set_focused(bool, now);
    key(ev, now) / pointer(text, theme, ev, now) / paste(&str) -> Response<TextInputEvent::{Changed, Submit, Cancel, Copy(String), Cut(String), PasteRequested}>
    ime_state() -> ImeState { cursor_rect, surrounding, cursor, anchor, password }   // → wayland TextInputState
    ime_preedit(&str, Option<(usize, usize)>); ime_commit(&str); ime_delete_surrounding(before, after)
    caret() -> (cursor, anchor); select_range(start, end) -> bool   // bayt; kırpılır, grafem kümesi başına çekilir; çapa = start
                                                                     // (dosya seçici kaydetme adı: uzantıdan önceki kısım seçili)

// Renk seçici — SV alanı + ton şeridi (gradient_rect) + hex + her an açık palet önerileri.
ColorPicker::new(theme, color); set_color(theme, c, now); set_recent(theme, &[Color]) /*≤ MAX_RECENT*/; suggestions();
    set_hex_focused(bool, now); pointer(text, theme, ev, now) / key(theme, ev, now) -> Response<ColorEvent::{Changed(Color), Committed(Color)}>

// Takvim — hafta başı ve ad tabloları nuxut-locale'den (tanınmayan ad → en_US), bağımlılıksız tarih.
Date { year, month, day }::new / to_days / from_days / weekday /*0 = Pzt*/ / add_days / add_months / from_unix_seconds(secs, utc_offset)
first_weekday_for_locale(&str) -> u8 /*çözülemeyen → 0*/; locale_from_env() -> String /*Locale::from_env().name()*/; CalendarLabels::for_locale(&str) /*months = yalın ad (standalone_month_name)*/;
    format_date(Date, &CalendarLabels, locale) -> String /*Locale::format_ymd(.., DateStyle::Long): "4 Ekim 2026", "October 4, 2026", "2026年10月4日"*/;
    // CalendarGrid ve WeekGrid başlıkları Locale::compose_month_year ile ("Ekim 2026", "2026年10月", "Ekim – Kasım 2026")
CalendarGrid::new(theme, today, locale); set_today; set_selected; set_events(Vec<(Date, Vec<Option<Color>>)>) /*≤ 3 nokta*/;
    show_month(theme, y, m, now); pointer/key -> Response<CalendarEvent::{DaySelected(Date), MonthChanged { year, month }}>
// Takvim locale'i (2026-10-13, görev; geriye uyumlu ek): uygulamalar nuxut::i18n::time_locale() verir
// (framework.toml [locale] formats; auto = LC_ALL > LC_TIME > LANG) — hafta başı (tr Pzt, en_US Paz, ar_EG Cmt), adlar, başlık sırası oradan.
CalendarGrid::with_locale(theme, today, &Locale); set_locale(theme, &Locale) /*locale_changed: ay/seçim/bugün/etkinlik korunur, süren geçiş biter; aynı locale'de iş yok*/;
    first_weekday_of(&Locale) -> u8; CalendarLabels::from_locale(&Locale); CalendarGrid::new(theme, today, &str) aynen (çözülemeyen ad → en_US adları + Pazartesi).
// Dar genişlik (≤ 280 px; shell takvim popout'u 220–320): ölçüme dayalı, ilk çizimde (TextSystem orada) bir kez hesaplanır,
// hücre boyu/etiketler değişmedikçe yeniden ölçülmez; varsayılan genişlikte (340 ve üstü; test) yerleşim öncekiyle birebir.
//  - Gün adları: kısa adlardan biri hücreye space(2) payla sığmazsa HEPSİ ilk grafeme düşer (narrow_weekday("Çar") = "Ç";
//    glibc'de CLDR narrow yok, CLDR'nin narrow'u da çoğu dilde ilk harf; birleşik aksan bölünmez). Karışık biçim yok.
//  - Noktalar: varsayılan yer rakama değiyorsa rakamın altına iner, yetmezse rakam yukarı kayar (tüm hücrelerde aynı),
//    o da yetmezse nokta çizilmez; sayı hücreye sığan kadar (≤ MAX_EVENT_DOTS). 220–320'de 3 nokta korunur (test).
//  - Bugün/seçim dairesi: noktalar varsayılan dairenin kenarına binecekse daire rakamı ortalayıp noktaların üstünde biter,
//    noktalar dışarıda kendi renginde (dairenin içindeyse seçili günde on_primary, öncekisi gibi).
//  - Ay başlığı tek satır, önceki-ay düğmesine kadar; sığmazsa üç nokta ("September 20…").
// Haftalık ızgara — 1–7 gün; zamanlar yerel saniye (Unix + çağıranın UTC ofseti; bileşen saat dilimi bilmez).
// Çakışanlar yan yana sütunda, gece yarısını aşan bölünür, tüm gün şeridi ≤ MAX_ALL_DAY_LANES (+N), "şimdi" çizgisi.
WeekEvent::timed(key, title, start, end) / all_day(key, title, first, last).with_color(Option<Color>)
WeekGrid::new(theme, today, locale).with_days(theme, n); set_days; range() -> (Date, u8); local_range(); set_today; set_now(Option<i64>);
    set_events(theme, Vec<WeekEvent>) /*≤ MAX_WEEK_EVENTS*/; scroll_to_minute; show_date(theme, d, now) -> Option<WeekGridEvent>;
    pointer/key -> Response<WeekGridEvent::{EventActivated(u64), SlotActivated { date, minute /*SLOT_MINUTES*/ }, DayActivated(Date), RangeChanged { start, days }}>
// Gündem — EventRow (liste satırı: renk şeridi, başlık, saat · yer; geçmiş soluk, süren vurgulu; tıklama/Enter = Response
// olayı) ve EventCard (ayrıntı: saat, yer, takvim, Document fontunda açıklama ≤ CARD_DESCRIPTION_LINES).
AgendaItem::new(title, time).location(s).calendar(s).description(s).color(Option<Color>).past(b).ongoing(b)  /*Debug redakte*/
// Palet renk seçici — temadan türetilen ayırt edilebilir renkler; isteğe bağlı "özel renk" düğmesi; ok tuşları + Enter.
swatch_palette(theme, n) -> Vec<Color>; SwatchPicker::new(theme, colors, selected).with_custom(theme); set_colors; set_selected;
    preferred_size(theme, w); pointer/key -> Response<SwatchEvent::{Selected(Color), Custom}>

// Tema önizleme şeritleri (Ayarlar tema kartı içinde; görsel çözmez, dokuyu uygulama verir):
pub type CursorPreview = ThemePreview<Cursors>;      // öğe space(8); doku kendi boyutunda (÷ ölçek), yuvaya sığmazsa küçülür
pub type IconThemePreview = ThemePreview<Icons>;     // öğe space(8); doku öğe kutusuna Contain
ThemePreview::<K>::new(theme, slots /* ≤ MAX_PREVIEW_SLOTS = 16, hepsi Loading */).with_item_size(logical)
    .with_background(ColorRole) /* kuyu zemini, varsayılan Surface; background() */;
    item_size(); load_px(scale) -> u32 /* item × scale: cursor_request px'i, ikon size×scale */;
    preferred_size(theme) /* tüm yuvalar */; preferred_height(theme) /* space(3)·2 + item */; set_rect(r); rect();
    len(); slot(i) -> Option<&PreviewSlot>; set_slot(i, PreviewSlot, now) /* Ready(boş) = Missing */;
    reset(theme, slots) /* başka tema: hepsi Loading, görünenler yeniden istenir */;
    take_load_request() -> Option<Range<usize>> /* çizimden sonra: yeni görünen yuvalar, bir kez */;
    pointer(ev, now) -> Response /* olay yok; üzerindeyken animasyonlu imleç oynar */; cursor(p) -> None;
    set_theme(theme, now); draw(ui)
pub enum PreviewSlot { Loading, Ready(Vec<PreviewFrame>), Missing }
pub struct PreviewFrame { texture: Texture, delay: Duration }   // PreviewFrame::still(tex)
// Görünüm: tek "kuyu" (surface dolgu — with_background ile rol —, radius_medium − space(1) ≥ radius_small; kenar yok), iç boşluk
// space(3), yuvalar eşit aralıklı; genişliğe sığmayan sondaki yuvalar çizilmez ve istenmez.
// Yükleniyor/yok: öğe × 0.22 çaplı on_surface_variant %18 nokta (statik). Görsel: motion.effects ile
// belirir, 0.92 → 1 büyür. Animasyon yalnızca işaretçi şeritteyken; sonraki kare zamanına wake_at
// (sürekli kare istenmez), ayrılınca ilk kare. Boşta iş yok.

// Ortak büyüme hareketi (iç modül `grow`): Menu, Popover ve Select'in listesi aynı tohum → hedef
// morfu, esneme, içerik solması ve bağımsız çizimi paylaşır.

// Bölümlü düğme — 2..=5 seçenek; seçili hap `spatial` ile kayar, içerik rengi örtüşmeyle geçer.
pub struct Segment { pub label: String, pub icon: Option<Icon>, pub disabled: bool }
    Segment::new(s).with_icon(Icon).with_disabled(bool)
SegmentedButton::new(theme, Vec<Segment>, selected); preferred_size(text, theme); set_rect(theme, rect);
    set_selected(i, now); set_enabled(bool, now); pointer(ev, now) / key(ev, now) /*Sol/Sağ/Home/End*/ -> Response<usize>
    set_segment_enabled(i, bool, now) /*aralık dışı etkisiz*/; is_segment_enabled(i) -> bool
// Seçenek başına devre dışı (istek, 2026-10-14; ayrıntı docs/design/segmented.md): soluk çizilir
// (disabled_content; yüksek karşıtlıkta zemine ≥ 3:1), fare ile seçilemez (hover yok, imleç Default), klavye
// gezinmesi atlar (Sol/Sağ en yakın etkin, Home/End ilk/son etkin; yoksa yerinde). Seçili seçenek devre dışı
// yapılırsa seçim DEĞİŞMEZ, yalnız etkileşim kapanır; gösterge yerinde kalır, dolgusu devre dışı kap (%12).
// set_selected (dış durum) devre dışı seçeneği de seçebilir.

// Açılır satır (Disclosure/Expander; aşamalı açıklama) — başlık + isteğe bağlı açıklama, kapalıyken özet,
// isteğe bağlı nokta rozeti, açılınca 180° dönen chevron (aynı yay). İçerik yüksekliği spatial_fast
// yayıyla kesilebilir açılır/kapanır; hareket kapalıysa anında; boşta kare yok. İçerik çağıranın.
pub enum DisclosureEvent { Toggled(bool) }   // kullanıcı açtı/kapattı (durum zaten güncel)
Disclosure::new(theme, title).with_description(theme, s); set_title(s); set_description(theme, Option<&str>);
    set_summary(s) /*boş = yok; açılırken söner*/; set_badge(bool, now); set_theme(theme, now) /*TextSystem'siz, ölçüm tembel*/;
    set_enabled(bool, now); set_focus_visible(bool) /*halka satırın içinde*/;
    is_open(); set_open(open, animate, now) -> bool /*olay üretmez; animate=false anında (arama kaydırması, oturum geri yükleme)*/;
    set_content_height(h) /*içeriğin tam yüksekliği; çağıran bildirir*/; header_height(text, theme, width) -> f32 /*≥ style::row_height (space(16)); özet sığmazsa alta iner*/;
    height(now) -> f32 /*başlık + içerik × progress*/; set_rect(rect) /*tüm blok*/; header_rect(); content_rect() /*tam içerik, yerleşim için*/;
    content_clip(now) /*görünen bölüm*/; progress(now) /*0..~1, yayda taşabilir*/; reveal(now) /*0..=1, solma*/; is_animating(now);
    pointer(ev, now) / key(ev, now) /*Enter/Space aç-kapa, Sağ aç, Sol kapa*/ -> Response<DisclosureEvent>; cursor(p) /*başlıkta Pointer*/;
    flash(now) /*başlık*/ | flash_content(rect /*içerik koordinatı*/, now) /*arama sonucu: timing.flash_hold tam, sonra effects ile söner; bekleyiş wake_at*/;
    draw(ui) /*başlık + vurgu*/; draw_content(ui, |ui, content_rect| ..) /*görünen bölüme kırpar, reveal ile soldurur*/
    // Çağıran her karede: set_content_height → header_height → height(now) → set_rect (yükseklik canlanırken altındakiler kayar). MAX_DISCLOSURE_TEXT = 256

// Açılır seçim — Menu'nün radyo öğeleri; alanın altında açılır, sığmazsa üstte; liste ≥ alan genişliği.
Select::new(theme, Vec<String>, selected); set_rect(theme, rect); set_bounds(rect); set_selected(i); is_open();
    set_theme(theme, now) /*TextSystem'siz: etiket genişliği ve açık menünün sayfaları sonraki set_rect/draw/draw_overlay/draw_menu_content/pointer/key'te yeniden ölçülür*/;
    pointer(text, theme, ev, now) / key(text, theme, ev, now) /*Enter/Space/Aşağı açar*/ -> Response<SelectEvent::Changed(usize)>;
    draw(ui) /*alan*/; draw_overlay(ui) /*bağımsız: kaplama katmanında + girdi bölgesi; alanın hemen ardından, kırpma içinden çağrılabilir*/ | menu_shape(now, theme) + draw_menu_content(ui) /*shell blob*/
    // MAX_SELECT_OPTIONS = 512 (girdi: seçenek + başlık + ayraç), MAX_SELECT_LABEL = 256; Menu::set_min_width(w)
// Bölümler ve ikonlar (2026-10-13, settings Varsayılan uygulamalar t116; geriye uyumlu ek — Select::new aynen çalışır):
pub struct SelectOption { pub label: String, pub icon: Option<Icon> }   SelectOption::new(s).with_icon(impl Into<Icon>); From<String>, From<&str>
pub enum SelectEntry { Option(SelectOption), Header(String), Separator }   SelectEntry::option(s) / header(s); From<SelectOption>
Select::with_entries(theme, Vec<SelectEntry>, selected) /*selected = SEÇENEK indeksi*/; .with_icon_slot(bool); has_icon_slot();
    options() -> &[String] /*yalnız seçenekler*/; option_icon(i) -> Option<&Icon>; set_option_icon(i, Option<Icon>) /*ikon geç geldi ya da yüklenemedi (None)*/
    // İndeks sözleşmesi: selected()/set_selected/SelectEvent::Changed(i) her zaman seçenekler arasındaki sıra; başlık ve ayraç
    //   sayılmaz (düz metin API'siyle aynı anlam; açık kimlik yolu yok — çağıran indeks → kendi anahtarı eşlemesini tutar).
    // Başlık/ayraç: Menu Header/Separator satırları (menü kimlikleri 1<<32 + sıra, seçeneklerle çakışmaz); gezinme, harfle
    //   arama ve tıklama atlar, imleç Default.
    // İkon yuvası: bir seçeneğin ikonu varsa ya da with_icon_slot(true) (ikonlar sonradan yüklenecek) — alanda seçili seçeneğin
    //   ikonu (TextRole::Icon kutusu, on_surface_variant; renkli doku kendi renginde) etiketin solunda, listede her seçenek
    //   satırında ikon sütunu (IconSmall) ve seçilide sağda primary onay işareti. İkonsuz (None) ya da dokusu henüz GPU'ya
    //   ulaşmamış seçenek: yer tutucu (ikon kutusu boyunda yuvarlak kare, style::icon_placeholder = on_surface_variant ×
    //   ICON_PLACEHOLDER_TONE 0.16). Kutu boyu sabit: ikon gelince/gidince yerleşim zıplamaz. Bekleyen doku için kare
    //   istenir, hazır olduğu karede bir kez yeniden boyanır; sonra boşta iş yok. set_option_icon açık listeyi de bir
    //   sonraki çizim/girdide güncel modelle yeniden kurar (yay hareketi Menu'nünki).
    // Uygulama ikonu taşıma: Icon::image(Texture) (nuxut_image::IconTheme/Loader → ImageData → Painter::upload_texture);
    //   semantik ikon Icon::named(IconName). Erişilebilirlik ağacı framework'te henüz yok: başlığın "seçilemez" bilgisi
    //   şimdilik yalnız davranışta (atlanır, Default imleç); a11y katmanı gelince Header rolüyle işaretlenecek.

// Popover — bağlantıdan blob olarak büyüyen kap; içerik çağıranın.
Popover::new(theme); open(theme, MenuAnchor, content_size, bounds, now); set_content_size(theme, size, now); close(now);
    shape(now, theme) -> Option<BlobShape>; content_rect(now, theme) /*görünen, iç boşluklu*/; content_target(theme) /*yerleşim*/;
    draw_background(ui) /*bağımsız*/; draw_overlay(ui, |ui, rect| ..) /*arka plan + içerik kaplama katmanında + girdi bölgesi*/; draw_content(ui, |ui, rect| ..) /*kırpar + soldurur*/; input_rect();
    pointer(ev, now) /*dışarı basma*/ / key(ev, now) /*Esc*/ -> Response<PopoverEvent::Dismissed>

// Hareket eğrisi — bezier tutamaçları (x ∈ [0,1], y ∈ [-1,2]) ya da yay grafiği; döngüsel önizleme
// yalnızca çizilirken kare ister; çizgiler yuvarlak nokta dizisi.
pub enum CurveValue { Bezier(CubicBezier), Spring(Spring) }
CurveEditor::new(CurveValue); set_value(v); set_preview(Option<Duration> /*bezier süresi*/); preferred_size(theme, w); set_rect(r);
    pointer(theme, ev) -> Response<CurveEvent::{Bezier(CubicBezier), Spring(Spring)}>   // yayda sürükleme: x → response, y → damping
    // Önizleme döngüsü = hareket + %30 bekleme; beklemede kare istenmez, sonraki döngü başı ui.wake_at.

// Sıralanan liste — tutamaçtan ya da gövdeden sürükle (yükselir + gölge, diğerleri yayla yer açar), Alt+↑/↓, Delete, kaldır düğmesi.
// Eşik altı (space(2)) bas-bırak ve odakta Enter/Boşluk = Activated (ör. ayar alt sayfasını aç).
pub struct ReorderItem { pub label: String, pub icon: Option<Icon> }
ReorderList::new(theme, Vec<ReorderItem>); set_rect(theme, rect, now); preferred_height(theme); insert(theme, i, item, now); remove(theme, i, now);
    pointer(theme, ev, now) / key(theme, ev, now) -> Response<ReorderEvent::{Moved { from, to }, Removed(usize), DraggedOut { index, position }, Activated(usize)}>

// İlerleme — Spinner: dönen, boyu nefes alan yay (hız tema hareketinden); belirli kip tonal rayla. Hareket kapalıyken
// belirsiz kip durağan: tonal iz + eşit aralıklı üç parça, kare istemez. Progress: doğrusal.
Spinner::new(theme); set_progress(theme, Option<f32>, now); set_rect(r) /*kare*/; draw(ui) /*görünürken kare ister*/
Progress::new(theme, v).with_color(Tint).with_threshold(Threshold); set_value(v, now); set_rect(r);
    set_color(Tint); color() -> Tint; set_threshold(Option<Threshold>, now); is_alert() -> bool; set_theme(theme, now)
// Gauge: dairesel gösterge — tonal iz + değer yayı (aralarında kalınlık kadar boşluk) + isteğe bağlı ortada etiket (tabular, tek satır, "…").
Gauge::new(theme, v).with_color(Tint).with_threshold(Threshold).with_thickness(px /*vars. space(1)*/)
    .with_sweep(turns /*1 = tam halka saat 12'den; <1 açıklığı altta ortalı*/).with_label(theme, s).with_label_role(theme, TextRole);
    set_value(v, now); set_label(&str) -> bool; set_color/color/set_threshold/is_alert/set_theme; preferred_size(theme) /*space(12) kare*/;
    set_rect(r) /*kısa kenara sığar, ortalanır*/; draw(ui)
// Renk: Tint = tema rolü (önerilen) ya da çağıranın rengi; her karede temadan çözülür, tema geçişini izler.
pub enum Tint { Role(ColorRole), Color(Color) }   // Tint::PRIMARY (vars.) / Tint::ERROR; From<ColorRole>, From<Color>; resolve(theme)
// Eşik: değer level'e ulaşınca renk tint'e (vars. error) motion.effects ile kesilebilir geçiş; level − hysteresis altında geri döner
// (vars. THRESHOLD_HYSTERESIS = 0.03; ölçüm eşikte titreşse de renk yanıp sönmez). Karar hedef değerle; NaN durumu değiştirmez.
pub struct Threshold { pub level: f32, pub tint: Tint, pub hysteresis: f32 }  // Threshold::new(level).with_tint(t).with_hysteresis(h)
// Pay birimi: new → THRESHOLD_HYSTERESIS (0.03, 0..=1 ölçeği). Başka birimde: for_range(level, full_scale)
// (pay = 0.03 × ölçek; ör. °C 0..=120 → 3.6), percent(level) = for_range(level, 100) (3 puan); açık pay with_hysteresis(h).
// Sparkline: küçük zaman serisi — çizgi (SPARKLINE_STROKE = 2) + isteğe bağlı tabana sönen dolgu (SPARKLINE_FILL = 0.28, Painter::area).
// Yeni örnek sağdan girer, grafik bir aralık sola kayar (spatial_fast; hızlı itmede birikir, MAX_SCROLL_LAG = 2 aralıkta
// atlar); Auto/AutoMax ekseni aynı yayla akar. Sonlu olmayan örnek boşluktur (çizgi kesilir, ölçek/eşik yok sayar).
// Örnek gelmezse kare yok, oturunca kare istenmez, hasar = kendi dikdörtgeni (çizim ona kırpılır), karede ayırma yok.
pub enum SparkScale { Fixed { min, max } /*UNIT = 0..1 vars.; yüzde: 0..100*/, AutoMax { min, min_max } /*ağ hızı*/, Auto { min_span } }
Sparkline::new(theme, capacity /*1..=MAX_SPARK_SAMPLES (1024); görünen örnek sayısı*/).with_scale(SparkScale).with_fill(bool)
    .with_tint(Tint).with_threshold(Threshold /*level örnek biriminde; son örneğe göre tüm çizgi*/);
    push(v, now); set_samples(iter) /*animasyonsuz, panel açılışında saklı geçmiş*/; clear(); history() -> &SampleHistory;
    set_scale(s, now); set_fill(bool); set_tint(Tint); set_threshold(Option<Threshold>, now); is_alert(); set_theme(theme, now);
    preferred_size(theme, width) /*yükseklik space(10)*/; set_rect(r); is_animating(now); draw(ui)
SampleHistory::new(capacity)   // sabit kapasiteli halka tampon (servis tarafında da kullanılabilir): push(v) -> Option<düşen>;
    get(i) /*0 = en eski*/; latest(); iter() /*eskiden yeniye*/; range() -> Option<(min, max)> /*sonlu örnekler*/; len/is_full/clear

// Uygulama düzeyi bileşenler.
// Odak sırası — Tab/Shift+Tab gezinir (devre dışı atlanır, dolanır), halka yalnızca klavyeyle; diğer tuşlar odaktakine.
FocusScope::new(len); set_len(n); set_enabled(i, bool); key(&KeyEvent) -> Option<FocusChange { from, to }>;
    focus_pointer(i) /*halka gizli*/; focus(i); clear(); focused(); ring_visible_for(i)
// Gezinme rayı — ikon + etiket, kayan hap gösterge, daraltılmış kip (genişlik yayla).
NavigationRail::new(theme, Vec<NavItem { label, icon }>, selected); set_collapsed(theme, bool, now); current_width(text, theme, now);
    expanded_width(text, theme); set_rect(theme, r); set_selected(theme, i, now); pointer(theme, ev, now) / key(theme, ev, now) /*↑↓ Home End*/ -> Response<usize>
// Diyalog — scrim (tema gölge rengi) + ortadan büyüyen modal; Esc/scrim → Dismissed (set_dismissible), Enter → birincil.
Dialog::new(theme); open(text, theme, title, Vec<DialogAction { label, kind: Default|Primary|Destructive }>, content_size, bounds, now); close(now);
    content_rect(theme); pointer(ev, now) / key(ev, now) -> Response<DialogEvent::{Action(usize), Dismissed}>; draw(ui, |ui, rect| gövde)
    access(tree, key, |tree| gövde) -> Option<AccessNode> /*açıkken Dialog, modal*/; access_action(container, req, now) -> Response<DialogEvent>
ButtonVariant::Destructive   // `error` dolgu, `on_error` içerik
// Snackbar — alt ortada yayla yükselir; zaman aşımı çağıranın zamanlayıcısı.
Snackbar::new(theme); show(text, theme, msg, Option<&str /*eylem*/>, area, now); hide(now); pointer(ev, now) -> Response<SnackbarEvent::Action>
// Chip'ler — Filter (✓ yer açar) / Choice (grupta tekli) / Input (× kaldırır) / Assist.
Chip::new(theme, ChipKind, label).with_icon(theme, Icon).selected(bool); pointer(theme, ev, now) / key(ev, now) -> Response<ChipEvent::{Toggled(bool), Clicked, Removed}>
ChipGroup::new(Vec<Chip>, single); layout(text, theme, origin, width) -> h; pointer(text, theme, ev, now) -> Response<(usize, ChipEvent)>; selected()
// Arama alanı — hap TextInput + arama ikonu + temizle; Esc metni temizler, boşsa Cancel.
SearchField::new(theme, placeholder); set_placeholder(&str) /* dil değişince; aynı metin iş yapmaz */; set_rect(theme, r); pointer(text, theme, ev, now) / key(ev, now) -> Response<TextInputEvent>
TextInput::with_insets(left, right).pill_shaped(bool); placeholder() -> &str; set_placeholder(&str)
// Tarih/saat alanları — alan + Popover; 12/24 saat kurulu Locale'den (`[locale] hour_cycle` dahil) ya da locale adından (yedek 24).
DatePicker::with_locale(theme, date, today, &nuxut_locale::Locale)   // uygulamalar time_locale() verir: alan metni (DateStyle::Long), takvim adları ve hafta başı oradan
DatePicker::new(theme, date, today, locale: &str)   // geriye uyumlu: addan çözer (çözülemeyen → en_US adları, Pazartesi başı)
    set_locale(theme, &Locale, now) -> bool   // locale_changed'de; değişmezse iş yok; seçili tarih + görünen ay korunur, açık popover yeni boyuta kurulur (CalendarGrid::set_locale)
    text() -> &str; locale() -> &Locale; calendar() -> &CalendarGrid
    set_date(d); set_bounds(r); pointer(theme, ev, now) / key(theme, ev, now) -> Response<DateEvent::Changed(Date)>; draw(ui); draw_overlay(ui)
TimePicker::with_locale(theme, Time { hour, minute }, &nuxut_locale::Locale)   // uygulamalar nuxut_i18n::time_locale() verir (hour_cycle'a uyar)
TimePicker::new(theme, time, locale: &str)   // geriye uyumlu: addan çözer, hour_cycle'ı GÖRMEZ
    .with_minute_step(step) / set_minute_step(theme, step, now) -> bool; minute_step()   // 60'ı bölen (MINUTE_STEPS; normalize_minute_step: 7 → 6); varsayılan MINUTE_STEP = 5
    set_locale(theme, &Locale, now) -> bool   // App::locale_changed'de time_locale() ile; değişmezse iş yok; açık ızgara yeni boyuta morf olur
    set_time(t, now) /*girişi bırakır*/; is_entering(); cancel_entry() -> bool /*odak kaybında çağıran*/; is_twelve_hour(); day_periods()
    pointer/key -> Response<TimeEvent::Changed(Time)>; draw; draw_overlay
    // Izgara, ↑/↓ ve tekerlek (alan üstünde ya da açık içerikte; yukarı = ileri; saat ızgarası üstünde ±1 saat) adımı izler;
    // hizasız saat önce ızgaraya oturur (14:07, adım 5: ↑ 14:10, ↓ 14:05). PageUp/PageDown ±1 saat. 30'dan çok dakika hücresinde
    // (1–2 dk) ızgara 10 sütun, alçak hücre (1 dk: 6 satır). Hücre etiketleri kurulurken bir kez; boşta iş yok, açılma Popover yayı.
    // Serbest giriş: odaktayken rakam yazmak saati girer ("1" → "1_:__", alan altı çizili). Saatin ilk rakamı tek başına saat
    // olamayacaksa bekler (24 s: 0–2, 12 s: 0–1), olamazsa saat hemen tamamlanır ("7" → 07). Bekleyen rakamdan sonra iki basamaklı
    // saat yapamayan rakam dakika onlar basamağıdır (12 s "130" → 1:30, 24 s "25" → 02:5_); `:` ya da `.` bekleyen rakamı saat yapar
    // ("1:30"; saatten sonra tek ayırıcı iş yapmaz, girişsiz ayırıcı giriş başlatmaz). Geçersiz rakam (dakika onlar > 5, 24 s "26",
    // 12 s "00") yutulur, durum değişmez. Dördüncü rakam ya da Enter uygular (eksik dakika basamağı 0; popover açıksa kapanır),
    // Backspace siler, Esc vazgeçer; ↑/↓/PageUp/PageDown girişi bırakıp adım atar; 12 saatlikte a/p gün dilimi (girişsiz: hemen
    // uygulanır). Adımdan bağımsız: tam dakika girilir. Ctrl/Alt/Logo'lu tuş giriş değildir. Seçici IME açmaz (text-input
    // etkinleştirmez): rakamlar ham tuş olarak gelir.
    // Alan metni Locale::format_hm (bar saatiyle tek kaynak: tr "14:05", en_US "2:05 PM", fi "14.05");
    // set_locale tüm Locale'i karşılaştırır (aynı döngüde tr → fi de yeniler). Yarım giriş metni her locale'de "hh:mm [gün dilimi]".
format_date(date, &CalendarLabels, locale); format_time(t, Option<&[String; 2]>) /*None = 24 saat*/; day_periods(locale) -> Option<[String; 2]> /*12 saatlik locale → %p adları (ÖÖ/ÖS, AM/PM)*/; uses_12_hour(locale); TimePicker::day_periods()
day_periods_for(&Locale) -> Option<[String; 2]>   // with_12_hour'a uyar: h12 + de_DE → [AM, PM] (boş %p yedeği), h23 + en_US → None
step_time(t, step, dir) -> Time; normalize_minute_step(u8) -> u8; MINUTE_STEPS = [1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 30]
// Not satırı — ikon + sarılan metin, dolgu/çerçeve yok (iç boşluğu çağıran verir). Renkler yalnızca tema rollerinden.
Notice::new(theme, NoticeKind::{Warning (varsayılan), Info, Success}, text); set_kind(k) -> bool; set_text(&str) -> bool; set_theme(theme, now);
    .with_size(theme, NoticeSize) / .compact(theme); set_size(theme, s) -> bool; size()
    measure(text, theme, width) -> Size /*metin ikon + boşluktan kalan genişlikte sarılır; yükseklik max(ikon, metin); düzey ölçüyü değiştirmez, boyut değiştirir*/; set_rect(r); draw(ui)
NoticeSize::{Regular (varsayılan): Body metin, Icon ikon, space(3) | Compact: Caption metin, IconSmall ikon, space(2)}   // Compact: form satırı altı (Ayarlar geçersiz kılma uyarısı); text_role(), icon_role(), gap(theme)
NoticeKind::icon() /*Warning | Info | CheckCircle*/; icon_color(&Palette) /*Warning: error, Info: on_surface_variant, Success: Palette::success()*/;
    text_color(&Palette) /*Warning, Success: on_surface; Info: on_surface_variant*/   // Success iki kipte, standart/yüksek karşıtlıkta: metin ≥ 4.5, ikon ≥ 4.5 (≥ 3 gerekli)
// Parola kipi — bkz. "TextInput parola kipi: güvenlik garantileri".
TextInput::password(true); with_secret(|&str| …) -> R; len(); is_empty(); is_password()   // parolada text()/selection() ""; PASSWORD_MAX_BYTES = 1024
// Parola göster/gizle düğmesi (istek, 2026-10-14). VARSAYILAN KAPALI: mevcut alanlar (kilit ekranı, polkit, DeviceRow) değişmez; açmak tüketicinin kararı.
// Boyut (2026-10-14, Ayarlar kırılması): düğme durumu ve IME ön düzenlemesi `Option<Box<..>>` içinde; `size_of::<TextInput>()` 840 bayt (göster/gizle öncesi 872, kutusuz 1648). `size_stays_small` testi 872 üst sınırını tutar: tüketicilerin enum kollarında satır içi duran bileşen büyürse clippy `large_enum_variant` onları kırar. Yeni nadir alan kutulanır.
TextInput::with_reveal_toggle(theme, bool) / set_reveal_toggle(theme, bool) /*kapatmak gizler*/; has_reveal_toggle(); is_revealed(); set_revealed(bool) -> bool /*parola kipi dışında etkisiz; düğmesiz de kullanılabilir*/;
    set_reveal_labels(show, hide) /*erişilebilir adlar çağıranın dilinde (t!); framework gömülü metin taşımaz*/; reveal_label() -> &str /*şu anki eylem*/;
    focus_step(&KeyEvent, now) -> bool /*alan içi Tab durağı: odaktaki parola alanında Tab metin → düğme, düğmede Shift+Tab → metin; true = tüketildi. Uygulama Tab'ı FocusScope'tan ÖNCE buraya verir*/;
    focus_from_end(now) /*Shift+Tab ile sondan gelince odak doğrudan düğmede*/; is_reveal_focused()
    // Düğme: alanın sağ ucunda IconButton (Standard, space(8), kenardan space(2); with_insets'in sağ yuvasının solunda), ikon eylemi gösterir: gizliyken Visibility, gösterilirken VisibilityOff.
    // Düğmedeyken Enter/Space çevirir, Esc = Cancel; yazı, paste, ime_commit, ime_delete_surrounding alana gitmez (yok sayılır); metin imleci gizli.
    // Fareyle düğmeye tıklama çevirir ama odağı/imleci taşımaz (halka yalnız klavyede); metne tıklama düğme odağını bırakır.
    // Gösterilen parola HÂLÂ parola kipidir: text()/selection() "", Copy/Cut olaysız, IME'ye çevre metin ve preedit yok, tampon kilitli; yalnızca çizim düz metin. clear() parolayı yeniden gizler.
SecretBuffer::with_capacity(n); replace_range(r, s) -> bool; set(s) -> bool; clear(); as_str()   // Clone yok, Debug kırpılmış, Drop'ta zeroize
SecretBuffer::to_zeroizing(&self) -> nuxut_ui::Zeroizing<Vec<u8>>   // tek kopya, Vec::with_capacity(len) (kapasite == uzunluk; ara String/yeniden ayırma yok), düşünce sıfırlanır.
    // Kilitli DEĞİL (sıradan yığın; takas/döküm mümkün): çağıran kısa tutar, başka tipe kopyalamaz, hemen düşürür (ör. PAM/D-Bus'a verip bırak).
// Sayfa yığını — panel içi alt sayfalar; kap boyutu `spatial` yayla morf, içerik Menu ile aynı kayma/sönme (grow::Slide).
PageStack::<K>::new(theme, root, size); push(page, size, now); pop(size, now) -> Option<K>; set_size(size, now); current(); depth();
    current_size(now); draw(ui, clip, |ui, &K, origin| …)   // geçişte iki kez çağrılır (eski + yeni); içerik `origin`e yerleşir
PageHeader::new(theme, title); set_title(s); pointer(ev, now) / key(ev, now) /*← Backspace*/ -> Response   // "‹ Başlık" geri satırı
// Hızlı ayar kutucuğu — açık: primary/on_primary, kapalı: tonal; "›" alanı alt sayfa ister; basınca köşeler sıkışır.
Tile::new(theme, TileItem::new(icon, label).subtitle(s).on(bool).expandable(bool)); set_on(bool, now); set_subtitle(theme, Option<&str>); set_icon(i);
    set_rect(theme, r); pointer(theme, ev, now) / key(ev, now) /*Enter Space; → genişlet*/ -> Response<TileEvent::{Toggled(bool), Expand}>   // yükseklik Tile::height = space(16)
TileGrid::new(theme, Vec<TileItem>); layout(theme, origin, width) -> h /*2–4 sütun: columns_for*/; size_for(theme, n, width); pointer -> Response<(usize, TileEvent)>; tiles_mut()
// OSD kapsülü — kenardan blob olarak büyür (grow); değer spatial_fast; gizleme zamanlayıcısı çağıranın.
Osd::new(theme); show(theme, icon, value, MenuAnchor, bounds, now) /*açıkken yalnız günceller*/; hide(now); shape(now, theme) -> Option<BlobShape>;
    draw_content(ui) /*şekil shell'in BlobField'ında*/; draw(ui) /*bağımsız*/   // ikon: Icon::named(IconName::volume(%, muted) | IconName::brightness(%)); sağda hedef yüzde (tabular)
    set_max(max, now) /*1..=OSD_MAX_OVERFLOW (2.0), varsayılan 1; value 0..=max; açıkken ölçek yayla akar, değer kırpılır*/; max()
    set_value_text(Option<String>) -> bool /*Some: yüzde yerine bu metin (büyüteç "2,0×"; ≤ OSD_MAX_VALUE_TEXT = 32 bayt), sonraki show'larda kalır; ray yine value; etiket alanı max("100", metin) genişliği; None: yüzde (varsayılan). 2026-10-16, widgets/kullanıcı geri bildirimi*/; value_text()
    set_muted(bool, now) /*içerik DISABLED_CONTENT opaklığına effects hareketiyle soluklaşır (kesilebilir; hareket kapalıyken ya da kapsül görünmezken anında); show'dan önce çağrılırsa sessiz açılış ilk karesinden soluk; değişmezse kare yok; kapsül şekli solmaz*/; is_muted()
    // Aşım (max > 1): ray 0..max, %100'de dolgunun arkasında, raydan taşan ince outline işareti; %100 üstü dolgu error tonunda (effects yayıyla belirir), tek kapsül; yüzde gerçek değer (135).
// Sonuç listesi (başlatıcı) — sanal: yalnız görünen satırlar; LRU layout önbelleği (RESULT_CACHE_ROWS = 96); eşleşme kalın + primary.
ResultList::new(theme); set_items(theme, Vec<ResultItem::new(title).subtitle(s).icon(i).matches(Vec<Range<usize>>) /*bayt*/ | .match_chars(&[u32]) /*nucleo karakter indeksleri*/>, now); set_rect(theme, r, now);
    pointer(theme, ev, now) / key(theme, ev, now) /*↑↓ PgUp PgDn Home End Enter →*/ -> Response<ResultEvent::{Selected(usize), Activated(usize), Actions(usize)}>; selected(); shaped_rows()
// Glif ızgarası (emoji/sembol seçici) — sanal: yalnız görünen satırlar şekillenir/çizilir; glif LRU (GLYPH_CACHE_CELLS = 512, en az 2× görünen). Tasarım: docs/design/glyph-grid.md.
GlyphGrid::new(theme); set_items(theme, Vec<GlyphItem::new(glyph, label /*erişilebilir ad*/)>, now) | set_sections(theme, Vec<GlyphSection::new(title /*"" = başlıksız*/, items)>, now);
    set_columns(theme, GlyphColumns::{Auto /*cell_size = space(11) kare, ortalı*/, Fixed(n)}, now); set_rect(theme, r, now); set_theme(theme, now); set_focus_visible(bool) -> bool;
    pointer(theme, ev, now) / key(ev, now) /*←→ doğrusal; ↑↓ aynı sütun (kısa satırda kırpılır, yapışkan); PgUp PgDn; Home End satır, Ctrl+Home/End uçlar; Enter*/ -> Response<GlyphGridEvent::{Selected(usize), Activated(usize)}>;
    set_selected(i, now) /*olaysız; yerleşimden önceyse ilk set_rect'te animasyonsuz*/; reveal_section(s, now) -> bool /*başlık tepeye, seçim bölüm başına*/; selected(); item(i); label(i);
    replace_sections(theme, sections, selected, now) /*aynı görünümün verisi yenilendi (dil, kayıt): kaydırma konumu ve süren hareketi korunur (yalnız yeni içerik yüksekliğine sınırlanır), seçim olaysız ve kaydırmasız 'selected'a (taşarsa son öğe); set_sections ise yeni liste: seçim ve kaydırma başa. 2026-10-15, power-emoji isteği: ofset get/set yerine — süren yay kesilmez, tek çağrı*/
    section_of(i); section_start(s); section_count(); columns(); len(); cursor(p); draw(ui); shaped_cells()   // sınırlar MAX_GLYPH_ITEMS 65536, MAX_GLYPH_TEXT 64, MAX_GLYPH_LABEL 256; Debug redakte
// Yol çubuğu (dosya seçici) — parçalar düğme, ChevronRight ayraç, sığmayınca baştan "…" ile katlanır. Tasarım: docs/design/path-bar.md.
PathBar::new(theme, Vec<PathSegment::new(label).with_icon(i)>); set_segments(theme, v); set_rect(r); layout(text, theme) /*draw da çağırır*/; preferred_size(theme, w) /*h = space(10)*/;
    pointer(text, theme, ev, now) / key(ev, now) /*←→ Home End Enter Space*/ -> Response<PathBarEvent::{Navigate(usize) /*"…" = first_visible()-1*/, Edit(TextInputEvent)}>;
    begin_edit(theme, text, now) /*Ctrl+L'yi uygulama yakalar*/; end_edit(); is_editing(); edit_text(); text_input_mut(); first_visible(); segment_rect(i); segment_label(i); set_focus_visible(b); cursor(p); draw(ui)
    access(tree, key, focused, more_label) /*Navigation; parçalar Button, geçerli selected; düzenlemede giriş*/; access_action(container, req) -> Response<PathBarEvent>
// Dosya ızgarası — sanal (eşit satırlar), ikon/küçük resim + 2 satır etiket, tekli/çoklu seçim. Tasarım: docs/design/file-grid.md.
FileGrid::new(theme, multiple); set_items(Vec<FileItem::new(label, icon)>, now); set_icon(i, icon); set_thumbnail(i, Option<Texture>) /*yüklenene dek ikon*/; set_rect(theme, r, now); set_theme(theme, now);
    pointer(theme, ev, modifiers /*App::modifiers_changed*/, now) / key(ev, now) /*oklar PgUp PgDn Home End Enter; Shift aralık, Ctrl imleç, Ctrl+Space, Ctrl+A*/ -> Response<FileGridEvent::{SelectionChanged, Activated(usize)}>;
    selection() -> &Selection; cursor(); select_only(i, now); clear_selection(); set_multiple(b); visible_range(now) /*küçük resim isteği*/; columns(); set_focus_visible(b); cursor_shape(p); draw(ui);
    access(tree, key, focused, now, item_key) /*ListBox(+multiselectable) / ListBoxOption*/; access_action(container, req, item_key, now)   // MAX_FILE_ITEMS 2^20, MAX_FILE_LABEL 1024; Debug redakte
// Çoklu seçim modeli (FileGrid, List): bit kümesi; tık/Ctrl/Shift/Ctrl+Shift, ok+Shift/Ctrl, Ctrl+Space, Ctrl+A. Dönüş: seçim kümesi değişti mi (imleç ayrı okunur).
Selection::new(multiple); reset(len); click(i, Modifiers) -> bool; navigate(i, Modifiers) -> bool; toggle_cursor(); select_all(); select_only(i); clear(); set_multiple(b);
    is_selected(i); iter() /*artan*/; count(); cursor(); anchor(); len()
IconView::with_size(theme, px) /*rol boyu yerine sabit kutu (dosya ızgarası ikonu); tema değişince korunur*/
// Cihaz satırı (Wi-Fi/Bluetooth) — sinyal yüzdesi ya da pil; Connecting → Spinner; şifreli yeni ağda satır içi parola (yükseklik yayla açılır).
DeviceRow::new(theme, name, DeviceKind::{Wifi { strength /*0–100*/, secured }, Bluetooth { battery }}, DeviceState::{Available, Saved, Connecting, Connected, Unavailable});
    set_state(s, now); set_kind(k, now); cancel_password(now) /*seçim değişti, hata, panel kapandı: tampon sıfırlanır*/; set_status_text(s); set_password_placeholder(s); height(theme, now); target_height(theme); set_rect(text, theme, r);
    pointer(text, theme, ev, now) / key(ev, now) -> Response<DeviceEvent::{Connect, Disconnect, Forget, Password(Zeroizing<String>)}>   // ikonlar IconName::wifi/battery; durum yazısı tabular rakamlı
DeviceRow::with_icon(Icon) / set_icon(Option<Icon>) -> bool /*None: türün varsayılanı (Wi-Fi sinyal kademesi, Bluetooth IconName::Bluetooth); özel ikon varken set_kind ikonu değiştirmez; bağlıyken dolu*/; icon() -> &Icon
    // Bluetooth cihaz türleri: IconName::{Headphones, Speaker, Keyboard, Mouse, Phone, Watch, Gamepad, Tablet} (istek, 2026-10-14). Varsayılan API ve görünüş değişmedi.
    // Ekran klavyesi tuşları: IconName::{Backspace, KeyboardReturn, Shift, ShiftLock, KeyboardCapslock, KeyboardTab, KeyboardHide, ArrowUpward, ArrowDownward} (osk isteği, 2026-10-15). Nerd'de Shift dolu = nf-md-arrow_up_bold (etkin Shift); KeyboardHide = nf-md-keyboard_close (Material keyboard_hide gibi aşağı oklu klavye).
// ResultList ekleri: satır kendi ailesiyle çizilebilir; seçim olaysız ayarlanabilir.
ResultItem::subtitle_font(family) /*alt metin (örnek) bu ailede; başlık temada, okunur kalır*/; ResultList::set_selected(theme, i, now) /*olay yok, görünür kılar; yerleşimden önceyse saklanır, ilk set_rect'te animasyonsuz*/; set_item_icon(i, Option<Icon>) /*yalnız o satırın önbelleği düşer*/
ResultItem::trailing(s) /*sağ metin yuvası (kısayol, kategori, saat · boyut): sağa hizalı, Caption + on_surface_variant, tabular rakam, tek satır; doğal genişliği en fazla satır metin alanının TRAILING_MAX_FRACTION (0,4) kadarı, aşan üç nokta. Başlık/alt metin kalan alanda (− space(3)) üç noktayla kırpılır; yuvasız satırın yerleşimi değişmez. Debug'da yalnız uzunluk*/
ResultItem::trailing_pill(s) /*aynı yuva Pill olarak (pub trailing_pill: bool); hapın tamamı TRAILING_MAX_FRACTION sınırında, seçili satırda surface zemin; düz trailing davranışı değişmedi (talep, 2026-10-13)*/
// Font seçici — gömülü ya da Popover içinde (preferred_size). Veri `TextSystem::families()`; ikinci tarama yok.
// Her satır: aile adı (tema fontunda, simge fontlarında da okunur) + örnek metin O AİLENİN fontuyla (aile adı veriden; temadan değil). Liste ResultList: sanal, LRU layout önbelleği.
// Seçili aile ✓ ile işaretli ve görünür kaydırılır; ↑↓ gezinir, Enter/tıklama seçer. Arama: büyük/küçük harf duyarsız alt dizi, eşleşen harf vurgulu.
// Ağırlık: statik ailede yalnız bulunan ağırlıklar (Choice çipleri, ad `labels.weight_names`); değişken ailede (`weight_range`) aralıkta
// Slider, 100'lük adımlar. `latin == false` aileler (sembol/Latin dışı) listelenmez. Filtre: SegmentedButton [Tümü, Eş aralıklı]. Önizleme: seçili aile+ağırlıkta tek satır. Boşta kare istemez.
FontPicker::new(theme, &[FamilyInfo], FontPickerLabels { search, all, monospace, sample, weight_names: [String; 9] /*100..900*/ }, selected: Option<(&str, u16)>);
    .with_filter(theme, bool) /*false: "Tümü / Eş aralıklı" gizli, yer kaplamaz, tüm aileler — ör. ikon fontu rolü*/
    set_families(theme, &[FamilyInfo], now); set_selected(theme, family, weight, now) /*olay yok*/; selected() -> Option<(&str, u16)>;
    preferred_size(theme) -> Size; set_rect(text, theme, r, now); search_mut() /*IME/yapıştırma tesisatı*/;
    pointer(text, theme, ev, now) / key(text, theme, ev, now) /*arama odakta: ↑↓ PgUp PgDn Enter listeye*/ -> Response<FontEvent::Changed { family: String, weight: u16 }>;
    cursor(p); draw(ui)   // aile değişince ağırlık eldekine en yakın bulunan ağırlığa oturur


```

#### TextInput parola kipi: güvenlik garantileri (kilit ekranı)

`TextInput::new(theme).password(true)` — metin her kipte `SecretBuffer`'da tutulur; parola kipinde:

- **Sabit kapasite, yeniden tahsis yok:** tampon `PASSWORD_MAX_BYTES` (1024) kapasiteyle bir kez
  ayrılır, hiç büyümez; sığmayan yazma/yapıştırma bütünüyle reddedilir. `with_max_bytes` yeni tampon
  ayırırsa eskisi düşerken sıfırlanır.
- **Silinen veri sıfırlanır:** silme/kesme tampon içinde kaydırılır, boşalan baytlar sıfırlanır;
  `Drop`'ta tüm tampon `zeroize` ile (derleyicinin silemeyeceği yazım).
- **Kopya yok:** `SecretBuffer` ve `TextInput` `Clone` değil; `Debug` yalnızca uzunluk yazar.
  `text()` ve `selection()` parolada boş döner; sır yalnızca ödünç `with_secret(|s| …)` ile.
- **Görüntü:** gizliyken layout'a yalnızca `•` maskesi girer (gerçek metin hiç şekillendirilmez). Görüntü
  dizgisi yenilenirken ve `TextInput` düşerken eskisi sıfırlanır (parola gösterilmişse düz metin içerebilir).
- **Göster/gizle (2026-10-14, istek):** `with_reveal_toggle` / `set_revealed`
  yalnızca çizimi değiştirir; alan parola kipinde kalır (yukarıdaki pano/IME/tampon garantileri aynen).
  Bir kez `password(true)` olmuş alanın düz metin layout'u `TextSystem::layout_secret` ile şekillenir:
  birleşik ara dizgi oluşmaz, cosmic-text satır dizgisi (`BufferLine`, metnin cosmic-text içindeki tek
  kopyası) dönmeden `zeroize` edilir, şekillendirme karalama tamponu (harfrust girdisi, kelime/glif
  vektörleri) aynı karakter sayısında dolgu metniyle yeniden şekillendirilerek ezilir (en iyi çaba).
  Layout bırakılırken (her düzenleme, gizleme, `password(false)`, `Drop`) `TextLayout::wipe` glif
  kimliklerini, bayt aralıklarını ve konumları sıfırlar. Görüntü dizgisi (gösterilmişse düz parola) de gizleme,
  `clear()`, `set_text`, her düzenleme ve IME değişiminde çizim beklenmeden `zeroize` edilir (gizlenip hiç
  çizilmeyen alanda düz kopya yığında kalmaz; inceleme). Gizleme layout'u hemen bırakır (sonraki kareyi
  beklemez). `clear()` ve düğmenin kapatılması parolayı yeniden gizler. Önceki bilinen açık
  ("`password(false)` ile gösterilen parolanın layout'u sıfırlanmadan düşer") bu yolla kapandı; kalan
  kopyalar aşağıda "Sınırlar"da.
- **Pano:** Copy/Cut parolada olay üretmez. Paste serbest; ara temizleme kopyası `Zeroizing`.
- **IME:** `ime_state()` parolada çevre metni boş verir (`password: true` hassas ipucu);
  `ime_preedit` parolada **yok sayılır** (kopya tutulmaz, varsa eskisi sıfırlanarak kalkar;
  `password(true)` da bırakır). Commit doğrudan `SecretBuffer`'a yazılır.
- **Kilitli bellek (`SecretBuffer::locked`):** parola tamponu yığında değil, kendi anonim `mmap`
  eşlemesindedir (`memmap2`, güvenli API; nuxut-ui `forbid(unsafe_code)`): sayfa hizalı, başka
  hiçbir ayırmayla sayfa paylaşmaz (iki tampon aynı sayfada olamaz). Unix'te `mlock` (takasa
  yazılmaz), Linux'ta `MADV_DONTDUMP` (çekirdek dökümüne girmez). `password(true)` tamponu buraya
  taşır (eski yığın tamponu sıfırlanarak düşer); parola kipinde `with_max_bytes` de kilitli ayırır.
  `Drop` sırası: tampon `zeroize` → eşlemenin tamamı (sayfa yuvarlaması dahil) `zeroize` →
  `munlock` → `munmap`; eşleme kendine ait olduğundan `munlock` başka kilitli tamponu çözmez.
  `mlock` başarısızsa (`RLIMIT_MEMLOCK` — systemd varsayılanı 8 MiB, tampon 1 sayfa —, `ENOMEM`)
  ya da eşleme açılamazsa (yığına düşer) alan aynen çalışır; `tracing::warn!` süreç başına
  **yalnızca bir kez**, yalnızca errno/tür (içerik asla). Durum: `is_isolated()`, `is_locked()`.
  Testli: sayfa hizası ve ayrık sayfalar, smaps `VmFlags` `lo`/`dd`, `VmLck` artar ve `Drop`
  sonrası geri iner (alt süreçte), gerçek `RLIMIT_MEMLOCK = 0` alt sürecinde alan çalışır ve
  uyarı tek, enjekte edilen `mlock` hatası.
- **Geri alma geçmişi yok.**

#### Gizli metin girişi (polkit ajanı, kilit ekranı)

Bir tuşun ya da IME commit'inin baytları, uygulamanın `SecretBuffer`'ına girene kadar şu yolu izler;
bizim elimizdeki her ara kopya sıfırlanır (testli):

1. **Klavye (nuxut-wayland `state.rs`):** sctk'nin `KeyEvent.utf8` dizgisi kopyalanmadan
   `SecretText`'e taşınır — odak yok / bırakma / kontrol karakteri dahil her dönüş yolunda düşerken
   sıfırlanır. `KeyEvent.text: Option<SecretText>`; `NamedKeysym` (keysym adı — yığında `String` — ve değeri)
   `Drop`'ta sıfırlanır. `XkbKey::keycode` (değerle taşınan `u32`, yığıtta) sıfırlanmaz: `XkbKey`'ye
   `Drop` eklemek tüketicinin yapı güncellemesi (`..xk`) kalıbını kırar. `KeyEvent` olay kuyruğunda tutulmaz (uygulamaya hemen, değerle
   gider); `Clone` edilen her kopya kendi düşüşünde sıfırlanır.
2. **Keymap'siz tuşlar (`keyfocus.rs`):** keymap gelmeden biriken ham tuş kodları (`RawKey`)
   bırakılırken (odak değişimi, yeniden oynatma, sıfırlama) yerinde sıfırlanır; yeniden oynatılan
   kopya `Zeroizing<Vec<RawKey>>`.
3. **IME (`ime.rs`):** `preedit_string`/`commit_string` dizgileri protokolden gelir gelmez
   `SecretText`'e taşınır; 4096 bayt kesimi atılan kuyruğu da sıfırlar (`truncate_zeroized`).
   Odaktaki yüzeyin durumu `TextPurpose::Password` ise compositor'a `content_type(purpose =
   password, hint = sensitive_data | hidden_text)` gönderilir, **çevre metin hiç gönderilmez**,
   gelen preedit saklanmadan düşer (alan gizliye dönmeden gösterilmiş preedit boş preedit ile
   temizlenir). Bekleyen (`done` gelmemiş) değişiklikler leave'de, `ImeEvent`'ler tüketilince ya da
   odak/izin yoksa atılırken sıfırlanır.
4. **Bileşen (`TextInput`):** yukarıdaki parola kipi garantileri.

`SecretText` (`nuxut_core`): `String` sarmalayıcı; `Deref<Target = str>` ve dönüşümler sayesinde
tüketici kaynağı değişmeden derlenir (`text.as_deref()`, `&text`, `push_str(&text)`,
`ime_commit(&text)`, `KeyEvent::new(…, Option<String>, …)`, `text: None`). Kırılan tek kalıp
yapı değişmezinde `text: Some(String)` yazmaktır (`Some(c.into())` / `Some(s.into())` olmalı) ve
`tracing` alanı olarak doğrudan vermek (`Value` değil — bilerek: içerik loglanmaz).

**Sınırlar — elimizde olmayan kopyalar (garanti dışı):**

- **sctk 0.21 klavyesi:** `press_key`'e `KeyEvent`'in klonunu verir; özgün değer (ve tuş tekrarı
  için `current_repeat`'te tuttuğu kopya, bırakma/leave'e kadar) sctk içinde sıfırlanmadan düşer.
- **xkbcommon:** `key_get_utf8` 64 baytlık yığıt (stack) tamponu ve `xkb_state`/compose iç durumu;
  libxkbcommon C tarafı. Bunları kapatmak sctk klavyesinin yerine kendi `wl_keyboard` + xkb
  yolumuzu (utf32; yığın ayırması yok) gerektirir — ayrı iş.
- **Wayland soket tamponları:** wayland-backend'in okuma tamponu ve ayrıştırmada üretilen ara
  `CString`'ler (tuş kodları, `commit_string`) ile libwayland; çekirdek soket tamponları.
- **Compositor ve IME süreçleri** (Hyprland, fcitx5…): kendi kopyaları; hassas ipucu yalnızca rica.
- **GPU:** maske glifleri (`•`) atlasta/çizim tamponlarında — yalnızca uzunluğu açığa çıkarır.
  Gösterilen parolanın glifleri atlasta kalır (karakter KÜMESİNİ açığa çıkarır, sırayı değil) ve çizim
  tamponlarındadır.
- **cosmic-text içi (gösterilen parola):** `BufferLine`'ın şekil/yerleşim önbelleği (`ShapeLine`,
  `LayoutLine`: glif kimlikleri + kümeler — font bilinirse metne çevrilebilir) satır dizgisi alınırken
  sıfırlanmadan düşer (cosmic-text 0.19 değiştirilebilir erişim vermiyor); karalama tamponunun ezilmesi
  dolgu metninin aynı yolu izlemesine dayanır, garanti değildir.
- **Takas alanı / çekirdek dökümü:** yalnızca `SecretBuffer` (parola kipi) kilitli ve dökümsüzdür;
  `SecretText` (`KeyEvent::text`, IME metni), sctk/xkbcommon/wayland-backend kopyaları ve
  `with_secret` dışına çıkan kopyalar sıradan yığında — takasa/dökümüne girebilir. `mlock` reddedilirse
  (`RLIMIT_MEMLOCK`) parola tamponu da takasa yazılabilir (dökümsüz kalır). `fork` edilen çocuk
  eşlemeyi `exec`'e dek kopya-yazımda paylaşır (`MADV_WIPEONFORK` memmap2'de `unsafe`; yok).
- **Uygulama:** `with_secret` kapanışının dışarı kopyaladıkları (PAM'e verilen dizgi vb.) ve
  `SecretText`'ten `to_owned()` ile alınan kopyalar çağıranın sorumluluğu (`Zeroizing` kullanın).
- Sıfırlama serbest bırakmadan öncedir; serbest bırakılmış bellek testlerde okunmaz (`unsafe` yok):
  testler ayrılmış tamponun (`SecretBuffer`, `RawKey` kuyruğu) içeriği dışındaki baytların sıfır
  olduğunu ve gizli alanda preedit'in hiç saklanmadığını doğrular.

## Erişilebilirlik — ekran okuyucu (ADR-0011)

Karar ve sınırlar: `docs/decisions/0011-screen-reader-accesskit.md`; araştırma `docs/design/accessibility.md` §4.

**Katmanlar.** `nuxut-access` (koşulsuz; cephe `nuxut::access`, ui'de `nuxut_ui::access`): `AccessTree`,
`AccessNode` (zincirlenen düzenleyici), `AccessRequest`/`AccessAction`, `child_id`/`root_child`, `ROOT_ID`.
`nuxut-wayland` `a11y` özelliği (cephe `nuxut/a11y`, kapalı varsayılan): yüzey başına `accesskit_unix`
adaptörü. API özellikten bağımsız hep vardır; özellik kapalıyken `App::accessibility*` hiç çağrılmaz.

```rust
// App (varsayılanları boş):
fn accessibility(&mut self, cx: &mut Context<Self>, surface: SurfaceId, tree: &mut AccessTree) {}
fn accessibility_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: AccessRequest) {}
// Context:
pub fn a11y_active(&self, surface: SurfaceId) -> bool;   // AT bu yüzeyin ağacını yayımlıyor mu
pub fn a11y_invalidate(&mut self, surface: SurfaceId);   // çizimsiz değişim; tur sonunda yeniden kur
pub fn set_a11y_suspended(&mut self, suspended: bool);  // kilitliyken ZORUNLU: ağaç içeriksiz, eylem/odak atılır (ADR-0011 tehdit modeli)
pub fn a11y_suspended(&self) -> bool;
pub fn a11y_exclude(&mut self, surface: SurfaceId);  // içeriksiz yardımcı yüzey (dışlama şeridi, duvar kağıdı) ağaçta boş pencere olmasın
pub fn set_a11y_lock_surfaces(&mut self, on: bool);  // kilit yüzeyleri ağaç yayımlasın (varsayılan kapalı; lock_session'dan önce; ADR-0011 madde 8: AT yalnız okur/odaklar, açma ve parola gerçek girdiyle)

// nuxut-access
AccessTree::new(); root() -> AccessNode /*Role::Window; .label(t!(..))*/; push(key, Role) -> AccessNode; nodes() /*(NodeId, &Node)*/;
    within(parent_id, |tree| ..) /*çocuklar*/; get(id); id_for(key); set_focus(id); unlabeled() -> Vec<NodeId>
    /*adı olması gereken adsız düğümler; testlerde boş olmalı*/; duplicates(); finish() -> TreeUpdate
AccessNode: id(); label(&str); label_or; description; value; numeric(v, min, max, step); toggled; selected;
    expanded; enabled(bool); bounds(Rect /*yüzeye göre mantıksal*/); action(Action); clickable() /*Click+Focus*/;
    live(Live); shortcut; position(i, n); labelled_by(id); active_descendant(id); focused(); focused_if(bool);
    text_content(text, caret, anchor, mask) /*tek TextRun + seçim; mask → • × grafem*/; node_mut()
child_id(parent, key) -> NodeId   // (ebeveyn, anahtar) karması; kurulumla aynı → eylem eşleme
clip(s, max_bytes) / preview(s, graphemes) / MAX_ACCESS_TEXT (512) / PREVIEW_GRAPHEMES (40)  // ad/açıklama/değer sınırı; pano önizlemesi
AccessRequest { node: NodeId, action: AccessAction::{Click, Focus, Increment, Decrement, SetValue(f64), Expand, Collapse, ScrollIntoView} }
    // from_accesskit: metin SetValue / ReplaceSelectedText / diğerleri → None (hiç iletilmez)
```

**Runtime akışı.** at-spi2-core yoksa (`org.a11y.Bus` hizmet dosyası `XDG_DATA_DIRS`'te değil) ya da
`NUXUT_A11Y=0` ise adaptör kurulmaz. Layer ve pencere yüzeyi yaratılınca adaptör (kilit yüzeyine
kurulmaz — MVP dışı). AccessKit işleyicileri (başka thread) calloop kanalına yazar: `Activate` →
yüzey etkin + kirli, `Deactivate`, `Action` → `App::accessibility_action`. Her olay turunun sonunda,
çizimlerden SONRA, etkin yüzey için ağaç kurulur: yüzey izinliyse (`!askıda && input_allowed`; değilse ağaç içeriksiz, eylem atılır, odak bildirilmez), kirliyse, ya da bu turda çizildiyse (animasyon
sürüyorsa en çok 100 ms'de bir; son kare her zaman). Yüzey en az bir kare çizmeden kurulmaz. Kök
sınırı: layer yüzeyinde çıkış konumu + anchor + margin + boyut (diğer yüzeylerin dışlama bölgesi
yok sayılır), pencerede (0,0). Klavye odağı `update_window_focus_state`. Yok edilen yüzeyin
adaptörü tur sonunda düşer.

**Bileşen eşlemesi** (`access(tree, key, ..) -> AccessNode`; ad görünmüyorsa çağıran `.label(t!(..))`):

| Bileşen | Rol / durum | Eylem yardımcısı |
|---|---|---|
| `Button` | Button, ad etiketten | çağıran tıklama yolunu çalıştırır |
| `IconButton` | Button (+toggled), **ad zorunlu** | 〃 |
| `Toggle` | Switch (AT-SPI ToggleButton, `pressed`) | `access_action(action, now) -> Response<bool>` |
| `Slider` | Slider, `range`'e eşlenmiş değer/adım | `access_action(action, range, now) -> Response<f32>` |
| `SegmentedButton` | RadioGroup → RadioButton (işaretli = seçili); `focused` → seçili seçenek odak | `access_action(container, req, now)`, `segment_id` |
| `List`/`ListItem` | List → görünen satırlar + odaktaki; anahtarlı satır Switch, diğerleri Button; grup başlığı Heading; alt metin + sağ değer açıklama; `position` | `item_key(i)` kalıcı kimlik |
| `ResultList` | ListBox → ListBoxOption (`selected`, konum); arama alanı `.active_descendant(access_selected(..))` | `access_action(theme, container, req, item_key, now) -> Response<ResultEvent>` |
| `TextInput` | TextInput / PasswordInput (yalnız maske, gösterilirken de); TextRun + imleç; yer tutucu; göster/gizle çocuk Button | `access_action(input, req) -> bool` (göster/gizle) |
| `Label` | Label (metin değer) | — |
| `IconView` | Image, yalnız ad (anlamlı ikon; süs ikonu ağaca girmez) | — |
| `NotificationCard` | Group (tıklanabilir): ad başlık, açıklama uygulama/zaman/gövde; eylem + kapat düğmeleri; açılır kartta `.live(Live::Polite)` | `access_action(card, req, now) -> Option<NotificationEvent>` |
| `EmptyState` | Group: ad başlık, açıklama gövde, eylem düğmesi; maskot/ikon yok | `is_access_action(state, req)` |
| `Tile`/`TileGrid` | Switch (ToggleButton, `pressed`): ad etiket, açıklama alt metin; alt sayfalıysa Expand eylemi; ızgara Group | `Tile::access_action(action, now) -> Response<TileEvent>` |
| `Select` | ComboBox: değer seçili seçenek, `expanded`; açıkken seçenekler ListBoxOption | Click menüyü açar (tıklama yolu), seçim klavyeyle |
| `PathBar` | Navigation (WAI-ARIA kırıntı kalıbı; ad çağıranın) → parçalar Button (geçerli klasör `selected`), görünürse `…` Button (`more_label`); düzenleme kipinde TextInput | `access_action(container, req) -> Response<PathBarEvent>` |
| `FileGrid` | ListBox (çoklu kipte `multiselectable`) → görünen hücreler + imleç ListBoxOption (`selected`, konum); `focused` → imleç odak | `access_action(container, req, item_key, now) -> Response<FileGridEvent>`; `item_key(i)` dosya adı karması |
| `Dialog` | Dialog (`modal`; ad başlık) → gövde (çağıranın `content` kapanışı) + eylem düğmeleri `("action", i)`; kapalıyken ağaçta yok | `access_action(container, req, now) -> Response<DialogEvent>` (tıklamayla aynı) |

**Test.** D-Bus'sız: `nuxut-access` birim + `nuxut-ui/src/access_tests.rs` (`accesskit_consumer::Tree`,
geçersiz ağaçta panik). Uçtan uca: `tools/a11y-e2e.sh <komut>` (headless sway + `dbus-run-session` +
at-spi-bus-launcher etkinleştirmesi + elle registryd; kullanıcının bus'larına dokunmaz), içinde
`crates/nuxut/tests/a11y_atspi.rs` (`--features a11y`, `NUXUT_A11Y_E2E=1` yoksa "atlandı" yazar) ve
`examples/atspi_dump.rs` (ağacı yazdırır, `--expect AD`).

## Değişmezler (invariant)

1. **Boşta sıfır iş.** Animasyon yoksa kare istenmez, timer yalnızca gerektiğinde kurulur
   (saat bir sonraki dakika/saniye sınırına hizalanır). `top`'ta 0.0% CPU görülmeli.
2. **Kare saatini dışarı verme.** Bir karedeki tüm animasyonlar aynı `now`'u kullanır.
3. **Yaylar kesilebilir.** Mekânsal hareket `Animated<T>` + `Spring`; hedef değişince konum ve
   hız korunur. Süreli tween yalnızca opaklık/renk gibi efektler için.
4. **Çizimde yapı değişmez.** `draw` içinde yüzey yaratılmaz/yok edilmez, input bölgesi
   değiştirilmez; bunlar olay işleyicilerinde yapılır.
5. **Piksel hizalama.** Dikdörtgen kenarları fiziksel piksele yuvarlanır; metin konumu
   yuvarlanmaz (glyph atlas alt-piksel konumlandırma yapar). Fractional scale desteklenir
   (`wp_fractional_scale_v1` + `wp_viewporter`, buffer scale 1).
6. **Premultiplied alpha** her yerde (blend `ONE, ONE_MINUS_SRC_ALPHA`).
7. **Hata yalıtımı.** Config hatası, IPC kopması, font eksikliği çalışmayı durdurmaz; son
   geçerli durumla devam edilir ve loglanır. Panic yalnızca programcı hatasıdır.
8. **Güvenlik.** Dış girdi (config, IPC, D-Bus) boyut sınırlıdır ve doğrulanır; shell komut
   çalıştırmaz, kullanıcı config'inden gelen komutlar ileride açıkça işaretli tek bir yoldan
   geçer. Güvenilmeyen kaynaktan gelen kodlanmış görsel shell'in adres alanında ayrıştırılmaz
   (ADR-0007; yalıtılmış, tek kullanımlık işçi; kilit beyanı el sıkışmada zorunlu; `Unavailable`'da
   yalnızca kapılardan geçmiş temel raster, ilk kare süreç içine düşer).
9. **Loglarda kullanıcı içeriği yok.** Metin girdisi, IME preedit/commit, pano, dosya yolları,
   URL'ler, görsel yolları, config değerlerindeki kişisel veri ve bildirim/pencere metni hiçbir
   log seviyesinde yazılmaz; yalnızca tür, uzunluk, sayı, hata türü loglanır. Girdi tiplerinin
   `Debug`'ı içeriği gizler (`KeyEvent.text` → `<n karakter gizli>`, `Key::Char` → `Char(…)`, `XkbKey` → `XkbKey(…)`;
   `ImeEvent`, `SurroundingText`, `TextInput`, `SecretBuffer`, `SecretText` aynı kural). `nuxut-ui`'de içerik
   taşıyan her tip `Debug`'ı elle yazar ve metin yerine `<N bayt>` / öğe sayısı verir
   (`crate::redact`): `Label`, `TextInputEvent::{Copy, Cut}`, `ImeState`, `NotificationContent`,
   `BodySpan`, `NotificationEvent::Link`, `MenuItem`, `ListTrailing::Text`, `Select`, `Avatar`,
   `ResultItem`, `TileItem`, `DeviceEvent::Password`; bunları içeren bileşenler böylece güvenli
   kalır. Yeni içerik tipi aynı kalıbı izler (`redact::assert_redacted` testiyle). `?event` ile olay
   loglamak yalnızca bu redakte Debug'larla yapılır; testler Debug çıktısında metnin görünmediğini
   doğrular.

## Doğrulama

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Görsel: `examples/` altındaki demo canlı Hyprland'de çalıştırılır, `grim` ile ekran görüntüsü
  alınıp incelenir. Kendi PID'ini öldür; `pkill -f` ile geniş öldürme yapma (kullanıcının
  canlı oturumu var).
- Bellek: `ps -o rss` ile ölçülür; hedef monitör başına onlarca MB, yüzlerce değil.
- Yük altında kararlı testler (görev, 2026-10-14): test süreci
  aç kalabilir ya da bellek bekçisince (`nuxut-mem-watch`) saniyelerce dondurulabilir. Bu
  yüzden (1) duvar saati sınırı yalnızca güvenlik ağıdır: işlevi sınayan testte geniş, test'e
  özgü süre verilir (ürün sabiti/`SandboxConfig::wallpaper()` sınırları değişmez; ör.
  `SystemBackend::with_test_timeouts`, testlerin `wp_cfg()`/`cfg()`'si); (2) koşul uyku ile
  değil, durumu yoklayarak (`eventually`, sınır 60 sn) beklenir; (3) "X sürede bitti" üst
  sınırı yerine sayaç/durum iddiası kullanılır (ör. bütçe → okuma sayısı, negatif önbellek →
  `Killed` + `negative_hits`); (4) kısa zaman aşımı gereken testte meşru istek `Timeout`
  alırsa yeniden denenir, başka her hata düşürür. Sınama: testleri kendi `systemd-run --user
  --scope -p CPUQuota=…` kapsamında çalıştırıp kapsamı `systemctl --user freeze/thaw` ile
  aralıklarla dondurmak bekçiyi taklit eder.
