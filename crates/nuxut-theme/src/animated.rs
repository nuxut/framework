//! Tema geçişi: yeni tema geldiğinde renkler `motion.effects` hareketiyle yeni değerlere
//! akar; renk dışı token'lar (yarıçap, yay, tipografi) hemen yeni temadan gelir.

use std::time::Instant;

use nuxut_motion::Animated;

use crate::{Palette, Theme};

/// Uygulama `theme_changed`'de [`set`](Self::set) çağırır, çizimde
/// [`colors`](Self::colors) ile o anki renkleri okur; [`is_animating`](Self::is_animating)
/// `true` iken kare ister. Geçiş sürerken yeni tema gelirse o anki renklerden devam eder.
#[derive(Debug, Clone)]
pub struct AnimatedTheme {
    theme: Theme,
    from: Palette,
    progress: Animated<f32>,
}

impl AnimatedTheme {
    pub fn new(theme: Theme) -> Self {
        let progress = Animated::new(1.0, theme.motion.effects);
        Self { from: theme.colors, theme, progress }
    }

    /// Yeni tema; renkler o anki değerlerinden akmaya başlar.
    pub fn set(&mut self, theme: Theme, now: Instant) {
        if theme == self.theme {
            return;
        }
        self.from = self.colors(now);
        self.progress = Animated::new(0.0, theme.motion.effects);
        self.progress.set_target(1.0, now);
        self.theme = theme;
    }

    /// Hedef tema (renk dışı token'lar için doğrudan bunu kullan).
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// O anki renkler.
    pub fn colors(&self, now: Instant) -> Palette {
        let t = self.progress.value(now).clamp(0.0, 1.0);
        if t >= 1.0 { self.theme.colors } else { self.from.lerp(&self.theme.colors, t) }
    }

    /// O anki renklerle tam tema (renkler dışında hedef temanın kopyası).
    pub fn current(&self, now: Instant) -> Theme {
        Theme { colors: self.colors(now), ..self.theme.clone() }
    }

    pub fn is_animating(&mut self, now: Instant) -> bool {
        !self.progress.is_settled(now)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn colors_flow_and_settle() {
        let t0 = Instant::now();
        let a = Theme::default();
        let (b, _) = Theme::parse("[colors]\nsurface = \"#ffffff\"\n").unwrap();
        let mut at = AnimatedTheme::new(a.clone());
        at.set(b.clone(), t0);
        assert!(at.is_animating(t0));
        assert_eq!(at.colors(t0).surface, a.colors.surface);
        let mid = at.colors(t0 + Duration::from_millis(60)).surface;
        assert!(mid.r > a.colors.surface.r && mid.r < 1.0, "{mid:?}");
        let end = t0 + Duration::from_secs(2);
        assert!(!at.is_animating(end));
        assert_eq!(at.colors(end), b.colors);
        assert_eq!(at.theme(), &b);
    }

    #[test]
    fn retarget_continues_from_current_colors() {
        let t0 = Instant::now();
        let a = Theme::default();
        let (b, _) = Theme::parse("[colors]\nsurface = \"#ffffff\"\n").unwrap();
        let mut at = AnimatedTheme::new(a.clone());
        at.set(b, t0);
        let t1 = t0 + Duration::from_millis(50);
        let before = at.colors(t1).surface;
        at.set(a, t1);
        assert_eq!(at.colors(t1).surface, before, "sıçrama olmamalı");
    }

    #[test]
    fn disabled_motion_switches_instantly() {
        let t0 = Instant::now();
        let (a, _) = Theme::parse("[motion]\nenabled = false\n").unwrap();
        let (b, _) = Theme::parse("[motion]\nenabled = false\n[colors]\nsurface = \"#ffffff\"\n").unwrap();
        let mut at = AnimatedTheme::new(a);
        at.set(b.clone(), t0);
        assert!(!at.is_animating(t0));
        assert_eq!(at.colors(t0), b.colors);
    }
}
