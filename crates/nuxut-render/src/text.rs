//! Metin: `cosmic-text` ile şekillendirme ve yerleşim. Glyph rasterleştirme `swash` ile
//! yapılır; GL atlas'ı `Renderer` tarafındadır (`atlas.rs`).

use cosmic_text::{
    Align, Attrs, Buffer, CacheKey, Ellipsize, EllipsizeHeightLimit, Family, FontSystem,
    CacheKeyFlags, LayoutGlyph, Metrics, Shaping, SwashImage, Weight, Wrap,
};
use nuxut_core::{Color, Point, Rect, Size};

/// Bir `TextLayout`'un kabul ettiği en uzun metin (bayt). Dış girdiden (pencere başlığı,
/// bildirim) gelen devasa metin şekillendiriciyi kilitlememeli.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// Font ailesi adı ya da genel aile: `sans-serif`, `serif`, `monospace`.
    pub family: String,
    /// Mantıksal piksel.
    pub size: f32,
    /// 100–1000.
    pub weight: u16,
    /// Satır yüksekliği çarpanı.
    pub line_height: f32,
    pub align: TextAlign,
    /// `Some(n)`: en fazla `n` satır, taşan kısım `…` ile kesilir. `max_width` gerekir.
    pub max_lines: Option<usize>,
    /// Değişken fontun `FILL` ekseni, 0..=1 (fontta yoksa etkisiz).
    pub fill: f32,
    /// OpenType `tnum`: tüm rakamlar eşit genişlikte (değişen sayılar titremez).
    pub tabular_numbers: bool,
    /// `family` kurulu değilse kullanılacak genel aile.
    pub fallback: GenericFamily,
    /// Glifler layout kutusunda nasıl ortalanır (kutu boyutu değişmez; bkz. [`OpticalCenter`]).
    pub optical: OpticalCenter,
}

/// Layout kutusu içinde gliflerin optik ortalaması (ADR-0009). Kutunun boyutu (genişlik =
/// ilerleme, yükseklik = satır yükseklikleri) her kipte aynıdır; yalnızca glifler kutu içinde
/// kaydırılır. Böylece kutuyu bir hücrede ortalayan çağıran (`rect.center() - size / 2`)
/// kendiliğinden optik ortalanmış metin/ikon çizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpticalCenter {
    /// Tek glife şekillenen ikon (Özel Kullanım Alanı kod noktası — Nerd Font — ya da çok
    /// karakterli ligatür — Material Symbols) → [`Ink`](Self::Ink); diğer metin →
    /// [`CapHeight`](Self::CapHeight).
    #[default]
    Auto,
    /// Fontun ascent/descent'i satır kutusunda ortalanır (fonta göre değişen eski davranış).
    Metrics,
    /// Büyük harf bandı (ilk satırın büyük harf üstünden son satırın taban çizgisine) kutunun
    /// dikey ortasında. İçerikten bağımsızdır: rakam değişince metin zıplamaz, yan yana
    /// etiketlerin taban çizgisi aynı kalır. Çok satırlı metinde tüm blok birlikte kayar
    /// (satır aralığı değişmez).
    CapHeight,
    /// Gliflerin mürekkep (çizim) kutusu layout kutusunun ortasında, iki eksende.
    Ink,
}

/// Kurulu bir font ailesi ([`TextSystem::families`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyInfo {
    pub name: String,
    /// Dik (normal) yüzlerin ağırlıkları, artan. Değişken ailede tek değer olabilir.
    pub weights: Vec<u16>,
    /// Değişken ailede `wght` ekseninin aralığı (min, max).
    pub weight_range: Option<(u16, u16)>,
    /// İtalik/eğik yüzü var.
    pub italic: bool,
    /// Tüm yüzleri eş aralıklı.
    pub monospace: bool,
    /// cmap'te `a` var. `false` = yalnız sembol fontu (ör. Symbols Nerd Font) ya da Latin dışı
    /// yazı sistemi fontu. Ligatür ikon fontları (Material Symbols) harf taşıdığından `true`dur.
    pub latin: bool,
}

/// Genel font ailesi (yedek); fontconfig tercihine çözülür.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GenericFamily {
    #[default]
    SansSerif,
    Serif,
    Monospace,
}

impl GenericFamily {
    fn name(self) -> &'static str {
        match self {
            Self::SansSerif => "sans-serif",
            Self::Serif => "serif",
            Self::Monospace => "monospace",
        }
    }
}

impl TextStyle {
    pub fn new(family: impl Into<String>, size: f32, weight: u16) -> Self {
        Self {
            family: family.into(),
            size,
            weight,
            line_height: 1.3,
            align: TextAlign::Start,
            max_lines: None,
            fill: 0.0,
            tabular_numbers: false,
            fallback: GenericFamily::SansSerif,
            optical: OpticalCenter::Auto,
        }
    }

    pub fn with_optical(mut self, optical: OpticalCenter) -> Self {
        self.optical = optical;
        self
    }

    /// Tabular rakamlar (`tnum`): saat, yüzde, sayaç.
    pub fn with_tabular_numbers(mut self) -> Self {
        self.tabular_numbers = true;
        self
    }

    pub fn with_fallback(mut self, fallback: GenericFamily) -> Self {
        self.fallback = fallback;
        self
    }

    pub fn with_max_lines(mut self, lines: usize) -> Self {
        self.max_lines = Some(lines.max(1));
        self
    }

    /// Değişken fontun `FILL` ekseni (0 = çizgi, 1 = dolu; ör. Material Symbols aktif ikon).
    pub fn with_fill(mut self, fill: f32) -> Self {
        self.fill = if fill.is_finite() { fill.clamp(0.0, 1.0) } else { 0.0 };
        self
    }

    pub fn with_align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }
}

/// Zengin metin parçasının biçimi ([`TextSystem::layout_rich`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpanStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    /// Parça rengi (düz alpha). `None` = çizimde verilen renk. Çizim rengi yarı saydamsa
    /// (ör. solan bildirim) bu rengin opaklığı da aynı oranda düşer.
    pub color: Option<Color>,
    /// Bağlantı kimliği: [`TextLayout::link_at`] bunu döndürür.
    pub link: Option<u32>,
}

/// Altı/üstü çizili parça çizgisi (mantıksal, layout'a göre).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DecorationLine {
    pub rect: Rect,
    pub color: Option<Color>,
}

fn to_cosmic(c: Color) -> cosmic_text::Color {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    cosmic_text::Color::rgba(q(c.r), q(c.g), q(c.b), q(c.a))
}

pub(crate) fn from_cosmic(c: cosmic_text::Color) -> Color {
    Color::from_rgba8(c.r(), c.g(), c.b(), c.a())
}

#[derive(Debug, Clone)]
pub(crate) struct PlacedGlyph {
    pub glyph: LayoutGlyph,
    /// Satırın taban çizgisi (mantıksal, layout'un üstünden).
    pub line_y: f32,
    /// Görsel satır indeksi (`TextLayout::lines`).
    pub line: usize,
    /// Kümenin metindeki bayt aralığı (mutlak).
    pub start: usize,
    pub end: usize,
}

/// Bir görsel satır (sarma sonrası).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VisualLine {
    pub top: f32,
    pub height: f32,
    /// Satırın kapsadığı bayt aralığı (mutlak).
    pub start: usize,
    pub end: usize,
    pub width: f32,
}

/// Şekillendirilmiş, konumlandırılmış metin. Ölçeğe bağımsızdır (mantıksal piksel); aynı
/// layout farklı ölçekli yüzeylerde çizilebilir. Çağıran önbellekler — her karede yeniden
/// şekillendirme yapılmaz.
#[derive(Debug, Clone, Default)]
pub struct TextLayout {
    pub(crate) glyphs: Vec<PlacedGlyph>,
    pub(crate) lines: Vec<VisualLine>,
    pub(crate) decorations: Vec<DecorationLine>,
    text_len: usize,
    size: Size,
    baseline: f32,
}

impl TextLayout {
    /// Layout'u boşaltır; metni ele verebilecek alanlar (glif kimlikleri, bayt aralıkları,
    /// konum/genişlikler) bırakılmadan önce `zeroize` ile sıfırlanır. Gizli metin layout'u
    /// ([`TextSystem::layout_secret`]) düşmeden önce çağrılır. Sonra layout boştur.
    pub fn wipe(&mut self) {
        use zeroize::Zeroize;
        for g in &mut self.glyphs {
            let c = &mut g.glyph;
            c.glyph_id.zeroize();
            c.start.zeroize();
            c.end.zeroize();
            c.x.zeroize();
            c.y.zeroize();
            c.w.zeroize();
            c.x_offset.zeroize();
            c.y_offset.zeroize();
            c.metadata.zeroize();
            g.line_y.zeroize();
            g.line.zeroize();
            g.start.zeroize();
            g.end.zeroize();
        }
        for l in &mut self.lines {
            l.start.zeroize();
            l.end.zeroize();
            l.width.zeroize();
        }
        self.glyphs.clear();
        self.lines.clear();
        self.decorations.clear();
        self.text_len = 0;
        self.size = Size::ZERO;
        self.baseline = 0.0;
    }

    /// Kaplanan alan (mantıksal piksel). Genişlik en uzun satır, yükseklik satırlar toplamı.
    pub fn size(&self) -> Size {
        self.size
    }

    /// İlk satırın taban çizgisinin üstten uzaklığı.
    pub fn baseline(&self) -> f32 {
        self.baseline
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// Görsel satır sayısı (sarma sonrası; boş metinde 1).
    pub fn line_count(&self) -> usize {
        self.lines.len().max(1)
    }

    /// Bayt konumundaki imlecin dikdörtgeni (genişlik 0; mantıksal, layout'un sol-üstüne
    /// göre). `byte` metin uzunluğuna sınırlanır; küme ortasındaki bayt (ör. ligatür)
    /// küme genişliği içinde orantılanır. Sağdan sola metinde kümenin sağ kenarı başlangıçtır.
    pub fn caret_rect(&self, byte: usize) -> Rect {
        let byte = byte.min(self.text_len);
        let Some(li) = self.line_of(byte) else {
            return Rect::new(0.0, 0.0, 0.0, self.size.height);
        };
        let line = self.lines[li];
        let x = self.caret_x_in_line(li, byte);
        Rect::new(x, line.top, 0.0, line.height)
    }

    /// Tek satırlık giriş için kısayol: [`caret_rect`](Self::caret_rect)`.x`.
    pub fn cursor_x(&self, byte: usize) -> f32 {
        self.caret_rect(byte).x
    }

    /// Noktaya en yakın imleç konumu (bayt). Satır dikeyde, küme yatayda en yakın
    /// kenarına göre seçilir; satır dışı tıklama satırın başı/sonudur.
    pub fn hit(&self, p: Point) -> usize {
        if self.lines.is_empty() {
            return 0;
        }
        let li = self
            .lines
            .iter()
            .position(|l| p.y < l.top + l.height)
            .unwrap_or(self.lines.len() - 1);
        let line = self.lines[li];
        let mut best = (f32::MAX, line.start);
        let mut consider = |x: f32, byte: usize| {
            let d = (x - p.x).abs();
            if d < best.0 {
                best = (d, byte);
            }
        };
        for g in self.glyphs.iter().filter(|g| g.line == li) {
            let (left, right) = (g.glyph.x, g.glyph.x + g.glyph.w);
            let rtl = g.glyph.level.is_rtl();
            consider(if rtl { right } else { left }, g.start);
            consider(if rtl { left } else { right }, g.end);
        }
        if self.glyphs.iter().all(|g| g.line != li) {
            consider(0.0, line.start);
        }
        best.1
    }

    /// Noktadaki bağlantının kimliği ([`SpanStyle::link`]); nokta layout'un sol-üstüne göre.
    pub fn link_at(&self, p: Point) -> Option<u32> {
        self.glyphs
            .iter()
            .filter(|g| g.glyph.metadata != 0)
            .find(|g| {
                let line = self.lines[g.line];
                p.y >= line.top && p.y < line.top + line.height && p.x >= g.glyph.x && p.x < g.glyph.x + g.glyph.w
            })
            .map(|g| (g.glyph.metadata - 1) as u32)
    }

    /// Tek satırlık giriş için kısayol: ilk satırın ortasında [`hit`](Self::hit).
    pub fn hit_byte(&self, x: f32) -> usize {
        let y = self.lines.first().map_or(0.0, |l| l.top + l.height * 0.5);
        self.hit(Point::new(x, y))
    }

    /// Baytı içeren görsel satır. Satır sınırındaki bayt (sarma noktası) sonraki satırın
    /// başıdır; metnin sonu son satırdadır.
    fn line_of(&self, byte: usize) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        self.lines
            .iter()
            .position(|l| byte >= l.start && byte < l.end)
            .or_else(|| self.lines.iter().rposition(|l| byte >= l.start && byte <= l.end))
            .or(Some(self.lines.len() - 1))
    }

    fn caret_x_in_line(&self, li: usize, byte: usize) -> f32 {
        let mut fallback: Option<f32> = None;
        for g in self.glyphs.iter().filter(|g| g.line == li) {
            let rtl = g.glyph.level.is_rtl();
            let (left, right) = (g.glyph.x, g.glyph.x + g.glyph.w);
            if byte >= g.start && byte < g.end {
                let span = (g.end - g.start).max(1) as f32;
                let t = (byte - g.start) as f32 / span;
                return if rtl { right - (right - left) * t } else { left + (right - left) * t };
            }
            if byte == g.end {
                fallback = Some(if rtl { left } else { right });
            }
        }
        fallback.unwrap_or(0.0)
    }
}

/// Font sistemi + glyph rasterleştirici. Uygulama başına bir tane; tüm yüzeyler paylaşır.
pub struct TextSystem {
    fonts: FontSystem,
    scale: swash::scale::ScaleContext,
    /// (aile, istenen ağırlık) → ailede bulunan en yakın ağırlık.
    weights: std::collections::HashMap<(String, u16), u16>,
    /// Aile adı → kurulu mu (büyük/küçük harf duyarsız).
    families: std::collections::HashMap<String, bool>,
    /// Kurulu aile listesi (ilk istekte veritabanından, ikinci tarama yok).
    family_list: Option<Vec<FamilyInfo>>,
}

impl std::fmt::Debug for TextSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextSystem").field("faces", &self.fonts.db().len()).finish()
    }
}

impl Default for TextSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSystem {
    /// Sistem fontlarını yükler (fontconfig dizinleri). Font yoksa bile çalışır; metin
    /// boş çizilir ve uyarı loglanır.
    ///
    /// Tarama sonucu `font_cache` ile önbelleğe alınır; sonraki açılışlar font dosyalarını
    /// ayrıştırmadan aynı veritabanını kurar (bkz. `font_cache` modül belgesi).
    pub fn new() -> Self {
        use crate::font_cache::Loaded;
        let mut fonts = match crate::font_cache::load() {
            Loaded::Fresh(db) => FontSystem::new_with_locale_and_db(system_locale(), db),
            loaded => {
                let reuse = match loaded {
                    Loaded::Stale(reuse) => Some(reuse),
                    _ => None,
                };
                // Kayıtlar taramadan önce: tarama sırasında eklenen font sonraki açılışta yakalanır.
                let (records, db) = crate::font_cache::scan(reuse);
                // `resolve_generic_families`'ten ÖNCE: kullanıcı tercihleri önbelleğe girmez.
                crate::font_cache::store(&db, records);
                FontSystem::new_with_locale_and_db(system_locale(), db)
            }
        };
        if fonts.db().is_empty() {
            tracing::warn!("hiç font bulunamadı; metin çizilmeyecek");
        }
        #[cfg(target_os = "linux")]
        crate::font_scan::prefetch(fonts.db());
        resolve_generic_families(fonts.db_mut());
        Self { fonts, scale: swash::scale::ScaleContext::new(), weights: std::collections::HashMap::new(),
            families: std::collections::HashMap::new(),
            family_list: None,
        }
    }

    /// Metni şekillendirir. `max_width` verilirse kelime sınırından sarılır.
    pub fn layout(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> TextLayout {
        self.layout_rich(&[(text, SpanStyle::default())], style, max_width)
    }

    /// Zengin metin: her parça kendi [`SpanStyle`]'ıyla (kalın, italik, altı çizili, renk,
    /// bağlantı). Parçalar art arda birleştirilir; bayt konumları birleşik metne göredir.
    /// Toplam uzunluk [`MAX_TEXT_BYTES`] ile sınırlıdır.
    pub fn layout_rich(&mut self, spans: &[(&str, SpanStyle)], style: &TextStyle, max_width: Option<f32>) -> TextLayout {
        let mut text = String::new();
        let mut ranges: Vec<(std::ops::Range<usize>, &SpanStyle)> = Vec::with_capacity(spans.len());
        for (s, span) in spans {
            let room = MAX_TEXT_BYTES - text.len();
            let s = truncate_to_char_boundary(s, room);
            let start = text.len();
            text.push_str(s);
            ranges.push((start..text.len(), span));
            if s.len() == room {
                break;
            }
        }
        self.shape(&text, &ranges, style, max_width, false)
    }

    /// Gizli metnin (gösterilen parola) layout'u: tek parça, sarmasız. Sonuç sıradan bir
    /// [`TextLayout`]'tur; farkı yol boyunca bizim elimizdeki metin kopyalarının sıfırlanmasıdır:
    /// birleşik ara dizgi hiç oluşmaz, cosmic-text satır dizgisi (`BufferLine`) dönmeden önce
    /// sıfırlanır, şekillendirme karalama tamponu aynı uzunlukta bir dolgu metniyle üzerine
    /// yazılır (en iyi çaba). Çağıran, işi bitince layout'u [`TextLayout::wipe`] ile siler.
    /// Glif atlası (karakter kümesi; sıra yok) ve cosmic-text'in satır içi şekil önbelleği
    /// (glif kimlikleri) kapsam dışıdır — `docs/ARCHITECTURE.md` "Gizli metin girişi".
    pub fn layout_secret(&mut self, text: &str, style: &TextStyle) -> TextLayout {
        let text = truncate_to_char_boundary(text, MAX_TEXT_BYTES);
        let plain = SpanStyle::default();
        let out = self.shape(text, &[(0..text.len(), &plain)], style, None, true);
        // Karalama tamponunu (harfrust girdi/glif dizisi, kelime/glif vektörleri) aynı karakter
        // sayısında zararsız bir metinle yeniden doldur: önceki parolanın izleri ezilir.
        let filler = zeroize::Zeroizing::new("x".repeat(text.chars().count()));
        let mut scratch = self.shape(&filler, &[(0..filler.len(), &plain)], style, None, true);
        scratch.wipe();
        out
    }

    fn shape(
        &mut self,
        text: &str,
        ranges: &[(std::ops::Range<usize>, &SpanStyle)],
        style: &TextStyle,
        max_width: Option<f32>,
        secret: bool,
    ) -> TextLayout {
        let size = sanitize(style.size, 13.0).clamp(1.0, 512.0);
        let line_height = size * sanitize(style.line_height, 1.3).clamp(0.5, 4.0);
        let max_width = max_width.filter(|w| w.is_finite()).map(|w| w.max(0.0));

        let mut buffer = Buffer::new_empty(Metrics::new(size, line_height));
        buffer.set_size(max_width, None);
        buffer.set_wrap(if max_width.is_some() { Wrap::WordOrGlyph } else { Wrap::None });
        if let Some(lines) = style.max_lines {
            buffer.set_ellipsize(Ellipsize::End(EllipsizeHeightLimit::Lines(lines)));
        }
        let family_name = self.resolve_family(&style.family, style.fallback);
        let base_weight = self.available_weight(&family_name, style.weight.clamp(1, 1000));
        let mut attrs = Attrs::new().family(family(&family_name)).weight(Weight(base_weight));
        if style.tabular_numbers {
            let mut features = cosmic_text::FontFeatures::new();
            features.enable(cosmic_text::FeatureTag::new(b"tnum"));
            attrs = attrs.font_features(features);
        }
        // Optik boyut (opsz) mantıksal boyuttan: aynı metin her ölçekte aynı tasarımla çizilir
        // (önceden fiziksel piksel kullanılıyordu: 2.5 ölçekte 13 px metin 32 px kesimiyle).
        attrs = attrs.cache_key_flags(fill_to_flags(style.fill) | opsz_to_flags(size));
        let align = match style.align {
            TextAlign::Start => None,
            TextAlign::Center => Some(Align::Center),
            TextAlign::End => Some(Align::End),
        };
        // CSS "bolder": 550 altı → 700, üstü → 900; ailenin gerçekten sahip olduğu ağırlığa
        // yuvarlanır, yoksa eşleştirici o ağırlığı başka bir AİLEDE bulur (ör. kalın parça
        // eş aralıklı fonta düşer).
        let bold_weight = self.available_weight(&family_name, if style.weight < 550 { 700 } else { 900 });
        let rich = ranges.iter().map(|(r, span)| {
            let mut a = attrs.clone();
            if span.bold {
                a = a.weight(Weight(bold_weight));
            }
            if span.italic {
                a = a.style(cosmic_text::Style::Italic);
            }
            if let Some(c) = span.color {
                a = a.color(to_cosmic(c));
            }
            if span.underline {
                a = a.underline(cosmic_text::UnderlineStyle::Single);
            }
            if span.strikethrough {
                a = a.strikethrough();
            }
            if let Some(link) = span.link {
                // 0 = bağlantı yok.
                a = a.metadata(link as usize + 1);
            }
            (&text[r.clone()], a)
        });
        if secret {
            // Tek parça: set_rich_text birleşik bir ara dizgi kopyası üretip sıfırlamadan bırakır;
            // set_text metni doğrudan satır dizgisine kopyalar (aşağıda sıfırlanır).
            buffer.set_text(text, &attrs, Shaping::Advanced, align);
        } else {
            buffer.set_rich_text(rich, &attrs, Shaping::Advanced, align);
        }
        buffer.shape_until_scroll(&mut self.fonts, false);

        let mut out = TextLayout::default();
        // Paragrafların metindeki bayt aralıkları; cosmic-text glyph aralıklarını paragrafa göre
        // verir ve metni yalnızca \n ile değil tüm Unicode paragraf ayırıcılarıyla (\r, U+0085,
        // U+2029, U+001C–U+001E: bidi sınıfı B) böler. Aynı bölücü kullanılır; yalnızca \n'e
        // göre hesaplamak bu karakterlerde glif aralıklarını metnin dışına taşıyordu.
        let text_ptr = text.as_ptr() as usize;
        let paras: Vec<std::ops::Range<usize>> = cosmic_text::BidiParagraphs::new(text)
            .map(|p| {
                let start = p.as_ptr() as usize - text_ptr;
                start..start + p.len()
            })
            .collect();
        let para_end = |p: usize| {
            let Some(r) = paras.get(p) else { return text.len() };
            // "\r\n" satır sonunun \r'si paragrafa dahil değildir.
            if r.end > r.start && text.as_bytes()[r.end - 1] == b'\r' { r.end - 1 } else { r.end }
        };
        let mut first = true;
        let mut width: f32 = 0.0;
        let mut height: f32 = 0.0;
        let mut prev_para: Option<usize> = None;
        for run in buffer.layout_runs() {
            if first {
                out.baseline = run.line_y;
                first = false;
            }
            let base = paras.get(run.line_i).map_or(text.len(), |r| r.start);
            // Sarılmış (paragrafın ilk olmayan) satırın başındaki boşluklar atılır. cosmic-text
            // zengin metinde parça sınırındaki boşluğu satır sonuna değil yeni satırın başına
            // taşıyabiliyor; satır içeri girintili görünmesin.
            let wrapped = prev_para == Some(run.line_i);
            prev_para = Some(run.line_i);
            let skip = if wrapped && !run.rtl && align.is_none() {
                run.glyphs
                    .iter()
                    .take_while(|g| text.get(base + g.start..base + g.end).is_some_and(|s| s.chars().all(char::is_whitespace)))
                    .count()
            } else {
                0
            };
            // Hizalı satırda ilk glifin x'i cosmic-text'in hizalama ofsetidir: korunur.
            let shift = if align.is_some() { 0.0 } else { run.glyphs.get(skip).map_or(0.0, |g| g.x) };
            let kept = &run.glyphs[skip..];
            let line_w = (run.line_w - shift).max(0.0);
            width = width.max(line_w);
            height = height.max(run.line_top + run.line_height);
            let (start, end) = match (kept.iter().map(|g| g.start).min(), kept.iter().map(|g| g.end).max()) {
                (Some(s), Some(e)) => (base + s, base + e),
                // Boş satır: paragrafın kendisi (boş) ya da sarmanın kalan boşluğu.
                _ => (base, para_end(run.line_i)),
            };
            out.lines.push(VisualLine {
                top: run.line_top,
                height: run.line_height,
                start,
                end,
                width: line_w,
            });
            let line = out.lines.len() - 1;
            out.glyphs.extend(kept.iter().map(|g| {
                let mut glyph = g.clone();
                glyph.x -= shift;
                PlacedGlyph { glyph, line_y: run.line_y, line, start: base + g.start, end: base + g.end }
            }));
            for d in run.decorations {
                let range = d.glyph_range.start.max(skip)..d.glyph_range.end.max(skip);
                let glyphs: Vec<cosmic_text::LayoutGlyph> = run.glyphs[range]
                    .iter()
                    .map(|g| {
                        let mut g = g.clone();
                        g.x -= shift;
                        g
                    })
                    .collect();
                let (Some(x0), Some(x1)) = (
                    glyphs.iter().map(|g| g.x).reduce(f32::min),
                    glyphs.iter().map(|g| g.x + g.w).reduce(f32::max),
                ) else {
                    continue;
                };
                let deco = &d.data.text_decoration;
                let mut line_at = |metrics: cosmic_text::DecorationMetrics, color: Option<cosmic_text::Color>| {
                    // Metrikler EM cinsinden; ofset taban çizgisinden yukarı pozitif.
                    let thickness = (metrics.thickness * d.font_size).max(1.0);
                    let y = run.line_y - metrics.offset * d.font_size - thickness * 0.5;
                    out.decorations.push(DecorationLine {
                        rect: Rect::new(x0, y, x1 - x0, thickness),
                        color: color.or(d.color_opt).map(from_cosmic),
                    });
                };
                if deco.underline != cosmic_text::UnderlineStyle::None {
                    line_at(d.data.underline_metrics, deco.underline_color_opt);
                }
                if deco.strikethrough {
                    line_at(d.data.strikethrough_metrics, deco.strikethrough_color_opt);
                }
            }
        }
        if secret {
            // cosmic-text satır dizgileri (metnin tek kopyası) bırakılmadan sıfırlanır.
            for line in std::mem::take(&mut buffer.lines) {
                zeroize::Zeroize::zeroize(&mut line.into_text());
            }
        }
        out.text_len = text.len();
        // Hizalama verildiyse sarma genişliği kutunun genişliğidir.
        if align.is_some()
            && let Some(w) = max_width
        {
            width = w;
        }
        out.size = Size::new(width, height);
        let (dx, dy) = self.optical_shift(&out, text, style.optical, &family_name);
        if dx != 0.0 || dy != 0.0 {
            for g in &mut out.glyphs {
                g.glyph.x += dx;
                g.line_y += dy;
            }
            for d in &mut out.decorations {
                d.rect = d.rect.translate(Point::new(dx, dy));
            }
            out.baseline += dy;
        }
        out
    }

    /// Optik ortalama için gliflerin kaydırılacağı (dx, dy) (mantıksal; ADR-0009).
    fn optical_shift(&mut self, layout: &TextLayout, text: &str, mode: OpticalCenter, family_name: &str) -> (f32, f32) {
        let (Some(first), Some(last)) = (layout.glyphs.first(), layout.glyphs.last()) else {
            return (0.0, 0.0);
        };
        let mode = match mode {
            OpticalCenter::Auto if layout.glyphs.len() == 1 && text.get(first.start..first.end).is_some_and(is_icon_text) => OpticalCenter::Ink,
            OpticalCenter::Auto => OpticalCenter::CapHeight,
            m => m,
        };
        let box_center = Point::new(layout.size.width * 0.5, layout.size.height * 0.5);
        if mode == OpticalCenter::Ink
            && let Some(ink) = self.ink_rect(layout)
        {
            let c = ink.center();
            return (box_center.x - c.x, box_center.y - c.y);
        }
        if mode == OpticalCenter::Metrics {
            return (0.0, 0.0);
        }
        // Büyük harf bandı: birincil fontun (ailenin ilk glifi; yoksa ilk glif) büyük harf
        // yüksekliği. Bilinmiyorsa (0 — eski/ikon fontu) kaydırma yok.
        let db = self.fonts.db();
        let primary = layout
            .glyphs
            .iter()
            .find(|g| db.face(g.glyph.font_id).is_some_and(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family_name))))
            .unwrap_or(first);
        let cap = self.cap_height(primary.glyph.font_id, primary.glyph.font_weight, primary.glyph.font_size);
        if cap <= 0.0 {
            return (0.0, 0.0);
        }
        let band_center = (first.line_y - cap + last.line_y) * 0.5;
        (0.0, box_center.y - band_center)
    }

    /// Fontun büyük harf yüksekliği (mantıksal px, `size` boyutunda). OS/2 `sCapHeight`; yoksa
    /// `H` glifinin çizim üstü; o da yoksa 0.
    fn cap_height(&mut self, font_id: cosmic_text::fontdb::ID, weight: Weight, size: f32) -> f32 {
        let Some(font) = self.fonts.get_font(font_id, weight) else { return 0.0 };
        let swash_font = font.as_swash();
        let coords = variation_coords(&swash_font, weight, 0.0, size.round().clamp(1.0, 255.0));
        let cap = swash_font.metrics(&coords).scale(size).cap_height;
        if cap > 0.0 {
            return cap;
        }
        let h = swash_font.charmap().map('H');
        if h == 0 {
            return 0.0;
        }
        let mut scaler = self.scale.builder(swash_font).size(size).hint(false).normalized_coords(&coords).build();
        scaler.scale_outline(h).map_or(0.0, |o| o.bounds().max.y.max(0.0))
    }

    /// Gliflerin çizim (mürekkep) kutularının birleşimi (mantıksal, layout'un sol-üstüne göre;
    /// ipucusuz anahat). Anahattı olmayan glif (renkli bitmap emoji) katılmaz; hiçbiri yoksa `None`.
    fn ink_rect(&mut self, layout: &TextLayout) -> Option<Rect> {
        let mut ink: Option<Rect> = None;
        for placed in &layout.glyphs {
            let g = &placed.glyph;
            let Some(font) = self.fonts.get_font(g.font_id, g.font_weight) else { continue };
            let swash_font = font.as_swash();
            let opsz = opsz_from_flags(g.cache_key_flags).unwrap_or(g.font_size);
            let coords = variation_coords(&swash_font, g.font_weight, fill_from_flags(g.cache_key_flags), opsz);
            let mut scaler = self.scale.builder(swash_font).size(g.font_size).hint(false).normalized_coords(&coords).build();
            let Some(outline) = scaler.scale_outline(g.glyph_id).or_else(|| scaler.scale_color_outline(g.glyph_id)) else {
                continue;
            };
            let b = outline.bounds();
            if !(b.max.x > b.min.x && b.max.y > b.min.y) {
                continue;
            }
            // Anahat y-yukarı, taban çizgisine göre; layout y-aşağı.
            let x = g.x + g.font_size * g.x_offset;
            let base = placed.line_y - g.font_size * g.y_offset;
            let r = Rect::new(x + b.min.x, base - b.max.y, b.max.x - b.min.x, b.max.y - b.min.y);
            ink = Some(ink.map_or(r, |i| i.union(&r)));
        }
        ink
    }

    /// `name` ailesi kurulu mu (tam ad; genel aile adları her zaman `true`). Önbellekli.
    pub fn has_family(&mut self, name: &str) -> bool {
        if !matches!(family(name), Family::Name(_)) {
            return true;
        }
        if let Some(&known) = self.families.get(name) {
            return known;
        }
        let known = self
            .fonts
            .db()
            .faces()
            .any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)));
        if !known {
            // Aile adı config'ten/varsayılandan gelir (kullanıcı içeriği değil).
            tracing::debug!("font ailesi kurulu değil, genel aileye düşülüyor: {name}");
        }
        self.families.insert(name.to_owned(), known);
        known
    }

    /// Kurulu font aileleri, ada göre sıralı (büyük/küçük harf duyarsız), önbellekli. İlk
    /// çağrıda her ailenin bir yüzünün yalnız cmap/fvar tablosu okunur (eşlenmiş dosya).
    pub fn families(&mut self) -> &[FamilyInfo] {
        let db = self.fonts.db();
        self.family_list.get_or_insert_with(|| {
            // (bilgi, yoklanacak ilk dik yüz)
            let mut map: std::collections::BTreeMap<String, (FamilyInfo, Option<cosmic_text::fontdb::ID>)> =
                std::collections::BTreeMap::new();
            for face in db.faces() {
                let Some((name, _)) = face.families.first() else { continue };
                let (info, probe) = map.entry(name.to_lowercase()).or_insert_with(|| {
                    let info = FamilyInfo {
                        name: name.clone(),
                        weights: Vec::new(),
                        weight_range: None,
                        italic: false,
                        monospace: true,
                        latin: false,
                    };
                    (info, None)
                });
                info.monospace &= face.monospaced;
                if face.style == cosmic_text::Style::Normal {
                    info.weights.push(face.weight.0);
                    probe.get_or_insert(face.id);
                } else {
                    info.italic = true;
                }
            }
            map.into_values()
                .filter_map(|(mut f, probe)| {
                    f.weights.sort_unstable();
                    f.weights.dedup();
                    // Yalnız cmap ve fvar okunur (fontdb dosyayı eşler; sayfa önbelleğinden).
                    db.with_face_data(probe?, |data, index| {
                        let font = swash::FontRef::from_index(data, index as usize)?;
                        let a = font.charmap().map('a');
                        f.latin = a != 0;
                        f.weight_range = font
                            .variations()
                            .find(|v| v.tag() == swash::Tag::from_be_bytes(*b"wght"))
                            .map(|v| (v.min_value().round() as u16, v.max_value().round() as u16));
                        Some(())
                    })?;
                    Some(f)
                })
                .collect()
        })
    }

    /// Aile değişken ağırlıklı mı (tek yüzlü ve `wght` eksenli). Yalnızca o ailenin tek yüzü
    /// yüklenir; seçili aile için çağrılır, listeleme için değil.
    pub fn is_variable(&mut self, family_name: &str) -> bool {
        let faces: Vec<(cosmic_text::fontdb::ID, u16)> = self
            .fonts
            .db()
            .faces()
            .filter(|f| f.style == cosmic_text::Style::Normal)
            .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family_name)))
            .map(|f| (f.id, f.weight.0))
            .collect();
        let single = faces.windows(2).all(|w| w[0].1 == w[1].1);
        single
            && faces.first().is_some_and(|&(id, w)| {
                self.fonts.get_font(id, Weight(w)).is_some_and(|font| {
                    font.as_swash().variations().any(|v| v.tag() == swash::Tag::from_be_bytes(*b"wght"))
                })
            })
    }

    /// Stilin gerçekte kullanacağı aile adı: kuruluysa `name`, değilse yedek genel ailenin
    /// çözüldüğü aile (ör. "Noto Sans"). Ayarlar'da "kurulu değil, X kullanılıyor" için.
    pub fn effective_family(&mut self, name: &str, fallback: GenericFamily) -> String {
        let resolved = self.resolve_family(name, fallback);
        self.fonts.db().family_name(&family(&resolved)).to_owned()
    }

    /// `text` yalnızca `family` ailesinin fontlarıyla (yedeğe düşmeden) kaç glife şekilleniyor;
    /// aile kurulu değilse ya da bir glif başka fonttan geliyorsa `None`. İkon ligatürünün ya da
    /// kod noktasının fontta gerçekten bulunduğunu doğrulamak için (ligatür = tek glif).
    pub fn glyphs_in_family(&mut self, family_name: &str, text: &str) -> Option<usize> {
        if !self.has_family(family_name) {
            return None;
        }
        let layout = self.layout(text, &TextStyle::new(family_name, 24.0, 400), None);
        let db = self.fonts.db();
        let from_family = |id| {
            db.face(id).is_some_and(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(family_name)))
        };
        let missing = |g: &PlacedGlyph| g.glyph.glyph_id == 0 || !from_family(g.glyph.font_id);
        (!layout.glyphs.iter().any(missing)).then_some(layout.glyphs.len())
    }

    /// Kurulu değilse yedek genel ailenin adı.
    fn resolve_family(&mut self, name: &str, fallback: GenericFamily) -> String {
        if self.has_family(name) { name.to_owned() } else { fallback.name().to_owned() }
    }

    /// `family` ailesinde `want`'a en yakın mevcut ağırlık. Değişken (wght eksenli) fontta
    /// `want` olduğu gibi döner; aile hiç yoksa da (fallback yine çalışsın diye). Eşitlikte
    /// ağır olan seçilir. Sonuç önbelleklenir.
    fn available_weight(&mut self, family_name: &str, want: u16) -> u16 {
        let key = (family_name.to_owned(), want);
        if let Some(w) = self.weights.get(&key) {
            return *w;
        }
        let name = self.fonts.db().family_name(&family(family_name)).to_owned();
        let faces: Vec<(cosmic_text::fontdb::ID, u16)> = self
            .fonts
            .db()
            .faces()
            .filter(|f| f.style == cosmic_text::Style::Normal)
            .filter(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)))
            .map(|f| (f.id, f.weight.0))
            .collect();
        // Değişken mi? Birden çok farklı ağırlıkta yüzü olan aile statiktir; font dosyası
        // açılmaz (soğuk açılışta her ağırlığın dosyasını okumak ~1 sn sürüyordu). Yalnızca tek
        // ağırlıklı ailede o tek yüz yüklenip wght ekseni aranır.
        let distinct_weights = {
            let mut ws: Vec<u16> = faces.iter().map(|&(_, w)| w).collect();
            ws.sort_unstable();
            ws.dedup();
            ws.len()
        };
        let variable = distinct_weights == 1
            && faces.first().is_some_and(|&(id, w)| {
                self.fonts.get_font(id, Weight(w)).is_some_and(|font| {
                    font.as_swash().variations().any(|v| v.tag() == swash::Tag::from_be_bytes(*b"wght"))
                })
            });
        let resolved = if faces.is_empty() || variable {
            want
        } else {
            faces
                .iter()
                .map(|&(_, w)| w)
                .min_by_key(|&w| (w.abs_diff(want), std::cmp::Reverse(w)))
                .unwrap_or(want)
        };
        self.weights.insert(key, resolved);
        resolved
    }

    /// Glyph'i rasterleştirir (önbelleksiz; önbellek GL atlas'ındadır). Değişken fontlarda
    /// `wght` (ağırlık), `FILL` (TextStyle::fill, cache anahtarı bayraklarından) ve `opsz`
    /// (piksel boyutuna göre optik boyut) eksenleri uygulanır.
    pub(crate) fn rasterize(&mut self, key: CacheKey) -> Option<SwashImage> {
        use swash::scale::{Render, Source, StrikeWith};
        use swash::zeno::{Format, Vector};

        let font = self.fonts.get_font(key.font_id, key.font_weight)?;
        let swash_font = font.as_swash();
        let size = f32::from_bits(key.font_size_bits);
        // Anahtar dışarıdan da kurulabilir: bozuk/aşırı boyut rasterleştirilmez.
        if !(size > 0.0 && size <= MAX_GLYPH_PX) {
            return None;
        }
        let opsz = opsz_from_flags(key.flags).unwrap_or(size);
        let coords = variation_coords(&swash_font, key.font_weight, fill_from_flags(key.flags), opsz);
        let mut scaler = self
            .scale
            .builder(swash_font)
            .size(size)
            .hint(!key.flags.contains(CacheKeyFlags::DISABLE_HINTING))
            .normalized_coords(&coords)
            .build();
        // Alt piksel kaydırma: swash anahattı y-yukarıdır; ekranda aşağı (y-aşağı) kaydırmak için
        // y eksiyle verilir (cosmic-text dikeyi hep tam piksele kestiğinden y_bin hep sıfırdı).
        let offset = Vector::new(key.x_bin.as_float(), -key.y_bin.as_float());
        Render::new(&[Source::ColorOutline(0), Source::ColorBitmap(StrikeWith::BestFit), Source::Outline])
            .format(Format::Alpha)
            .offset(offset)
            .render(&mut scaler, key.glyph_id)
    }
}

/// Genel aileleri (`sans-serif`, `serif`, `monospace`) **kurulu** bir aileye bağlar.
/// cosmic-text bunları sabit adlara ("Open Sans" …) ayarlıyor; ad kurulu değilse her parça
/// fallback'e düşer ve ağırlığa göre farklı ailelere dağılır (kalın kelime başka fontta).
/// Sıra: kullanıcının fontconfig tercihleri (`~/.config/fontconfig/fonts.conf`), sonra
/// yaygın Linux varsayılanları.
/// cosmic-text'in `FontSystem::new` içinde kullandığı yerel ayar (onunki private).
fn system_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| String::from("en-US"))
}

fn resolve_generic_families(db: &mut cosmic_text::fontdb::Database) {
    let user = user_font_preferences();
    let installed = |db: &cosmic_text::fontdb::Database, name: &str| {
        db.faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
    };
    let pick = |db: &cosmic_text::fontdb::Database, generic: &str, defaults: &[&str]| -> Option<String> {
        let aliases: &[&str] = if generic == "sans-serif" { &["sans-serif", "sans serif", "sans"] } else { &[generic] };
        user.iter()
            .filter(|(a, _)| aliases.contains(&a.as_str()))
            .map(|(_, n)| n.as_str())
            .chain(defaults.iter().copied())
            .find(|n| installed(db, n))
            .map(str::to_owned)
    };
    if let Some(n) = pick(db, "sans-serif", DEFAULT_SANS) {
        db.set_sans_serif_family(n);
    }
    if let Some(n) = pick(db, "serif", DEFAULT_SERIF) {
        db.set_serif_family(n);
    }
    if let Some(n) = pick(db, "monospace", DEFAULT_MONO) {
        db.set_monospace_family(n);
    }
    tracing::debug!(
        sans = db.family_name(&Family::SansSerif),
        serif = db.family_name(&Family::Serif),
        mono = db.family_name(&Family::Monospace),
        "genel font aileleri"
    );
}

// Platform başına yaygın varsayılan aileler (ilk kurulu olan seçilir).
#[cfg(target_os = "windows")]
const DEFAULT_SANS: &[&str] = &["Segoe UI", "Arial"];
#[cfg(target_os = "windows")]
const DEFAULT_SERIF: &[&str] = &["Times New Roman", "Georgia"];
#[cfg(target_os = "windows")]
const DEFAULT_MONO: &[&str] = &["Cascadia Mono", "Consolas", "Courier New"];
#[cfg(target_os = "macos")]
const DEFAULT_SANS: &[&str] = &["SF Pro Text", "Helvetica Neue", "Helvetica"];
#[cfg(target_os = "macos")]
const DEFAULT_SERIF: &[&str] = &["New York", "Times New Roman", "Times"];
#[cfg(target_os = "macos")]
const DEFAULT_MONO: &[&str] = &["SF Mono", "Menlo", "Monaco"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DEFAULT_SANS: &[&str] = &["Noto Sans", "Inter", "Cantarell", "DejaVu Sans", "Liberation Sans", "FreeSans", "Open Sans"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DEFAULT_SERIF: &[&str] = &["Noto Serif", "DejaVu Serif", "Liberation Serif", "FreeSerif"];
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const DEFAULT_MONO: &[&str] = &["Noto Sans Mono", "JetBrains Mono", "DejaVu Sans Mono", "Liberation Mono", "FreeMono"];

/// Kullanıcının fontconfig takma ad tercihleri: (genel ad, aile) — Linux'ta
/// `~/.config/fontconfig/fonts.conf`; diğer platformlarda boş (ADR-0003).
#[cfg(target_os = "linux")]
fn user_font_preferences() -> Vec<(String, String)> {
    let mut user = Vec::new();
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")));
    let Some(path) = base.map(|b| b.join("fontconfig").join("fonts.conf")).filter(|p| p.exists()) else {
        return user;
    };
    let mut fc = fontconfig_parser::FontConfig::default();
    if fc.merge_config(&path).is_ok() {
        for a in fc.aliases {
            for name in a.prefer.into_iter().chain(a.accept).chain(a.default) {
                user.push((a.alias.to_lowercase(), name));
            }
        }
    }
    user
}

#[cfg(not(target_os = "linux"))]
fn user_font_preferences() -> Vec<(String, String)> {
    Vec::new()
}

/// `FILL` ekseni cache anahtarının üst bayrak bitlerinde taşınır (cosmic-text'in kendi
/// bayrakları alt bitleri kullanır). 8 bit nicemleme: 256 dolgu düzeyi yeter.
const FILL_SHIFT: u32 = 16;

fn fill_to_flags(fill: f32) -> CacheKeyFlags {
    let q = (fill.clamp(0.0, 1.0) * 255.0).round() as u32;
    CacheKeyFlags::from_bits_retain(q << FILL_SHIFT)
}

fn fill_from_flags(flags: CacheKeyFlags) -> f32 {
    ((flags.bits() >> FILL_SHIFT) & 0xff) as f32 / 255.0
}

/// Optik boyut (`opsz`, mantıksal px, 1..=255'e yuvarlanmış) bayrakların en üst 8 bitinde;
/// 0 = bilinmiyor (rasterleştirme piksel boyutunu kullanır).
const OPSZ_SHIFT: u32 = 24;

fn opsz_to_flags(size: f32) -> CacheKeyFlags {
    let q = size.round().clamp(1.0, 255.0) as u32;
    CacheKeyFlags::from_bits_retain(q << OPSZ_SHIFT)
}

fn opsz_from_flags(flags: CacheKeyFlags) -> Option<f32> {
    let q = (flags.bits() >> OPSZ_SHIFT) & 0xff;
    (q != 0).then_some(q as f32)
}

/// Değişken fontun normalize eksen koordinatları: `wght`, `FILL`, `opsz` (piksel boyutu).
fn variation_coords(font: &swash::FontRef, weight: Weight, fill: f32, size: f32) -> Vec<i16> {
    let settings: Vec<swash::Setting<f32>> = font
        .variations()
        .filter_map(|v| {
            let value = match &v.tag().to_be_bytes() {
                b"wght" => f32::from(weight.0),
                b"FILL" => fill,
                // Optik boyut nokta cinsindendir; piksel boyutu iyi bir yaklaşım.
                b"opsz" => size,
                _ => return None,
            };
            Some(swash::Setting { tag: v.tag(), value: value.max(v.min_value()).min(v.max_value()) })
        })
        .collect();
    font.variations().normalized_coords(settings.iter().map(|s| (s.tag, s.value))).collect()
}

/// Tek glife şekillenmiş metin ikon mu: Özel Kullanım Alanı kod noktası (Nerd Font vb.) ya da
/// birden çok karakterin tek glife birleştiği ligatür/dizi (Material Symbols adı, emoji ZWJ dizisi).
fn is_icon_text(cluster: &str) -> bool {
    let mut chars = cluster.chars();
    let Some(c) = chars.next() else { return false };
    let private_use = matches!(u32::from(c), 0xE000..=0xF8FF | 0xF_0000..=0xF_FFFD | 0x10_0000..=0x10_FFFD);
    private_use || chars.next().is_some()
}

/// Rasterleştirilen glifin en büyük fiziksel boyutu (px): mantıksal üst sınır (512) × 4 ölçek.
/// Bozuk ölçek (çok büyük ya da sonsuz) devasa bitmap ayırmasın.
pub(crate) const MAX_GLYPH_PX: f32 = 2048.0;
/// Rasterleştirilen glifin en küçük fiziksel boyutu (px); sıfıra yakın ölçekte de pozitif.
const MIN_GLYPH_PX: f32 = 0.25;

/// Glifin fiziksel konumu ve önbellek anahtarı: `origin` (mantıksal) + ölçek (ADR-0009).
///
/// **Ölçek 1.0**: anahat ipuçlanır (TrueType hinting) ve taban çizgisi en yakın tam piksele
/// yuvarlanır (dikeyde alt piksel yok); yatayda çeyrek piksel alt konum. Küçük metinde (11–14 px)
/// yatay gövdeler tam piksele oturur; ipucusuz + dikey alt piksel 1.0'da ölçülür biçimde
/// yumuşaktı (eş aralıklı fontta tam dolu piksel %15 az). Optik kaydırma yine uygulanır; dikey
/// sapma ≤0.75 px (yuvarlama + ipuçlamanın büyük harf yüksekliği yuvarlaması).
///
/// **Diğer ölçekler** (kesirli ve 2.0+): anahat ipucusuz rasterleştirilir ve iki eksende çeyrek
/// piksel alt konumla yerleştirilir. Orada ipuçlama glifi ±1 fiziksel px kaydırıyordu; optik
/// ortalama ancak o kalkınca ≤0.5 px tutar, yoğunluk netliği zaten sağlar. Önceden cosmic-text'in
/// `physical`'ı taban çizgisini aşağı kesiyordu (metin sistematik olarak ≤1 fiziksel px yukarı).
pub(crate) fn place_glyph(placed: &PlacedGlyph, origin: Point, scale: f32) -> (CacheKey, i32, i32) {
    let g = &placed.glyph;
    // Bozuk ölçek (NaN/sonsuz/≤0; Recorder ve doğrudan çağıranlar süzmeden verebilir) 1'e düşer.
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let unit_scale = (scale - 1.0).abs() < 1e-3;
    let x = (g.x + g.font_size * g.x_offset).mul_add(scale, origin.x * scale);
    let y = (g.y - g.font_size * g.y_offset).mul_add(scale, (origin.y + placed.line_y) * scale);
    // cosmic-text'in alt piksel bölmesi `i32` kesmesinde taşar (hata ayıklamada panik): konum
    // sonlu ve ekran ötesi makul aralıkta tutulur (NaN → 0).
    let safe = |v: f32| if v.is_finite() { v.clamp(-1e7, 1e7) } else { 0.0 };
    let (x, y) = (safe(x), safe(y));
    let (y, flags) = if unit_scale {
        (y.round(), g.cache_key_flags)
    } else {
        (y, g.cache_key_flags | CacheKeyFlags::DISABLE_HINTING)
    };
    // Fiziksel boyut sınırlı: çok büyük ölçek devasa glif bitmap'i ayırmasın.
    let size = (g.font_size * scale).clamp(MIN_GLYPH_PX, MAX_GLYPH_PX);
    CacheKey::new(g.font_id, g.glyph_id, size, (x, y), g.font_weight, flags)
}

fn family(name: &str) -> Family<'_> {
    match name {
        "sans-serif" | "sans" => Family::SansSerif,
        "serif" => Family::Serif,
        "monospace" | "mono" => Family::Monospace,
        "cursive" => Family::Cursive,
        "fantasy" => Family::Fantasy,
        other => Family::Name(other),
    }
}

fn sanitize(v: f32, default: f32) -> f32 {
    if v.is_finite() { v } else { default }
}

fn truncate_to_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paralel tam tarama (`font_cache::scan`) `FontSystem::new()` ile aynı yüzleri AYNI SIRADA
    /// ve aynı genel aileleri vermeli (sıra eşleşmede eşitlik bozucudur).
    #[test]
    fn parallel_scan_matches_fontdb() {
        use cosmic_text::fontdb::{Database, Family};
        let (_, sequential) = FontSystem::new().into_locale_and_db();
        let (_, parallel) = crate::font_cache::scan(None);
        let faces = |db: &Database| -> Vec<String> {
            db.faces()
                .map(|f| {
                    format!(
                        "{:?}#{} {:?} {} {:?} {} {:?} {}",
                        f.source, f.index, f.families, f.post_script_name, f.style, f.weight.0, f.stretch,
                        f.monospaced
                    )
                })
                .collect()
        };
        assert!(!sequential.is_empty(), "test makinesinde font yok");
        assert_eq!(faces(&sequential), faces(&parallel));
        let generics = |db: &Database| {
            [Family::Serif, Family::SansSerif, Family::Cursive, Family::Fantasy, Family::Monospace]
                .map(|f| db.family_name(&f).to_owned())
        };
        assert_eq!(generics(&sequential), generics(&parallel));
    }

    /// Önbellekten kurulan veritabanı, taramayla kurulanla aynı yüzleri ve (resolve_generic_families
    /// öncesi ve sonrası) aynı genel aileleri vermeli.
    #[test]
    fn font_cache_rebuilds_identical_database() {
        use cosmic_text::fontdb::{Database, Family, Source};
        let (_, scanned) = FontSystem::new().into_locale_and_db();
        let restored = crate::font_cache::roundtrip_db(&scanned).expect("önbellek biçimi geri okunmalı");
        let key = |db: &Database| {
            let mut faces: Vec<String> = db
                .faces()
                .map(|f| {
                    let path = match &f.source {
                        Source::File(p) | Source::SharedFile(p, _) => p.display().to_string(),
                        Source::Binary(_) => String::from("<bellek>"),
                    };
                    let names: Vec<&str> = f.families.iter().map(|(n, _)| n.as_str()).collect();
                    format!(
                        "{path}#{} {names:?} {} {:?} {} {:?} {}",
                        f.index, f.post_script_name, f.style, f.weight.0, f.stretch, f.monospaced
                    )
                })
                .collect();
            faces.sort();
            faces
        };
        assert_eq!(key(&scanned), key(&restored));
        let generics = |db: &Database| {
            [Family::Serif, Family::SansSerif, Family::Cursive, Family::Fantasy, Family::Monospace]
                .map(|f| db.family_name(&f).to_owned())
        };
        assert_eq!(generics(&scanned), generics(&restored));
        let (mut a, mut b) = (scanned, restored);
        resolve_generic_families(&mut a);
        resolve_generic_families(&mut b);
        assert_eq!(generics(&a), generics(&b));
    }

    /// Center/End hizalaması çizimde korunur: ilk glif cosmic-text'in hizalama ofsetinde kalır
    /// (önceden satır başı kaydırması bu ofseti siliyordu, metin hep solda çiziliyordu).
    #[test]
    fn center_and_end_alignment_keep_offset() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let text = "Hoş geldin";
        let start = ts.layout(text, &TextStyle::new("sans-serif", 13.0, 500), Some(300.0));
        let w = start.size().width;
        assert!(w > 10.0 && w < 200.0, "{w}");
        assert!(start.cursor_x(0).abs() < 1.0);
        for (align, want) in [(TextAlign::Center, (300.0 - w) / 2.0), (TextAlign::End, 300.0 - w)] {
            let l = ts.layout(text, &TextStyle::new("sans-serif", 13.0, 500).with_align(align), Some(300.0));
            let x0 = l.cursor_x(0);
            assert!((x0 - want).abs() < 1.0, "{align:?}: ilk glif {x0}, beklenen {want}");
            assert!((l.cursor_x(text.len()) - x0 - w).abs() < 1.0, "{align:?}: satır genişliği korunur");
            assert_eq!(l.size().width, 300.0);
        }
    }

    #[test]
    fn truncation_respects_utf8() {
        assert_eq!(truncate_to_char_boundary("ğğ", 3), "ğ");
        assert_eq!(truncate_to_char_boundary("abc", 10), "abc");
    }

    #[test]
    fn caret_and_hit_roundtrip() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let style = TextStyle::new("sans-serif", 13.0, 500);
        let text = "ağaç ve şeker";
        let l = ts.layout(text, &style, None);
        assert_eq!(l.cursor_x(0), 0.0);
        let end = l.cursor_x(text.len());
        assert!((end - l.size().width).abs() < 0.5, "{end} vs {}", l.size().width);
        // Her karakter sınırı monoton artar ve hit ile geri bulunur.
        let mut prev = -1.0;
        for (i, _) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
            let x = l.cursor_x(i);
            assert!(x > prev, "bayt {i}: {x} <= {prev}");
            prev = x;
            assert_eq!(l.hit_byte(x + 0.1), i, "bayt {i}");
        }
        assert_eq!(l.hit_byte(-50.0), 0);
        assert_eq!(l.hit_byte(1e4), text.len());
        // Sınır dışı bayt metin sonuna sınırlanır.
        assert_eq!(l.cursor_x(999), end);
    }

    #[test]
    fn rich_spans_style_links_and_decorations() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let style = TextStyle::new("sans-serif", 13.0, 400);
        let plain = ts.layout("kalın", &style, None);
        let bold = ts.layout_rich(&[("kalın", SpanStyle { bold: true, ..Default::default() })], &style, None);
        assert!(bold.size().width > plain.size().width, "kalın daha geniş olmalı");

        let link = SpanStyle { underline: true, link: Some(7), ..Default::default() };
        let l = ts.layout_rich(&[("Bkz. ", SpanStyle::default()), ("belge", link), (" sonu", SpanStyle::default())], &style, None);
        assert_eq!(l.decorations.len(), 1, "yalnızca bağlantı altı çizili");
        let d = l.decorations[0].rect;
        let mid_y = l.caret_rect(0).y + l.caret_rect(0).height * 0.5;
        assert_eq!(l.link_at(Point::new(d.x + d.width * 0.5, mid_y)), Some(7));
        assert_eq!(l.link_at(Point::new(1.0, mid_y)), None);
        // Altı çizgi taban çizgisinin altında.
        assert!(d.y > l.baseline() - 1.0, "{} vs {}", d.y, l.baseline());
        // Bayt konumları birleşik metne göre.
        assert!(l.cursor_x("Bkz. ".len()) < d.x + 0.5 && l.cursor_x("Bkz. belge".len()) > d.right() - 0.5);
    }

    #[test]
    fn wrapped_line_has_no_leading_space_at_span_boundary() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let n = SpanStyle::default();
        let spans = [
            ("Akşam yemeğine geliyor musun? Saat ", n),
            ("sekizde", SpanStyle { bold: true, ..n }),
            (" Kadıköy'de buluşuyoruz; ", n),
            ("konum", SpanStyle { underline: true, link: Some(0), ..n }),
            (". Geç kalırsan haber ver, ", n),
            ("kitabı da getir", SpanStyle { italic: true, ..n }),
            (" lütfen.", n),
        ];
        let text: String = spans.iter().map(|(s, _)| *s).collect();
        // Bildirim kartıyla aynı: ağırlık 500, en çok 12 satır (üç nokta yolu).
        let style = TextStyle::new("sans-serif", 13.0, 500).with_max_lines(12);
        // Her sarma genişliğinde: sarılmış satır boşlukla ya da girintili başlamaz.
        for w in (60..420).step_by(2) {
            let l = ts.layout_rich(&spans, &style, Some(w as f32));
            for line in 1..l.lines.len() {
                let mut on_line: Vec<_> = l.glyphs.iter().filter(|g| g.line == line).collect();
                on_line.sort_by(|a, b| a.glyph.x.total_cmp(&b.glyph.x));
                let Some(first) = on_line.first() else { continue };
                assert!(first.glyph.x.abs() < 0.5, "genişlik {w}, satır {line}: girintili (x = {})", first.glyph.x);
                assert!(!text[first.start..first.end].trim().is_empty(), "genişlik {w}, satır {line}: boşlukla başlıyor");
            }
        }
    }

    /// Kurulu olmayan aile sessizce stilin yedek genel ailesine düşer (rastgele bir fonta değil).
    #[test]
    fn missing_family_uses_generic_fallback() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let missing = "Nuxut Olmayan Aile 7f3a";
        assert!(!ts.has_family(missing));
        assert!(ts.has_family("monospace"));
        for generic in [GenericFamily::Monospace, GenericFamily::Serif, GenericFamily::SansSerif] {
            let want = ts.layout("iiiWWW", &TextStyle::new(generic.name(), 13.0, 400), None);
            let got = ts.layout("iiiWWW", &TextStyle::new(missing, 13.0, 400).with_fallback(generic), None);
            assert_eq!(want.size(), got.size(), "{generic:?}");
            assert_eq!(want.glyphs[0].glyph.font_id, got.glyphs[0].glyph.font_id, "{generic:?}");
        }
    }

    #[test]
    fn family_list_and_variable_detection() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let list = ts.families().to_vec();
        assert!(!list.is_empty());
        assert!(list.windows(2).all(|w| w[0].name.to_lowercase() < w[1].name.to_lowercase()), "sıralı ve tekil");
        assert!(list.iter().all(|f| !f.weights.is_empty() && f.weights.windows(2).all(|w| w[0] < w[1])));
        for f in list.iter().take(5) {
            assert!(ts.has_family(&f.name));
        }
        if let Some(m) = list.iter().find(|f| f.name == "JetBrainsMono Nerd Font Mono") {
            assert!(m.monospace && m.italic && m.weights.len() > 1);
        }
        let get = |n: &str| list.iter().find(|f| f.name == n);
        if let Some(m) = get("Material Symbols Rounded") {
            assert!(m.weight_range.is_some_and(|(lo, hi)| lo <= 100 && hi >= 700), "{:?}", m.weight_range);
            assert!(ts.is_variable("Material Symbols Rounded"));
        }
        for text in ["Noto Sans", "JetBrainsMono Nerd Font", "DejaVu Sans"] {
            if let Some(f) = get(text) {
                assert!(f.latin, "{text}");
            }
        }
        if ts.has_family("JetBrainsMono Nerd Font") {
            assert!(!ts.is_variable("JetBrainsMono Nerd Font"), "çok ağırlıklı statik aile");
        }
        // Kurulu olmayan aile: gerçekte kullanılan aile, genel ailenin çözüldüğü aile.
        let serif = ts.fonts.db().family_name(&Family::Serif).to_owned();
        assert_eq!(ts.effective_family("Nuxut Olmayan Aile 7f3a", GenericFamily::Serif), serif);
        let first = list[0].name.clone();
        assert_eq!(ts.effective_family(&first, GenericFamily::Serif), first);
    }

    /// `tnum` açıkken rakamlar eşit genişlikte (değişen saat/yüzde titremez).
    #[test]
    fn tabular_numbers_have_equal_advance() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        // Inter ve türevi Adwaita Sans varsayılanda orantılı rakam kullanır: özellik gerçekten
        // uygulanıyor mu, orada görülür. Hiçbiri kurulu değilse sınanacak bir şey yok.
        let Some(name) = ["Inter", "Adwaita Sans"].into_iter().find(|n| ts.has_family(n)) else {
            return;
        };
        let width = |ts: &mut TextSystem, s: &str, tnum: bool| {
            let mut style = TextStyle::new(name, 20.0, 400);
            style.tabular_numbers = tnum;
            ts.layout(s, &style, None).size().width
        };
        let (p1, p8) = (width(&mut ts, "1111", false), width(&mut ts, "8888", false));
        let (t1, t8) = (width(&mut ts, "1111", true), width(&mut ts, "8888", true));
        assert!((p1 - p8).abs() > 0.5, "{name}: orantılı rakam bekleniyordu");
        assert!((t1 - t8).abs() < 0.5, "{name}: tnum 1111={t1} 8888={t8}");
    }

    #[test]
    fn bold_span_keeps_family() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let family_of = |ts: &TextSystem, l: &TextLayout, i: usize| -> String {
            let id = l.glyphs[i].glyph.font_id;
            ts.fonts.db().face(id).map(|f| f.families[0].0.clone()).unwrap_or_default()
        };
        for weight in [400u16, 500, 600] {
            let style = TextStyle::new("sans-serif", 13.0, weight);
            let l = ts.layout_rich(
                &[("Saat ", SpanStyle::default()), ("sekizde", SpanStyle { bold: true, ..Default::default() })],
                &style,
                None,
            );
            let plain = family_of(&ts, &l, 0);
            let bold = family_of(&ts, &l, l.glyphs.len() - 1);
            assert_eq!(plain, bold, "taban ağırlık {weight}: kalın parça başka aileye düştü");
        }
    }

    #[test]
    fn caret_multiline_and_empty() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let style = TextStyle::new("sans-serif", 13.0, 500);
        let text = "bir\n\nüç";
        let l = ts.layout(text, &style, None);
        assert_eq!(l.line_count(), 3);
        let r0 = l.caret_rect(0);
        let r_empty = l.caret_rect(4);
        let r2 = l.caret_rect(5);
        assert!(r0.y < r_empty.y && r_empty.y < r2.y);
        assert_eq!(r_empty.x, 0.0);
        assert_eq!(l.hit(Point::new(1e4, r_empty.y + 1.0)), 4);
        assert_eq!(l.hit(Point::new(1e4, r2.y + 1.0)), text.len());
        let empty = ts.layout("", &style, None);
        assert_eq!(empty.cursor_x(0), 0.0);
        assert_eq!(empty.hit_byte(10.0), 0);
    }

    #[test]
    fn layout_measures_text() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return; // Fontsuz CI ortamı.
        }
        let style = TextStyle::new("sans-serif", 13.0, 500);
        let short = ts.layout("Merhaba", &style, None);
        let long = ts.layout("Merhaba dünya, nasılsın?", &style, None);
        assert!(short.size().width > 0.0 && short.size().height > 0.0);
        assert!(long.size().width > short.size().width);
        let wrapped = ts.layout("Merhaba dünya, nasılsın?", &style, Some(60.0));
        assert!(wrapped.size().height > long.size().height);
        let clipped = ts.layout("Merhaba dünya, nasılsın?", &style.clone().with_max_lines(1), Some(60.0));
        assert!((clipped.size().height - short.size().height).abs() < 0.5);
    }

    /// Gizli layout sıradan layout'la aynı ölçümü verir; `wipe` her şeyi boşaltır.
    #[test]
    fn secret_layout_matches_plain_and_wipes() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return; // Fontsuz CI ortamı.
        }
        let style = TextStyle::new("sans-serif", 13.0, 500);
        for s in ["", "çok-gizli parola", "ab\u{301}c日本"] {
            let plain = ts.layout(s, &style, None);
            let mut secret = ts.layout_secret(s, &style);
            assert_eq!(plain.size(), secret.size(), "{s}");
            assert_eq!(plain.glyphs.len(), secret.glyphs.len());
            for b in 0..=s.len() {
                assert_eq!(plain.cursor_x(b), secret.cursor_x(b));
            }
            secret.wipe();
            assert!(secret.is_empty() && secret.lines.is_empty() && secret.size() == Size::ZERO);
        }
        // Dolgu şekillendirmesi sonraki layout'ları etkilemez.
        let a = ts.layout("Merhaba", &style, None);
        let _ = ts.layout_secret("parola123", &style);
        assert_eq!(ts.layout("Merhaba", &style, None).size(), a.size());
    }

    /// Güvenilmeyen metin (bildirim, pencere başlığı): optik ortalamanın hiçbir yolunda panik
    /// yok — çok baytlı/PUA/birleşik karakterler, satır sonları, üç nokta kırpması (kırpılan
    /// glifin aralığı karakter sınırına denk gelmeyebilir), RTL, boş ve aşırı değerler.
    #[test]
    fn optical_centering_survives_hostile_text() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let texts = [
            "",
            "\n",
            "\n\u{F0239}",
            "\u{F0239}\n",
            "\u{E000}\u{301}\u{301}",
            "a\u{200D}\u{F0239}",
            "home",
            "👩\u{200D}👩\u{200D}👧\u{200D}👦",
            "ağaç şeker ığdır İstanbul ağaç şeker ığdır İstanbul",
            "مرحبا بالعالم مرحبا بالعالم",
            "\u{FEFF}\u{FFFD}\u{10FFFD}\u{F0000}\r\n\t",
            "\u{F0239}\u{F0239}\u{F0239}\u{F0239}\u{F0239}\u{F0239}\u{F0239}\u{F0239}",
        ];
        let modes = [OpticalCenter::Auto, OpticalCenter::Metrics, OpticalCenter::CapHeight, OpticalCenter::Ink];
        for family in ["sans-serif", "monospace", "Symbols Nerd Font"] {
            for size in [f32::NAN, f32::INFINITY, -3.0, 0.0, 1.0, 13.0, 1e9] {
                for mode in modes {
                    for t in texts {
                        let style = TextStyle::new(family, size, 400).with_optical(mode);
                        for width in [None, Some(0.0), Some(8.0), Some(f32::NAN)] {
                            let l = ts.layout(t, &style, width);
                            let l1 = ts.layout(t, &style.clone().with_max_lines(1), width);
                            for l in [l, l1] {
                                assert!(l.size().width.is_finite() && l.size().height.is_finite(), "{t:?}");
                                for g in &l.glyphs {
                                    let _ = place_glyph(g, Point::new(f32::NAN, 1e30), 2.5);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Güvenilmeyen metnin düşmanca sınıfları (bildirim gövdesi, pencere başlığı, MPRIS başlığı):
/// şekillendirme, imleç/isabet sorguları ve Painter::text (bozuk ölçek/köken dahil) panik
/// vermez; tüm boyut ve konumlar sonludur; rasterleştirilen glifin boyutu sınırlıdır.
#[cfg(test)]
mod hostile_tests {
    use super::*;
    use crate::painter::{Painter, Recording};

    fn hostile_texts() -> Vec<(&'static str, String)> {
        let rep = |s: &str, n: usize| s.repeat(n);
        vec![
            // Yalnızca birleştirici işaretler (taban karakter yok).
            ("combining-acute", rep("\u{301}", 400)),
            ("combining-mixed", rep("\u{301}\u{308}\u{327}\u{20DD}", 150)),
            ("combining-on-base", format!("a{}", rep("\u{308}", 600))),
            ("combining-on-space", format!(" {} ", rep("\u{301}", 300))),
            ("combining-zalgo-words", rep("e\u{301}\u{302}\u{303}\u{304}\u{305}\u{306} ", 200)),
            // MAX_TEXT_BYTES sınırında ve üstünde.
            ("long-ascii-at-limit", rep("a", MAX_TEXT_BYTES)),
            ("long-ascii-over", rep("a", MAX_TEXT_BYTES + 1)),
            ("long-words-over", rep("abc ", MAX_TEXT_BYTES / 4 + 7)),
            ("long-2byte-straddle", format!("a{}", rep("ğ", MAX_TEXT_BYTES / 2 + 3))),
            ("long-4byte-straddle", format!("ab{}", rep("😀", MAX_TEXT_BYTES / 4 + 3))),
            ("long-combining-straddle", format!("x{}", rep("\u{301}", MAX_TEXT_BYTES / 2 + 1))),
            ("long-newlines", rep("\n", MAX_TEXT_BYTES + 5)),
            ("long-crlf", rep("a\r\n", MAX_TEXT_BYTES / 3 + 2)),
            // Kontrol ve yön karakterleri.
            ("nul", "\0".into()),
            ("nul-mid", "ab\0cd\0".into()),
            ("bell-escape", "\x07\x1b[31mkırmızı\x1b[0m\x1b]0;başlık\x07".into()),
            ("rlo", "\u{202E}abc def\u{202C} ghi".into()),
            ("bidi-isolates", "\u{2066}\u{2067}\u{2068}a\u{2069}\u{200F}\u{200E}\u{061C}".into()),
            ("separators", "a\u{2028}b\u{2029}c\u{0085}d\u{000B}e\u{000C}f".into()),
            ("c0-c1", (0u32..0x20).chain(0x7F..0xA0).filter_map(char::from_u32).collect()),
            ("backspace-del", "\x08\x08\x7f\u{9F}".into()),
            ("only-cr", rep("\r", 50)),
            ("tabs", rep("\t", 200)),
            ("zero-width", rep("\u{200B}\u{200C}\u{200D}\u{2060}\u{FEFF}", 100)),
            ("interlinear", "\u{FFF9}a\u{FFFA}b\u{FFFB}\u{FFFC}".into()),
            // Fontta olmayan glifler.
            ("unassigned", "\u{0378}\u{0379}\u{0380}\u{2FFFF}\u{3FFFD}".into()),
            ("max-scalar", "\u{10FFFF}".into()),
            ("noncharacters", "\u{FFFE}\u{FFFF}\u{FDD0}\u{FDEF}\u{1FFFE}".into()),
            ("tags", "\u{E0001}\u{E0020}\u{E007F}".into()),
            ("pua-planes", "\u{F0000}\u{FFFFD}\u{100000}\u{10FFFD}".into()),
            ("pua-bmp-unused", "\u{E000}\u{E001}\u{F8FE}".into()),
            ("missing-with-combining", rep("\u{10FFFF}\u{301}", 100)),
            ("variation-selectors", rep("\u{FE0F}\u{E0100}", 100)),
            ("lone-skin-tone", rep("\u{1F3FB}", 50)),
        ]
    }

    fn assert_finite_layout(name: &str, l: &TextLayout) {
        let s = l.size();
        assert!(s.width.is_finite() && s.height.is_finite() && s.width >= 0.0 && s.height >= 0.0, "{name}: boyut {s:?}");
        assert!(l.baseline().is_finite(), "{name}: taban çizgisi");
        assert!(l.text_len <= MAX_TEXT_BYTES, "{name}: uzunluk {}", l.text_len);
        for g in &l.glyphs {
            assert!(g.glyph.x.is_finite() && g.glyph.w.is_finite() && g.line_y.is_finite(), "{name}: glif konumu");
            assert!(g.start <= g.end && g.end <= l.text_len, "{name}: glif aralığı");
            assert!(g.line < l.lines.len(), "{name}: glif satırı");
        }
        for line in &l.lines {
            assert!(line.top.is_finite() && line.height.is_finite() && line.width.is_finite(), "{name}: satır");
            assert!(line.start <= line.end && line.end <= l.text_len, "{name}: satır aralığı");
        }
        for d in &l.decorations {
            assert!(d.rect.x.is_finite() && d.rect.y.is_finite() && d.rect.width.is_finite(), "{name}: süsleme");
        }
        // İmleç ve isabet sorguları (uzun metinde seyreltilmiş).
        let step = (l.text_len / 64).max(1);
        for byte in (0..=l.text_len + 2).step_by(step) {
            let r = l.caret_rect(byte);
            assert!(r.x.is_finite() && r.y.is_finite() && r.height.is_finite(), "{name}: imleç {byte}");
            let _ = l.cursor_x(byte);
        }
        for p in [Point::new(f32::NAN, f32::NAN), Point::new(f32::INFINITY, -f32::INFINITY), Point::new(-1e30, 1e30), Point::new(3.0, 5.0)] {
            let b = l.hit(p);
            assert!(b <= l.text_len, "{name}: isabet {b}");
            let _ = l.link_at(p);
            let _ = l.hit_byte(p.x);
        }
    }

    #[test]
    fn hostile_text_classes_layout_without_panic() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let modes = [OpticalCenter::Auto, OpticalCenter::Ink];
        for (name, text) in hostile_texts() {
            for size in [f32::NAN, -5.0, 0.0, 13.0, 1e9] {
                for mode in modes {
                    let style = TextStyle::new("sans-serif", size, 400).with_optical(mode);
                    // Uzun metinde yalnızca birkaç bileşim (şekillendirme pahalı).
                    let widths: &[Option<f32>] =
                        if text.len() > 4096 { &[None, Some(120.0)] } else { &[None, Some(0.0), Some(40.0)] };
                    if text.len() > 4096 && !(size.is_nan() && mode == OpticalCenter::Auto) {
                        continue;
                    }
                    for &w in widths {
                        assert_finite_layout(name, &ts.layout(&text, &style, w));
                        assert_finite_layout(name, &ts.layout(&text, &style.clone().with_max_lines(2), w));
                    }
                }
            }
            // Zengin metin: parça sınırında bağlantı/süsleme.
            let span = SpanStyle { underline: true, strikethrough: true, link: Some(u32::MAX), ..SpanStyle::default() };
            let half = text.len() / 2;
            let half = (0..=half).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
            let style = TextStyle::new("sans-serif", 13.0, 400);
            let l = ts.layout_rich(&[(&text[..half], span), (&text[half..], SpanStyle::default())], &style, Some(80.0));
            assert_finite_layout(name, &l);
        }
    }

    /// MAX_TEXT_BYTES: sınırın üstü karakter sınırında kesilir; rich parçalar toplamda da.
    #[test]
    fn long_text_is_truncated_on_char_boundary() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let style = TextStyle::new("sans-serif", 13.0, 400);
        let over = "😀".repeat(MAX_TEXT_BYTES / 4 + 3);
        let l = ts.layout(&format!("ab{over}"), &style, None);
        assert!(l.text_len <= MAX_TEXT_BYTES && l.text_len > MAX_TEXT_BYTES - 4, "{}", l.text_len);
        let a = "ğ".repeat(MAX_TEXT_BYTES / 2 - 1);
        let b = "x".repeat(10);
        let l = ts.layout_rich(&[(&a, SpanStyle::default()), (&b, SpanStyle::default()), (&b, SpanStyle::default())], &style, None);
        assert_eq!(l.text_len, MAX_TEXT_BYTES);
    }

    /// Painter::text bozuk ölçek ve kökenle: panik yok, kaydedilen glif anahtarının fiziksel
    /// boyutu sonlu ve sınırlı; rasterleştirme (atlas yolu) devasa bitmap ayırmaz.
    #[test]
    fn painter_text_survives_hostile_scale_and_origin() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let texts: Vec<String> = hostile_texts()
            .into_iter()
            .filter(|(_, t)| t.len() <= 4096)
            .map(|(_, t)| t)
            .chain(["Ag", "\u{F0239}", "👩\u{200D}👩\u{200D}👧"].map(String::from))
            .collect();
        let layouts: Vec<TextLayout> = texts
            .iter()
            .flat_map(|t| {
                [OpticalCenter::Auto, OpticalCenter::Ink].map(|m| (t, m))
            })
            .map(|(t, m)| {
                let style = TextStyle::new("sans-serif", 13.0, 400).with_optical(m);
                ts.layout(t, &style, Some(200.0))
            })
            .collect();
        let scales = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, 1e-30, 1.0, 1.25, 1.5, 2.0, 1e4, 1e9, f32::MAX];
        let origins = [Point::new(0.0, 0.0), Point::new(f32::NAN, f32::INFINITY), Point::new(-1e30, 1e30), Point::new(f32::MAX, f32::MIN)];
        let mut rasterized = std::collections::HashSet::new();
        for scale in scales {
            for origin in origins {
                let mut rec = Recording::default();
                {
                    let mut p = Painter::new(&mut rec, &mut ts, (64, 64), scale);
                    for l in &layouts {
                        p.text(l, origin, nuxut_core::Color::rgba(1.0, 1.0, 1.0, 1.0));
                    }
                }
                for g in &rec.glyphs {
                    let size = f32::from_bits(g.key.font_size_bits);
                    assert!(size.is_finite() && size > 0.0 && size <= MAX_GLYPH_PX, "ölçek {scale}: glif boyutu {size}");
                    // Atlas yolu: her farklı anahtar bir kez rasterleştirilir.
                    if rasterized.insert(g.key)
                        && let Some(img) = ts.rasterize(g.key)
                    {
                        let px = img.placement.width as u64 * img.placement.height as u64;
                        assert!(px <= (4 * MAX_GLYPH_PX as u64).pow(2), "ölçek {scale}: {px} px glif");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod paragraph_tests {
    use super::*;

    /// \n dışındaki Unicode paragraf ayırıcıları (U+2029, U+0085, \r, U+001C): glif bayt
    /// aralıkları doğru paragrafa göre hesaplanır (önceden yalnızca \n sayılıyordu; aralıklar
    /// kayıyor, metnin dışına taşıyordu).
    #[test]
    fn glyph_ranges_follow_all_paragraph_separators() {
        let mut ts = TextSystem::new();
        if ts.fonts.db().is_empty() {
            return;
        }
        let style = TextStyle::new("sans-serif", 13.0, 400);
        for sep in ["\n", "\r", "\r\n", "\u{2029}", "\u{85}", "\u{1C}"] {
            let text = format!("ab{sep}cd{sep}{sep}ef");
            let l = ts.layout(&text, &style, None);
            // cosmic-text salt ASCII metinde tek \r'yi bölmez (glif olur); harf glifleri yeter.
            let starts: Vec<usize> = l
                .glyphs
                .iter()
                .filter(|g| text.get(g.start..g.end).is_some_and(|s| !s.chars().all(char::is_control)))
                .map(|g| g.start)
                .collect();
            let want: Vec<usize> = text.char_indices().filter(|(_, c)| c.is_ascii_alphabetic()).map(|(i, _)| i).collect();
            assert_eq!(starts, want, "{sep:?}");
        }
    }
}
