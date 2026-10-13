//! Fixed-size timing histogram. Percentiles are bucket upper bounds, not exact samples.
#[derive(Clone, Copy)]
pub struct Histogram {
    pub count: u64,
    pub total_us: u64,
    pub max_us: u64,
    pub buckets: [u64; 24],
}
impl Histogram {
    pub const fn new() -> Self {
        Self { count: 0, total_us: 0, max_us: 0, buckets: [0; 24] }
    }
    pub fn record(&mut self, us: u64) {
        let bucket = if us <= 1 { 0 } else { (64 - (us - 1).leading_zeros() as usize).min(23) };
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.count = self.count.saturating_add(1);
        self.total_us = self.total_us.saturating_add(us);
        self.max_us = self.max_us.max(us);
    }
    pub fn percentile_upper(&self, percent: u64) -> u64 {
        if self.count == 0 { return 0; }
        let target = (self.count.saturating_mul(percent.min(100)).saturating_add(99) / 100).max(1);
        let mut seen = 0;
        for (i, &n) in self.buckets.iter().enumerate() {
            seen += n;
            if seen >= target { return if i == 23 { self.max_us.max(1 << 23) } else { 1 << i }; }
        }
        self.max_us
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn boundaries_and_percentile_bounds() {
        let mut h = Histogram::new();
        assert_eq!(h.percentile_upper(95), 0);
        for x in [0, 1, 2, 3, 4, 5, 8, 9, 16, 17] { h.record(x); }
        assert_eq!(h.count, 10);
        assert_eq!(h.total_us, 65);
        assert_eq!(h.percentile_upper(50), 4);
        assert_eq!(h.percentile_upper(95), 32);
        assert_eq!(h.max_us, 17);
    }
    #[test]
    fn overflow_bucket_remains_an_upper_bound() {
        let mut h = Histogram::new();
        h.record(100_000_000);
        assert!(h.percentile_upper(95) >= h.max_us);
    }
}
