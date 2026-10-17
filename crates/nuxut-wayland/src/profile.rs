//! Açılış profili: süreç başlangıcından ilk kareye kadar aşama süreleri.
//!
//! `NUXUT_PROFILE_STARTUP=1` ise ilk kare gönderilince tek satırlık özet `info` düzeyinde,
//! değilse `debug` düzeyinde loglanır. Maliyeti birkaç `Instant::now()`.

use std::time::Instant;

#[derive(Debug)]
pub(crate) struct StartupProfile {
    start: Instant,
    /// `run` çağrılmadan önce süreçte geçen süre (ms; /proc'tan, 10 ms çözünürlük).
    before_run_ms: Option<f64>,
    marks: Vec<(&'static str, f64)>,
    done: bool,
    loud: bool,
}

impl StartupProfile {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            before_run_ms: process_age_ms(),
            marks: Vec::with_capacity(12),
            done: false,
            loud: std::env::var_os("NUXUT_PROFILE_STARTUP").is_some_and(|v| v != "0"),
        }
    }

    /// Aşama bitti; `run` başlangıcından bu yana geçen süre kaydedilir.
    pub fn mark(&mut self, phase: &'static str) {
        if !self.done {
            self.marks
                .push((phase, self.start.elapsed().as_secs_f64() * 1000.0));
        }
    }

    /// Aynı aşama zaten kaydedildiyse yok sayar (ör. birden çok yüzeyin ilk configure'ı).
    pub fn mark_once(&mut self, phase: &'static str) {
        if !self.marks.iter().any(|(p, _)| *p == phase) {
            self.mark(phase);
        }
    }

    /// Bir aşamanın kendi süresi (ör. Renderer::new) — başlangıca göre değil.
    pub fn mark_span(&mut self, phase: &'static str, since: Instant) {
        if !self.done {
            self.marks
                .push((phase, -(since.elapsed().as_secs_f64() * 1000.0)));
        }
    }

    /// İlk kare gönderildi: özeti bir kez logla.
    pub fn first_frame(&mut self) {
        if self.done {
            return;
        }
        self.mark("ilk kare");
        self.done = true;
        let mut line = String::new();
        if let Some(ms) = self.before_run_ms {
            line.push_str(&format!("süreç→run {ms:.0} ms; "));
        }
        let mut prev = 0.0;
        for (phase, ms) in &self.marks {
            if *ms < 0.0 {
                line.push_str(&format!("[{phase} {:.1} ms] ", -ms));
            } else {
                line.push_str(&format!("{phase} +{:.1} (={ms:.1}) ", ms - prev));
                prev = *ms;
            }
        }
        if self.loud {
            tracing::info!("açılış: {line}");
        } else {
            tracing::debug!("açılış: {line}");
        }
    }
}

/// Sürecin yaşı (ms): /proc/self/stat starttime ile /proc/uptime farkı.
fn process_age_ms() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // comm parantez içinde boşluk içerebilir: son ')'dan sonrası alanlardır (3. alandan başlar).
    let rest = &stat[stat.rfind(')')? + 2..];
    let starttime: f64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let uptime: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    // Saat tıkı Linux'ta pratikte her yerde 100 Hz.
    Some(((uptime - starttime / 100.0) * 1000.0).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_age_is_sane() {
        let age = process_age_ms().expect("/proc okunabilmeli");
        assert!((0.0..86_400_000.0).contains(&age), "{age}");
    }

    #[test]
    fn marks_stop_after_first_frame() {
        let mut p = StartupProfile::new();
        p.mark("a");
        p.mark_span("b", Instant::now());
        p.first_frame();
        p.mark("sonra");
        assert_eq!(p.marks.len(), 3);
        assert!(p.done);
    }
}
