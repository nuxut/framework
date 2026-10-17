//! Süreç geneli katalog ve kaynak kaydı.
//!
//! **Okuma kilitsiz:** [`current`] iş parçacığı yerel önbellekten `Arc` klonlar; yalnızca
//! [`set_current`] sonrası (nesil sayacı değişince) bir kez okuma kilidi alınır. Dil
//! değişimi nadir, okuma her metin kurulumunda: yeni crate (arc-swap) gerekmez.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use nuxut_config::locale::LocaleConfig;

use crate::{Catalog, LanguagePrefs, Source};

static CURRENT: RwLock<Option<Arc<Catalog>>> = RwLock::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(1);
static EMPTY: OnceLock<Arc<Catalog>> = OnceLock::new();

thread_local! {
    static CACHE: RefCell<Option<(u64, Arc<Catalog>)>> = const { RefCell::new(None) };
}

struct Registry {
    sources: Vec<Source>,
    /// Son uygulanan `[locale]`; `None` = henüz yapılandırılmadı.
    settings: Option<LocaleConfig>,
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry { sources: Vec::new(), settings: None });

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    // Zehirlenme: kayıt yalnızca Vec/Option; panik yarım durum bırakmaz.
    REGISTRY.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Süreç geneli katalogu değiştirir (sonraki [`current`] çağrıları yenisini görür).
pub fn set_current(catalog: Arc<Catalog>) {
    let mut w = CURRENT.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *w = Some(catalog);
    // Yazımdan SONRA artır: eski nesli okuyan iş parçacığı en kötü yeni değeri eski nesille
    // önbelleğe alır ve bir sonraki çağrıda yeniden okur (asla eski değerde takılmaz).
    GENERATION.fetch_add(1, Ordering::Release);
}

/// Süreç geneli katalog; hiç kurulmadıysa boş katalog (anahtarlar döner).
pub fn current() -> Arc<Catalog> {
    let generation = GENERATION.load(Ordering::Acquire);
    CACHE.with(|cache| {
        if let Some((g, cat)) = &*cache.borrow()
            && *g == generation
        {
            return cat.clone();
        }
        let cat = CURRENT
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .unwrap_or_else(|| EMPTY.get_or_init(|| Arc::new(Catalog::empty())).clone());
        *cache.borrow_mut() = Some((generation, cat.clone()));
        cat
    })
}

/// Yeni domain kaydında çağrılan kanca (runtime: kullanıcı çeviri dizini izleyicisini
/// günceller). Kayıt kilidi dışında çağrılır; kanca kayıt işlevlerini çağırabilir.
type RegisterHook = Arc<dyn Fn() + Send + Sync>;

static REGISTER_HOOK: Mutex<Option<RegisterHook>> = Mutex::new(None);

/// Yeni (daha önce kayıtlı olmayan) bir domain kaydedilince çağrılacak kancayı kurar
/// (`None`: kaldırır). Runtime çalışma başladıktan sonraki kayıtları izlemek için kullanır;
/// aynı domain'in yeniden kaydı kancayı çağırmaz.
pub fn set_register_hook(hook: Option<RegisterHook>) {
    *REGISTER_HOOK.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = hook;
}

/// Bir domain'i kaydeder (aynı domain'in eski kaydının yerine). Katalogu yeniden kurmaz:
/// [`init`], [`configure`] ya da [`reload`] kurar. Domain yeniyse kayıt kancası
/// ([`set_register_hook`]) çağrılır.
pub fn register(source: Source) {
    let added = {
        let mut reg = registry();
        match reg.sources.iter_mut().find(|s| s.domain == source.domain) {
            Some(slot) => {
                *slot = source;
                false
            }
            None => {
                reg.sources.push(source);
                true
            }
        }
    };
    if added {
        let hook = REGISTER_HOOK.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        if let Some(hook) = hook {
            hook();
        }
    }
}

/// Kayıtlı kaynaklar.
pub fn registered() -> Vec<Source> {
    registry().sources.clone()
}

/// Uygulama girişi: kaynakları kaydeder, `framework.toml [locale]`'i okur, katalogu kurar ve
/// süreç geneli yapar. `run`'dan önce (uygulama metin kurmadan) çağrılır; runtime aynı
/// ayarla açılırsa yeniden kurmaz.
pub fn init(sources: &[Source]) -> Arc<Catalog> {
    for s in sources {
        register(*s);
    }
    let cfg = nuxut_config::locale::load_locale_config();
    let catalog = build(&cfg);
    registry().settings = Some(cfg);
    catalog
}

/// `[locale]` ayarını uygular. Ayar son uygulanandan farklıysa `true` döner ve (kayıtlı
/// kaynak varsa) katalog yeniden kurulup süreç geneli yapılır. Runtime açılışta ve
/// framework.toml değişince çağırır; `true` ise uygulamaya `locale_changed` gider.
pub fn configure(cfg: &LocaleConfig) -> bool {
    let has_sources = {
        let mut reg = registry();
        if reg.settings.as_ref() == Some(cfg) {
            return false;
        }
        reg.settings = Some(cfg.clone());
        !reg.sources.is_empty()
    };
    if has_sources {
        build(cfg);
    }
    true
}

/// Etkin sayı locale'i: `NUMBER` / `{ $n }` ayırıcıları ve yüzde kalıbı bununla biçimlenir;
/// uygulama kendi biçimlediği sayıda (`Locale::format_percent`, `format_number`) aynısını
/// kullanır. framework.toml `[locale] formats` ayarını yansıtır (`Locale::numeric_from_env`
/// yansıtmaz). Sıra: süreç kataloğu kuruluysa onun locale'i ([`Catalog::numeric`]); değilse
/// son `init`/`configure` ayarı ([`crate::formats_locale`], `auto` ise ortam); hiç
/// yapılandırılmadıysa ortam (`LC_ALL` > `LC_NUMERIC` > `LANG`). Dönen değer kopyadır (`Clone`
/// ucuz değil): sık çağrılan yolda değil, katalog değişiminde (`locale_changed`) alınıp saklanır.
pub fn numeric_locale() -> nuxut_locale::Locale {
    let installed = CURRENT.read().unwrap_or_else(std::sync::PoisonError::into_inner).is_some();
    if installed {
        return current().numeric().clone();
    }
    let formats = registry().settings.as_ref().map(|cfg| cfg.formats.clone());
    formats
        .and_then(|f| crate::formats_locale(&f))
        .unwrap_or_else(nuxut_locale::Locale::numeric_from_env)
}

/// Etkin saat/tarih locale'i: son `init`/`configure` ayarının `formats` + `hour_cycle` değeri
/// ([`crate::time_locale_for`]); hiç yapılandırılmadıysa ortam (`LC_ALL` > `LC_TIME` > `LANG`,
/// locale verisinin 12/24 saati). `hour_cycle` değişince runtime `locale_changed` gönderir;
/// uygulama değeri orada yeniden alıp saklar (`Clone` ucuz değil).
pub fn time_locale() -> nuxut_locale::Locale {
    let settings = registry().settings.clone();
    match settings {
        Some(cfg) => crate::time_locale_for(&cfg),
        None => nuxut_locale::Locale::from_env(),
    }
}

/// Kayıtlı kaynaklar ve son ayarla yeniden kurar (ör. kullanıcı çeviri dosyası değişti).
pub fn reload() -> Arc<Catalog> {
    let cfg = registry().settings.clone().unwrap_or_default();
    build(&cfg)
}

fn build(cfg: &LocaleConfig) -> Arc<Catalog> {
    let sources = registered();
    let catalog = Arc::new(Catalog::load(&sources, &LanguagePrefs::from_config(cfg)));
    set_current(catalog.clone());
    catalog
}
