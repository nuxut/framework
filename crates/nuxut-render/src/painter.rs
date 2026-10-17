//! `Painter`: çizim komutlarını kaydeder. GL çağrısı yapılmaz; `Renderer::render` sonunda
//! kayıtlar sırası korunarak toplu (instanced) çizilir.

use cosmic_text::CacheKey;
use nuxut_core::{Color, Corners, Point, Rect, Size};

use crate::blob::BlobField;
use crate::damage::{PxRect, Repaint};
use crate::gradient::GradientInstance;
use crate::stroke::{KIND_ARC, KIND_AREA, KIND_SEGMENT, StrokeInstance};
use crate::style::{RectStyle, ShadowStyle};
use crate::text::{TextLayout, TextSystem, from_cosmic};
use crate::gl::GlPass;
use crate::texture::{self, BlurJob, ImageData, ImageDraw, ImageStyle, PassJob, TargetAlloc, Texture, TextureOptions, Upload};

/// Dikdörtgen/gölge örneği (fiziksel piksel). Düzen shader'daki attribute'larla aynı.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct QuadInstance {
    /// x, y, w, h — şeklin kendisi (genişletilmemiş).
    pub rect: [f32; 4],
    /// sol-üst, sağ-üst, sağ-alt, sol-alt.
    pub radii: [f32; 4],
    /// Premultiplied.
    pub fill: [f32; 4],
    /// Premultiplied.
    pub border: [f32; 4],
    /// border_width, sigma (0 = keskin şekil), quad payı, kullanılmıyor.
    pub params: [f32; 4],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GlyphRef {
    pub key: CacheKey,
    /// Glyph orijininin tam piksel konumu (fiziksel).
    pub x: i32,
    pub y: i32,
    /// Premultiplied.
    pub color: [f32; 4],
}

#[derive(Debug, Clone)]
pub(crate) enum Op {
    Quads(std::ops::Range<usize>),
    Gradients(std::ops::Range<usize>),
    Strokes(std::ops::Range<usize>),
    Glyphs(std::ops::Range<usize>),
    Blob(usize),
    Image(usize),
    /// Fiziksel piksel scissor; `None` = kapalı.
    Clip(Option<[i32; 4]>),
}

#[derive(Debug, Default)]
pub(crate) struct Recording {
    /// Bu karede bildirilen hasar; `None` = hiç bildirilmedi (tüm yüzey).
    pub damage: Option<Repaint>,
    /// Etkin katmanın komutları. Kaplama ([`Painter::begin_overlay`]) açıkken buradaki liste
    /// kaplamanınkidir; temel katman `parked_ops`'ta bekler (yer değiştirme: çizim yolları
    /// hep `ops`'a yazar).
    pub ops: Vec<Op>,
    /// Etkin olmayan katmanın komutları: kaplama kapalıyken kaplama listesi, açıkken temel
    /// liste. Kare sonunda [`Recording::merge_overlay`] kaplamayı kırpmasız en sona ekler.
    pub parked_ops: Vec<Op>,
    pub quads: Vec<QuadInstance>,
    pub gradients: Vec<GradientInstance>,
    pub strokes: Vec<StrokeInstance>,
    pub glyphs: Vec<GlyphRef>,
    pub blobs: Vec<BlobField>,
    pub images: Vec<ImageDraw>,
    /// Bu karede yüklenecek dokular (çizimden önce, erken dönüşte de işlenir).
    pub uploads: Vec<Upload>,
    /// Bu karede istenen bulanıklıklar.
    pub blurs: Vec<BlurJob>,
    /// Bu karede ayrılacak boş hedef dokular ve çalıştırılacak GL geçişleri.
    pub targets: Vec<TargetAlloc>,
    pub passes: Vec<PassJob>,
    /// Düşen `Texture` tutamaklarının id kuyruğu (renderer'a ait); `clear` dokunmaz.
    pub release: Option<std::sync::mpsc::Sender<u64>>,
    /// GL thread'inde, context güncelken düşürülecek değerlerin kuyruğu (renderer'a ait;
    /// `gl::GlGraveyard`); `clear` dokunmaz.
    pub graveyard: Option<std::sync::mpsc::Sender<crate::gl::GraveItem>>,
}

impl Recording {
    pub fn clear(&mut self) {
        self.damage = None;
        self.ops.clear();
        self.parked_ops.clear();
        self.quads.clear();
        self.gradients.clear();
        self.strokes.clear();
        self.glyphs.clear();
        self.blobs.clear();
        self.images.clear();
        self.uploads.clear();
        self.blurs.clear();
        self.targets.clear();
        self.passes.clear();
    }

    /// Kare sonu: kaplama komutları kırpma kapatılarak temel katmanın sonuna eklenir (en
    /// üstte, hiçbir kapsayıcının kırpmasına takılmadan çizilir).
    pub(crate) fn merge_overlay(&mut self) {
        if self.parked_ops.is_empty() {
            return;
        }
        self.ops.push(Op::Clip(None));
        self.ops.append(&mut self.parked_ops);
    }
}

/// Bir karenin çizim yüzeyi. Tüm koordinatlar mantıksal; ölçek içeride uygulanır.
///
/// İki katman vardır: temel ve **kaplama** (overlay). [`begin_overlay`](Self::begin_overlay)
/// ile [`end_overlay`](Self::end_overlay) arasındaki çizimler kare sonunda her şeyin üstüne,
/// temel katmanın kırpma yığınından bağımsız çizilir (ipucu, açılır menü, popover: kaydırma
/// alanı ya da kartın içindeki tetikleyiciden açılsalar da kırpılmaz). Kaplamanın kendi
/// kırpma yığını boş başlar.
#[derive(Debug)]
pub struct Painter<'a> {
    pub(crate) rec: &'a mut Recording,
    text: &'a mut TextSystem,
    pub(crate) scale: f32,
    pub(crate) size_px: (u32, u32),
    clips: Vec<[i32; 4]>,
    /// Kaplama açıkken temel katmanın kırpma yığını (açık değilse `None`).
    base_clips: Option<Vec<[i32; 4]>>,
}

impl Drop for Painter<'_> {
    fn drop(&mut self) {
        if self.in_overlay() {
            tracing::warn!("Painter: kaplama kapatılmadan kare bitti");
            self.end_overlay();
        }
    }
}

impl<'a> Painter<'a> {
    pub(crate) fn new(rec: &'a mut Recording, text: &'a mut TextSystem, size_px: (u32, u32), scale: f32) -> Self {
        Self { rec, text, scale, size_px, clips: Vec::new(), base_clips: None }
    }

    /// Kaplama katmanına geçer. Zaten kaplamadaysa hiçbir şey yapmaz ve `false` döner (iç
    /// içe çağrı: dıştaki kapatır). `true` dönerse çağıran [`end_overlay`](Self::end_overlay)
    /// ile kapatmalı.
    pub fn begin_overlay(&mut self) -> bool {
        if self.in_overlay() {
            return false;
        }
        std::mem::swap(&mut self.rec.ops, &mut self.rec.parked_ops);
        self.base_clips = Some(std::mem::take(&mut self.clips));
        true
    }

    /// Temel katmana döner; kaplamanın kapatılmamış kırpmaları atılır.
    pub fn end_overlay(&mut self) {
        let Some(base) = self.base_clips.take() else {
            tracing::warn!("end_overlay: kaplama açık değil");
            return;
        };
        if !self.clips.is_empty() {
            tracing::warn!("end_overlay: kaplamada kapatılmamış kırpma");
            self.rec.ops.push(Op::Clip(None));
        }
        self.clips = base;
        std::mem::swap(&mut self.rec.ops, &mut self.rec.parked_ops);
    }

    /// Şu an kaplama katmanına mı çiziliyor?
    pub fn in_overlay(&self) -> bool {
        self.base_clips.is_some()
    }

    /// Çizim sırasında metin şekillendirmek için (ör. önbelleği boş bir `Label`). Sonuç
    /// çağıran tarafından önbelleklenmeli; her karede şekillendirme pahalıdır.
    pub fn text_system(&mut self) -> &mut TextSystem {
        self.text
    }

    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Hedefin mantıksal boyutu.
    pub fn size(&self) -> Size {
        Size::new(self.size_px.0 as f32 / self.scale, self.size_px.1 as f32 / self.scale)
    }

    /// Yuvarlak uçlu doğru parçası (`width` mantıksal piksel). Kenarlar SDF ile 1 fiziksel
    /// piksel yumuşatılır; konum yuvarlanmaz (eğik çizgiler alt piksel doğrulukta).
    pub fn line(&mut self, p0: Point, p1: Point, width: f32, color: Color) {
        let s = self.scale;
        if color.a <= 0.0 || width.is_nan() || width <= 0.0 || ![p0.x, p0.y, p1.x, p1.y].iter().all(|v| v.is_finite()) {
            return;
        }
        self.push_stroke(StrokeInstance {
            a: [p0.x * s, p0.y * s, p1.x * s, p1.y * s],
            b: [0.0; 4],
            color: color.premultiplied(),
            param: [width * 0.5 * s, KIND_SEGMENT],
        });
    }

    /// Kırık çizgi: ardışık noktalar yuvarlak uçlu parçalarla birleşir (birleşimler doğal
    /// olarak yuvarlaktır). Yarı saydam renkte birleşim noktaları iki kez karışır; yarı saydam
    /// eğri gerekiyorsa rengi zemine önceden karıştırın.
    pub fn polyline(&mut self, points: &[Point], width: f32, color: Color) {
        for w in points.windows(2) {
            self.line(w[0], w[1], width, color);
        }
        if points.len() == 1 {
            self.line(points[0], points[0], width, color);
        }
    }

    /// Kırık çizginin (`points`) altındaki alan, `baseline` y'sine kadar: her parça için bir
    /// sütun örneği (eğik üst kenar 1 fiziksel px AA, sütunlar ek yeri ve çift karışım olmadan
    /// birleşir — yarı saydam renk güvenli). `fade`: tabandaki opaklık çarpanı (1 = düz renk,
    /// 0 = tabana doğru tamamen sönen; geçiş alanın en uç noktasından tabana tek parça).
    /// Taban noktaların üstündeyse alan yukarı doğru dolar. Noktalar x'e göre artan olmalıdır:
    /// x'i artmayan ya da sonlu olmayan uçlu parça atlanır (grafikte boşluk).
    pub fn area(&mut self, points: &[Point], baseline: f32, color: Color, fade: f32) {
        let s = self.scale;
        if color.a <= 0.0 || points.len() < 2 || !baseline.is_finite() {
            return;
        }
        let fade = if fade.is_finite() { fade.clamp(0.0, 1.0) } else { 1.0 };
        let finite = |p: &Point| p.x.is_finite() && p.y.is_finite();
        // Geçişin başladığı uç: tabandan en uzak sonlu nokta.
        let (lo, hi) = points
            .iter()
            .filter(|p| finite(p))
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
        if lo > hi {
            return;
        }
        let top = if lo <= baseline { lo } else { hi };
        let color = color.premultiplied();
        for w in points.windows(2) {
            let (p0, p1) = (w[0], w[1]);
            if !finite(&p0) || !finite(&p1) || p1.x <= p0.x {
                continue;
            }
            self.push_stroke(StrokeInstance {
                a: [p0.x * s, p0.y * s, p1.x * s, p1.y * s],
                b: [baseline * s, fade, top * s, 0.0],
                color,
                param: [0.0, KIND_AREA],
            });
        }
    }

    /// Yuvarlak uçlu çember yayı (tek dörtgen, SDF). Açılar radyan; 0 = +x, pozitif süpürme
    /// ekranda saat yönü (y aşağı). `sweep` ≥ 2π tam halkadır.
    pub fn arc(&mut self, center: Point, radius: f32, start: f32, sweep: f32, width: f32, color: Color) {
        let s = self.scale;
        if color.a <= 0.0 || width.is_nan() || width <= 0.0 || radius.is_nan() || radius <= 0.0 || ![center.x, center.y, start, sweep].iter().all(|v| v.is_finite()) {
            return;
        }
        self.push_stroke(StrokeInstance {
            a: [center.x * s, center.y * s, radius * s, 0.0],
            b: [start, sweep.clamp(-std::f32::consts::TAU, std::f32::consts::TAU), 0.0, 0.0],
            color: color.premultiplied(),
            param: [width * 0.5 * s, KIND_ARC],
        });
    }

    fn push_stroke(&mut self, inst: StrokeInstance) {
        let start = self.rec.strokes.len();
        self.rec.strokes.push(inst);
        match self.rec.ops.last_mut() {
            Some(Op::Strokes(range)) if range.end == start => range.end = start + 1,
            _ => self.rec.ops.push(Op::Strokes(start..start + 1)),
        }
    }

    /// Köşe renkleri arasında bilineer geçişli yuvarlatılmış dikdörtgen. `colors` sırası:
    /// sol-üst, sağ-üst, sağ-alt, sol-alt (düz alpha; karışım premultiplied uzayda). Yatay
    /// geçiş için `[a, b, b, a]`, dikey için `[a, a, b, b]`.
    pub fn gradient_rect(&mut self, rect: Rect, radii: Corners, colors: [Color; 4]) {
        let px = self.snap(rect);
        if px.is_empty() || colors.iter().all(|c| c.a <= 0.0) {
            return;
        }
        let radii = self.scaled_radii(radii.clamped(rect.size()));
        let start = self.rec.gradients.len();
        self.rec.gradients.push(GradientInstance {
            rect: [px.x, px.y, px.width, px.height],
            radii,
            colors: colors.map(|c| c.premultiplied()),
        });
        match self.rec.ops.last_mut() {
            Some(Op::Gradients(range)) if range.end == start => range.end = start + 1,
            _ => self.rec.ops.push(Op::Gradients(start..start + 1)),
        }
    }

    /// `source`'un bulanık kopyası (aynı boyutta, premultiplied). Sonuç bu karede (kaynak
    /// hazırsa) GPU'da üretilir; `is_ready()` olana kadar çizilmez — runtime hazır olana dek
    /// kare ister. **Her kare çağırma**: kaynak ya da yarıçap değişmedikçe dönen dokuyu sakla.
    /// `radius` mantıksal piksel (dual Kawase; yaklaşık Gauss yarıçapı). Kaynağın bütçeye
    /// takılıp bekleyen bir güncellemesi (`update_texture`) varsa bulanıklık o güncelleme
    /// GPU'ya ulaşınca üretilir; o zamana dek sonuç hazır değildir (eski içerik bulanıklaşmaz).
    pub fn blur_texture(&mut self, source: &Texture, radius: f32) -> Texture {
        let (w, h) = source.size();
        let t = texture::new_texture(self.rec.release.clone(), w, h);
        self.rec.blurs.push(BlurJob {
            src: source.clone(),
            dst_id: t.id(),
            dst: texture::weak(&t),
            radius_px: radius * self.scale,
        });
        t
    }

    /// Yeni doku. GL çağrısı yapılmaz: renderer bu karede, çizimden önce yükler. `image`
    /// premultiplied RGBA8 olmalı; geçersiz ya da `MAX_TEXTURE_SIDE`'ı aşan veri yüklenmez
    /// (uyarı loglanır, doku çizilmez). Tutamak düşünce doku sonraki karede serbest kalır.
    pub fn upload_texture(&mut self, image: ImageData, options: TextureOptions) -> Texture {
        let t = texture::new_texture(self.rec.release.clone(), image.width, image.height);
        self.rec.uploads.push(Upload { id: t.id(), handle: texture::weak(&t), data: image, options: Some(options) });
        t
    }

    /// Mevcut dokunun içeriğini değiştirir (animasyon karesi). Boyut aynıysa yerinde
    /// güncellenir, farklıysa yeniden ayrılır.
    pub fn update_texture(&mut self, texture: &Texture, image: ImageData) {
        texture::resize_handle(texture, image.width, image.height);
        // Aynı karede bekleyen yükleme varsa yalnızca sonuncusu gerekir (seçenekleri korunur).
        let id = texture.id();
        let options = self.rec.uploads.iter().rev().find(|u| u.id == id).and_then(|u| u.options);
        self.rec.uploads.retain(|u| u.id != id);
        self.rec.uploads.push(Upload { id, handle: texture::weak(texture), data: image, options });
    }

    /// Dış GL üreticisinin (ör. video) çizeceği boş doku; fiziksel `width`×`height`, şeffaf
    /// başlar. Bu karede ayrılır (yükleme bütçesine sayılmaz). Boyut değişmedikçe aynı dokuyu
    /// tutun: her karede çağırmak her karede yeni doku ayırır.
    pub fn render_target(&mut self, width: u32, height: u32) -> Texture {
        let t = texture::new_texture(self.rec.release.clone(), width.max(1), height.max(1));
        self.rec.targets.push(TargetAlloc { id: t.id(), handle: texture::weak(&t), width: width.max(1), height: height.max(1) });
        t
    }

    /// GL çöplüğünün göndericisi (`crate::gl::PainterGlExt::gl_graveyard`).
    pub(crate) fn graveyard_sender(&self) -> Option<std::sync::mpsc::Sender<crate::gl::GraveItem>> {
        self.rec.graveyard.clone()
    }

    /// GL uzantısının kancası (`crate::gl::PainterGlExt`).
    pub(crate) fn push_gl_pass(&mut self, target: &Texture, pass: std::sync::Arc<dyn GlPass>) {
        self.rec.passes.push(PassJob { target: target.clone(), pass });
    }

    /// Doku çizimi: `style.fit`'e göre yerleşir, köşeler SDF ile (1 fiziksel piksel AA)
    /// kırpılır, `opacity` ile çarpılır. Hasar bildirmez; değişen görsel için çağıran
    /// `damage` der.
    pub fn image(&mut self, rect: Rect, texture: &Texture, style: &ImageStyle) {
        let opacity = if style.opacity.is_finite() { style.opacity.clamp(0.0, 1.0) } else { 1.0 };
        if opacity <= 0.0 || rect.is_empty() {
            return;
        }
        let (dst, uv) = texture::layout(rect, texture.size(), style.fit);
        let s = self.scale;
        let radii = style.radii.clamped(dst.size());
        self.rec.images.push(ImageDraw {
            texture: texture.clone(),
            rect: [dst.x * s, dst.y * s, dst.width * s, dst.height * s],
            uv,
            radii: self.scaled_radii(radii),
            opacity,
            tint: style.tint.map_or([0.0; 4], |c| c.premultiplied()),
        });
        self.rec.ops.push(Op::Image(self.rec.images.len() - 1));
    }

    /// Yuvarlatılmış dikdörtgen: dolgu + opsiyonel (içe çizilen) kenarlık. Kenarlar fiziksel
    /// piksele hizalanır.
    pub fn rect(&mut self, rect: Rect, style: &RectStyle) {
        let px = self.snap(rect);
        if px.is_empty() {
            return;
        }
        let (border_width, border) = match style.border {
            Some(b) if b.width > 0.0 && b.color.a > 0.0 => (b.width * self.scale, b.color.premultiplied()),
            _ => (0.0, [0.0; 4]),
        };
        if style.fill.a <= 0.0 && border_width <= 0.0 {
            return;
        }
        let radii = style.radii.clamped(rect.size());
        self.push_quad(QuadInstance {
            rect: [px.x, px.y, px.width, px.height],
            radii: self.scaled_radii(radii),
            fill: style.fill.premultiplied(),
            border,
            params: [border_width, 0.0, 1.0, 0.0],
        });
    }

    /// `rect` biçimli (köşe yarıçapları `radii`) bir şeklin bulanık gölgesi.
    pub fn shadow(&mut self, rect: Rect, radii: Corners, style: &ShadowStyle) {
        if style.color.a <= 0.0 || rect.is_empty() {
            return;
        }
        let shape = rect.translate(style.offset).expand(style.spread);
        if shape.is_empty() {
            return;
        }
        let s = self.scale;
        // `blur` ≈ 2σ: gölge kenarı blur kadar mesafede neredeyse sıfırlanır.
        let sigma = (style.blur * 0.5 * s).max(0.0);
        let radii = Corners::new(
            radii.top_left + style.spread,
            radii.top_right + style.spread,
            radii.bottom_right + style.spread,
            radii.bottom_left + style.spread,
        )
        .clamped(shape.size());
        let pad = sigma * 3.0 + 1.0;
        self.push_quad(QuadInstance {
            rect: [shape.x * s, shape.y * s, shape.width * s, shape.height * s],
            radii: self.scaled_radii(radii),
            fill: style.color.premultiplied(),
            border: [0.0; 4],
            params: [0.0, sigma, pad, 0.0],
        });
    }

    /// Metin. `origin` layout kutusunun sol-üst köşesi. Yatayda glyph'ler alt piksel
    /// konumlandırılır; dikeyde 1.0 ölçekte taban çizgisi tam piksele yuvarlanır (`place_glyph`).
    pub fn text(&mut self, layout: &TextLayout, origin: Point, color: Color) {
        if layout.is_empty() || color.a <= 0.0 {
            return;
        }
        let s = self.scale;
        // Parça rengi varsa o kullanılır; çizim renginin opaklığı onu da söndürür.
        let resolve = |over: Option<Color>| match over {
            Some(c) => c.with_alpha(c.a * color.a),
            None => color,
        };
        let base = color.premultiplied();
        let start = self.rec.glyphs.len();
        for placed in &layout.glyphs {
            let (key, x, y) = crate::text::place_glyph(placed, origin, s);
            let c = match placed.glyph.color_opt {
                Some(over) => resolve(Some(from_cosmic(over))).premultiplied(),
                None => base,
            };
            self.rec.glyphs.push(GlyphRef { key, x, y, color: c });
        }
        let end = self.rec.glyphs.len();
        match self.rec.ops.last_mut() {
            Some(Op::Glyphs(range)) if range.end == start => range.end = end,
            _ => self.rec.ops.push(Op::Glyphs(start..end)),
        }
        for d in &layout.decorations {
            self.rect(d.rect.translate(origin), &RectStyle::filled(resolve(d.color), Corners::ZERO));
        }
    }

    /// Blob alanı (çerçeve + paneller, yumuşak birleşim).
    pub fn blob(&mut self, field: &BlobField) {
        if field.frame.is_none() && field.shapes.is_empty() {
            return;
        }
        self.rec.blobs.push(field.clone());
        self.rec.ops.push(Op::Blob(self.rec.blobs.len() - 1));
    }

    /// Bu karede değişen bölgeyi bildirir (mantıksal). Birden çok çağrı ayrık dikdörtgenlerden
    /// oluşan bir bölgede toplanır ([`crate::Region`]: uzak bölgeler ayrı kalır, örtüşen ya da
    /// bitişikler birleşir, en çok [`crate::MAX_DAMAGE_RECTS`] parça). Karede hiç çağrılmazsa tüm
    /// yüzey hasarlı sayılır. Hasar bildiren uygulama yine **her şeyi** çizer; renderer yalnızca
    /// hasarlı parçalara kırpar. Değişen şeklin eski ve yeni konumu birlikte bildirilmeli (eski
    /// yerin silinmesi için); taşan efektler (gölge, blob, bulanıklık) payıyla birlikte.
    pub fn damage(&mut self, rect: Rect) {
        if rect.is_empty() {
            self.rec.damage.get_or_insert(Repaint::NOTHING);
            return;
        }
        let s = self.scale;
        // AA ve alt piksel taşmaları için 1 fiziksel piksel pay.
        let r = Rect::new(rect.x * s, rect.y * s, rect.width * s, rect.height * s).expand(1.0).round_out();
        let px = Repaint::rect(PxRect::new(r.x as i32, r.y as i32, r.width as i32, r.height as i32));
        self.rec.damage = Some(match self.rec.damage {
            Some(d) => d.union(px),
            None => px,
        });
    }

    /// Tüm yüzeyi hasarlı işaretler.
    pub fn damage_all(&mut self) {
        self.rec.damage = Some(Repaint::Full);
    }

    /// Sonraki çizimleri `rect` (mevcut kırpmayla kesişimi) içine kırpar.
    pub fn push_clip(&mut self, rect: Rect) {
        let s = self.scale;
        let r = Rect::new(rect.x * s, rect.y * s, rect.width * s, rect.height * s).round_out();
        let mut c = [r.x as i32, r.y as i32, r.right() as i32, r.bottom() as i32];
        if let Some(top) = self.clips.last() {
            c = [c[0].max(top[0]), c[1].max(top[1]), c[2].min(top[2]), c[3].min(top[3])];
            c[2] = c[2].max(c[0]);
            c[3] = c[3].max(c[1]);
        }
        self.clips.push(c);
        self.rec.ops.push(Op::Clip(Some(c)));
    }

    pub fn pop_clip(&mut self) {
        if self.clips.pop().is_none() {
            tracing::warn!("pop_clip: kırpma yığını zaten boş");
        }
        self.rec.ops.push(Op::Clip(self.clips.last().copied()));
    }

    fn push_quad(&mut self, q: QuadInstance) {
        let start = self.rec.quads.len();
        self.rec.quads.push(q);
        match self.rec.ops.last_mut() {
            Some(Op::Quads(range)) if range.end == start => range.end = start + 1,
            _ => self.rec.ops.push(Op::Quads(start..start + 1)),
        }
    }

    fn snap(&self, r: Rect) -> Rect {
        let s = self.scale;
        let x0 = (r.x * s).round();
        let y0 = (r.y * s).round();
        let x1 = (r.right() * s).round();
        let y1 = (r.bottom() * s).round();
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    fn scaled_radii(&self, r: Corners) -> [f32; 4] {
        let s = self.scale;
        [r.top_left * s, r.top_right * s, r.bottom_right * s, r.bottom_left * s]
    }
}
