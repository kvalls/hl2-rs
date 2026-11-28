//! A bounded distribution with exact short recordings and whole-run statistics.
const EXACT_LIMIT: usize = 4096;
const WARM_UP: usize = 30;
const SHIFT: u32 = 47;
const FIRST: u64 = 1013 << 5; // 2^-10 ms.
const LAST: u64 = 1043 << 5; // 2^20 ms.
const BUCKETS: usize = (LAST - FIRST) as usize + 2;

#[derive(Default)]
pub(super) struct Samples {
    calls: usize,
    exact: Vec<f64>,
    tail: Option<Box<Tail>>,
}

struct Tail {
    buckets: [u64; BUCKETS],
    sum: f64,
    slowest: Slowest,
}

#[derive(Default)]
struct Slowest([Option<(usize, f64)>; 3]);

impl Slowest {
    fn insert(&mut self, sample: (usize, f64)) {
        if let Some(i) = self.0.iter().position(|current| {
            current.is_none_or(|(index, ms)| sample.1 > ms || (sample.1 == ms && sample.0 < index))
        }) {
            self.0[i..].rotate_right(1);
            self.0[i] = Some(sample);
        }
    }

    fn max(&self) -> f64 {
        self.0[0].map_or(0., |(_, ms)| ms)
    }
}

impl Samples {
    pub(super) fn count(&self) -> usize {
        self.calls.saturating_sub(WARM_UP)
    }

    pub(super) fn record(&mut self, ms: f64) {
        self.calls += 1;
        let count = self.count();
        if count == 0 {
            return;
        }
        if count <= EXACT_LIMIT {
            self.exact.push(ms);
            return;
        }
        let tail = self.tail.get_or_insert_with(|| {
            Box::new(Tail {
                buckets: [0; BUCKETS],
                sum: 0.,
                slowest: Slowest::default(),
            })
        });
        tail.buckets[bucket(ms)] += 1;
        tail.sum += ms;
        tail.slowest.insert((count - 1, ms));
    }

    pub(super) fn report(&self) -> serde_json::Value {
        let count = self.count();
        let mut slowest = Slowest::default();
        for (index, &ms) in self.exact.iter().enumerate() {
            slowest.insert((index, ms));
        }
        let rank = |percent| (count.saturating_sub(1) as u128 * percent / 100) as usize;
        let (sum, p95, p99) = if let Some(tail) = &self.tail {
            let mut buckets = tail.buckets;
            for &ms in &self.exact {
                buckets[bucket(ms)] += 1;
            }
            for sample in tail.slowest.0.into_iter().flatten() {
                slowest.insert(sample);
            }
            let upper = |percent| bound(&buckets, rank(percent), slowest.max());
            (
                self.exact.iter().sum::<f64>() + tail.sum,
                upper(95),
                upper(99),
            )
        } else {
            let mut sorted = self.exact.clone();
            sorted.sort_by(f64::total_cmp);
            let at = |percent| sorted.get(rank(percent)).copied().unwrap_or_default();
            (sorted.iter().sum::<f64>(), at(95), at(99))
        };
        serde_json::json!({
            "calls": self.calls,
            "samples": count,
            "mean_ms": sum / count.max(1) as f64,
            "p95_ms": p95,
            "p99_ms": p99,
            "percentiles_exact": self.tail.is_none(),
            "max_ms": slowest.max(),
            "slowest": slowest.0.into_iter().flatten().collect::<Vec<_>>(),
        })
    }
}

fn bucket(ms: f64) -> usize {
    ((ms.to_bits() >> SHIFT).saturating_sub(FIRST - 1) as usize).min(BUCKETS - 1)
}

fn bound(buckets: &[u64; BUCKETS], rank: usize, max: f64) -> f64 {
    let mut cumulative = 0;
    for (index, count) in buckets.iter().enumerate() {
        cumulative += count;
        if cumulative > rank as u64 {
            let edge = f64::from_bits((FIRST + index as u64) << SHIFT);
            return if index == BUCKETS - 1 {
                max
            } else {
                edge.min(max)
            };
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::{EXACT_LIMIT, Samples, WARM_UP};

    #[test]
    fn reports_late_hitches_and_percentiles_across_the_full_recording() {
        let mut samples = Samples::default();
        for _ in 0..WARM_UP + EXACT_LIMIT {
            samples.record(1.);
        }
        assert_eq!(samples.report()["percentiles_exact"], true);
        for _ in 0..512 {
            samples.record(8.125);
        }
        for ms in [20., 21., 20.] {
            samples.record(ms);
        }
        let report = samples.report();
        let count = EXACT_LIMIT + 515;
        assert_eq!(report["samples"], count);
        assert_eq!(report["percentiles_exact"], false);
        for key in ["p95_ms", "p99_ms"] {
            let value = report[key].as_f64().unwrap();
            assert!((8.125..=8.25).contains(&value));
        }
        assert_eq!(
            report["mean_ms"],
            (4096. + 512. * 8.125 + 61.) / count as f64
        );
        assert_eq!(report["max_ms"], 21.);
        assert_eq!(
            report["slowest"],
            serde_json::json!([[4609, 21.], [4608, 20.], [4610, 20.]])
        );
        assert_eq!(samples.exact.len(), EXACT_LIMIT);
    }
}
