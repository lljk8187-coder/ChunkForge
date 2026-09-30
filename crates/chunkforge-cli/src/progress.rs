//! Opt-in stderr progress lines for long-running CLI ops (Phase 12 M6 / O1).
//!
//! Enabled only with `--progress` (default off ≡ 1.1.0). Lines look like:
//! `progress: op=push done=12/64` when total is known, or
//! `progress: op=scrub done=3` when total is unknown.
//!
//! Progress always goes to **stderr** so it stays orthogonal to `--format json`
//! (JSON summary stays on stdout only). No `indicatif` / tracing / otel —
//! plain `eprintln!` after each completed unit of work.

use std::sync::atomic::{AtomicUsize, Ordering};

/// Thread-safe progress counter for parallel `map_indexed` workers.
pub(crate) struct ProgressReporter {
    enabled: bool,
    op: &'static str,
    total: Option<usize>,
    done: AtomicUsize,
}

impl ProgressReporter {
    pub(crate) fn new(enabled: bool, op: &'static str, total: Option<usize>) -> Self {
        Self {
            enabled,
            op,
            total,
            done: AtomicUsize::new(0),
        }
    }

    /// Record one completed unit of work; emit a progress line when enabled.
    pub(crate) fn tick(&self) {
        if !self.enabled {
            return;
        }
        let n = self.done.fetch_add(1, Ordering::Relaxed) + 1;
        match self.total {
            Some(t) => eprintln!("progress: op={} done={n}/{t}", self.op),
            None => eprintln!("progress: op={} done={n}", self.op),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_tick_is_noop() {
        let p = ProgressReporter::new(false, "scrub", Some(3));
        p.tick();
        p.tick();
        assert_eq!(p.done.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn enabled_increments() {
        let p = ProgressReporter::new(true, "push", Some(2));
        p.tick();
        p.tick();
        assert_eq!(p.done.load(Ordering::Relaxed), 2);
    }
}
