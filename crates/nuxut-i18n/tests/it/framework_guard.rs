//! Framework'ün kendi kaynağında gömülü kullanıcı metni yok (ADR-0010 §4): her bulgu ya `t!`
//! ya da repo kökündeki `i18n-allow.txt`'te / `// i18n: tanı` işaretinde gerekçeli tanı metni.

#[test]
fn framework_has_no_embedded_ui_text() {
    let r = nuxut_i18n::check::source_guard(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    // Framework `dosya:*` ve `dizin/**` kullanmaz: toptan muafiyet yeni UI metnini de gizler.
    let wide: Vec<&String> = r
        .notes
        .iter()
        .filter(|n| n.contains("tüm dosyayı muaf tutar") || n.contains("tüm dosyaları muaf tutar"))
        .collect();
    assert!(wide.is_empty(), "dosya:* / dizin/** izinleri: {wide:?}");
    r.assert_ok();
}
