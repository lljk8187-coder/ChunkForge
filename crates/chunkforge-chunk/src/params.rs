use crate::Error;

/// FastCDC chunk size parameters (bytes). All values must be even and satisfy
/// `min ≤ avg ≤ max`. Defaults match Phase 1: 16 KiB / 64 KiB / 256 KiB.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ChunkParams {
    pub min: u64,
    pub avg: u64,
    pub max: u64,
}

impl Default for ChunkParams {
    fn default() -> Self {
        Self {
            min: 16 * 1024,  // 16384
            avg: 64 * 1024,  // 65536
            max: 256 * 1024, // 262144
        }
    }
}

impl ChunkParams {
    /// Validate and construct parameters.
    pub fn new(min: u64, avg: u64, max: u64) -> Result<Self, Error> {
        if min == 0 || avg == 0 || max == 0 {
            return Err(Error::InvalidParams("chunk sizes must be non-zero".into()));
        }
        if min % 2 != 0 || avg % 2 != 0 || max % 2 != 0 {
            return Err(Error::InvalidParams(format!(
                "chunk sizes must all be even (got min={min}, avg={avg}, max={max})"
            )));
        }
        if !(min <= avg && avg <= max) {
            return Err(Error::InvalidParams(format!(
                "require min ≤ avg ≤ max (got min={min}, avg={avg}, max={max})"
            )));
        }
        Ok(Self { min, avg, max })
    }
}
