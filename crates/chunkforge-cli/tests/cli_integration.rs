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

    let out1 = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let err1 = String::from_utf8_lossy(&out1.stderr);
    assert!(err1.contains("new="), "stderr={err1}");
    assert!(
        err1.contains("reused=0"),
        "first make should insert all: {err1}"
    );
    let count1 = count_cnk(&store.join("chunks"));

    let out2 = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        input.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&out2.stderr);
    assert!(
        err2.contains("new=0"),
        "identical remake must report new=0: {err2}"
    );
    assert!(err2.contains("reused="), "stderr={err2}");
    let count2 = count_cnk(&store.join("chunks"));
    assert_eq!(count1, count2, "store .cnk count must not grow on remake");
    assert!(count1 >= 1);
}

/// Always-on CI: small chunk params + mid-file mutation → some new, some reused.
#[test]
fn dedup_mid_file_mutation_reuses_chunks() {
    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx1 = dir.path().join("v1.cfidx");
    let idx2 = dir.path().join("v2.cfidx");

    // Build a multi-chunk blob with tiny CDC params (even, min≤avg≤max).
    // 48 KiB of patterned data → several ~4–8 KiB chunks.
    let mut data = Vec::with_capacity(48 * 1024);
    for i in 0..(48 * 1024) {
        data.push(((i * 17 + 3) % 251) as u8);
    }
    let orig = dir.path().join("orig.bin");
    fs::write(&orig, &data).unwrap();

    let mut mutated = data.clone();
    let mid = mutated.len() / 2;
    for b in &mut mutated[mid..mid + 512] {
        *b = b.wrapping_add(1);
    }
    let mut_path = dir.path().join("mut.bin");
    fs::write(&mut_path, &mutated).unwrap();

    let chunk_size = "2048:4096:8192";
    let out1 = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        "--chunk-size",
        chunk_size,
        orig.to_str().unwrap(),
    ]);
    let err1 = String::from_utf8_lossy(&out1.stderr);
    let (new1, reused1) = parse_make_stats(&err1);
    assert!(
        new1 >= 2,
        "expected multiple chunks, got new={new1} ({err1})"
    );
    assert_eq!(reused1, 0, "first make: {err1}");
    let count1 = count_cnk(&store.join("chunks"));

    let out2 = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        "--chunk-size",
        chunk_size,
        mut_path.to_str().unwrap(),
    ]);
    let err2 = String::from_utf8_lossy(&out2.stderr);
    let (new2, reused2) = parse_make_stats(&err2);
    assert!(
        reused2 >= 1,
        "mid-file mutation should reuse some chunks: {err2}"
    );
    assert!(
        new2 >= 1,
        "mid-file mutation should insert some new chunks: {err2}"
    );
    let count2 = count_cnk(&store.join("chunks"));
    assert!(
        count2 > count1,
        "store should gain some .cnk files ({count1} → {count2})"
    );
    assert!(
        count2 < count1 + new1,
        "reuse should keep growth well below a full rewrite ({count1}+{new1} vs {count2})"
    );

    run_ok(&[
        "verify",
        "--store",
        store.to_str().unwrap(),
        idx2.to_str().unwrap(),
    ]);
}

/// Optional large-file path (ignored in default CI). Uses scripts/gen_large.sh.
#[test]
#[ignore = "generates multi-MiB fixtures; run with --ignored"]
fn large_file_dedup() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let gen_dir = repo.join("fixtures/gen");
    let size_mib: u64 = std::env::var("CHUNKFORGE_GEN_MIB")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let script = repo.join("scripts/gen_large.sh");
    assert!(script.is_file(), "missing {}", script.display());

    let status = Command::new("bash")
        .arg(&script)
        .arg(&gen_dir)
        .arg(size_mib.to_string())
        .status()
        .expect("run gen_large.sh");
    assert!(status.success(), "gen_large.sh failed");

    let orig = gen_dir.join(format!("large-{size_mib}m.bin"));
    let mut_path = gen_dir.join(format!("large-{size_mib}m-mut.bin"));
    assert!(orig.is_file() && mut_path.is_file());

    let dir = tempdir().unwrap();
    let store = dir.path().join("store");
    let idx1 = dir.path().join("v1.cfidx");
    let idx1b = dir.path().join("v1b.cfidx");
    let idx2 = dir.path().join("v2.cfidx");

    let out1 = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1.to_str().unwrap(),
        orig.to_str().unwrap(),
    ]);
    let (new1, reused1) = parse_make_stats(&String::from_utf8_lossy(&out1.stderr));
    assert!(new1 >= 1 && reused1 == 0);
    let count1 = count_cnk(&store.join("chunks"));

    let out1b = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx1b.to_str().unwrap(),
        orig.to_str().unwrap(),
    ]);
    let (new1b, _) = parse_make_stats(&String::from_utf8_lossy(&out1b.stderr));
    assert_eq!(new1b, 0);
    assert_eq!(count1, count_cnk(&store.join("chunks")));

    let out2 = run_ok(&[
        "make",
        "--store",
        store.to_str().unwrap(),
        "-o",
        idx2.to_str().unwrap(),
        mut_path.to_str().unwrap(),
    ]);
    let (new2, reused2) = parse_make_stats(&String::from_utf8_lossy(&out2.stderr));
    assert!(new2 >= 1 && reused2 >= 1, "new={new2} reused={reused2}");
    assert!(count_cnk(&store.join("chunks")) > count1);
}

fn parse_make_stats(stderr: &str) -> (usize, usize) {
    let mut new_n = None;
    let mut reused_n = None;
    for part in stderr.split([' ', ',', ';', '(', ')']) {
        if let Some(rest) = part.strip_prefix("new=") {
            new_n = rest.parse().ok();
        }
        if let Some(rest) = part.strip_prefix("reused=") {
            reused_n = rest.parse().ok();
        }
    }
    (
        new_n.unwrap_or_else(|| panic!("missing new= in stderr: {stderr}")),
        reused_n.unwrap_or_else(|| panic!("missing reused= in stderr: {stderr}")),
    )
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
