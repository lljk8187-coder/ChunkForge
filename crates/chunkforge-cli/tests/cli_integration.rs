//! Integration tests for the `chunkforge` binary (make → verify → cat).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chunkforge"))
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures")
}

fn run_ok(args: &[&str]) -> std::process::Output {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn chunkforge");
    assert!(
        out.status.success(),
        "chunkforge {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn run_fail(args: &[&str]) -> std::process::Output {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("spawn chunkforge");
    assert!(
        !out.status.success(),
        "chunkforge {:?} unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn make_verify_cat_cmp(fixture_rel: &str) {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = fixtures_dir().join(fixture_rel);
    assert!(input.is_file(), "missing fixture {}", input.display());

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    run_ok(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);

    let original = fs::read(&input).unwrap();
    let rebuilt = fs::read(&out).unwrap();
    assert_eq!(
        original,
        rebuilt,
        "cmp failed for fixture {fixture_rel}: {} vs {} bytes",
        original.len(),
        rebuilt.len()
    );
}

#[test]
fn help_and_version() {
    let help = run_ok(&["--help"]);
    let help_s = String::from_utf8_lossy(&help.stdout);
    assert!(help_s.contains("make"), "{help_s}");
    assert!(help_s.contains("cat"), "{help_s}");
    assert!(help_s.contains("verify"), "{help_s}");

    let ver = run_ok(&["--version"]);
    let ver_s = String::from_utf8_lossy(&ver.stdout);
    assert!(ver_s.contains("chunkforge"), "{ver_s}");
}

#[test]
fn make_verify_cat_hello() {
    make_verify_cat_cmp("hello.txt");
}

#[test]
fn make_verify_cat_empty() {
    make_verify_cat_cmp("empty");
}

#[test]
fn make_verify_cat_binary_256() {
    make_verify_cat_cmp("binary-256.bin");
}

#[test]
fn make_verify_cat_zeros_64k() {
    make_verify_cat_cmp("zeros-64k.bin");
}

#[test]
fn chunk_id_prints_hello() {
    let input = fixtures_dir().join("hello.txt");
    let out = run_ok(&["chunk-id", input.to_str().unwrap()]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().next().expect("one chunk line");
    let parts: Vec<&str> = line.split('\t').collect();
    assert_eq!(parts.len(), 3, "{line}");
    assert_eq!(parts[0], "0");
    assert_eq!(parts[1], "17");
    assert_eq!(parts[2].len(), 64);
}

#[test]
fn chunk_size_rejects_odd() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    let out = run_fail(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        "16385:65536:262144",
        input.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(err.contains("even") || err.contains("odd"), "stderr={err}");
}

#[test]
fn chunk_size_rejects_min_gt_avg() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let input = fixtures_dir().join("hello.txt");
    let out = run_fail(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        "--chunk-size",
        "65536:16384:262144",
        input.to_str().unwrap(),
    ]);
    let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
    assert!(
        err.contains("min") && (err.contains("avg") || err.contains("≤") || err.contains("<=")),
        "stderr={err}"
    );
}

#[test]
fn missing_chunk_fails_cat_and_verify() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx = dir.path().join("out.cfidx");
    let out = dir.path().join("reassembled");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);

    // Delete all .cnk files under the store.
    delete_cnk_files(&store.join("chunks"));

    run_fail(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
    ]);
    run_fail(&[
        "cat",
        "--store",
        store.to_str().unwrap(),
        idx.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
}

#[test]
fn dedup_second_make_same_file() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx1 = dir.path().join("v1.cfidx");
    let idx2 = dir.path().join("v2.cfidx");
    let input = fixtures_dir().join("hello.txt");

    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let count1 = count_cnk(&store.join("chunks"));
    run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let count2 = count_cnk(&store.join("chunks"));
    assert_eq!(count1, count2);
    assert!(count1 >= 1);
}

fn delete_cnk_files(root: &Path) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("cnk") {
                fs::remove_file(&path).unwrap();
            }
        }
    }
}

fn count_cnk(root: &Path) -> usize {
    if !root.exists() {
        return 0;
    }
    let mut n = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("cnk") {
                n += 1;
            }
        }
    }
    n
}
