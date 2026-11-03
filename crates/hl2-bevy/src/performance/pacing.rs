//! Background test pacing is separate from uncapped profiling throughput.
use std::time::Duration;

pub(crate) fn background_interval(profile: bool) -> Duration {
    if profile {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(1. / 60.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_profiling_removes_the_background_interval() {
        assert_eq!(background_interval(true), Duration::ZERO);
        assert_eq!(
            background_interval(false),
            Duration::from_secs_f64(1. / 60.)
        );
    }
}
