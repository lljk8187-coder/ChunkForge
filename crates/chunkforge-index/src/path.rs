//! Archive path validation for `.cfdir` entries.
//!
//! Rules (Phase5 §3.1): UTF-8 relative, `/`-separated; reject `..`, absolute
//! paths, empty segments, Windows drive letters, and other escape forms.

use crate::Error;

/// Validate a relative archive path for `.cfdir` storage.
///
/// Accepted: non-empty UTF-8, `/`-separated segments, no leading `/`.
/// Rejected: empty path, absolute (`/…`), empty segments (`a//b`, trailing `/`),
/// `.` or `..` segments, backslashes, NUL, Windows drive letters (`C:…`).
pub fn validate_archive_path(path: &str) -> Result<(), Error> {
    if path.is_empty() {
        return Err(Error::InvalidPath("path must not be empty".into()));
    }
    if path.starts_with('/') {
        return Err(Error::InvalidPath(format!(
            "absolute path rejected: {path:?}"
        )));
    }
    if path.contains('\0') {
        return Err(Error::InvalidPath("path must not contain NUL".into()));
    }
    if path.contains('\\') {
        return Err(Error::InvalidPath(format!(
            "backslash rejected (use '/'): {path:?}"
        )));
    }
    // Windows drive letter: "C:…" or "c:/…"
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(Error::InvalidPath(format!(
            "Windows drive letter rejected: {path:?}"
        )));
    }

    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(Error::InvalidPath(format!(
                "empty path segment rejected: {path:?}"
            )));
        }
        if segment == ".." {
            return Err(Error::InvalidPath(format!(
                "'..' path segment rejected: {path:?}"
            )));
        }
        if segment == "." {
            return Err(Error::InvalidPath(format!(
                "'.' path segment rejected: {path:?}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_simple_relative() {
        validate_archive_path("a.txt").unwrap();
        validate_archive_path("sub/b.txt").unwrap();
        validate_archive_path("a/b/c").unwrap();
    }

    #[test]
    fn rejects_dotdot() {
        let err = validate_archive_path("../x").unwrap_err();
        assert!(matches!(err, Error::InvalidPath(_)), "{err:?}");
        let err = validate_archive_path("a/../b").unwrap_err();
        assert!(matches!(err, Error::InvalidPath(_)), "{err:?}");
        assert!(err.to_string().contains(".."), "{err}");
    }

    #[test]
    fn rejects_absolute_and_empty_segments() {
        assert!(matches!(
            validate_archive_path("/abs"),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            validate_archive_path("a//b"),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            validate_archive_path("a/"),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            validate_archive_path(""),
            Err(Error::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_windows_drive_and_backslash() {
        assert!(matches!(
            validate_archive_path("C:foo"),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            validate_archive_path("c:/windows"),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            validate_archive_path("a\\b"),
            Err(Error::InvalidPath(_))
        ));
    }

    #[test]
    fn rejects_dot_segment() {
        assert!(matches!(
            validate_archive_path("./x"),
            Err(Error::InvalidPath(_))
        ));
        assert!(matches!(
            validate_archive_path("a/./b"),
            Err(Error::InvalidPath(_))
        ));
    }
}
