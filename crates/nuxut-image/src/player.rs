//! Animasyon kare zamanlaması. Saat yalnızca görünür ve duraklatılmamışken ilerler; görünmez
//! animasyon için hiçbir iş yapılmaz (bir sonraki kare zamanı `None`).

use std::time::{Duration, Instant};

/// Kare zamanlayıcısı. Kare verisini tutmaz; yalnızca hangi karenin gösterileceğini ve
/// bir sonrakinin ne zaman geleceğini söyler. Uygulama `next_change` kadar sonra bir timer
/// kurar (ya da o anda kare ister); her vsync'te çizim gerekmez.
#[derive(Debug, Clone)]
pub struct Player {
    /// Karelerin bitiş anları (döngü başından, birikimli).
    ends: Vec<Duration>,
    /// Görünmezken/duraklatılmışken biriken oynatma süresi.
    elapsed: Duration,
    /// Saat çalışıyorsa başladığı an.
    running_since: Option<Instant>,
    visible: bool,
    paused: bool,
}

/// Bir karenin en kısa süresi (0 süreli kareler sonsuz döngü kilidi yaratmasın).
const MIN_DELAY: Duration = Duration::from_millis(10);

impl Player {
    /// `delays`: her karenin süresi. Başlangıçta görünmez kabul edilir; ilk
    /// [`set_visible`](Self::set_visible) saati başlatır.
    pub fn new(delays: impl IntoIterator<Item = Duration>) -> Self {
        let mut acc = Duration::ZERO;
        let ends = delays
            .into_iter()
            .map(|d| {
                acc += d.max(MIN_DELAY);
                acc
            })
            .collect();
        Self { ends, elapsed: Duration::ZERO, running_since: None, visible: false, paused: false }
    }

    pub fn for_animation(a: &crate::Animation) -> Self {
        Self::new(a.frames.iter().map(|f| f.delay))
    }

    pub fn frame_count(&self) -> usize {
        self.ends.len()
    }

    /// Yüzey görünür oldu/olmaz oldu (ör. tam ekran pencere duvar kağıdını örttü).
    pub fn set_visible(&mut self, visible: bool, now: Instant) {
        self.visible = visible;
        self.update_clock(now);
    }

    pub fn set_paused(&mut self, paused: bool, now: Instant) {
        self.paused = paused;
        self.update_clock(now);
    }

    pub fn is_running(&self) -> bool {
        self.running_since.is_some()
    }

    /// Başa sar (saat durumu korunur).
    pub fn restart(&mut self, now: Instant) {
        self.elapsed = Duration::ZERO;
        if self.running_since.is_some() {
            self.running_since = Some(now);
        }
    }

    /// `now` anında gösterilecek kare.
    pub fn frame(&self, now: Instant) -> usize {
        let Some(&total) = self.ends.last() else { return 0 };
        let t = self.position(now).as_nanos() % total.as_nanos();
        let t = Duration::from_nanos(t as u64);
        self.ends.partition_point(|&end| end <= t).min(self.ends.len() - 1)
    }

    /// Bir sonraki kare değişimine kalan süre; saat durmuşsa ya da tek kare varsa `None`.
    pub fn next_change(&self, now: Instant) -> Option<Duration> {
        if self.running_since.is_none() || self.ends.len() < 2 {
            return None;
        }
        let total = *self.ends.last()?;
        let t = Duration::from_nanos((self.position(now).as_nanos() % total.as_nanos()) as u64);
        let end = self.ends[self.frame(now)];
        Some(end.saturating_sub(t).max(Duration::from_millis(1)))
    }

    fn position(&self, now: Instant) -> Duration {
        self.elapsed + self.running_since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }

    fn update_clock(&mut self, now: Instant) {
        let should_run = self.visible && !self.paused && self.ends.len() > 1;
        match (self.running_since, should_run) {
            (None, true) => self.running_since = Some(now),
            (Some(s), false) => {
                self.elapsed += now.saturating_duration_since(s);
                self.running_since = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn advances_only_while_visible() {
        let t0 = Instant::now();
        let mut p = Player::new([ms(100), ms(50), ms(100)]);
        assert_eq!(p.frame(t0 + ms(500)), 0);
        assert_eq!(p.next_change(t0), None);
        p.set_visible(true, t0);
        assert_eq!(p.frame(t0 + ms(99)), 0);
        assert_eq!(p.frame(t0 + ms(100)), 1);
        assert_eq!(p.frame(t0 + ms(160)), 2);
        assert_eq!(p.frame(t0 + ms(250)), 0); // döngü
        assert_eq!(p.next_change(t0 + ms(120)), Some(ms(30)));
        // 120 ms'de gizle; 10 s sonra göster: 120 ms'den devam.
        p.set_visible(false, t0 + ms(120));
        assert_eq!(p.next_change(t0 + ms(5000)), None);
        p.set_visible(true, t0 + ms(10_000));
        assert_eq!(p.frame(t0 + ms(10_000)), 1);
        assert_eq!(p.frame(t0 + ms(10_040)), 2);
    }

    #[test]
    fn single_frame_never_ticks() {
        let t0 = Instant::now();
        let mut p = Player::new([ms(100)]);
        p.set_visible(true, t0);
        assert!(!p.is_running());
        assert_eq!(p.next_change(t0), None);
        assert_eq!(p.frame(t0 + ms(1000)), 0);
    }

    #[test]
    fn zero_delays_are_clamped() {
        let t0 = Instant::now();
        let mut p = Player::new([Duration::ZERO, Duration::ZERO]);
        p.set_visible(true, t0);
        assert_eq!(p.frame(t0 + ms(15)), 1);
    }
}
