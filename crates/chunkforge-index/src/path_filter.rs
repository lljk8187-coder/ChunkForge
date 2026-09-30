//! Path include/exclude filter for archive paths (Phase 13).
//!
//! Used by `archive` / `extract` / `pull` / `push` `--path` / `--exclude`
//! (Phase 13–14). [`load_exclude_file`] reads `--exclude-from` text (Phase 14 M4).
//! Matching is literal UTF-8 / byte-oriented (no casefold). No `ignore` / `globset`.
//!
//! Archive paths follow the same conventions as [`crate::validate_archive_path`]
//! (usually `/`-separated, no leading `/`).

use crate::Error;

/// Compiled exclude pattern (validated at construction).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExcludePat {
    /// Exact path equality.
    Exact(String),
    /// Directory subtree: `dir/` → match `dir` or anything under `dir/`.
    DirPrefix(String),
    /// Prefix wildcard: `temp*` → `path.starts_with("temp")`.
    StartsWith(String),
    /// Suffix wildcard: `*.o` → `path.ends_with(".o")`.
    EndsWith(String),
}

impl ExcludePat {
    /// Parse and validate a single `--exclude` pattern.
    ///
    /// Rules (§3.1):
    /// - trailing `/` → directory prefix (no `*` allowed)
    /// - single `*` only at the start (`*.o`) or end (`temp*`) → wildcard
    /// - otherwise exact match
    /// - middle `*`, `**`, or multiple `*` → error
    pub fn parse(pat: impl Into<String>) -> Result<Self, Error> {
        let pat = pat.into();
        if pat.is_empty() {
            return Err(Error::InvalidExcludePattern(
                "exclude pattern must not be empty".into(),
            ));
        }
        if pat.ends_with('/') {
            if pat.contains('*') {
                return Err(Error::InvalidExcludePattern(format!(
                    "directory exclude must not contain '*': {pat:?}"
                )));
            }
            let dir = pat.trim_end_matches('/').to_string();
            if dir.is_empty() {
                return Err(Error::InvalidExcludePattern(
                    "directory exclude '/' is not allowed".into(),
                ));
            }
            return Ok(ExcludePat::DirPrefix(dir));
        }
        if pat.contains('*') {
            return parse_wildcard(&pat);
        }
        Ok(ExcludePat::Exact(pat))
    }

    fn matches(&self, path: &str) -> bool {
        match self {
            ExcludePat::Exact(p) => path == p,
            ExcludePat::DirPrefix(dir) => path_include_hit(dir, path),
            ExcludePat::StartsWith(prefix) => path.starts_with(prefix.as_str()),
            ExcludePat::EndsWith(suffix) => path.ends_with(suffix.as_str()),
        }
    }
}

fn parse_wildcard(pat: &str) -> Result<ExcludePat, Error> {
    let star_count = bytecount_star(pat);
    if star_count != 1 {
        return Err(Error::InvalidExcludePattern(format!(
            "exclude pattern may contain at most one '*' (no '**' / middle '*'): {pat:?}"
        )));
    }
    if let Some(suffix) = pat.strip_prefix('*') {
        // `*.o` or bare `*`
        if suffix.contains('*') {
            return Err(Error::InvalidExcludePattern(format!(
                "'*' only allowed at start or end of exclude pattern: {pat:?}"
            )));
        }
        return Ok(ExcludePat::EndsWith(suffix.to_string()));
    }
    if let Some(prefix) = pat.strip_suffix('*') {
        // `temp*`
        if prefix.contains('*') {
            return Err(Error::InvalidExcludePattern(format!(
                "'*' only allowed at start or end of exclude pattern: {pat:?}"
            )));
        }
        return Ok(ExcludePat::StartsWith(prefix.to_string()));
    }
    Err(Error::InvalidExcludePattern(format!(
        "'*' only allowed at start or end of exclude pattern: {pat:?}"
    )))
}

fn bytecount_star(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'*').count()
}

/// Load exclude patterns from a UTF-8 text file (`--exclude-from`).
///
/// One pattern per line (same grammar as [`ExcludePat::parse`] / CLI `--exclude`).
/// Blank lines and lines whose trimmed text starts with `#` are skipped.
/// Leading and trailing whitespace on each line is trimmed.
///
/// Does **not** compile patterns — callers merge the strings with CLI
/// `--exclude` and pass them to [`PathFilter::new`] so illegal patterns share
/// [`Error::InvalidExcludePattern`].
///
/// Open / read failure and non-UTF-8 → [`Error::ExcludeFile`].
pub fn load_exclude_file(path: impl AsRef<std::path::Path>) -> Result<Vec<String>, Error> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .map_err(|e| Error::ExcludeFile(format!("cannot read {}: {e}", path.display())))?;
    let text = std::string::String::from_utf8(bytes)
        .map_err(|_| Error::ExcludeFile(format!("{} is not valid UTF-8", path.display())))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        out.push(line.to_string());
    }
    Ok(out)
}

/// Include/exclude filter over archive-relative paths.
///
/// - Empty `paths` ⇒ include-all at the include stage.
/// - Non-empty `paths` ⇒ path must match **any** include (`==` or prefix `P/`).
/// - Then any matching exclude rejects the path.
/// - Empty paths + empty excludes ⇒ [`allows`](Self::allows) is always `true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathFilter {
    paths: Vec<String>,
    excludes: Vec<ExcludePat>,
}

impl PathFilter {
    /// Build a filter from include paths and exclude pattern strings.
    ///
    /// Exclude patterns are validated (illegal middle `*` / `**` → [`Error`]).
    pub fn new(
        paths: impl IntoIterator<Item = impl Into<String>>,
        excludes: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, Error> {
        let paths: Vec<String> = paths.into_iter().map(Into::into).collect();
        let mut excl = Vec::new();
        for e in excludes {
            excl.push(ExcludePat::parse(e)?);
        }
        Ok(Self {
            paths,
            excludes: excl,
        })
    }

    /// Include paths (empty ⇒ include-all).
    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    /// Compiled exclude patterns.
    pub fn excludes(&self) -> &[ExcludePat] {
        &self.excludes
    }

    /// Whether `archive_path` passes include then exclude.
    pub fn allows(&self, archive_path: &str) -> bool {
        if !self.included(archive_path) {
            return false;
        }
        !self.excluded(archive_path)
    }

    fn included(&self, path: &str) -> bool {
        if self.paths.is_empty() {
            return true;
        }
        self.paths.iter().any(|p| path_include_hit(p, path))
    }

    fn excluded(&self, path: &str) -> bool {
        self.excludes.iter().any(|e| e.matches(path))
    }
}

/// `--path P` hit: `path == P` or `path` is under `P/`.
fn path_include_hit(include: &str, path: &str) -> bool {
    path == include
        || (path.starts_with(include) && path.as_bytes().get(include.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_flags_allows_all() {
        let f = PathFilter::new(Vec::<String>::new(), Vec::<String>::new()).unwrap();
        assert!(f.allows("a.txt"));
        assert!(f.allows("sub/b.txt"));
        assert!(f.allows(""));
    }

    #[test]
    fn single_path_exact_and_subtree() {
        let f = PathFilter::new(["pkg/foo"], Vec::<String>::new()).unwrap();
        assert!(f.allows("pkg/foo"));
        assert!(f.allows("pkg/foo/bar.rs"));
        assert!(f.allows("pkg/foo/a/b"));
        assert!(!f.allows("pkg/foo2"));
        assert!(!f.allows("pkg"));
        assert!(!f.allows("other"));
    }

    #[test]
    fn multi_path_or() {
        let f = PathFilter::new(["a", "b/c"], Vec::<String>::new()).unwrap();
        assert!(f.allows("a"));
        assert!(f.allows("a/x"));
        assert!(f.allows("b/c"));
        assert!(f.allows("b/c/d"));
        assert!(!f.allows("b"));
        assert!(!f.allows("b/d"));
        assert!(!f.allows("c"));
    }

    #[test]
    fn exclude_exact() {
        let f = PathFilter::new(Vec::<String>::new(), ["secret.txt"]).unwrap();
        assert!(!f.allows("secret.txt"));
        assert!(f.allows("secret.txt.bak"));
        assert!(f.allows("dir/secret.txt"));
    }

    #[test]
    fn exclude_dir_prefix() {
        let f = PathFilter::new(Vec::<String>::new(), [".git/", "node_modules/"]).unwrap();
        assert!(!f.allows(".git"));
        assert!(!f.allows(".git/config"));
        assert!(!f.allows(".git/hooks/pre-commit"));
        assert!(!f.allows("node_modules"));
        assert!(!f.allows("node_modules/x"));
        assert!(f.allows(".gitignore"));
        assert!(f.allows("src/main.rs"));
    }

    #[test]
    fn exclude_star_suffix_and_prefix() {
        let f = PathFilter::new(Vec::<String>::new(), ["*.o", "temp*"]).unwrap();
        assert!(!f.allows("foo.o"));
        assert!(!f.allows("src/foo.o"));
        assert!(f.allows("foo.c"));
        assert!(!f.allows("temp"));
        assert!(!f.allows("temp123"));
        assert!(!f.allows("temporary/x"));
        assert!(f.allows("src/temp")); // whole-path prefix, not basename
        assert!(f.allows("mytemp"));
    }

    #[test]
    fn bare_star_excludes_all() {
        let f = PathFilter::new(Vec::<String>::new(), ["*"]).unwrap();
        assert!(!f.allows("a"));
        assert!(!f.allows(""));
    }

    #[test]
    fn path_then_exclude() {
        let f = PathFilter::new(["src"], ["src/vendor/", "*.o"]).unwrap();
        assert!(f.allows("src/main.rs"));
        assert!(!f.allows("src/vendor"));
        assert!(!f.allows("src/vendor/x.rs"));
        assert!(!f.allows("src/foo.o"));
        assert!(!f.allows("lib/main.rs")); // not in include
        assert!(!f.allows("lib/foo.o"));
    }

    #[test]
    fn case_sensitive_literal() {
        // Include paths are case-sensitive.
        let f = PathFilter::new(["Foo"], Vec::<String>::new()).unwrap();
        assert!(f.allows("Foo"));
        assert!(f.allows("Foo/x"));
        assert!(!f.allows("foo"));

        // Exclude patterns are case-sensitive (whole-path match).
        let f = PathFilter::new(Vec::<String>::new(), ["Bar/", "*.O"]).unwrap();
        assert!(!f.allows("Bar"));
        assert!(!f.allows("Bar/x"));
        assert!(f.allows("bar"));
        assert!(f.allows("bar/x"));
        assert!(!f.allows("x.O"));
        assert!(f.allows("x.o"));
    }

    #[test]
    fn illegal_middle_star() {
        let err = PathFilter::new(Vec::<String>::new(), ["a*b"]).unwrap_err();
        assert!(matches!(err, Error::InvalidExcludePattern(_)), "{err:?}");
    }

    #[test]
    fn illegal_double_star() {
        let err = PathFilter::new(Vec::<String>::new(), ["**"]).unwrap_err();
        assert!(matches!(err, Error::InvalidExcludePattern(_)), "{err:?}");
        let err = PathFilter::new(Vec::<String>::new(), ["a**b"]).unwrap_err();
        assert!(matches!(err, Error::InvalidExcludePattern(_)), "{err:?}");
        let err = PathFilter::new(Vec::<String>::new(), ["foo/**"]).unwrap_err();
        assert!(matches!(err, Error::InvalidExcludePattern(_)), "{err:?}");
    }

    #[test]
    fn illegal_star_in_dir_exclude() {
        let err = PathFilter::new(Vec::<String>::new(), ["foo*/"]).unwrap_err();
        assert!(matches!(err, Error::InvalidExcludePattern(_)), "{err:?}");
    }

    #[test]
    fn exclude_pat_parse_roundtrip_kinds() {
        assert_eq!(
            ExcludePat::parse("exact").unwrap(),
            ExcludePat::Exact("exact".into())
        );
        assert_eq!(
            ExcludePat::parse("dir/").unwrap(),
            ExcludePat::DirPrefix("dir".into())
        );
        assert_eq!(
            ExcludePat::parse("temp*").unwrap(),
            ExcludePat::StartsWith("temp".into())
        );
        assert_eq!(
            ExcludePat::parse("*.o").unwrap(),
            ExcludePat::EndsWith(".o".into())
        );
    }

    #[test]
    fn load_exclude_file_skips_blank_hash_and_trims() {
        let path = std::env::temp_dir().join(format!("cf-exclude-from-{}.txt", std::process::id()));
        std::fs::write(
            &path,
            "\n# comment\n  *.o  \n\n  # also comment\njunk/\n   \n",
        )
        .unwrap();
        let v = load_exclude_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(v, vec!["*.o".to_string(), "junk/".to_string()]);
    }

    #[test]
    fn load_exclude_file_missing_and_bad_utf8() {
        let missing =
            std::env::temp_dir().join(format!("cf-exclude-missing-{}", std::process::id()));
        let err = load_exclude_file(&missing).unwrap_err();
        assert!(matches!(err, Error::ExcludeFile(_)), "{err:?}");

        let path =
            std::env::temp_dir().join(format!("cf-exclude-badutf-{}.txt", std::process::id()));
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let err = load_exclude_file(&path).unwrap_err();
        let _ = std::fs::remove_file(&path);
        assert!(matches!(err, Error::ExcludeFile(_)), "{err:?}");
    }
}
