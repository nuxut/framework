//! Kullanılabilir locale listesi ve dil/bölge adları (glibc `LC_ADDRESS`).

use crate::Locale;

/// Gömülü veride biçimlenebilen locale adları (kanonik, bayt sırasıyla, tekrarsız).
///
/// Kaynak `pure-rust-locales` (glibc `localedata`) listesidir: biçimleme bu gömülü veriyle
/// yapıldığından sistemde kurulu olup olmaması önemsizdir; bu yüzden "kurulu locale'ler" değil
/// gömülü liste doğru kaynaktır. Dışarıda kalanlar:
/// - `POSIX` (bir dil değil; `Locale::parse` zaten reddeder);
/// - `@euro` değiştiricileri: avro öncesi para birimi için tarihî eşler (`de_DE@euro` =
///   `de_DE`); tarih/saat ve sayı verisi değiştiricisiz adla aynıdır, listede çift görünürdü.
///
/// Yazı/varyant değiştiricileri (`sr_RS@latin`, `uz_UZ@cyrillic`, `ca_ES@valencia`,
/// `ks_IN@devanagari`…) dahildir: ad tabloları gerçekten farklıdır.
pub(crate) const AVAILABLE: &[&str] = &[
    "aa_DJ", "aa_ER", "aa_ER@saaho", "aa_ET", "af_ZA", "agr_PE", "ak_GH", "am_ET", "an_ES", "anp_IN",
    "ar_AE", "ar_BH", "ar_DZ", "ar_EG", "ar_IN", "ar_IQ", "ar_JO", "ar_KW", "ar_LB", "ar_LY",
    "ar_MA", "ar_OM", "ar_QA", "ar_SA", "ar_SD", "ar_SS", "ar_SY", "ar_TN", "ar_YE", "as_IN",
    "ast_ES", "ayc_PE", "az_AZ", "az_IR", "be_BY", "be_BY@latin", "bem_ZM", "ber_DZ", "ber_MA", "bg_BG",
    "bhb_IN", "bho_IN", "bho_NP", "bi_VU", "bn_BD", "bn_IN", "bo_CN", "bo_IN", "br_FR", "brx_IN",
    "bs_BA", "byn_ER", "ca_AD", "ca_ES", "ca_ES@valencia", "ca_FR", "ca_IT", "ce_RU", "chr_US", "cmn_TW",
    "crh_UA", "cs_CZ", "csb_PL", "cv_RU", "cy_GB", "da_DK", "de_AT", "de_BE", "de_CH", "de_DE",
    "de_IT", "de_LI", "de_LU", "doi_IN", "dsb_DE", "dv_MV", "dz_BT", "el_CY", "el_GR", "en_AG",
    "en_AU", "en_BW", "en_CA", "en_DK", "en_GB", "en_HK", "en_IE", "en_IL", "en_IN", "en_NG",
    "en_NZ", "en_PH", "en_SC", "en_SG", "en_US", "en_ZA", "en_ZM", "en_ZW", "eo", "es_AR",
    "es_BO", "es_CL", "es_CO", "es_CR", "es_CU", "es_DO", "es_EC", "es_ES", "es_GT", "es_HN",
    "es_MX", "es_NI", "es_PA", "es_PE", "es_PR", "es_PY", "es_SV", "es_US", "es_UY", "es_VE",
    "et_EE", "eu_ES", "fa_IR", "ff_SN", "fi_FI", "fil_PH", "fo_FO", "fr_BE", "fr_CA", "fr_CH",
    "fr_FR", "fr_LU", "fur_IT", "fy_DE", "fy_NL", "ga_IE", "gd_GB", "gez_ER", "gez_ER@abegede", "gez_ET",
    "gez_ET@abegede", "gl_ES", "gu_IN", "gv_GB", "ha_NG", "hak_TW", "he_IL", "hi_IN", "hif_FJ", "hne_IN",
    "hr_HR", "hsb_DE", "ht_HT", "hu_HU", "hy_AM", "ia_FR", "id_ID", "ig_NG", "ik_CA", "is_IS",
    "it_CH", "it_IT", "iu_CA", "ja_JP", "ka_GE", "kab_DZ", "kk_KZ", "kl_GL", "km_KH", "kn_IN",
    "ko_KR", "kok_IN", "ks_IN", "ks_IN@devanagari", "ku_TR", "kw_GB", "ky_KG", "lb_LU", "lg_UG", "li_BE",
    "li_NL", "lij_IT", "ln_CD", "lo_LA", "lt_LT", "lv_LV", "lzh_TW", "mag_IN", "mai_IN", "mai_NP",
    "mfe_MU", "mg_MG", "mhr_RU", "mi_NZ", "miq_NI", "mjw_IN", "mk_MK", "ml_IN", "mn_MN", "mni_IN",
    "mnw_MM", "mr_IN", "ms_MY", "mt_MT", "my_MM", "nan_TW", "nan_TW@latin", "nb_NO", "nds_DE", "nds_NL",
    "ne_NP", "nhn_MX", "niu_NU", "niu_NZ", "nl_AW", "nl_BE", "nl_NL", "nn_NO", "nr_ZA", "nso_ZA",
    "oc_FR", "om_ET", "om_KE", "or_IN", "os_RU", "pa_IN", "pa_PK", "pap_AW", "pap_CW", "pl_PL",
    "ps_AF", "pt_BR", "pt_PT", "quz_PE", "raj_IN", "ro_RO", "ru_RU", "ru_UA", "rw_RW", "sa_IN",
    "sah_RU", "sat_IN", "sc_IT", "sd_IN", "sd_IN@devanagari", "se_NO", "sgs_LT", "shn_MM", "shs_CA", "si_LK",
    "sid_ET", "sk_SK", "sl_SI", "sm_WS", "so_DJ", "so_ET", "so_KE", "so_SO", "sq_AL", "sq_MK",
    "sr_ME", "sr_RS", "sr_RS@latin", "ss_ZA", "st_ZA", "sv_FI", "sv_SE", "sw_KE", "sw_TZ", "szl_PL",
    "ta_IN", "ta_LK", "tcy_IN", "te_IN", "tg_TJ", "th_TH", "the_NP", "ti_ER", "ti_ET", "tig_ER",
    "tk_TM", "tl_PH", "tn_ZA", "to_TO", "tpi_PG", "tr_CY", "tr_TR", "ts_ZA", "tt_RU", "tt_RU@iqtelif",
    "ug_CN", "uk_UA", "unm_US", "ur_IN", "ur_PK", "uz_UZ", "uz_UZ@cyrillic", "ve_ZA", "vi_VN", "wa_BE",
    "wae_CH", "wal_ET", "wo_SN", "xh_ZA", "yi_US", "yo_NG", "yue_HK", "yuw_PG", "zh_CN", "zh_HK",
    "zh_SG", "zh_TW", "zu_ZA",
];

/// Yalnızca dil verilen adın (`en`, `ja`) bölgesi: `<dil>_<DİL>` veride yoksa (ör. `en_EN`)
/// kullanılan yaygın bölge (CLDR "likely subtags"). Tabloda olmayan dil → listede o dille
/// başlayan ilk ad.
pub(crate) fn likely_region(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "ar" => "ar_EG",
        "be" => "be_BY",
        "bn" => "bn_BD",
        "ca" => "ca_ES",
        "cs" => "cs_CZ",
        "cy" => "cy_GB",
        "da" => "da_DK",
        "el" => "el_GR",
        "en" => "en_US",
        "et" => "et_EE",
        "fa" => "fa_IR",
        "ga" => "ga_IE",
        "he" => "he_IL",
        "hi" => "hi_IN",
        "hy" => "hy_AM",
        "ja" => "ja_JP",
        "ka" => "ka_GE",
        "kk" => "kk_KZ",
        "ko" => "ko_KR",
        "ms" => "ms_MY",
        "nb" => "nb_NO",
        "nn" => "nn_NO",
        "sl" => "sl_SI",
        "sq" => "sq_AL",
        "sr" => "sr_RS",
        "sv" => "sv_SE",
        "sw" => "sw_TZ",
        "ta" => "ta_IN",
        "te" => "te_IN",
        "uk" => "uk_UA",
        "ur" => "ur_PK",
        "vi" => "vi_VN",
        "zh" => "zh_CN",
        _ => return None,
    })
}

/// Listede `lang` diliyle başlayan ilk ad (`lang_` öneki ya da tam ad).
pub(crate) fn first_with_language(lang: &str) -> Option<&'static str> {
    AVAILABLE.iter().copied().find(|n| n.split(['_', '@']).next() == Some(lang))
}

/// İlk harfi büyütür (menü öğesi biçimi: "français" → "Français"). Türk dilleri noktalı
/// büyük İ kullanır ("i" → "İ"). Değişmeyenler: büyük harfle başlayan ad, ilk sözcüğünde
/// büyük harf olan ad (Bantu önekleri "isiZulu", "siSwati" yazıldığı gibi kalır) ve Gürcüce
/// (Mkhedruli'nin büyük karşılığı Mtavruli başlıkta kullanılmaz).
pub(crate) fn capitalize_first(s: &str, lang: &str) -> String {
    let mut chars = s.chars();
    let Some(first) = chars.next() else { return String::new() };
    let first_word = s.split_whitespace().next().unwrap_or_default();
    if !first.is_lowercase() || lang == "ka" || first_word.chars().any(char::is_uppercase) {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len() + 2);
    if first == 'i' && matches!(lang, "tr" | "az" | "crh" | "gag") {
        out.push('İ');
    } else {
        out.extend(first.to_uppercase());
    }
    out.push_str(chars.as_str());
    out
}

impl Locale {
    /// Dilin kendi dilindeki adı, menü biçiminde ilk harfi büyük: `tr_TR` → "Türkçe", `en_US`
    /// → "English", `de_DE` → "Deutsch", `fr_FR` → "Français" (glibc `LC_ADDRESS lang_name`).
    /// Veride ad yoksa dil kodu (`eo`).
    pub fn language_name(&self) -> String {
        let raw: Option<&str> = pure_rust_locales::locale_match!(self.inner => LC_ADDRESS::LANG_NAME);
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => capitalize_first(name, self.language()),
            None => self.language().to_owned(),
        }
    }

    /// Bölgenin locale dilindeki adı (`tr_TR` → "Türkiye", `en_US` → "United States"; glibc
    /// `LC_ADDRESS country_name`); veride yoksa `None`.
    pub fn country_name(&self) -> Option<&'static str> {
        let raw: Option<&'static str> = pure_rust_locales::locale_match!(self.inner => LC_ADDRESS::COUNTRY_NAME);
        raw.map(str::trim).filter(|s| !s.is_empty())
    }

    /// Bölgeyi de ayırt eden gösterim adı: "English (United States)", "Türkçe (Kıbrıs)".
    /// Ülke adı yoksa yalnızca dil adı. Değiştiricili ad aynı dil + ülke adını veriyorsa
    /// değiştirici eklenir ("Català (Espanya, valencia)").
    pub fn display_name(&self) -> String {
        let lang = self.language_name();
        let mut parts: Vec<&str> = Vec::new();
        if let Some(country) = self.country_name() {
            parts.push(country);
        }
        if let Some(modifier) = self.modifier() {
            let base = self.name.split('@').next().and_then(Locale::parse);
            if base.is_some_and(|b| b.language_name() == lang) {
                parts.push(modifier);
            }
        }
        if parts.is_empty() { lang } else { format!("{lang} ({})", parts.join(", ")) }
    }

    /// Değiştirici (`sr_RS@latin` → `latin`).
    pub fn modifier(&self) -> Option<&str> {
        self.name.split_once('@').map(|(_, m)| m)
    }
}

/// Gömülü veride biçimlenebilen tüm locale adları (kanonik, sıralı, tekrarsız; `POSIX` ve
/// `@euro` eşleri hariç — gerekçe: crate belgesi / ARCHITECTURE "Locale" kural 4). Her ad [`Locale::parse`] ile aynı ada
/// çözülür. Sistemde kurulu olup olmadıklarına bakılmaz (biçimleme gömülü veriyle).
pub fn available() -> &'static [&'static str] {
    AVAILABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(s: &str) -> Locale {
        Locale::parse(s).unwrap_or_else(|| panic!("{s}"))
    }

    #[test]
    fn available_is_sorted_canonical_and_parsable() {
        let all = available();
        assert!(all.len() > 300, "{}", all.len());
        assert!(all.windows(2).all(|w| w[0] < w[1]), "sıralı ve tekrarsız");
        for name in ["tr_TR", "en_US", "de_DE", "ja_JP", "en_IN", "sr_RS@latin", "eo"] {
            assert!(all.contains(&name), "{name}");
        }
        assert!(!all.iter().any(|n| n.ends_with("@euro") || *n == "POSIX"));
        for name in all {
            // Kanonik: parse aynı adı verir; chrono adı da aynı (veri gerçekten var).
            let l = loc(name);
            assert_eq!(l.name(), *name);
            assert_eq!(l.chrono().to_string(), *name);
            assert!(!l.language_name().is_empty() && !l.display_name().is_empty(), "{name}");
        }
    }

    #[test]
    fn language_names_from_lc_address() {
        assert_eq!(loc("tr_TR").language_name(), "Türkçe");
        assert_eq!(Locale::en_us().language_name(), "English");
        assert_eq!(loc("de_DE").language_name(), "Deutsch");
        assert_eq!(loc("fr_FR").language_name(), "Français");
        assert_eq!(loc("ja_JP").language_name(), "日本語");
        assert_eq!(loc("ko_KR").language_name(), "한국어");
        assert_eq!(loc("ka_GE").language_name(), "ქართული");
        assert_eq!(loc("zu_ZA").language_name(), "isiZulu");
        assert_eq!(loc("sr_RS@latin").language_name(), "Srpski");
        assert_eq!(Locale::en_us().display_name(), "English (United States)");
        assert_eq!(loc("tr_CY").display_name(), "Türkçe (Kıbrıs)");
        assert_eq!(loc("ca_ES@valencia").display_name(), "Català (Espanya, valencia)");
        assert_eq!(loc("eo").display_name(), "Esperanto");
        assert_eq!(loc("en_IN").country_name(), Some("India"));
        assert_eq!(loc("sr_RS@latin").modifier(), Some("latin"));
        assert_eq!(loc("tr_TR").modifier(), None);
    }

    #[test]
    fn turkic_capitalization() {
        assert_eq!(capitalize_first("ingilizce", "tr"), "İngilizce");
        assert_eq!(capitalize_first("ingilizce", "en"), "Ingilizce");
        assert_eq!(capitalize_first("ıslak", "tr"), "Islak");
        assert_eq!(capitalize_first("français", "fr"), "Français");
        assert_eq!(capitalize_first("isiXhosa", "xh"), "isiXhosa");
        assert_eq!(capitalize_first("", "tr"), "");
    }
}
