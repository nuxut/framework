# ADR-0003 — Platform kapsamı: framework cross-platform, shell yalnızca Linux/Hyprland

Tarih: 2026-10-04 · Durum: kabul edildi (kullanıcı kararı; aynı gün netleştirildi)

## Karar
- **nuxut-shell yalnızca Linux/Wayland (Hyprland) içindir.** Hyprland'e özgü davranışlara bağımlı olabilir.
- **nuxut-framework cross-platform bir çerçevedir.** Nuxut uygulamaları Linux, Windows ve macOS'ta
  çalışacak şekilde tasarlanır.
- Zaman planı: Windows/macOS **platform katmanı** (winit tabanlı pencere/girdi; macOS için GL yerine
  wgpu/Metal render yolu) ilk cross-platform uygulama yazılırken eklenir. Bugünden itibaren ise
  framework bu katmanın eklenmesini engelleyecek hiçbir şey yapmaz.

## Bugünden geçerli kurallar
1. `nuxut-wayland` dışındaki her framework crate'i (core, motion, config, theme, render, ui, image,
   palette, video, cephe) Windows ve macOS hedeflerinde **derlenebilir** kalır
   (`cargo check --target x86_64-pc-windows-msvc` / `aarch64-apple-darwin`, mümkün olan crate'ler için).
2. Linux'a özgü her şey (fontconfig, inotify, XDG yolları, libmpv/VA-API, Secret Service, ikon teması
   araması…) `#[cfg(target_os = "linux")]` arkasında, platform bağımsız bir trait/arayüz üzerinden ve
   diğer platformlar için çalışan bir yedekle (en azından "kullanılamıyor" durumu) sunulur.
3. `nuxut-render` ve `nuxut-ui` Wayland, EGL veya Linux API'si bilmez; GL'e özgü tipler `Painter`
   API'sine sızmaz (ikinci render backend'i eklenebilsin).
4. Yeni bağımlılık seçerken cross-platform olanı tercih edilir; Linux-only bir crate yalnızca
   platforma özgü modülde kullanılabilir. (DEPENDENCIES.md'de "platform" sütunu.)

## Ekler
- 2026-10-13 (görev): **nuxut-portal** (xdg-desktop-portal
  FileChooser) kural 1–2 kapsamındadır: Linux dışında derlenir, her istek
  `PortalError::Unavailable` ile biter (zbus yalnız Linux hedefinde); `tools/cross-check.sh`
  listesinde, cephe `--features portal` ile denetlenir. Linux'a özgü (derlenmesi gerekmeyen) tek
  crate yine `nuxut-wayland`'dir.

## Sonuçlar
- Bugün Windows/macOS üzerinde çalışan bir uygulama veya testi yoktur; uyum derleme kontrolüyle korunur.
- Öncelik değişmez: önce shell'in günlük kullanılabilirliği ve Linux uygulamaları; cross-platform
  platform katmanı ilk cross-platform uygulama ihtiyacıyla gelir.
