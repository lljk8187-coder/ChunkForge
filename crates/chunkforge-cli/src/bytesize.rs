//! Human-friendly byte size parsing for `--cache-max-bytes` (Phase16-M3).
//!
//! Accepts a pure decimal integer (`1048576`) **or** `<num>[K|M|G|Ki|Mi|Gi]`
//! on a **1024** base (case-insensitive):
//! - `K` / `Ki` = 2^10
//! - `M` / `Mi` = 2^20
//! - `G` / `Gi` = 2^30
//!
//! Deliberately **rejects** suffixes that include `B` (`64MB`, `1KB`, `1KiB`):
//! only bare `K`/`M`/`G`/`Ki`/`Mi`/`Gi` are recognized (no KB/MB/GB).
//! Decimals (`1.5M`), empty input, unknown suffixes, and `u64` overflow → Err.

/// Parse a human-friendly byte size into a `u64` byte count.
///
/// See module docs for the accepted grammar and deliberate rejections.
pub(crate) fn parse_byte_size(input: &str) -> Result<u64, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty byte size (expected decimal integer or <num>[K|M|G|Ki|Mi|Gi])".into());
    }

    // Digits only — no signs, no decimals. A `.` after digits (e.g. `1.5M`)
    // lands in the suffix and is rejected below.
    let mut num_end = 0usize;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() {
            num_end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if num_end == 0 {
        return Err(format!(
            "invalid byte size '{input}': expected decimal integer or <num>[K|M|G|Ki|Mi|Gi]"
        ));
    }

    let num_str = &s[..num_end];
    let suffix = &s[num_end..];

    let n: u64 = num_str
        .parse()
        .map_err(|_| format!("invalid byte size '{input}': number does not fit in u64"))?;

    if suffix.is_empty() {
        return Ok(n);
    }

    // Reject decimals explicitly for a clearer message (`1.5M`).
    if suffix.starts_with('.') {
        return Err(format!(
            "invalid byte size '{input}': decimals are not accepted (use whole numbers only)"
        ));
    }

    let lower = suffix.to_ascii_lowercase();
    let mult = match lower.as_str() {
        "k" | "ki" => 1u64 << 10,
        "m" | "mi" => 1u64 << 20,
        "g" | "gi" => 1u64 << 30,
        other if other.contains('b') => {
            return Err(format!(
                "invalid byte size '{input}': suffixes with 'B' are not accepted \
                 (use K/M/G/Ki/Mi/Gi, not KB/MB/GB)"
            ));
        }
        _ => {
            return Err(format!(
                "invalid byte size '{input}': unknown suffix '{suffix}' \
                 (accepted: K/M/G/Ki/Mi/Gi, or plain integer bytes)"
            ));
        }
    };

    n.checked_mul(mult)
        .ok_or_else(|| format!("invalid byte size '{input}': overflow u64"))
}

#[cfg(test)]
mod tests {
    use super::parse_byte_size;

    #[test]
    fn plain_integer() {
        assert_eq!(parse_byte_size("1048576").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("0").unwrap(), 0);
        assert_eq!(parse_byte_size("1024").unwrap(), 1024);
    }

    #[test]
    fn suffix_m_case_insensitive() {
        assert_eq!(parse_byte_size("1M").unwrap(), 1_048_576);
        assert_eq!(parse_byte_size("1m").unwrap(), 1_048_576);
    }

    #[test]
    fn suffix_mi() {
        assert_eq!(parse_byte_size("64Mi").unwrap(), 67_108_864);
        assert_eq!(parse_byte_size("64mi").unwrap(), 67_108_864);
    }

    #[test]
    fn suffix_k_and_gi() {
        assert_eq!(parse_byte_size("1K").unwrap(), 1024);
        assert_eq!(parse_byte_size("1k").unwrap(), 1024);
        assert_eq!(parse_byte_size("1Ki").unwrap(), 1024);
        assert_eq!(parse_byte_size("1Gi").unwrap(), 1u64 << 30);
        assert_eq!(parse_byte_size("1G").unwrap(), 1u64 << 30);
        assert_eq!(parse_byte_size("2g").unwrap(), 2u64 << 30);
    }

    #[test]
    fn rejects_decimal() {
        let err = parse_byte_size("1.5M").unwrap_err();
        assert!(err.contains("decimal") || err.contains("1.5M"), "err={err}");
    }

    #[test]
    fn rejects_b_suffix() {
        for bad in ["64MB", "64mb", "1KB", "1kb", "1KiB", "1MiB", "1GB"] {
            let err = parse_byte_size(bad).unwrap_err();
            assert!(
                err.contains('B')
                    || err.contains('b')
                    || err.to_lowercase().contains("not accepted"),
                "expected B-rejection for {bad}, got {err}"
            );
        }
    }

    #[test]
    fn rejects_empty_and_unknown() {
        assert!(parse_byte_size("").is_err());
        assert!(parse_byte_size("   ").is_err());
        assert!(parse_byte_size("M").is_err());
        assert!(parse_byte_size("1T").is_err());
        assert!(parse_byte_size("1X").is_err());
        assert!(parse_byte_size("-1").is_err());
        assert!(parse_byte_size("+1").is_err());
    }

    #[test]
    fn rejects_overflow() {
        // u64::MAX as plain number is ok; MAX+1 is not.
        assert_eq!(parse_byte_size(&u64::MAX.to_string()).unwrap(), u64::MAX);
        assert!(parse_byte_size("18446744073709551616").is_err());
        // 2^34 * 2^30 = 2^64 → overflow for both G and Gi.
        assert!(parse_byte_size("17179869184Gi").is_err());
        assert!(parse_byte_size("17179869184G").is_err());
        // Just under the limit still fits.
        assert_eq!(
            parse_byte_size("17179869183G").unwrap(),
            17179869183u64 << 30
        );
    }
}
