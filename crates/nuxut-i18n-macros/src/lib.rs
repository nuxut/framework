//! `nuxut_i18n::embed!`'in arkasındaki proc-macro (ADR-0010 §3). Doğrudan çağrılmaz:
//! `nuxut_i18n::embed!("settings")` bunu `$crate` yoluyla çağırır, böylece çağıran crate
//! `nuxut_i18n`'e doğrudan bağımlı olmadan (`nuxut::i18n::embed!`) kullanabilir.
//!
//! Girdi: `<crate yolu>, "<domain>" [, "<dizin>"]`. Dizin çağıran crate'in
//! `CARGO_MANIFEST_DIR`'ine görelidir (varsayılan `locales`). Dizindeki `<dil>.ftl`
//! dosyaları derleme zamanında listelenir ve her biri `include_str!` ile gömülür (içerik
//! değişince yeniden derlenir). **Yeni dil dosyası eklemek** dosya içeriği değişmediği için
//! derlemeyi tetiklemez: makroyu çağıran dosyaya dokunun; `nuxut_i18n::check` diskteki
//! dosyalarla gömülü listeyi karşılaştırıp bunu yakalar.
//!
//! Bağımlılık yok (yalnızca `proc_macro`); ayrıştırma elle, panik etmez — hata
//! `compile_error!` olarak döner.

use std::path::PathBuf;

use proc_macro::{Delimiter, TokenStream, TokenTree};

#[doc(hidden)]
#[proc_macro]
pub fn __embed(input: TokenStream) -> TokenStream {
    match embed(input) {
        Ok(ts) => ts,
        Err(msg) => error(&msg),
    }
}

fn error(msg: &str) -> TokenStream {
    format!("::core::compile_error!({msg:?})").parse().unwrap_or_default()
}

fn embed(input: TokenStream) -> Result<TokenStream, String> {
    let mut path = TokenStream::new();
    let mut literals = Vec::new();
    let mut seen_comma = false;
    for tt in input {
        match &tt {
            TokenTree::Punct(p) if p.as_char() == ',' => seen_comma = true,
            // `$crate` makro genişlemesinde görünmez gruplar içinde gelebilir.
            TokenTree::Group(g) if !seen_comma && g.delimiter() == Delimiter::None => path.extend(g.stream()),
            _ if !seen_comma => path.extend([tt.clone()]),
            TokenTree::Literal(lit) => literals.push(string_literal(&lit.to_string())?),
            TokenTree::Group(g) if g.delimiter() == Delimiter::None => {
                let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                match inner.as_slice() {
                    [TokenTree::Literal(lit)] => literals.push(string_literal(&lit.to_string())?),
                    _ => return Err("embed!: dizge sabiti bekleniyor".into()),
                }
            }
            _ => return Err("embed!: dizge sabiti bekleniyor".into()),
        }
    }
    let (domain, dir) = match literals.as_slice() {
        [d] => (d.clone(), "locales".to_owned()),
        [d, dir] => (d.clone(), dir.clone()),
        _ => return Err("embed!: kullanım embed!(\"<domain>\") ya da embed!(\"<domain>\", \"<dizin>\")".into()),
    };
    if domain.is_empty() || !domain.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(format!("embed!: geçersiz domain {domain:?} ([a-z0-9-]+)"));
    }
    if dir.is_empty() || dir.starts_with('/') || dir.contains('\\') {
        return Err(format!("embed!: dizin crate köküne göreli olmalı: {dir:?}"));
    }
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or("embed!: CARGO_MANIFEST_DIR yok")?;
    let root = PathBuf::from(manifest).join(&dir);
    let entries = std::fs::read_dir(&root).map_err(|e| format!("embed!: {} okunamadı: {e}", root.display()))?;
    let mut langs: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(lang) = name.strip_suffix(".ftl") else { continue };
        if !valid_lang(lang) {
            return Err(format!("embed!: {name}: dosya adı bir dil etiketi değil (tr.ftl, pt-BR.ftl)"));
        }
        let full = root.join(name);
        let full = full.to_str().ok_or("embed!: yol UTF-8 değil")?.to_owned();
        langs.push((lang.to_owned(), full));
    }
    if langs.is_empty() {
        return Err(format!("embed!: {} içinde .ftl yok", root.display()));
    }
    langs.sort();
    let items: String = langs
        .iter()
        .map(|(lang, full)| format!("({lang:?}, ::core::include_str!({full:?})),"))
        .collect();
    let tail: TokenStream = format!("::Source {{ domain: {domain:?}, embedded: &[{items}] }}")
        .parse()
        .map_err(|_| "embed!: iç hata (çıktı ayrıştırılamadı)".to_owned())?;
    let mut out = path;
    out.extend(tail);
    Ok(out)
}

/// Yalnızca kaçışsız, düz `"..."` dizge sabiti.
fn string_literal(s: &str) -> Result<String, String> {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .filter(|s| !s.contains('\\') && !s.contains('"'))
        .map(str::to_owned)
        .ok_or_else(|| format!("embed!: kaçışsız dizge sabiti bekleniyor: {s}"))
}

/// `[a-zA-Z]{2,3}(-[A-Za-z0-9]{1,8})*` (BCP 47'nin dosya adında kullanılan alt kümesi).
fn valid_lang(s: &str) -> bool {
    let mut parts = s.split('-');
    let first = parts.next().unwrap_or_default();
    (2..=3).contains(&first.len())
        && first.bytes().all(|b| b.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}
