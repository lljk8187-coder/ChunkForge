use crate::Error;
use std::fmt;

/// BLAKE3 content address: 32 raw bytes, displayed as 64 lowercase hex chars.
#[derive(Clone, Copy, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ChunkId([u8; 32]);

impl ChunkId {
    /// Hash plaintext chunk bytes with BLAKE3.
    pub fn hash(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    /// Raw 32-byte digest.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Construct from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Lowercase hex (exactly 64 characters).
    pub fn to_hex(&self) -> String {
        hex_encode(&self.0)
    }

    /// Parse lowercase hex (exactly 64 characters). Uppercase is rejected.
    pub fn from_hex(s: &str) -> Result<Self, Error> {
        if s.len() != 64 {
            return Err(Error::InvalidChunkId(format!(
                "expected 64 hex chars, got {}",
                s.len()
            )));
        }
        if s.bytes()
            .any(|b| !b.is_ascii_hexdigit() || b.is_ascii_uppercase())
        {
            return Err(Error::InvalidChunkId(
                "ChunkId hex must be lowercase [0-9a-f]".into(),
            ));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks_exact(2).enumerate() {
            let hi = hex_nibble(chunk[0])?;
            let lo = hex_nibble(chunk[1])?;
            out[i] = (hi << 4) | lo;
        }
        Ok(Self(out))
    }
}

impl fmt::Display for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for ChunkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ChunkId({})", self.to_hex())
    }
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(64);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn hex_nibble(b: u8) -> Result<u8, Error> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        _ => Err(Error::InvalidChunkId(format!(
            "invalid hex digit {:?}",
            b as char
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_blake3_empty() {
        // Official BLAKE3 digest of the empty input.
        let id = ChunkId::hash(b"");
        assert_eq!(
            id.to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }
}
