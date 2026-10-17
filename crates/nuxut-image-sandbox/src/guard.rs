//! İşçi başlatmadan önceki kapılar (ADR-0007 §Havuz): hız sınırı (belirteç kovası), anahtar
//! susturma (art arda başarısızlık) ve negatif önbellek (işçiyi öldüren baytın özeti).
//! Zamanı parametre olarak alır; birim testleri gerçek saate bağlı değildir.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use nuxut_image::NO_KEY;

/// Belirteç kovası: saniyede `rate`, en fazla `burst`.
#[derive(Debug)]
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(rate: u32, burst: u32, now: Instant) -> Self {
        let burst = f64::from(burst.max(1));
        Self { rate: f64::from(rate), burst, tokens: burst, last: now }
    }

    /// Bir belirteç alır; kova boşsa `false`.
    pub fn take(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct KeyState {
    consecutive: u32,
    muted_until: Option<Instant>,
}

/// Anahtar başına art arda yalıtım başarısızlığı sayacı; eşikte anahtar bir süre susturulur.
#[derive(Debug)]
pub struct KeyGuard {
    threshold: u32,
    mute: Duration,
    keys: HashMap<u64, KeyState>,
}

/// Harita boşta büyümesin: bu kadar anahtarı aşınca etkisiz kayıtlar atılır.
const MAX_KEYS: usize = 4096;

impl KeyGuard {
    pub fn new(threshold: u32, mute: Duration) -> Self {
        Self { threshold: threshold.max(1), mute, keys: HashMap::new() }
    }

    pub fn is_muted(&mut self, key: u64, now: Instant) -> bool {
        if key == NO_KEY {
            return false;
        }
        match self.keys.get_mut(&key) {
            Some(s) => match s.muted_until {
                Some(t) if now < t => true,
                Some(_) => {
                    // Susturma bitti: sayaç sıfırdan başlar.
                    self.keys.remove(&key);
                    false
                }
                None => false,
            },
            None => false,
        }
    }

    pub fn success(&mut self, key: u64) {
        self.keys.remove(&key);
    }

    pub fn failure(&mut self, key: u64, now: Instant) {
        if key == NO_KEY {
            return;
        }
        let s = self.keys.entry(key).or_default();
        s.consecutive += 1;
        if s.consecutive >= self.threshold {
            s.muted_until = Some(now + self.mute);
        }
        if self.keys.len() > MAX_KEYS {
            self.keys.retain(|_, s| s.muted_until.is_some_and(|t| now < t));
            if self.keys.len() > MAX_KEYS {
                self.keys.clear();
            }
        }
    }
}

/// İşçiyi öldüren / zaman aşımına uğratan girdilerin özetleri (blake3, ilk 16 bayt), LRU.
#[derive(Debug)]
pub struct NegativeCache {
    cap: usize,
    entries: VecDeque<[u8; 16]>,
}

pub fn digest(bytes: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&blake3::hash(bytes).as_bytes()[..16]);
    out
}

impl NegativeCache {
    pub fn new(cap: usize) -> Self {
        Self { cap: cap.max(1), entries: VecDeque::new() }
    }

    /// Varsa en yeniye taşır ve `true` döner.
    pub fn hit(&mut self, d: &[u8; 16]) -> bool {
        match self.entries.iter().position(|e| e == d) {
            Some(i) => {
                if let Some(e) = self.entries.remove(i) {
                    self.entries.push_back(e);
                }
                true
            }
            None => false,
        }
    }

    pub fn insert(&mut self, d: [u8; 16]) {
        if self.hit(&d) {
            return;
        }
        if self.entries.len() >= self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(d);
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_bursts_then_refills() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(30, 60, t0);
        assert_eq!((0..100).filter(|_| b.take(t0)).count(), 60);
        assert!(!b.take(t0));
        // 1 sn sonra 30 belirteç.
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!((0..100).filter(|_| b.take(t1)).count(), 30);
        // Uzun boşluk patlamayı aşmaz.
        let t2 = t1 + Duration::from_secs(3600);
        assert_eq!((0..100).filter(|_| b.take(t2)).count(), 60);
    }

    #[test]
    fn key_mutes_after_consecutive_failures_and_recovers() {
        let t0 = Instant::now();
        let mut g = KeyGuard::new(3, Duration::from_secs(60));
        g.failure(7, t0);
        g.failure(7, t0);
        assert!(!g.is_muted(7, t0));
        g.success(7);
        g.failure(7, t0);
        g.failure(7, t0);
        assert!(!g.is_muted(7, t0), "başarı sayacı sıfırlar");
        g.failure(7, t0);
        assert!(g.is_muted(7, t0 + Duration::from_secs(59)));
        assert!(!g.is_muted(7, t0 + Duration::from_secs(61)));
        assert!(!g.is_muted(8, t0));
        for _ in 0..10 {
            g.failure(NO_KEY, t0);
        }
        assert!(!g.is_muted(NO_KEY, t0), "anahtarsız istek susturulmaz");
        for k in 1..=(MAX_KEYS as u64 + 10) {
            g.failure(k, t0);
        }
        assert!(g.keys.len() <= MAX_KEYS + 1);
    }

    #[test]
    fn negative_cache_is_bounded_lru() {
        let mut c = NegativeCache::new(3);
        let (a, b, d, e) = (digest(b"a"), digest(b"b"), digest(b"d"), digest(b"e"));
        c.insert(a);
        c.insert(b);
        c.insert(d);
        assert!(c.hit(&a), "a en yeniye taşındı");
        c.insert(e);
        assert!(!c.hit(&b), "en eski (b) atıldı");
        assert!(c.hit(&a) && c.hit(&d) && c.hit(&e));
        assert_eq!(c.len(), 3);
    }
}
