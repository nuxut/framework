//! Bağlantıdan blob olarak büyüyen kapların ortak hareketi ([`Menu`](crate::Menu),
//! [`Popover`](crate::Popover), [`Select`](crate::Select)'in listesi).
//!
//! Kapalıyken bağlantıya bakan kenarda sıfır kalınlıklı bir tohum; açılınca hedef dikdörtgene
//! `spatial` hareketle büyür, büyüme hızı şekli esnetir (`Deform`, sabit nokta bağlantı
//! kenarı). Kapanırken tohuma geri erir; yay sıfırı aşarsa kalınlık bağlantıya gömülür
//! (shell'in blob alanında çökme/solma terimleri bunu yumuşatır). İçerik, şeklin ana
//! eksendeki doluluğuna göre belirir/söner.

use std::time::Instant;

use nuxut_core::{Corners, Point, Rect, Size};
use nuxut_motion::{Animated, Deform};
use nuxut_render::{BlobField, BlobShape, ShadowStyle};
use nuxut_theme::Theme;

use crate::menu::{MenuAlign, MenuAnchor, MenuSide};
use crate::ui::Ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Closed,
    Opening,
    Open,
    Closing,
}

#[derive(Debug, Clone)]
pub(crate) struct Grow {
    pub anchor: MenuAnchor,
    pub bounds: Rect,
    /// Gerçek açılış tarafı (sığmazsa çevrilir).
    pub side: MenuSide,
    phase: Phase,
    shape: Animated<Rect>,
    deform: Deform,
    /// Kapanış için son açık dikdörtgen (içerik solması buna göre).
    last_open: Rect,
    /// Bağımsız çizimde blob şeklinin son hasar bölgesi.
    last_damage: Option<Rect>,
    /// Kapanış [`shape`](Self::shape) içinde bitti (oturmuş tohum çizilmedi); bir sonraki
    /// [`settle`](Self::settle) bunu "kapanış bitti" olarak bir kez bildirir.
    closed_unreported: bool,
}

impl Grow {
    pub fn new(theme: &Theme) -> Self {
        Self {
            anchor: MenuAnchor::below(Rect::ZERO),
            bounds: Rect::ZERO,
            side: MenuSide::Below,
            phase: Phase::Closed,
            shape: Animated::new(Rect::ZERO, theme.motion.spatial),
            deform: Deform::new(theme.blob.deform_spring, theme.blob.deform),
            last_open: Rect::ZERO,
            last_damage: None,
            closed_unreported: false,
        }
    }

    pub fn set_theme(&mut self, theme: &Theme, now: Instant) {
        self.shape.set_motion(theme.motion.spatial, now);
        self.deform.set_params(theme.blob.deform_spring, theme.blob.deform, now);
    }

    pub fn is_open(&self) -> bool {
        matches!(self.phase, Phase::Opening | Phase::Open)
    }

    pub fn is_visible(&self) -> bool {
        self.phase != Phase::Closed
    }

    /// Bu karedeki (yay) dikdörtgen.
    #[cfg(test)]
    pub fn current(&self, now: Instant) -> Rect {
        self.shape.value(now)
    }

    /// Hedef (açık) dikdörtgen.
    pub fn target(&self) -> Rect {
        self.shape.target()
    }

    /// Şeklin hareketi (içerik geçişleri aynı dili konuşsun diye).
    pub fn motion(&self) -> nuxut_motion::Motion {
        self.shape.motion()
    }

    /// Açar (kapalıysa tohumdan, açıksa bulunduğu yerden yeni hedefe morf).
    pub fn open(&mut self, anchor: MenuAnchor, bounds: Rect, size: Size, now: Instant) -> Rect {
        self.anchor = anchor;
        self.bounds = bounds;
        let target = self.place(size);
        self.closed_unreported = false;
        if self.phase == Phase::Closed {
            self.shape.jump_to(self.seed_rect(target));
        }
        self.phase = Phase::Opening;
        self.shape.set_target(target, now);
        self.last_open = target;
        target
    }

    /// Açıkken içerik boyutu değişti (sayfa geçişi, model güncellemesi): yeni boyuta morf.
    pub fn resize(&mut self, size: Size, now: Instant) {
        if self.is_open() {
            let target = self.place(size);
            self.shape.set_target(target, now);
            self.last_open = target;
        }
    }

    pub fn close(&mut self, now: Instant) -> bool {
        if !self.is_open() {
            return false;
        }
        let target = self.shape.target();
        self.last_open = target;
        self.phase = Phase::Closing;
        self.shape.set_target(self.seed_rect(target), now);
        true
    }

    fn place(&mut self, size: Size) -> Rect {
        let (rect, side) = place(self.anchor, self.bounds, size);
        self.side = side;
        rect
    }

    /// Kapalı tohum: açık dikdörtgenin bağlantıya bakan kenarında, bağlantı merkezine yakın,
    /// sıfır kalınlıklı şerit.
    fn seed_rect(&self, open: Rect) -> Rect {
        let c = self.anchor.rect.center();
        match self.side {
            MenuSide::Below | MenuSide::Above => {
                let w = open.width * 0.5;
                let x = (c.x - w * 0.5).clamp(open.x, (open.right() - w).max(open.x));
                let y = if self.side == MenuSide::Below { open.y } else { open.bottom() };
                Rect::new(x, y, w, 0.0)
            }
            MenuSide::Left | MenuSide::Right => {
                let h = open.height * 0.5;
                let y = (c.y - h * 0.5).clamp(open.y, (open.bottom() - h).max(open.y));
                let x = if self.side == MenuSide::Right { open.x } else { open.right() };
                Rect::new(x, y, 0.0, h)
            }
        }
    }

    /// Bağlantıya yapışık kenarın normalize ortası (deform sabit noktası).
    fn deform_anchor(&self) -> Point {
        match self.side {
            MenuSide::Below => Point::new(0.5, 0.0),
            MenuSide::Above => Point::new(0.5, 1.0),
            MenuSide::Right => Point::new(0.0, 0.5),
            MenuSide::Left => Point::new(1.0, 0.5),
        }
    }

    /// Yapışık (`gap <= 0`) kenarın köşeleri düz; blob filetosu birleştirir.
    fn radii(&self, r: f32) -> Corners {
        if self.anchor.gap > 0.0 {
            return Corners::uniform(r);
        }
        match self.side {
            MenuSide::Below => Corners::new(0.0, 0.0, r, r),
            MenuSide::Above => Corners::new(r, r, 0.0, 0.0),
            MenuSide::Right => Corners::new(0.0, r, r, 0.0),
            MenuSide::Left => Corners::new(r, 0.0, 0.0, r),
        }
    }

    /// Bu karedeki ham (yay) dikdörtgen; negatif kalınlık bağlantıya gömülür.
    fn raw_rect(&self, now: Instant) -> Rect {
        let r = self.shape.value(now);
        let (mut x, mut y, mut w, mut h) = (r.x, r.y, r.width, r.height);
        if h < 0.0 {
            if self.side == MenuSide::Below {
                y += h;
            }
            h = 0.0;
        }
        if w < 0.0 {
            if self.side == MenuSide::Right {
                x += w;
            }
            w = 0.0;
        }
        Rect::new(x, y, w, h)
    }

    /// Blob şekli (kapalıyken `None`). Karede bir kez çağrılmalı (esneme durumu ilerler).
    pub fn shape(&mut self, now: Instant, radius: f32) -> Option<BlobShape> {
        if self.phase == Phase::Closing && self.shape.is_settled(now) && self.deform.is_settled(now) {
            // Kapanış oturdu: sıfır kalınlıklı tohum çizilmez (kenar yumuşatması ve gölgesi
            // soluk bir çizgi bırakır, ardından kare istenmediği için ekranda kalırdı). Hareket
            // kapalıyken bu, `close`'tan sonraki ilk karedir.
            self.phase = Phase::Closed;
            self.closed_unreported = true;
        }
        if self.phase == Phase::Closed {
            return None;
        }
        let rect = self.raw_rect(now);
        let v = self.shape.velocity(now);
        let growth = match self.side {
            MenuSide::Below | MenuSide::Above => Point::new(0.0, v.y + v.height * 0.5),
            MenuSide::Right | MenuSide::Left => Point::new(v.x + v.width * 0.5, 0.0),
        };
        self.deform.update(growth, now);
        Some(BlobShape::new(rect, self.radii(radius)).with_deform(self.deform.matrix(now), self.deform_anchor()))
    }

    /// İçerik opaklığı: açılırken/kapanırken şeklin ana eksendeki doluluğuna göre.
    pub fn reveal(&self, visual: Rect) -> f32 {
        match self.phase {
            Phase::Open => 1.0,
            Phase::Closed => 0.0,
            Phase::Opening | Phase::Closing => {
                let (cur, full) = match self.side {
                    MenuSide::Below | MenuSide::Above => (visual.height, self.last_open.height),
                    _ => (visual.width, self.last_open.width),
                };
                let frac = if full > 0.0 { cur / full } else { 1.0 };
                let r = ((frac - 0.35) / 0.5).clamp(0.0, 1.0);
                if self.phase == Phase::Opening && frac >= 0.98 { 1.0 } else { r }
            }
        }
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.shape.is_settled(now) | !self.deform.is_settled(now)
    }

    /// Kare sonu: açılış/kapanış bitti mi. Kapanış bittiyse `true`.
    pub fn settle(&mut self, now: Instant) -> bool {
        if std::mem::take(&mut self.closed_unreported) {
            return true;
        }
        let settled = self.shape.is_settled(now) && self.deform.is_settled(now);
        match self.phase {
            Phase::Opening if settled => self.phase = Phase::Open,
            Phase::Closing if settled => {
                self.phase = Phase::Closed;
                return true;
            }
            _ => {}
        }
        false
    }

    /// Kendi blob alanı olmayanlar için: şekli `surface` dolgusu ve tema gölgesiyle çizer,
    /// hasarını (eski ∪ yeni) bildirir.
    pub fn draw_standalone(&mut self, ui: &mut Ui, shape: Option<BlobShape>) {
        let theme = ui.theme;
        let Some(shape) = shape else {
            if let Some(d) = self.last_damage.take() {
                ui.damage(d);
            }
            return;
        };
        let field = BlobField {
            frame: None,
            shapes: vec![shape],
            smoothing: theme.blob.smoothing,
            // Tek başına şekil: köşelerini kendi yarıçapları belirler.
            corner_radius: 0.0,
            fill: ui.fade(theme.colors.surface),
            shadow: theme.shadow.enabled.then(|| ShadowStyle {
                color: ui.fade(theme.shadow.color),
                blur: theme.shadow.blur,
                offset: Point::new(0.0, theme.shadow.offset_y),
                spread: 0.0,
            }),
        };
        let d = field.damage_rect(&shape);
        if self.last_damage != Some(d) {
            for r in [self.last_damage, Some(d)].into_iter().flatten() {
                ui.damage(r);
            }
        }
        self.last_damage = Some(d);
        ui.painter.blob(&field);
    }
}

/// Sayfa geçişi (Menu alt sayfaları, PageStack): eski içerik `dir` yönünün tersine kayıp söner,
/// yeni içerik `dir` yönünden gelir. İlerleme kabın hareketiyle (aynı dil).
#[derive(Debug, Clone)]
pub(crate) struct Slide {
    /// +1 ileri (alt sayfaya), −1 geri.
    pub dir: f32,
    progress: Animated<f32>,
}

/// Bir sayfanın bu karedeki yatay kayması ve opaklığı.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SlideFrame {
    pub dx: f32,
    pub alpha: f32,
}

impl Slide {
    pub fn new(dir: f32, motion: nuxut_motion::Motion, now: Instant) -> Self {
        let mut progress = Animated::new(0.0, motion);
        progress.set_target(1.0, now);
        Self { dir, progress }
    }

    pub fn is_settled(&mut self, now: Instant) -> bool {
        self.progress.is_settled(now)
    }

    /// (eski sayfa, yeni sayfa) kayma ve opaklığı; `distance`: kayma mesafesi.
    pub fn frames(&self, now: Instant, distance: f32) -> (SlideFrame, SlideFrame) {
        let p = self.progress.value(now);
        let old = SlideFrame { dx: -self.dir * distance * p, alpha: (1.0 - p * 1.6).clamp(0.0, 1.0) };
        let new = SlideFrame { dx: self.dir * distance * (1.0 - p), alpha: ((p - 0.25) / 0.75).clamp(0.0, 1.0) };
        (old, new)
    }
}

/// Bağlantı ve sınırlara göre dikdörtgen ve gerçek açılış tarafı (sığmazsa çevrilir).
pub(crate) fn place(anchor: MenuAnchor, bounds: Rect, size: Size) -> (Rect, MenuSide) {
    let a = anchor.rect;
    let gap = anchor.gap.max(0.0);
    let w = size.width.min(bounds.width).max(0.0);
    let h = size.height.min(bounds.height).max(0.0);
    let align = |start: f32, extent: f32, len: f32, lo: f32, hi: f32| -> f32 {
        let v = match anchor.align {
            MenuAlign::Start => start,
            MenuAlign::Center => start + extent * 0.5 - len * 0.5,
            MenuAlign::End => start + extent - len,
        };
        v.clamp(lo, (hi - len).max(lo))
    };
    let side = match anchor.side {
        MenuSide::Below if a.bottom() + gap + h > bounds.bottom() && a.y - gap - h >= bounds.y => MenuSide::Above,
        MenuSide::Above if a.y - gap - h < bounds.y && a.bottom() + gap + h <= bounds.bottom() => MenuSide::Below,
        MenuSide::Right if a.right() + gap + w > bounds.right() && a.x - gap - w >= bounds.x => MenuSide::Left,
        MenuSide::Left if a.x - gap - w < bounds.x && a.right() + gap + w <= bounds.right() => MenuSide::Right,
        s => s,
    };
    let rect = match side {
        MenuSide::Below | MenuSide::Above => {
            let x = align(a.x, a.width, w, bounds.x, bounds.right());
            let y = if side == MenuSide::Below { a.bottom() + gap } else { a.y - gap - h };
            let y = y.clamp(bounds.y, (bounds.bottom() - h).max(bounds.y));
            Rect::new(x, y, w, h)
        }
        MenuSide::Left | MenuSide::Right => {
            let y = align(a.y, a.height, h, bounds.y, bounds.bottom());
            let x = if side == MenuSide::Right { a.right() + gap } else { a.x - gap - w };
            let x = x.clamp(bounds.x, (bounds.right() - w).max(bounds.x));
            Rect::new(x, y, w, h)
        }
    };
    (rect, side)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn grows_from_seed_and_melts_back() {
        let theme = Theme::default();
        let t0 = Instant::now();
        let mut g = Grow::new(&theme);
        let anchor = MenuAnchor::below(Rect::new(100.0, 0.0, 40.0, 40.0));
        let target = g.open(anchor, Rect::new(0.0, 0.0, 800.0, 600.0), Size::new(200.0, 300.0), t0);
        assert_eq!(g.shape.value(t0).height, 0.0);
        let t1 = t0 + Duration::from_secs(3);
        assert!(g.shape(t1, 20.0).is_some());
        while g.is_animating(t1) {}
        g.settle(t1);
        assert!(g.is_open());
        assert_eq!(g.shape.value(t1), target);
        assert!(g.close(t1));
        let t2 = t1 + Duration::from_secs(3);
        let _ = g.is_animating(t2);
        assert!(g.settle(t2));
        assert!(g.shape(t2, 20.0).is_none());
    }

    #[test]
    fn flips_above_when_no_room() {
        let size = Size::new(200.0, 300.0);
        let anchor = MenuAnchor { gap: 4.0, ..MenuAnchor::below(Rect::new(10.0, 900.0, 20.0, 20.0)) };
        let (r, side) = place(anchor, Rect::new(0.0, 0.0, 1000.0, 1000.0), size);
        assert_eq!(side, MenuSide::Above);
        assert_eq!(r.bottom(), 896.0);
    }
}
