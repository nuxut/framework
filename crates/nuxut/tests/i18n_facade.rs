//! `nuxut::i18n` cephesi: makrolar `nuxut_i18n`'e doğrudan bağımlılık olmadan çalışır
//! (`$crate` yolu proc-macro'dan geçer).

use std::sync::Arc;

#[test]
fn macros_through_facade() {
    let src = nuxut::i18n::embed!("fx", "../nuxut-i18n/tests/fixtures/good");
    assert!(src.languages().any(|l| l == "tr"));
    let prefs = nuxut::i18n::LanguagePrefs::fixed(&["en"]);
    nuxut::i18n::set_current(Arc::new(nuxut::i18n::Catalog::load(&[src], &prefs)));
    assert_eq!(nuxut::i18n::t!("fx-greet", name = "Ada"), "Hello Ada");
}
