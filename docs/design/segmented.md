# Bölümlü düğme: seçenek başına devre dışı

Bileşen: `nuxut_ui::SegmentedButton` (`crates/nuxut-ui/src/segmented.rs`). Sözleşme özeti:
`docs/ARCHITECTURE.md` (Bölümlü düğme).

## Kararlar

| Karar | Kim, ne zaman |
|---|---|
| Seçenek başına devre dışı: `Segment::with_disabled(bool)` (kurucu) ve `SegmentedButton::set_segment_enabled(i, bool, now)` / `is_segment_enabled(i)` | istek, 2026-10-14 |
| Seçili seçenek devre dışı yapılırsa **seçim değişmez**, yalnız etkileşim kapanır | istek, 2026-10-14 |

## Davranış

- **Çizim:** devre dışı seçeneğin ikon/etiketi `disabled_content(on_surface)` (%38); yüksek
  karşıtlıkta altındaki zemine (ray ya da devre dışı kap) ≥ 3:1 yükseltilir
  (`contrast_tests.rs` "devre dışı bölüm"). Hover/basılı katmanı yoktur.
- **Fare:** devre dışı seçenek olay almaz (tıklama seçim üretmez, hover yok); imleç `Default`.
- **Klavye:** Sol/Sağ o yöndeki en yakın etkin seçeneğe gider, Home/End ilk/son etkin seçeneğe;
  o yönde etkin seçenek yoksa seçim yerinde kalır (olay yok, yalnız yeniden çizim).
- **Seçili seçenek devre dışıysa:** seçim korunur (uygulama durumu örn. donanımın desteklemediği
  ama şu an etkin olan bir kip olabilir; kendiliğinden başka seçeneğe atlamak kullanıcının
  ayarını sessizce değiştirirdi). Gösterge yerinde kalır ama vurgusunu yitirir: dolgusu
  `primary` yerine devre dışı kap (`on_surface` %12), içerik kap üstünde soluk. Kullanıcı
  klavye/fare ile başka etkin seçeneğe geçebilir, devre dışı olana geri dönemez.
- **Dış durum:** `set_selected` devre dışı seçeneği de seçebilir (uygulamanın kararı).
- Bileşenin tamamını kapatan `set_enabled(false)` bundan bağımsızdır ve önceliklidir; seçenek
  bazındaki değerler korunur.
