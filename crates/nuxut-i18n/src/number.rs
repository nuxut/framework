//! Fluent sayı biçimi `nuxut-locale` ile (glibc `LC_NUMERIC`; tek doğruluk kaynağı ve tek
//! uygulama: [`nuxut_locale::NumberSymbols::format`]). `NUMBER($n, minimumFractionDigits: 2)`
//! seçenekleri birleştirir (fluent yerleşiği); burada yalnızca [`NumberOptions`]'a çevrilir.
//! Fluent biçimleyicisi bağlamsız bir `fn` işaretçisi olduğundan katalogun simgeleri
//! biçimleme süresince iş parçacığı yereline konur ([`scoped`]); başka katalogları etkilemez.

use std::cell::Cell;

use fluent_bundle::FluentValue;
use fluent_bundle::types::{FluentNumber, FluentNumberStyle};
use intl_memoizer::concurrent::IntlLangMemoizer;
use nuxut_locale::{NumberOptions, NumberSymbols};

/// Bir locale'in sayı simgeleri (glibc verisi, `'static`).
pub(crate) type Symbols = NumberSymbols;

thread_local! {
    static SYMBOLS: Cell<Option<Symbols>> = const { Cell::new(None) };
}

/// `f` süresince bu iş parçacığında `s` geçerli (iç içe çağrı öncekini geri yükler).
pub(crate) fn scoped<R>(s: Symbols, f: impl FnOnce() -> R) -> R {
    let prev = SYMBOLS.replace(Some(s));
    let out = f();
    SYMBOLS.set(prev);
    out
}

/// `FluentBundle::set_formatter` kancası: yalnızca sayılar.
pub(crate) fn formatter(v: &FluentValue<'_>, _: &IntlLangMemoizer) -> Option<String> {
    match v {
        FluentValue::Number(n) => Some(format(n, SYMBOLS.get().unwrap_or_default())),
        _ => None,
    }
}

pub(crate) fn format(n: &FluentNumber, s: Symbols) -> String {
    let o = &n.options;
    let mut options = NumberOptions::default();
    let percent = o.style == FluentNumberStyle::Percent;
    options.min_fraction_digits = o.minimum_fraction_digits.unwrap_or(0);
    // ECMA-402 (SetNumberFormatDigitOptions): percent stilinde varsayılan kesir aralığı 0..0;
    // yalnızca minimum verilmişse en çok = en az. Diğer stillerde sınır yok (en kısa gösterim).
    options.max_fraction_digits =
        o.maximum_fraction_digits.or_else(|| percent.then_some(options.min_fraction_digits));
    options.min_integer_digits = o.minimum_integer_digits.unwrap_or(1);
    options.grouping = o.use_grouping;
    options.percent = percent;
    s.format(n.value, &options)
}

#[cfg(test)]
mod tests {
    use fluent_bundle::types::FluentNumberOptions;
    use nuxut_locale::Locale;

    use super::*;

    fn num(v: f64, f: impl FnOnce(&mut FluentNumberOptions)) -> FluentNumber {
        let mut options = FluentNumberOptions::default();
        f(&mut options);
        FluentNumber::new(v, options)
    }

    fn sym(name: &str) -> Symbols {
        Locale::parse(name).unwrap().number_symbols()
    }

    #[test]
    fn locale_separators() {
        let tr = sym("tr_TR");
        let en = sym("en_US");
        assert_eq!(format(&num(1234567.5, |_| {}), tr), "1.234.567,5");
        assert_eq!(format(&num(1234567.5, |_| {}), en), "1,234,567.5");
        assert_eq!(format(&num(-0.25, |_| {}), tr), "-0,25");
        assert_eq!(format(&num(12.0, |o| o.minimum_fraction_digits = Some(2)), tr), "12,00");
        assert_eq!(format(&num(4.56789, |o| o.maximum_fraction_digits = Some(2)), en), "4.57");
        assert_eq!(format(&num(1234.0, |o| o.use_grouping = false), en), "1234");
        assert_eq!(format(&num(0.5, |o| o.style = FluentNumberStyle::Percent), en), "50%");
        assert_eq!(format(&num(0.5, |o| o.style = FluentNumberStyle::Percent), tr), "%50");
        assert_eq!(format(&num(-0.0001, |o| o.maximum_fraction_digits = Some(2)), en), "0");
        assert_eq!(format(&num(f64::NAN, |_| {}), en), "NaN");
        assert_eq!(format(&num(1234567.0, |_| {}), sym("en_IN")), "12,34,567");
        // Aşırı seçenekler sınırlanır.
        let big = format(&num(1.0, |o| {
            o.minimum_fraction_digits = Some(usize::MAX);
            o.minimum_integer_digits = Some(usize::MAX);
        }), en);
        assert!(big.len() < 200, "{}", big.len());
    }

    #[test]
    fn percent_fraction_defaults() {
        let tr = sym("tr_TR");
        let en = sym("en_US");
        let pct = |o: &mut FluentNumberOptions| o.style = FluentNumberStyle::Percent;
        // Varsayılan 0 kesir (ECMA-402): artık da kesir de görünmez, yuvarlanır.
        assert_eq!(format(&num(0.29, pct), tr), "%29");
        assert_eq!(format(&num(0.126, pct), en), "13%");
        assert_eq!(format(&num(0.994, pct), en), "99%");
        // Yalnızca minimum: en çok = en az.
        let min1 = num(0.1234, |o| {
            pct(o);
            o.minimum_fraction_digits = Some(1);
        });
        assert_eq!(format(&min1, en), "12.3%");
        // Açık maksimum geçerli; sondaki sıfırlar atılır.
        let max2 = num(0.1234, |o| {
            pct(o);
            o.maximum_fraction_digits = Some(2);
        });
        assert_eq!(format(&max2, en), "12.34%");
        assert_eq!(format(&num(0.29, |o| {
            pct(o);
            o.maximum_fraction_digits = Some(2);
        }), en), "29%");
        // Yüzde dışı stil sınırsız kalır.
        assert_eq!(format(&num(0.1234, |_| {}), en), "0.1234");
    }

    #[test]
    fn same_as_locale_api() {
        // Tek uygulama: Fluent yolu Locale::format_number ile aynı metni üretir.
        let l = Locale::parse("de_DE").unwrap();
        let fluent = format(&num(1234.5, |o| {
            o.minimum_fraction_digits = Some(2);
            o.maximum_fraction_digits = Some(2);
        }), l.number_symbols());
        assert_eq!(fluent, l.format_number(1234.5, 2));
    }
}
