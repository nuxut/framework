//! Hasar takibi: yalnızca değişen bölge yeniden çizilir.
//!
//! Uygulama `Painter::damage` ile bu karede değişen bölgeleri bildirir; bildirmezse tüm yüzey
//! hasarlı sayılır. Bölge ([`Region`]) birkaç ayrık dikdörtgendir: birbirinden uzak iki
//! değişiklik (bar'ın ortasındaki saat ve sağındaki sistem izleyici) tek sınırlayıcı kutuya
//! şişmez. Örtüşen ya da kenar paylaşan dikdörtgenler birleşir; üst sınır ([`MAX_RECTS`])
//! aşılınca birleşince alanı en az büyüyen çift birleştirilir.
//!
//! EGL tamponları dönüşümlüdür: `EGL_EXT_buffer_age` bir tamponun kaç kare önceki içeriği
//! taşıdığını söyler; o kareden bu yana biriken bölgelerin birleşimi yeniden çizilir. Yaş
//! bilinmiyorsa (0) tüm kare.

use std::collections::VecDeque;

/// Fiziksel piksel dikdörtgeni, sol-üst köşe orijinli.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PxRect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self { x, y, width, height }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// Alan (piksel); boşsa 0.
    pub fn area(&self) -> i64 {
        if self.is_empty() { 0 } else { i64::from(self.width) * i64::from(self.height) }
    }

    /// İkisini kapsayan en küçük dikdörtgen (sınırlayıcı kutu).
    pub fn union(&self, o: &Self) -> Self {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        Self::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }

    pub fn intersect(&self, o: &Self) -> Self {
        let (x, y) = (self.x.max(o.x), self.y.max(o.y));
        Self::new(x, y, (self.right().min(o.right()) - x).max(0), (self.bottom().min(o.bottom()) - y).max(0))
    }

    /// EGL `eglSwapBuffersWithDamage` biçimi: sol-alt orijinli `[x, y, w, h]`.
    pub fn to_egl(&self, surface_height: u32) -> [i32; 4] {
        [self.x, surface_height as i32 - self.bottom(), self.width, self.height]
    }
}

/// Bir bölgedeki en çok dikdörtgen. Her dikdörtgen renderer'da kayıtlı komutların bir kez
/// daha (scissor'la) gönderilmesi demektir; çok küçük parçalar yerine birkaç kutu.
pub const MAX_RECTS: usize = 8;

/// Birleşince sınırlayıcı kutuya eklenen boş alan bu kadar (fiziksel px²) ya da azsa iki
/// dikdörtgen birleşir: ayrı geçişin sabit maliyeti birkaç bin pikselin yeniden çiziminden
/// pahalı (yan yana iki metin satırı gibi neredeyse bitişik kutular).
const MERGE_SLACK: i64 = 32 * 32;

/// Ayrık (örtüşmeyen) dikdörtgenlerden oluşan hasar bölgesi; en çok [`MAX_RECTS`] parça.
///
/// Parçalar hiçbir pikselde örtüşmez: renderer her parçayı ayrı geçişte çizer ve örtüşen
/// alan iki kez harmanlanırdı. Örtüşen/kenar paylaşan dikdörtgen eklenince ikisi sınırlayıcı
/// kutularında birleşir (kutu yeni örtüşmeler doğurursa onlar da). Bölge her zaman eklenen
/// alanların tamamını kapsar; birleştirme yalnızca alanı büyütür.
#[derive(Clone, Copy)]
pub struct Region {
    /// `len`'den sonrası kullanılmaz. Taşma anında bir parça fazlası geçici olarak tutulur.
    rects: [PxRect; MAX_RECTS + 1],
    len: usize,
}

impl Region {
    /// Boş bölge: hiçbir şey değişmedi.
    pub const EMPTY: Self = Self {
        rects: [PxRect::new(0, 0, 0, 0); MAX_RECTS + 1],
        len: 0,
    };

    /// Tek dikdörtgenlik bölge (boşsa boş bölge).
    pub fn from_rect(r: PxRect) -> Self {
        let mut s = Self::EMPTY;
        s.add(r);
        s
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Ayrık parçalar.
    pub fn rects(&self) -> &[PxRect] {
        &self.rects[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    /// Toplam alan (parçalar ayrık olduğundan toplam).
    pub fn area(&self) -> i64 {
        self.rects().iter().map(PxRect::area).sum()
    }

    /// Tüm parçaları kapsayan kutu; boşsa boş dikdörtgen.
    pub fn bounds(&self) -> PxRect {
        self.rects().iter().fold(PxRect::default(), |a, b| a.union(b))
    }

    /// `r`'yi bölgeye ekler.
    pub fn add(&mut self, r: PxRect) {
        if r.is_empty() {
            return;
        }
        let mut cur = r;
        // Birleşen kutu yeni komşulara ulaşabilir: birleşecek parça kalmayana dek sür.
        while let Some(i) = self.rects().iter().position(|e| must_merge(e, &cur)) {
            cur = cur.union(&self.remove(i));
        }
        self.rects[self.len] = cur;
        self.len += 1;
        if self.len > MAX_RECTS {
            self.merge_cheapest();
        }
    }

    /// İki bölgenin birleşimi.
    pub fn union(mut self, o: &Self) -> Self {
        for r in o.rects() {
            self.add(*r);
        }
        self
    }

    /// Her parçayı `clip` ile keser (boş kalanlar düşer); ayrıklık korunur.
    pub fn clipped(&self, clip: &PxRect) -> Self {
        let mut out = Self::EMPTY;
        for r in self.rects() {
            let c = r.intersect(clip);
            if !c.is_empty() {
                out.rects[out.len] = c;
                out.len += 1;
            }
        }
        out
    }

    fn remove(&mut self, i: usize) -> PxRect {
        let r = self.rects[i];
        self.rects.copy_within(i + 1..self.len, i);
        self.len -= 1;
        self.rects[self.len] = PxRect::default();
        r
    }

    /// Üst sınır aşıldı: birleşince alanı en az büyüyen çifti birleştirir. Parçalar ayrık
    /// olduğundan artış = kutu − iki alan.
    fn merge_cheapest(&mut self) {
        let rects = self.rects();
        let mut best = (0, 1, i64::MAX);
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let growth = rects[i].union(&rects[j]).area() - rects[i].area() - rects[j].area();
                if growth < best.2 {
                    best = (i, j, growth);
                }
            }
        }
        let (i, j, _) = best;
        let b = self.remove(j);
        let a = self.remove(i);
        // Kutu başka parçalarla örtüşebilir: `add` onları da toplar (sayı en çok MAX_RECTS).
        self.add(a.union(&b));
    }
}

/// İki dikdörtgen birleşmeli mi: örtüşüyor ya da kenar paylaşıyor (yalnızca köşe değmesi
/// sayılmaz) ya da kutuları neredeyse boşluksuz ([`MERGE_SLACK`]).
fn must_merge(a: &PxRect, b: &PxRect) -> bool {
    let ix = a.right().min(b.right()) - a.x.max(b.x);
    let iy = a.bottom().min(b.bottom()) - a.y.max(b.y);
    if ix >= 0 && iy >= 0 && (ix > 0 || iy > 0) {
        return true;
    }
    a.union(b).area() - a.area() - b.area() <= MERGE_SLACK
}

/// Küme eşitliği: parça sırası önemsiz (parçalar ayrık ve boş değil, yineleme yok).
impl PartialEq for Region {
    fn eq(&self, o: &Self) -> bool {
        self.len == o.len && self.rects().iter().all(|r| o.rects().contains(r))
    }
}

impl Eq for Region {}

impl Default for Region {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl std::fmt::Debug for Region {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.rects()).finish()
    }
}

impl From<PxRect> for Region {
    fn from(r: PxRect) -> Self {
        Self::from_rect(r)
    }
}

/// Bir karenin hasarı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repaint {
    /// Tüm yüzey.
    Full,
    /// Yalnızca bu bölge (fiziksel piksel). Boş olabilir: hiçbir şey değişmedi.
    Partial(Region),
}

impl Repaint {
    /// Boş kısmi hasar.
    pub const NOTHING: Self = Self::Partial(Region::EMPTY);

    /// Tek dikdörtgenlik kısmi hasar.
    pub fn rect(r: PxRect) -> Self {
        Self::Partial(Region::from_rect(r))
    }

    /// Boş kısmi hasar: bu karede hiçbir şey değişmedi.
    pub fn is_nothing(&self) -> bool {
        matches!(self, Self::Partial(r) if r.is_empty())
    }

    pub fn union(self, o: Self) -> Self {
        match (self, o) {
            (Self::Partial(a), Self::Partial(b)) => Self::Partial(a.union(&b)),
            _ => Self::Full,
        }
    }

    /// `size_px` yüzeyinde dikdörtgenleri (`Full` → tüm yüzey); boş hasar → boş liste.
    pub fn region(&self, size_px: (u32, u32)) -> Region {
        let screen = PxRect::new(0, 0, size_px.0 as i32, size_px.1 as i32);
        match self {
            Self::Full => Region::from_rect(screen),
            Self::Partial(r) => r.clipped(&screen),
        }
    }
}

/// Yüzey başına hasar geçmişi; runtime tutar ve her karede `Renderer::render_frame`'e verir.
#[derive(Debug, Clone, Default)]
pub struct DamageHistory {
    size_px: (u32, u32),
    /// En yeni başta.
    frames: VecDeque<Repaint>,
    /// Son `accumulate`'in döndürdüğü yeniden çizim bölgesi (ölçüm/teşhis).
    last: Option<Repaint>,
}

/// Bundan eski tamponlar için tam kare çizilir (EGL genelde 2–4 tampon döndürür).
const MAX_AGE: usize = 6;

impl DamageHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bu karenin hasarını kaydeder ve tamponun yaşına göre yeniden çizilmesi gereken bölgeyi
    /// döndürür. `buffer_age` 0 = içerik bilinmiyor.
    ///
    /// Geçmiş **swap** başına tutulur (buffer age swap sayar): boş hasarlı kare (hiçbir şey
    /// değişmedi → swap yok) kaydedilmez ve boş döner; yoksa sonraki yaş hesabı bir kare
    /// kayar ve eski tampondaki bir bölge yeniden çizilmeden kalır (artık piksel).
    pub fn accumulate(&mut self, size_px: (u32, u32), buffer_age: u32, current: Repaint) -> Repaint {
        let region = self.accumulate_inner(size_px, buffer_age, current);
        self.last = Some(region);
        region
    }

    fn accumulate_inner(&mut self, size_px: (u32, u32), buffer_age: u32, current: Repaint) -> Repaint {
        if size_px != self.size_px {
            self.size_px = size_px;
            self.frames.clear();
            self.frames.push_front(Repaint::Full);
            return Repaint::Full;
        }
        if current.is_nothing() {
            return current;
        }
        self.frames.push_front(current);
        self.frames.truncate(MAX_AGE);
        let age = buffer_age as usize;
        if age == 0 || age > self.frames.len() {
            return Repaint::Full;
        }
        // Yaş n: tampon n kare önceki içeriği taşır → son n karenin (bu dahil) bölgelerinin
        // birleşimi (her parça ayrı; uzak bölgeler tek kutuya şişmez).
        let screen = PxRect::new(0, 0, size_px.0 as i32, size_px.1 as i32);
        match self.frames.iter().take(age).fold(Repaint::NOTHING, |a, b| a.union(*b)) {
            Repaint::Partial(r) => Repaint::Partial(r.clipped(&screen)),
            Repaint::Full => Repaint::Full,
        }
    }

    /// Bu boyutta geçmiş yok (ilk kare ya da yeniden boyut): tampon baştan, tümü çizilir ve
    /// compositor'a tüm yüzey hasarlı bildirilmelidir.
    pub fn is_reset_for(&self, size_px: (u32, u32)) -> bool {
        size_px != self.size_px
    }

    /// Son karede yeniden çizilen bölge (`accumulate`'in sonucu); henüz kare yoksa `None`.
    pub fn last_region(&self) -> Option<Repaint> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (u32, u32) = (100, 100);

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Repaint {
        Repaint::rect(PxRect::new(x, y, w, h))
    }

    /// Bölgenin değişmezleri: sınırda, parçalar boş değil ve ikişer ikişer ayrık.
    fn assert_valid(r: &Region) {
        assert!(r.len() <= MAX_RECTS, "{r:?}");
        for (i, a) in r.rects().iter().enumerate() {
            assert!(!a.is_empty(), "{r:?}");
            for b in &r.rects()[i + 1..] {
                assert!(a.intersect(b).is_empty(), "örtüşen parçalar {a:?} {b:?}");
            }
        }
    }

    fn covers(r: &Region, x: i32, y: i32) -> bool {
        r.rects().iter().any(|p| x >= p.x && x < p.right() && y >= p.y && y < p.bottom())
    }

    #[test]
    fn distant_rects_stay_separate() {
        // Bar: ortada saat, sağda sistem izleyici — tek kutuya (≈900 px) şişmemeli.
        let clock = PxRect::new(915, 0, 90, 36);
        let sysmon = PxRect::new(1750, 0, 120, 36);
        let r = Region::from_rect(clock).union(&Region::from_rect(sysmon));
        assert_eq!(r.rects(), &[clock, sysmon]);
        assert_eq!(r.area(), 90 * 36 + 120 * 36);
        assert_eq!(r.bounds(), PxRect::new(915, 0, 955, 36));
    }

    #[test]
    fn overlapping_and_adjacent_merge() {
        let mut r = Region::EMPTY;
        r.add(PxRect::new(0, 0, 10, 10));
        r.add(PxRect::new(5, 5, 10, 10));
        assert_eq!(r.rects(), &[PxRect::new(0, 0, 15, 15)]);
        // Kenar paylaşan.
        let mut r = Region::from_rect(PxRect::new(0, 0, 100, 100));
        r.add(PxRect::new(100, 20, 100, 100));
        assert_eq!(r.rects(), &[PxRect::new(0, 0, 200, 120)]);
        // İçerilen: bölge değişmez.
        let mut r = Region::from_rect(PxRect::new(0, 0, 100, 100));
        r.add(PxRect::new(10, 10, 5, 5));
        assert_eq!(r.rects(), &[PxRect::new(0, 0, 100, 100)]);
        // Yalnızca köşe değmesi (ve büyük boşluk): ayrı kalır.
        let mut r = Region::from_rect(PxRect::new(0, 0, 100, 100));
        r.add(PxRect::new(100, 100, 100, 100));
        assert_eq!(r.len(), 2);
        // Küçük boşluk (kutu artışı ≤ MERGE_SLACK): birleşir.
        let mut r = Region::from_rect(PxRect::new(0, 0, 40, 20));
        r.add(PxRect::new(42, 0, 40, 20));
        assert_eq!(r.rects(), &[PxRect::new(0, 0, 82, 20)]);
        // Boş dikdörtgen yok sayılır.
        r.add(PxRect::new(500, 500, 0, 10));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn merged_box_cascades_into_neighbours() {
        // Uzak iki parça; araya giren geniş kutu ikisiyle de örtüşür → hepsi tek kutu.
        let mut r = Region::from_rect(PxRect::new(0, 0, 100, 100));
        r.add(PxRect::new(400, 0, 100, 100));
        assert_eq!(r.len(), 2);
        r.add(PxRect::new(50, 300, 400, 10));
        assert_eq!(r.len(), 3);
        r.add(PxRect::new(90, 50, 320, 260));
        assert_eq!(r.rects(), &[PxRect::new(0, 0, 500, 310)]);
    }

    #[test]
    fn cap_merges_cheapest_pair() {
        let mut r = Region::EMPTY;
        // 8 uzak kare (aralık 1000 px) ve bir tanesine 150 px komşu dokuzuncu.
        for i in 0..MAX_RECTS as i32 {
            r.add(PxRect::new(i * 1000, 0, 100, 100));
        }
        assert_eq!(r.len(), MAX_RECTS);
        r.add(PxRect::new(3000 + 250, 0, 100, 100));
        assert_valid(&r);
        assert_eq!(r.len(), MAX_RECTS);
        // En ucuz çift (3000 ile 3250) birleşti; diğerleri yerinde.
        assert!(r.rects().contains(&PxRect::new(3000, 0, 350, 100)), "{r:?}");
        assert!(r.rects().contains(&PxRect::new(7000, 0, 100, 100)));
        assert!(r.rects().contains(&PxRect::new(0, 0, 100, 100)));
    }

    #[test]
    fn random_regions_are_disjoint_and_cover_inputs() {
        // Basit LCG: tekrarlanabilir rastgele dikdörtgenler.
        let mut seed = 0x2545_f491_u64;
        let mut next = |m: i32| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) % m as u64) as i32
        };
        for _ in 0..200 {
            let mut r = Region::EMPTY;
            let mut inputs = Vec::new();
            for _ in 0..1 + next(14) {
                let p = PxRect::new(next(380), next(180), 1 + next(60), 1 + next(40));
                inputs.push(p);
                r.add(p);
                assert_valid(&r);
            }
            for p in &inputs {
                for (x, y) in [(p.x, p.y), (p.right() - 1, p.bottom() - 1), (p.x + p.width / 2, p.y + p.height / 2)] {
                    assert!(covers(&r, x, y), "{p:?} kapsanmadı: {r:?}");
                }
            }
        }
    }

    #[test]
    fn clipped_drops_outside_parts() {
        let r = Region::from_rect(PxRect::new(-10, -10, 20, 20)).union(&Region::from_rect(PxRect::new(500, 0, 10, 10)));
        assert_eq!(r.clipped(&PxRect::new(0, 0, 100, 100)).rects(), &[PxRect::new(0, 0, 10, 10)]);
        assert_eq!(Repaint::Full.region((4, 3)).rects(), &[PxRect::new(0, 0, 4, 3)]);
        assert!(Repaint::NOTHING.region((4, 3)).is_empty());
    }

    #[test]
    fn first_frame_and_resize_are_full() {
        let mut h = DamageHistory::new();
        let small = rect(0, 0, 10, 10);
        assert_eq!(h.accumulate(SIZE, 1, small), Repaint::Full);
        assert_eq!(h.accumulate(SIZE, 1, small), small);
        assert_eq!(h.accumulate((50, 50), 1, small), Repaint::Full);
    }

    #[test]
    fn buffer_age_accumulates_previous_frames() {
        let mut h = DamageHistory::new();
        h.accumulate(SIZE, 0, Repaint::Full);
        h.accumulate(SIZE, 1, rect(0, 0, 10, 10));
        // Uzak iki bölge birleşmez: yaş 2 = iki parça.
        let r = h.accumulate(SIZE, 2, rect(50, 50, 10, 10));
        assert_eq!(r, Repaint::Partial(Region::from_rect(PxRect::new(0, 0, 10, 10)).union(&Region::from_rect(PxRect::new(50, 50, 10, 10)))));
        // Örtüşen önceki kare bölgesi birleşir.
        let r = h.accumulate(SIZE, 2, rect(55, 55, 10, 10));
        assert_eq!(r, rect(50, 50, 15, 15));
        // Yaş 4: ilk (tam) kareyi de kapsar.
        assert_eq!(h.accumulate(SIZE, 5, rect(0, 0, 1, 1)), Repaint::Full);
        // Bilinmeyen yaş.
        assert_eq!(h.accumulate(SIZE, 0, rect(0, 0, 1, 1)), Repaint::Full);
    }

    #[test]
    fn buffer_age_unions_multi_rect_frames() {
        // Her kare iki uzak parça (saat + sistem izleyici); yaş 3 = son üç karenin parçaları.
        let size = (1920, 36);
        let frame = |dx: i32| {
            Repaint::Partial(Region::from_rect(PxRect::new(900 + dx, 0, 90, 36)).union(&Region::from_rect(PxRect::new(1700 - dx, 0, 100, 36))))
        };
        let mut h = DamageHistory::new();
        h.accumulate(size, 0, Repaint::Full);
        h.accumulate(size, 1, frame(0));
        h.accumulate(size, 1, frame(5));
        let Repaint::Partial(r) = h.accumulate(size, 3, frame(10)) else { panic!("kısmi olmalı") };
        assert_valid(&r);
        assert_eq!(r, Region::from_rect(PxRect::new(900, 0, 100, 36)).union(&Region::from_rect(PxRect::new(1690, 0, 110, 36))));
        // Bölge ekran dışına taşmaz.
        let Repaint::Partial(r) = h.accumulate(size, 1, rect(1900, 30, 100, 100)) else { panic!() };
        assert_eq!(r.rects(), &[PxRect::new(1900, 30, 20, 6)]);
    }

    #[test]
    fn unchanged_frame_is_not_recorded() {
        let mut h = DamageHistory::new();
        let a = rect(0, 0, 10, 10);
        let b = rect(50, 50, 10, 10);
        h.accumulate(SIZE, 0, Repaint::Full);
        h.accumulate(SIZE, 1, a);
        // Swap'sız kare: boş döner, geçmişe girmez.
        assert_eq!(h.accumulate(SIZE, 2, Repaint::NOTHING), Repaint::NOTHING);
        assert_eq!(h.last_region(), Some(Repaint::NOTHING));
        // Yaş 2 = bir önceki swap'ın tamponu: a ∪ b (boş kare araya girmiş olsa da).
        assert_eq!(h.accumulate(SIZE, 2, b), a.union(b));
    }

    #[test]
    fn reset_on_new_size() {
        let mut h = DamageHistory::new();
        assert!(h.is_reset_for(SIZE));
        h.accumulate(SIZE, 0, Repaint::Full);
        assert!(!h.is_reset_for(SIZE));
        assert!(h.is_reset_for((50, 50)));
    }

    #[test]
    fn egl_rect_is_bottom_left() {
        assert_eq!(PxRect::new(10, 20, 30, 40).to_egl(100), [10, 40, 30, 40]);
    }
}
