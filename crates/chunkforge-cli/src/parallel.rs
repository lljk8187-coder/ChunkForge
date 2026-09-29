//! Bounded concurrency helpers for CLI orchestration (Phase 4 M5).
//!
//! Uses `std::thread::scope` only — no async runtime. Traits (`ChunkSource` /
//! `ChunkSink`) stay synchronous; parallelism lives in the caller.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Run `f(index, item)` over `items` with at most `jobs` worker threads.
///
/// Results are returned in the same order as `items`. When `jobs <= 1` or there
/// is at most one item, work runs on the calling thread (no spawn) so
/// `--jobs 1` matches prior serial behaviour.
pub(crate) fn map_indexed<T, R, F>(items: &[T], jobs: usize, f: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    if jobs <= 1 || n == 1 {
        return items
            .iter()
            .enumerate()
            .map(|(i, item)| f(i, item))
            .collect();
    }

    let workers = jobs.min(n);
    let results: Vec<Mutex<Option<R>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    let value = f(i, &items[i]);
                    *results[i].lock().expect("parallel worker result lock") = Some(value);
                }
            });
        }
    });

    results
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .expect("parallel result mutex")
                .expect("parallel worker must fill every slot")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn jobs_one_preserves_order_and_values() {
        let items: Vec<u32> = (0..10).collect();
        let out = map_indexed(&items, 1, |i, v| (i, *v * 2));
        assert_eq!(out.len(), 10);
        for (i, (idx, val)) in out.iter().enumerate() {
            assert_eq!(*idx, i);
            assert_eq!(*val, (i as u32) * 2);
        }
    }

    #[test]
    fn jobs_four_preserves_order() {
        let items: Vec<u32> = (0..32).collect();
        let out = map_indexed(&items, 4, |_i, v| v.wrapping_mul(3).wrapping_add(7));
        let expected: Vec<u32> = items
            .iter()
            .map(|v| v.wrapping_mul(3).wrapping_add(7))
            .collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn empty_input() {
        let items: Vec<u32> = vec![];
        let out: Vec<u32> = map_indexed(&items, 8, |_, v| *v);
        assert!(out.is_empty());
    }

    #[test]
    fn concurrent_workers_observe_all_items() {
        let items: Vec<usize> = (0..64).collect();
        let seen = AtomicUsize::new(0);
        let out = map_indexed(&items, 8, |_i, v| {
            seen.fetch_add(1, Ordering::SeqCst);
            *v
        });
        assert_eq!(seen.load(Ordering::SeqCst), 64);
        assert_eq!(out, items);
    }
}
