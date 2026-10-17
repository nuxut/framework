//! ureq taşıması üstünde EINTR (`ErrorKind::Interrupted`) yeniden denemesi.
//!
//! **Kök neden (2026-10-14):** ureq 3.4'ün `TcpTransport::await_input`'u soketten
//! tek bir `read` yapar ve `Interrupted`'ı yeniden denemeden `Error::Io` olarak döndürür. Linux'ta
//! `SO_RCVTIMEO`/`SO_SNDTIMEO` ayarlı bir soket çağrısı, sinyal işleyicisi `SA_RESTART` olsa bile
//! **hiçbir zaman yeniden başlatılmaz** (signal(7)); süreç durdurulup sürdürüldüğünde
//! (SIGSTOP/SIGCONT, cgroup dondurma) ya da herhangi bir sinyal işlendiğinde okuma `EINTR` ile
//! döner. Sessiz sunucuyu bekleyen istek böylece `HttpError::Timeout` yerine
//! `HttpError::Io(Interrupted)` verirdi (`times_out_on_silent_server` yük altında bir kez düştü:
//! testler `cargo-gate` ile `nuxut-build.slice`'ta koşar, bellek bekçisi baskıda slice'ı dondurup
//! çözer). Okuma sırasında test sürecine SIGSTOP/SIGCONT göndermek hatayı her seferinde üretir.
//!
//! Düzeltme: TCP taşıması ile TLS (rustls) arasına [`RetryInterrupted`] bağlayıcısı girer. Okuma ve
//! yazma `Interrupted` dönerse **kalan süreyle** yeniden denenir; süre dolmuşsa
//! `Error::Timeout` döner (toplam süre sınırı korunur, sonsuz döngü yok). rustls kendi baytlarını
//! bu taşıma üzerinden okuduğundan TLS yolu da aynı kurala uyar. Bağlanma yolu zaten güvenli:
//! std'nin `TcpStream::connect_timeout`'u `EINTR`'de kalan süreyle `poll`'u yineler.

use std::time::Instant;

use ureq::Error;
use ureq::unversioned::transport::time::Duration as UreqDuration;
use ureq::unversioned::transport::{Buffers, ConnectionDetails, Connector, NextTimeout, Transport};

/// Zincirdeki önceki taşımayı [`Interruptible`] ile sarar (bağlantı kurmaz).
#[derive(Debug, Default)]
pub(crate) struct RetryInterrupted;

impl<In: Transport> Connector<In> for RetryInterrupted {
    type Out = Interruptible<In>;

    fn connect(&self, _details: &ConnectionDetails, chained: Option<In>) -> Result<Option<Self::Out>, Error> {
        Ok(chained.map(Interruptible))
    }
}

/// `Interrupted`'ı kalan süre içinde yeniden deneyen taşıma.
#[derive(Debug)]
pub(crate) struct Interruptible<T>(pub(crate) T);

fn is_interrupted(e: &Error) -> bool {
    matches!(e, Error::Io(io) if io.kind() == std::io::ErrorKind::Interrupted)
}

/// `op`'u `Interrupted` döndükçe yeniden çağırır; her denemeye `timeout`'tan kalan süre verilir.
/// Süre sınırsızsa (`NotHappening`) her deneme de sınırsızdır; sınırlıysa ve süre bittiyse
/// `Error::Timeout(timeout.reason)` döner.
fn retry_interrupted<R>(timeout: NextTimeout, mut op: impl FnMut(NextTimeout) -> Result<R, Error>) -> Result<R, Error> {
    let start = Instant::now();
    let mut attempt = timeout;
    loop {
        match op(attempt) {
            Err(e) if is_interrupted(&e) => {}
            other => return other,
        }
        if let UreqDuration::Exact(total) = timeout.after {
            let left = total.saturating_sub(start.elapsed());
            if left.is_zero() {
                return Err(Error::Timeout(timeout.reason));
            }
            attempt = NextTimeout { after: UreqDuration::Exact(left), reason: timeout.reason };
        }
    }
}

impl<T: Transport> Transport for Interruptible<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        // `TcpTransport` `write_all` kullanır; o `Interrupted`'ı kendisi yiner ve dışarı vermez,
        // ama her yinelemede `SO_SNDTIMEO` baştan sayar. Bu katman yalnız dışarı sızan
        // `Interrupted`'ı (yazılan bayt yokken) kalan süreyle yeniden dener.
        retry_interrupted(timeout, |t| self.0.transmit_output(amount, t))
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        retry_interrupted(timeout, |t| self.0.await_input(t))
    }

    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }

    fn is_tls(&self) -> bool {
        self.0.is_tls()
    }
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::time::Duration;

    use ureq::Timeout;
    use ureq::unversioned::transport::LazyBuffers;

    use super::*;

    /// İlk `interrupts` okumada `Interrupted` döner, sonra `data`'yı verir. `interrupts` çok
    /// büyükse hiç veri vermez (sürekli sinyal yağmuru). Her çağrıya verilen süre kaydedilir.
    #[derive(Debug)]
    struct EintrReader {
        buffers: LazyBuffers,
        interrupts: usize,
        calls: usize,
        data: &'static [u8],
        seen: Vec<UreqDuration>,
        pause: Duration,
    }

    impl EintrReader {
        fn new(interrupts: usize, data: &'static [u8]) -> Self {
            Self { buffers: LazyBuffers::new(64, 64), interrupts, calls: 0, data, seen: Vec::new(), pause: Duration::ZERO }
        }
    }

    impl Transport for EintrReader {
        fn buffers(&mut self) -> &mut dyn Buffers {
            &mut self.buffers
        }

        fn transmit_output(&mut self, _amount: usize, timeout: NextTimeout) -> Result<(), Error> {
            self.seen.push(timeout.after);
            self.calls += 1;
            if self.calls <= self.interrupts {
                std::thread::sleep(self.pause);
                return Err(Error::Io(ErrorKind::Interrupted.into()));
            }
            Ok(())
        }

        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            self.seen.push(timeout.after);
            self.calls += 1;
            if self.calls <= self.interrupts {
                std::thread::sleep(self.pause);
                return Err(Error::Io(ErrorKind::Interrupted.into()));
            }
            let buf = self.buffers.input_append_buf();
            let n = self.data.len().min(buf.len());
            buf[..n].copy_from_slice(&self.data[..n]);
            self.buffers.input_appended(n);
            Ok(n > 0)
        }

        fn is_open(&mut self) -> bool {
            true
        }
    }

    fn timeout(after: UreqDuration) -> NextTimeout {
        NextTimeout { after, reason: Timeout::RecvResponse }
    }

    #[test]
    fn interrupted_read_is_retried_until_data_arrives() {
        let mut t = Interruptible(EintrReader::new(3, b"HTTP/1.1"));
        assert!(t.await_input(timeout(UreqDuration::from_secs(30))).unwrap());
        assert_eq!(t.0.calls, 4);
        assert_eq!(t.0.buffers.input(), b"HTTP/1.1");
        // Yeniden denemeler toplam süreyi uzatmaz: her deneme kalan süreyi alır, artmaz.
        let exact: Vec<_> = t.0.seen.iter().map(|d| **d).collect();
        assert!(exact.windows(2).all(|w| w[1] <= w[0]), "{exact:?}");
    }

    #[test]
    fn endless_interrupts_end_in_timeout_within_the_deadline() {
        let mut reader = EintrReader::new(usize::MAX, b"");
        reader.pause = Duration::from_millis(5);
        let mut t = Interruptible(reader);
        let started = Instant::now();
        let err = t.await_input(timeout(UreqDuration::from_millis(60))).unwrap_err();
        assert!(matches!(err, Error::Timeout(Timeout::RecvResponse)), "{err:?}");
        assert!(t.0.calls > 1, "yeniden denenmedi");
        // Güvenlik ağı: süre sınırı korunur (yük altında gecikmeye pay bırakılır).
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn interrupted_write_is_retried() {
        let mut t = Interruptible(EintrReader::new(2, b""));
        t.transmit_output(0, timeout(UreqDuration::from_secs(30))).unwrap();
        assert_eq!(t.0.calls, 3);
    }

    #[test]
    fn unbounded_timeout_stays_unbounded_across_retries() {
        let mut t = Interruptible(EintrReader::new(2, b"x"));
        assert!(t.await_input(timeout(UreqDuration::NotHappening)).unwrap());
        assert!(t.0.seen.iter().all(|d| d.is_not_happening()));
    }

    #[test]
    fn other_errors_are_not_retried() {
        #[derive(Debug)]
        struct Reset(LazyBuffers, usize);
        impl Transport for Reset {
            fn buffers(&mut self) -> &mut dyn Buffers {
                &mut self.0
            }
            fn transmit_output(&mut self, _: usize, _: NextTimeout) -> Result<(), Error> {
                Ok(())
            }
            fn await_input(&mut self, _: NextTimeout) -> Result<bool, Error> {
                self.1 += 1;
                Err(Error::Io(ErrorKind::ConnectionReset.into()))
            }
            fn is_open(&mut self) -> bool {
                false
            }
        }
        let mut t = Interruptible(Reset(LazyBuffers::new(8, 8), 0));
        let err = t.await_input(timeout(UreqDuration::from_secs(30))).unwrap_err();
        assert!(matches!(err, Error::Io(ref e) if e.kind() == ErrorKind::ConnectionReset), "{err:?}");
        assert_eq!(t.0.1, 1);
    }
}
