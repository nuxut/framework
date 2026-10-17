//! SVG rasterleştirme (`resvg`). SVG **güvenilmeyen girdidir** (tepsi/bildirim ikonları,
//! ikon temaları, kullanıcı dosyaları); bu yüzden:
//!
//! - Dış kaynak yok: `<image href>` / `<feImage href>` dosya yolu ya da URL ile hiçbir şey
//!   yüklemez (`resources_dir` yok, dize çözücüsü her zaman `None`). Yalnızca gömülü `data:`
//!   PNG/JPEG/GIF/WebP kabul edilir; onlar da bayt ve **başlıktaki boyut** sınırından geçer.
//!   Gömülü SVG (`image/svg+xml`) reddedilir (iç içe ayrıştırma yok).
//! - Metin/font desteği derlenmez (`resvg` `text` özelliği kapalı): SVG'den font dosyası
//!   okunamaz, sistem font veritabanı taranmaz.
//! - Tek ayrıştırma: `roxmltree` belgesi kendi düğüm sınırıyla bir kez ayrıştırılır ve usvg'ye
//!   AYNI belge verilir (`from_xmltree`); sayılan ile işlenen aynıdır. `<!ENTITY>` yok.
//! - Dönüştürmeden önce: derinlik ve başvuru açılımı (`href` ile her başvuru, aynı `id`'nin en
//!   büyüğü, işaretçi kopyaları — CSS ile bağlananlar dahil) üstten tahmin edilir.
//! - Rasterleştirmeden önce dönüştürülmüş ağaç **nihai cihaz pikselinde** denetlenir (alt
//!   köklerde dönüşüm zinciri çarpılır, kırpma/maske zincirleri sonuna dek gezilir): düğüm, yol
//!   segmenti, filtre işlemi, bulanıklık, eşzamanlı ara tampon belleği, gradyan durakları,
//!   kesikli çizgi parçaları, desen karosu ve tek bir piksel-işlem bütçesi (dolgu, katman,
//!   gömülü görsel çözümü, morfoloji/evrişim/türbülans maliyeti).
//! - Rasterleştirme tuvali piksel sınırlıdır.

use std::collections::HashMap;
use std::sync::Arc;

use resvg::usvg::{self, ImageHrefResolver, ImageKind};

use crate::{DecodeOptions, Fit, Image, ImageError, TargetSize};

/// Kabul edilen en büyük SVG girdisi.
pub const MAX_SVG_BYTES: usize = 8 << 20;
/// XML düğümü (öğe + metin + yorum) üst sınırı.
pub const MAX_SVG_NODES: u32 = 200_000;
/// En derin öğe iç içeliği.
pub const MAX_SVG_DEPTH: u32 = 128;
/// `<use>` başvuruları açıldıktan sonraki etkin öğe sayısı.
pub const MAX_SVG_EXPANDED: u64 = 250_000;
/// Gömülü (`data:`) raster görselin sıkıştırılmış ve çözülmüş sınırları.
pub const MAX_EMBEDDED_BYTES: usize = 4 << 20;
pub const MAX_EMBEDDED_SIDE: u32 = 4096;
pub const MAX_EMBEDDED_PIXELS: u64 = 16_000_000;
/// Rasterleştirme tuvalinin mutlak sınırları.
pub const MAX_RASTER_SIDE: u32 = 16_384;
pub const MAX_RASTER_PIXELS: u64 = 64_000_000;
/// Hedef verilmezse doğal boyut bu kenara sınırlanır.
const DEFAULT_MAX_SIDE: u32 = 1024;

/// SVG'yi `target` kutusuna rasterleştirir. `Contain`/`Cover`'da en-boy oranı korunur ve
/// çıktı ölçeklenmiş SVG boyutundadır (kutuya doldurma yok); `Fill` tam kutu boyutudur.
/// SVG vektör olduğundan küçükse büyütülür. Kırpılmış `Cover` ([`TargetSize::cover_crop`]):
/// tuval kutunun kendisidir, taşan eksen ortadan kırpılır (çıktı ≤ kutu; tam görüntü ayrılmaz).
pub fn render_svg(bytes: &[u8], target: TargetSize) -> Result<Image, ImageError> {
    rasterize(&parse(bytes)?, target)
}

pub(crate) fn decode_svg(bytes: &[u8], opts: &DecodeOptions) -> Result<Image, ImageError> {
    let tree = parse(bytes)?;
    let target = opts.target.unwrap_or_else(|| {
        let s = tree.size();
        let w = (s.width().ceil() as u32).clamp(1, DEFAULT_MAX_SIDE);
        let h = (s.height().ceil() as u32).clamp(1, DEFAULT_MAX_SIDE);
        TargetSize::new(w, h, Fit::Contain)
    });
    opts.limits.check_dimensions(target.width, target.height)?;
    let img = rasterize(&tree, target)?;
    opts.limits.check_dimensions(img.width(), img.height())?;
    Ok(img)
}

fn parse(bytes: &[u8]) -> Result<usvg::Tree, ImageError> {
    if bytes.len() > MAX_SVG_BYTES {
        return Err(ImageError::FileTooLarge { size: bytes.len() as u64, limit: MAX_SVG_BYTES as u64 });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ImageError::Svg("UTF-8 değil".into()))?;
    if text.contains("<!ENTITY") {
        return Err(ImageError::Svg("XML varlık bildirimi (<!ENTITY>) desteklenmiyor".into()));
    }
    // Tek ayrıştırma: sınırlar ayrıştırıcının kendisinde (nodes_limit) ve denetlenen belge,
    // usvg'nin işlediği belgenin AYNISIDIR (from_xmltree) — ön tarama ile ayrıştırıcı farkı yok.
    let xml = roxmltree::ParsingOptions { allow_dtd: true, nodes_limit: MAX_SVG_NODES, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(text, xml).map_err(|e| xml_error(&e))?;
    check_structure(&doc)?;
    let opts = usvg_options();
    usvg::Tree::from_xmltree(&doc, &opts).map_err(|e| match e {
        usvg::Error::ParsingFailed(x) => xml_error(&x),
        other => ImageError::Svg(other.to_string()),
    })
}

/// usvg seçenekleri — dış kaynak İÇİN TEK NOKTA (ADR-0007): `resources_dir` yok (göreli yol
/// hiçbir dizine çözülmez) ve `<image>`/`<feImage>` `href` dize çözücüsü her şemada (`file:`,
/// yol, göreli yol, `http(s):`, `ftp:`, `//host`, …) **hiçbir şey açmadan** `None` döner;
/// usvg'nin varsayılan dize çözücüsü (kütüphanedeki tek dosya okuması) hiç kurulmaz. Yalnızca
/// `data:` URI'leri [`embedded_image`]'a gider: PNG/JPEG/GIF/WebP, bayt + başlık boyutu
/// sınırıyla; gömülü SVG (`image/svg+xml`, iç içe ayrıştırma) reddedilir. `text` özelliği
/// derlenmediği için font veritabanı da yok. `options_never_resolve_external_sources` testi
/// bu sözleşmeyi bu fonksiyon üzerinden doğrular.
pub(crate) fn usvg_options() -> usvg::Options<'static> {
    usvg::Options {
        resources_dir: None,
        image_href_resolver: ImageHrefResolver {
            resolve_data: Box::new(|mime, data, _| embedded_image(mime, data)),
            resolve_string: Box::new(|href, _| {
                tracing::debug!("SVG dış kaynağı yok sayıldı ({} bayt)", href.len());
                None
            }),
        },
        ..usvg::Options::default()
    }
}

/// Ayrıştırıcı iletisi belgeden ad/değer alıntılayabilir (varlık, öznitelik, ad alanı):
/// yalnızca konum taşınır (loglarda kullanıcı içeriği yok).
fn xml_error(e: &roxmltree::Error) -> ImageError {
    ImageError::Svg(format!("XML ayrıştırılamadı (konum {})", e.pos()))
}

const XLINK: &str = "http://www.w3.org/1999/xlink";
/// Açılım tahmininde bir öğe sayılan `data:` URI baytı (sınır: ~64 MiB toplam çözme).
const DATA_URI_BYTES_PER_ELEMENT: u64 = 256;

/// Ayrıştırma sonrası, dönüştürme öncesi yapı denetimi: derinlik ve başvuru açılımı.
///
/// usvg `<use>`'u ve işaretçileri (marker) kopyalayarak açar; `href` ile başvurulan içerik
/// (`<use>`, `<feImage>`, `<pattern>`/gradyan kalıtımı) her kullanımda yeniden işlenir. Bu
/// yüzden etkin boyut, başvurular ve işaretçi kopyaları dahil, **üstten** tahmin edilir:
/// - aynı `id`'li birden çok öğe varsa en büyüğü sayılır (hangisini seçerse seçsin);
/// - belgede `<marker>` varsa her şeklin köşe sayısı × en büyük işaretçi eklenir (CSS ile
///   bağlanan işaretçiler dahil).
fn check_structure(doc: &roxmltree::Document<'_>) -> Result<(), ImageError> {
    // Düğüm kimlikleri belge sırasındadır: ebeveyn her zaman çocuğundan önce gelir.
    let mut depth = vec![0u32; doc.descendants().count() + 1];
    let mut ids: HashMap<&str, Vec<roxmltree::NodeId>> = HashMap::new();
    let mut markers = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        let d = n.parent().map_or(0, |p| depth[p.id().get_usize()]) + 1;
        if d > MAX_SVG_DEPTH {
            return Err(ImageError::Svg(format!("iç içe derinlik {MAX_SVG_DEPTH}'i aşıyor")));
        }
        depth[n.id().get_usize()] = d;
        if let Some(id) = n.attribute("id") {
            ids.entry(id.trim()).or_default().push(n.id());
        }
        if n.tag_name().name() == "marker" {
            markers.push(n.id());
        }
    }
    let mut cx = Expand { doc, ids, memo: HashMap::new(), stack: Vec::new(), marker: 0 };
    for m in markers {
        if let Some(node) = doc.get_node(m) {
            let w = cx.weight(node)?;
            cx.marker = cx.marker.max(w);
        }
    }
    // İşaretçi ağırlığı belli olunca bellek sıfırlanır (şekiller işaretçiyle yeniden sayılır).
    cx.memo.clear();
    let total = cx.weight(doc.root_element())?;
    if total > MAX_SVG_EXPANDED {
        return Err(ImageError::Svg(format!("başvuru/işaretçi açılımı {total} öğe (sınır {MAX_SVG_EXPANDED})")));
    }
    Ok(())
}

struct Expand<'a, 'i> {
    doc: &'a roxmltree::Document<'i>,
    ids: HashMap<&'a str, Vec<roxmltree::NodeId>>,
    memo: HashMap<roxmltree::NodeId, u64>,
    stack: Vec<roxmltree::NodeId>,
    /// En büyük işaretçinin ağırlığı (0 = belgede işaretçi yok).
    marker: u64,
}

impl<'a, 'i> Expand<'a, 'i> {
    /// Öğenin etkin boyutu (bellekli). Döngü ve aşırı zincir reddedilir.
    fn weight(&mut self, node: roxmltree::Node<'a, 'i>) -> Result<u64, ImageError> {
        if let Some(&w) = self.memo.get(&node.id()) {
            return Ok(w);
        }
        if self.stack.contains(&node.id()) {
            return Err(ImageError::Svg("başvuru döngüsü".into()));
        }
        if self.stack.len() as u32 > MAX_SVG_DEPTH * 4 {
            return Err(ImageError::Svg("başvuru zinciri çok uzun".into()));
        }
        self.stack.push(node.id());
        let mut w: u64 = 1;
        for child in node.children().filter(|c| c.is_element()) {
            w = w.saturating_add(self.weight(child)?);
            if w > MAX_SVG_EXPANDED {
                break;
            }
        }
        let href = node.attribute((XLINK, "href")).or_else(|| node.attribute("href"));
        // usvg `data:` URI'sini her örnekte (her `<use>` kopyasında) yeniden çözer: bayt
        // da açılıma sayılır, böylece bir görselin binlerce kopyası dönüştürmeden önce durur.
        if let Some(h) = href.filter(|h| !h.trim_start().starts_with('#')) {
            w = w.saturating_add(h.len() as u64 / DATA_URI_BYTES_PER_ELEMENT);
        }
        if let Some(id) = href.map(str::trim).and_then(|h| h.strip_prefix('#')) {
            let targets = self.ids.get(id.trim()).cloned().unwrap_or_default();
            let mut most = 0;
            for t in targets.into_iter().filter_map(|t| self.doc.get_node(t)) {
                most = most.max(self.weight(t)?);
            }
            w = w.saturating_add(most);
        }
        if self.marker > 0 {
            w = w.saturating_add(vertex_estimate(node).saturating_mul(self.marker));
        }
        self.stack.pop();
        self.memo.insert(node.id(), w);
        Ok(w)
    }
}

/// Şeklin işaretçi alabilecek köşe sayısının üst tahmini (sayı adedi; komut harfleri dahil).
fn vertex_estimate(node: roxmltree::Node<'_, '_>) -> u64 {
    let data = match node.tag_name().name() {
        "path" => node.attribute("d"),
        "polyline" | "polygon" => node.attribute("points"),
        "line" => return 2,
        _ => return 0,
    };
    let Some(d) = data else { return 0 };
    // Her sayı ya da komut en fazla bir köşe başlatır.
    let mut count = 0u64;
    let mut in_num = false;
    for c in d.bytes() {
        let num = c.is_ascii_digit() || c == b'.';
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            count += 1;
        }
        if num && !in_num {
            count += 1;
        }
        in_num = num;
    }
    count
}

/// Rasterleştirme öncesi ağaç denetimi (dönüştürülmüş usvg ağacı). resvg'nin çizim yolunu
/// izler; her sınır **nihai cihaz pikselinde** uygulanır:
///
/// - Alt kökler (kırpma yolu, maske, desen, `feImage`) usvg'de kendi köklerine göre
///   `abs_transform` taşır; cihaz dönüşümü, başvuran öğenin dönüşüm zinciriyle çarpılarak
///   bulunur (`Target::base`), resvg'nin kullandığıyla aynı kuralla (desende
///   `transform ∘ patternTransform` ölçeği, kırpmada `transform ∘ clip.transform`).
/// - Kırpma/maske zincirlerinin **tamamı** gezilir (usvg `subroots` yalnızca iki seviye verir).
/// - Bellek: bir anda yaşayan ara tamponlar (katman, efekt sonuçları, kırpma/maske tuvali,
///   desen karosu) gerçek boyutlarıyla toplanır; resvg katmanı tuvalin 5×5 katına kadar büyütür.
/// - İş: tüm tampon ayırma/birleştirme, yol dolgusu, gömülü görsel çözümü ve filtre işlemi
///   (piksel × işlem başı maliyet: morfoloji yarıçapı, evrişim çekirdeği, türbülans oktavı…)
///   tek bir piksel-işlem bütçesinden düşülür.
struct Audit {
    /// Tuval kenarları; resvg katman/filtre bölgesini kenar başına 5 katla sınırlar.
    max_w: f64,
    max_h: f64,
    nodes: u64,
    segments: u64,
    primitives: u64,
    stops: u64,
    dashes: f64,
    work: f64,
    /// Gömülü görselin çözülmüş piksel sayısı (aynı veri her çizimde yeniden çözülür).
    images: HashMap<usize, f64>,
}

/// Bir çizim hedefi: tuval, katman, desen karosu ya da filtre bölgesi.
#[derive(Clone, Copy)]
struct Target {
    /// Alt ağaç kökünden cihaz pikseline dönüşüm (ana ağaçta tuval ölçeği).
    base: usvg::Transform,
    /// Bu hedef çizilirken eşzamanlı yaşayan ara tamponların baytı.
    live: f64,
    /// Hedef tamponun piksel alanı (içine yapılan dolgu bununla sınırlı).
    area: f64,
}

/// Rasterleştirmede eşzamanlı ara tamponların (katman, efekt, kırpma/maske, desen) toplam
/// bellek bütçesi.
pub const MAX_LAYER_BYTES: u64 = 512 << 20;
/// Bir rasterleştirmenin toplam piksel-işlem bütçesi (ayırma, birleştirme, dolgu, filtre).
pub const MAX_RASTER_WORK: f64 = 4_000_000_000.0;
pub const MAX_PATH_SEGMENTS: u64 = 2_000_000;
pub const MAX_FILTER_PRIMITIVES: u64 = 256;
/// Cihaz pikselinde en büyük bulanıklık sapması (akıl sınırı: resvg kutu bulanıklığı kayan
/// pencereyle çalışır, maliyeti yarıçapa değil bölgeye bağlıdır ve iş bütçesinden düşülür).
pub const MAX_BLUR_PX: f32 = 4096.0;
pub const MAX_GRADIENT_STOPS: u64 = 65_536;
pub const MAX_DASHES: f64 = 1_000_000.0;
/// Kırpma/maske zincirinin derinliği.
pub const MAX_CLIP_DEPTH: u32 = 8;

/// Sonlu olmayan (taşmış ya da NaN) ölçü sınırsız sayılır.
/// Sınır içinde mi; NaN (ör. ∞ × 0) sınır dışıdır.
fn within(v: f64, limit: f64) -> bool {
    v <= limit
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v.max(0.0) } else { f64::INFINITY }
}

fn fail<T>(msg: impl Into<String>) -> Result<T, ImageError> {
    Err(ImageError::Svg(msg.into()))
}

impl Audit {
    fn spend(&mut self, px: f64) -> Result<(), ImageError> {
        self.work += finite(px);
        if !within(self.work, MAX_RASTER_WORK) {
            return fail("rasterleştirme işi bütçeyi aşıyor");
        }
        Ok(())
    }

    fn check_live(bytes: f64) -> Result<(), ImageError> {
        if !within(bytes, MAX_LAYER_BYTES as f64) {
            return fail("ara katmanlar bellek bütçesini aşıyor");
        }
        Ok(())
    }

    /// Cihaz uzayındaki dikdörtgenin resvg'nin ayıracağı katman boyutu (kenar başına tuvalin
    /// 5 katıyla sınırlı). Hesaplanamayan kutu en büyük katman sayılır.
    fn layer_dims(&self, r: Option<usvg::NonZeroRect>) -> (f64, f64) {
        match r {
            Some(b) => (
                (finite(f64::from(b.width())).ceil() + 4.0).min(self.max_w),
                (finite(f64::from(b.height())).ceil() + 4.0).min(self.max_h),
            ),
            None => (self.max_w, self.max_h),
        }
    }

    fn group(&mut self, g: &usvg::Group, t: Target) -> Result<(), ImageError> {
        let dev = t.base.pre_concat(g.abs_transform());
        let mut inner = t;
        if g.should_isolate() {
            clip_depth(g.clip_path(), g.mask())?;
            let (w, h) = self.layer_dims(g.layer_bounding_box().transform(dev));
            let area = w * h;
            let live = t.live + area * 4.0;
            Self::check_live(live)?;
            // Katman ayrılır, çizilir ve hedefe birleştirilir.
            self.spend(area * 2.0)?;
            inner = Target { base: t.base, live, area };
            self.filters(g, dev, live, (w, h))?;
            // Kırpma ve maske: her zincir halkası katman boyunda bir tuval + alfa maskesi.
            let effect = Target { base: dev, live: live + area * 5.0, area };
            Self::check_live(effect.live)?;
            let mut clip = g.clip_path();
            while let Some(c) = clip {
                self.spend(area * 3.0)?;
                self.group(c.root(), Target { base: dev.pre_concat(c.transform()), ..effect })?;
                clip = c.clip_path();
            }
            let mut mask = g.mask();
            while let Some(m) = mask {
                self.spend(area * 3.0)?;
                self.group(m.root(), effect)?;
                mask = m.mask();
            }
        }
        for child in g.children() {
            self.node(child, inner)?;
        }
        Ok(())
    }

    /// Filtreler: bölge katmanla sınırlı; her işlemin sonucu filtre bitene dek yaşar.
    fn filters(&mut self, g: &usvg::Group, dev: usvg::Transform, live: f64, layer: (f64, f64)) -> Result<(), ImageError> {
        let (sx, sy) = dev.get_scale();
        for f in g.filters() {
            let (rw, rh) = self.layer_dims(f.rect().transform(dev));
            let (rw, rh) = (rw.min(layer.0), rh.min(layer.1));
            let region = rw * rh;
            let prims = f.primitives().len() as f64;
            let results = live + region * 4.0 * (prims + 1.0);
            Self::check_live(results)?;
            for p in f.primitives() {
                self.primitives += 1;
                if self.primitives > MAX_FILTER_PRIMITIVES {
                    return fail("çok fazla filtre işlemi");
                }
                use usvg::filter::Kind;
                let blur = |x: f32, y: f32| x * sx > MAX_BLUR_PX || y * sy > MAX_BLUR_PX;
                // Çıktı pikseli başına yaklaşık işlem.
                let cost = match p.kind() {
                    Kind::GaussianBlur(b) => {
                        if blur(b.std_dev_x().get(), b.std_dev_y().get()) {
                            return fail("bulanıklık yarıçapı çok büyük");
                        }
                        6.0
                    }
                    Kind::DropShadow(d) => {
                        if blur(d.std_dev_x().get(), d.std_dev_y().get()) {
                            return fail("gölge bulanıklığı çok büyük");
                        }
                        8.0
                    }
                    Kind::Morphology(m) => {
                        let kx = (2.0 * finite(f64::from(m.radius_x().get() * sx)).ceil()).min(rw);
                        let ky = (2.0 * finite(f64::from(m.radius_y().get() * sy)).ceil()).min(rh);
                        kx.max(1.0) * ky.max(1.0)
                    }
                    Kind::ConvolveMatrix(c) => f64::from(c.matrix().columns()) * f64::from(c.matrix().rows()),
                    Kind::Turbulence(tb) => 2.0 * f64::from(tb.num_octaves()).max(1.0),
                    Kind::DiffuseLighting(_) | Kind::SpecularLighting(_) => 8.0,
                    Kind::Image(img) => {
                        // Kendi alt ağacı filtre bölgesinde, katman ölçeğiyle çizilir.
                        let base = usvg::Transform::from_scale(sx, sy);
                        self.group(img.root(), Target { base, live: results, area: region })?;
                        1.0
                    }
                    _ => 2.0,
                };
                self.spend(region * cost)?;
            }
        }
        Ok(())
    }

    fn node(&mut self, n: &usvg::Node, t: Target) -> Result<(), ImageError> {
        self.nodes += 1;
        if self.nodes > MAX_SVG_EXPANDED {
            return fail("çok fazla düğüm");
        }
        match n {
            usvg::Node::Group(g) => self.group(g, t),
            usvg::Node::Path(p) => self.path(p, t),
            usvg::Node::Image(img) => self.image(img, t),
            // `text` özelliği derlenmez; gelirse denetlenmemiş içerik sayılır.
            usvg::Node::Text(_) => fail("metin düğümü desteklenmiyor"),
        }
    }

    /// Cihaz uzayında kutunun kapladığı alan, hedef tamponla sınırlı.
    fn cover(r: usvg::Rect, dev: usvg::Transform, t: Target) -> f64 {
        r.transform(dev).map_or(t.area, |b| finite(f64::from(b.width()) * f64::from(b.height())).min(t.area))
    }

    fn path(&mut self, p: &usvg::Path, t: Target) -> Result<(), ImageError> {
        let dev = t.base.pre_concat(p.abs_transform());
        let pts = p.data().points();
        self.segments += pts.len() as u64;
        if self.segments > MAX_PATH_SEGMENTS {
            return fail("çok fazla yol segmenti");
        }
        let passes = f64::from(u8::from(p.fill().is_some()) + u8::from(p.stroke().is_some()));
        self.spend(Self::cover(p.stroke_bounding_box(), dev, t) * passes)?;
        let paints = [p.fill().map(|f| f.paint()), p.stroke().map(|s| s.paint())];
        for paint in paints.into_iter().flatten() {
            match paint {
                usvg::Paint::LinearGradient(g) => self.stops += g.stops().len() as u64,
                usvg::Paint::RadialGradient(g) => self.stops += g.stops().len() as u64,
                usvg::Paint::Pattern(pat) => {
                    // resvg karoyu `transform ∘ patternTransform` ölçeğinde ayrı bir tuvale çizer.
                    let (sx, sy) = dev.pre_concat(pat.transform()).get_scale();
                    let r = pat.rect();
                    let (tw, th) = (finite(f64::from(r.width() * sx)).round(), finite(f64::from(r.height() * sy)).round());
                    let tile = tw * th;
                    if !within(tile, MAX_RASTER_PIXELS as f64) {
                        return fail("desen karosu raster bütçesini aşıyor");
                    }
                    let live = t.live + tile * 4.0;
                    Self::check_live(live)?;
                    self.spend(tile)?;
                    let base = usvg::Transform::from_scale(sx, sy);
                    self.group(pat.root(), Target { base, live, area: tile })?;
                }
                usvg::Paint::Color(_) => {}
            }
        }
        if self.stops > MAX_GRADIENT_STOPS {
            return fail("çok fazla gradyan durağı");
        }
        if let Some(stroke) = p.stroke()
            && let Some(dash) = stroke.dasharray()
        {
            let period: f32 = dash.iter().sum();
            if period > 0.0 {
                let length: f32 = pts.windows(2).map(|w| ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt()).sum();
                self.dashes += f64::from(length / period) * dash.len() as f64;
                if !self.dashes.is_finite() || self.dashes > MAX_DASHES {
                    return fail("kesikli çizgi çok fazla parça üretir");
                }
            }
        }
        Ok(())
    }

    /// Gömülü görsel her çizimde yeniden çözülür (resvg önbelleklemez).
    fn image(&mut self, img: &usvg::Image, t: Target) -> Result<(), ImageError> {
        let data = match img.kind() {
            ImageKind::PNG(d) | ImageKind::JPEG(d) | ImageKind::GIF(d) | ImageKind::WEBP(d) => d,
            ImageKind::SVG(_) => return fail("gömülü SVG desteklenmiyor"),
        };
        let key = Arc::as_ptr(data) as usize;
        let decoded = match self.images.get(&key) {
            Some(&px) => px,
            None => {
                let px = image::ImageReader::new(std::io::Cursor::new(data.as_slice()))
                    .with_guessed_format()
                    .ok()
                    .and_then(|r| r.into_dimensions().ok())
                    .map_or(MAX_EMBEDDED_PIXELS as f64, |(w, h)| f64::from(w) * f64::from(h));
                self.images.insert(key, px);
                px
            }
        };
        let dev = t.base.pre_concat(img.abs_transform());
        let view = usvg::Rect::from_xywh(0.0, 0.0, img.size().width(), img.size().height());
        let cover = view.map_or(t.area, |r| Self::cover(r, dev, t));
        Self::check_live(t.live + decoded * 4.0)?;
        self.spend(decoded * 2.0 + cover)
    }
}

/// Zincir derinliği gezmeden önce sınırlanır (döngü usvg'de kırılır, yine de üst sınır).
fn clip_depth(clip: Option<&usvg::ClipPath>, mask: Option<&usvg::Mask>) -> Result<(), ImageError> {
    let (mut c, mut d) = (clip, 0u32);
    while let Some(cp) = c {
        d += 1;
        if d > MAX_CLIP_DEPTH {
            return fail("kırpma zinciri çok derin");
        }
        c = cp.clip_path();
    }
    let (mut m, mut e) = (mask, 0u32);
    while let Some(mk) = m {
        e += 1;
        if e > MAX_CLIP_DEPTH {
            return fail("maske zinciri çok derin");
        }
        m = mk.mask();
    }
    Ok(())
}

/// Gömülü `data:` görsel: yalnızca raster biçimler, bayt ve başlık boyutu sınırlı.
fn embedded_image(mime: &str, data: Arc<Vec<u8>>) -> Option<ImageKind> {
    if data.len() > MAX_EMBEDDED_BYTES {
        tracing::debug!("SVG gömülü görseli çok büyük ({} bayt)", data.len());
        return None;
    }
    let format = image::guess_format(&data).ok()?;
    let declared_ok = matches!(mime, "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" | "text/plain");
    if !declared_ok {
        return None;
    }
    let reader = image::ImageReader::with_format(std::io::Cursor::new(data.as_slice()), format);
    let (w, h) = reader.into_dimensions().ok()?;
    if w == 0 || h == 0 || w > MAX_EMBEDDED_SIDE || h > MAX_EMBEDDED_SIDE || u64::from(w) * u64::from(h) > MAX_EMBEDDED_PIXELS {
        tracing::debug!("SVG gömülü görseli sınır dışı: {w}x{h}");
        return None;
    }
    match format {
        image::ImageFormat::Png => Some(ImageKind::PNG(data)),
        image::ImageFormat::Jpeg => Some(ImageKind::JPEG(data)),
        image::ImageFormat::Gif => Some(ImageKind::GIF(data)),
        image::ImageFormat::WebP => Some(ImageKind::WEBP(data)),
        _ => None,
    }
}

fn rasterize(tree: &usvg::Tree, target: TargetSize) -> Result<Image, ImageError> {
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if !(sw > 0.0 && sh > 0.0) {
        return Err(ImageError::Empty);
    }
    let (tw, th) = (target.width.max(1) as f32, target.height.max(1) as f32);
    let (sx, sy) = match target.fit {
        Fit::Fill => (tw / sw, th / sh),
        Fit::Contain => {
            let s = (tw / sw).min(th / sh);
            (s, s)
        }
        Fit::Cover => {
            let s = (tw / sw).max(th / sh);
            (s, s)
        }
    };
    let (mut w, mut h) = ((sw * sx).round(), (sh * sy).round());
    // Kırpılmış Cover: tuval kutunun kendisi (taşan eksen ortadan kırpılır, çıktı ≤ kutu);
    // ölçeklenmiş tam görüntü hiç ayrılmaz.
    let (mut ox, mut oy) = (0.0, 0.0);
    if target.crops() && w.is_finite() && h.is_finite() {
        let (cw, ch) = (w.min(tw), h.min(th));
        (ox, oy) = ((w - cw) * 0.5, (h - ch) * 0.5);
        (w, h) = (cw, ch);
    }
    // Uç en-boy oranlı SVG + Cover devasa bir tuval isteyebilir (f32 → u32 doyar).
    if !(w.is_finite() && h.is_finite()) || w > MAX_RASTER_SIDE as f32 || h > MAX_RASTER_SIDE as f32 {
        return Err(ImageError::TooLarge { width: w.min(u32::MAX as f32) as u32, height: h.min(u32::MAX as f32) as u32 });
    }
    let (w, h) = ((w as u32).max(1), (h as u32).max(1));
    if u64::from(w) * u64::from(h) > MAX_RASTER_PIXELS {
        return Err(ImageError::TooLarge { width: w, height: h });
    }
    let (cw, ch) = (f64::from(w), f64::from(h));
    let mut audit = Audit {
        max_w: cw * 5.0,
        max_h: ch * 5.0,
        nodes: 0,
        segments: 0,
        primitives: 0,
        stops: 0,
        dashes: 0.0,
        work: 0.0,
        images: HashMap::new(),
    };
    let base = usvg::Transform::from_row(sx, 0.0, 0.0, sy, -ox, -oy);
    audit.group(tree.root(), Target { base, live: cw * ch * 4.0, area: cw * ch })?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or(ImageError::TooLarge { width: w, height: h })?;
    resvg::render(tree, base, &mut pixmap.as_mut());
    // tiny-skia pikselleri zaten premultiplied RGBA8.
    Image::from_premultiplied(w, h, pixmap.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="#ff0000"/></svg>"##;

    fn t(w: u32, h: u32) -> TargetSize {
        TargetSize::new(w, h, Fit::Contain)
    }

    #[test]
    fn renders_to_target_keeping_aspect() {
        let img = render_svg(SVG, t(64, 64)).unwrap();
        assert_eq!(img.size(), (64, 32));
        assert_eq!(img.straight_pixel(10, 10), [255, 0, 0, 255]);
    }

    /// Kırpılmış Cover: çıktı tam kutu, taşan eksen ortadan kırpılır (sol yarı kırmızı, sağ
    /// yarı mavi 2:1 SVG → 1:1 kutuda orta şerit).
    #[test]
    fn cover_crop_renders_exact_box_from_center() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="10" height="10" fill="#ff0000"/><rect x="10" width="10" height="10" fill="#0000ff"/></svg>"##;
        let img = render_svg(svg, TargetSize::cover_crop(40, 40)).unwrap();
        assert_eq!(img.size(), (40, 40));
        assert_eq!(img.straight_pixel(5, 20), [255, 0, 0, 255]);
        assert_eq!(img.straight_pixel(35, 20), [0, 0, 255, 255]);
        // Kırpmasız Cover ölçeklenmiş tam görüntüdür.
        assert_eq!(render_svg(svg, TargetSize::new(40, 40, Fit::Cover)).unwrap().size(), (80, 40));
        // Kutudan küçük eksen kırpılmaz (Cover zaten kaplar): 2:1 kutu → tam boy.
        assert_eq!(render_svg(svg, TargetSize::cover_crop(60, 30)).unwrap().size(), (60, 30));
    }

    #[test]
    fn rejects_garbage() {
        assert!(render_svg(b"<svg", t(8, 8)).is_err());
        assert!(render_svg(&[0xff, 0xfe, 0x00], t(8, 8)).is_err());
    }

    fn png_data_uri(w: u32, h: u32) -> String {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 255, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        format!("data:image/png;base64,{}", base64(&png))
    }

    fn base64(b: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
            for i in 0..4 {
                if i <= c.len() {
                    s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    s.push('=');
                }
            }
        }
        s
    }

    fn with_image(href: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><image width="8" height="8" xlink:href="{href}"/></svg>"#
        )
    }

    /// Kod düzeyinde sözleşme (ADR-0007): `resources_dir` yok; dize çözücüsü her
    /// şemada `None` (var olan dosyanın yolu dahil — açmaya çalışmaz); `data:` yalnızca raster ve
    /// sınırlı; gömülü SVG reddedilir.
    #[test]
    fn options_never_resolve_external_sources() {
        let o = usvg_options();
        assert!(o.resources_dir.is_none(), "göreli yol hiçbir dizine çözülmez");
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("x.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 255, 0, 255])).save(&png).unwrap();
        let hrefs = [
            "file:///etc/passwd".to_owned(),
            format!("file://{}", png.display()),
            png.display().to_string(),
            "x.png".into(),
            "../x.png".into(),
            "./x.png".into(),
            "/dev/zero".into(),
            "~/x.png".into(),
            "http://127.0.0.1/a.png".into(),
            "https://example.com/a.png".into(),
            "ftp://example.com/a.png".into(),
            "smb://host/share/a.png".into(),
            "//host/a.png".into(),
            "javascript:alert(1)".into(),
            "jar:file:///x.zip!/a.png".into(),
            String::new(),
        ];
        for href in &hrefs {
            assert!((o.image_href_resolver.resolve_string)(href, &o).is_none(), "{href} çözülmemeli");
        }
        // data: — raster çözülür (sınırla), SVG ve sınır dışı raster reddedilir.
        let small = {
            let mut v = Vec::new();
            image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255])).write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
            v
        };
        assert!((o.image_href_resolver.resolve_data)("image/png", Arc::new(small.clone()), &o).is_some());
        assert!((o.image_href_resolver.resolve_data)("image/svg+xml", Arc::new(SVG.to_vec()), &o).is_none(), "iç içe SVG yok");
        assert!((o.image_href_resolver.resolve_data)("image/svg+xml", Arc::new(small), &o).is_none(), "beyan edilen tür raster değil");
        let huge = {
            let mut v = Vec::new();
            image::RgbaImage::from_pixel(5000, 1, image::Rgba([0, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut v), image::ImageFormat::Png).unwrap();
            v
        };
        assert!((o.image_href_resolver.resolve_data)("image/png", Arc::new(huge), &o).is_none(), "başlık boyutu sınırı");
    }

    #[test]
    fn never_reads_external_files() {
        // Var olan bir dosya: okunsaydı (PNG olarak) çizilirdi.
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("x.png");
        image::RgbaImage::from_pixel(8, 8, image::Rgba([0, 255, 0, 255])).save(&png).unwrap();
        for href in [
            format!("file://{}", png.display()),
            png.display().to_string(),
            "x.png".into(),
            "../x.png".into(),
            "file:///etc/passwd".into(),
            "/dev/zero".into(),
            "https://example.com/a.png".into(),
        ] {
            let img = render_svg(with_image(&href).as_bytes(), t(8, 8)).unwrap();
            assert_eq!(img.straight_pixel(4, 4)[3], 0, "{href} çizilmemeli");
        }
    }

    #[test]
    fn embedded_images_are_bounded() {
        let ok = render_svg(with_image(&png_data_uri(8, 8)).as_bytes(), t(8, 8)).unwrap();
        assert_eq!(ok.straight_pixel(4, 4), [0, 0, 255, 255], "küçük gömülü PNG çizilir");
        // Başlığı 5000x5000 diyen PNG: çözülmeden atlanır.
        let big = render_svg(with_image(&png_data_uri(5000, 1)).as_bytes(), t(8, 8)).unwrap();
        assert_eq!(big.straight_pixel(4, 4)[3], 0);
        // Gömülü SVG reddedilir.
        let sub = format!("data:image/svg+xml;base64,{}", base64(SVG));
        assert_eq!(render_svg(with_image(&sub).as_bytes(), t(8, 8)).unwrap().straight_pixel(4, 4)[3], 0);
    }

    #[test]
    fn rejects_oversized_inputs_and_canvases() {
        let huge = vec![b' '; MAX_SVG_BYTES + 1];
        assert!(matches!(render_svg(&huge, t(8, 8)), Err(ImageError::FileTooLarge { .. })));
        let thin = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="10000"/>"#;
        assert!(matches!(render_svg(thin, TargetSize::new(4000, 4000, Fit::Cover)), Err(ImageError::TooLarge { .. })));
        let vb = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1e30" height="1e30" viewBox="0 0 1e30 1e30"/>"#;
        // Doğal boyutta (hedefsiz) sınırlanır, tuval taşmaz.
        let img = decode_svg(vb, &DecodeOptions::default()).unwrap();
        assert!(img.width() <= DEFAULT_MAX_SIDE && img.height() <= DEFAULT_MAX_SIDE);
        assert!(render_svg(vb, TargetSize::new(u32::MAX, u32::MAX, Fit::Fill)).is_err());
    }

    fn svg(body: &str) -> Vec<u8> {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="64" height="64">{body}</svg>"#).into_bytes()
    }

    fn quick_err(body: &str) {
        let start = std::time::Instant::now();
        let r = render_svg(&svg(body), t(64, 64));
        assert!(matches!(r, Err(ImageError::Svg(_))), "reddedilmeliydi: {:?}", r.map(|i| i.size()));
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "yavaş red: {:?}", start.elapsed());
    }

    #[test]
    fn rejects_marker_multiplication() {
        // 2000 köşeli yol × 300 öğeli işaretçi = 600k kopya.
        let pts: String = (0..2000).map(|i| format!("{} {} ", i % 64, (i * 7) % 64)).collect();
        let marker_body = "<rect width=\"1\" height=\"1\"/>".repeat(300);
        quick_err(&format!(r#"<defs><marker id="m" markerWidth="2" markerHeight="2">{marker_body}</marker></defs><polyline points="{pts}" marker-mid="url(#m)" stroke="red"/>"#));
        // CSS ile bağlanan işaretçi de sayılır.
        quick_err(&format!(r#"<style>polyline {{ marker-mid: url(#m) }}</style><defs><marker id="m">{marker_body}</marker></defs><polyline points="{pts}"/>"#));
    }

    #[test]
    fn duplicate_ids_count_the_largest() {
        // İlk "a" küçük, ikincisi büyük: ayrıştırıcı hangisini seçerse seçsin büyüğü sayılır.
        let big: String = (0..60).map(|_| "<rect width=\"1\" height=\"1\"/>".to_string()).collect();
        let mut s = format!(r#"<defs><g id="a"/><g id="a">{big}</g>"#);
        for i in 1..=4 {
            s.push_str(&format!(r#"<g id="l{i}">"#));
            for _ in 0..20 {
                s.push_str(&format!(r##"<use xlink:href="#{}"/>"##, if i == 1 { "a".to_string() } else { format!("l{}", i - 1) }));
            }
            s.push_str("</g>");
        }
        s.push_str(r##"</defs><use xlink:href="#l4"/>"##);
        quick_err(&s);
    }

    #[test]
    fn cdata_and_comments_do_not_hide_elements() {
        // Yorum/CDATA içindeki "<use" sayılmaz, gerçek öğeler sayılır (aynı ayrıştırıcı).
        let ok = svg(r##"<!-- <use href="#x"/> --><style><![CDATA[ rect { fill: red } ]]></style><rect width="64" height="64"/>"##);
        assert_eq!(render_svg(&ok, t(64, 64)).unwrap().straight_pixel(5, 5), [255, 0, 0, 255]);
    }

    #[test]
    fn rejects_huge_blur_filter_regions_and_layers() {
        quick_err(r#"<filter id="f"><feGaussianBlur stdDeviation="5000"/></filter><rect width="64" height="64" filter="url(#f)"/>"#);
        // Dev filtre bölgesi resvg'de tuvalin 5×5 katına kırpılır: zararsız ve hızlı.
        let start = std::time::Instant::now();
        let region = r#"<filter id="f" x="-100000" y="-100000" width="200000" height="200000" filterUnits="userSpaceOnUse"><feOffset/></filter><rect width="64" height="64" filter="url(#f)"/>"#;
        assert!(render_svg(&svg(region), t(64, 64)).is_ok());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        // 120 iç içe yarı saydam grup: her biri bir ara katman.
        let deep = format!("{}<rect width=\"64\" height=\"64\"/>{}", "<g opacity=\"0.9\">".repeat(120), "</g>".repeat(120));
        let r = render_svg(&svg(&deep), TargetSize::new(4096, 4096, Fit::Contain));
        assert!(matches!(r, Err(ImageError::Svg(_))));
    }

    #[test]
    fn rejects_pattern_and_dash_bombs_and_deep_masks() {
        quick_err(r#"<pattern id="p" width="100000" height="100000" patternUnits="userSpaceOnUse"><rect width="10" height="10"/></pattern><rect width="64" height="64" fill="url(#p)"/>"#);
        quick_err(r#"<path d="M0 0 L64 64 L0 64 L64 0 Z" stroke="red" stroke-dasharray="0.00001"/>"#);
        let mut masks = String::new();
        for i in 0..12 {
            if i == 0 {
                masks.push_str(r#"<mask id="m0"><rect width="64" height="64" fill="white"/></mask>"#);
            } else {
                masks.push_str(&format!(r#"<mask id="m{i}" mask="url(#m{})"><rect width="64" height="64" fill="white"/></mask>"#, i - 1));
            }
        }
        quick_err(&format!(r#"<defs>{masks}</defs><rect width="64" height="64" mask="url(#m11)"/>"#));
    }

    #[test]
    fn rejects_deep_nesting_entities_and_use_bombs() {
        let deep = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8">{}{}</svg>"#,
            "<g>".repeat(MAX_SVG_DEPTH as usize + 5),
            "</g>".repeat(MAX_SVG_DEPTH as usize + 5)
        );
        assert!(render_svg(deep.as_bytes(), t(8, 8)).is_err());

        let lol = br#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;">]><svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><text>&b;</text></svg>"#;
        assert!(render_svg(lol, t(8, 8)).is_err());

        // 10 seviye × 10 başvuru = 10^10 öğe açılımı.
        let mut s = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><defs><rect id="l0" width="1" height="1"/>"#);
        for i in 1..=10 {
            s.push_str(&format!(r#"<g id="l{i}">"#));
            for _ in 0..10 {
                s.push_str(&format!(r##"<use xlink:href="#l{}"/>"##, i - 1));
            }
            s.push_str("</g>");
        }
        s.push_str(r##"</defs><use xlink:href="#l10"/></svg>"##);
        let start = std::time::Instant::now();
        assert!(matches!(render_svg(s.as_bytes(), t(8, 8)), Err(ImageError::Svg(_))));
        assert!(start.elapsed() < std::time::Duration::from_secs(1));

        let cycle = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><g id="a"><use xlink:href="#a"/></g></svg>"##;
        assert!(render_svg(cycle, t(8, 8)).is_err());

        // DOCTYPE'lı ama varlıksız eski ikonlar hâlâ çalışır.
        let doctype = br#"<?xml version="1.0"?><!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#;
        assert!(render_svg(doctype, t(8, 8)).is_ok());
    }

    fn quick_err_at(body: &str, side: u32) {
        let start = std::time::Instant::now();
        let r = render_svg(&svg(body), t(side, side));
        assert!(matches!(r, Err(ImageError::Svg(_))), "reddedilmeliydi: {:?}", r.map(|i| i.size()));
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "yavaş red: {:?}", start.elapsed());
    }

    const BLUR20: &str = r#"<filter id="b"><feGaussianBlur stdDeviation="20"/></filter>"#;
    const BLUR100: &str = r#"<filter id="b"><feGaussianBlur stdDeviation="100"/></filter>"#;

    #[test]
    fn limits_apply_in_device_pixels_through_subroots() {
        // Alt kökün içeriği kendi köküne göre dönüşüm taşır; başvuran grubun scale(64)'ü
        // zincirle çarpılmalı: 100 × 64 = 6400 px bulanıklık.
        quick_err(&format!(
            r#"<defs>{BLUR100}<mask id="m"><rect width="1" height="1" fill="white" filter="url(#b)"/></mask></defs><g transform="scale(64)"><rect width="1" height="1" mask="url(#m)"/></g>"#
        ));
        quick_err(&format!(
            r#"<defs>{BLUR100}<pattern id="p" width="1" height="1" patternUnits="userSpaceOnUse"><rect width="1" height="1" filter="url(#b)"/></pattern></defs><g transform="scale(64)"><rect width="1" height="1" fill="url(#p)"/></g>"#
        ));
        quick_err(&format!(
            r#"<defs>{BLUR100}<clipPath id="c"><rect width="1" height="1"/></clipPath><mask id="m"><g clip-path="url(#c)" filter="url(#b)"><rect width="1" height="1" fill="white"/></g></mask></defs><g transform="scale(64)"><rect width="1" height="1" mask="url(#m)"/></g>"#
        ));
        // Aynı içerik ölçeksiz kabul edilir.
        let ok = format!(r#"<defs>{BLUR20}<mask id="m"><rect width="64" height="64" fill="white" filter="url(#b)"/></mask></defs><rect width="64" height="64" mask="url(#m)"/>"#);
        assert!(render_svg(&svg(&ok), t(64, 64)).is_ok());
    }

    #[test]
    fn whole_mask_and_clip_chains_are_audited() {
        // usvg subroots yalnızca iki halka verir; üçüncü maskedeki bomba da görülmeli.
        let bomb = r#"<filter id="x"><feGaussianBlur stdDeviation="5000"/></filter>"#;
        quick_err(&format!(
            r#"<defs>{bomb}<mask id="c"><rect width="64" height="64" fill="white" filter="url(#x)"/></mask><mask id="b" mask="url(#c)"><rect width="64" height="64" fill="white"/></mask><mask id="a" mask="url(#b)"><rect width="64" height="64" fill="white"/></mask></defs><rect width="64" height="64" mask="url(#a)"/>"#
        ));
        let segs: String = (0..3000).map(|i| format!("L{} {} ", i % 64, (i * 7) % 64)).collect();
        let heavy = format!("M0 0 {segs}Z");
        let mut clips = format!(r##"<path id="h" d="{heavy}"/><clipPath id="k0">{}</clipPath>"##, r##"<use xlink:href="#h"/>"##.repeat(700));
        for i in 1..4 {
            clips.push_str(&format!(r#"<clipPath id="k{i}" clip-path="url(#k{})"><rect width="64" height="64"/></clipPath>"#, i - 1));
        }
        // 700 × 3000 segment yalnızca dördüncü halkada.
        quick_err(&format!(r#"<defs>{clips}</defs><rect width="64" height="64" clip-path="url(#k3)"/>"#));
    }

    #[test]
    fn rejects_costly_filter_primitives() {
        quick_err(r#"<filter id="f"><feDropShadow stdDeviation="5000"/></filter><rect width="64" height="64" filter="url(#f)"/>"#);
        quick_err(r#"<filter id="f"><feTurbulence baseFrequency="0.05" numOctaves="1000000000"/></filter><rect width="64" height="64" filter="url(#f)"/>"#);
        quick_err_at(r#"<filter id="f"><feMorphology operator="dilate" radius="100000"/></filter><rect width="64" height="64" filter="url(#f)"/>"#, 1024);
        let kernel = vec!["1"; 200 * 200].join(" ");
        quick_err_at(
            &format!(r#"<filter id="f"><feConvolveMatrix order="200" kernelMatrix="{kernel}"/></filter><rect width="64" height="64" filter="url(#f)"/>"#),
            512,
        );
    }

    #[test]
    fn rejects_overdraw_and_repeated_image_decode() {
        // 2000 tam tuval dolgusu 4096² hedefte: 3.3·10¹⁰ piksel.
        quick_err_at(&"<rect width=\"64\" height=\"64\"/>".repeat(2000), 4096);
        // Aynı gömülü görsel her çizimde yeniden çözülür: 600 × 4 MP.
        let uri = png_data_uri(2000, 2000);
        let body = format!(r##"<defs><image id="i" width="64" height="64" xlink:href="{uri}"/></defs>{}"##, r##"<use xlink:href="#i"/>"##.repeat(600));
        // usvg her kopyada URI'yi yeniden çözer (debug derlemede, yük altında birkaç yüz ms):
        // asıl ölçüt reddedilmesi; süre sınırı gevşek.
        let start = std::time::Instant::now();
        assert!(matches!(render_svg(&svg(&body), t(64, 64)), Err(ImageError::Svg(_))));
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "{:?}", start.elapsed());
    }

    #[test]
    fn rejects_data_uri_multiplication_before_conversion() {
        // 1 MiB'lık data: URI × 300 kopya: dönüştürme 300 MiB base64 çözerdi; açılım
        // tahmininde bayt sayıldığı için ayrıştırmadan hemen sonra reddedilir.
        let uri = format!("data:image/png;base64,{}", "A".repeat(1 << 20));
        quick_err(&format!(r##"<defs><image id="i" width="8" height="8" xlink:href="{uri}"/></defs>{}"##, r##"<use xlink:href="#i"/>"##.repeat(300)));
        // Tek bir büyük gömülü görsel hâlâ geçer (açılım ~4k öğe).
        let one = format!(r#"<image width="8" height="8" xlink:href="{uri}"/>"#);
        assert!(render_svg(&svg(&one), t(8, 8)).is_ok());
    }

    #[test]
    fn ordinary_rich_svg_still_renders() {
        let body = format!(
            r##"<defs>{BLUR20}<linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient><pattern id="p" width="8" height="8" patternUnits="userSpaceOnUse"><circle cx="4" cy="4" r="3" fill="url(#g)"/></pattern><clipPath id="c"><circle cx="32" cy="32" r="30"/></clipPath><mask id="m"><rect width="64" height="64" fill="white" opacity="0.8"/></mask></defs><g clip-path="url(#c)" mask="url(#m)"><rect width="64" height="64" fill="url(#p)"/><rect x="16" y="16" width="32" height="32" fill="url(#g)" filter="url(#b)" stroke="#000" stroke-dasharray="2 1"/></g>"##
        );
        for side in [24, 256, 2048] {
            let img = render_svg(&svg(&body), t(side, side)).unwrap();
            assert_eq!(img.size(), (side, side));
        }
    }
}
