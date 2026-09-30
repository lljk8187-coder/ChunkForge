//! Ordered multi-source read failover (Missing-only).
//!
//! [`FallbackSource`] tries sources in order. [`get`](ChunkSource::get) advances
//! to the next source **only** on [`SourceError::NotFound`]; `Corrupt` / `Io` /
//! `Backend` fail immediately. [`has`](ChunkSource::has) is a short-circuit OR
//! (any `Err` propagates). Empty construction is rejected.

use crate::source::{ChunkSource, SourceError};
use chunkforge_chunk::ChunkId;
use thiserror::Error;

/// Errors from constructing a [`FallbackSource`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FallbackError {
    /// `sources` was empty; at least one [`ChunkSource`] is required.
    #[error("FallbackSource requires at least one source")]
    Empty,
}

/// Ordered multi-source failover over [`ChunkSource`] backends.
///
/// Only [`SourceError::NotFound`] (Missing) triggers trying the next source.
/// Transient / permanent / corrupt failures from any source are returned as-is
/// and never silently swallowed. Invariant: `sources.len() >= 1`.
pub struct FallbackSource {
    sources: Vec<Box<dyn ChunkSource>>,
}

impl std::fmt::Debug for FallbackSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FallbackSource")
            .field("sources", &format_args!("[{} sources]", self.sources.len()))
            .finish()
    }
}

impl FallbackSource {
    /// Build a failover chain. `sources` must be non-empty.
    pub fn new(sources: Vec<Box<dyn ChunkSource>>) -> Result<Self, FallbackError> {
        if sources.is_empty() {
            return Err(FallbackError::Empty);
        }
        Ok(Self { sources })
    }

    /// Number of underlying sources (always ≥ 1 after a successful [`new`]).
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// `true` only if the inner list is empty (rejected by [`new`]).
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Borrow the ordered source list.
    pub fn sources(&self) -> &[Box<dyn ChunkSource>] {
        &self.sources
    }
}

impl ChunkSource for FallbackSource {
    fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
        for s in &self.sources {
            if s.has(id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
        for s in &self.sources {
            match s.get(id) {
                Ok(data) => return Ok(data),
                Err(SourceError::NotFound(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(SourceError::NotFound(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Configurable mock [`ChunkSource`] with call counters.
    struct MockSource {
        /// When `Some(data)`, `get`/`has` succeed with that payload; `None` → NotFound.
        data: Option<Vec<u8>>,
        /// If set, `get` returns this error instead of data/NotFound (and `has` too if Backend/Io/Corrupt).
        fail: Option<SourceError>,
        gets: Arc<AtomicUsize>,
        has_calls: Arc<AtomicUsize>,
    }

    impl MockSource {
        fn hit(data: Vec<u8>, gets: Arc<AtomicUsize>, has_calls: Arc<AtomicUsize>) -> Self {
            Self {
                data: Some(data),
                fail: None,
                gets,
                has_calls,
            }
        }

        fn miss(gets: Arc<AtomicUsize>, has_calls: Arc<AtomicUsize>) -> Self {
            Self {
                data: None,
                fail: None,
                gets,
                has_calls,
            }
        }

        fn failing(err: SourceError, gets: Arc<AtomicUsize>, has_calls: Arc<AtomicUsize>) -> Self {
            Self {
                data: None,
                fail: Some(err),
                gets,
                has_calls,
            }
        }
    }

    impl ChunkSource for MockSource {
        fn has(&self, id: &ChunkId) -> Result<bool, SourceError> {
            self.has_calls.fetch_add(1, Ordering::SeqCst);
            if let Some(ref e) = self.fail {
                return Err(clone_source_error(e, id));
            }
            Ok(self.data.is_some())
        }

        fn get(&self, id: &ChunkId) -> Result<Vec<u8>, SourceError> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            if let Some(ref e) = self.fail {
                return Err(clone_source_error(e, id));
            }
            match &self.data {
                Some(d) => Ok(d.clone()),
                None => Err(SourceError::NotFound(*id)),
            }
        }
    }

    fn clone_source_error(e: &SourceError, id: &ChunkId) -> SourceError {
        match e {
            SourceError::NotFound(_) => SourceError::NotFound(*id),
            SourceError::Corrupt(_) => SourceError::Corrupt(*id),
            SourceError::Io(io) => SourceError::Io(std::io::Error::new(io.kind(), io.to_string())),
            SourceError::Backend(msg) => SourceError::Backend(msg.clone()),
        }
    }

    fn id_for(data: &[u8]) -> ChunkId {
        ChunkId::hash(data)
    }

    #[test]
    fn empty_new_returns_err() {
        let err = FallbackSource::new(vec![]).unwrap_err();
        assert_eq!(err, FallbackError::Empty);
    }

    #[test]
    fn a_miss_b_hit_get_succeeds_both_queried() {
        let payload = b"fallback-a-miss-b-hit".to_vec();
        let id = id_for(&payload);
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::miss(Arc::clone(&a_gets), Arc::clone(&a_has))),
            Box::new(MockSource::hit(
                payload.clone(),
                Arc::clone(&b_gets),
                Arc::clone(&b_has),
            )),
        ])
        .unwrap();

        assert_eq!(chain.get(&id).unwrap(), payload);
        assert_eq!(a_gets.load(Ordering::SeqCst), 1);
        assert_eq!(b_gets.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_hit_does_not_touch_b() {
        let payload = b"fallback-a-hit-skip-b".to_vec();
        let id = id_for(&payload);
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::hit(
                payload.clone(),
                Arc::clone(&a_gets),
                Arc::clone(&a_has),
            )),
            Box::new(MockSource::hit(
                b"should-not-be-read".to_vec(),
                Arc::clone(&b_gets),
                Arc::clone(&b_has),
            )),
        ])
        .unwrap();

        assert_eq!(chain.get(&id).unwrap(), payload);
        assert_eq!(a_gets.load(Ordering::SeqCst), 1);
        assert_eq!(b_gets.load(Ordering::SeqCst), 0);
        assert_eq!(b_has.load(Ordering::SeqCst), 0);

        assert!(chain.has(&id).unwrap());
        assert_eq!(a_has.load(Ordering::SeqCst), 1);
        assert_eq!(b_has.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_transient_backend_fails_fast_without_touching_b() {
        let id = id_for(b"transient-probe");
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::failing(
                SourceError::Backend("503".into()),
                Arc::clone(&a_gets),
                Arc::clone(&a_has),
            )),
            Box::new(MockSource::hit(
                b"must-not-reach".to_vec(),
                Arc::clone(&b_gets),
                Arc::clone(&b_has),
            )),
        ])
        .unwrap();

        let err = chain.get(&id).unwrap_err();
        match &err {
            SourceError::Backend(m) if m == "503" => {}
            other => panic!("expected Backend(503), got {other:?}"),
        }
        assert_eq!(a_gets.load(Ordering::SeqCst), 1);
        assert_eq!(b_gets.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_io_fails_fast_without_touching_b() {
        let id = id_for(b"io-probe");
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::failing(
                SourceError::Io(std::io::Error::other("disk failed")),
                Arc::clone(&a_gets),
                Arc::clone(&a_has),
            )),
            Box::new(MockSource::hit(
                b"must-not-reach".to_vec(),
                Arc::clone(&b_gets),
                Arc::clone(&b_has),
            )),
        ])
        .unwrap();

        assert!(matches!(chain.get(&id), Err(SourceError::Io(_))));
        assert_eq!(a_gets.load(Ordering::SeqCst), 1);
        assert_eq!(b_gets.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_corrupt_fails_fast_without_touching_b() {
        let id = id_for(b"corrupt-probe");
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::failing(
                SourceError::Corrupt(id),
                Arc::clone(&a_gets),
                Arc::clone(&a_has),
            )),
            Box::new(MockSource::hit(
                b"must-not-reach".to_vec(),
                Arc::clone(&b_gets),
                Arc::clone(&b_has),
            )),
        ])
        .unwrap();

        assert!(matches!(
            chain.get(&id),
            Err(SourceError::Corrupt(c)) if c == id
        ));
        assert_eq!(a_gets.load(Ordering::SeqCst), 1);
        assert_eq!(b_gets.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn all_miss_returns_not_found() {
        let id = id_for(b"all-miss-probe");
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::miss(Arc::clone(&a_gets), Arc::clone(&a_has))),
            Box::new(MockSource::miss(Arc::clone(&b_gets), Arc::clone(&b_has))),
        ])
        .unwrap();

        assert!(matches!(
            chain.get(&id),
            Err(SourceError::NotFound(c)) if c == id
        ));
        assert_eq!(a_gets.load(Ordering::SeqCst), 1);
        assert_eq!(b_gets.load(Ordering::SeqCst), 1);

        assert!(!chain.has(&id).unwrap());
        assert_eq!(a_has.load(Ordering::SeqCst), 1);
        assert_eq!(b_has.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn has_propagates_error_without_swallowing() {
        let id = id_for(b"has-err-probe");
        let a_gets = Arc::new(AtomicUsize::new(0));
        let b_gets = Arc::new(AtomicUsize::new(0));
        let a_has = Arc::new(AtomicUsize::new(0));
        let b_has = Arc::new(AtomicUsize::new(0));

        let chain = FallbackSource::new(vec![
            Box::new(MockSource::failing(
                SourceError::Backend("503".into()),
                Arc::clone(&a_gets),
                Arc::clone(&a_has),
            )),
            Box::new(MockSource::hit(
                b"x".to_vec(),
                Arc::clone(&b_gets),
                Arc::clone(&b_has),
            )),
        ])
        .unwrap();

        match chain.has(&id) {
            Err(SourceError::Backend(ref m)) if m == "503" => {}
            other => panic!("expected Backend(503), got {other:?}"),
        }
        assert_eq!(a_has.load(Ordering::SeqCst), 1);
        assert_eq!(b_has.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn single_source_passthrough() {
        let payload = b"single-source".to_vec();
        let id = id_for(&payload);
        let gets = Arc::new(AtomicUsize::new(0));
        let has_calls = Arc::new(AtomicUsize::new(0));
        let chain = FallbackSource::new(vec![Box::new(MockSource::hit(
            payload.clone(),
            Arc::clone(&gets),
            Arc::clone(&has_calls),
        ))])
        .unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain.get(&id).unwrap(), payload);
        assert!(chain.has(&id).unwrap());
    }
}
