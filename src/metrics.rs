// src/metrics.rs
// Simple timing helper for Phase 0 foundations.
// Usage: let result = timed!("operation_name", || { /* code */ });

use std::time::Instant;
use tracing::info;

/// Executes a closure, measures its execution time, logs it, and returns the closure's result.
pub fn timed<T>(name: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    let elapsed = start.elapsed();
    info!("timed", operation = name, duration_ms = elapsed.as_millis());
    result
}
