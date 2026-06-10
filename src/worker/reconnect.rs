use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    max: Duration,
    jitter: bool,
    attempt: u32,
}

impl Backoff {
    pub fn new(initial_secs: u64, max_secs: u64, jitter: bool) -> Self {
        Self {
            initial: Duration::from_secs(initial_secs.max(1)),
            max: Duration::from_secs(max_secs.max(initial_secs.max(1))),
            jitter,
            attempt: 0,
        }
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    pub fn next_delay(&mut self, slow: bool) -> Duration {
        let exponent = self.attempt.min(20);
        self.attempt = self.attempt.saturating_add(1);
        let mut secs = self.initial.as_secs().saturating_mul(1u64 << exponent);
        if slow {
            secs = secs.max(30);
        }
        secs = secs.min(self.max.as_secs());
        let mut delay = Duration::from_secs(secs);
        if self.jitter {
            delay = apply_jitter(delay);
        }
        delay.max(Duration::from_secs(1))
    }
}

fn apply_jitter(delay: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let factor = 80 + (nanos % 41) as u64;
    Duration::from_millis((delay.as_millis() as u64).saturating_mul(factor) / 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_to_max_without_jitter() {
        let mut b = Backoff::new(2, 10, false);
        assert_eq!(b.next_delay(false), Duration::from_secs(2));
        assert_eq!(b.next_delay(false), Duration::from_secs(4));
        assert_eq!(b.next_delay(false), Duration::from_secs(8));
        assert_eq!(b.next_delay(false), Duration::from_secs(10));
    }

    #[test]
    fn slow_backoff_has_floor() {
        let mut b = Backoff::new(2, 60, false);
        assert_eq!(b.next_delay(true), Duration::from_secs(30));
    }
}
