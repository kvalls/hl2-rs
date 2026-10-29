//! Allocation-free frame distribution, collected only by --profile after warm-up.
pub(super) struct Distribution {
    bins: [u32; 256],
    samples: u32,
    max: f64,
}
impl Default for Distribution {
    fn default() -> Self {
        Self {
            bins: [0; 256],
            samples: 0,
            max: 0.,
        }
    }
}
impl Distribution {
    pub(super) fn observe(&mut self, ms: f64) {
        self.bins[(ms.max(0.) as usize).min(255)] += 1;
        self.samples += 1;
        self.max = self.max.max(ms);
    }
    pub(super) fn percentile(&self, percent: u32) -> Option<f64> {
        if self.samples == 0 {
            return None;
        }
        let rank = (u64::from(self.samples) * u64::from(percent)).div_ceil(100);
        let mut count = 0;
        for (index, bin) in self.bins.iter().enumerate() {
            count += u64::from(*bin);
            if count >= rank {
                return Some(if index == 255 {
                    self.max
                } else {
                    (index + 1) as f64
                });
            }
        }
        Some(self.max)
    }
    pub(super) fn maximum(&self) -> Option<f64> {
        (self.samples > 0).then_some(self.max)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upper_bounds_include_long_hitches_without_allocating_or_truncating_them() {
        let mut d = Distribution::default();
        assert_eq!(d.percentile(95), None);
        for _ in 0..95 {
            d.observe(19.5);
        }
        for _ in 0..4 {
            d.observe(42.25);
        }
        d.observe(350.5);
        assert_eq!(d.percentile(95), Some(20.));
        assert_eq!(d.percentile(99), Some(43.));
        assert_eq!(d.percentile(100), Some(350.5));
        assert_eq!(d.maximum(), Some(350.5));
    }
}
