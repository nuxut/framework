//! Gömülü kullanıcı metni bekçisi (ADR-0010 §4): `src/**/*.rs` dizge sabitlerini tarar.
//!
//! Hata: (a) Türkçeye özgü harf (`çğıöşüÇĞİÖŞÜ`) içeren sabit; (b) bilinen bir UI
//! kurucusuna/ayarlayıcısına ([`DEFAULT_UI_CALLS`]) verilen, harf içeren sabit.
//!
//! Yapısal olarak muaf (tanı metni; izin satırı gerekmez):
//! - yorumlar; tüm öznitelikler (`#[error("..")]` dahil — thiserror iletisi `Display`'dir);
//! - `impl (fmt::)Debug for` gövdeleri; `impl (fmt::)Display for` gövdeleri yalnızca hata/uyarı
//!   türünde (adı `Error`/`Warning` ile biten ya da dosyada `impl Error for Tür` /
//!   `#[derive(.., Error)]` taşıyan tür) — başka türün Display'i `to_string()` ile arayüze
//!   gidebileceği için denetlenir;
//! - hata türü kurucuları: hata türünün `Tür::Varyant(..)`/`{..}`/`Tür::new(..)` çağrısı
//!   (`ApplyError::Invalid(".."), io::Error::other("..")`) ve hata türünün `impl` bloğu
//!   içinde `Self::Varyant(..)`;
//! - log/tanı makroları (`tracing`/`log` düzeyleri, `eprintln!`, `println!`, `panic!`,
//!   `unreachable!`, `assert*!`…), `.expect(..)`, `t!(..)` anahtarları;
//! - `#[cfg(test)]`/`#[test]` öğeleri ve `#[cfg(test)] mod x;` ile bağlanan dosyalar;
//!   `tests/`, `examples/`, `benches/` dizinleri;
//! - [`SourceGuard::add_diagnostic_calls`] ile verilen repo geneli tanı çağrıları.
//!
//! UI kurucusunun argümanı her zaman denetlenir: hata kurucusu, tanı çağrısı ya da izin
//! listesindeki `dosya:çağrı()` içinde kalsa da (`Label::new(t, XError::A("Kaydedilemedi"))`)
//! bulgudur. Orada yalnızca harfsiz (`"{n}"`) ve makine biçimli dizgeler (yol/URL, küçük harfli
//! `snake_case` ya da noktalı kimlik, `-symbolic` ikon adı) muaftır; tek sözcük metin sayılır.
//! Kesin muaf bölgeler (test, log/tanı makrosu, `t!`, öznitelik) UI çağrısını da kapsar.
//!
//! Muaf ama raporlanır ([`Report::notes`]): CLI ikilileri (`src/main.rs`, `src/bin/**`) ve
//! proc-macro crate'leri — Türkçe harfli sabitler muaf (CLI çıktısı ve derleme tanısı
//! çevrilmez), UI kurucusu bulguları yine hata.
//!
//! Satır içi işaret `// i18n: tanı <gerekçe>` (düz `//` yorumu; `///`/`//!`/`/* */` değil):
//! kapsadığı Türkçe harfli dizge sabitleri tanı metni sayılır.
//! - Satır sonu yorumu (`let m = "bağlanamadı"; // i18n: tanı journal'a gider`): o satırda
//!   *başlayan* dizgeler.
//! - Tek başına satırdaki yorum: hemen sonraki deyim — derinlik 0'daki ilk `;` ya da `,`'e dek
//!   (dahil), kapsayan parantezin kapanışına dek ya da sonraki satırda `else`/`.`/`?` ile
//!   sürmeyen `{..}` bloğunun sonuna dek; parantez grupları bütün kapsanır.
//! - Gerekçe zorunlu; `tanı` dışındaki `// i18n:` işaretleri hata. Hiçbir bulguyu bastırmayan
//!   işaret ("gereksiz işaret") hatadır: yapısal muafiyetin, izin listesindeki `dosya:çağrı()`'nın
//!   ya da CLI/proc-macro muafiyetinin zaten kapsadığı, ya da önceki bir işaretle çakışan işaret.
//! - UI kurucusu argümanını muaf tutmaz (UI bulgusu kalır, işaret gereksiz sayılır).
//! - `#[cfg(test)] mod x;` ile bağlanan ve `tests/` gibi taranmayan dosyalarda denetlenmez.
//!
//! İzin listesi `i18n-allow.txt` (köke göreli düz yol — `..`, mutlak yol, `.` parçası ve glob
//! karakteri reddedilir; her satır gerekçeli):
//! - `dosya:çağrı()  gerekçe` — o dosyada bu çağrının argümanları tanı metni
//!   (`err()`, `ApplyError::new()`, `warnings.push()`); satır kaymasından etkilenmez. Makro da
//!   olur: `dosya:s!()` o dosyadaki `s!(..)` / `s![..]` / `s!{..}` argümanları (bilinçli yedek
//!   metin tablosu gibi);
//! - `dosya:satır  gerekçe` — tek satır;
//! - `dosya:*  gerekçe` — tüm dosya; desteklenir ama her kullanımda uyarı notu üretir (dosyaya
//!   eklenen yeni UI metni de kaçar);
//! - `dizin/**  gerekçe` — dizin ve alt dizinlerindeki tüm dosyalar, `dosya:*` gibi (uyarı notu;
//!   `**` yalnızca sonda, başka glob yok). Öncelik: `dosya:satır` > `dosya:*` > en dar `dizin/**`.
//!
//! Gerekçesiz, bozuk, yinelenen ya da artık hiçbir bulguyu bastırmayan satır hatadır — liste
//! yalnızca küçülür. Geçiş istisnası: taranan bir dosyayı kapsayan ama gereksiz kalan `dosya:*` /
//! `dizin/**` satırı not; kapsadığı taranan dosya yoksa hata.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use super::scan::{LineComment, Tok, Token, lex};
use super::{Finding, Report};

/// Harfli sabit verilince hata olan çağrılar: `Tür::new` (ilişkili işlev) ya da yöntem adı.
pub const DEFAULT_UI_CALLS: &[&str] = &[
    "Label::new",
    "Chip::new",
    "Dialog::new",
    "Confirm::new",
    "Disclosure::new",
    "Tile::new",
    "Toast::new",
    "Tooltip::new",
    "Button::new",
    "TextButton::new",
    "ListItem::new",
    "with_label",
    "set_label",
    "set_labels",
    "with_text",
    "set_text",
    "with_title",
    "set_title",
    "with_subtitle",
    "set_subtitle",
    "with_description",
    "set_description",
    "with_summary",
    "set_summary",
    "with_placeholder",
    "set_placeholder",
    "with_tooltip",
    "set_tooltip",
    "with_hint",
    "set_hint",
];

/// İçindeki dizgeler denetlenmeyen makrolar (log, tanı, test, derleme zamanı, `t!`).
const EXCLUDED_MACROS: &[&str] = &[
    "trace",
    "debug",
    "info",
    "warn",
    "error",
    "event",
    "span",
    "trace_span",
    "debug_span",
    "info_span",
    "warn_span",
    "error_span",
    "eprintln",
    "eprint",
    "println",
    "print",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "t",
    "include_str",
    "include_bytes",
    "env",
    "option_env",
    "compile_error",
];

const TURKISH: &[char] = &['ç', 'ğ', 'ı', 'ö', 'ş', 'ü', 'Ç', 'Ğ', 'İ', 'Ö', 'Ş', 'Ü'];

/// Taranmayan dizinler.
const SKIP_DIRS: &[&str] = &[
    "target",
    "tests",
    "examples",
    "benches",
    "fixtures",
    "node_modules",
];

/// Çağrı kalıbı: `ad` (serbest işlev ya da yöntem), `Tür::ad`, `alıcı.ad`, makro `ad!`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CallSpec {
    Name(String),
    Path(String, String),
    Method(String, String),
    /// `ad!(..)` / `ad![..]` / `ad!{..}` (yalnız izin listesinde; UI/tanı çağrı listeleri işlev içindir).
    Macro(String),
}

impl CallSpec {
    fn parse(s: &str) -> Option<Self> {
        let ident = |x: &str| {
            !x.is_empty()
                && !x.starts_with(|c: char| c.is_ascii_digit())
                && x.chars().all(|c| c.is_alphanumeric() || c == '_')
        };
        if let Some(m) = s.strip_suffix('!') {
            return ident(m).then(|| Self::Macro(m.to_owned()));
        }
        if let Some((ty, f)) = s.rsplit_once("::") {
            return (ident(ty) && ident(f)).then(|| Self::Path(ty.to_owned(), f.to_owned()));
        }
        if let Some((recv, f)) = s.rsplit_once('.') {
            return (ident(recv) && ident(f)).then(|| Self::Method(recv.to_owned(), f.to_owned()));
        }
        ident(s).then(|| Self::Name(s.to_owned()))
    }

    /// `i`'deki kimlik bu çağrının adı mı (ardından `(` gelmesi çağıranın işi).
    fn matches(&self, toks: &[Token], i: usize) -> bool {
        let Some(name) = ident_at(toks, i) else {
            return false;
        };
        match self {
            Self::Name(f) => f == name,
            Self::Macro(m) => m == name && punct_at(toks, i + 1, b'!'),
            Self::Path(ty, f) => {
                f == name && i >= 3 && path_sep(toks, i) && ident_at(toks, i - 3) == Some(ty)
            }
            Self::Method(recv, f) => {
                f == name
                    && i >= 2
                    && punct_at(toks, i - 1, b'.')
                    && ident_at(toks, i - 2) == Some(recv)
            }
        }
    }
}

/// Bulgu türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HitKind {
    Turkish,
    Ui,
}

impl fmt::Display for HitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // i18n: tanı bulgu türü iletisi (bekçi tanısı, geliştiriciye)
        f.write_str(match self {
            Self::Turkish => "Türkçe harfli dizge sabiti (t! kullanın)",
            Self::Ui => "UI kurucusuna harfli dizge sabiti (t! kullanın)",
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct Hit {
    line: usize,
    kind: HitKind,
    /// Türkçe harfli bulguyu kapsayan geçerli `// i18n: tanı` işaretinin indeksi
    /// ([`FileScan::markers`]); UI bulgusunda hep `None`.
    marker: Option<usize>,
}

/// Tek dosyanın tarama sonucu: bulgular, bir bulguyu bastıran dosya-yerel çağrı izinleri,
/// geçerli işaretlerin satırları ve bozuk işaret hataları.
#[derive(Debug, Default)]
struct FileScan {
    hits: Vec<Hit>,
    used_local_calls: BTreeSet<usize>,
    markers: Vec<usize>,
    marker_errors: Vec<(usize, &'static str)>,
}

impl FileScan {
    /// İşaretli bulguları düşürür (CLI/proc-macro muafiyetinden sonra çağrılır: orada Türkçe
    /// bulgu zaten muaftır, işaret gereksiz kalır). Bozuk ve hiçbir bulguyu bastırmayan
    /// işaretler `(satır, ileti)` olarak döner.
    fn resolve_markers(
        hits: Vec<Hit>,
        markers: &[usize],
        mut errors: Vec<(usize, &'static str)>,
    ) -> (Vec<Hit>, Vec<(usize, &'static str)>) {
        let mut used = vec![false; markers.len()];
        let kept = hits
            .into_iter()
            .filter(|h| match h.marker {
                Some(m) => {
                    used[m] = true;
                    false
                }
                None => true,
            })
            .collect();
        errors.extend(
            markers
                .iter()
                .zip(used)
                .filter(|(_, u)| !u)
                .map(|(line, _)| (*line, MARKER_UNUSED)),
        );
        errors.sort_unstable();
        (kept, errors)
    }
}

// i18n: tanı bekçi tanısı (geliştiriciye)
const MARKER_UNUSED: &str =
    "gereksiz işaret: '// i18n: tanı' hiçbir bulguyu bastırmıyor (yorumu silin)";
// i18n: tanı bekçi tanısı
const MARKER_NO_REASON: &str = "i18n işaretinde gerekçe yok ('// i18n: tanı <gerekçe>')";
// i18n: tanı bekçi tanısı
const MARKER_UNKNOWN: &str = "bilinmeyen i18n işareti (beklenen '// i18n: tanı <gerekçe>')";

/// Dosyanın türü: CLI ikilisi ve proc-macro'da Türkçe harfli sabitler muaf (raporlanır).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileClass {
    Library,
    CliBinary,
    ProcMacro,
}

/// Belirteçlenmiş dosya: `(köke göreli yol, tam yol, belirteçler)`.
type ParsedFile = (String, PathBuf, Vec<Token>);

/// Bekçi yapılandırması.
#[derive(Debug, Clone)]
pub struct SourceGuard {
    root: PathBuf,
    ui_calls: Vec<CallSpec>,
    diagnostic_calls: Vec<CallSpec>,
    allow_file: PathBuf,
}

/// `root` altındaki her `src/**/*.rs`'i varsayılan ayarlarla tarar (izin listesi
/// `root/i18n-allow.txt`).
pub fn source_guard(root: impl AsRef<Path>) -> Report {
    SourceGuard::new(root).run()
}

impl SourceGuard {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_owned();
        let allow_file = root.join("i18n-allow.txt");
        Self {
            root,
            ui_calls: DEFAULT_UI_CALLS
                .iter()
                .filter_map(|s| CallSpec::parse(s))
                .collect(),
            diagnostic_calls: Vec::new(),
            allow_file,
        }
    }

    /// UI çağrı listesine ekler (repo'ya özgü kurucular): `Tür::ad`, `ad` ya da `alıcı.ad`.
    /// Geçersiz kalıp yok sayılır.
    pub fn add_ui_calls(mut self, calls: &[&str]) -> Self {
        self.ui_calls
            .extend(calls.iter().filter_map(|s| CallSpec::parse(s)));
        self
    }

    /// Repo genelinde argümanları tanı metni sayılan çağrılar (`Tür::ad`, `ad`, `alıcı.ad`).
    /// Kısa/genel adlar (`err`, `fail`) her dosyada eşleşir; tek dosyaya özgü yardımcılar için
    /// izin listesindeki `dosya:çağrı()` biçimi daha dar ve tercih edilir. Geçersiz kalıp
    /// yok sayılır.
    pub fn add_diagnostic_calls(mut self, calls: &[&str]) -> Self {
        self.diagnostic_calls
            .extend(calls.iter().filter_map(|s| CallSpec::parse(s)));
        self
    }

    /// İzin listesi dosyası (varsayılan `root/i18n-allow.txt`; yoksa boş).
    pub fn allow_file(mut self, path: impl AsRef<Path>) -> Self {
        self.allow_file = path.as_ref().to_owned();
        self
    }

    pub fn run(&self) -> Report {
        let mut report = Report::default();
        let mut allow = self.read_allow_list(&mut report);
        let mut files = Vec::new();
        collect(&self.root, &self.root, false, &mut files, 0);
        files.sort();
        // Önce tüm dosyaları belirteçle; `#[cfg(test)] mod x;` ile bağlanan dosyalar test.
        let (parsed, comments): (Vec<ParsedFile>, Vec<Vec<LineComment>>) = files
            .into_iter()
            .filter_map(|(rel, path)| {
                let text = std::fs::read_to_string(&path).ok()?;
                let (toks, comments) = lex(&text);
                Some(((rel, path, toks), comments))
            })
            .unzip();
        let test_files = test_only_files(&parsed);
        let mut crates = CrateKinds::default();
        for ((rel, path, toks), comments) in parsed.iter().zip(&comments) {
            if test_files.contains(path) {
                continue;
            }
            let local: Vec<CallSpec> = allow
                .calls
                .get(rel)
                .map(|v| v.iter().map(|c| c.spec.clone()).collect())
                .unwrap_or_default();
            let scan = self.scan_file(toks, comments, &local);
            if let Some(entries) = allow.calls.get_mut(rel) {
                for k in &scan.used_local_calls {
                    if let Some(e) = entries.get_mut(*k) {
                        e.entry.used = true;
                    }
                }
            }
            let class = if is_cli_binary(rel) {
                FileClass::CliBinary
            } else if crates.is_proc_macro(&self.root, path) {
                FileClass::ProcMacro
            } else {
                FileClass::Library
            };
            let (hits, exempt) = split_exempt(scan.hits, class);
            if exempt > 0 {
                report.notes.push(
                    ExemptNote {
                        rel,
                        class,
                        n: exempt,
                    }
                    .to_string(),
                );
            }
            let (hits, marker_errors) =
                FileScan::resolve_markers(hits, &scan.markers, scan.marker_errors);
            for (line, message) in marker_errors {
                report.error(rel, line, message);
            }
            for h in hits {
                let line_key = (rel.clone(), h.line);
                let file_key = (rel.clone(), 0);
                if let Some(e) = allow.lines.get_mut(&line_key) {
                    e.used = true;
                } else if let Some(e) = allow.lines.get_mut(&file_key) {
                    e.used = true;
                } else if let Some(e) = allow.dir_for(rel) {
                    e.used = true;
                } else {
                    report.errors.push(Finding {
                        file: rel.clone(),
                        line: h.line,
                        message: h.kind.to_string(),
                    });
                }
            }
        }
        let scanned: BTreeSet<&str> = parsed
            .iter()
            .filter(|(_, path, _)| !test_files.contains(path))
            .map(|(rel, _, _)| rel.as_str())
            .collect();
        allow.report_unused(&mut report, &scanned);
        report
    }

    /// Tek dosyanın metni (testler): bulgular, izin listesi uygulanmadan. CLI yolu kuralı
    /// (`src/main.rs`, `src/bin/**`) uygulanır; proc-macro tespiti `Cargo.toml` ister, yapılmaz.
    /// İşaret (`// i18n: tanı`) hataları da bulgu olarak döner; satıra göre sıralı.
    pub fn scan_text(&self, rel: &str, text: &str) -> Vec<Finding> {
        let (toks, comments) = lex(text);
        let scan = self.scan_file(&toks, &comments, &[]);
        let class = if is_cli_binary(rel) {
            FileClass::CliBinary
        } else {
            FileClass::Library
        };
        let (hits, _) = split_exempt(scan.hits, class);
        let (hits, marker_errors) =
            FileScan::resolve_markers(hits, &scan.markers, scan.marker_errors);
        let mut out: Vec<Finding> = hits
            .into_iter()
            .map(|h| Finding {
                file: rel.to_owned(),
                line: h.line,
                message: h.kind.to_string(),
            })
            .chain(marker_errors.into_iter().map(|(line, m)| Finding {
                file: rel.to_owned(),
                line,
                message: m.to_owned(),
            }))
            .collect();
        out.sort_by_key(|f| f.line);
        out
    }

    /// `local`: dosyaya özgü tanı çağrıları (izin listesinden); kullanılanlar raporlanır.
    /// `comments`: `// i18n:` işaretleri.
    fn scan_file(&self, toks: &[Token], comments: &[LineComment], local: &[CallSpec]) -> FileScan {
        let n = toks.len();
        // `skip`: kesin tanı bölgesi (öznitelik, test, Debug/hata Display gövdesi, log/tanı
        // makrosu, `t!`, `.expect`). `diag`: tanı metni taşıyan çağrı argümanı (hata türü
        // kurucusu, tanı çağrısı) — UI kurucusunun argümanı içindeyse UI bulgusu yine verilir
        // (`Label::new(t, XError::A("Kaydedilemedi"))`).
        let mut skip = vec![false; n];
        let mut diag = vec![false; n];
        let mut ui = vec![false; n];
        let mut owner: Vec<Option<usize>> = vec![None; n];
        let mut error_impls: Vec<(usize, usize)> = Vec::new();
        let errors = error_types(toks);
        let is_error_ty = |ty: &str| error_like_name(ty) || errors.contains(ty);

        let mut i = 0usize;
        while i < n {
            // Öznitelik: `#[..]` / `#![..]` — hepsi muaf (`#[error("..")]` dahil).
            if punct_at(toks, i, b'#') {
                let br = if punct_at(toks, i + 1, b'!') {
                    i + 2
                } else {
                    i + 1
                };
                if open_at(toks, br, b'[') {
                    let close = toks[br].close;
                    mark(&mut skip, br, close + 1);
                    if is_test_attr(toks, br + 1, close) {
                        let end = item_end(toks, close + 1);
                        mark(&mut skip, close + 1, end + 1);
                    }
                    i = br + 1;
                    continue;
                }
            }
            if let Some(name) = ident_at(toks, i) {
                if name == "impl"
                    && let Some(block) = parse_impl(toks, i)
                {
                    let error_self = block.self_ty.is_some_and(is_error_ty);
                    // Debug her zaman tanı; Display yalnızca hata/uyarı türünde (başka türün
                    // Display'i `to_string()` ile arayüze gidebilir: `Label::new(&eylem.to_string())`).
                    let fmt_exempt = match block.trait_name {
                        Some("Debug") => true,
                        Some("Display") => error_self,
                        _ => false,
                    };
                    if fmt_exempt {
                        mark(&mut skip, block.open, block.close + 1);
                    }
                    if error_self {
                        error_impls.push((block.open, block.close));
                    }
                }
                // Makro çağrısı: `ad!(..)`.
                if punct_at(toks, i + 1, b'!')
                    && let Some(open) = open_any_at(toks, i + 2)
                    && EXCLUDED_MACROS.contains(&name)
                {
                    mark(&mut skip, i + 2, toks[open].close + 1);
                }
                // İzin listesindeki makro (`dosya:s!()`): argümanları o izne ait.
                if punct_at(toks, i + 1, b'!')
                    && let Some(open) = open_any_at(toks, i + 2)
                    && let Some(k) = local.iter().position(|c| c.matches(toks, i))
                {
                    let end = (toks[open].close + 1).min(n);
                    for o in owner.iter_mut().take(end).skip(open) {
                        o.get_or_insert(k);
                    }
                }
                if open_at(toks, i + 1, b'(') {
                    let args = (i + 1, toks[i + 1].close + 1);
                    // `.expect(..)`.
                    if name == "expect" && i > 0 && punct_at(toks, i - 1, b'.') {
                        mark(&mut skip, args.0, args.1);
                    }
                    if self.ui_calls.iter().any(|c| c.matches(toks, i)) {
                        mark(&mut ui, args.0, args.1);
                    }
                    if self.diagnostic_calls.iter().any(|c| c.matches(toks, i)) {
                        mark(&mut diag, args.0, args.1);
                    }
                    if let Some(k) = local.iter().position(|c| c.matches(toks, i)) {
                        for o in owner.iter_mut().take(args.1.min(n)).skip(args.0) {
                            o.get_or_insert(k);
                        }
                    }
                }
                // Hata türü kurucusu: `XError::Varyant(..)` / `{..}`, `io::Error::new(..)`,
                // hata türünün `impl` bloğunda `Self::Varyant(..)`.
                if (open_at(toks, i + 1, b'(') || open_at(toks, i + 1, b'{'))
                    && i >= 3
                    && path_sep(toks, i)
                    && let Some(ty) = ident_at(toks, i - 3)
                    && (is_error_ty(ty)
                        || (ty == "Self" && error_impls.iter().any(|(o, c)| *o < i && i < *c)))
                {
                    mark(&mut diag, i + 1, toks[i + 1].close + 1);
                }
            }
            i += 1;
        }

        let mut out = FileScan::default();
        let covered = marker_coverage(toks, comments, &mut out);
        for (k, t) in toks.iter().enumerate() {
            let Tok::Str(s) = &t.tok else { continue };
            if skip[k] {
                continue;
            }
            // UI kurucusuna giden metin: tanı çağrısı/hata kurucusu/izin çağrısı içinde de
            // bulgu; yalnızca makine biçimli dizge (yol, kimlik, ikon adı) muaf.
            if ui[k] && has_text(s) && !machine_shaped(s) {
                out.hits.push(Hit {
                    line: t.line,
                    kind: HitKind::Ui,
                    marker: None,
                });
                continue;
            }
            if !s.contains(TURKISH) || diag[k] {
                continue;
            }
            if let Some(o) = owner[k] {
                out.used_local_calls.insert(o);
                continue;
            }
            out.hits.push(Hit {
                line: t.line,
                kind: HitKind::Turkish,
                marker: covered[k],
            });
        }
        out
    }

    fn read_allow_list(&self, report: &mut Report) -> AllowList {
        let shown = self
            .allow_file
            .strip_prefix(&self.root)
            .unwrap_or(&self.allow_file)
            .display()
            .to_string();
        let mut allow = AllowList {
            shown: shown.clone(),
            ..AllowList::default()
        };
        let Ok(text) = std::fs::read_to_string(&self.allow_file) else {
            return allow;
        };
        for (n, raw) in text.lines().enumerate() {
            let at = n + 1;
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (loc, reason) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let Some(key) = parse_allow_key(loc) else {
                report.error(
                    &shown,
                    at,
                    "biçim 'dosya:satır|dosya:çağrı()|dosya:*|dizin/**  gerekçe' değil",
                );
                continue;
            };
            if reason.trim().is_empty() {
                report.error(&shown, at, "gerekçe yok");
                continue;
            }
            let entry = AllowEntry { at, used: false };
            let duplicate = match key {
                AllowKey::Line(file, l) => {
                    if l == 0 {
                        report.notes.push(format!(
                            "{shown}:{at}: uyarı: '{file}:*' tüm dosyayı muaf tutar; dosyaya \
                             eklenen yeni UI metni de kaçar (dosya:çağrı() ya da dosya:satır \
                             tercih edin)"
                        ));
                    }
                    allow.lines.insert((file, l), entry).is_some()
                }
                AllowKey::Dir(dir) => {
                    report.notes.push(format!(
                        "{shown}:{at}: uyarı: '{dir}/**' dizindeki tüm dosyaları muaf tutar; \
                         eklenen yeni UI metni de kaçar (dosya:çağrı(), dosya:satır ya da \
                         '// i18n: tanı' tercih edin)"
                    ));
                    allow.dirs.insert(dir, entry).is_some()
                }
                AllowKey::Call(file, spec, raw) => {
                    let list = allow.calls.entry(file).or_default();
                    let dup = list.iter().any(|c| c.spec == spec);
                    if !dup {
                        list.push(CallAllow { spec, raw, entry });
                    }
                    dup
                }
            };
            if duplicate {
                report.error(&shown, at, format!("yinelenen izin {loc}"));
            }
        }
        allow
    }
}

#[derive(Debug, Clone, Copy)]
struct AllowEntry {
    /// İzin dosyasındaki satır.
    at: usize,
    used: bool,
}

#[derive(Debug)]
struct CallAllow {
    spec: CallSpec,
    raw: String,
    entry: AllowEntry,
}

#[derive(Debug, Default)]
struct AllowList {
    shown: String,
    /// `(dosya, satır)`; satır 0 = tüm dosya (`dosya:*`).
    lines: BTreeMap<(String, usize), AllowEntry>,
    /// Dosya → o dosyada argümanları tanı metni olan çağrılar.
    calls: BTreeMap<String, Vec<CallAllow>>,
    /// `dizin/**`: dizin (sonda `/` yok) → giriş.
    dirs: BTreeMap<String, AllowEntry>,
}

impl AllowList {
    /// `rel`'i kapsayan en uzun (en dar) `dizin/**` girişi.
    fn dir_for(&mut self, rel: &str) -> Option<&mut AllowEntry> {
        self.dirs
            .iter_mut()
            .filter(|(d, _)| in_dir(rel, d))
            .max_by_key(|(d, _)| d.len())
            .map(|(_, e)| e)
    }

    /// `scanned`: taranan (test dışı) dosyalar. Var olan bir dosyanın kullanılmayan `dosya:*`
    /// satırı geçiş döneminde not olur (yapısal muafiyet onu gereksiz kıldı; tüketici repolar
    /// framework'ü güncelleyince testleri kırılmasın); yok olan dosyanınki hatadır.
    fn report_unused(self, report: &mut Report, scanned: &BTreeSet<&str>) {
        let shown = self.shown;
        for ((file, line), e) in self.lines {
            if !e.used {
                if line == 0 && scanned.contains(file.as_str()) {
                    report.notes.push(format!(
                        "{shown}:{}: gereksiz izin {file}:* — yapısal muafiyet kapsıyor (satırı silin)",
                        e.at
                    ));
                    continue;
                }
                let loc = if line == 0 {
                    "*".to_owned()
                } else {
                    line.to_string()
                };
                report.error(
                    &shown,
                    e.at,
                    format!("kullanılmayan izin {file}:{loc} (satırı silin)"),
                );
            }
        }
        // `dizin/**`: `dosya:*` ile aynı — dizinde taranan dosya varsa not, yoksa hata.
        for (dir, e) in self.dirs.iter().filter(|(_, e)| !e.used) {
            if scanned.iter().any(|f| in_dir(f, dir)) {
                report.notes.push(format!(
                    "{shown}:{}: gereksiz izin {dir}/** — yapısal muafiyet kapsıyor (satırı silin)",
                    e.at
                ));
            } else {
                report.error(
                    &shown,
                    e.at,
                    format!("kullanılmayan izin {dir}/** — eşleşen dosya yok (satırı silin)"),
                );
            }
        }
        for (file, calls) in self.calls {
            for c in calls.iter().filter(|c| !c.entry.used) {
                report.error(
                    &shown,
                    c.entry.at,
                    format!("kullanılmayan izin {file}:{}() (satırı silin)", c.raw),
                );
            }
        }
    }
}

enum AllowKey {
    /// Satır 0 = tüm dosya.
    Line(String, usize),
    Call(String, CallSpec, String),
    /// `dizin/**` (sonda `/` yok).
    Dir(String),
}

/// `dosya.rs:12`, `dosya.rs:*`, `dosya.rs:Tür::ad()` / `dosya.rs:ad()` / `dosya.rs:alıcı.ad()`,
/// `dizin/**`. Yol köke göreli ve düz olmalı ([`plain_rel_path`]); başka glob biçimi yok.
fn parse_allow_key(loc: &str) -> Option<AllowKey> {
    if let Some(dir) = loc.strip_suffix("/**") {
        return plain_rel_path(dir).then(|| AllowKey::Dir(dir.to_owned()));
    }
    let idx = loc.rfind(".rs:")?;
    let file = loc.get(..idx + 3)?.to_owned();
    if !plain_rel_path(&file) {
        return None;
    }
    let rest = loc.get(idx + 4..)?;
    if rest == "*" {
        return Some(AllowKey::Line(file, 0));
    }
    if let Some(call) = rest.strip_suffix("()") {
        return CallSpec::parse(call).map(|s| AllowKey::Call(file, s, call.to_owned()));
    }
    rest.parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .map(|n| AllowKey::Line(file, n))
}

/// Köke göreli düz yol: boş değil, `/` ile başlamaz, boş/`.`/`..` parçası yok, glob ya da kaçış
/// karakteri (`*?[]{}\`) yok.
fn plain_rel_path(p: &str) -> bool {
    !p.is_empty()
        && !p.contains(['*', '?', '[', ']', '{', '}', '\\'])
        && p.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// `rel` dosyası `dir` dizininin (alt dizinleri dahil) altında mı.
fn in_dir(rel: &str, dir: &str) -> bool {
    rel.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
}

/// CLI/proc-macro dosyasında Türkçe harfli bulgular muaf: `(kalan bulgular, muaf sayısı)`.
fn split_exempt(hits: Vec<Hit>, class: FileClass) -> (Vec<Hit>, usize) {
    if class == FileClass::Library {
        return (hits, 0);
    }
    let before = hits.len();
    let kept: Vec<Hit> = hits.into_iter().filter(|h| h.kind == HitKind::Ui).collect();
    let exempt = before - kept.len();
    (kept, exempt)
}

/// CLI/proc-macro muafiyet notu.
struct ExemptNote<'a> {
    rel: &'a str,
    class: FileClass,
    n: usize,
}

impl fmt::Display for ExemptNote<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { rel, class, n } = self;
        // i18n: tanı muafiyet notu (bekçi tanısı, geliştiriciye)
        match class {
            FileClass::CliBinary => write!(
                f,
                "{rel}: CLI ikilisi — {n} Türkçe harfli sabit muaf (CLI çıktısı çevrilmez, \
                 ADR-0010 §4; UI kurucuları yine denetlenir)"
            ),
            FileClass::ProcMacro => write!(
                f,
                "{rel}: proc-macro crate'i — {n} Türkçe harfli sabit muaf (derleme zamanı tanısı)"
            ),
            FileClass::Library => Ok(()),
        }
    }
}

/// `src/main.rs` ya da `src/bin/**` (köke göreli yol, `/` ayraçlı).
fn is_cli_binary(rel: &str) -> bool {
    rel == "src/main.rs"
        || rel.ends_with("/src/main.rs")
        || rel.starts_with("src/bin/")
        || rel.contains("/src/bin/")
}

/// Dizin → crate'i proc-macro mu (en yakın `Cargo.toml`'a göre; önbellekli).
#[derive(Debug, Default)]
struct CrateKinds(BTreeMap<PathBuf, bool>);

impl CrateKinds {
    fn is_proc_macro(&mut self, root: &Path, file: &Path) -> bool {
        let mut dir = file.parent();
        while let Some(d) = dir {
            if let Some(v) = self.0.get(d) {
                return *v;
            }
            let manifest = d.join("Cargo.toml");
            if manifest.is_file() {
                let v = std::fs::read_to_string(&manifest).is_ok_and(|t| {
                    t.lines().any(|l| {
                        let l: String = l.chars().filter(|c| !c.is_whitespace()).collect();
                        l == "proc-macro=true" || l == "proc_macro=true"
                    })
                });
                self.0.insert(d.to_owned(), v);
                return v;
            }
            if d == root {
                break;
            }
            dir = d.parent();
        }
        false
    }
}

/// `// i18n: tanı <gerekçe>` işaretlerinin kapsamı (belirteç → `out.markers` indeksi); geçerli
/// işaretler `out.markers`'a, bozuklar `out.marker_errors`'a yazılır.
/// - Satır sonu yorumu: o satırda *başlayan* belirteçler (önceki satırda başlayan çok satırlı
///   dizge kapsanmaz).
/// - Tek başına satırdaki yorum: hemen sonraki deyim ([`statement_end`]).
///
/// Aynı dizgeyi iki işaret kapsarsa ilki kazanır; ikincisi gereksiz işaret hatası olur.
fn marker_coverage(
    toks: &[Token],
    comments: &[LineComment],
    out: &mut FileScan,
) -> Vec<Option<usize>> {
    let mut covered = vec![None; toks.len()];
    for c in comments {
        let Some(reason) = marker_reason(&c.body) else {
            out.marker_errors.push((c.line, MARKER_UNKNOWN));
            continue;
        };
        if reason.is_empty() {
            out.marker_errors.push((c.line, MARKER_NO_REASON));
            continue;
        }
        let m = out.markers.len();
        out.markers.push(c.line);
        let next = c.next_tok.min(toks.len());
        let range = if c.standalone {
            next..statement_end(toks, next)
        } else {
            let mut from = next;
            while from > 0 && toks[from - 1].line == c.line {
                from -= 1;
            }
            from..next
        };
        for k in covered.get_mut(range).unwrap_or_default() {
            k.get_or_insert(m);
        }
    }
    covered
}

/// `i18n:` sonrası metin `tanı <gerekçe>` ise kırpılmış gerekçe (boş olabilir); başka işaret
/// (`tani`, `tanılar`, `izin`) `None`.
fn marker_reason(body: &str) -> Option<&str> {
    let rest = body.trim().strip_prefix("tanı")?; // i18n: tanı işaret anahtar sözcüğü
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

/// `start`'ta başlayan deyimin bitişi (dışlayan): derinlik 0'daki ilk `;` ya da `,` (dahil),
/// kapsayan parantezin kapanışı (hariç) ya da `{..}` bloğu ardından yeni satırda `else`/`.`/`?`
/// dışında bir belirteç (`if x { .. }`, `fn`, `match` kolu `=> { .. }`) — hangisi önce gelirse.
/// Parantez grupları bütün atlanır.
fn statement_end(toks: &[Token], start: usize) -> usize {
    let n = toks.len();
    let mut j = start;
    while j < n {
        match toks[j].tok {
            Tok::Open(c) => {
                let close = toks[j].close;
                if close >= n {
                    return n;
                }
                j = close + 1;
                if c == b'{'
                    && let Some(next) = toks.get(j)
                    && next.line > toks[close].line
                    && !matches!(&next.tok, Tok::Punct(b'.' | b'?'))
                    && ident_at(toks, j) != Some("else")
                {
                    return j;
                }
            }
            Tok::Close(_) => return j,
            Tok::Punct(b';' | b',') => return j + 1,
            _ => j += 1,
        }
    }
    n
}

fn mark(v: &mut [bool], from: usize, to: usize) {
    let n = v.len();
    for x in v.iter_mut().take(to.min(n)).skip(from) {
        *x = true;
    }
}

fn ident_at(toks: &[Token], i: usize) -> Option<&str> {
    match toks.get(i).map(|t| &t.tok) {
        Some(Tok::Ident(s)) => Some(s.as_str()),
        _ => None,
    }
}

fn punct_at(toks: &[Token], i: usize, c: u8) -> bool {
    matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Punct(p)) if *p == c)
}

fn open_at(toks: &[Token], i: usize, c: u8) -> bool {
    matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Open(p)) if *p == c)
}

fn open_any_at(toks: &[Token], i: usize) -> Option<usize> {
    matches!(toks.get(i).map(|t| &t.tok), Some(Tok::Open(_))).then_some(i)
}

/// `i`'deki kimlikten önce `::` var mı.
fn path_sep(toks: &[Token], i: usize) -> bool {
    i >= 2 && punct_at(toks, i - 1, b':') && punct_at(toks, i - 2, b':')
}

/// Bir `impl` öğesi: gövde sınırları, trait adı (`impl Trait for`) ve kendi türünün adı.
struct ImplBlock<'a> {
    open: usize,
    close: usize,
    trait_name: Option<&'a str>,
    self_ty: Option<&'a str>,
}

/// `i`'deki `impl` bir öğe başlığıysa (`impl<..> Trait for Tür<..> where .. {`) ayrıştırır.
/// Konum `impl Trait`'leri (`-> impl Display`, `x: impl Debug`) öğe değildir: önceki belirteç
/// öğe sınırı (`}`/`]`/`;`/`{`/anahtar kelime) değilse ya da `for` yoksa trait sayılmaz.
fn parse_impl(toks: &[Token], i: usize) -> Option<ImplBlock<'_>> {
    if i > 0 {
        match &toks[i - 1].tok {
            Tok::Close(_) | Tok::Punct(b';') | Tok::Open(b'{') | Tok::Ident(_) => {}
            _ => return None,
        }
    }
    let mut angle = 0usize;
    let mut seen_for = false;
    let mut in_where = false;
    let mut head: Vec<&str> = Vec::new();
    let mut self_ty: Vec<&str> = Vec::new();
    let mut j = i + 1;
    while j < toks.len() {
        match &toks[j].tok {
            Tok::Punct(b'<') => angle += 1,
            Tok::Punct(b'>') if !punct_at(toks, j.wrapping_sub(1), b'-') => {
                angle = angle.saturating_sub(1);
            }
            Tok::Open(b'{') if angle == 0 => {
                let close = toks[j].close.min(toks.len().saturating_sub(1));
                let trait_name = if seen_for { head.last().copied() } else { None };
                let ty = if seen_for {
                    self_ty.last()
                } else {
                    head.last()
                };
                return Some(ImplBlock {
                    open: j,
                    close,
                    trait_name,
                    self_ty: ty.copied(),
                });
            }
            Tok::Open(_) => {
                j = toks[j].close.max(j) + 1;
                continue;
            }
            Tok::Close(_) | Tok::Punct(b';') if angle == 0 => return None,
            Tok::Ident(s) if angle == 0 && !in_where => match s.as_str() {
                "for" if !seen_for => seen_for = true,
                "where" => in_where = true,
                "dyn" | "unsafe" | "const" => {}
                s if seen_for => self_ty.push(s),
                s => head.push(s),
            },
            _ => {}
        }
        j += 1;
    }
    None
}

/// Adı hata/uyarı türü gibi mi (`..Error`, `..Warning`).
fn error_like_name(ty: &str) -> bool {
    ty.ends_with("Error") || ty.ends_with("Warning")
}

/// Dosyada hata türü olarak tanımlanan türler: `impl (std::error::)Error for Tür` ya da
/// `#[derive(.., Error, ..)]` (`thiserror::Error` dahil) taşıyan `struct`/`enum Tür`.
fn error_types(toks: &[Token]) -> BTreeSet<&str> {
    let mut out = BTreeSet::new();
    let mut derive_error = false;
    let mut i = 0usize;
    while i < toks.len() {
        match &toks[i].tok {
            Tok::Punct(b'#') if open_at(toks, i + 1, b'[') => {
                let close = toks[i + 1].close.min(toks.len());
                if ident_at(toks, i + 2) == Some("derive")
                    && (i + 2..close).any(|k| ident_at(toks, k) == Some("Error"))
                {
                    derive_error = true;
                }
                i = close.max(i + 1) + 1;
                continue;
            }
            Tok::Ident(kw) if kw == "impl" => {
                derive_error = false;
                if let Some(b) = parse_impl(toks, i)
                    && b.trait_name == Some("Error")
                    && let Some(ty) = b.self_ty
                {
                    out.insert(ty);
                }
            }
            Tok::Ident(kw) if derive_error && (kw == "struct" || kw == "enum") => {
                if let Some(ty) = ident_at(toks, i + 1) {
                    out.insert(ty);
                }
                derive_error = false;
            }
            // Öznitelik ile öğe arasında yalnızca görünürlük (`pub`, `pub(crate)`) olabilir.
            Tok::Ident(kw) if kw == "pub" => {}
            Tok::Open(b'(') if i > 0 && ident_at(toks, i - 1) == Some("pub") => {
                i = toks[i].close.max(i) + 1;
                continue;
            }
            _ => derive_error = false,
        }
        i += 1;
    }
    out
}

/// `cfg(test)` / `cfg(all(test, ..))` / `test` özniteliği (`not`/`any` içermeyen: onlar test
/// dışında da derlenebilir).
fn is_test_attr(toks: &[Token], from: usize, to: usize) -> bool {
    let idents: Vec<&str> = toks
        .get(from..to.min(toks.len()))
        .unwrap_or_default()
        .iter()
        .filter_map(|t| match &t.tok {
            Tok::Ident(s) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    match idents.first() {
        Some(&"test") => idents.len() == 1,
        Some(&"cfg") => {
            idents.contains(&"test") && !idents.contains(&"not") && !idents.contains(&"any")
        }
        _ => false,
    }
}

/// `from`'dan başlayan öğenin son belirteci: sonraki öznitelikler atlanır, derinlik 0'daki
/// ilk `{..}` ya da `;`.
fn item_end(toks: &[Token], from: usize) -> usize {
    let mut j = from;
    while j < toks.len() {
        match toks[j].tok {
            Tok::Open(b'{') => return toks[j].close.min(toks.len().saturating_sub(1)),
            Tok::Open(_) => j = toks[j].close.max(j) + 1,
            Tok::Punct(b';') => return j,
            _ => j += 1,
        }
    }
    toks.len().saturating_sub(1)
}

/// `#[cfg(test)] mod ad;` ile bağlanan dosyalar ve onların alt modülleri.
fn test_only_files(parsed: &[ParsedFile]) -> BTreeSet<PathBuf> {
    // Dosya → bildirdiği (`mod x;`) alt modül yolları, ayrıca test olanlar.
    let mut children: BTreeMap<&Path, Vec<PathBuf>> = BTreeMap::new();
    let mut tests: BTreeSet<PathBuf> = BTreeSet::new();
    for (_, path, toks) in parsed {
        let dir = module_dir(path);
        let mut test_attr = false;
        let mut path_attr: Option<String> = None;
        let mut i = 0usize;
        while i < toks.len() {
            match &toks[i].tok {
                Tok::Punct(b'#') if open_at(toks, i + 1, b'[') => {
                    let close = toks[i + 1].close;
                    if is_test_attr(toks, i + 2, close) {
                        test_attr = true;
                    }
                    if ident_at(toks, i + 2) == Some("path")
                        && let Some(Tok::Str(p)) = toks.get(i + 4).map(|t| &t.tok)
                    {
                        path_attr = Some(p.clone());
                    }
                    i = close.max(i) + 1;
                    continue;
                }
                Tok::Ident(kw) if kw == "mod" => {
                    if let (Some(name), true) = (ident_at(toks, i + 1), punct_at(toks, i + 2, b';'))
                    {
                        let files = match path_attr.take() {
                            Some(p) => vec![path.parent().unwrap_or(Path::new("")).join(p)],
                            None => vec![
                                dir.join(format!("{name}.rs")),
                                dir.join(name).join("mod.rs"),
                            ],
                        };
                        if test_attr {
                            tests.extend(files.iter().cloned());
                        }
                        children.entry(path.as_path()).or_default().extend(files);
                    }
                }
                _ => {}
            }
            if !matches!(toks[i].tok, Tok::Ident(ref s) if s == "pub" || s == "mod") {
                test_attr = false;
                path_attr = None;
            }
            i += 1;
        }
    }
    // Test modülünün alt modülleri de test (sabit noktaya kadar).
    loop {
        let before = tests.len();
        let add: Vec<PathBuf> = tests
            .iter()
            .filter_map(|t| children.get(t.as_path()))
            .flatten()
            .cloned()
            .collect();
        tests.extend(add);
        if tests.len() == before {
            break;
        }
    }
    tests
}

/// `mod x;` dosyalarının arandığı dizin: `lib.rs`/`main.rs`/`mod.rs` için kendi dizini,
/// `a.rs` için `a/`.
fn module_dir(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(Path::new("")).to_owned();
    match path.file_stem().and_then(|s| s.to_str()) {
        Some("lib" | "main" | "mod") | None => parent,
        Some(stem) => parent.join(stem),
    }
}

/// `src` altındaki `.rs` dosyaları: `(köke göreli yol, tam yol)`.
fn collect(root: &Path, dir: &Path, in_src: bool, out: &mut Vec<(String, PathBuf)>, depth: usize) {
    if depth > 32 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        if ft.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            collect(root, &path, in_src || name == "src", out, depth + 1);
        } else if ft.is_file() && in_src && name.ends_with(".rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
}

/// Arayüz metni olamayacak, makine biçimli dizge (UI kurucusu argümanında muaf): boşluksuz
/// ASCII ve şunlardan biri — mutlak/göreli yol ya da URL (`/usr/..`, `./x`, `~/x`,
/// `file://..`); küçük harfli `snake_case` kimlik; küçük harfli noktalı kimlik
/// (`org.nuxut.shell`, `bar.height`; boş parça yok); `-symbolic` ikon adı. Tek sözcük
/// (`Settings`, `ok`), `Kaydet/İptal`, `on/off`, `Wi-Fi`, `e.g.` metin sayılır.
fn machine_shaped(s: &str) -> bool {
    if s.is_empty() || !s.is_ascii() || s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let lower = !s.chars().any(|c| c.is_ascii_uppercase());
    let word = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    };
    ["/", "./", "../", "~/"].iter().any(|p| s.starts_with(p))
        || s.split_once("://").is_some_and(|(scheme, _)| word(scheme))
        || (lower && s.contains('_') && word(s))
        || (lower && s.contains('.') && s.split('.').all(word))
        || (s.ends_with("-symbolic") && word(s))
}

/// Biçim yer tutucuları (`{ad}`, `{0:>8}`) dışında harf var mı; `{{`/`}}` kaçışları metindir.
fn has_text(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '}' if depth == 0 && chars.peek() == Some(&'}') => {
                chars.next();
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            c if depth == 0 && c.is_alphabetic() => return true,
            _ => {}
        }
    }
    false
}
