//! Spans for profiling builds (`trace` feature). Without the feature `profile_span!` expands
//! to a zero-sized marker: its arguments are not evaluated and nothing is recorded.

#[cfg(feature = "trace")]
#[doc(hidden)]
pub use tracing;

/// An entered span named by a literal, with optional `tracing` fields; keep the guard alive for
/// the work it measures: `let _span = profile_span!("npc_think", id, class = %name);`.
#[cfg(feature = "trace")]
#[macro_export]
macro_rules! profile_span {
    ($($args:tt)*) => {
        $crate::profiling::tracing::info_span!($($args)*).entered()
    };
}

/// An entered span named by a literal, with optional `tracing` fields; keep the guard alive for
/// the work it measures: `let _span = profile_span!("npc_think", id, class = %name);`.
#[cfg(not(feature = "trace"))]
#[macro_export]
macro_rules! profile_span {
    ($($args:tt)*) => {
        $crate::profiling::NoSpan
    };
}

/// What `profile_span!` gives without the `trace` feature: nothing to record.
#[cfg(not(feature = "trace"))]
pub struct NoSpan;
