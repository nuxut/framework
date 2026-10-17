# ADR-0001 — Rust, kendi renderer'ımız, katman bazlı crate'ler

Tarih: 2026-10-04 · Durum: kabul edildi

## Bağlam
Hedef: estetik, akıcı animasyonlu, hafif ve son derece kişiselleştirilebilir bir Hyprland
shell'i ve aynı tasarım dilini paylaşan cross-platform uygulamalar. Referanslar: caelestia
(Quickshell/QML, en iyi hareket), noctalia v5 (Qt'den çıkıp kendi C++/GLES renderer'ı, ~6×
daha az bellek). Mevcut Quickshell tabanlı eski shell bu makinede ~340 MB RSS kullanıyor.

## Karar
1. Her şey Rust.
2. Kendi scene/renderer'ımız: OpenGL ES 3.0 (`glow`), EGL (`khronos-egl`), SDF tabanlı
   şekiller ve gölgeler, `cosmic-text` ile metin. Toolkit (Qt/GTK) yok.
3. Wayland: `smithay-client-toolkit` + `calloop`.
4. Crate'ler katman/bağımlılık sınırına göre bölünür (bkz. ARCHITECTURE.md). Bileşenler tek
   `nuxut-ui` crate'inde; ağır opsiyonel bileşenler feature arkasında.
5. Config: TOML (`framework.toml` global görünüm, `<app>.toml` uygulamaya özel).

## Neden GLES, neden wgpu değil (şimdilik)
GLES: en küçük bellek/başlangıç maliyeti, sanal makinelerde (virgl) ve her Linux GPU'sunda
çalışır, noctalia aynı yolu kanıtladı. wgpu macOS/Windows için daha doğru olacaktır; bu yüzden
`Painter` API'si GL'e özgü tip sızdırmaz ve ileride ikinci bir backend eklenebilir.

## Sonuçlar
- Metin girişi, IME, erişilebilirlik gibi toolkit işleri bizim sorumluluğumuzda; ihtiyaç
  doğdukça `cosmic-text` (düzenleme), `accesskit` gibi Rust crate'leri kullanılacak.
- Cross-platform uygulamalar için ikinci bir platform crate'i (`winit` tabanlı) gerekecek.
