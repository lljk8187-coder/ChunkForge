//! Store `meta.toml` (magic/version + uniform compression policy).

use crate::Error;
use std::fmt;
use std::fs;
use std::path::Path;

/// On-disk magic string for ChunkForge local stores.
pub const MAGIC: &str = "CFSTORE";

/// Phase-1 store metadata format version.
pub const VERSION: u32 = 1;

/// Uniform compression policy for every chunk in a store.
///
/// Phase 1 forbids per-chunk mixed compression; the policy is written once
/// into `meta.toml` and applies to all puts/gets for that store root.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Compression {
    /// Store plaintext bytes as-is in `.cnk` files.
    None,
    /// Store zstd frames in `.cnk` files (requires the `zstd` feature).
    #[cfg(feature = "zstd")]
    Zstd,
}

impl Compression {
    pub fn as_str(self) -> &'static str {
        match self {
            Compression::None => "none",
            #[cfg(feature = "zstd")]
            Compression::Zstd => "zstd",
        }
    }

    pub fn parse(s: &str) -> Result<Self, Error> {
        match s.trim() {
            "none" => Ok(Compression::None),
            #[cfg(feature = "zstd")]
            "zstd" => Ok(Compression::Zstd),
            #[cfg(not(feature = "zstd"))]
            "zstd" => Err(Error::CompressionUnavailable("zstd".into())),
            other => Err(Error::InvalidMeta(format!("unknown compression {other:?}"))),
        }
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsed contents of `<store_root>/meta.toml`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct StoreMeta {
    pub magic: String,
    pub version: u32,
    pub compression: Compression,
}

impl StoreMeta {
    pub fn new(compression: Compression) -> Self {
        Self {
            magic: MAGIC.to_string(),
            version: VERSION,
            compression,
        }
    }

    pub fn write_to(&self, path: &Path) -> Result<(), Error> {
        let body = format!(
            "# ChunkForge local CAS store metadata\n\
             magic = \"{}\"\n\
             version = {}\n\
             compression = \"{}\"\n",
            self.magic,
            self.version,
            self.compression.as_str()
        );
        fs::write(path, body)?;
        Ok(())
    }

    pub fn read_from(path: &Path) -> Result<Self, Error> {
        let text = fs::read_to_string(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::NotAStore(format!("missing meta.toml at {}", path.display()))
            } else {
                Error::Io(e)
            }
        })?;
        Self::parse(&text)
    }

    fn parse(text: &str) -> Result<Self, Error> {
        let mut magic: Option<String> = None;
        let mut version: Option<u32> = None;
        let mut compression: Option<Compression> = None;

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(Error::InvalidMeta(format!(
                    "expected key = value, got {line:?}"
                )));
            };
            let key = key.trim();
            let value = strip_quotes(value.trim());
            match key {
                "magic" => magic = Some(value.to_string()),
                "version" => {
                    let v: u32 = value
                        .parse()
                        .map_err(|_| Error::InvalidMeta(format!("invalid version {value:?}")))?;
                    version = Some(v);
                }
                "compression" => compression = Some(Compression::parse(value)?),
                _ => {
                    // Unknown keys are ignored for forward compatibility within
                    // the same major format; Phase 1 writers only emit the three
                    // keys above.
                }
            }
        }

        let magic = magic.ok_or_else(|| Error::InvalidMeta("missing magic".into()))?;
        if magic != MAGIC {
            return Err(Error::NotAStore(format!(
                "bad magic {magic:?}, expected {MAGIC:?}"
            )));
        }
        let version = version.ok_or_else(|| Error::InvalidMeta("missing version".into()))?;
        if version != VERSION {
            return Err(Error::InvalidMeta(format!(
                "unsupported store version {version} (expected {VERSION})"
            )));
        }
        let compression =
            compression.ok_or_else(|| Error::InvalidMeta("missing compression".into()))?;

        Ok(Self {
            magic,
            version,
            compression,
        })
    }
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_none() {
        let meta = StoreMeta::new(Compression::None);
        let text = format!(
            "magic = \"{}\"\nversion = {}\ncompression = \"{}\"\n",
            meta.magic,
            meta.version,
            meta.compression.as_str()
        );
        let parsed = StoreMeta::parse(&text).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn rejects_bad_magic() {
        let err = StoreMeta::parse("magic = \"NOPE\"\nversion = 1\ncompression = \"none\"\n")
            .unwrap_err();
        assert!(matches!(err, Error::NotAStore(_)));
    }
}
